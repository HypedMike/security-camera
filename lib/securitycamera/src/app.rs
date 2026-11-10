//! Wires everything together: the camera, the detectors, Telegram, the mini
//! app and the periodic jobs. [`run`] is the program.

use std::io;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Days, Local, TimeDelta};
use jpeg_encoder::{ColorType, Encoder};
use my_video::{Device, Frame};

use crate::alarms::{self, AlarmBook, Alarms};
use crate::clips::ClipRecorder;
use crate::commands::{Commands, Paths, Shutdown, State};
use crate::config::{self, Live};
use crate::detector::{
    BackgroundDetector, Detector, FrameDifference, NAME_BACKGROUND, NAME_DIFFERENCE,
};
use crate::edit_config::{self, Settings};
use crate::env;
use crate::livegraph::{self, Graph};
use crate::miniapp::{self, Access, Members};
use crate::monitor::Monitor;
use crate::stream::{Stream, Tick};
use crate::sysinfo;
use crate::syswatch::{self, Limits, Reading};
use crate::telegram::{Bot, Chat};
use crate::update;
use crate::util::hostname;

const USAGE: &str = "usage: securitycamera [-fetch-camera]\n  -fetch-camera  list the cameras again and choose one, even if one is already saved";

/// The program: reads the settings, opens the camera and runs until it is
/// stopped (Ctrl+C) or updated through Telegram.
pub fn run() -> Result<(), String> {
    let mut fetch_camera = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-fetch-camera" | "--fetch-camera" => fetch_camera = true,
            "-h" | "-help" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    sysinfo::init();

    env::load(config::ENV_FILE).map_err(|e| e.to_string())?;
    {
        let stdin = io::stdin();
        env::ensure(
            config::ENV_FILE,
            &mut stdin.lock(),
            &mut io::stdout(),
            &[
                env::Var {
                    key: "TELEGRAM_BOT_TOKEN",
                    prompt: "Telegram bot token from @BotFather",
                    validate: None,
                },
                env::Var {
                    key: "TELEGRAM_CHAT_ID",
                    prompt: "Telegram chat id",
                    validate: None,
                },
            ],
        )?;
    }
    let token = env::get("TELEGRAM_BOT_TOKEN");
    let chat_id = env::get("TELEGRAM_CHAT_ID");
    let bot = Arc::new(Bot::new(&token, &chat_id).map_err(|e| format!("{e} (set them in .env)"))?);
    let chat: Arc<dyn Chat> = bot.clone();

    let print = Printing {
        scores: env::bool("PRINT_DIFFERENCES"),
        alarms: env::bool("PRINT_ALARMS"),
    };
    if env::bool("PRINT_TELEGRAM_MESSAGES") {
        bot.add_listener(|m| println!("Telegram message from {}: {}", m.from, m.text));
    }

    let shutdown = Arc::new(Shutdown::new());
    {
        let shutdown = shutdown.clone();
        ctrlc::set_handler(move || {
            // a second Ctrl+C does not wait for the camera loop
            if shutdown.is_set() {
                std::process::exit(130);
            }
            shutdown.trigger();
        })
        .map_err(|e| format!("signal handler: {e}"))?;
    }

    // ALARM_THRESHOLD is the name the difference threshold had before there
    // were several detectors
    let background_threshold = env_threshold("BACKGROUND_THRESHOLD", config::BACKGROUND_THRESHOLD)?;
    let difference_threshold = env_threshold(
        "DIFFERENCE_THRESHOLD",
        env_threshold("ALARM_THRESHOLD", config::DIFFERENCE_THRESHOLD)?,
    )?;
    let detector_name = match env::get("DETECTOR") {
        d if d.is_empty() => config::DEFAULT_DETECTOR.to_owned(),
        d => d,
    };

    let device_id = pick_camera(fetch_camera)?;
    let monitor = Arc::new(Monitor::new(
        &device_id,
        &detector_name,
        &[
            (NAME_BACKGROUND, background_threshold),
            (NAME_DIFFERENCE, difference_threshold),
        ],
        config::ALARM_COOLDOWN,
    ));
    monitor.set_detection_frames(config::DETECTION_FRAMES);

    let names = [NAME_BACKGROUND, NAME_DIFFERENCE];
    if !names.contains(&detector_name.as_str()) {
        return Err(format!(
            "DETECTOR: unknown detector {detector_name:?}, use one of: {}",
            names.join(", ")
        ));
    }
    let detectors: Vec<Box<dyn Detector>> = {
        let m = monitor.clone();
        vec![
            Box::new(BackgroundDetector::new()),
            Box::new(FrameDifference::new(move || m.detection_frames())),
        ]
    };

    // if the chosen camera can't be opened, the others are tried
    let capture = Arc::new(my_video::open_any(
        &my_video::Config {
            width: 640,
            height: 480,
            warmup_frames: 30,
            lock_exposure: env::bool("CAMERA_LOCK_EXPOSURE"),
        },
        &device_id,
    )?);
    {
        // a Weak: the camera is released when main drops it, to restart
        let weak = Arc::downgrade(&capture);
        shutdown.on_trigger(move || {
            if let Some(c) = weak.upgrade() {
                c.stop();
            }
        });
    }

    // the mini app is only served when MINI_APP_URL is set. A mistake in its
    // settings turns it off and is reported on Telegram: it must not stop the
    // camera
    let mut mini_app_url = String::new();
    let mut mini_app_access = None;
    let mut mini_app_problem = String::new();
    match miniapp::parse_url(&env::get("MINI_APP_URL")).map_err(|e| format!("MINI_APP_URL: {e}")) {
        Ok(url) if url.is_empty() => {}
        Ok(url) => match Access::new(&chat_id, Some(bot.clone() as Arc<dyn Members>)) {
            Ok(access) => {
                mini_app_url = url;
                mini_app_access = Some(access);
            }
            Err(e) => mini_app_problem = format!("The mini app is off: {e}"),
        },
        Err(e) => mini_app_problem = format!("The mini app is off: {e}"),
    }
    if !mini_app_problem.is_empty() {
        eprintln!("{mini_app_problem}");
    }

    // the live graph page is only served when LIVE_STREAM_GRAPH is set; the
    // mini app plots the same scores
    let graph = (env::bool("LIVE_STREAM_GRAPH") || !mini_app_url.is_empty())
        .then(|| Arc::new(Graph::new()));
    let mut app = None;
    if let Some(access) = mini_app_access {
        let server = miniapp::Server::new(miniapp::Options {
            token: token.clone(),
            allowed: Box::new(move |id| access.allowed(id)),
            max_age: config::MINI_APP_AUTH_MAX_AGE,
            max_viewers: config::MINI_APP_MAX_VIEWERS,
            max_stream: config::MINI_APP_MAX_STREAM,
            events: graph.clone(),
        });
        match server.listen(config::MINI_APP_ADDR) {
            Ok(_) => {
                println!(
                    "mini app listening on http://{}, public address {mini_app_url}",
                    config::MINI_APP_ADDR
                );
                app = Some(server);
            }
            Err(e) => {
                eprintln!("mini app: {e}");
                mini_app_problem = format!("The mini app is off: {e}");
                mini_app_url.clear();
            }
        }
    }
    if env::bool("LIVE_STREAM_GRAPH") {
        let graph = graph.clone().expect("made for LIVE_STREAM_GRAPH");
        livegraph::listen(graph, config::GRAPH_ADDR)
            .map_err(|e| format!("live graph on {}: {e}", config::GRAPH_ADDR))?;
        println!(
            "live graph at http://localhost:{}",
            config::GRAPH_ADDR.rsplit(':').next().unwrap_or("")
        );
    }

    let stream = Arc::new(Stream::new(config::STREAM_HISTORY));

    if let Some(app) = app {
        // the camera frames for the mini app's video, only made while somebody watches
        let mut last: Option<Instant> = None;
        stream.add_handler(move |t| {
            if !app.wanted()
                || last.is_some_and(|l| {
                    t.mono.duration_since(l) < Duration::from_secs(1) / config::MINI_APP_FPS
                })
            {
                return;
            }
            last = Some(t.mono);
            if let Ok(jpeg) = encode_jpeg(t.frame, config::MINI_APP_JPEG_QUALITY) {
                app.push_frame(&jpeg);
            }
        });
    }

    // without the store the alarms still work, they just have no GIF
    let gifs = match gif::store::Store::new(config::GIF_DIR, config::GIF_STORAGE_LIMIT) {
        Ok(s) => Some(Arc::new(s)),
        Err(e) => {
            eprintln!("gifs: the alarm GIFs will not be saved: {e}");
            None
        }
    };

    let live = Arc::new(Live::new());

    // the saved settings (settings.json, written by /edit_config) override the
    // defaults from the constants and the .env file. A bad file is reported and
    // ignored: it must not stop the camera
    let defaults = Settings::current(&monitor, &stream, &live);
    if let Err(e) = edit_config::load_saved(config::SETTINGS_FILE, &monitor, &stream, &live, &names)
    {
        let problem = format!("The saved settings were not loaded, using the defaults: {e}");
        eprintln!("{problem}");
        let chat = chat.clone();
        thread::spawn(move || {
            if let Err(e) = chat.send_message(&problem) {
                eprintln!("settings: {e}");
            }
        });
    }

    // every alarm is told at once; its GIF is recorded in the background and
    // sent to whoever reacts to the message
    let alarms = Arc::new(Alarms {
        chat: chat.clone(),
        book: AlarmBook::new(),
        clips: ClipRecorder::new(),
        gifs,
    });
    {
        let alarms = alarms.clone();
        bot.add_reaction_listener(move |r| alarms::on_reaction(&alarms, r));
    }
    let state = Arc::new(State::default());
    Commands::new(
        chat.clone(),
        monitor.clone(),
        stream.clone(),
        names.to_vec(),
        live.clone(),
        defaults,
        alarms.clone(),
        &mini_app_url,
        Paths {
            settings_file: config::SETTINGS_FILE.into(),
            update_script: config::UPDATE_SCRIPT.into(),
            update_timeout: config::UPDATE_TIMEOUT,
        },
        shutdown.clone(),
        state.clone(),
    )
    .register(&bot);

    {
        let monitor = monitor.clone();
        stream.add_handler(move |t| monitor.frame(t.wall));
    }

    // before the detector's handler, which starts a recording with the frame
    // that set the alarm off
    {
        let (alarms, live) = (alarms.clone(), live.clone());
        stream.add_handler(move |t| {
            let v = live.get();
            alarms.clips.frame(
                t.frame,
                t.mono,
                v.hd_gif_width,
                config::ALARM_GIF_PRE,
                Duration::from_secs(1) / v.alarm_gif_fps.max(1),
            );
        });
    }

    {
        let (state, live, chat) = (state.clone(), live.clone(), chat.clone());
        stream.add_handler(move |t| {
            if !state.snap_requested.swap(false, Ordering::SeqCst) {
                return;
            }
            // encode now: the frame is only valid until this handler returns
            let jpeg = encode_jpeg(t.frame, live.get().snap_jpeg_quality);
            let caption = format!("Snapshot at {}", t.wall.format("%H:%M:%S"));
            let chat = chat.clone();
            thread::spawn(move || {
                if let Err(e) = jpeg.and_then(|j| chat.send_photo(&j, &caption)) {
                    eprintln!("snap: {e}");
                }
            });
        });
    }

    stream.add_handler(detection_handler(
        monitor.clone(),
        graph.clone(),
        alarms.clone(),
        live.clone(),
        detectors,
        print,
    ));

    {
        let (bot, stop) = (bot.clone(), shutdown.clone());
        thread::spawn(move || bot.run(stop.flag()));
    }

    {
        let (chat, monitor, stop) = (chat.clone(), monitor.clone(), shutdown.clone());
        thread::spawn(move || {
            dailyreport::run(stop.flag(), config::DAILY_REPORT_HOUR, |now| {
                send_daily_report(&*chat, &monitor, now, "", "", weather_coordinates(env::get))
            })
        });
    }

    {
        let (bot, stop) = (bot.clone(), shutdown.clone());
        let limits = Limits {
            disk_percent: config::DISK_ALERT_PERCENT,
            cpu_percent: config::CPU_ALERT_PERCENT,
            cpu_checks: config::CPU_ALERT_CHECKS,
            temp_c: config::TEMP_ALERT_C,
            offline_checks: config::OFFLINE_ALERT_CHECKS,
        };
        let sender = bot.clone();
        thread::spawn(move || {
            syswatch::run(
                stop.flag(),
                config::SYSTEM_CHECK_INTERVAL,
                limits,
                || Reading {
                    // the disk that holds the GIFs and the .env file
                    disk: sysinfo::disk_used_percent("."),
                    cpu: sysinfo::cpu_percent(),
                    temp: sysinfo::max_temperature(),
                    online: bot.reachable(),
                },
                |text| sender.send_message(text),
            )
        });
    }

    if !mini_app_url.is_empty() {
        // the menu button of the private chats with the bot, which is where the
        // mini app opens from when TELEGRAM_CHAT_ID is a group
        let (bot, url) = (bot.clone(), mini_app_url.clone());
        thread::spawn(move || {
            if let Err(e) = bot.set_menu_web_app("Camera", &url) {
                eprintln!("mini app menu button: {e}");
            }
        });
    }

    // the camera is open and the stream is about to start
    {
        let chat = chat.clone();
        thread::spawn(move || {
            if let Err(e) = chat.send_message(&format!(
                "Security camera is up and running on {}",
                hostname()
            )) {
                eprintln!("{e}");
            }
            if !mini_app_problem.is_empty()
                && let Err(e) = chat.send_message(&mini_app_problem)
            {
                eprintln!("{e}");
            }
        });
    }

    let mut source = capture;
    let result = stream.run(&mut source, shutdown.flag());
    // release the camera, so a new process can open it
    drop(source);
    result?;

    if state.restart.load(Ordering::SeqCst) {
        let e = update::restart();
        return Err(format!("restart: {e}"));
    }
    Ok(())
}

