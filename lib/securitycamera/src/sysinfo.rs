//! Reports the state of the machine and of this process. It reads `/proc` and
//! `/sys`, so most values are only available on Linux; what cannot be read is
//! left out of the report instead of failing it.

use std::fmt::Write;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use crate::util::{go_duration, hostname, round_secs};

/// How long CPU usage is measured over.
const SAMPLE_WINDOW: Duration = Duration::from_millis(300);

fn start_time() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

/// Starts the clock of "running for". Call it when the program starts.
pub fn init() {
    start_time();
}

/// The busy and total jiffies of the whole machine from the first line of
/// `/proc/stat`.
fn cpu_times(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().next()?;
    let f: Vec<&str> = line.split_whitespace().collect();
    if f.len() < 5 || f[0] != "cpu" {
        return None;
    }
    let (mut total, mut idle) = (0u64, 0u64);
    for (i, s) in f[1..].iter().enumerate() {
        let v: u64 = s.parse().ok()?;
        if i >= 8 {
            break; // guest times are already counted in user/nice
        }
        total += v;
        if i == 3 || i == 4 {
            idle += v; // idle, iowait
        }
    }
    Some((total - idle, total))
}

/// This process's utime+stime in jiffies from `/proc/self/stat`.
fn proc_times(stat: &str) -> Option<u64> {
    // the command name is in parentheses and may contain spaces
    let rest = &stat[stat.rfind(')')? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    // utime and stime are fields 14 and 15 of the whole line
    if f.len() < 13 {
        return None;
    }
    Some(f[11].parse::<u64>().ok()? + f[12].parse::<u64>().ok()?)
}

/// Parses "Key:   123 kB" lines into kilobytes by key.
fn kv_kb(s: &str) -> std::collections::HashMap<String, u64> {
    s.lines()
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            Some((k.to_owned(), v.split_whitespace().next()?.parse().ok()?))
        })
        .collect()
}

fn read_file(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).map(|s| s.trim().to_owned()).unwrap_or_default()
}

/// One temperature reading.
struct Sensor {
    name: String,
    c: f64,
}

/// Every file matching `dir/<prefix>*/<file>` style patterns that std can't glob.
fn sensors_in(dir: &str, entry_prefix: &str, temp_files: impl Fn(&Path) -> Vec<std::path::PathBuf>, name_file: &str) -> Vec<Sensor> {
    let mut entries: Vec<_> = fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    entries.sort();
    let mut out = Vec::new();
    for d in entries.into_iter().filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(entry_prefix))) {
        let mut name = read_file(d.join(name_file));
        if name.is_empty() {
            name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        }
        for f in temp_files(&d) {
            if let Ok(v) = read_file(&f).parse::<f64>() {
                out.push(Sensor { name: name.clone(), c: v / 1000.0 });
            }
        }
    }
    out
}

/// Every thermal zone, falling back to hwmon sensors when there are none.
fn read_temperatures() -> Vec<Sensor> {
    let zones = sensors_in("/sys/class/thermal", "thermal_zone", |d| vec![d.join("temp")], "type");
    if !zones.is_empty() {
        return zones;
    }
    sensors_in(
        "/sys/class/hwmon",
        "hwmon",
        |d| {
            let mut files: Vec<_> = fs::read_dir(d)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("temp") && n.to_string_lossy().ends_with("_input")))
                .collect();
            files.sort();
            files
        },
        "name",
    )
}

/// The hottest sensor in °C; `None` if the machine has no readable sensor.
pub fn max_temperature() -> Option<f64> {
    read_temperatures().into_iter().map(|s| s.c).max_by(f64::total_cmp)
}

/// The size of the disk holding `dir` and how much of it is free to use, in bytes.
fn disk(dir: &str) -> Option<(u64, u64)> {
    let path = std::ffi::CString::new(dir).ok()?;
    // SAFETY: statvfs fills the struct, which is plain data, from a valid C string.
    let mut fs: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut fs) } != 0 {
        return None;
    }
    let unit = fs.f_frsize as u64;
    let total = fs.f_blocks as u64 * unit;
    (total > 0).then(|| (total, fs.f_bavail as u64 * unit))
}

/// How full, from 0 to 100, the disk holding `dir` is.
pub fn disk_used_percent(dir: &str) -> Option<f64> {
    let (total, free) = disk(dir)?;
    Some(pct(total - free, total))
}

/// The usage of the whole machine, from 0 to 100, measured over a fraction of
/// a second.
pub fn cpu_percent() -> Option<f64> {
    let (busy0, total0) = cpu_times(&read_file("/proc/stat"))?;
    thread::sleep(SAMPLE_WINDOW);
    let (busy1, total1) = cpu_times(&read_file("/proc/stat"))?;
    (total1 > total0).then(|| pct(busy1 - busy0, total1 - total0))
}

fn mb(kb: u64) -> String {
    format!("{:.0} MB", kb as f64 / 1024.0)
}

fn gb(b: u64) -> String {
    format!("{:.1} GB", b as f64 / (1u64 << 30) as f64)
}

fn pct(part: u64, whole: u64) -> f64 {
    if whole == 0 { 0.0 } else { 100.0 * part as f64 / whole as f64 }
}

/// A duration as "3d 4h5m6s", the days first when there are any.
fn format_duration(d: Duration) -> String {
    let d = round_secs(d);
    let days = d.as_secs() / 86_400;
    let rest = d - Duration::from_secs(days * 86_400);
    if days > 0 { format!("{days}d {}", go_duration(rest)) } else { go_duration(rest) }
}

fn cores() -> usize {
    thread::available_parallelism().map_or(1, |n| n.get())
}

