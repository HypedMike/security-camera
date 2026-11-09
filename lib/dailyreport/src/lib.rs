//! A short "I'm alive" message to Telegram every day, with the alarms of the
//! night, a weather based suggestion and the events of the day from the
//! parish calendar.

use std::fmt::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Datelike, Days, Local, NaiveDate, TimeZone, Weekday};
use serde::Deserialize;

/// The forecast of one day.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weather {
    pub sunny: bool,
    pub max_c: f64,
    pub min_c: f64,
}

/// An event of the day in the calendar.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub title: String,
    /// Free text, e.g. "18:30" or "10 · 12 · 18"; empty if unknown.
    pub time: String,
    /// Empty if unknown.
    pub location: String,
}

/// Builds the message sent at `now`. `alarms` are the times of the alarms
/// since the last report, `paused` says whether alarms are switched off,
/// `w` is today's forecast, `None` if it is not known, and `events` are the
/// events of the day in the calendar.
pub fn compose(now: DateTime<Local>, alarms: &[DateTime<Local>], paused: bool, w: Option<Weather>, events: &[Event]) -> String {
    let mut b = String::from("i'm alive, tonight ");
    match alarms {
        [] => b.push_str("there have been no alarms or movement"),
        [only] => {
            let _ = write!(b, "there has been 1 alarm, at {}", only.format("%H:%M"));
        }
        [.., last] => {
            let _ = write!(b, "there have been {} alarms, the last at {}", alarms.len(), last.format("%H:%M"));
        }
    }
    if paused {
        b.push_str(" (alarms are paused, so some movement may have gone unnoticed)");
    }
    b.push('.');

    if let Some(w) = w {
        push_weather(&mut b, now, w);
    }
    if !events.is_empty() {
        b.push_str("\n\nToday in the calendar:");
        for e in events {
            b.push_str("\n- ");
            if !e.time.is_empty() {
                let _ = write!(b, "{} ", e.time);
            }
            b.push_str(&e.title);
            if !e.location.is_empty() {
                let _ = write!(b, " ({})", e.location);
            }
        }
    }
    b
}

fn push_weather(b: &mut String, now: DateTime<Local>, w: Weather) {
    b.push_str(" Today ");
    let temp = format!("{:.0}°C to {:.0}°C", w.min_c, w.max_c);
    let _ = match (w.sunny, w.max_c) {
        (true, max) if max >= 30.0 => write!(b, "it's gonna be a great day, but hot ({temp}): bring water and sunscreen."),
        (true, max) if max < 10.0 => write!(b, "it's gonna be a great day, but cold ({temp}): dress warm."),
        (true, _) => write!(b, "it's gonna be a great day ({temp})."),
        (false, max) if max < 10.0 => write!(b, "you should find something to do at home, it's cold and not sunny ({temp})."),
        (false, max) if max >= 30.0 => write!(b, "you should find something to do at home, somewhere cool: it's hot and not sunny ({temp})."),
        (false, _) => write!(b, "you should find something to do at home, it's not going to be sunny ({temp})."),
    };
    if w.sunny && now.weekday() == Weekday::Sun {
        b.push_str(" sunny sunday would make a great hike!");
    }
}

/// Whether a WMO weather code, as Open-Meteo gives it, is a clear sky (0) or
/// a mostly clear one (1).
fn sunny(code: i64) -> bool {
    code == 0 || code == 1
}

/// The API the forecast comes from; it needs no key.
const OPEN_METEO: &str = "https://api.open-meteo.com/v1/forecast";

