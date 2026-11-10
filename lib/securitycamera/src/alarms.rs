//! What happens when motion is detected: the alarm is told at once, its GIF
//! is recorded in the background, and it is sent to whoever reacts to the
//! message.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use alarmlog::{Action, Log};
use chrono::{DateTime, Local, Offset};
use gif::Image;
use gif::store::{Error as StoreError, Store};
use my_video::Frame;

use crate::clips::{ClipRecorder, Recorded};
use crate::config::Values;
use crate::telegram::{Chat, Reaction};
use crate::util::lock;
use crate::waveform::{self, Trace};

/// An alarm in an [`AlarmBook`].
#[derive(Clone, Debug)]
pub struct Alarm {
    pub id: u64,
    /// When the alarm went off.
    pub at: DateTime<Local>,
    /// What the GIF of the alarm is saved under.
    pub code: String,
}

/// The latest alarms: the message that told about each one and where its GIF
/// is (see the `alarmlog` crate).
#[derive(Default)]
pub struct AlarmBook {
    log: Log,
    /// the ones the log still remembers
    known: Mutex<HashMap<u64, Alarm>>,
}

impl AlarmBook {
    pub fn new() -> AlarmBook {
        AlarmBook::default()
    }

    /// Records a new alarm detected at `at`, in the recording state. `code` is
    /// what the GIF is saved under.
    pub fn begin(&self, at: DateTime<Local>, detector: &str, score: f64, code: &str) -> Alarm {
        let id = self.log.begin(
            at.timestamp(),
            at.offset().fix().local_minus_utc(),
            detector,
            score,
            code,
        );
        let alarm = Alarm {
            id,
            at,
            code: code.to_owned(),
        };
        let mut known = lock(&self.known);
        known.insert(id, alarm.clone());
        if id >= alarmlog::KEPT as u64 {
            known.remove(&(id - alarmlog::KEPT as u64));
        }
        alarm
    }

    /// The id of the alarm message, 0 if it is not sent (yet).
    pub fn message_id(&self, a: &Alarm) -> i64 {
        self.log.message_id(a.id)
    }

    /// Says the message of the alarm was sent as message `message_id`, so a
    /// reaction to that message asks for the GIF.
    pub fn posted(&self, a: &Alarm, message_id: i64) {
        self.log.posted(a.id, message_id);
    }

    /// Says the GIF is saved, or, if `err` is given, that it could not be
    /// made. Returns true if somebody has already asked for the GIF and it is
    /// to be sent now.
    pub fn finish(&self, a: &Alarm, err: Option<&str>) -> bool {
        self.log.finish(a.id, err)
    }

    /// Says the GIF could not be sent after `request` or `finish` told to send
    /// it, so the next request tries again.
    pub fn unsent(&self, a: &Alarm) {
        self.log.unsent(a.id);
    }

    /// Called when somebody reacts to the message `message_id`. Returns the
    /// alarm that message is of and what to do.
    pub fn request(&self, message_id: i64) -> (Option<Alarm>, Action) {
        match self.log.request(message_id) {
            (Some(id), action) => match lock(&self.known).get(&id) {
                Some(a) => (Some(a.clone()), action),
                None => (None, Action::Unknown),
            },
            (None, _) => (None, Action::Unknown),
        }
    }

    /// Describes the latest `n` alarms for a Telegram message.
    pub fn report(&self, n: usize) -> String {
        self.log.report(n)
    }
}

/// What the alarms need: where to tell, where the GIFs are kept, and the clips.
pub struct Alarms {
    pub chat: Arc<dyn Chat>,
    pub book: AlarmBook,
    pub clips: ClipRecorder,
    /// Without the store the alarms still work, they just have no GIF.
    pub gifs: Option<Arc<Store>>,
}

/// A random code to save a GIF under: six hex digits, short enough to type in
/// a Telegram command.
pub fn new_code() -> Result<String, String> {
    let mut b = [0u8; 3];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|e| format!("no random code: {e}"))?;
    Ok(hex::encode(b))
}

