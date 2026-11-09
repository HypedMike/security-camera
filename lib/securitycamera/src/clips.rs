//! Records a few frames of the camera over a span of time, starting when an
//! alarm goes off, without holding up the stream.
//!
//! The timing (which frame belongs to which clip) is decided by the
//! `recorder` crate. The frames stay here, shrunk so they stay valid after
//! the camera reuses its buffers.

use std::collections::HashMap;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use gif::{Image, RgbaImage};
use my_video::Frame;
use recorder::{Action, Recorder};

use crate::util::lock;

type Done = Box<dyn FnOnce(Vec<RgbaImage>) + Send>;

struct Clip {
    frames: Vec<RgbaImage>,
    done: Done,
}

struct Inner {
    timing: Recorder,
    clips: HashMap<u64, Clip>,
}

/// Records several clips at once, one per [`start`](ClipRecorder::start).
pub struct ClipRecorder {
    /// where the times given to the recorder start from
    epoch: Instant,
    inner: Mutex<Inner>,
}

fn nanos(from: Instant, to: Instant) -> i64 {
    i64::try_from(to.saturating_duration_since(from).as_nanos()).unwrap_or(i64::MAX)
}

/// A copy of the frame that stays valid after the camera reuses its buffer,
/// scaled down to at most `width` pixels wide (0 keeps the size).
pub fn shrink(frame: &Frame, width: usize) -> RgbaImage {
    gif::shrink(&Image::Rgba { data: &frame.pixels, width: frame.width as usize, height: frame.height as usize, stride: frame.width as usize * 4 }, width)
}

impl ClipRecorder {
    pub fn new() -> ClipRecorder {
        ClipRecorder { epoch: Instant::now(), inner: Mutex::new(Inner { timing: Recorder::new(), clips: HashMap::new() }) }
    }

    /// Begins a clip with `frame`, taken at `at`, then one frame every
    /// `interval` until `span` has passed. `done` is then called, in its own
    /// thread, with the frames. [`frame`](ClipRecorder::frame) feeds the
    /// following frames. Frames are shrunk to at most `width` pixels wide.
    pub fn start(&self, at: Instant, frame: &Frame, width: usize, span: Duration, interval: Duration, done: impl FnOnce(Vec<RgbaImage>) + Send + 'static) {
        let first = shrink(frame, width);
        let mut inner = lock(&self.inner);
        let id = inner.timing.start(nanos(self.epoch, at), span.as_nanos() as i64, interval.as_nanos() as i64);
        inner.clips.insert(id, Clip { frames: vec![first], done: Box::new(done) });
    }

    /// Called for every frame of the stream: it keeps the ones the clips being
    /// recorded are waiting for, and finishes the clips whose span is over.
    pub fn frame(&self, frame: &Frame, ts: Instant, width: usize) {
        let mut inner = lock(&self.inner);
        let events: Vec<_> = inner.timing.frame(nanos(self.epoch, ts)).to_vec();
        let mut shrunk = None;
        for e in events {
            match e.action {
                Action::Keep => {
                    let copy = shrunk.get_or_insert_with(|| shrink(frame, width)).clone();
                    if let Some(c) = inner.clips.get_mut(&e.clip) {
                        c.frames.push(copy);
                    }
                }
                Action::Finish => {
                    if let Some(c) = inner.clips.remove(&e.clip) {
                        thread::spawn(move || (c.done)(c.frames));
                    }
                }
            }
        }
    }

    /// How many clips are being recorded.
    pub fn active(&self) -> usize {
        lock(&self.inner).timing.active()
    }
}

impl Default for ClipRecorder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn frame(v: u8) -> Frame {
        Frame { width: 2, height: 2, pixels: [v, 0, 0, 255].repeat(4) }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Feeds a 10 fps camera for `frames` frames after `start`.
    fn feed(rec: &ClipRecorder, start: Instant, frames: u64) {
        for i in 1..=frames {
            rec.frame(&frame(i as u8), start + ms(100 * i), 0);
        }
    }

    #[test]
    fn a_clip_has_the_first_frame_and_one_every_interval() {
        let rec = ClipRecorder::new();
        let (tx, rx) = mpsc::channel();
        let t0 = Instant::now();
        // 1 second at 2 frames a second
        rec.start(t0, &frame(0), 0, ms(1000), ms(500), move |f| tx.send(f).unwrap());
        assert_eq!(rec.active(), 1);
        feed(&rec, t0, 12);
        let frames = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let reds: Vec<u8> = frames.iter().map(|f| f.pix[0]).collect();
        assert_eq!(reds, [0, 5], "the alarm's frame, then the one at 0.5 s; the one at 1 s ends the clip");
        assert_eq!(rec.active(), 0);
    }

    #[test]
    fn clips_overlap() {
        let rec = ClipRecorder::new();
        let (tx, rx) = mpsc::channel();
        let t0 = Instant::now();
        let tx2 = tx.clone();
        rec.start(t0, &frame(0), 0, ms(1000), ms(500), move |f| tx.send(("a", f.len())).unwrap());
        feed(&rec, t0, 5);
        rec.start(t0 + ms(500), &frame(0), 0, ms(1000), ms(500), move |f| tx2.send(("b", f.len())).unwrap());
        assert_eq!(rec.active(), 2);
        for i in 6..=20 {
            rec.frame(&frame(i), t0 + ms(100 * u64::from(i)), 0);
        }
        let mut got = vec![rx.recv_timeout(Duration::from_secs(5)).unwrap(), rx.recv_timeout(Duration::from_secs(5)).unwrap()];
        got.sort();
        assert_eq!(got, [("a", 2), ("b", 2)]);
    }

    #[test]
    fn frames_are_shrunk_to_the_width() {
        let big = Frame { width: 8, height: 4, pixels: [9, 9, 9, 255].repeat(32) };
        let img = shrink(&big, 4);
        assert_eq!((img.width, img.height), (4, 2));
        assert_eq!(shrink(&big, 0).width, 8, "0 keeps the size");
        let rec = ClipRecorder::new();
        let (tx, rx) = mpsc::channel();
        let t0 = Instant::now();
        rec.start(t0, &big, 4, ms(100), ms(100), move |f| tx.send(f).unwrap());
        rec.frame(&big, t0 + ms(100), 4);
        let frames = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!((frames[0].width, frames[0].height), (4, 2));
    }

    #[test]
    fn a_copy_stays_valid_when_the_camera_reuses_its_buffer() {
        let mut f = frame(7);
        let copy = shrink(&f, 0);
        f.pixels.fill(0);
        assert_eq!(copy.pix[0], 7);
    }
}