/// Today's forecast for the coordinates. `api` is the URL of the Open-Meteo
/// forecast endpoint, "" for the real one.
pub fn fetch_weather(api: &str, lat: f64, lon: f64, timeout: Duration) -> Result<Weather, String> {
    let api = if api.is_empty() { OPEN_METEO } else { api };
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let resp = match agent
        .get(api)
        .query("latitude", &lat.to_string())
        .query("longitude", &lon.to_string())
        .query("daily", "weather_code,temperature_2m_max,temperature_2m_min")
        .query("forecast_days", "1")
        .query("timezone", "auto")
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let body: String = r.into_string().unwrap_or_default().chars().take(256).collect();
            return Err(format!("weather: status {code}: {body}"));
        }
        Err(e) => return Err(format!("weather: {e}")),
    };
    #[derive(Deserialize)]
    struct Data {
        #[serde(default)]
        daily: Daily,
    }
    #[derive(Deserialize, Default)]
    struct Daily {
        #[serde(default)]
        weather_code: Vec<i64>,
        #[serde(default)]
        temperature_2m_max: Vec<f64>,
        #[serde(default)]
        temperature_2m_min: Vec<f64>,
    }
    let data: Data = resp.into_json().map_err(|e| format!("weather: {e}"))?;
    let d = data.daily;
    match (d.weather_code.first(), d.temperature_2m_max.first(), d.temperature_2m_min.first()) {
        (Some(&code), Some(&max_c), Some(&min_c)) => Ok(Weather { sunny: sunny(code), max_c, min_c }),
        _ => Err("weather: no forecast for today".into()),
    }
}

/// The API the events come from: the parish website, which needs no key.
pub const CALENDAR_API: &str = "https://parrocchiasandonato.vercel.app/api/events";

/// The events in the calendar on `day`. `api` is the URL of the events
/// endpoint, "" for the parish website.
pub fn fetch_events(api: &str, day: NaiveDate, timeout: Duration) -> Result<Vec<Event>, String> {
    let api = if api.is_empty() { CALENDAR_API } else { api };
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let resp = match agent.get(api).query("date", &day.format("%Y-%m-%d").to_string()).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let body: String = r.into_string().unwrap_or_default().chars().take(256).collect();
            return Err(format!("calendar: status {code}: {body}"));
        }
        Err(e) => return Err(format!("calendar: {e}")),
    };
    #[derive(Deserialize)]
    struct Data {
        events: Vec<Item>,
    }
    #[derive(Deserialize)]
    struct Item {
        title: String,
        #[serde(rename = "eventTime")]
        time: Option<String>,
        location: Option<String>,
    }
    let data: Data = resp.into_json().map_err(|e| format!("calendar: {e}"))?;
    Ok(data
        .events
        .into_iter()
        .map(|i| Event { title: i.title, time: i.time.unwrap_or_default(), location: i.location.unwrap_or_default() })
        .collect())
}

/// The local time `hour`:00 on `day`. A clock that skips that hour (the
/// change to summer time) gives the first moment after it.
pub fn at_hour(day: NaiveDate, hour: u32) -> DateTime<Local> {
    for h in [hour, hour + 1, hour + 2] {
        if let Some(t) = day.and_hms_opt(h % 24, 0, 0).and_then(|t| Local.from_local_datetime(&t).earliest()) {
            return t;
        }
    }
    Local.from_utc_datetime(&day.and_hms_opt(hour % 24, 0, 0).expect("an hour of the day"))
}

/// The next time after `now` that the clock reads `hour`:00.
pub fn next_run(now: DateTime<Local>, hour: u32) -> DateTime<Local> {
    let today = now.date_naive();
    let next = at_hour(today, hour);
    if next > now { next } else { at_hour(today.checked_add_days(Days::new(1)).unwrap_or(today), hour) }
}

