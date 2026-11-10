//! A Telegram mini app: a web page that shows the live camera view and the
//! live motion graph inside Telegram.
//!
//! The server only listens on the local machine; a tunnel (Tailscale Funnel)
//! makes it reachable over HTTPS. Every request except the page itself has to
//! carry the initData Telegram gives the app when it opens, which proves the
//! request comes from a Telegram user, and only the users `Options::allowed`
//! accepts are let in.

mod access;
mod auth;

use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use chrono::Local;
use include_dir::{Dir, include_dir};

pub use access::{Access, Members};
pub use auth::{Unauthorized, validate};

use crate::httpd::{self, Request, Response};
use crate::livegraph::Graph;
use crate::util::lock;

/// The React app (miniapp-web), built with `pnpm run build` and committed so
/// the machine that runs the camera does not need Node.
static DIST: Dir = include_dir!("$CARGO_MANIFEST_DIR/dist");

/// Checks the public address of the mini app (MINI_APP_URL). Telegram only
/// opens https addresses. "" means the mini app is off.
pub fn parse_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(String::new());
    }
    match url::Url::parse(raw) {
        Ok(u) if u.scheme() == "https" && u.host_str().is_some_and(|h| !h.is_empty()) => {
            Ok(raw.to_owned())
        }
        _ => Err(format!(
            "{raw:?} is not an https address (Telegram only opens https mini apps)"
        )),
    }
}

/// How long a viewer may take to read what is sent to it.
const STREAM_WRITE_TIMEOUT: Duration = httpd::WRITE_TIMEOUT;

pub struct Options {
    /// The bot token the initData signature is checked with.
    pub token: String,
    /// Whether a Telegram user may open the app.
    pub allowed: Box<dyn Fn(i64) -> bool + Send + Sync>,
    /// How old the initData may be.
    pub max_age: Duration,
    /// How many video streams may be open at once.
    pub max_viewers: usize,
    /// The longest a single video stream is kept open, so a forgotten app
    /// does not stream forever.
    pub max_stream: Duration,
    /// The motion graph, served as server-sent events.
    pub events: Option<Arc<Graph>>,
}

/// The slot a video stream takes its next frame from: only the latest is kept.
#[derive(Default)]
struct Slot {
    frame: Mutex<Option<Vec<u8>>>,
    ready: Condvar,
}

/// The HTTP side of the mini app.
pub struct Server {
    opts: Options,
    viewers: AtomicUsize,
    subs: Mutex<Vec<Arc<Slot>>>,
}

impl Server {
    pub fn new(opts: Options) -> Arc<Server> {
        Arc::new(Server {
            opts,
            viewers: AtomicUsize::new(0),
            subs: Mutex::new(Vec::new()),
        })
    }

    /// Whether anybody is watching, so the camera loop can skip encoding
    /// frames nobody will see.
    pub fn wanted(&self) -> bool {
        self.viewers.load(Ordering::SeqCst) > 0
    }

    /// Hands a JPEG to every open video stream. It never blocks: a slow
    /// viewer just skips frames.
    pub fn push_frame(&self, jpeg: &[u8]) {
        for slot in lock(&self.subs).iter() {
            // replaces the frame the viewer has not taken yet
            *lock(&slot.frame) = Some(jpeg.to_vec());
            slot.ready.notify_one();
        }
    }

    /// Serves on `addr` (e.g. "127.0.0.1:8081") in a new thread.
    pub fn listen(self: &Arc<Self>, addr: &str) -> io::Result<std::net::SocketAddr> {
        let me = self.clone();
        httpd::spawn(addr, 0, Arc::new(move |req: &Request| me.handle(req)))
    }

    /// The routes of the mini app.
    pub fn handle(self: &Arc<Self>, req: &Request) -> Response {
        match req.path.as_str() {
            "/api/check" => self.guard(req, |_| Response::no_content()),
            "/stream" => self.guard(req, |me| me.stream()),
            "/events" if self.opts.events.is_some() => {
                self.guard(req, |me| me.opts.events.as_ref().expect("checked").events())
            }
            _ => self.file(&req.path),
        }
    }

    /// The page and its scripts are not secret: the data is what is guarded.
    fn file(&self, path: &str) -> Response {
        let name = path.trim_start_matches('/');
        let name = if name.is_empty() { "index.html" } else { name };
        match DIST.get_file(name).filter(|_| !name.contains("..")) {
            Some(f) => Response::new(200, content_type(name), f.contents())
                .header("Cache-Control", "no-cache"),
            None => Response::text(404, "not found"),
        }
    }

