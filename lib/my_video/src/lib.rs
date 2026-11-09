//! The camera: listing the cameras, choosing one and reading its frames.
//!
//! Capture goes through [nokhwa](https://crates.io/crates/nokhwa), which talks
//! to V4L2 on Linux and AVFoundation on macOS. A thread owns the camera and
//! keeps the latest frame; [`Capture::read`] waits for a frame that was not
//! read yet. Everything that does not touch the hardware (the order cameras
//! are tried in, finding the saved one, the capture thread) is generic and
//! tested with a fake camera.

mod camera;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

/// An available camera.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// What [`open`] takes. It changes between runs: use the label to remember a camera.
    pub id: String,
    pub label: String,
}

/// Lists the cameras currently available, sorted by label (the drivers hand
/// them out in an order that is not always the same).
pub fn devices() -> Vec<Device> {
    let mut devs = camera::devices();
    sort_devices(&mut devs);
    devs
}

pub fn sort_devices(devs: &mut [Device]) {
    devs.sort_by(|a, b| a.label.cmp(&b.label));
}

/// Returns the camera `saved` names: by label, or by id (which only matches
/// within the run that saved it).
pub fn find<'a>(devs: &'a [Device], saved: &str) -> Option<&'a Device> {
    devs.iter().find(|d| d.label == saved || d.id == saved)
}

/// Returns the ids to try: `first`, then the others in the order of `devs`.
/// With no camera listed it is just `first` (maybe empty, the default
/// camera), so the driver reports its own error.
pub fn open_order(devs: &[Device], first: &str) -> Vec<String> {
    let mut order = vec![first.to_string()];
    order.extend(devs.iter().filter(|d| d.id != first).map(|d| d.id.clone()));
    order
}

pub struct Config {
    pub width: u32,
    pub height: u32,
    /// Frames read and thrown away on open, so auto-exposure can settle.
    pub warmup_frames: usize,
}

/// Opens the camera `first`. A camera often shows up as several video nodes of
/// which only one delivers images, so if that one can't be opened the other
/// cameras are tried in turn, and a wrong choice does not stop the program. If
/// none opens, it returns the first error.
pub fn open_any(cfg: &Config, first: &str) -> Result<Capture, String> {
    open_any_with(&devices(), first, |id| {
        let (id, w, h) = (id.to_string(), cfg.width, cfg.height);
        Capture::start(cfg.warmup_frames, move || camera::open(&id, w, h))
    })
}

fn open_any_with(
    devs: &[Device],
    first: &str,
    mut open: impl FnMut(&str) -> Result<Capture, String>,
) -> Result<Capture, String> {
    let mut first_err = None;
    for id in open_order(devs, first) {
        match open(&id) {
            Ok(c) => return Ok(c),
            Err(e) => {
                let label = devs.iter().find(|d| d.id == id).map_or("", |d| d.label.as_str());
                eprintln!("camera {:?} can't be opened: {e}", describe(label, &id));
                first_err.get_or_insert(e);
            }
        }
    }
    Err(first_err.unwrap_or_else(|| "my_video: no camera".into()))
}

fn describe<'a>(label: &'a str, id: &'a str) -> &'a str {
    match (label, id) {
        ("", "") => "default",
        ("", id) => id,
        (label, _) => label,
    }
}

/// An RGBA image, `width * height * 4` bytes.
#[derive(Default, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Where the frames come from, only used by the capture thread.
pub trait Source {
    /// Waits for the next frame and writes it into `frame`.
    fn read(&mut self, frame: &mut Frame) -> Result<(), String>;
}

struct Slot {
    frame: Frame,
    /// How many frames were stored; the reader remembers the last it took.
    seq: u64,
    /// Why the capture stopped.
    error: Option<String>,
}

struct Shared {
    slot: Mutex<Slot>,
    ready: Condvar,
    stop: AtomicBool,
}

/// A running capture. All methods work from any thread, `stop` also while
/// another thread waits in `read`.
pub struct Capture {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
    last_seq: Mutex<u64>,
}

