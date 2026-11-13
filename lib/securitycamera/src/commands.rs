//! The commands the bot understands:
//!
//! ```text
//! /snap    sends the next camera frame as a photo
//! /info    reports the state of the machine and of the program
//! /pause   stops sending alarms
//! /resume  starts sending alarms again
//! /edit_config [json]  sends all the settings as JSON, or changes them to the JSON that follows (and saves them)
//! /reset_config  goes back to the default settings and deletes the saved ones
//! /alarms  lists the latest alarms and whether their GIF is recorded and sent (/queue is the old name)
//! /hd <code>   sends the GIF of the alarm with that code, like a reaction to the alarm message
//! /app     opens the mini app with the live view and the motion graph
//! /update  pulls, rebuilds and restarts the program
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gif::store::{Error as StoreError, Store};

use crate::alarms::Alarms;
use crate::config::Live;
use crate::edit_config::{self, Settings};
use crate::monitor::Monitor;
use crate::stream::Stream;
use crate::telegram::{Bot, Chat, Message};
use crate::{sysinfo, update};

/// How many alarms /alarms lists.
const ALARMS_SHOWN: usize = 10;

/// Stops the program: sets a flag and runs what has to be woken up (the
/// camera read that waits for a frame).
#[derive(Default)]
pub struct Shutdown {
    flag: AtomicBool,
    hooks: Mutex<Vec<Box<dyn Fn() + Send>>>,
}

impl Shutdown {
    pub fn new() -> Shutdown {
        Shutdown::default()
    }

    /// Runs `f` when the program is asked to stop.
    pub fn on_trigger(&self, f: impl Fn() + Send + 'static) {
        crate::util::lock(&self.hooks).push(Box::new(f));
    }

    pub fn trigger(&self) {
        self.flag.store(true, Ordering::SeqCst);
        for hook in crate::util::lock(&self.hooks).iter() {
            hook();
        }
    }

    /// The flag the loops that must stop look at.
    pub fn flag(&self) -> &AtomicBool {
        &self.flag
    }

    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// What the commands hand over to the main loop.
#[derive(Default)]
pub struct State {
    /// Set by /snap: the stream handler sends the next frame.
    pub snap_requested: AtomicBool,
    /// Set by /update once the new build is ready: main restarts when the
    /// stream has stopped.
    pub restart: AtomicBool,
}

/// What the commands work on.
pub struct Commands {
    pub chat: Arc<dyn Chat>,
    pub monitor: Arc<Monitor>,
    pub stream: Arc<Stream>,
    pub detectors: Vec<&'static str>,
    pub live: Arc<Live>,
    /// The settings before the saved ones were applied: /reset_config goes
    /// back to them.
    pub defaults: Settings,
    pub alarms: Arc<Alarms>,
    /// The public address of the mini app, "" if it is off.
    pub mini_app_url: String,
    pub settings_file: String,
    pub update_script: String,
    pub update_timeout: Duration,
    /// Stopping the stream is how /update makes main restart.
    pub shutdown: Arc<Shutdown>,
    pub state: Arc<State>,
    updating: AtomicBool,
}

/// What [`Commands::new`] needs besides the parts of the program.
pub struct Paths {
    pub settings_file: String,
    pub update_script: String,
    pub update_timeout: Duration,
}

impl Commands {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chat: Arc<dyn Chat>,
        monitor: Arc<Monitor>,
        stream: Arc<Stream>,
        detectors: Vec<&'static str>,
        live: Arc<Live>,
        defaults: Settings,
        alarms: Arc<Alarms>,
        mini_app_url: &str,
        paths: Paths,
        shutdown: Arc<Shutdown>,
        state: Arc<State>,
    ) -> Arc<Commands> {
        Arc::new(Commands {
            chat,
            monitor,
            stream,
            detectors,
            live,
            defaults,
            alarms,
            mini_app_url: mini_app_url.to_owned(),
            settings_file: paths.settings_file,
            update_script: paths.update_script,
            update_timeout: paths.update_timeout,
            shutdown,
            state,
            updating: AtomicBool::new(false),
        })
    }