    /// Lets only the allowed Telegram users through.
    fn guard(
        self: &Arc<Self>,
        req: &Request,
        next: impl FnOnce(&Arc<Self>) -> Response,
    ) -> Response {
        let auth = req.param("auth").unwrap_or("");
        match validate(auth, &self.opts.token, Local::now(), self.opts.max_age) {
            Ok(id) if (self.opts.allowed)(id) => next(self).header("Cache-Control", "no-store"),
            _ => Response::text(401, "unauthorized"),
        }
    }

    /// The camera as MJPEG: a never-ending multipart response whose parts are
    /// JPEGs, which a plain `<img>` shows as video.
    fn stream(self: &Arc<Self>) -> Response {
        if self.viewers.fetch_add(1, Ordering::SeqCst) + 1 > self.opts.max_viewers {
            self.viewers.fetch_sub(1, Ordering::SeqCst);
            return Response::text(503, "too many viewers");
        }
        let slot = Arc::new(Slot::default());
        lock(&self.subs).push(slot.clone());
        let (me, max_stream) = (self.clone(), self.opts.max_stream);
        Response::stream("multipart/x-mixed-replace; boundary=frame", move |conn| {
            // however it ends, the viewer stops counting
            let _leave = Leave {
                server: &me,
                slot: &slot,
            };
            conn.set_write_timeout(Some(STREAM_WRITE_TIMEOUT))?;
            send_frames(conn, &slot, Instant::now() + max_stream)
        })
    }
}

/// Takes a viewer out of the server when its stream ends, however it ends.
struct Leave<'a> {
    server: &'a Server,
    slot: &'a Arc<Slot>,
}

impl Drop for Leave<'_> {
    fn drop(&mut self) {
        lock(&self.server.subs).retain(|s| !Arc::ptr_eq(s, self.slot));
        self.server.viewers.fetch_sub(1, Ordering::SeqCst);
    }
}

