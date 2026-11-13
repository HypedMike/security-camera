//! Counts what the camera loop does so it can be reported.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, TimeDelta};

use crate::ignore::Area;
use crate::util::{go_duration, lock, round_secs};

/// How long the time of an alarm is remembered by [`Monitor::alarms_since`].
const HISTORY_KEPT: TimeDelta = TimeDelta::hours(48);

struct Detectors {
    current: String,
    thresholds: HashMap<String, f64>,
}

#[derive(Default)]
struct Times {
    last_frame: Option<DateTime<Local>>,
    last_alarm: Option<DateTime<Local>>,
    /// the times of the alarms of the last [`HISTORY_KEPT`], oldest first
    history: Vec<DateTime<Local>>,
}

/// Records frames and alarms. It can be shared between threads.
pub struct Monitor {
    camera: String,
    started: Instant,
    detectors: Mutex<Detectors>,
    paused: AtomicBool,
    cooldown_ms: AtomicU64,
    detection_frames: AtomicUsize,
    ignore: Mutex<Vec<Area>>,
    frames: AtomicU64,
    alarms: AtomicU64,
    times: Mutex<Times>,
}

impl Monitor {
    /// Starts a monitor. `camera` is the device id ("" for the default
    /// camera); `detector` is the name of the motion detector in use and
    /// `thresholds` the alarm threshold of each detector to start with; they
    /// and the cooldown are shown in the report.
    pub fn new(
        camera: &str,
        detector: &str,
        thresholds: &[(&str, f64)],
        cooldown: Duration,
    ) -> Monitor {
        Monitor {
            camera: camera.to_owned(),
            started: Instant::now(),
            detectors: Mutex::new(Detectors {
                current: detector.to_owned(),
                thresholds: thresholds.iter().map(|&(n, t)| (n.to_owned(), t)).collect(),
            }),
            paused: AtomicBool::new(false),
            cooldown_ms: AtomicU64::new(cooldown.as_millis() as u64),
            detection_frames: AtomicUsize::new(0),
            ignore: Mutex::new(Vec::new()),
            frames: AtomicU64::new(0),
            alarms: AtomicU64::new(0),
            times: Mutex::new(Times::default()),
        }
    }

    /// The name of the motion detector in use.
    pub fn detector(&self) -> String {
        lock(&self.detectors).current.clone()
    }

    /// Changes the motion detector in use. The caller checks that the name is
    /// a known detector.
    pub fn set_detector(&self, name: &str) {
        lock(&self.detectors).current = name.to_owned();
    }

    /// How many of the latest frames the difference is computed on.
    pub fn detection_frames(&self) -> usize {
        self.detection_frames.load(Ordering::Relaxed)
    }

    pub fn set_detection_frames(&self, n: usize) {
        self.detection_frames.store(n, Ordering::Relaxed);
    }

    /// The areas of the picture where motion does not count.
    pub fn ignore_areas(&self) -> Vec<Area> {
        lock(&self.ignore).clone()
    }

    /// Changes them. The caller checks that they are valid.
    pub fn set_ignore_areas(&self, areas: Vec<Area>) {
        *lock(&self.ignore) = areas;
    }

    /// The motion score at or above which the detector in use sends an alarm.
    pub fn threshold(&self) -> f64 {
        let d = lock(&self.detectors);
        d.thresholds.get(&d.current).copied().unwrap_or(0.0)
    }

    /// The alarm threshold of the named detector.
    pub fn threshold_of(&self, detector: &str) -> f64 {
        lock(&self.detectors)
            .thresholds
            .get(detector)
            .copied()
            .unwrap_or(0.0)
    }

    /// Changes the alarm threshold of the detector in use.
    pub fn set_threshold(&self, t: f64) {
        let mut d = lock(&self.detectors);
        let name = d.current.clone();
        d.thresholds.insert(name, t);
    }

    pub fn set_threshold_of(&self, detector: &str, t: f64) {
        lock(&self.detectors)
            .thresholds
            .insert(detector.to_owned(), t);
    }

    /// The minimum time between two alarms.
    pub fn cooldown(&self) -> Duration {
        Duration::from_millis(self.cooldown_ms.load(Ordering::Relaxed))
    }

