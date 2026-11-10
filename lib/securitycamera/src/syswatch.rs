//! Warns on Telegram when the machine is in trouble: the disk is almost full,
//! the CPU or its temperature stay high, or the connection to Telegram was
//! lost.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};

use crate::util::{go_duration, round_secs};

/// Says when a reading is a problem.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// The disk usage (0 to 100) from which the disk is almost full.
    pub disk_percent: f64,
    /// The CPU usage (0 to 100) that is a problem when it lasts for
    /// `cpu_checks` checks in a row: short peaks are normal.
    pub cpu_percent: f64,
    pub cpu_checks: usize,
    /// The temperature in °C from which the machine is too hot.
    pub temp_c: f64,
    /// How many failed connection checks in a row mean the connection is
    /// lost: a single failure is often just a hiccup.
    pub offline_checks: usize,
}

/// How far below its limit a reading must fall before the problem is over, so
/// a value hovering around the limit does not send a message at every check.
const HYSTERESIS: f64 = 5.0;

/// What the machine looks like at one check. A value that could not be read
/// is `None`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Reading {
    pub disk: Option<f64>,
    pub cpu: Option<f64>,
    pub temp: Option<f64>,
    /// Whether Telegram could be reached.
    pub online: bool,
}

/// Turns readings into messages. It remembers which problems are ongoing, so
/// each one is reported when it starts and when it ends, not at every check.
pub struct Watcher {
    limits: Limits,
    disk: bool,
    cpu: bool,
    temp: bool,
    /// checks in a row with the CPU over its limit
    cpu_high: usize,
    /// failed connection checks in a row
    failures: usize,
    /// when the current run of failures began
    first_failure: DateTime<Local>,
    /// the connection is considered lost
    offline: bool,
}

impl Watcher {
    pub fn new(mut limits: Limits) -> Watcher {
        limits.cpu_checks = limits.cpu_checks.max(1);
        limits.offline_checks = limits.offline_checks.max(1);
        Watcher {
            limits,
            disk: false,
            cpu: false,
            temp: false,
            cpu_high: 0,
            failures: 0,
            first_failure: Local::now(),
            offline: false,
        }
    }

    /// Takes the reading made at `now` and returns the messages to send. A
    /// lost connection is only reported when it comes back, because there is
    /// no way to tell Telegram before.
    pub fn update(&mut self, now: DateTime<Local>, r: Reading) -> Vec<String> {
        let mut out = Vec::new();
        let l = self.limits;

        if let Some(disk) = r.disk {
            out.extend(step(
                &mut self.disk,
                disk,
                l.disk_percent,
                format!(
                    "Disk almost full: {disk:.0}% used (alert from {:.0}%)",
                    l.disk_percent
                ),
                format!("Disk back to normal: {disk:.0}% used"),
            ));
        }

        if let Some(cpu) = r.cpu {
            if cpu >= l.cpu_percent {
                self.cpu_high += 1;
            } else {
                self.cpu_high = 0;
            }
            if !self.cpu && self.cpu_high >= l.cpu_checks {
                self.cpu = true;
                out.push(format!(
                    "CPU busy: {cpu:.0}% for {} checks in a row (alert from {:.0}%)",
                    self.cpu_high, l.cpu_percent
                ));
            } else if self.cpu && cpu < l.cpu_percent - HYSTERESIS {
                self.cpu = false;
                out.push(format!("CPU back to normal: {cpu:.0}%"));
            }
        }

        if let Some(temp) = r.temp {
            out.extend(step(
                &mut self.temp,
                temp,
                l.temp_c,
                format!(
                    "Temperature high: {temp:.0}°C (alert from {:.0}°C)",
                    l.temp_c
                ),
                format!("Temperature back to normal: {temp:.0}°C"),
            ));
        }

        if r.online {
            if self.offline {
                let lost = (now - self.first_failure).to_std().unwrap_or_default();
                out.push(format!(
                    "Connection back after {} (lost at {})",
                    go_duration(round_secs(lost)),
                    self.first_failure.format("%H:%M")
                ));
            }
            self.failures = 0;
            self.offline = false;
        } else {
            if self.failures == 0 {
                self.first_failure = now;
            }
            self.failures += 1;
            if self.failures >= l.offline_checks {
                self.offline = true;
            }
        }
        out
    }
}

