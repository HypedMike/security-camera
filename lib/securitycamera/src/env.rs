//! Settings from the environment and the `.env` file.
//!
//! The values read from `.env` or typed at the first start are kept here and
//! not put in the process environment: changing that is not safe while other
//! threads run. [`get`] looks at both, the process environment first.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::util::lock;

fn overlay() -> &'static Mutex<HashMap<String, String>> {
    static OVERLAY: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    OVERLAY.get_or_init(Default::default)
}

/// The value of `key`, "" if it is not set.
pub fn get(key: &str) -> String {
    match std::env::var(key) {
        Ok(v) if !v.is_empty() => v,
        _ => lock(overlay()).get(key).cloned().unwrap_or_default(),
    }
}

/// Sets `key` for the rest of the run, as if it was in `.env`.
pub fn set(key: &str, value: &str) {
    lock(overlay()).insert(key.to_owned(), value.to_owned());
}

/// Whether the variable is set to a true value (1, true, ...).
pub fn bool(key: &str) -> bool {
    parse_bool(&get(key))
}

fn parse_bool(v: &str) -> bool {
    matches!(v, "1" | "t" | "T" | "TRUE" | "true" | "True")
}

/// Reads KEY=VALUE lines from `path`. Blank lines and lines starting with #
/// are ignored, and variables that already have a non-empty value are not
/// overridden. A missing file is not an error.
pub fn load(path: impl AsRef<Path>) -> io::Result<()> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        if get(key).is_empty() {
            set(key, value);
        }
    }
    Ok(())
}

/// Rejects invalid input: the user is asked again.
pub type Validate = fn(&str) -> Result<(), String>;

/// A required variable that [`ensure`] asks for when it is missing.
pub struct Var<'a> {
    pub key: &'a str,
    pub prompt: &'a str,
    pub validate: Option<Validate>,
}

/// Asks on `out`/`input` for every variable that is unset or empty, then sets
/// it and saves it in the file at `path` (created with mode 0600, since it
/// holds secrets) so it is found on the next start.
pub fn ensure(
    path: impl AsRef<Path>,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    vars: &[Var],
) -> Result<(), String> {
    for v in vars {
        if !get(v.key).is_empty() {
            continue;
        }
        let value = loop {
            let _ = write!(out, "{} ({}): ", v.prompt, v.key);
            let _ = out.flush();
            let mut line = String::new();
            let read = input.read_line(&mut line);
            let value = line.trim().to_owned();
            if value.is_empty() {
                match read {
                    Ok(n) if n > 0 => continue,
                    Ok(_) => {
                        return Err(format!(
                            "env: {} is not set and could not be read: end of input",
                            v.key
                        ));
                    }
                    Err(e) => {
                        return Err(format!(
                            "env: {} is not set and could not be read: {e}",
                            v.key
                        ));
                    }
                }
            }
            if let Some(validate) = v.validate
                && let Err(e) = validate(&value)
            {
                let _ = writeln!(out, "invalid: {e}");
                continue;
            }
            break value;
        };
        save(path.as_ref(), v.key, &value).map_err(|e| e.to_string())?;
        set(v.key, &value);
    }
    Ok(())
}

