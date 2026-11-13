//! A Telegram mini app: a web page that shows the live camera view and the
//! live motion graph inside Telegram, sets the brightness of the picture and
//! draws the areas where motion is ignored.
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
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::Local;
use include_dir::{Dir, include_dir};

pub use access::{Access, Members};
pub use auth::{Unauthorized, validate};

use crate::config::{BRIGHTNESS_MAX, BRIGHTNESS_MIN, BRIGHTNESS_STEP};
use crate::httpd::{self, Request, Response};
use crate::ignore::{self as ignore_areas, Area};
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

/// How the mini app reads and changes the brightness of the picture. It is
/// given to the server once the rest of the program is up ([`Server::set_brightness`]).
pub struct Brightness {
    /// The current brightness in percent.
    pub get: Box<dyn Fn() -> u32 + Send + Sync>,
    /// Changes it (and saves it) and returns the value now in use, or why it
    /// can't be changed.
    pub set: Box<dyn Fn(u32) -> Result<u32, String> + Send + Sync>,
}

/// How the mini app reads and changes the ignore areas (where motion does not
/// count), given to the server like [`Brightness`] ([`Server::set_ignore`]).
pub struct Ignore {
    pub get: Box<dyn Fn() -> Vec<Area> + Send + Sync>,
    /// Replaces the areas (and saves them) and returns the ones now in use,
    /// or why they can't be used.
    pub set: Box<SetAreas>,
}

type SetAreas = dyn Fn(Vec<Area>) -> Result<Vec<Area>, String> + Send + Sync;

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
    brightness: OnceLock<Brightness>,
    ignore: OnceLock<Ignore>,
}

impl Server {
    pub fn new(opts: Options) -> Arc<Server> {
        Arc::new(Server {
            opts,
            viewers: AtomicUsize::new(0),
            subs: Mutex::new(Vec::new()),
            brightness: OnceLock::new(),
            ignore: OnceLock::new(),
        })
    }

    /// Turns on the brightness control of the app. Without it the route is
    /// not found and the app hides the control. Only the first call counts.
    pub fn set_brightness(&self, b: Brightness) {
        let _ = self.brightness.set(b);
    }