/// Tells about the alarm at once and, in the background, records the GIF of
/// it: the frame that set it off, the ones of the next `v.alarm_gif_span`,
/// at `v.alarm_gif_fps` a second, and the ones kept from just before it. The GIF is saved under the code in the
/// message, and goes out when somebody reacts to the message with any emoji.
/// The GIF has the waveform of the motion score along the bottom: `threshold`
/// is the score that set the alarm off, drawn on it. The caller feeds the
/// scores with [`ClipRecorder::score`], the one of `frame` included.
/// `frame` is only valid until the caller returns; `mono` is when it was
/// taken, on the clock the following frames are timed with.
#[allow(clippy::too_many_arguments)]
pub fn start_alarm(
    alarms: &Arc<Alarms>,
    at: DateTime<Local>,
    mono: Instant,
    frame: &Frame,
    detector: &str,
    score: f64,
    threshold: f64,
    v: Values,
) {
    // without a code the alarm still goes out, it just has no GIF
    let code = new_code();
    let a = alarms
        .book
        .begin(at, detector, score, code.as_deref().unwrap_or(""));

    let mut text = format!(
        "Alarm: motion detected at {} ({detector} score {score:.4})",
        at.format("%H:%M:%S")
    );
    if let (Ok(code), Some(_)) = (&code, &alarms.gifs) {
        text.push_str(&format!(
            "\nReact with any emoji to get the GIF (or send /hd {code})"
        ));
    }
    // the message must not wait for the camera loop, nor the recording for the message
    {
        let (alarms, a) = (alarms.clone(), a.clone());
        thread::spawn(move || match alarms.chat.send(&text) {
            Ok(id) => alarms.book.posted(&a, id),
            Err(e) => eprintln!("alarm: {e}"),
        });
    }

    let interval = Duration::from_secs(1) / v.alarm_gif_fps.max(1);
    // the delay is in hundredths of a second
    let delay = ((interval.as_millis() / 10) as u16).max(1);
    let done = {
        let alarms = alarms.clone();
        move |recorded: Recorded| {
            let result = match (&code, &alarms.gifs) {
                (Err(e), _) => Err(e.clone()),
                (Ok(_), None) => Err("GIFs are not being saved".to_owned()),
                (Ok(code), Some(gifs)) => save_gif(gifs, code, recorded, delay, v),
            };
            if let Err(e) = &result {
                eprintln!("alarm gif: {e}");
            }
            // somebody may have asked for the GIF while it was being made
            if alarms.book.finish(&a, result.err().as_deref()) {
                send_gif(&alarms, &a);
            }
        }
    };
    alarms.clips.start(
        mono,
        frame,
        v.hd_gif_width,
        v.alarm_gif_span,
        interval,
        threshold,
        done,
    );
}

/// Draws the waveform on the frames of `rec`, encodes them as a GIF, `delay`
/// hundredths of a second apart, and saves it under `code`.
fn save_gif(gifs: &Store, code: &str, rec: Recorded, delay: u16, v: Values) -> Result<(), String> {
    let Recorded {
        mut frames,
        times,
        scores,
        span,
        threshold,
    } = rec;
    let trace = Trace {
        scores: &scores,
        span,
        threshold,
    };
    for (frame, &t) in frames.iter_mut().zip(&times) {
        waveform::overlay(frame, &trace, t);
    }
    let images: Vec<Image> = frames
        .iter()
        .map(|f| Image::Rgba {
            data: &f.pix,
            width: f.width,
            height: f.height,
            stride: f.width * 4,
        })
        .collect();
    let data = gif::encode(&images, delay, v.hd_gif_dither).ok_or("gif: no images")?;
    gifs.save(code, &data).map_err(|e| e.to_string())
}

