use gif::store::{Error, Store, valid_code};
use std::fs::{self, File, FileTimes};
use std::path::Path;
use std::time::{Duration, SystemTime};

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("gifstore-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    d
}

fn age(root: &Path, code: &str, secs_ago: u64) {
    let t = SystemTime::now() - Duration::from_secs(secs_ago);
    File::options().write(true).open(root.join(format!("{code}.gif"))).unwrap()
        .set_times(FileTimes::new().set_modified(t)).unwrap();
}

#[test]
fn save_get() {
    let root = dir("save_get");
    let s = Store::new(&root, 100).unwrap();
    s.save("abc123", b"data").unwrap();
    assert_eq!(s.get("ABC123").unwrap(), b"data");
    assert!(matches!(s.get("ffffff"), Err(Error::NotFound)));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_codes() {
    let root = dir("invalid");
    let s = Store::new(&root, 100).unwrap();
    for code in ["", "../x", "abc12", "abc1234", "ghijkl", "..%2f..", "a/b123", "ABC123"] {
        assert!(!valid_code(code), "{code}");
        assert!(matches!(s.save(code, b""), Err(Error::InvalidCode)), "{code}");
    }
    for code in ["", "../x", "abc12", "abc1234", "ghijkl", "a/b123"] {
        assert!(matches!(s.get(code), Err(Error::NotFound)), "{code}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn evicts_oldest_first() {
    let root = dir("evict");
    let s = Store::new(&root, 25).unwrap();
    for (i, code) in ["000001", "000002", "000003"].iter().enumerate() {
        s.save(code, &[b'x'; 10]).unwrap();
        age(&root, code, 3600 - 60 * i as u64);
    }
    s.save("000004", &[b'x'; 10]).unwrap();
    for (code, present) in [("000001", false), ("000002", false), ("000003", true), ("000004", true)] {
        assert_eq!(s.get(code).is_ok(), present, "{code}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn oversize_gif_is_dropped() {
    let root = dir("oversize");
    let s = Store::new(&root, 5).unwrap();
    s.save("000001", &[0; 10]).unwrap();
    assert!(matches!(s.get("000001"), Err(Error::NotFound)));
    fs::remove_dir_all(root).unwrap();
}
