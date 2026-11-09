//! Keeps track of the latest alarms: the message that told about each one
//! and the GIF that is recorded after it.
//!
//! An alarm starts as `Recording`, the GIF is being made. It becomes `Ready`
//! when the GIF is saved, or `Failed` if it could not be. Whoever reacts to
//! the alarm message with a thumbs up asks for the GIF: `request` says
//! whether to send it now or, if it is still being recorded, once it is
//! ready (`finish` says when).
//!
//! An alarm is named by the id `begin` gives. Once it is forgotten (only the
//! latest `KEPT` are remembered) every call about it does nothing.


use std::collections::{HashMap, VecDeque};
use std::fmt::Write;
use std::sync::{Mutex, MutexGuard};

/// How many alarms are remembered.
pub const KEPT: usize = 100;

/// Where the GIF of an alarm is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The frames are still being captured or encoded.
    Recording,
    /// The GIF is saved and can be sent.
    Ready,
    /// The GIF could not be made or saved.
    Failed,
}

/// What to do about a request for a GIF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// No alarm has that message (it is not an alarm, or too old).
    Unknown,
    /// Send the GIF now.
    Send,
    /// The GIF is still being recorded, `finish` will say when to send it.
    Later,
    /// The GIF was already sent, or is being.
    Nothing,
    /// The GIF could not be made.
    NoGif,
}

/// The state of an alarm.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    /// Seconds since the Unix epoch.
    pub at: i64,
    /// The offset of the local time of the alarm from UTC, in seconds.
    pub utc_offset: i32,
    pub detector: String,
    pub score: f64,
    pub code: String,
    pub status: Status,
    /// Whether the GIF has been asked for and sent.
    pub sent: bool,
    /// Why a `Failed` alarm has no GIF.
    pub reason: String,
}

struct Alarm {
    id: u64,
    at: i64,
    utc_offset: i32,
    detector: String,
    score: f64,
    code: String,
    message_id: i64,
    status: Status,
    reason: String,
    /// Somebody asked for the GIF before it was ready.
    wanted: bool,
    /// The GIF is being sent, or was.
    claimed: bool,
}

#[derive(Default)]
struct Inner {
    next_id: u64,
    /// Oldest first, at most `KEPT`.
    alarms: VecDeque<Alarm>,
    /// Alarm id of each alarm message.
    by_message: HashMap<i64, u64>,
}

impl Inner {
    fn get(&mut self, id: u64) -> Option<&mut Alarm> {
        find(&mut self.alarms, id)
    }
}

/// The alarm `id`. Ids grow by one and the oldest are dropped: the alarm is
/// where its distance from the oldest says.
fn find(alarms: &mut VecDeque<Alarm>, id: u64) -> Option<&mut Alarm> {
    let first = alarms.front()?.id;
    let i = usize::try_from(id.checked_sub(first)?).ok()?;
    alarms.get_mut(i)
}

/// Remembers the latest alarms. It can be shared between threads.
#[derive(Default)]
pub struct Log {
    inner: Mutex<Inner>,
}