    /// Makes the bot run the commands.
    pub fn register(self: &Arc<Self>, bot: &Bot) {
        let me = self.clone();
        bot.add_listener(move |m| me.handle(m));
    }

    /// Sends `text` without holding up the other messages, the work of a
    /// command can take a moment.
    fn reply(self: &Arc<Self>, command: &'static str, text: impl Into<String>) {
        let (me, text) = (self.clone(), text.into());
        thread::spawn(move || {
            if let Err(e) = me.chat.send_message(&text) {
                eprintln!("{command}: {e}");
            }
        });
    }

    fn config_json(&self) -> String {
        Settings::current(&self.monitor, &self.stream, &self.live).to_json()
    }

    /// Runs the command in the message, if it is one.
    pub fn handle(self: &Arc<Self>, m: &Message) {
        match m.command() {
            "snap" => self.state.snap_requested.store(true, Ordering::SeqCst),
            "info" => {
                // the report takes a moment to measure
                let me = self.clone();
                thread::spawn(move || {
                    let text = format!("{}\n\n{}", sysinfo::report("."), me.monitor.report());
                    me.reply("info", text);
                });
            }
            "pause" => {
                self.monitor.set_paused(true);
                self.reply("pause", "Alarms paused. Send /resume to turn them back on.");
            }
            "resume" => {
                self.monitor.set_paused(false);
                self.reply("resume", "Alarms resumed.");
            }
            "edit_config" => self.edit_config(m.payload()),
            "reset_config" => self.reset_config(),
            "alarms" | "queue" => self.reply("alarms", self.alarms.book.report(ALARMS_SHOWN)),
            "hd" => self.hd(&m.args()),
            "app" => self.app(),
            "update" => self.update(),
            _ => {}
        }
    }

    fn edit_config(self: &Arc<Self>, payload: &str) {
        if payload.is_empty() {
            return self.reply("edit_config", self.config_json());
        }
        if let Err(e) = edit_config::apply(
            payload,
            &self.monitor,
            &self.stream,
            &self.live,
            &self.detectors,
        ) {
            return self.reply("edit_config", format!("Not changed: {e}"));
        }
        let current = Settings::current(&self.monitor, &self.stream, &self.live);
        match edit_config::save(&self.settings_file, &current) {
            Ok(()) => self.reply("edit_config", format!("Settings changed and saved in {}:\n{}", self.settings_file, current.to_json())),
            Err(e) => self.reply("edit_config", format!("Settings changed, but not saved: they will be lost at the next restart ({e}):\n{}", current.to_json())),
        }
    }

    fn reset_config(self: &Arc<Self>) {
        match edit_config::reset(
            &self.settings_file,
            self.defaults.clone(),
            &self.monitor,
            &self.stream,
            &self.live,
            &self.detectors,
        ) {
            Ok(()) => self.reply(
                "reset_config",
                format!(
                    "Settings back to the defaults, {} deleted:\n{}",
                    self.settings_file,
                    self.config_json()
                ),
            ),
            Err(e) => self.reply("reset_config", format!("Not reset: {e}")),
        }
    }

    fn hd(self: &Arc<Self>, args: &[&str]) {
        let Some(&code) = args.first() else {
            return self.reply("hd", "Usage: /hd <code>, the code is in the alarm message.");
        };
        let Some(gifs) = self.alarms.gifs.clone() else {
            return self.reply("hd", "The alarm GIFs are not being saved.");
        };
        let (me, code) = (self.clone(), code.to_owned());
        // reading and uploading the GIF takes a while, don't hold up the other messages
        thread::spawn(move || me.send_hd(&gifs, &code));
    }

