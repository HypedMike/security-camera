//! Small helpers shared by the modules.

use std::time::Duration;

/// Formats a duration like Go's `Duration.String`: `30s`, `1m0s`, `1h2m3.5s`,
/// `250ms`. The reports sent to Telegram have always read like that.
pub fn go_duration(d: Duration) -> String {
    let ns = d.as_nanos();
    if ns == 0 {
        return "0s".into();
    }
    if ns < 1_000_000_000 {
        let (unit, div) = if ns < 1_000 {
            ("ns", 1)
        } else if ns < 1_000_000 {
            ("µs", 1_000)
        } else {
            ("ms", 1_000_000)
        };
        return format!("{}{unit}", fraction(ns, div));
    }
    let total = ns / 1_000_000_000;
    let secs = fraction(ns % 60_000_000_000, 1_000_000_000);
    let (h, m) = (total / 3600, total / 60 % 60);
    if h > 0 {
        format!("{h}h{m}m{secs}s")
    } else if m > 0 {
        format!("{m}m{secs}s")
    } else {
        format!("{secs}s")
    }
}

/// `ns / div` with the decimals that are not zero.
fn fraction(ns: u128, div: u128) -> String {
    let (int, rest) = (ns / div, ns % div);
    if rest == 0 {
        return int.to_string();
    }
    let width = div.ilog10() as usize;
    let decimals = format!("{rest:0width$}");
    format!("{int}.{}", decimals.trim_end_matches('0'))
}

/// `d` rounded to the nearest whole second.
pub fn round_secs(d: Duration) -> Duration {
    Duration::from_secs(d.as_secs() + u64::from(d.subsec_millis() >= 500))
}

/// The name of this machine, "" if it can't be read.
pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for its whole length and gethostname
    // writes at most that many bytes.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// Locks `m` even if a thread panicked while holding it: the data in this
/// program stays consistent whatever a panicking thread was doing.
pub fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Reads `l` even if a thread panicked while writing to it.
pub fn lock_rw_read<T>(l: &std::sync::RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_like_go() {
        let s = Duration::from_secs;
        for (d, want) in [
            (Duration::ZERO, "0s"),
            (s(30), "30s"),
            (s(60), "1m0s"),
            (s(600), "10m0s"),
            (s(3723), "1h2m3s"),
            (Duration::from_millis(12_500), "12.5s"),
            (Duration::from_millis(250), "250ms"),
            (Duration::from_micros(1500), "1.5ms"),
            (Duration::from_secs(3600), "1h0m0s"),
            (Duration::from_millis(61_250), "1m1.25s"),
        ] {
            assert_eq!(go_duration(d), want);
        }
    }

    #[test]
    fn rounds_to_the_second() {
        assert_eq!(
            round_secs(Duration::from_millis(1499)),
            Duration::from_secs(1)
        );
        assert_eq!(
            round_secs(Duration::from_millis(1500)),
            Duration::from_secs(2)
        );
    }

    #[test]
    fn the_hostname_is_known() {
        assert!(!hostname().is_empty());
    }
}
