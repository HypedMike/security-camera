//! The settings `/edit_config` shows and changes, and the file they are saved in.

use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{Live, Values};
use crate::detector::{NAME_BACKGROUND, NAME_DIFFERENCE};
use crate::monitor::Monitor;
use crate::stream::Stream;

// The longest alarm GIF and the most frames a second it can have: the frames
// of a GIF wait in memory until it is encoded.
const MAX_ALARM_GIF_SECONDS: f64 = 30.0;
const MAX_ALARM_GIF_FPS: i64 = 10;

/// The JSON `/edit_config` sends and accepts. Times are in seconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub detector: String,
    pub background_threshold: f64,
    pub difference_threshold: f64,
    pub paused: bool,

    pub stack_frames: i64,
    pub detection_frames: i64,

    pub alarm_cooldown_seconds: f64,

    pub alarm_gif_seconds: f64,
    pub alarm_gif_fps: i64,
    pub hd_gif_width: i64,
    pub hd_gif_dither: bool,

    pub snap_jpeg_quality: i64,
}

impl Settings {
    /// The settings the program is running with.
    pub fn current(mon: &Monitor, stream: &Stream, live: &Live) -> Settings {
        let v = live.get();
        Settings {
            detector: mon.detector(),
            background_threshold: mon.threshold_of(NAME_BACKGROUND),
            difference_threshold: mon.threshold_of(NAME_DIFFERENCE),
            paused: mon.paused(),

            stack_frames: stream.history() as i64,
            detection_frames: mon.detection_frames() as i64,

            alarm_cooldown_seconds: mon.cooldown().as_secs_f64(),

            alarm_gif_seconds: v.alarm_gif_span.as_secs_f64(),
            alarm_gif_fps: i64::from(v.alarm_gif_fps),
            hd_gif_width: v.hd_gif_width as i64,
            hd_gif_dither: v.hd_gif_dither,

            snap_jpeg_quality: i64::from(v.snap_jpeg_quality),
        }
    }

    /// Why these settings can't be used, if they can't. `detectors` are the
    /// names of the known motion detectors.
    pub fn validate(&self, detectors: &[&str]) -> Result<(), String> {
        if !detectors.contains(&self.detector.as_str()) {
            return Err(format!("detector: unknown detector {:?}, use one of: {}", self.detector, detectors.join(", ")));
        }
        for (name, v) in [("background_threshold", self.background_threshold), ("difference_threshold", self.difference_threshold)] {
            if !(0.0..=1.0).contains(&v) {
                return Err(format!("{name}: must be between 0 and 1"));
            }
        }
        if self.detection_frames < 2 {
            return Err("detection_frames: must be at least 2".into());
        }
        if self.stack_frames < self.detection_frames {
            return Err(format!("stack_frames: must be at least {}, detection_frames", self.detection_frames));
        }
        if !self.alarm_cooldown_seconds.is_finite() || self.alarm_cooldown_seconds < 0.0 {
            return Err("alarm_cooldown_seconds: must be 0 or more".into());
        }
        if !(0.5..=MAX_ALARM_GIF_SECONDS).contains(&self.alarm_gif_seconds) {
            return Err(format!("alarm_gif_seconds: must be between 0.5 and {MAX_ALARM_GIF_SECONDS}"));
        }
        for (name, v) in [("alarm_gif_fps", self.alarm_gif_fps), ("hd_gif_width", self.hd_gif_width)] {
            if v < 1 {
                return Err(format!("{name}: must be at least 1"));
            }
        }
        if self.alarm_gif_fps > MAX_ALARM_GIF_FPS {
            return Err(format!("alarm_gif_fps: must be at most {MAX_ALARM_GIF_FPS}"));
        }
        if !(1..=100).contains(&self.snap_jpeg_quality) {
            return Err("snap_jpeg_quality: must be between 1 and 100".into());
        }
        Ok(())
    }

    /// The settings as indented JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("settings are plain data")
    }
}