    fn send_hd(self: &Arc<Self>, gifs: &Store, code: &str) {
        let result = gifs.get(code).and_then(|data| {
            self.chat
                .send_animation(&data, &format!("GIF {}", code.to_lowercase()))
                .map_err(|e| StoreError::Io(std::io::Error::other(e)))
        });
        match result {
            Ok(()) => {}
            Err(StoreError::NotFound) => self.reply(
                "hd",
                format!("No GIF with code {code:?}: it may still be recording, or an old one deleted to keep the storage under the limit."),
            ),
            Err(e) => {
                eprintln!("hd: {e}");
                self.reply("hd", "Could not send the HD GIF.");
            }
        }
    }

    fn app(self: &Arc<Self>) {
        if self.mini_app_url.is_empty() {
            return self.reply(
                "app",
                "The mini app is off. Set MINI_APP_URL in .env to turn it on.",
            );
        }
        let me = self.clone();
        thread::spawn(move || {
            if let Err(e) = me.chat.send_web_app_button(
                "Live view and motion graph:",
                "Open the camera",
                &me.mini_app_url,
            ) {
                eprintln!("app: {e}");
                me.reply("app", "Could not send the button: Telegram only allows mini app buttons in a private chat with the bot. Open the bot's private chat and use the Camera button next to the message box.");
            }
        });
    }