/// What to print on the console for every alarm and every score.
#[derive(Clone, Copy, Default)]
pub struct Printing {
    pub alarms: bool,
    pub scores: bool,
}

/// The handler that scores every frame with the detector in use and sets off
/// the alarm. The detector in use is only touched by this handler;
/// /edit_config switches it through the monitor, and the newly chosen one
/// starts learning from scratch.
pub fn detection_handler(
    monitor: Arc<Monitor>,
    graph: Option<Arc<Graph>>,
    alarms: Arc<Alarms>,
    live: Arc<Live>,
    mut detectors: Vec<Box<dyn Detector>>,
    print: Printing,
) -> impl FnMut(&Tick) + Send + 'static {
    let mut current: Option<usize> = None;
    move |t| {
        let name = monitor.detector();
        if current.is_none_or(|i| detectors[i].name() != name) {
            let Some(i) = detectors.iter().position(|d| d.name() == name) else {
                return;
            };
            detectors[i].reset();
            current = Some(i);
        }
        let detector = &mut detectors[current.expect("set above")];
        let score = match detector.score(t.frame, t.history) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("detection: {e}");
                return;
            }
        };
        if let Some(graph) = &graph {
            graph.push(score);
        }
        let threshold = monitor.threshold_of(&name);
        if !monitor.paused() && score >= threshold && cooldown_over(&monitor, t.wall) {
            monitor.alarm(t.wall);
            if print.alarms {
                println!(
                    "{} alarm: {name} score {score:.4}",
                    t.wall.format("%H:%M:%S%.3f")
                );
            }
            alarms::start_alarm(
                &alarms,
                t.wall,
                t.mono,
                t.frame,
                &name,
                score,
                threshold,
                live.get(),
            );
        }
        // after the alarm, so the clip it starts has the score of its first frame
        alarms.clips.score(t.mono, score);
        if print.scores {
            println!("{} {name} score: {score:.4}", t.wall.format("%H:%M:%S%.3f"));
        }
    }
}