/// A multi-line, human readable summary of the machine and of this process.
/// It blocks for a fraction of a second to measure CPU usage. `dir` is the
/// directory whose disk usage is reported.
pub fn report(dir: &str) -> String {
    let mut b = String::new();
    macro_rules! section {
        ($title:expr) => {
            let _ = write!(b, "\n{}\n", $title);
        };
    }
    macro_rules! line {
        ($($arg:tt)*) => {
            let _ = writeln!(b, "  {}", format_args!($($arg)*));
        };
    }

    // sample CPU usage over a short window, for the machine and for this process
    let stat0 = read_file("/proc/stat");
    let proc0 = proc_times(&read_file("/proc/self/stat"));
    thread::sleep(SAMPLE_WINDOW);
    let times0 = cpu_times(&stat0);
    let times1 = cpu_times(&read_file("/proc/stat"));
    let proc1 = proc_times(&read_file("/proc/self/stat"));
    let machine = match (times0, times1) {
        (Some((b0, t0)), Some((b1, t1))) if t1 > t0 => Some((b1 - b0, t1 - t0)),
        _ => None,
    };

    section!("System");
    line!("host: {} ({}/{})", hostname(), std::env::consts::OS, std::env::consts::ARCH);
    if let Some(secs) = read_file("/proc/uptime").split(' ').next().and_then(|s| s.parse::<f64>().ok()) {
        line!("uptime: {}", format_duration(Duration::from_secs_f64(secs)));
    }
    let load: Vec<String> = read_file("/proc/loadavg").split_whitespace().map(str::to_owned).collect();
    if load.len() >= 3 {
        line!("load average: {} {} {} ({} cores)", load[0], load[1], load[2], cores());
    }

    section!("CPU");
    if let Some((busy, total)) = machine {
        line!("usage: {:.1}%", pct(busy, total));
    }
    let temps = read_temperatures();
    if temps.is_empty() {
        line!("temperature: not available");
    }
    for t in temps {
        line!("temperature: {} {:.1}°C", t.name, t.c);
    }

    let mem = kv_kb(&read_file("/proc/meminfo"));
    let get = |k: &str| mem.get(k).copied().unwrap_or(0);
    if get("MemTotal") > 0 {
        section!("Memory");
        let used = get("MemTotal").saturating_sub(get("MemAvailable"));
        line!("ram: {} / {} ({:.1}%)", mb(used), mb(get("MemTotal")), pct(used, get("MemTotal")));
        if get("SwapTotal") > 0 {
            let swap_used = get("SwapTotal").saturating_sub(get("SwapFree"));
            line!("swap: {} / {} ({:.1}%)", mb(swap_used), mb(get("SwapTotal")), pct(swap_used, get("SwapTotal")));
        }
    }

    if let Some((total, free)) = disk(dir) {
        section!("Disk");
        line!("{dir}: {} free of {} ({:.1}% used)", gb(free), gb(total), pct(total - free, total));
    }

    section!("Security camera process");
    line!("pid: {}, running for {}", std::process::id(), format_duration(start_time().elapsed()));
    if let (Some(p0), Some(p1), Some((_, total))) = (proc0, proc1, machine) {
        // jiffies of this process over jiffies of all cores: 100% means one full core
        let share = p1.saturating_sub(p0) as f64 / total as f64 * cores() as f64;
        line!("cpu: {:.1}% of one core", 100.0 * share);
    }
    let status = kv_kb(&read_file("/proc/self/status"));
    if let Some(rss) = status.get("VmRSS").filter(|&&v| v > 0) {
        line!("memory (rss): {}", mb(*rss));
    }
    if let Some(threads) = status.get("Threads") {
        line!("threads: {threads}");
    }

    b.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_times_from_proc_stat() {
        assert_eq!(cpu_times("cpu  100 10 50 800 40 0 0 0 0 0\ncpu0 1 2 3 4 5\n"), Some((160, 1000)));
        assert_eq!(cpu_times("garbage"), None);
        assert_eq!(cpu_times("cpu 1 2 x 4 5"), None);
    }

    #[test]
    fn proc_times_from_proc_self_stat() {
        let stat = "42 (my (cam) prog) S 1 2 3 4 5 6 7 8 9 10 111 222 0 0 20";
        assert_eq!(proc_times(stat), Some(333));
        assert_eq!(proc_times("no parenthesis"), None);
        assert_eq!(proc_times("1 (x) S 1 2"), None);
    }

    #[test]
    fn kv_kb_parses_key_value_lines() {
        let m = kv_kb("MemTotal:  2048 kB\nMemFree:   1 kB\nnope\nThreads:\t7\n");
        assert_eq!((m["MemTotal"], m["MemFree"], m["Threads"]), (2048, 1, 7));
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn durations_have_days_first() {
        assert_eq!(format_duration(Duration::from_secs(90_061)), "1d 1h1m1s");
        assert_eq!(format_duration(Duration::from_secs(61)), "1m1s");
    }

    #[test]
    fn the_report_has_its_sections() {
        let r = report(".");
        for want in ["System", "CPU", "Disk", "Security camera process", "pid: "] {
            assert!(r.contains(want), "report misses {want:?}:\n{r}");
        }
    }

    #[test]
    fn the_disk_and_the_cpu_can_be_measured() {
        let used = disk_used_percent(".").expect("the disk of the current directory");
        assert!((0.0..=100.0).contains(&used));
        assert!(disk_used_percent("/no/such/place").is_none());
        let cpu = cpu_percent().expect("/proc/stat");
        assert!((0.0..=100.0).contains(&cpu));
    }
}