/// Calls `send` every day at `hour`:00 local time until `stop` is set.
pub fn run(stop: &AtomicBool, hour: u32, send: impl Fn(DateTime<Local>)) {
    while !stop.load(Ordering::SeqCst) {
        let next = next_run(Local::now(), hour);
        while Local::now() < next {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            // short sleeps: the clock may change while waiting (summer time, a sync)
            thread::sleep(Duration::from_millis(500).min((next - Local::now()).to_std().unwrap_or_default()));
        }
        send(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    fn w(sunny: bool, max_c: f64, min_c: f64) -> Option<Weather> {
        Some(Weather { sunny, max_c, min_c })
    }

    #[test]
    fn composes_the_message() {
        let sunday = local(2026, 11, 1, 20, 0);
        let monday = local(2026, 11, 2, 20, 0);
        let at = local(2026, 11, 1, 3, 15);
        let hour_later = at + chrono::TimeDelta::hours(1);
        struct Case {
            name: &'static str,
            now: DateTime<Local>,
            alarms: Vec<DateTime<Local>>,
            paused: bool,
            weather: Option<Weather>,
            events: Vec<Event>,
            want: Vec<&'static str>,
            not_want: Vec<&'static str>,
        }
        let case = |name, now, alarms, paused, weather, want, not_want| Case { name, now, alarms, paused, weather, events: vec![], want, not_want };
        let cases = vec![
            case("quiet sunny sunday", sunday, vec![], false, w(true, 20.0, 10.0),
                vec!["i'm alive, tonight there have been no alarms or movement.", "it's gonna be a great day", "sunny sunday would make a great hike!"], vec!["at home"]),
            case("sunny monday", monday, vec![], false, w(true, 20.0, 10.0), vec!["great day"], vec!["hike"]),
            case("rainy sunday", sunday, vec![], false, w(false, 15.0, 10.0), vec!["find something to do at home"], vec!["hike", "great day"]),
            case("one alarm", monday, vec![at], false, None, vec!["1 alarm, at 03:15"], vec!["Today"]),
            case("alarms", monday, vec![at, hour_later], false, None, vec!["2 alarms, the last at 04:15"], vec![]),
            case("paused", monday, vec![], true, None, vec!["alarms are paused"], vec![]),
            case("hot", monday, vec![], false, w(true, 33.0, 22.0), vec!["hot", "sunscreen"], vec![]),
            case("hot and not sunny", monday, vec![], false, w(false, 33.0, 22.0), vec!["somewhere cool"], vec![]),
            case("cold sunny", monday, vec![], false, w(true, 5.0, -2.0), vec!["dress warm", "-2°C to 5°C"], vec![]),
            case("cold rainy", monday, vec![], false, w(false, 5.0, 1.0), vec!["at home", "cold"], vec![]),
        ];
        let mut cases = cases;
        let ev = |title: &str, time: &str, location: &str| Event { title: title.into(), time: time.into(), location: location.into() };
        cases.push(Case {
            events: vec![ev("Messa", "18:30", "Chiesa"), ev("Incontro giovani", "", ""), ev("Rosario", "21", "")],
            ..case("events", monday, vec![], false, w(true, 20.0, 10.0), vec!["great day (10°C to 20°C).\n\nToday in the calendar:\n- 18:30 Messa (Chiesa)\n- Incontro giovani\n- 21 Rosario"], vec![])
        });
        cases.push(Case { events: vec![ev("Messa", "", "")], ..case("events without weather", monday, vec![], false, None, vec!["movement.\n\nToday in the calendar:\n- Messa"], vec!["Today it"]) });
        for c in cases {
            let got = compose(c.now, &c.alarms, c.paused, c.weather, &c.events);
            for want in c.want {
                assert!(got.contains(want), "{}: message misses {want:?}:\n{got}", c.name);
            }
            for not in c.not_want {
                assert!(!got.contains(not), "{}: message has {not:?}:\n{got}", c.name);
            }
        }
    }

    /// The query strings of the requests a fake server got.
    type Seen = Arc<Mutex<Vec<String>>>;

    /// Serves `body` with `status` to every request, on a local port.
    fn server(status: u16, body: &'static str) -> (String, Seen) {
        let seen: Seen = Default::default();
        let log = seen.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { continue };
                let mut line = String::new();
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                reader.read_line(&mut line).unwrap();
                // the rest of the headers
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) <= 2 {
                        break;
                    }
                }
                let target = line.split_whitespace().nth(1).unwrap_or("");
                log.lock().unwrap().push(target.split_once('?').map_or("", |(_, q)| q).to_owned());
                let _ = write!(conn, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (url, seen)
    }

    fn weather_server(status: u16, body: &'static str) -> (String, Seen) {
        server(status, body)
    }

    const TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn fetches_the_forecast() {
        let (url, seen) = weather_server(200, r#"{"daily":{"weather_code":[1],"temperature_2m_max":[21.4],"temperature_2m_min":[9.8]}}"#);
        let got = fetch_weather(&url, 45.5, 9.2, TIMEOUT).unwrap();
        assert_eq!(got, Weather { sunny: true, max_c: 21.4, min_c: 9.8 });
        let seen = seen.lock().unwrap();
        assert!(seen[0].contains("latitude=45.5") && seen[0].contains("longitude=9.2"), "{seen:?}");

        let (url, _) = weather_server(200, r#"{"daily":{"weather_code":[61],"temperature_2m_max":[12],"temperature_2m_min":[8]}}"#);
        assert!(!fetch_weather(&url, 0.0, 0.0, TIMEOUT).unwrap().sunny, "rain");
    }

    #[test]
    fn a_bad_forecast_is_an_error() {
        let (url, _) = weather_server(200, "{}");
        assert!(fetch_weather(&url, 0.0, 0.0, TIMEOUT).unwrap_err().contains("no forecast"));
        let (url, _) = weather_server(200, "not json");
        assert!(fetch_weather(&url, 0.0, 0.0, TIMEOUT).is_err());
        let (url, _) = weather_server(500, "no");
        assert!(fetch_weather(&url, 0.0, 0.0, TIMEOUT).unwrap_err().contains("status 500"));
        assert!(fetch_weather("http://127.0.0.1:1", 0.0, 0.0, TIMEOUT).is_err());
    }

    #[test]
    fn fetches_the_events() {
        let (url, seen) = server(
            200,
            r#"{"date":"2026-11-09","events":[{"id":1,"title":"Messa","description":null,"eventTime":"18:30","location":"Chiesa","kind":"messa","dayTitle":null},{"id":2,"title":"Incontro","description":"x","eventTime":null,"location":null,"kind":null,"dayTitle":null}]}"#,
        );
        let day = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        let got = fetch_events(&url, day, TIMEOUT).unwrap();
        let ev = |title: &str, time: &str, location: &str| Event { title: title.into(), time: time.into(), location: location.into() };
        assert_eq!(got, [ev("Messa", "18:30", "Chiesa"), ev("Incontro", "", "")]);
        assert_eq!(*seen.lock().unwrap(), ["date=2026-11-09"]);
    }

    #[test]
    fn bad_events_are_an_error() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        for (status, body) in [(200, "{}"), (200, "not json"), (500, "no")] {
            let (url, _) = server(status, body);
            assert!(fetch_events(&url, day, TIMEOUT).is_err(), "{status} {body}");
        }
        assert!(fetch_events("http://127.0.0.1:1", day, TIMEOUT).is_err());
    }

    #[test]
    fn next_run_is_the_next_time_the_clock_reads_the_hour() {
        let d = |day, h, m| local(2026, 10, day, h, m);
        assert_eq!(next_run(d(30, 13, 0), 20), d(30, 20, 0), "before");
        assert_eq!(next_run(d(30, 20, 0), 20), d(31, 20, 0), "at the hour");
        assert_eq!(next_run(d(30, 21, 5), 20), d(31, 20, 0), "after");
        assert_eq!(next_run(d(31, 23, 59), 9), local(2026, 11, 1, 9, 0), "next month");
    }

    #[test]
    fn run_stops_when_asked() {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let t = thread::spawn(move || run(&stop2, 9, |_| panic!("not at 9 o'clock")));
        thread::sleep(Duration::from_millis(50));
        stop.store(true, Ordering::SeqCst);
        t.join().unwrap();
    }
}