/// Whether enough time went by since the last alarm for another one.
fn cooldown_over(monitor: &Monitor, now: DateTime<Local>) -> bool {
    monitor.last_alarm().is_none_or(|last| {
        now - last > TimeDelta::from_std(monitor.cooldown()).unwrap_or(TimeDelta::MAX)
    })
}

/// Encodes a camera frame as a JPEG.
pub fn encode_jpeg(frame: &Frame, quality: u8) -> Result<Vec<u8>, String> {
    let (w, h) = (
        u16::try_from(frame.width).map_err(|e| e.to_string())?,
        u16::try_from(frame.height).map_err(|e| e.to_string())?,
    );
    let mut out = Vec::new();
    Encoder::new(&mut out, quality)
        .encode(&frame.pixels, w, h, ColorType::Rgba)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// Sends the daily "i'm alive" message with the alarms of the night, from
/// `NIGHT_START_HOUR` of the previous evening until now. The weather is for
/// `coordinates`; it is left out if the forecast can't be fetched. `api` is
/// where the forecast comes from, "" for Open-Meteo. The events of the day
/// come from the calendar API at `calendar_api`, "" for the parish website;
/// they are left out if they can't be fetched.
pub fn send_daily_report(
    chat: &dyn Chat,
    monitor: &Monitor,
    now: DateTime<Local>,
    api: &str,
    calendar_api: &str,
    (lat, lon): (f64, f64),
) {
    let weather = match dailyreport::fetch_weather(api, lat, lon, Duration::from_secs(15)) {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("daily report: {e}");
            None
        }
    };
    let events = dailyreport::fetch_events(calendar_api, now.date_naive(), Duration::from_secs(15))
        .unwrap_or_else(|e| {
            eprintln!("daily report: {e}");
            Vec::new()
        });
    let yesterday = now
        .date_naive()
        .checked_sub_days(Days::new(1))
        .unwrap_or(now.date_naive());
    let night_start = dailyreport::at_hour(yesterday, config::NIGHT_START_HOUR);
    let msg = dailyreport::compose(
        now,
        &monitor.alarms_since(night_start),
        monitor.paused(),
        weather,
        &events,
    );
    if let Err(e) = chat.send_message(&msg) {
        eprintln!("daily report: {e}");
    }
}