/// Sets key=value in the file at `path`: lines already defining `key` are
/// replaced, otherwise a line is appended. The file is created with mode 0600.
pub fn save(path: impl AsRef<Path>, key: &str, value: &str) -> io::Result<()> {
    let path = path.as_ref();
    let data = match fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let line = format!("{key}={value}");
    let mut found = false;
    let mut out: Vec<String> = Vec::new();
    for l in data.trim_end_matches('\n').lines() {
        if let Some((k, _)) = l.trim().split_once('=')
            && k.trim() == key
        {
            if !found {
                out.push(line.clone());
            }
            found = true;
            continue;
        }
        out.push(l.to_owned());
    }
    if !found {
        out.push(line);
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all((out.join("\n") + "\n").as_bytes())
}

/// Lists `options` on `out` and reads the number of one from `input`. If
/// nothing is entered within `timeout` (or the input is closed), the first
/// option is returned. Invalid entries ask again; the timeout restarts only
/// with the whole call, not per entry.
pub fn choose(
    input: impl Read + Send + 'static,
    out: &mut dyn Write,
    prompt: &str,
    options: &[String],
    timeout: Duration,
) -> usize {
    let _ = writeln!(out, "{prompt}");
    for (i, o) in options.iter().enumerate() {
        let _ = writeln!(out, "  {}) {o}", i + 1);
    }
    let (tx, rx) = mpsc::channel::<String>();
    // may outlive the call if the timeout fires; it only blocks on the read
    thread::spawn(move || {
        for line in BufReader::new(input).lines() {
            let Ok(line) = line else { return };
            if !line.trim().is_empty() && tx.send(line.trim().to_owned()).is_err() {
                return;
            }
        }
    });
    let deadline = Instant::now() + timeout;
    loop {
        let _ = write!(
            out,
            "Choose 1-{} [1 in {}s]: ",
            options.len(),
            timeout.as_secs()
        );
        let _ = out.flush();
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => match line.parse::<usize>() {
                Ok(n) if (1..=options.len()).contains(&n) => return n - 1,
                _ => {
                    let _ = writeln!(out, "invalid choice");
                }
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = writeln!(out);
                return 0;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = writeln!(out, "\nno answer, using {}", options[0]);
                return 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::os::unix::fs::PermissionsExt;

    fn dir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "sc-env-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn bool_values() {
        for (value, want) in [
            ("1", true),
            ("true", true),
            ("TRUE", true),
            ("0", false),
            ("false", false),
            ("", false),
            ("nonsense", false),
        ] {
            assert_eq!(parse_bool(value), want, "{value:?}");
        }
        set("ENV_BOOL_TEST", "true");
        assert!(bool("ENV_BOOL_TEST"));
        assert!(!bool("ENV_BOOL_NOT_SET"));
    }

    #[test]
    fn load_reads_lines_and_keeps_what_is_set() {
        let path = dir().join(".env");
        fs::write(
            &path,
            "# comment\n\nLOAD_A=1\nLOAD_B = \"two\"\nnot a line\nLOAD_C='x'\nLOAD_SET=file\n",
        )
        .unwrap();
        set("LOAD_SET", "already");
        load(&path).unwrap();
        assert_eq!(
            (
                get("LOAD_A").as_str(),
                get("LOAD_B").as_str(),
                get("LOAD_C").as_str()
            ),
            ("1", "two", "x")
        );
        assert_eq!(get("LOAD_SET"), "already");
        assert!(load(dir().join("missing")).is_ok());
    }

    #[test]
    fn choose_picks_a_valid_number() {
        let opts: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        let mut out = Vec::new();
        assert_eq!(
            choose(
                Cursor::new("x\n9\n2\n"),
                &mut out,
                "p",
                &opts,
                Duration::from_secs(1)
            ),
            1
        );
        assert_eq!(
            choose(
                Cursor::new(""),
                &mut out,
                "p",
                &opts,
                Duration::from_secs(1)
            ),
            0
        );
    }

    #[test]
    fn choose_times_out_with_the_first() {
        let opts: Vec<String> = ["a", "b"].map(String::from).to_vec();
        let (reader, _writer) = io::pipe().unwrap(); // never written to
        let start = Instant::now();
        assert_eq!(
            choose(
                reader,
                &mut Vec::new(),
                "p",
                &opts,
                Duration::from_millis(50)
            ),
            0
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn save_replaces_and_appends() {
        let path = dir().join(".env");
        fs::write(&path, "# c\nA=1\nB=2\nA=3\n").unwrap();
        save(&path, "A", "9").unwrap();
        save(&path, "C", "4").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "# c\nA=9\nB=2\nC=4\n");
    }

    #[test]
    fn ensure_asks_for_what_is_missing() {
        set("ENSURE_SET", "already");
        let path = dir().join(".env");
        fn num(s: &str) -> Result<(), String> {
            s.parse::<f64>().map(|_| ()).map_err(|e| e.to_string())
        }
        let mut input = Cursor::new("tok\nabc\n0.05\n");
        ensure(
            &path,
            &mut input,
            &mut Vec::new(),
            &[
                Var {
                    key: "ENSURE_SET",
                    prompt: "set",
                    validate: None,
                },
                Var {
                    key: "ENSURE_A",
                    prompt: "a",
                    validate: None,
                },
                Var {
                    key: "ENSURE_B",
                    prompt: "b",
                    validate: Some(num),
                },
            ],
        )
        .unwrap();
        // "tok" fills A, then "abc" is rejected and "0.05" fills B
        assert_eq!(
            (get("ENSURE_A").as_str(), get("ENSURE_B").as_str()),
            ("tok", "0.05")
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "ENSURE_A=tok\nENSURE_B=0.05\n"
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        // no input left: an error instead of looping forever
        let r = ensure(
            &path,
            &mut Cursor::new(""),
            &mut Vec::new(),
            &[Var {
                key: "ENSURE_C",
                prompt: "c",
                validate: None,
            }],
        );
        assert!(r.is_err());
    }
}