/// Changes the settings to the JSON in `payload`. Keys it leaves out keep
/// their value. Nothing is changed unless all of it is valid.
pub fn apply(payload: &str, mon: &Monitor, stream: &Stream, live: &Live, detectors: &[&str]) -> Result<(), String> {
    let not_valid = |why: &dyn std::fmt::Display| format!("not valid settings JSON: {why}");
    let Value::Object(given) = serde_json::from_str::<Value>(payload).map_err(|e| not_valid(&e))? else {
        return Err(not_valid(&"the settings must be a JSON object"));
    };
    // what is left out keeps its value: start from the current settings
    let Value::Object(mut merged) = serde_json::to_value(Settings::current(mon, stream, live)).expect("settings are plain data") else {
        unreachable!("settings are an object")
    };
    for (key, value) in given {
        if !merged.contains_key(&key) {
            let known: Vec<&str> = merged.keys().map(String::as_str).collect();
            return Err(format!("not valid settings JSON: unknown field {key:?}, use one of: {}", known.join(", ")));
        }
        merged.insert(key, value);
    }
    let mut c: Settings = serde_json::from_value(Value::Object(merged)).map_err(|e| not_valid(&e))?;
    c.detector = c.detector.to_lowercase();
    c.validate(detectors)?;

    mon.set_threshold_of(NAME_BACKGROUND, c.background_threshold);
    mon.set_threshold_of(NAME_DIFFERENCE, c.difference_threshold);
    mon.set_detector(&c.detector);
    mon.set_paused(c.paused);
    mon.set_cooldown(seconds(c.alarm_cooldown_seconds));
    stream.set_history(c.stack_frames as usize);
    mon.set_detection_frames(c.detection_frames as usize);
    live.set(Values {
        alarm_gif_span: seconds(c.alarm_gif_seconds),
        alarm_gif_fps: c.alarm_gif_fps as u32,
        hd_gif_width: c.hd_gif_width as usize,
        hd_gif_dither: c.hd_gif_dither,
        snap_jpeg_quality: c.snap_jpeg_quality as u8,
    });
    Ok(())
}

fn seconds(s: f64) -> Duration {
    Duration::from_secs_f64(s)
}

/// Writes the settings to the JSON file at `path`, so they survive a
/// restart. "paused" is left out: a camera that was paused must not come back
/// paused after a reboot, where nobody would notice.
pub fn save(path: &str, c: &Settings) -> io::Result<()> {
    let Value::Object(mut m) = serde_json::to_value(c).expect("settings are plain data") else { unreachable!("settings are an object") };
    m.shift_remove("paused");
    let json = serde_json::to_string_pretty(&m).expect("a map of plain data");
    // write a temporary file and rename it, so a crash can't leave half a file
    let tmp = format!("{path}.tmp");
    fs::write(&tmp, json + "\n")?;
    fs::rename(&tmp, path)
}

/// Applies the JSON file at `path` over the current settings. Returns whether
/// the file exists; a missing file is not an error. An invalid file changes
/// nothing.
pub fn load_saved(path: &str, mon: &Monitor, stream: &Stream, live: &Live, detectors: &[&str]) -> Result<bool, String> {
    let data = match fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    apply(&data, mon, stream, live, detectors).map_err(|e| format!("{path}: {e}"))?;
    Ok(true)
}