/// The coordinates in WEATHER_LATITUDE and WEATHER_LONGITUDE (read with
/// `get`), or those of Lucca if either is unset or not a valid
/// latitude/longitude.
pub fn weather_coordinates(get: impl Fn(&str) -> String) -> (f64, f64) {
    let (lat, lon) = (get("WEATHER_LATITUDE"), get("WEATHER_LONGITUDE"));
    let lucca = (config::DEFAULT_LATITUDE, config::DEFAULT_LONGITUDE);
    if lat.is_empty() && lon.is_empty() {
        return lucca;
    }
    match (lat.trim().parse::<f64>(), lon.trim().parse::<f64>()) {
        (Ok(lat), Ok(lon)) if lat.abs() <= 90.0 && lon.abs() <= 180.0 => (lat, lon),
        _ => {
            eprintln!(
                "daily report: WEATHER_LATITUDE/WEATHER_LONGITUDE are not valid coordinates, using Lucca"
            );
            lucca
        }
    }
}

/// The alarm threshold in the variable `key`, or `default` if it is not set.
fn env_threshold(key: &str, default: f64) -> Result<f64, String> {
    threshold_from(key, &env::get(key), default)
}

fn threshold_from(key: &str, value: &str, default: f64) -> Result<f64, String> {
    if value.is_empty() {
        return Ok(default);
    }
    match value.trim().parse::<f64>() {
        Ok(t) if (0.0..=1.0).contains(&t) => Ok(t),
        _ => Err(format!("{key}: {value:?} is not a number between 0 and 1")),
    }
}