/// Sends the saved GIF of the alarm as a reply to its message. If that fails
/// the next request for it tries again.
pub fn send_gif(alarms: &Alarms, a: &Alarm) {
    let (err, gone) = match &alarms.gifs {
        None => ("GIFs are not being saved".to_owned(), false),
        Some(gifs) => match gifs.get(&a.code) {
            Ok(data) => match alarms.chat.send_animation_reply(
                &data,
                &format!("GIF of the alarm at {}", a.at.format("%H:%M:%S")),
                alarms.book.message_id(a),
            ) {
                Ok(()) => return,
                Err(e) => (e, false),
            },
            Err(StoreError::NotFound) => (StoreError::NotFound.to_string(), true),
            Err(e) => (e.to_string(), false),
        },
    };
    alarms.book.unsent(a);
    eprintln!("alarm gif: {err}");
    let msg = if gone {
        "The GIF is gone: old ones are deleted to keep the storage under the limit."
    } else {
        "Could not send the GIF."
    };
    if let Err(e) = alarms.chat.send_message(msg) {
        eprintln!("alarm gif: {e}");
    }
}

/// Sends the GIF of an alarm when its message gets a reaction, any emoji will
/// do.
pub fn on_reaction(alarms: &Arc<Alarms>, r: &Reaction) {
    let (alarm, action) = alarms.book.request(r.message_id);
    let alarms = alarms.clone();
    match (action, alarm) {
        // uploading takes a while, don't hold up the other updates
        (Action::Send, Some(a)) => {
            thread::spawn(move || send_gif(&alarms, &a));
        }
        (Action::NoGif, _) => {
            thread::spawn(move || {
                if let Err(e) = alarms
                    .chat
                    .send_message("There is no GIF of that alarm: it could not be made.")
                {
                    eprintln!("alarm gif: {e}");
                }
            });
        }
        _ => {}
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::mpsc::{self, Receiver, Sender};

    /// What the fake chat was asked to send.
    #[derive(Debug, PartialEq)]
    pub enum Sent {
        Text(String),
        Photo(Vec<u8>, String),
        Animation(Vec<u8>, String),
        AnimationReply(Vec<u8>, String, i64),
        Button(String, String, String),
    }

    /// A chat that hands out message ids from 100 and reports what it is sent.
    pub struct FakeChat {
        pub tx: Mutex<Sender<Sent>>,
        pub next_id: Mutex<i64>,
        pub fail: Mutex<bool>,
    }

    pub fn fake_chat() -> (Arc<FakeChat>, Receiver<Sent>) {
        let (tx, rx) = mpsc::channel();
        (
            Arc::new(FakeChat {
                tx: Mutex::new(tx),
                next_id: Mutex::new(100),
                fail: Mutex::new(false),
            }),
            rx,
        )
    }

    impl FakeChat {
        fn push(&self, s: Sent) -> Result<(), String> {
            if *lock(&self.fail) {
                return Err("down".into());
            }
            lock(&self.tx).send(s).map_err(|e| e.to_string())
        }
    }

    impl Chat for FakeChat {
        fn send(&self, text: &str) -> Result<i64, String> {
            self.push(Sent::Text(text.into()))?;
            let mut id = lock(&self.next_id);
            *id += 1;
            Ok(*id)
        }
        fn send_photo(&self, jpeg: &[u8], caption: &str) -> Result<(), String> {
            self.push(Sent::Photo(jpeg.into(), caption.into()))
        }
        fn send_animation(&self, gif: &[u8], caption: &str) -> Result<(), String> {
            self.push(Sent::Animation(gif.into(), caption.into()))
        }
        fn send_animation_reply(
            &self,
            gif: &[u8],
            caption: &str,
            reply_to: i64,
        ) -> Result<(), String> {
            self.push(Sent::AnimationReply(gif.into(), caption.into(), reply_to))
        }
        fn send_web_app_button(&self, text: &str, label: &str, url: &str) -> Result<(), String> {
            self.push(Sent::Button(text.into(), label.into(), url.into()))
        }
    }

    pub fn temp_dir(name: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "sc-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const WAIT: Duration = Duration::from_secs(10);

    fn frame(v: u8) -> Frame {
        Frame {
            width: 4,
            height: 4,
            pixels: [v, 20, 30, 255].repeat(16),
        }
    }

    fn values() -> Values {
        Values {
            alarm_gif_span: Duration::from_millis(500),
            alarm_gif_fps: 4,
            ..Values::default()
        }
    }

    fn alarms(chat: Arc<FakeChat>, with_store: bool) -> Arc<Alarms> {
        let gifs = with_store.then(|| Arc::new(Store::new(temp_dir("gifs"), 1 << 20).unwrap()));
        Arc::new(Alarms {
            chat,
            book: AlarmBook::new(),
            clips: ClipRecorder::new(),
            gifs,
        })
    }

    /// Waits until the first alarm's message has an id (ids start at 101 here).
    fn wait_posted(a: &Alarms) {
        let deadline = Instant::now() + WAIT;
        while a.book.log.message_id(0) == 0 {
            assert!(
                Instant::now() < deadline,
                "the alarm message was never posted"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Feeds a 10 fps camera until the clip that started at `t0` is over.
    fn finish_clip(a: &Alarms, t0: Instant) {
        for i in 1..=10u64 {
            a.clips.frame(
                &frame(i as u8),
                t0 + Duration::from_millis(100 * i),
                0,
                Duration::ZERO,
                Duration::ZERO,
            );
        }
    }

    fn text(rx: &Receiver<Sent>) -> String {
        match rx.recv_timeout(WAIT).expect("nothing sent") {
            Sent::Text(t) => t,
            other => panic!("expected a text, got {other:?}"),
        }
    }

    fn react(a: &Arc<Alarms>, message_id: i64) {
        on_reaction(
            a,
            &Reaction {
                message_id,
                emoji: "👍".into(),
                from: "bob".into(),
            },
        );
    }

    fn code_of(message: &str) -> String {
        message
            .split("/hd ")
            .nth(1)
            .expect("a code in the message")
            .trim_end_matches(')')
            .to_owned()
    }

    #[test]
    fn the_alarm_is_told_at_once_and_the_gif_goes_out_on_a_reaction() {
        let (chat, rx) = fake_chat();
        let a = alarms(chat, true);
        let (t0, at) = (Instant::now(), Local::now());
        start_alarm(&a, at, t0, &frame(0), "background", 0.0123, 0.01, values());

        let message = text(&rx);
        assert!(
            message.starts_with(&format!(
                "Alarm: motion detected at {} (background score 0.0123)",
                at.format("%H:%M:%S")
            )),
            "{message}"
        );
        assert!(
            message.contains("React with any emoji to get the GIF (or send /hd "),
            "{message}"
        );
        let code = code_of(&message);
        assert!(gif::store::valid_code(&code), "{code}");

        wait_posted(&a);
        finish_clip(&a, t0);
        // 0.5 s at 4 frames a second: the alarm's frame and 1 more before the end
        let deadline = Instant::now() + WAIT;
        while a.gifs.as_ref().unwrap().get(&code).is_err() {
            assert!(Instant::now() < deadline, "the GIF was not saved");
            thread::sleep(Duration::from_millis(5));
        }
        let gif = a.gifs.as_ref().unwrap().get(&code).unwrap();
        assert!(gif.starts_with(b"GIF89a"));

        react(&a, 101);
        match rx.recv_timeout(WAIT).expect("the GIF was not sent") {
            Sent::AnimationReply(data, caption, reply_to) => {
                assert_eq!(data, gif);
                assert_eq!(
                    caption,
                    format!("GIF of the alarm at {}", at.format("%H:%M:%S"))
                );
                assert_eq!(reply_to, 101);
            }
            other => panic!("{other:?}"),
        }
        // a second reaction does not send it twice
        react(&a, 101);
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "sent twice"
        );
        assert!(a.book.report(1).contains(&format!("GIF sent ({code})")));
    }

    #[test]
    fn a_reaction_before_the_gif_is_ready_sends_it_when_it_is() {
        let (chat, rx) = fake_chat();
        let a = alarms(chat, true);
        let t0 = Instant::now();
        start_alarm(
            &a,
            Local::now(),
            t0,
            &frame(0),
            "difference",
            0.5,
            0.1,
            values(),
        );
        text(&rx);
        wait_posted(&a);
        // the GIF is still recording: nothing to send yet
        react(&a, 101);
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        finish_clip(&a, t0);
        match rx
            .recv_timeout(WAIT)
            .expect("the GIF was not sent once ready")
        {
            Sent::AnimationReply(data, _, 101) => assert!(data.starts_with(b"GIF89a")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn without_a_store_the_alarm_has_no_gif() {
        let (chat, rx) = fake_chat();
        let a = alarms(chat, false);
        let t0 = Instant::now();
        start_alarm(
            &a,
            Local::now(),
            t0,
            &frame(0),
            "background",
            0.1,
            0.05,
            values(),
        );
        let message = text(&rx);
        assert!(!message.contains("React"), "{message}");
        wait_posted(&a);
        finish_clip(&a, t0);
        let deadline = Instant::now() + WAIT;
        while !a.book.report(1).contains("no GIF") {
            assert!(
                Instant::now() < deadline,
                "the alarm never failed: {}",
                a.book.report(1)
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert!(a.book.report(1).contains("GIFs are not being saved"));
        react(&a, 101);
        assert_eq!(
            text(&rx),
            "There is no GIF of that alarm: it could not be made."
        );
    }

    #[test]
    fn a_gif_that_can_not_be_sent_is_tried_again_on_the_next_reaction() {
        let (chat, rx) = fake_chat();
        let a = alarms(chat.clone(), true);
        let t0 = Instant::now();
        start_alarm(
            &a,
            Local::now(),
            t0,
            &frame(0),
            "background",
            0.1,
            0.05,
            values(),
        );
        let code = code_of(&text(&rx));
        wait_posted(&a);
        finish_clip(&a, t0);
        let deadline = Instant::now() + WAIT;
        while a.gifs.as_ref().unwrap().get(&code).is_err() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        // the first request (above) left it claimed; a failed send frees it
        let alarm = a.book.request(101).0.expect("known alarm");
        *lock(&chat.fail) = true;
        send_gif(&a, &alarm);
        *lock(&chat.fail) = false;
        react(&a, 101);
        assert!(
            matches!(rx.recv_timeout(WAIT).unwrap(), Sent::AnimationReply(..)),
            "not sent again"
        );
    }

    #[test]
    fn a_deleted_gif_is_reported() {
        let (chat, rx) = fake_chat();
        let a = alarms(chat, true);
        let alarm = a.book.begin(Local::now(), "background", 0.1, "abcdef");
        send_gif(&a, &alarm);
        assert_eq!(
            text(&rx),
            "The GIF is gone: old ones are deleted to keep the storage under the limit."
        );
    }

    #[test]
    fn reactions_to_other_messages_do_nothing() {
        let (chat, rx) = fake_chat();
        let a = alarms(chat, true);
        react(&a, 12345);
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn a_failed_alarm_message_is_not_fatal() {
        let (chat, rx) = fake_chat();
        *lock(&chat.fail) = true;
        let a = alarms(chat, true);
        start_alarm(
            &a,
            Local::now(),
            Instant::now(),
            &frame(0),
            "background",
            0.1,
            0.05,
            values(),
        );
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        assert_eq!(
            a.clips.active(),
            1,
            "the recording goes on without the message"
        );
    }

    #[test]
    fn the_book_forgets_old_alarms() {
        let book = AlarmBook::new();
        let first = book.begin(Local::now(), "background", 0.1, "000001");
        book.posted(&first, 7);
        assert!(book.request(7).0.is_some());
        for i in 0..alarmlog::KEPT {
            book.begin(Local::now(), "background", 0.1, &format!("{i:06x}"));
        }
        assert_eq!(book.request(7).1, Action::Unknown);
        assert!(book.report(3).starts_with("Latest 3 alarms"));
    }

    #[test]
    fn codes_are_six_hex_digits_and_differ() {
        let (a, b) = (new_code().unwrap(), new_code().unwrap());
        assert!(gif::store::valid_code(&a) && gif::store::valid_code(&b));
        assert_ne!(a, b, "two random codes were equal");
    }
}
