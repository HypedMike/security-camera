//! The GIFs saved on disk under a code.
//!
//! A `Store` keeps its files in one directory, named `<code>.gif`, and keeps
//! their total size at or below a limit by deleting the oldest ones.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

const EXT: &str = ".gif";

#[derive(Debug)]
pub enum Error {
    /// The code is not six hex digits.
    InvalidCode,
    /// No GIF is saved under the code (never saved, or deleted to make room).
    NotFound,
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::InvalidCode => f.write_str("gif: invalid code"),
            Error::NotFound => f.write_str("gif: no such gif"),
            Error::Io(e) => e.fmt(f),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// Whether `code` looks like what the program generates: six lowercase hex
/// digits. It also keeps a user supplied code from naming any other file.
pub fn valid_code(code: &str) -> bool {
    code.len() == 6 && code.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

pub struct Store {
    root: PathBuf,
    max: u64,
    lock: Mutex<()>,
}

impl Store {
    /// Opens the store in directory `root`, creating it if needed. `max` is
    /// the largest total size of the GIFs in bytes.
    pub fn new(root: impl Into<PathBuf>, max: u64) -> io::Result<Store> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Store { root, max, lock: Mutex::new(()) })
    }

    fn path(&self, code: &str) -> PathBuf {
        self.root.join(format!("{code}{EXT}"))
    }

    /// Stores `data` under `code`, then deletes the oldest GIFs until the
    /// total size is within the limit. A GIF bigger than the limit is deleted
    /// at once.
    pub fn save(&self, code: &str, data: &[u8]) -> Result<(), Error> {
        if !valid_code(code) {
            return Err(Error::InvalidCode);
        }
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        write_atomic(&self.root, &self.path(code), data)?;
        self.evict()?;
        Ok(())
    }

    /// Returns the GIF saved under `code`, in any letter case.
    pub fn get(&self, code: &str) -> Result<Vec<u8>, Error> {
        let code = code.to_ascii_lowercase();
        if !valid_code(&code) {
            return Err(Error::NotFound);
        }
        match fs::read(self.path(&code)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(Error::NotFound),
            r => Ok(r?),
        }
    }

    /// Deletes the oldest GIFs until the total size is within `max`.
    fn evict(&self) -> io::Result<()> {
        let mut files: Vec<(SystemTime, OsString, u64)> = Vec::new();
        let mut total = 0u64;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            if !name.to_string_lossy().ends_with(EXT) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                continue;
            }
            let Ok(modified) = meta.modified() else { continue };
            total += meta.len();
            files.push((modified, name, meta.len()));
        }
        files.sort();
        for (_, name, size) in files {
            if total <= self.max {
                break;
            }
            match fs::remove_file(self.root.join(name)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
            total -= size;
        }
        Ok(())
    }
}

/// Writes `data` to a temporary file in `dir` and renames it to `path`, so a
/// reader never sees half a GIF.
fn write_atomic(dir: &Path, path: &Path, data: &[u8]) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let tmp = dir.join(format!(".tmp-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    let result = fs::write(&tmp, data).and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
