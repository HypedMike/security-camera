//! A web page that plots the motion score live. The scores are pushed with
//! [`Graph::push`] and sent to the browsers as server-sent events.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::Duration;

use chrono::Local;

use crate::httpd::{self, Request, Response};
use crate::util::lock;

const PAGE: &str = include_str!("livegraph.html");
const MAX_HISTORY: usize = 300;
/// How often an idle stream writes a comment, which is how a browser that
/// went away is found out.
const KEEPALIVE: Duration = Duration::from_secs(15);

#[derive(Clone, Copy)]
struct Point {
    /// milliseconds since the Unix epoch
    t: i64,
    v: f64,
}

#[derive(Default)]
struct State {
    history: VecDeque<Point>,
    subs: Vec<SyncSender<Point>>,
}

/// Serves a web page that plots pushed values live.
#[derive(Default)]
pub struct Graph {
    state: Mutex<State>,
}

impl Graph {
    pub fn new() -> Graph {
        Graph::default()
    }

    /// Adds a value to the graph. It never blocks: a slow browser drops points.
    pub fn push(&self, v: f64) {
        self.push_at(Local::now().timestamp_millis(), v);
    }

    fn push_at(&self, t: i64, v: f64) {
        let p = Point { t, v };
        let mut s = lock(&self.state);
        s.history.push_back(p);
        if s.history.len() > MAX_HISTORY {
            s.history.pop_front();
        }
        // a browser that went away has dropped its receiver
        s.subs.retain(|tx| !matches!(tx.try_send(p), Err(TrySendError::Disconnected(_))));
    }

    fn subscribe(&self) -> (Vec<Point>, Receiver<Point>) {
        let (tx, rx) = mpsc::sync_channel(64);
        let mut s = lock(&self.state);
        s.subs.push(tx);
        (s.history.iter().copied().collect(), rx)
    }

    /// The routes of the page: `/` and `/events`.
    pub fn handle(&self, req: &Request) -> Response {
        match req.path.as_str() {
            "/" => Response::new(200, "text/html; charset=utf-8", PAGE),
            "/events" => self.events(),
            _ => Response::text(404, "not found"),
        }
    }

    /// The points as server-sent events, starting with the recent history.
    /// Also served inside the mini app.
    pub fn events(&self) -> Response {
        let (backlog, rx) = self.subscribe();
        Response::stream("text/event-stream", move |conn: &mut TcpStream| {
            let send = |conn: &mut TcpStream, p: Point| writeln!(conn, "data: [{},{}]\n", p.t, p.v);
            for p in backlog {
                send(conn, p)?;
            }
            conn.flush()?;
            loop {
                match rx.recv_timeout(KEEPALIVE) {
                    Ok(p) => send(conn, p)?,
                    Err(RecvTimeoutError::Timeout) => {
                        if httpd::peer_closed(conn) {
                            return Ok(());
                        }
                        write!(conn, ": keepalive\n\n")?;
                    }
                    Err(RecvTimeoutError::Disconnected) => return Ok(()),
                }
                conn.flush()?;
            }
        })
    }
}

/// Serves the graph page on `addr` (e.g. "0.0.0.0:8080") in a new thread.
pub fn listen(graph: std::sync::Arc<Graph>, addr: &str) -> io::Result<()> {
    httpd::spawn(addr, 0, std::sync::Arc::new(move |req: &Request| graph.handle(req)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::sync::Arc;

    fn data_line(r: &mut impl BufRead) -> String {
        loop {
            let mut line = String::new();
            assert!(r.read_line(&mut line).unwrap() > 0, "stream ended");
            if line.starts_with("data:") {
                return line.trim().to_owned();
            }
        }
    }

    #[test]
    fn events_send_the_backlog_and_then_live_points() {
        let g = Arc::new(Graph::new());
        g.push(0.25);
        let g2 = g.clone();
        let addr = httpd::spawn("127.0.0.1:0", 0, Arc::new(move |r: &Request| g2.handle(r))).unwrap();

        let resp = ureq::get(&format!("http://{addr}/events")).call().unwrap();
        assert_eq!(resp.content_type(), "text/event-stream");
        let mut r = BufReader::new(resp.into_reader());
        let l = data_line(&mut r);
        assert!(l.ends_with(",0.25]"), "backlog: {l}");
        g.push(0.5);
        let l = data_line(&mut r);
        assert!(l.ends_with(",0.5]"), "live: {l}");
    }

    #[test]
    fn serves_the_page_and_nothing_else() {
        let g = Arc::new(Graph::new());
        let addr = httpd::spawn("127.0.0.1:0", 0, Arc::new(move |r: &Request| g.handle(r))).unwrap();
        let page = ureq::get(&format!("http://{addr}/")).call().unwrap().into_string().unwrap();
        assert!(page.contains("/events"));
        assert!(matches!(ureq::get(&format!("http://{addr}/other")).call(), Err(ureq::Error::Status(404, _))));
    }

    #[test]
    fn keeps_only_the_latest_points_and_forgets_gone_browsers() {
        let g = Graph::new();
        for i in 0..MAX_HISTORY + 10 {
            g.push_at(i as i64, 0.0);
        }
        let (backlog, rx) = g.subscribe();
        assert_eq!(backlog.len(), MAX_HISTORY);
        assert_eq!(backlog[0].t, 10);
        drop(rx);
        g.push(1.0);
        assert!(lock(&g.state).subs.is_empty());
    }

    #[test]
    fn a_slow_browser_does_not_block_push() {
        let g = Graph::new();
        let (_, _rx) = g.subscribe();
        for _ in 0..1000 {
            g.push(0.1); // the channel holds 64, the rest are dropped
        }
    }
}