/// Updates `on`, whether a problem is ongoing, with a reading `v` against its
/// `limit` and returns the message for a problem that starts or ends, `None`
/// if nothing changed.
fn step(on: &mut bool, v: f64, limit: f64, start: String, end: String) -> Option<String> {
    if !*on && v >= limit {
        *on = true;
        Some(start)
    } else if *on && v < limit - HYSTERESIS {
        *on = false;
        Some(end)
    } else {
        None
    }
}

/// Checks the machine every `interval` until `stop` is set and sends what
/// [`Watcher::update`] returns. A message that cannot be sent (the connection
/// is down) is kept and tried again at the next check, in order.
pub fn run(
    stop: &AtomicBool,
    interval: Duration,
    limits: Limits,
    check: impl Fn() -> Reading,
    send: impl Fn(&str) -> Result<(), String>,
) {
    let mut w = Watcher::new(limits);
    let mut pending: VecDeque<String> = VecDeque::new();
    let mut next = Instant::now() + interval;
    while !stop.load(Ordering::SeqCst) {
        if Instant::now() < next {
            thread::sleep(Duration::from_millis(5).min(interval));
            continue;
        }
        next += interval;
        let r = check();
        pending.extend(w.update(Local::now(), r));
        if r.online {
            while let Some(msg) = pending.front() {
                if send(msg).is_err() {
                    break;
                }
                pending.pop_front();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex, mpsc};

    const LIMITS: Limits = Limits {
        disk_percent: 90.0,
        cpu_percent: 90.0,
        cpu_checks: 3,
        temp_c: 80.0,
        offline_checks: 2,
    };

    fn at(min: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, 3, 14, min, 0).unwrap()
    }

    fn ok() -> Reading {
        Reading {
            disk: Some(50.0),
            cpu: Some(10.0),
            temp: Some(40.0),
            online: true,
        }
    }

    fn want_one(got: Vec<String>, sub: &str) {
        assert!(
            got.len() == 1 && got[0].contains(sub),
            "got {got:?}, want one message with {sub:?}"
        );
    }

    #[test]
    fn quiet_when_all_is_fine() {
        let mut w = Watcher::new(LIMITS);
        for i in 0..5 {
            assert_eq!(w.update(at(i), ok()), Vec::<String>::new(), "check {i}");
        }
    }

    #[test]
    fn disk() {
        let mut w = Watcher::new(LIMITS);
        let mut r = ok();
        r.disk = Some(93.0);
        want_one(w.update(at(0), r), "Disk almost full: 93%");
        assert!(w.update(at(1), r).is_empty(), "repeated");
        r.disk = Some(87.0); // below the limit, but within the hysteresis
        assert!(w.update(at(2), r).is_empty(), "flapping");
        r.disk = Some(80.0);
        want_one(w.update(at(3), r), "Disk back to normal: 80%");
        assert!(w.update(at(4), r).is_empty(), "after recovery");
    }

    #[test]
    fn temperature() {
        let mut w = Watcher::new(LIMITS);
        let mut r = ok();
        r.temp = Some(85.0);
        want_one(w.update(at(0), r), "Temperature high: 85°C");
        r.temp = Some(60.0);
        want_one(w.update(at(1), r), "Temperature back to normal: 60°C");
    }

    #[test]
    fn the_cpu_needs_to_stay_busy() {
        let mut w = Watcher::new(LIMITS);
        let mut r = ok();
        r.cpu = Some(99.0);
        for i in 0..2 {
            assert!(w.update(at(i), r).is_empty(), "peak {i} reported");
        }
        r.cpu = Some(20.0); // the peak ends: the count starts again
        w.update(at(2), r);
        r.cpu = Some(99.0);
        for i in 3..5 {
            assert!(w.update(at(i), r).is_empty(), "second peak {i} reported");
        }
        want_one(w.update(at(5), r), "CPU busy: 99% for 3 checks");
        assert!(w.update(at(6), r).is_empty(), "repeated");
        r.cpu = Some(30.0);
        want_one(w.update(at(7), r), "CPU back to normal");
    }

    #[test]
    fn a_lost_connection_is_told_when_it_is_back() {
        let mut w = Watcher::new(LIMITS);
        let off = Reading {
            online: false,
            ..ok()
        };
        for i in 0..5 {
            // nothing can be said while offline
            assert!(w.update(at(i), off).is_empty(), "offline check {i}");
        }
        want_one(
            w.update(at(10), ok()),
            "Connection back after 10m0s (lost at 14:00)",
        );
        assert!(w.update(at(11), ok()).is_empty(), "after recovery");
    }

    #[test]
    fn a_single_failed_check_is_a_hiccup() {
        let mut w = Watcher::new(LIMITS); // 2 failures needed
        let off = Reading {
            online: false,
            ..ok()
        };
        w.update(at(0), off);
        assert!(w.update(at(1), ok()).is_empty(), "single failure reported");
        w.update(at(2), off); // the count must have started again
        assert!(w.update(at(3), ok()).is_empty(), "single failure reported");
    }

    #[test]
    fn values_that_could_not_be_read_are_ignored() {
        let mut w = Watcher::new(LIMITS);
        assert!(
            w.update(
                at(0),
                Reading {
                    online: true,
                    ..Reading::default()
                }
            )
            .is_empty()
        );
    }

    #[test]
    fn limits_of_zero_checks_count_as_one() {
        let mut w = Watcher::new(Limits {
            cpu_checks: 0,
            offline_checks: 0,
            ..LIMITS
        });
        let mut r = ok();
        r.cpu = Some(99.0);
        want_one(w.update(at(0), r), "CPU busy");
        w.update(
            at(1),
            Reading {
                online: false,
                ..ok()
            },
        );
        want_one(w.update(at(2), ok()), "Connection back");
    }

    #[test]
    fn run_retries_unsent_messages_in_order() {
        let stop = Arc::new(AtomicBool::new(false));
        let checks = AtomicUsize::new(0);
        let check = move || {
            let mut r = ok();
            if checks.fetch_add(1, Ordering::SeqCst) == 0 {
                r.disk = Some(95.0);
            }
            r
        };
        let (tx, rx) = mpsc::channel();
        let failures = Mutex::new(2);
        let send = move |s: &str| {
            let mut f = failures.lock().unwrap();
            if *f > 0 {
                *f -= 1;
                return Err("down".to_owned());
            }
            tx.send(s.to_owned()).unwrap();
            Ok(())
        };
        let stop2 = stop.clone();
        let t = thread::spawn(move || run(&stop2, Duration::from_millis(1), LIMITS, check, send));
        let s = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("message never sent");
        assert!(s.contains("Disk almost full"), "sent {s:?}");
        stop.store(true, Ordering::SeqCst);
        t.join().unwrap();
    }

    #[test]
    fn nothing_is_sent_while_offline() {
        let stop = Arc::new(AtomicBool::new(false));
        let sent = Arc::new(AtomicUsize::new(0));
        let n = sent.clone();
        let check = || Reading {
            disk: Some(99.0),
            online: false,
            ..Reading::default()
        };
        let stop2 = stop.clone();
        let t = thread::spawn(move || {
            run(&stop2, Duration::from_millis(1), LIMITS, check, move |_| {
                n.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        });
        thread::sleep(Duration::from_millis(50));
        stop.store(true, Ordering::SeqCst);
        t.join().unwrap();
        assert_eq!(sent.load(Ordering::SeqCst), 0);
    }
}