    pub fn set_cooldown(&self, d: Duration) {
        self.cooldown_ms
            .store(d.as_millis() as u64, Ordering::Relaxed);
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// Whether the alarms are paused.
    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Records a frame captured at `ts`.
    pub fn frame(&self, ts: DateTime<Local>) {
        self.frames.fetch_add(1, Ordering::Relaxed);
        lock(&self.times).last_frame = Some(ts);
    }

    /// Records an alarm raised at `ts`.
    pub fn alarm(&self, ts: DateTime<Local>) {
        self.alarms.fetch_add(1, Ordering::Relaxed);
        let mut t = lock(&self.times);
        t.last_alarm = Some(ts);
        t.history.push(ts);
        let old = t
            .history
            .iter()
            .take_while(|&&a| ts - a > HISTORY_KEPT)
            .count();
        t.history.drain(..old);
    }

    /// The times of the alarms raised after `t`, oldest first. Only the last
    /// 48 hours are remembered.
    pub fn alarms_since(&self, t: DateTime<Local>) -> Vec<DateTime<Local>> {
        lock(&self.times)
            .history
            .iter()
            .copied()
            .filter(|&a| a > t)
            .collect()
    }

    /// When the last alarm was raised, `None` if none was.
    pub fn last_alarm(&self) -> Option<DateTime<Local>> {
        lock(&self.times).last_alarm
    }

    /// Describes what the camera part of the program has been doing.
    pub fn report(&self) -> String {
        let camera = if self.camera.is_empty() {
            "default"
        } else {
            &self.camera
        };
        let frames = self.frames.load(Ordering::Relaxed);
        let elapsed = self.started.elapsed().as_secs_f64();
        let fps = if elapsed > 0.0 {
            frames as f64 / elapsed
        } else {
            0.0
        };
        let state = if self.paused() { "paused" } else { "active" };
        let (last_frame, last_alarm) = {
            let t = lock(&self.times);
            (t.last_frame, t.last_alarm)
        };
        format!(
            "Camera\n  device: {camera}\n  frames: {frames} ({fps:.1} fps average)\n  last frame: {}\n  alarms sent: {}\n  alarm state: {state}\n  last alarm: {}\n  detector: {}\n  alarm threshold: {}, cooldown {}\n  detection frames: {}",
            ago(last_frame),
            self.alarms.load(Ordering::Relaxed),
            ago(last_alarm),
            self.detector(),
            self.threshold(),
            go_duration(self.cooldown()),
            self.detection_frames(),
        )
    }
}

fn ago(t: Option<DateTime<Local>>) -> String {
    match t {
        None => "never".into(),
        Some(t) => format!(
            "{} ago",
            go_duration(round_secs((Local::now() - t).to_std().unwrap_or_default()))
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: &[(&str, f64)] = &[("difference", 0.05), ("background", 0.003)];

    fn monitor() -> Monitor {
        Monitor::new("", "difference", BOTH, Duration::from_secs(30))
    }

    fn has(report: &str, wants: &[&str]) {
        for w in wants {
            assert!(report.contains(w), "report misses {w:?}:\n{report}");
        }
    }

    #[test]
    fn counts_frames_and_alarms() {
        let m = monitor();
        assert!(m.last_alarm().is_none());
        has(
            &m.report(),
            &[
                "device: default",
                "frames: 0",
                "last frame: never",
                "last alarm: never",
            ],
        );

        let now = Local::now();
        m.frame(now);
        m.frame(now);
        m.alarm(now);
        assert_eq!(m.last_alarm(), Some(now));
        has(
            &m.report(),
            &[
                "frames: 2",
                "alarms sent: 1",
                "threshold: 0.05",
                "cooldown 30s",
                "last alarm: 0s ago",
            ],
        );
    }

    #[test]
    fn pause_and_threshold() {
        let m = monitor();
        assert!(!m.paused() && m.threshold() == 0.05);
        m.set_paused(true);
        m.set_threshold(0.2);
        assert!(m.paused() && m.threshold() == 0.2);
        has(&m.report(), &["alarm state: paused", "threshold: 0.2"]);
        m.set_paused(false);
        assert!(!m.paused());
    }

    #[test]
    fn each_detector_has_its_threshold() {
        let m = monitor();
        m.set_detector("background");
        assert!(m.detector() == "background" && m.threshold() == 0.003);
        m.set_threshold(0.01);
        assert!(m.threshold_of("background") == 0.01 && m.threshold_of("difference") == 0.05);
        has(&m.report(), &["detector: background", "threshold: 0.01"]);
    }

    #[test]
    fn cooldown_and_threshold_of() {
        let m = monitor();
        m.set_cooldown(Duration::from_secs(5));
        m.set_threshold_of("background", 0.01);
        assert_eq!(m.cooldown(), Duration::from_secs(5));
        assert_eq!(m.threshold_of("background"), 0.01);
        assert_eq!(m.threshold(), 0.05);
        m.set_cooldown(Duration::from_millis(12_500));
        has(&m.report(), &["cooldown 12.5s"]);
    }

    #[test]
    fn alarms_since_remembers_two_days() {
        let m = Monitor::new(
            "cam",
            "difference",
            &[("difference", 0.05)],
            Duration::from_secs(1),
        );
        let now = Local::now();
        let h = TimeDelta::hours;
        for ago in [60, 30, 2, 0] {
            m.alarm(now - h(ago));
        }
        assert_eq!(m.alarms_since(now - h(24)).len(), 2);
        // the 60 hour old one was dropped when the newer ones came
        assert_eq!(m.alarms_since(now - h(100)).len(), 3);
        assert!(m.report().contains("device: cam"));
    }

    #[test]
    fn unknown_detectors_have_no_threshold() {
        assert_eq!(monitor().threshold_of("nope"), 0.0);
    }
}