    fn update(self: &Arc<Self>) {
        if self.updating.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        // the build takes a while, don't hold up the other messages
        thread::spawn(move || {
            let _ = me.chat.send_message("Updating...");
            // each step the script reports goes to the chat as it starts
            let chat = me.chat.clone();
            let (out, result) = update::build(&me.update_script, me.update_timeout, move |step| {
                let _ = chat.send_message(step);
            });
            match result {
                Err(e) => {
                    me.updating.store(false, Ordering::SeqCst);
                    let _ = me.chat.send_message(&format!(
                        "Update failed, still running the old version:\n{}",
                        update::tail(&format!("{out}\n{e}"), 3000)
                    ));
                }
                Ok(()) => {
                    let _ = me
                        .chat
                        .send_message(&format!("{}\nRestarting...", update::tail(&out, 3000)));
                    me.state.restart.store(true, Ordering::SeqCst);
                    me.shutdown.trigger();
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alarms::AlarmBook;
    use crate::alarms::tests::{FakeChat, Sent, fake_chat, temp_dir};
    use crate::clips::ClipRecorder;
    use crate::config;
    use crate::detector::{NAME_BACKGROUND, NAME_DIFFERENCE};
    use chrono::Local;
    use std::sync::mpsc::Receiver;
    use std::time::Instant;

    const WAIT: Duration = Duration::from_secs(10);

    struct Fixture {
        cmds: Arc<Commands>,
        rx: Receiver<Sent>,
        chat: Arc<FakeChat>,
        dir: std::path::PathBuf,
    }

    fn fixture(with_store: bool, mini_app: &str) -> Fixture {
        let (chat, rx) = fake_chat();
        let dir = temp_dir("commands");
        let monitor = Arc::new(Monitor::new(
            "",
            NAME_BACKGROUND,
            &[
                (NAME_BACKGROUND, config::BACKGROUND_THRESHOLD),
                (NAME_DIFFERENCE, config::DIFFERENCE_THRESHOLD),
            ],
            config::ALARM_COOLDOWN,
        ));
        monitor.set_detection_frames(config::DETECTION_FRAMES);
        let stream = Arc::new(Stream::new(config::STREAM_HISTORY));
        let live = Arc::new(Live::new());
        let defaults = Settings::current(&monitor, &stream, &live);
        let gifs = with_store.then(|| Arc::new(Store::new(dir.join("gifs"), 1 << 20).unwrap()));
        let alarms = Arc::new(Alarms {
            chat: chat.clone(),
            book: AlarmBook::new(),
            clips: ClipRecorder::new(),
            gifs,
        });
        let cmds = Commands::new(
            chat.clone(),
            monitor,
            stream,
            vec![NAME_BACKGROUND, NAME_DIFFERENCE],
            live,
            defaults,
            alarms,
            mini_app,
            Paths {
                settings_file: dir.join("settings.json").to_string_lossy().into_owned(),
                update_script: "/no/such/script".into(),
                update_timeout: Duration::from_secs(60),
            },
            Arc::new(Shutdown::new()),
            Arc::new(State::default()),
        );
        Fixture {
            cmds,
            rx,
            chat,
            dir,
        }
    }

    impl Fixture {
        fn say(&self, text: &str) {
            self.cmds.handle(&Message::new(text));
        }
        fn reply(&self) -> String {
            match self.rx.recv_timeout(WAIT).expect("no reply") {
                Sent::Text(t) => t,
                other => panic!("expected a text, got {other:?}"),
            }
        }
        fn silent(&self) {
            assert!(
                self.rx.recv_timeout(Duration::from_millis(150)).is_err(),
                "unexpected reply"
            );
        }
    }

    #[test]
    fn pause_and_resume() {
        let f = fixture(false, "");
        f.say("/pause");
        assert!(f.cmds.monitor.paused());
        assert!(f.reply().contains("Alarms paused"));
        f.say("/resume@mybot");
        assert!(!f.cmds.monitor.paused());
        assert_eq!(f.reply(), "Alarms resumed.");
    }

    #[test]
    fn snap_asks_the_stream_for_the_next_frame() {
        let f = fixture(false, "");
        f.say("/snap");
        assert!(f.cmds.state.snap_requested.load(Ordering::SeqCst));
        f.silent();
    }

    #[test]
    fn info_reports_the_machine_and_the_camera() {
        let f = fixture(false, "");
        f.say("/info");
        let r = f.reply();
        assert!(
            r.contains("Security camera process") && r.contains("Camera\n  device: default"),
            "{r}"
        );
    }

    #[test]
    fn other_messages_are_ignored() {
        let f = fixture(false, "");
        f.say("hello");
        f.say("/unknown");
        f.say("/");
        f.silent();
    }

    #[test]
    fn edit_config_shows_changes_and_saves() {
        let f = fixture(false, "");
        f.say("/edit_config");
        let shown = f.reply();
        assert!(shown.contains("\"detector\": \"background\""), "{shown}");

        f.say(r#"/edit_config {"detector": "difference", "snap_jpeg_quality": 50}"#);
        let r = f.reply();
        assert!(r.starts_with("Settings changed and saved in "), "{r}");
        assert_eq!(f.cmds.monitor.detector(), NAME_DIFFERENCE);
        assert_eq!(f.cmds.live.get().snap_jpeg_quality, 50);
        let saved = std::fs::read_to_string(&f.cmds.settings_file).unwrap();
        assert!(saved.contains("\"snap_jpeg_quality\": 50"), "{saved}");

        f.say(r#"/edit_config {"snap_jpeg_quality": 0}"#);
        assert!(f.reply().starts_with("Not changed: snap_jpeg_quality"));
        assert_eq!(f.cmds.live.get().snap_jpeg_quality, 50);
    }

    #[test]
    fn edit_config_says_when_it_could_not_save() {
        let mut f = fixture(false, "");
        Arc::get_mut(&mut f.cmds).unwrap().settings_file = "/no/such/dir/settings.json".into();
        f.say(r#"/edit_config {"snap_jpeg_quality": 40}"#);
        let r = f.reply();
        assert!(r.starts_with("Settings changed, but not saved"), "{r}");
        assert_eq!(
            f.cmds.live.get().snap_jpeg_quality,
            40,
            "it is changed all the same"
        );
    }

    #[test]
    fn reset_config_goes_back_and_deletes_the_file() {
        let f = fixture(false, "");
        f.say(r#"/edit_config {"detector": "difference"}"#);
        f.reply();
        assert!(std::path::Path::new(&f.cmds.settings_file).exists());
        f.say("/reset_config");
        let r = f.reply();
        assert!(r.starts_with("Settings back to the defaults"), "{r}");
        assert_eq!(f.cmds.monitor.detector(), NAME_BACKGROUND);
        assert!(!std::path::Path::new(&f.cmds.settings_file).exists());
    }

    #[test]
    fn alarms_lists_them_under_both_names() {
        let f = fixture(false, "");
        for name in ["/alarms", "/queue"] {
            f.say(name);
            assert_eq!(f.reply(), "No alarms yet.");
        }
        f.cmds
            .alarms
            .book
            .begin(Local::now(), "background", 0.5, "abc123");
        f.say("/alarms");
        let r = f.reply();
        assert!(
            r.starts_with("Latest 1 alarms") && r.contains("recording the GIF"),
            "{r}"
        );
    }

    #[test]
    fn hd_sends_the_gif_with_that_code() {
        let f = fixture(true, "");
        f.say("/hd");
        assert!(f.reply().starts_with("Usage: /hd"));

        f.say("/hd abc123");
        let r = f.reply();
        assert!(r.contains("No GIF with code \"abc123\""), "{r}");

        f.cmds
            .alarms
            .gifs
            .as_ref()
            .unwrap()
            .save("abc123", b"GIF89a-data")
            .unwrap();
        f.say("/hd ABC123");
        match f.rx.recv_timeout(WAIT).unwrap() {
            Sent::Animation(data, caption) => assert_eq!(
                (data.as_slice(), caption.as_str()),
                (&b"GIF89a-data"[..], "GIF abc123")
            ),
            other => panic!("{other:?}"),
        }

        *crate::util::lock(&f.chat.fail) = true;
        f.say("/hd abc123");
        f.silent(); // the failure message can't be sent either, and nothing crashes
    }

    #[test]
    fn hd_says_when_gifs_are_not_saved() {
        let f = fixture(false, "");
        f.say("/hd abc123");
        assert_eq!(f.reply(), "The alarm GIFs are not being saved.");
    }

    #[test]
    fn hd_reports_a_failed_upload() {
        let f = fixture(true, "");
        f.cmds
            .alarms
            .gifs
            .as_ref()
            .unwrap()
            .save("abc123", b"x")
            .unwrap();
        // the animation can't be sent but texts can: a chat that only fails uploads
        struct NoUploads(Arc<FakeChat>);
        impl Chat for NoUploads {
            fn send(&self, t: &str) -> Result<i64, String> {
                self.0.send(t)
            }
            fn send_photo(&self, _: &[u8], _: &str) -> Result<(), String> {
                Err("down".into())
            }
            fn send_animation(&self, _: &[u8], _: &str) -> Result<(), String> {
                Err("down".into())
            }
            fn send_animation_reply(&self, _: &[u8], _: &str, _: i64) -> Result<(), String> {
                Err("down".into())
            }
            fn send_web_app_button(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
                Err("down".into())
            }
        }
        let Fixture { cmds, rx, chat, .. } = f;
        let mut cmds = Arc::try_unwrap(cmds).ok().expect("one owner");
        cmds.chat = Arc::new(NoUploads(chat));
        let cmds = Arc::new(cmds);
        let text = |rx: &Receiver<Sent>| match rx.recv_timeout(WAIT).expect("no reply") {
            Sent::Text(t) => t,
            other => panic!("{other:?}"),
        };
        cmds.handle(&Message::new("/hd abc123"));
        assert_eq!(text(&rx), "Could not send the HD GIF.");
        cmds.handle(&Message::new("/app"));
        assert_eq!(
            text(&rx),
            "The mini app is off. Set MINI_APP_URL in .env to turn it on."
        );
    }

    #[test]
    fn app_sends_the_button_or_says_it_is_off() {
        let f = fixture(false, "");
        f.say("/app");
        assert!(f.reply().starts_with("The mini app is off"));
        let f = fixture(false, "https://cam.ts.net");
        f.say("/app");
        assert_eq!(
            f.rx.recv_timeout(WAIT).unwrap(),
            Sent::Button(
                "Live view and motion graph:".into(),
                "Open the camera".into(),
                "https://cam.ts.net".into()
            )
        );
        *crate::util::lock(&f.chat.fail) = true;
        f.say("/app");
        f.silent();
    }

    fn script(f: &Fixture, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = f.dir.join("update.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn a_failed_update_keeps_running_and_can_be_retried() {
        let mut f = fixture(false, "");
        let script = script(&f, "echo compiling\necho error: nope >&2\nexit 1\n");
        Arc::get_mut(&mut f.cmds).unwrap().update_script = script;
        f.say("/update");
        assert_eq!(f.reply(), "Updating...");
        let r = f.reply();
        assert!(
            r.starts_with("Update failed, still running the old version:\ncompiling\nerror: nope"),
            "{r}"
        );
        assert!(!f.cmds.state.restart.load(Ordering::SeqCst));
        assert!(!f.cmds.shutdown.is_set());
        // it can be tried again
        f.say("/update");
        assert_eq!(f.reply(), "Updating...");
    }

    #[test]
    fn an_update_tells_its_steps_while_it_runs() {
        let mut f = fixture(false, "");
        let script = script(
            &f,
            "echo '@@ Pulling...'\necho '@@ Building...'\necho 'updated a -> b'\n",
        );
        Arc::get_mut(&mut f.cmds).unwrap().update_script = script;
        f.say("/update");
        assert_eq!(f.reply(), "Updating...");
        assert_eq!(f.reply(), "Pulling...");
        assert_eq!(f.reply(), "Building...");
        assert_eq!(f.reply(), "updated a -> b\nRestarting...");
    }

    #[test]
    fn a_good_update_stops_the_stream_to_restart() {
        let mut f = fixture(false, "");
        let script = script(&f, "echo 'updated a -> b'\n");
        Arc::get_mut(&mut f.cmds).unwrap().update_script = script;
        let woken = Arc::new(AtomicBool::new(false));
        let w = woken.clone();
        f.cmds
            .shutdown
            .on_trigger(move || w.store(true, Ordering::SeqCst));
        f.say("/update");
        assert_eq!(f.reply(), "Updating...");
        assert_eq!(f.reply(), "updated a -> b\nRestarting...");
        let deadline = Instant::now() + WAIT;
        while !f.cmds.state.restart.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        assert!(f.cmds.shutdown.is_set() && woken.load(Ordering::SeqCst));
        // asking again while it restarts does nothing
        f.say("/update");
        f.silent();
    }

    #[test]
    fn a_message_from_telegram_reaches_the_commands() {
        use crate::httpd::{self, Request, Response};
        let f = fixture(false, "");
        let calls = Arc::new(AtomicBool::new(false));
        let c = calls.clone();
        let addr = httpd::spawn(
            "127.0.0.1:0",
            1 << 20,
            Arc::new(move |_: &Request| {
                let body = if c.swap(true, Ordering::SeqCst) {
                    r#"{"ok":true,"result":[]}"#
                } else {
                    r#"{"ok":true,"result":[{"update_id":1,"message":{"message_id":1,"date":1,"text":"/pause","chat":{"id":5},"from":{"username":"bob"}}}]}"#
                };
                Response::new(200, "application/json", body)
            }),
        )
        .unwrap();
        let mut bot = Bot::with_api("T", "5", &format!("http://{addr}")).unwrap();
        bot.set_poll_timeout(Duration::ZERO);
        f.cmds.register(&bot);
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let t = thread::spawn(move || bot.run(&s));
        let deadline = Instant::now() + WAIT;
        while !f.cmds.monitor.paused() {
            assert!(Instant::now() < deadline, "/pause never ran");
            thread::sleep(Duration::from_millis(5));
        }
        stop.store(true, Ordering::SeqCst);
        t.join().unwrap();
    }

    #[test]
    fn shutdown_sets_the_flag_and_wakes_the_hooks() {
        let s = Shutdown::new();
        let n = Arc::new(Mutex::new(0));
        for _ in 0..2 {
            let n = n.clone();
            s.on_trigger(move || *n.lock().unwrap() += 1);
        }
        assert!(!s.is_set());
        s.trigger();
        assert!(s.is_set() && s.flag().load(Ordering::SeqCst));
        assert_eq!(*n.lock().unwrap(), 2);
    }
}
