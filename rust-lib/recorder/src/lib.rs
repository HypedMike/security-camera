//! Decides which frames of the camera belong to which clip.
//!
//! An alarm starts a clip: its first frame is taken at once, then one frame
//! every `interval` until `span` has passed. Several clips can run at the
//! same time. This crate only keeps the timing: the frames themselves stay
//! with the caller, which is told for every frame of the stream what to do
//! with it (see [`Recorder::frame`]). Times are nanoseconds on any clock, as
//! long as it is the same one for every call.

pub mod ffi;

/// What to do with a frame for one clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The frame is the next one of the clip: keep a copy.
    Keep,
    /// The clip is over: do not keep the frame, hand over what was kept.
    Finish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    /// The id [`Recorder::start`] returned.
    pub clip: u64,
    pub action: Action,
}

struct Clip {
    id: u64,
    end: i64,
    next: i64,
    interval: i64,
}

/// Records several clips at once. Not thread safe: use one at a time.
#[derive(Default)]
pub struct Recorder {
    clips: Vec<Clip>,
    next_id: u64,
    events: Vec<Event>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Begins a clip at `at`, whose first frame the caller keeps itself, then
    /// one frame every `interval` until `span` has passed. An `interval` that
    /// is not positive means just the first frame. Returns the id of the clip.
    pub fn start(&mut self, at: i64, span: i64, interval: i64) -> u64 {
        let interval = if interval <= 0 { span } else { interval };
        let id = self.next_id;
        self.next_id += 1;
        self.clips.push(Clip {
            id,
            end: at.saturating_add(span),
            next: at.saturating_add(interval),
            interval,
        });
        id
    }

    /// Called for every frame of the stream, taken at `ts`. Returns what to do
    /// with it for each clip that needs something, until the next call.
    pub fn frame(&mut self, ts: i64) -> &[Event] {
        self.events.clear();
        let events = &mut self.events;
        self.clips.retain_mut(|clip| {
            if ts >= clip.end {
                events.push(Event { clip: clip.id, action: Action::Finish });
                return false;
            }
            if ts >= clip.next {
                events.push(Event { clip: clip.id, action: Action::Keep });
                // stay on the grid even if a frame came late; an interval
                // that is not positive can't be here, start() fixed it
                let behind = (ts - clip.next) / clip.interval + 1;
                clip.next = clip.next.saturating_add(behind.saturating_mul(clip.interval));
            }
            true
        });
        &self.events
    }

    /// How many clips are being recorded.
    pub fn active(&self) -> usize {
        self.clips.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: i64 = 1_000_000_000;

    /// Feeds a 30 fps camera and counts the frames kept per clip.
    fn run(r: &mut Recorder, from: i64, frames: i64, kept: &mut Vec<(u64, i64)>, done: &mut Vec<u64>) {
        for i in 1..=frames {
            let ts = from + i * SEC / 30;
            for e in r.frame(ts) {
                match e.action {
                    Action::Keep => kept.push((e.clip, ts)),
                    Action::Finish => done.push(e.clip),
                }
            }
        }
    }

    #[test]
    fn records_on_the_grid_then_finishes() {
        let mut r = Recorder::new();
        let id = r.start(1000 * SEC, 5 * SEC, SEC / 2);
        let (mut kept, mut done) = (vec![], vec![]);
        run(&mut r, 1000 * SEC, 160, &mut kept, &mut done);
        // the caller keeps the first frame itself: 2 a second for 5 seconds
        assert_eq!(kept.len() + 1, 10);
        assert_eq!(done, [id]);
        assert_eq!(r.active(), 0);
    }

    #[test]
    fn slow_camera_still_finishes() {
        let mut r = Recorder::new();
        let id = r.start(0, 5 * SEC, 0);
        assert_eq!(r.frame(10 * SEC), [Event { clip: id, action: Action::Finish }]);
        assert_eq!(r.active(), 0);
    }

    #[test]
    fn a_late_frame_stays_on_the_grid() {
        let mut r = Recorder::new();
        let id = r.start(0, 10 * SEC, SEC);
        // the frame at 3.5 s is the one for 1 s, the next is due at 4 s
        assert_eq!(r.frame(SEC * 7 / 2), [Event { clip: id, action: Action::Keep }]);
        assert!(r.frame(SEC * 39 / 10).is_empty());
        assert_eq!(r.frame(4 * SEC).len(), 1);
    }

    #[test]
    fn overlapping_clips() {
        let mut r = Recorder::new();
        let a = r.start(0, 2 * SEC, SEC);
        let b = r.start(SEC, 2 * SEC, SEC);
        assert_ne!(a, b);
        let (mut kept, mut done) = (vec![], vec![]);
        run(&mut r, 0, 90, &mut kept, &mut done);
        for id in [a, b] {
            assert_eq!(kept.iter().filter(|k| k.0 == id).count() + 1, 2, "clip {id}");
        }
        assert_eq!(done, [a, b]);
    }
}