/// Changes the settings back to `defaults` and deletes the JSON file at
/// `path`. The paused state is not a saved setting, so it stays as it is.
pub fn reset(path: &str, mut defaults: Settings, mon: &Monitor, stream: &Stream, live: &Live, detectors: &[&str]) -> Result<(), String> {
    defaults.paused = mon.paused();
    apply(&defaults.to_json(), mon, stream, live, detectors)?;
    match fs::remove_file(Path::new(path)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(format!("settings are back to the defaults but {path} could not be deleted: {e}")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    const NAMES: &[&str] = &[NAME_BACKGROUND, NAME_DIFFERENCE];

    fn settings() -> (Monitor, Stream, Live) {
        let mon = Monitor::new(
            "",
            NAME_BACKGROUND,
            &[(NAME_BACKGROUND, config::BACKGROUND_THRESHOLD), (NAME_DIFFERENCE, config::DIFFERENCE_THRESHOLD)],
            config::ALARM_COOLDOWN,
        );
        mon.set_detection_frames(config::DETECTION_FRAMES);
        (mon, Stream::new(config::STREAM_HISTORY), Live::new())
    }

    fn json(s: &(Monitor, Stream, Live)) -> String {
        Settings::current(&s.0, &s.1, &s.2).to_json()
    }

    fn apply_to(s: &(Monitor, Stream, Live), payload: &str) -> Result<(), String> {
        apply(payload, &s.0, &s.1, &s.2, NAMES)
    }

    fn temp_path(name: &str) -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("sc-settings-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir_all(&dir).unwrap();
        dir.join(name).to_string_lossy().into_owned()
    }

    #[test]
    fn sending_the_json_back_changes_nothing() {
        let s = settings();
        let before = json(&s);
        apply_to(&s, &before).unwrap();
        assert_eq!(json(&s), before);
    }

    #[test]
    fn applies_what_it_is_given() {
        let s = settings();
        apply_to(
            &s,
            r#"{"detector": "Difference", "difference_threshold": 0.2, "alarm_cooldown_seconds": 12.5,
                "alarm_gif_seconds": 8, "alarm_gif_fps": 4, "paused": true, "stack_frames": 400, "hd_gif_dither": false, "snap_jpeg_quality": 50}"#,
        )
        .unwrap();
        let (mon, stream, live) = &s;
        assert_eq!(mon.detector(), NAME_DIFFERENCE);
        assert_eq!(mon.threshold_of(NAME_DIFFERENCE), 0.2);
        assert!(mon.paused());
        assert_eq!(mon.cooldown(), Duration::from_millis(12_500));
        assert_eq!(stream.history(), 400);
        let v = live.get();
        assert_eq!(
            v,
            Values { alarm_gif_span: Duration::from_secs(8), alarm_gif_fps: 4, hd_gif_width: config::HD_GIF_WIDTH, hd_gif_dither: false, snap_jpeg_quality: 50 }
        );
        // what was left out kept its value
        assert_eq!(mon.threshold_of(NAME_BACKGROUND), config::BACKGROUND_THRESHOLD);
        assert_eq!(mon.detection_frames(), config::DETECTION_FRAMES);
    }

    #[test]
    fn rejects_without_changing_anything() {
        for (payload, want) in [
            (r#"{"detector": "magic"}"#, "unknown detector"),
            (r#"{"background_threshold": 2}"#, "background_threshold"),
            (r#"{"difference_threshold": -0.1}"#, "difference_threshold"),
            (r#"{"detection_frames": 1}"#, "detection_frames"),
            (r#"{"stack_frames": 10}"#, "stack_frames"),
            (r#"{"alarm_cooldown_seconds": -1}"#, "alarm_cooldown_seconds"),
            (r#"{"hd_gif_width": 0}"#, "hd_gif_width"),
            (r#"{"snap_jpeg_quality": 101}"#, "snap_jpeg_quality"),
            (r#"{"no_such_setting": 1}"#, "unknown field"),
            (r#"{"alarm_gif_seconds": "x"}"#, "not valid settings JSON"),
            (r#"{"alarm_gif_seconds": 0}"#, "alarm_gif_seconds"),
            (r#"{"alarm_gif_seconds": 31}"#, "alarm_gif_seconds"),
            (r#"{"alarm_gif_fps": 0}"#, "alarm_gif_fps"),
            (r#"{"alarm_gif_fps": 99}"#, "alarm_gif_fps"),
            (r#"{"alarm_gif_fps": 2.5}"#, "not valid settings JSON"),
            (r#"{"paused": true} trailing"#, "not valid settings JSON"),
            (r#"[1, 2]"#, "not valid settings JSON"),
            ("not json", "not valid settings JSON"),
            (r#"{"paused": true, "snap_jpeg_quality": 0}"#, "snap_jpeg_quality"),
        ] {
            let s = settings();
            let before = json(&s);
            let err = apply_to(&s, payload).expect_err(payload);
            assert!(err.contains(want), "{payload}: error {err:?}, want one with {want:?}");
            assert_eq!(json(&s), before, "{payload}: settings changed despite the error");
        }
    }

    #[test]
    fn the_json_is_valid_and_readable() {
        let s = settings();
        let m: Value = serde_json::from_str(&json(&s)).unwrap();
        assert_eq!(m["detector"], "background");
        assert_eq!(m["alarm_cooldown_seconds"], 5.0);
        assert!(json(&s).starts_with("{\n  \"detector\""), "indented, detector first");
    }

    #[test]
    fn saved_settings_survive_a_restart() {
        let path = temp_path("settings.json");
        let before = settings();
        apply_to(&before, r#"{"detector": "difference", "alarm_gif_seconds": 8, "snap_jpeg_quality": 50, "paused": true}"#).unwrap();
        save(&path, &Settings::current(&before.0, &before.1, &before.2)).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("paused"), "paused is not saved");
        assert!(!Path::new(&format!("{path}.tmp")).exists(), "the temporary file is gone");

        // a new run starts from the defaults and loads the file over them
        let after = settings();
        assert_eq!(load_saved(&path, &after.0, &after.1, &after.2, NAMES), Ok(true));
        assert_eq!(after.0.detector(), NAME_DIFFERENCE);
        assert_eq!(after.2.get().alarm_gif_span, Duration::from_secs(8));
        assert_eq!(after.2.get().snap_jpeg_quality, 50);
        assert!(!after.0.paused(), "the camera came back paused: paused must not be saved");
    }

    #[test]
    fn a_missing_or_invalid_file() {
        let s = settings();
        let before = json(&s);
        assert_eq!(load_saved(&temp_path("none.json"), &s.0, &s.1, &s.2, NAMES), Ok(false));

        let bad = temp_path("bad.json");
        fs::write(&bad, r#"{"snap_jpeg_quality": 0}"#).unwrap();
        let err = load_saved(&bad, &s.0, &s.1, &s.2, NAMES).unwrap_err();
        assert!(err.contains("bad.json") && err.contains("snap_jpeg_quality"), "{err}");
        assert_eq!(json(&s), before, "an invalid file changed the settings");
    }

    #[test]
    fn reset_goes_back_to_the_defaults_and_deletes_the_file() {
        let path = temp_path("settings.json");
        let s = settings();
        let defaults = Settings::current(&s.0, &s.1, &s.2);
        apply_to(&s, r#"{"detector": "difference", "stack_frames": 400, "alarm_gif_fps": 4, "paused": true}"#).unwrap();
        save(&path, &Settings::current(&s.0, &s.1, &s.2)).unwrap();
        reset(&path, defaults.clone(), &s.0, &s.1, &s.2, NAMES).unwrap();

        let want = Settings { paused: true, ..defaults.clone() }; // /reset_config doesn't unpause
        assert_eq!(Settings::current(&s.0, &s.1, &s.2), want);
        assert!(!Path::new(&path).exists(), "the settings file is still there");
        // resetting with no file is fine
        reset(&path, defaults, &s.0, &s.1, &s.2, NAMES).unwrap();
    }

    #[test]
    fn detector_names_are_case_insensitive_and_checked() {
        let s = settings();
        apply_to(&s, r#"{"detector": "BACKGROUND"}"#).unwrap();
        assert_eq!(s.0.detector(), NAME_BACKGROUND);
        let err = apply_to(&s, r#"{"detector": "x"}"#).unwrap_err();
        assert!(err.contains("background, difference"), "{err}");
    }

    #[test]
    fn the_defaults_are_valid() {
        let s = settings();
        Settings::current(&s.0, &s.1, &s.2).validate(NAMES).unwrap();
    }
}
