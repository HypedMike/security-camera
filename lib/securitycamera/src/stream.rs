//! Reads frames from a source and fans them out to handlers, keeping the
//! last few frames as history.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::{DateTime, Local};
use gif::Image;
use my_video::{Capture, Frame};
use streaming::Ring;

use crate::util::lock;

/// What a handler is given for each frame. Everything in it is only valid
/// until the handler returns: the buffers are reused for later frames, so copy
/// what you need to keep.
pub struct Tick<'a> {
    pub frame: &'a Frame,
    /// The previous frames, oldest first, not including the current one.
    pub history: &'a Ring,
    /// When the frame was captured, as the clock on the wall and as a
    /// monotonic one (the wall clock can be set while the camera runs).
    pub wall: DateTime<Local>,
    pub mono: Instant,
}

/// Handlers run in the order they were added, for every frame.
pub type Handler = Box<dyn FnMut(&Tick) + Send>;

/// Where the frames come from (the camera, or a fake in the tests).
pub trait Source {
    /// Waits for the next frame and writes it into `frame`.
    fn read(&mut self, frame: &mut Frame) -> Result<(), String>;
}

impl Source for Arc<Capture> {
    fn read(&mut self, frame: &mut Frame) -> Result<(), String> {
        Capture::read(self, frame)
    }
}

/// The frames of a source, fanned out to handlers.
pub struct Stream {
    handlers: Mutex<Vec<Handler>>,
    /// The last frames. The handlers see it while they run, so anything that
    /// changes it waits for them.
    ring: Mutex<Ring>,
}

impl Stream {
    /// `history` is how many past frames are passed to handlers. 0 disables it.
    pub fn new(history: usize) -> Stream {
        Stream {
            handlers: Mutex::new(Vec::new()),
            ring: Mutex::new(Ring::new(history)),
        }
    }

    /// Registers a handler.
    pub fn add_handler(&self, h: impl FnMut(&Tick) + Send + 'static) {
        lock(&self.handlers).push(Box::new(h));
    }

    /// How many past frames are kept.
    pub fn history(&self) -> usize {
        lock(&self.ring).capacity()
    }

    /// Changes how many past frames are kept. When shrinking, the newest
    /// frames are kept. It waits for the handlers of the current frame, so a
    /// handler must not call it.
    pub fn set_history(&self, n: usize) {
        lock(&self.ring).set_capacity(n);
    }

    /// Reads frames until `stop` is set or the source fails, calling every
    /// handler in registration order for each frame. Handlers run
    /// synchronously, so a slow handler delays the next frame; hand work off
    /// to a thread if needed. A source that fails after `stop` was set (the
    /// camera is released to stop a read that waits) is not an error.
    pub fn run(&self, source: &mut dyn Source, stop: &AtomicBool) -> Result<(), String> {
        let mut frame = Frame::default();
        while !stop.load(Ordering::SeqCst) {
            if let Err(e) = source.read(&mut frame) {
                return if stop.load(Ordering::SeqCst) {
                    Ok(())
                } else {
                    Err(e)
                };
            }
            let (wall, mono) = (Local::now(), Instant::now());
            let mut ring = lock(&self.ring);
            for h in lock(&self.handlers).iter_mut() {
                h(&Tick {
                    frame: &frame,
                    history: &ring,
                    wall,
                    mono,
                });
            }
            let (width, height) = (frame.width as usize, frame.height as usize);
            if width > 0 && height > 0 && frame.pixels.len() >= width * height * 4 {
                ring.remember(&Image::Rgba {
                    data: &frame.pixels,
                    width,
                    height,
                    stride: width * 4,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Frames of one pixel, each with the value it was given; fails when it runs out.
    struct Fake {
        values: VecDeque<u8>,
        on_empty: Box<dyn Fn()>,
    }

    impl Source for Fake {
        fn read(&mut self, frame: &mut Frame) -> Result<(), String> {
            match self.values.pop_front() {
                Some(v) => {
                    *frame = Frame {
                        width: 1,
                        height: 1,
                        pixels: vec![v, 0, 0, 255],
                    };
                    Ok(())
                }
                None => {
                    (self.on_empty)();
                    Err("camera gone".into())
                }
            }
        }
    }

    fn fake(values: &[u8]) -> Fake {
        Fake {
            values: values.iter().copied().collect(),
            on_empty: Box::new(|| {}),
        }
    }

    #[test]
    fn handlers_see_every_frame_in_order_with_the_history() {
        let stream = Stream::new(2);
        let seen = Arc::new(Mutex::new(Vec::new()));
        for name in ["first", "second"] {
            let seen = seen.clone();
            stream.add_handler(move |t| {
                let history: Vec<u8> = (0..t.history.len())
                    .map(|i| t.history.frame(i).unwrap().0[0])
                    .collect();
                seen.lock()
                    .unwrap()
                    .push((name, t.frame.pixels[0], history));
            });
        }
        let err = stream
            .run(&mut fake(&[1, 2, 3]), &AtomicBool::new(false))
            .unwrap_err();
        assert_eq!(err, "camera gone");
        let want: Vec<(&str, u8, Vec<u8>)> = vec![
            ("first", 1, vec![]),
            ("second", 1, vec![]),
            ("first", 2, vec![1]),
            ("second", 2, vec![1]),
            ("first", 3, vec![1, 2]),
            ("second", 3, vec![1, 2]),
        ];
        assert_eq!(*seen.lock().unwrap(), want);
    }

    #[test]
    fn the_history_can_change_while_running() {
        let stream = Arc::new(Stream::new(5));
        assert_eq!(stream.history(), 5);
        stream.set_history(1);
        assert_eq!(stream.history(), 1);
        let lens = Arc::new(Mutex::new(Vec::new()));
        let l = lens.clone();
        stream.add_handler(move |t| l.lock().unwrap().push(t.history.len()));
        let _ = stream.run(&mut fake(&[1, 2, 3]), &AtomicBool::new(false));
        assert_eq!(*lens.lock().unwrap(), [0, 1, 1]);
    }

    #[test]
    fn a_failure_after_stop_is_not_an_error() {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let mut source = fake(&[1]);
        source.on_empty = Box::new(move || s.store(true, Ordering::SeqCst));
        assert_eq!(Stream::new(0).run(&mut source, &stop), Ok(()));
    }

    #[test]
    fn stops_between_frames() {
        let stop = Arc::new(AtomicBool::new(false));
        let stream = Stream::new(0);
        let (s, n) = (stop.clone(), Arc::new(Mutex::new(0)));
        let count = n.clone();
        stream.add_handler(move |_| {
            *count.lock().unwrap() += 1;
            s.store(true, Ordering::SeqCst);
        });
        assert_eq!(stream.run(&mut fake(&[1, 2, 3]), &stop), Ok(()));
        assert_eq!(*n.lock().unwrap(), 1);
    }

    #[test]
    fn frames_have_a_time() {
        let stream = Stream::new(0);
        let before = Local::now();
        let at = Arc::new(Mutex::new(None));
        let a = at.clone();
        stream.add_handler(move |t| *a.lock().unwrap() = Some(t.wall));
        let _ = stream.run(&mut fake(&[1]), &AtomicBool::new(false));
        let t = at.lock().unwrap().unwrap();
        assert!(t >= before && t <= Local::now());
    }
}