fn send_frames(conn: &mut TcpStream, slot: &Slot, end: Instant) -> io::Result<()> {
    loop {
        let Some(left) = end.checked_duration_since(Instant::now()) else {
            return Ok(());
        };
        let frame = {
            let mut f = lock(&slot.frame);
            if f.is_none() {
                // wakes up now and then to see whether the viewer left
                f = slot
                    .ready
                    .wait_timeout(f, left.min(Duration::from_secs(1)))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            f.take()
        };
        match frame {
            Some(jpeg) => {
                // one write for the whole part, so it goes out in one piece
                let mut part = format!(
                    "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                    jpeg.len()
                )
                .into_bytes();
                part.extend_from_slice(&jpeg);
                part.extend_from_slice(b"\r\n");
                conn.write_all(&part)?;
                conn.flush()?;
            }
            None if httpd::peer_closed(conn) => return Ok(()),
            None => {}
        }
    }
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::auth::tests::{TOKEN, signed};
    use super::*;
    use chrono::TimeDelta;
    use std::io::Read;

    fn options(max_viewers: usize) -> Options {
        Options {
            token: TOKEN.into(),
            allowed: Box::new(|id| id == 42),
            max_age: Duration::from_secs(3600),
            max_viewers,
            max_stream: Duration::from_secs(60),
            events: None,
        }
    }

    fn serve(opts: Options) -> (Arc<Server>, String) {
        let s = Server::new(opts);
        let addr = s.listen("127.0.0.1:0").unwrap();
        (s, format!("http://{addr}"))
    }

    fn auth(user: i64) -> String {
        let q = signed(TOKEN, user, TimeDelta::seconds(1), Local::now(), &[]);
        format!(
            "?auth={}",
            url::form_urlencoded::byte_serialize(q.as_bytes()).collect::<String>()
        )
    }

    fn status(url: &str) -> u16 {
        match ureq::get(url).call() {
            Ok(r) => r.status(),
            Err(ureq::Error::Status(code, _)) => code,
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn the_page_is_open_and_the_data_needs_auth() {
        let graph = Arc::new(Graph::new());
        graph.push(0.5);
        let (_, base) = serve(Options {
            events: Some(graph),
            ..options(1)
        });
        let page = ureq::get(&format!("{base}/")).call().unwrap();
        assert_eq!(page.status(), 200);
        assert!(page.content_type().starts_with("text/html"));
        assert_eq!(page.header("Cache-Control"), Some("no-cache"));
        assert_eq!(status(&format!("{base}/nothing-here")), 404);

        for path in ["/api/check", "/stream", "/events"] {
            assert_eq!(status(&format!("{base}{path}")), 401, "{path} without auth");
            assert_eq!(
                status(&format!("{base}{path}{}", auth(7))),
                401,
                "{path} for a user not allowed"
            );
        }
        assert_eq!(status(&format!("{base}/api/check{}", auth(42))), 204);
        let events = ureq::get(&format!("{base}/events{}", auth(42)))
            .call()
            .unwrap();
        assert_eq!(events.status(), 200);
        assert_eq!(events.header("Cache-Control"), Some("no-store"));
        let mut text = [0u8; 64];
        let n = events.into_reader().read(&mut text).unwrap();
        assert!(
            String::from_utf8_lossy(&text[..n]).starts_with("data: ["),
            "the graph's history comes first"
        );
    }

    #[test]
    fn without_a_graph_there_are_no_events() {
        let (_, base) = serve(options(1));
        assert_eq!(status(&format!("{base}/events{}", auth(42))), 404);
    }

    #[test]
    fn the_embedded_page_loads_its_assets() {
        let (_, base) = serve(options(1));
        let page = ureq::get(&format!("{base}/"))
            .call()
            .unwrap()
            .into_string()
            .unwrap();
        let asset = page
            .split('"')
            .find(|s| s.starts_with("/assets/") && s.ends_with(".js"))
            .unwrap_or_else(|| panic!("the page names no script: {page}"));
        let js = ureq::get(&format!("{base}{asset}")).call().unwrap();
        assert!(js.content_type().starts_with("text/javascript"));
    }

    #[test]
    fn streams_frames_to_viewers_up_to_the_limit() {
        let (s, base) = serve(options(1));
        assert!(!s.wanted(), "wanted with no viewer");
        let resp = ureq::get(&format!("{base}/stream{}", auth(42)))
            .call()
            .unwrap();
        assert!(
            resp.content_type().starts_with("multipart/x-mixed-replace"),
            "{}",
            resp.content_type()
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !s.wanted() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(s.wanted(), "not wanted with a viewer");

        // the second viewer is over the limit
        assert_eq!(status(&format!("{base}/stream{}", auth(42))), 503);

        s.push_frame(b"JPEGDATA");
        let mut reader = resp.into_reader();
        let mut got = Vec::new();
        let mut buf = [0u8; 256];
        while !got.ends_with(b"JPEGDATA\r\n") {
            let n = reader.read(&mut buf).unwrap();
            assert!(
                n > 0,
                "the stream ended: {:?}",
                String::from_utf8_lossy(&got)
            );
            got.extend_from_slice(&buf[..n]);
        }
        let got = String::from_utf8_lossy(&got);
        assert!(
            got.starts_with(
                "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: 8\r\n\r\nJPEGDATA"
            ),
            "{got:?}"
        );

        // when the viewer goes away the stream is over, and the next can come
        drop(reader);
        let deadline = Instant::now() + Duration::from_secs(10);
        while s.wanted() && Instant::now() < deadline {
            s.push_frame(b"x");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!s.wanted(), "the viewer is still counted after it left");
        assert!(lock(&s.subs).is_empty());
    }

    #[test]
    fn a_stream_ends_after_the_longest_time() {
        let (s, base) = serve(Options {
            max_stream: Duration::from_millis(100),
            ..options(1)
        });
        let mut body = String::new();
        ureq::get(&format!("{base}/stream{}", auth(42)))
            .call()
            .unwrap()
            .into_reader()
            .read_to_string(&mut body)
            .unwrap();
        assert!(body.is_empty());
        let deadline = Instant::now() + Duration::from_secs(5);
        while s.wanted() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!s.wanted());
    }

    #[test]
    fn push_frame_keeps_only_the_latest() {
        let s = Server::new(options(1));
        let slot = Arc::new(Slot::default());
        lock(&s.subs).push(slot.clone());
        s.push_frame(b"a");
        s.push_frame(b"b");
        assert_eq!(lock(&slot.frame).take().as_deref(), Some(&b"b"[..]));
    }

    #[test]
    fn parse_url_wants_https() {
        for (input, want) in [
            ("", ""),
            ("  ", ""),
            ("https://cam.tail1234.ts.net", "https://cam.tail1234.ts.net"),
        ] {
            assert_eq!(parse_url(input).as_deref(), Ok(want), "{input:?}");
        }
        for input in ["http://cam.ts.net", "cam.ts.net", "https://", "ftp://x"] {
            assert!(parse_url(input).is_err(), "{input:?} accepted");
        }
    }
}