    /// Turns on the ignore areas of the app. Without it the route is not
    /// found and the app hides the editor. Only the first call counts.
    pub fn set_ignore(&self, i: Ignore) {
        let _ = self.ignore.set(i);
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
            "/api/brightness" if self.brightness.get().is_some() => {
                self.guard(req, |me| me.brightness(req))
            }
            "/api/ignore" if self.ignore.get().is_some() => self.guard(req, |me| me.ignore(req)),
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

    /// GET shows the brightness, POST with `value` (percent) changes it.
    fn brightness(&self, req: &Request) -> Response {
        let b = self.brightness.get().expect("checked by the route");
        let now = match req.method.as_str() {
            "POST" => {
                let Some(wanted) = req.param("value").and_then(|v| v.parse::<u32>().ok()) else {
                    return Response::text(400, "value must be a whole number");
                };
                match (b.set)(wanted) {
                    Ok(v) => v,
                    Err(e) => return Response::text(400, &e),
                }
            }
            _ => (b.get)(),
        };
        Response::new(
            200,
            "application/json",
            format!(
                r#"{{"value":{now},"min":{BRIGHTNESS_MIN},"max":{BRIGHTNESS_MAX},"step":{BRIGHTNESS_STEP}}}"#
            ),
        )
    }

    /// GET shows the ignore areas, POST with `areas` (a JSON array of
    /// `{x, y, width, height}`, fractions of the picture) replaces them.
    fn ignore(&self, req: &Request) -> Response {
        let i = self.ignore.get().expect("checked by the route");
        let areas = match req.method.as_str() {
            "POST" => {
                let Some(raw) = req.param("areas") else {
                    return Response::text(400, "areas must be a JSON list");
                };
                let wanted: Vec<Area> = match serde_json::from_str(raw) {
                    Ok(a) => a,
                    Err(e) => return Response::text(400, &format!("areas: {e}")),
                };
                if let Err(e) = ignore_areas::validate(&wanted) {
                    return Response::text(400, &e);
                }
                match (i.set)(wanted) {
                    Ok(a) => a,
                    Err(e) => return Response::text(400, &e),
                }
            }
            _ => (i.get)(),
        };
        Response::new(
            200,
            "application/json",
            serde_json::json!({ "areas": areas, "max": ignore_areas::MAX_AREAS }).to_string(),
        )
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
    fn the_brightness_is_read_and_changed_with_auth() {
        use std::sync::atomic::AtomicU32;
        let (s, base) = serve(options(1));
        // not turned on: not found
        assert_eq!(status(&format!("{base}/api/brightness{}", auth(42))), 404);

        let now = Arc::new(AtomicU32::new(100));
        let (get, set) = (now.clone(), now.clone());
        s.set_brightness(Brightness {
            get: Box::new(move || get.load(Ordering::SeqCst)),
            set: Box::new(move |v| {
                if v > 300 {
                    return Err("brightness: too much".into());
                }
                set.store(v, Ordering::SeqCst);
                Ok(v)
            }),
        });
        let url = format!("{base}/api/brightness{}", auth(42));
        assert_eq!(status(&format!("{base}/api/brightness")), 401);
        assert_eq!(status(&format!("{base}/api/brightness{}", auth(7))), 401);

        let got = ureq::get(&url).call().unwrap().into_string().unwrap();
        assert_eq!(got, r#"{"value":100,"min":20,"max":400,"step":10}"#);

        let got = ureq::post(&format!("{url}&value=130"))
            .call()
            .unwrap()
            .into_string()
            .unwrap();
        assert!(got.starts_with(r#"{"value":130,"#), "{got}");
        assert_eq!(now.load(Ordering::SeqCst), 130);

        // a GET never changes it, a bad value is refused
        ureq::get(&format!("{url}&value=50")).call().unwrap();
        assert_eq!(now.load(Ordering::SeqCst), 130);
        assert_eq!(
            status_of(ureq::post(&format!("{url}&value=301")).call()),
            400
        );
        assert_eq!(status_of(ureq::post(&format!("{url}&value=x")).call()), 400);
        assert_eq!(status_of(ureq::post(&url).call()), 400);
        assert_eq!(now.load(Ordering::SeqCst), 130);
    }

    #[test]
    fn the_ignore_areas_are_read_and_changed_with_auth() {
        use std::sync::Mutex as M;
        let (s, base) = serve(options(1));
        assert_eq!(status(&format!("{base}/api/ignore{}", auth(42))), 404);

        let now = Arc::new(M::new(Vec::<Area>::new()));
        let (get, set) = (now.clone(), now.clone());
        s.set_ignore(Ignore {
            get: Box::new(move || lock(&get).clone()),
            set: Box::new(move |a| {
                *lock(&set) = a.clone();
                Ok(a)
            }),
        });
        let url = format!("{base}/api/ignore{}", auth(42));
        assert_eq!(status(&format!("{base}/api/ignore")), 401);
        assert_eq!(status(&format!("{base}/api/ignore{}", auth(7))), 401);

        let got = ureq::get(&url).call().unwrap().into_string().unwrap();
        assert_eq!(got, r#"{"areas":[],"max":20}"#);

        let areas = r#"[{"x":0.1,"y":0.2,"width":0.3,"height":0.4}]"#;
        let got = ureq::post(&url)
            .query("areas", areas)
            .call()
            .unwrap()
            .into_string()
            .unwrap();
        assert!(
            got.starts_with(r#"{"areas":[{"x":0.1,"y":0.2,"width":0.3,"height":0.4}]"#),
            "{got}"
        );
        assert_eq!(lock(&now).len(), 1);

        // a GET never changes it; bad input is refused and changes nothing
        ureq::get(&url).query("areas", "[]").call().unwrap();
        assert_eq!(lock(&now).len(), 1);
        for bad in [
            "x",
            r#"[{"x":0.9,"y":0,"width":0.5,"height":0.5}]"#,
            r#"[{"x":0,"y":0,"width":0,"height":0.5}]"#,
        ] {
            assert_eq!(
                status_of(ureq::post(&url).query("areas", bad).call()),
                400,
                "{bad}"
            );
        }
        assert_eq!(status_of(ureq::post(&url).call()), 400);
        assert_eq!(lock(&now).len(), 1);

        // an empty list clears them
        ureq::post(&url).query("areas", "[]").call().unwrap();
        assert!(lock(&now).is_empty());
    }

    fn status_of(r: Result<ureq::Response, ureq::Error>) -> u16 {
        match r {
            Ok(r) => r.status(),
            Err(ureq::Error::Status(code, _)) => code,
            Err(e) => panic!("{e}"),
        }
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