impl Capture {
    /// Runs `open` in a new thread, which keeps the camera from then on (some
    /// camera handles can't move between threads), reads and drops
    /// `warmup_frames`, and returns once the first real frame can be read.
    pub fn start<F>(warmup_frames: usize, open: F) -> Result<Capture, String>
    where
        F: FnOnce() -> Result<Box<dyn Source>, String> + Send + 'static,
    {
        let shared = Arc::new(Shared {
            slot: Mutex::new(Slot { frame: Frame::default(), seq: 0, error: None }),
            ready: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let (opened_tx, opened_rx) = mpsc::channel();
        let thread = {
            let shared = shared.clone();
            thread::spawn(move || run(&shared, warmup_frames, open, opened_tx))
        };
        let capture = Capture { shared, thread: Mutex::new(Some(thread)), last_seq: Mutex::new(0) };
        match opened_rx.recv() {
            Ok(Ok(())) => Ok(capture),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("my_video: the capture thread died".into()),
        }
    }

    /// Waits for a frame newer than the last one read and copies it into
    /// `out`. Frames the caller was too slow for are skipped. Fails once the
    /// capture has stopped, with the reason.
    pub fn read(&self, out: &mut Frame) -> Result<(), String> {
        let mut last = self.last_seq.lock().unwrap();
        let mut slot = self.shared.slot.lock().unwrap();
        loop {
            if self.shared.stop.load(Ordering::SeqCst) {
                return Err("my_video: closed".into());
            }
            if slot.seq != *last {
                break;
            }
            if let Some(e) = &slot.error {
                return Err(e.clone());
            }
            slot = self.shared.ready.wait(slot).unwrap();
        }
        *last = slot.seq;
        out.width = slot.frame.width;
        out.height = slot.frame.height;
        out.pixels.clear();
        out.pixels.extend_from_slice(&slot.frame.pixels);
        Ok(())
    }

    /// Makes a `read` that waits (and every later one) fail. The camera is
    /// released when the capture is dropped.
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        let _slot = self.shared.slot.lock().unwrap();
        self.shared.ready.notify_all();
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
        if let Some(t) = self.thread.lock().unwrap().take() {
            let _ = t.join();
        }
    }
}

fn run<F>(shared: &Shared, warmup: usize, open: F, opened: mpsc::Sender<Result<(), String>>)
where
    F: FnOnce() -> Result<Box<dyn Source>, String>,
{
    let fail = |e: String| {
        let mut slot = shared.slot.lock().unwrap();
        slot.error = Some(e);
        shared.ready.notify_all();
    };
    let mut source = match open() {
        Ok(s) => s,
        Err(e) => {
            let _ = opened.send(Err(e));
            return;
        }
    };
    let mut frame = Frame::default();
    for _ in 0..warmup {
        if let Err(e) = source.read(&mut frame) {
            let _ = opened.send(Err(e));
            return;
        }
    }
    let _ = opened.send(Ok(()));
    while !shared.stop.load(Ordering::SeqCst) {
        if let Err(e) = source.read(&mut frame) {
            fail(e);
            return;
        }
        let mut slot = shared.slot.lock().unwrap();
        std::mem::swap(&mut slot.frame, &mut frame);
        slot.seq += 1;
        shared.ready.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn dev(id: &str, label: &str) -> Device {
        Device { id: id.into(), label: label.into() }
    }

    fn devs() -> Vec<Device> {
        vec![dev("x1", "usb-Cam-video-index0;video0"), dev("x2", "usb-Cam-video-index1;video1")]
    }

    #[test]
    fn find_by_label_or_id() {
        let d = devs();
        assert_eq!(find(&d, "usb-Cam-video-index1;video1").unwrap().id, "x2");
        // the id the driver gave in this run
        assert_eq!(find(&d, "x1").unwrap().label, "usb-Cam-video-index0;video0");
        assert!(find(&d, "usb-Other-video-index0;video0").is_none());
    }

    #[test]
    fn open_order_puts_the_chosen_one_first() {
        let d = devs();
        assert_eq!(open_order(&d, "x2"), ["x2", "x1"]);
        assert_eq!(open_order(&d, ""), ["", "x1", "x2"]);
        // no cameras: the default one must still be tried
        assert_eq!(open_order(&[], ""), [""]);
        let order = open_order(&d, "unknown");
        assert_eq!((order[0].as_str(), order.len()), ("unknown", 3));
    }

    #[test]
    fn devices_are_sorted_by_label() {
        let mut d = vec![dev("1", "b"), dev("2", "a")];
        sort_devices(&mut d);
        assert_eq!(d[0].label, "a");
    }

    /// Frames whose first pixel byte counts up from 1, until `limit`.
    struct Counter {
        n: u8,
        limit: u8,
        delay: Duration,
    }

    impl Source for Counter {
        fn read(&mut self, frame: &mut Frame) -> Result<(), String> {
            thread::sleep(self.delay);
            if self.n >= self.limit {
                return Err("unplugged".into());
            }
            self.n += 1;
            frame.width = 1;
            frame.height = 1;
            frame.pixels = vec![self.n, 0, 0, 255];
            Ok(())
        }
    }

    fn counter(limit: u8, delay_ms: u64) -> impl FnOnce() -> Result<Box<dyn Source>, String> + Send + 'static {
        move || Ok(Box::new(Counter { n: 0, limit, delay: Duration::from_millis(delay_ms) }) as Box<dyn Source>)
    }

    #[test]
    fn warmup_frames_are_dropped() {
        let c = Capture::start(5, counter(255, 1)).unwrap();
        let mut f = Frame::default();
        c.read(&mut f).unwrap();
        assert!(f.pixels[0] > 5, "got frame {}", f.pixels[0]);
        assert_eq!((f.width, f.height, f.pixels.len()), (1, 1, 4));
    }

    #[test]
    fn frames_come_in_order_without_repeats() {
        let c = Capture::start(0, counter(255, 2)).unwrap();
        let mut f = Frame::default();
        let mut prev = 0;
        for _ in 0..10 {
            c.read(&mut f).unwrap();
            assert!(f.pixels[0] > prev, "{} after {prev}", f.pixels[0]);
            prev = f.pixels[0];
        }
    }

    #[test]
    fn open_errors_are_returned() {
        let r = Capture::start(0, || Err("no such camera".into()));
        assert_eq!(r.err().as_deref(), Some("no such camera"));
        let r = Capture::start(3, counter(1, 0));
        assert_eq!(r.err().as_deref(), Some("unplugged"));
    }

    #[test]
    fn a_failing_camera_reports_why_after_the_last_frame() {
        let c = Capture::start(0, counter(3, 0)).unwrap();
        let mut f = Frame::default();
        let err = loop {
            if let Err(e) = c.read(&mut f) {
                break e;
            }
        };
        assert_eq!(err, "unplugged");
    }

    #[test]
    fn stop_wakes_a_reader() {
        let c = Arc::new(Capture::start(0, counter(255, 500)).unwrap());
        let reader = {
            let c = c.clone();
            thread::spawn(move || c.read(&mut Frame::default()))
        };
        thread::sleep(Duration::from_millis(50));
        c.stop();
        assert_eq!(reader.join().unwrap().unwrap_err(), "my_video: closed");
    }

    #[test]
    fn open_any_tries_the_others_and_keeps_the_first_error() {
        let d = devs();
        let mut tried = vec![];
        let r = open_any_with(&d, "x2", |id| {
            tried.push(id.to_string());
            if id == "x1" { Capture::start(0, counter(255, 1)) } else { Err(format!("{id} is broken")) }
        });
        assert!(r.is_ok());
        assert_eq!(tried, ["x2", "x1"]);

        let r = open_any_with(&d, "x2", |id| Err(format!("{id} is broken")));
        assert_eq!(r.err().as_deref(), Some("x2 is broken"));
        let r = open_any_with(&[], "", |_| Err("nothing".into()));
        assert_eq!(r.err().as_deref(), Some("nothing"));
    }
}