/// The id of the camera to use. The choice is kept in CAMERA_ID; it is asked
/// again only when `refetch` is set, nothing is saved yet or the saved camera
/// is gone. With a single camera there is nothing to ask.
fn pick_camera(refetch: bool) -> Result<String, String> {
    let devices = my_video::devices();
    pick_from(
        &devices,
        &env::get("CAMERA_ID"),
        refetch,
        |labels| {
            env::choose(
                io::stdin(),
                &mut io::stdout(),
                "Several cameras found:",
                labels,
                config::CAMERA_CHOICE_TIMEOUT,
            )
        },
        |label| {
            // the label is saved, not the id: the driver gives a new random id to
            // every camera at every start, so a saved id never matched again
            env::save(config::ENV_FILE, "CAMERA_ID", label).map_err(|e| e.to_string())?;
            env::set("CAMERA_ID", label);
            Ok(())
        },
    )
}

fn pick_from(
    devices: &[Device],
    saved: &str,
    refetch: bool,
    choose: impl FnOnce(&[String]) -> usize,
    save: impl FnOnce(&str) -> Result<(), String>,
) -> Result<String, String> {
    match devices {
        [] => {
            // let open try the default camera and report the driver's own error
            eprintln!("no camera listed, trying the default one");
            return Ok(String::new());
        }
        [only] => return Ok(only.id.clone()),
        _ => {}
    }
    if !saved.is_empty() && !refetch {
        if let Some(d) = my_video::find(devices, saved) {
            return Ok(d.id.clone());
        }
        println!("saved camera not found, choosing again");
    }
    let labels: Vec<String> = devices.iter().map(|d| d.label.clone()).collect();
    let chosen = &devices[choose(&labels).min(devices.len() - 1)];
    save(&chosen.label)?;
    Ok(chosen.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alarms::tests::{Sent, fake_chat};
    use crate::httpd::{self, Request, Response};
    use chrono::TimeZone;
    use std::sync::Mutex;

    fn dev(id: &str, label: &str) -> Device {
        Device {
            id: id.into(),
            label: label.into(),
        }
    }

    fn pick(
        devices: &[Device],
        saved: &str,
        refetch: bool,
        answer: usize,
    ) -> (Result<String, String>, Option<String>, bool) {
        let asked = std::cell::Cell::new(false);
        let saved_label = std::cell::RefCell::new(None);
        let r = pick_from(
            devices,
            saved,
            refetch,
            |labels| {
                asked.set(true);
                assert_eq!(labels.len(), devices.len());
                answer
            },
            |label| {
                *saved_label.borrow_mut() = Some(label.to_owned());
                Ok(())
            },
        );
        (r, saved_label.into_inner(), asked.get())
    }

    #[test]
    fn no_camera_listed_tries_the_default() {
        assert_eq!(pick(&[], "", false, 0), (Ok(String::new()), None, false));
    }

    #[test]
    fn a_single_camera_is_not_asked_about() {
        assert_eq!(
            pick(&[dev("7", "Cam")], "other", false, 0),
            (Ok("7".into()), None, false)
        );
    }

    #[test]
    fn the_saved_camera_is_used_without_asking() {
        let devs = [dev("1", "A"), dev("2", "B")];
        assert_eq!(
            pick(&devs, "B", false, 0),
            (Ok("2".into()), None, false),
            "by label"
        );
        assert_eq!(
            pick(&devs, "1", false, 1),
            (Ok("1".into()), None, false),
            "by an older id"
        );
    }

    #[test]
    fn the_camera_is_asked_and_its_label_saved() {
        let devs = [dev("1", "A"), dev("2", "B")];
        assert_eq!(
            pick(&devs, "", false, 1),
            (Ok("2".into()), Some("B".into()), true),
            "nothing saved yet"
        );
        assert_eq!(
            pick(&devs, "gone", false, 0),
            (Ok("1".into()), Some("A".into()), true),
            "the saved one is gone"
        );
        assert_eq!(
            pick(&devs, "B", true, 0),
            (Ok("1".into()), Some("A".into()), true),
            "-fetch-camera"
        );
        assert_eq!(
            pick(&devs, "", false, 99).0,
            Ok("2".into()),
            "an answer out of range is the last one"
        );
    }

    #[test]
    fn a_camera_that_can_not_be_saved_is_an_error() {
        let devs = [dev("1", "A"), dev("2", "B")];
        let r = pick_from(&devs, "", false, |_| 0, |_| Err("read-only".into()));
        assert_eq!(r, Err("read-only".into()));
    }

    #[test]
    fn thresholds_come_from_the_environment() {
        assert_eq!(threshold_from("K", "", 0.5), Ok(0.5));
        assert_eq!(threshold_from("K", "0.25", 0.5), Ok(0.25));
        assert_eq!(threshold_from("K", " 1 ", 0.5), Ok(1.0));
        for bad in ["abc", "-0.1", "1.5", "NaN", "inf"] {
            let e = threshold_from("BACKGROUND_THRESHOLD", bad, 0.5).unwrap_err();
            assert!(
                e.contains("BACKGROUND_THRESHOLD") && e.contains("between 0 and 1"),
                "{bad}: {e}"
            );
        }
    }

    #[test]
    fn the_weather_is_for_lucca_unless_told_otherwise() {
        let lucca = (config::DEFAULT_LATITUDE, config::DEFAULT_LONGITUDE);
        let with = |lat: &'static str, lon: &'static str| {
            weather_coordinates(move |k| match k {
                "WEATHER_LATITUDE" => lat.into(),
                "WEATHER_LONGITUDE" => lon.into(),
                _ => String::new(),
            })
        };
        assert_eq!(with("", ""), lucca);
        assert_eq!(with("45.46", "9.19"), (45.46, 9.19));
        assert_eq!(with("-33.9", "-151.2"), (-33.9, -151.2));
        for (lat, lon) in [
            ("45.46", ""),
            ("", "9.19"),
            ("abc", "9"),
            ("91", "0"),
            ("0", "181"),
        ] {
            assert_eq!(with(lat, lon), lucca, "{lat:?} {lon:?}");
        }
    }

    #[test]
    fn frames_are_encoded_as_jpeg() {
        let frame = Frame {
            width: 16,
            height: 8,
            pixels: [200, 100, 50, 255].repeat(128),
        };
        let jpeg = encode_jpeg(&frame, 85).unwrap();
        assert_eq!(&jpeg[..2], [0xff, 0xd8]);
        assert_eq!(&jpeg[jpeg.len() - 2..], [0xff, 0xd9]);
        assert!(
            encode_jpeg(&Frame::default(), 85).is_err(),
            "an empty frame"
        );
        let smaller = encode_jpeg(
            &Frame {
                width: 64,
                height: 64,
                pixels: (0..64 * 64 * 4).map(|i| (i * 7 % 251) as u8).collect(),
            },
            10,
        )
        .unwrap();
        let bigger = encode_jpeg(
            &Frame {
                width: 64,
                height: 64,
                pixels: (0..64 * 64 * 4).map(|i| (i * 7 % 251) as u8).collect(),
            },
            95,
        )
        .unwrap();
        assert!(smaller.len() < bigger.len(), "quality counts");
    }

    #[test]
    fn the_cooldown_spaces_the_alarms() {
        let m = Monitor::new("", NAME_BACKGROUND, &[], Duration::from_secs(5));
        let now = Local::now();
        assert!(cooldown_over(&m, now), "no alarm yet");
        m.alarm(now);
        assert!(
            !cooldown_over(&m, now + TimeDelta::seconds(5)),
            "exactly the cooldown is not enough"
        );
        assert!(cooldown_over(&m, now + TimeDelta::milliseconds(5001)));
        m.set_cooldown(Duration::ZERO);
        assert!(cooldown_over(&m, now + TimeDelta::milliseconds(1)));
    }

    #[test]
    fn the_daily_report_counts_the_alarms_since_the_evening() {
        let (chat, rx) = fake_chat();
        let m = Monitor::new("", NAME_BACKGROUND, &[], Duration::from_secs(5));
        let now = Local.with_ymd_and_hms(2026, 11, 3, 9, 0, 0).unwrap();
        m.alarm(Local.with_ymd_and_hms(2026, 11, 2, 19, 59, 0).unwrap()); // before the night
        m.alarm(Local.with_ymd_and_hms(2026, 11, 3, 3, 15, 0).unwrap());
        m.set_paused(true);
        let seen = Arc::new(Mutex::new(None));
        let s = seen.clone();
        let api = httpd::spawn(
            "127.0.0.1:0",
            0,
            Arc::new(move |r: &Request| {
                *s.lock().unwrap() = Some((r.param("latitude").map(str::to_owned), r.param("longitude").map(str::to_owned)));
                Response::new(200, "application/json", r#"{"daily":{"weather_code":[0],"temperature_2m_max":[20],"temperature_2m_min":[10]}}"#)
            }),
        )
        .unwrap();
        let calendar = httpd::spawn(
            "127.0.0.1:0",
            0,
            Arc::new(|r: &Request| {
                assert_eq!(r.param("date"), Some("2026-11-03"));
                Response::new(
                    200,
                    "application/json",
                    r#"{"events":[{"title":"Messa","eventTime":"18:30","location":"Chiesa"}]}"#,
                )
            }),
        )
        .unwrap();
        send_daily_report(
            &*chat,
            &m,
            now,
            &format!("http://{api}"),
            &format!("http://{calendar}"),
            (45.5, 9.2),
        );
        let Sent::Text(text) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("not a text")
        };
        assert!(
            text.starts_with(
                "i'm alive, tonight there has been 1 alarm, at 03:15 (alarms are paused"
            ),
            "{text}"
        );
        assert!(
            text.contains("it's gonna be a great day (10°C to 20°C)"),
            "{text}"
        );
        assert!(
            text.ends_with("Today in the calendar:\n- 18:30 Messa (Chiesa)"),
            "{text}"
        );
        assert_eq!(
            *seen.lock().unwrap(),
            Some((Some("45.5".into()), Some("9.2".into())))
        );
    }

    #[test]
    fn the_daily_report_goes_out_without_the_weather_and_the_events() {
        let (chat, rx) = fake_chat();
        let m = Monitor::new("", NAME_BACKGROUND, &[], Duration::from_secs(5));
        send_daily_report(
            &*chat,
            &m,
            Local::now(),
            "http://127.0.0.1:1",
            "http://127.0.0.1:1",
            (0.0, 0.0),
        );
        let Sent::Text(text) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("not a text")
        };
        assert_eq!(
            text,
            "i'm alive, tonight there have been no alarms or movement."
        );
    }

    // ---- the whole path from the camera to the chat, with a scripted camera ----

    use crate::alarms::tests::{FakeChat, temp_dir};
    use crate::stream::Source;
    use gif::store::Store;
    use std::collections::VecDeque;
    use std::sync::mpsc::Receiver;

    const W: usize = 160;
    const H: usize = 120;

    /// A still room with a little texture, and a bright 40x40 block at `block`.
    fn scene(block: Option<usize>) -> Frame {
        let mut pixels = Vec::with_capacity(W * H * 4);
        for y in 0..H {
            for x in 0..W {
                let mut v = 90 + ((x * 3 + y * 5) % 20) as u8;
                if let Some(bx) = block
                    && (bx..bx + 40).contains(&x)
                    && (40..80).contains(&y)
                {
                    v = 250;
                }
                pixels.extend_from_slice(&[v, v, v, 255]);
            }
        }
        Frame {
            width: W as u32,
            height: H as u32,
            pixels,
        }
    }

    /// A camera that plays `frames`, one every `delay`, and then fails.
    struct Scripted {
        frames: VecDeque<Frame>,
        delay: Duration,
    }

    impl Source for Scripted {
        fn read(&mut self, frame: &mut Frame) -> Result<(), String> {
            thread::sleep(self.delay);
            *frame = self.frames.pop_front().ok_or("the script is over")?;
            Ok(())
        }
    }

    struct Rig {
        stream: Stream,
        monitor: Arc<Monitor>,
        alarms: Arc<Alarms>,
        rx: Receiver<Sent>,
        jumping: bool,
    }

    /// The pieces `run` wires together, with `detector` in use.
    fn rig(detector: &str) -> Rig {
        let (chat, rx) = fake_chat();
        let monitor = Arc::new(Monitor::new(
            "",
            detector,
            &[
                (NAME_BACKGROUND, config::BACKGROUND_THRESHOLD),
                (NAME_DIFFERENCE, config::DIFFERENCE_THRESHOLD),
            ],
            config::ALARM_COOLDOWN,
        ));
        monitor.set_detection_frames(2);
        let stream = Stream::new(config::STREAM_HISTORY);
        let live = Arc::new(Live::new());
        live.set(config::Values {
            alarm_gif_span: Duration::from_millis(300),
            alarm_gif_fps: 10,
            ..live.get()
        });
        let store = Arc::new(Store::new(temp_dir("pipeline"), 1 << 24).unwrap());
        let alarms = Arc::new(Alarms {
            chat: chat as Arc<FakeChat>,
            book: AlarmBook::new(),
            clips: ClipRecorder::new(),
            gifs: Some(store),
        });

        {
            let monitor = monitor.clone();
            stream.add_handler(move |t| monitor.frame(t.wall));
        }
        {
            let (alarms, live) = (alarms.clone(), live.clone());
            stream.add_handler(move |t| {
                alarms.clips.frame(
                    t.frame,
                    t.mono,
                    live.get().hd_gif_width,
                    Duration::ZERO,
                    Duration::ZERO,
                )
            });
        }
        let m = monitor.clone();
        let detectors: Vec<Box<dyn Detector>> = vec![
            Box::new(BackgroundDetector::new()),
            Box::new(FrameDifference::new(move || m.detection_frames())),
        ];
        stream.add_handler(detection_handler(
            monitor.clone(),
            None,
            alarms.clone(),
            live,
            detectors,
            Printing::default(),
        ));
        Rig {
            stream,
            monitor,
            alarms,
            rx,
            jumping: detector == NAME_DIFFERENCE,
        }
    }

    impl Rig {
        /// `still` frames of the empty room, then `moving` with the block going across.
        fn play(&self, still: usize, moving: usize) {
            let mut frames: VecDeque<Frame> = (0..still).map(|_| scene(None)).collect();
            // the frame difference only sees what changes from one frame to the next: for
            // it the block jumps from one side to the other, which is plenty of movement
            let x = |i: usize| {
                if self.jumping {
                    10 + (i % 2) * 100
                } else {
                    10 + i * 2
                }
            };
            frames.extend((0..moving).map(|i| scene(Some(x(i)))));
            let mut camera = Scripted {
                frames,
                delay: Duration::from_millis(10),
            };
            let err = self
                .stream
                .run(&mut camera, &std::sync::atomic::AtomicBool::new(false))
                .unwrap_err();
            assert_eq!(err, "the script is over");
        }

        fn texts(&self) -> Vec<String> {
            let mut out = Vec::new();
            while let Ok(s) = self.rx.recv_timeout(Duration::from_millis(300)) {
                if let Sent::Text(t) = s {
                    out.push(t);
                }
            }
            out
        }
    }

    fn check_alarm_and_gif(detector: &str, still: usize, moving: usize) {
        let r = rig(detector);
        r.play(still, moving);
        let texts = r.texts();
        assert_eq!(
            texts.len(),
            1,
            "one alarm, the cooldown holds back the rest: {texts:?}"
        );
        assert!(
            texts[0].starts_with("Alarm: motion detected at ")
                && texts[0].contains(&format!("({detector} score ")),
            "{}",
            texts[0]
        );
        assert!(
            r.monitor.report().contains("alarms sent: 1"),
            "{}",
            r.monitor.report()
        );

        // the clip was recorded while the camera went on, and is saved under the code in the message
        let code = texts[0]
            .split("/hd ")
            .nth(1)
            .unwrap()
            .trim_end_matches(')')
            .to_owned();
        let store = r.alarms.gifs.as_ref().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while store.get(&code).is_err() {
            assert!(
                Instant::now() < deadline,
                "the GIF was never saved: {}",
                r.alarms.book.report(1)
            );
            thread::sleep(Duration::from_millis(10));
        }
        let data = store.get(&code).unwrap();
        assert!(data.starts_with(b"GIF89a") && data.len() > 100);

        // a reaction to the alarm message gets the GIF as a reply to it
        alarms::on_reaction(
            &r.alarms,
            &crate::telegram::Reaction {
                message_id: 101,
                emoji: "👍".into(),
                from: "bob".into(),
            },
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match r.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Sent::AnimationReply(sent, _, 101)) => {
                    assert_eq!(sent, data);
                    break;
                }
                Ok(_) | Err(_) => assert!(Instant::now() < deadline, "the GIF was never sent"),
            }
        }
    }

    #[test]
    fn a_moving_object_sets_off_the_background_detector() {
        check_alarm_and_gif(NAME_BACKGROUND, 40, 60);
    }

    #[test]
    fn a_moving_object_sets_off_the_difference_detector() {
        check_alarm_and_gif(NAME_DIFFERENCE, 5, 40);
    }

    #[test]
    fn a_still_room_sets_off_nothing() {
        for detector in [NAME_BACKGROUND, NAME_DIFFERENCE] {
            let r = rig(detector);
            r.play(60, 0);
            assert!(r.texts().is_empty(), "{detector}");
            assert!(
                r.monitor.report().contains("alarms sent: 0")
                    && r.monitor.report().contains("frames: 60")
            );
        }
    }

    #[test]
    fn paused_alarms_stay_quiet_and_resume() {
        let r = rig(NAME_DIFFERENCE);
        r.monitor.set_paused(true);
        r.play(5, 20);
        assert!(r.texts().is_empty(), "an alarm while paused");
        r.monitor.set_paused(false);
        r.play(5, 20);
        assert_eq!(r.texts().len(), 1, "no alarm after resuming");
    }

    #[test]
    fn the_detector_can_be_switched_while_running() {
        let mut r = rig(NAME_BACKGROUND);
        r.play(40, 0);
        r.monitor.set_detector(NAME_DIFFERENCE);
        r.jumping = true;
        r.play(5, 20);
        let texts = r.texts();
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(texts[0].contains("(difference score "), "{}", texts[0]);
    }
}