impl Log {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // the data stays consistent whatever a panicking thread was doing
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Records a new alarm detected at `at` (seconds since the Unix epoch;
    /// `utc_offset` is the offset of the local time it is told in), in the
    /// `Recording` state. `code` is what the GIF is saved under. Returns the
    /// id of the alarm.
    pub fn begin(&self, at: i64, utc_offset: i32, detector: &str, score: f64, code: &str) -> u64 {
        let mut l = self.lock();
        let id = l.next_id;
        l.next_id += 1;
        l.alarms.push_back(Alarm {
            id,
            at,
            utc_offset,
            detector: detector.to_owned(),
            score,
            code: code.to_owned(),
            message_id: 0,
            status: Status::Recording,
            reason: String::new(),
            wanted: false,
            claimed: false,
        });
        while l.alarms.len() > KEPT {
            if let Some(old) = l.alarms.pop_front() {
                if l.by_message.get(&old.message_id) == Some(&old.id) {
                    l.by_message.remove(&old.message_id);
                }
            }
        }
        id
    }

    /// The id of the message of the alarm, or 0 if it is not sent (yet) or
    /// the alarm is forgotten.
    pub fn message_id(&self, id: u64) -> i64 {
        self.lock().get(id).map_or(0, |a| a.message_id)
    }

    /// Says the message of the alarm was sent as message `message_id`, so a
    /// reaction to that message asks for the GIF.
    pub fn posted(&self, id: u64, message_id: i64) {
        if message_id == 0 {
            return;
        }
        let mut l = self.lock();
        let Inner { alarms, by_message, .. } = &mut *l;
        let Some(a) = find(alarms, id) else { return };
        let old = std::mem::replace(&mut a.message_id, message_id);
        if by_message.get(&old) == Some(&id) {
            by_message.remove(&old);
        }
        by_message.insert(message_id, id);
    }

    /// Says the GIF is saved, or, if `err` is given, that it could not be
    /// made. Returns true if somebody has already asked for the GIF and it is
    /// to be sent now.
    pub fn finish(&self, id: u64, err: Option<&str>) -> bool {
        let mut l = self.lock();
        let Some(a) = l.get(id) else { return false };
        if a.status != Status::Recording {
            return false;
        }
        if let Some(err) = err {
            a.status = Status::Failed;
            a.reason = err.to_owned();
            return false;
        }
        a.status = Status::Ready;
        if a.wanted && !a.claimed {
            a.claimed = true;
            return true;
        }
        false
    }

    /// Says the GIF could not be sent after `request` or `finish` told to
    /// send it, so the next request tries again.
    pub fn unsent(&self, id: u64) {
        if let Some(a) = self.lock().get(id) {
            a.claimed = false;
        }
    }

    /// Called when somebody reacts to the message `message_id`. Returns the
    /// id of the alarm that message is of and what to do.
    pub fn request(&self, message_id: i64) -> (Option<u64>, Action) {
        let mut l = self.lock();
        let Some(&id) = l.by_message.get(&message_id) else {
            return (None, Action::Unknown);
        };
        let Some(a) = l.get(id) else {
            return (None, Action::Unknown);
        };
        let action = match a.status {
            Status::Failed => Action::NoGif,
            Status::Recording if a.wanted => Action::Nothing,
            Status::Recording => {
                a.wanted = true;
                Action::Later
            }
            Status::Ready if a.claimed => Action::Nothing,
            Status::Ready => {
                a.claimed = true;
                Action::Send
            }
        };
        (Some(id), action)
    }

    /// The state of the latest `n` alarms, newest first.
    pub fn recent(&self, n: usize) -> Vec<Info> {
        let l = self.lock();
        l.alarms
            .iter()
            .rev()
            .take(n)
            .map(|a| Info {
                at: a.at,
                utc_offset: a.utc_offset,
                detector: a.detector.clone(),
                score: a.score,
                code: a.code.clone(),
                status: a.status,
                sent: a.claimed && a.status == Status::Ready,
                reason: a.reason.clone(),
            })
            .collect()
    }

    /// Describes the latest `n` alarms for a Telegram message.
    pub fn report(&self, n: usize) -> String {
        let alarms = self.recent(n);
        if alarms.is_empty() {
            return "No alarms yet.".to_owned();
        }
        let mut b = format!("Latest {} alarms, newest first:", alarms.len());
        for a in &alarms {
            let _ = write!(b, "\n{} {} {:.4}: ", clock(a.at, a.utc_offset), a.detector, a.score);
            let _ = match a.status {
                Status::Recording => write!(b, "recording the GIF"),
                Status::Failed => write!(b, "no GIF ({})", a.reason),
                Status::Ready if a.sent => write!(b, "GIF sent ({})", a.code),
                Status::Ready => write!(b, "GIF ready, react with 👍 or /hd {}", a.code),
            };
        }
        b
    }
}

/// The time of day, as HH:MM:SS, of `at` seconds since the Unix epoch in the
/// time zone `utc_offset` seconds from UTC.
fn clock(at: i64, utc_offset: i32) -> String {
    let t = (at + i64::from(utc_offset)).rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_after_ready() {
        let l = Log::new();
        let a = l.begin(0, 0, "background", 0.1, "abc123");
        l.posted(a, 7);
        assert_eq!(l.request(7), (Some(a), Action::Later));
        assert!(l.finish(a, None), "somebody asked before it was ready: finish should say to send");
        assert_eq!(l.request(7).1, Action::Nothing, "after it is being sent");
        l.unsent(a);
        assert_eq!(l.request(7), (Some(a), Action::Send), "after a failed send");
    }

    #[test]
    fn request_when_already_ready() {
        let l = Log::new();
        let a = l.begin(0, 0, "background", 0.1, "abc123");
        l.posted(a, 7);
        assert!(!l.finish(a, None), "nobody asked: finish must not send");
        assert_eq!(l.request(7).1, Action::Send);
        assert_eq!(l.request(7).1, Action::Nothing, "second thumbs up");
    }

    #[test]
    fn asking_twice_while_recording_sends_once() {
        let l = Log::new();
        let a = l.begin(0, 0, "background", 0.1, "abc123");
        l.posted(a, 7);
        assert_eq!(l.request(7).1, Action::Later);
        assert_eq!(l.request(7).1, Action::Nothing);
        assert!(l.finish(a, None));
        assert!(!l.finish(a, None), "finish only counts once");
    }

    #[test]
    fn failed_and_unknown() {
        let l = Log::new();
        let a = l.begin(0, 0, "background", 0.1, "abc123");
        l.posted(a, 7);
        assert!(!l.finish(a, Some("boom")), "a failed GIF must not be sent");
        assert_eq!(l.request(7).1, Action::NoGif);
        assert_eq!(l.request(99), (None, Action::Unknown));
    }

    #[test]
    fn message_zero_is_not_posted() {
        let l = Log::new();
        let a = l.begin(0, 0, "d", 0.0, "c");
        l.posted(a, 0);
        assert_eq!(l.message_id(a), 0);
        assert_eq!(l.request(0), (None, Action::Unknown));
        l.posted(a, 5);
        assert_eq!(l.message_id(a), 5);
    }

    #[test]
    fn old_alarms_forgotten() {
        let l = Log::new();
        let mut ids = vec![];
        for i in 1..=(KEPT + 5) as i64 {
            let id = l.begin(0, 0, "d", 0.0, "c");
            l.posted(id, i);
            ids.push(id);
        }
        assert_eq!(l.request(1).1, Action::Unknown, "alarm 1 should be forgotten");
        assert_ne!(l.request((KEPT + 5) as i64).1, Action::Unknown, "the latest alarm should be remembered");
        assert_eq!(l.lock().by_message.len(), KEPT);
        // calls about a forgotten alarm do nothing
        l.posted(ids[0], 1000);
        assert_eq!(l.request(1000).1, Action::Unknown);
        assert_eq!(l.message_id(ids[0]), 0);
        assert!(!l.finish(ids[0], None));
        l.unsent(ids[0]);
        assert_eq!(l.recent(usize::MAX).len(), KEPT);
    }

    #[test]
    fn a_message_moved_to_another_alarm_stays_with_it() {
        let l = Log::new();
        let a = l.begin(0, 0, "d", 0.0, "a");
        l.posted(a, 7);
        let b = l.begin(0, 0, "d", 0.0, "b");
        l.posted(b, 7);
        assert_eq!(l.request(7).0, Some(b));
        // the first one is forgotten, that must not take message 7 away from b
        for _ in 0..KEPT - 1 {
            l.begin(0, 0, "d", 0.0, "x");
        }
        assert_eq!(l.request(7).0, Some(b));
    }

    #[test]
    fn recent_is_newest_first() {
        let l = Log::new();
        for i in 0..5 {
            l.begin(i, 0, "d", i as f64, &format!("c{i}"));
        }
        let codes: Vec<_> = l.recent(3).into_iter().map(|i| i.code).collect();
        assert_eq!(codes, ["c4", "c3", "c2"]);
    }

    #[test]
    fn report() {
        let l = Log::new();
        assert_eq!(l.report(5), "No alarms yet.");
        let at = 3 * 3600 + 4 * 60 + 5;
        l.begin(at, 0, "background", 0.1, "aaaaaa");
        let b = l.begin(at, 3600, "background", 0.2, "bbbbbb");
        l.finish(b, None);
        let c = l.begin(at, -3600 * 4, "background", 0.03125, "cccccc");
        l.finish(c, Some("disk full"));
        let got = l.report(5);
        assert_eq!(
            got,
            "Latest 3 alarms, newest first:\
             \n23:04:05 background 0.0312: no GIF (disk full)\
             \n04:04:05 background 0.2000: GIF ready, react with 👍 or /hd bbbbbb\
             \n03:04:05 background 0.1000: recording the GIF"
        );
        l.posted(b, 9);
        assert_eq!(l.request(9), (Some(b), Action::Send));
        assert_eq!(
            l.report(2),
            "Latest 2 alarms, newest first:\
             \n23:04:05 background 0.0312: no GIF (disk full)\
             \n04:04:05 background 0.2000: GIF sent (bbbbbb)"
        );
        assert_eq!(l.report(0), "No alarms yet.");
    }

    #[test]
    fn clock_wraps_around_midnight() {
        assert_eq!(clock(0, 0), "00:00:00");
        assert_eq!(clock(-1, 0), "23:59:59");
        assert_eq!(clock(86_399, 1), "00:00:00");
    }
}
