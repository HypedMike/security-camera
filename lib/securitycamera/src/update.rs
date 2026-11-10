//! Pulls and rebuilds the program, and restarts it in place.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Runs the update script and returns its combined output and whether it
/// worked. The script must pull and build; a non-zero exit is reported as an
/// error and leaves the running program untouched.
pub fn build(script: &str, timeout: Duration) -> (String, Result<(), String>) {
    // the script's output and error go to the same pipe, in the order they happen
    let mut child = match Command::new("sh")
        .args(["-c", "exec \"$0\" 2>&1", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // its own process group, to kill git and cargo too on a timeout
        .process_group(0)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return (String::new(), Err(format!("{script}: {e}"))),
    };
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let reader = thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    });

    let deadline = Instant::now() + timeout;
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(status)) => break Err(status.to_string()),
            Ok(None) if Instant::now() >= deadline => {
                // SAFETY: killing a process group we created; the pid is our child's.
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
                let _ = child.wait();
                break Err(format!("timed out after {}s", timeout.as_secs_f32()));
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(e) => break Err(e.to_string()),
        }
    };
    (reader.join().unwrap_or_default().trim().to_owned(), result)
}

/// At most `max` bytes from the end of `s`, which is where the outcome of a
/// long build log is.
pub fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("...{}", &s[start..])
}

/// The path of the running binary, even after a rebuild replaced it on disk
/// (Linux then marks the old path as " (deleted)").
fn executable() -> std::io::Result<std::path::PathBuf> {
    let exe = std::env::current_exe()?;
    Ok(match exe.to_string_lossy().strip_suffix(" (deleted)") {
        Some(path) => path.into(),
        None => exe,
    })
}

/// Replaces the running process with a fresh copy of the (rebuilt) binary,
/// same arguments and environment. It only returns on failure. Close the
/// camera first: the new process needs to open it again.
pub fn restart() -> std::io::Error {
    match executable() {
        Ok(exe) => Command::new(exe).args(std::env::args_os().skip(1)).exec(),
        Err(e) => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn script(body: &str) -> String {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "sc-update-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("update");
        fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn build_returns_the_output() {
        let (out, r) = build(&script("echo 'updated a -> b'\n"), MINUTE);
        assert_eq!((out.as_str(), r), ("updated a -> b", Ok(())));
    }

    #[test]
    fn build_failure_keeps_the_output() {
        let (out, r) = build(&script("echo boom >&2\nexit 1\n"), MINUTE);
        assert!(out.contains("boom") && r.is_err(), "{out:?} {r:?}");
    }

    #[test]
    fn build_output_is_in_order() {
        let (out, _) = build(&script("echo one\necho two >&2\necho three\n"), MINUTE);
        assert_eq!(out, "one\ntwo\nthree");
    }

    #[test]
    fn a_slow_script_is_stopped_with_its_children() {
        let start = Instant::now();
        let (_, r) = build(
            &script("sleep 30 &\nsleep 30\n"),
            Duration::from_millis(100),
        );
        assert!(r.unwrap_err().contains("timed out"));
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "waited for the children: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn a_missing_script_is_an_error() {
        let (_, r) = build("/no/such/script", MINUTE);
        assert!(r.is_err());
    }

    #[test]
    fn tail_keeps_the_end() {
        assert_eq!(tail("abcdef", 3), "...def");
        assert_eq!(tail("abc", 3), "abc");
        assert_eq!(tail("aé", 2), "...é", "never cuts a character in half");
    }
}
