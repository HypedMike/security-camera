//! A small HTTP/1.1 server, for the mini app and the live graph.
//!
//! It does what those need and nothing more: one request per connection
//! (`Connection: close`), a body only when the handler asks for one, and
//! responses that stay open and are written as they go (the MJPEG video and
//! the server-sent events). Owning the sockets makes it easy to flush after
//! every frame, to give up on a viewer that stops reading, and to notice one
//! that went away.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

/// How long a client has to send its request.
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one write may block: a viewer that stops reading is dropped.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HEADER_BYTES: usize = 16 << 10;
const MAX_HEADERS: usize = 64;
/// The most connections served at once; the others get a 503.
const MAX_CONNECTIONS: usize = 64;

pub struct Request {
    pub method: String,
    /// The path, without the query.
    pub path: String,
    /// The query parameters, decoded, in order.
    pub query: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// The first value of the query parameter `name`.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

type StreamFn = Box<dyn FnOnce(&mut TcpStream) -> io::Result<()> + Send>;

pub enum Body {
    Bytes(Vec<u8>),
    /// Headers go out at once, then the function writes the body until it is
    /// over or fails. Not run for HEAD.
    Stream(StreamFn),
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Body,
}

impl Response {
    pub fn new(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Response {
        Response {
            status,
            headers: vec![("Content-Type".into(), content_type.into())],
            body: Body::Bytes(body.into()),
        }
    }

    pub fn text(status: u16, text: &str) -> Response {
        Response::new(status, "text/plain; charset=utf-8", format!("{text}\n"))
    }

    pub fn no_content() -> Response {
        Response {
            status: 204,
            headers: Vec::new(),
            body: Body::Bytes(Vec::new()),
        }
    }

    pub fn stream(
        content_type: &str,
        f: impl FnOnce(&mut TcpStream) -> io::Result<()> + Send + 'static,
    ) -> Response {
        Response {
            status: 200,
            headers: vec![("Content-Type".into(), content_type.into())],
            body: Body::Stream(Box::new(f)),
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Response {
        self.headers.push((name.into(), value.into()));
        self
    }
}

pub type Handler = Arc<dyn Fn(&Request) -> Response + Send + Sync>;

/// Serves `listener` until the process ends, a thread per connection.
/// `max_body` is the largest request body accepted.
pub fn serve(listener: TcpListener, max_body: usize, handler: Handler) {
    let active = Arc::new(AtomicUsize::new(0));
    for conn in listener.incoming() {
        let Ok(mut conn) = conn else { continue };
        if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            active.fetch_sub(1, Ordering::SeqCst);
            let _ = write_response(
                &mut conn,
                Response::text(503, "too many connections"),
                false,
            );
            continue;
        }
        let (active, handler) = (active.clone(), handler.clone());
        thread::spawn(move || {
            let _ = handle(conn, max_body, &handler);
            active.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// Binds `addr` and serves it in a new thread. Returns the address it listens
/// on (useful with port 0).
pub fn spawn(
    addr: impl ToSocketAddrs,
    max_body: usize,
    handler: Handler,
) -> io::Result<std::net::SocketAddr> {
    let listener = TcpListener::bind(addr)?;
    let local = listener.local_addr()?;
    thread::spawn(move || serve(listener, max_body, handler));
    Ok(local)
}

fn handle(mut conn: TcpStream, max_body: usize, handler: &Handler) -> io::Result<()> {
    conn.set_read_timeout(Some(READ_TIMEOUT))?;
    conn.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let req = match read_request(&mut BufReader::new(&conn), max_body) {
        Ok(r) => r,
        Err(Bad(status, why)) => {
            return write_response(&mut conn, Response::text(status, why), false);
        }
    };
    let head = req.method == "HEAD";
    if req.method != "GET" && req.method != "HEAD" && req.method != "POST" {
        return write_response(&mut conn, Response::text(405, "method not allowed"), false);
    }
    write_response(&mut conn, handler(&req), head)
}

struct Bad(u16, &'static str);

fn read_request(r: &mut impl BufRead, max_body: usize) -> Result<Request, Bad> {
    let mut seen = 0;
    let mut line = |r: &mut dyn BufRead| -> Result<String, Bad> {
        let mut buf = Vec::new();
        // one byte past the limit tells a line that is too long from one that fits
        let n = r
            .take((MAX_HEADER_BYTES - seen + 1) as u64)
            .read_until(b'\n', &mut buf)
            .map_err(|_| Bad(400, "bad request"))?;
        seen += n;
        if n == 0 || seen > MAX_HEADER_BYTES {
            return Err(Bad(431, "request too large"));
        }
        Ok(String::from_utf8_lossy(&buf)
            .trim_end_matches(['\r', '\n'])
            .to_owned())
    };
    let first = line(r)?;
    let mut parts = first.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(Bad(400, "bad request"));
    };
    if !version.starts_with("HTTP/1.") || !target.starts_with('/') {
        return Err(Bad(400, "bad request"));
    }
    let mut content_length = 0usize;
    for _ in 0..=MAX_HEADERS {
        let h = line(r)?;
        if h.is_empty() {
            let url = url::Url::parse(&format!("http://localhost{target}"))
                .map_err(|_| Bad(400, "bad request"))?;
            let mut body = vec![0u8; content_length];
            r.read_exact(&mut body)
                .map_err(|_| Bad(400, "bad request"))?;
            return Ok(Request {
                method: method.to_owned(),
                path: url.path().to_owned(),
                query: url
                    .query_pairs()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect(),
                body,
            });
        }
        if let Some((name, value)) = h.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().map_err(|_| Bad(400, "bad request"))?;
            if content_length > max_body {
                return Err(Bad(413, "body too large"));
            }
        }
    }
    Err(Bad(431, "too many headers"))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Content Too Large",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    }
}

fn write_response(conn: &mut TcpStream, resp: Response, head_only: bool) -> io::Result<()> {
    let mut out = format!(
        "HTTP/1.1 {} {}\r\nConnection: close\r\n",
        resp.status,
        reason(resp.status)
    );
    for (k, v) in &resp.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    match resp.body {
        Body::Bytes(bytes) => {
            if resp.status != 204 {
                out.push_str(&format!("Content-Length: {}\r\n", bytes.len()));
            }
            out.push_str("\r\n");
            conn.write_all(out.as_bytes())?;
            if !head_only {
                conn.write_all(&bytes)?;
            }
            conn.flush()
        }
        Body::Stream(f) => {
            out.push_str("\r\n");
            // the headers go now, not with the first part of the body: the
            // client knows it is connected even when the first one is slow
            conn.write_all(out.as_bytes())?;
            conn.flush()?;
            if head_only { Ok(()) } else { f(conn) }
        }
    }
}

/// Whether the client at the other end of `conn` has gone away. For streams
/// that may go a while without writing anything.
pub fn peer_closed(conn: &TcpStream) -> bool {
    if conn.set_nonblocking(true).is_err() {
        return true;
    }
    let closed = match conn.peek(&mut [0u8; 1]) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => e.kind() != io::ErrorKind::WouldBlock,
    };
    let _ = conn.set_nonblocking(false);
    closed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(addr: std::net::SocketAddr, request: &str) -> String {
        let mut c = TcpStream::connect(addr).unwrap();
        c.write_all(request.as_bytes()).unwrap();
        let mut out = String::new();
        c.read_to_string(&mut out).unwrap();
        out
    }

    fn server() -> std::net::SocketAddr {
        spawn(
            "127.0.0.1:0",
            16,
            Arc::new(|req: &Request| match req.path.as_str() {
                "/echo" => Response::text(
                    200,
                    &format!(
                        "{} {:?} {:?}",
                        req.method,
                        req.param("a"),
                        String::from_utf8_lossy(&req.body)
                    ),
                ),
                "/stream" => Response::stream("text/plain", |w| w.write_all(b"one\ntwo\n")),
                _ => Response::text(404, "no"),
            }),
        )
        .unwrap()
    }

    #[test]
    fn answers_get_with_a_decoded_query() {
        let r = raw(
            server(),
            "GET /echo?a=x%20y&b=2 HTTP/1.1\r\nHost: x\r\n\r\n",
        );
        assert!(r.starts_with("HTTP/1.1 200 OK\r\n"), "{r}");
        assert!(r.contains("Content-Length: "), "{r}");
        assert!(r.ends_with("GET Some(\"x y\") \"\"\n"), "{r}");
    }

    #[test]
    fn reads_a_post_body() {
        let r = raw(
            server(),
            "POST /echo HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello",
        );
        assert!(r.ends_with("POST None \"hello\"\n"), "{r}");
    }

    #[test]
    fn refuses_what_it_does_not_understand() {
        let addr = server();
        assert!(raw(addr, "DELETE /echo HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 405"));
        assert!(raw(addr, "garbage\r\n\r\n").starts_with("HTTP/1.1 400"));
        assert!(raw(addr, "GET nowhere HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 400"));
        assert!(
            raw(addr, "POST /echo HTTP/1.1\r\nContent-Length: 17\r\n\r\n")
                .starts_with("HTTP/1.1 413")
        );
        let huge = format!(
            "GET /echo HTTP/1.1\r\nX: {}\r\n\r\n",
            "a".repeat(MAX_HEADER_BYTES)
        );
        assert!(raw(addr, &huge).starts_with("HTTP/1.1 431"));
        assert!(raw(addr, "GET /missing HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
    }

    #[test]
    fn head_has_no_body_and_streams_do_not_run() {
        let addr = server();
        let r = raw(addr, "HEAD /echo HTTP/1.1\r\n\r\n");
        assert!(
            r.starts_with("HTTP/1.1 200") && r.ends_with("\r\n\r\n"),
            "{r}"
        );
        let r = raw(addr, "HEAD /stream HTTP/1.1\r\n\r\n");
        assert!(!r.contains("one"), "{r}");
    }

    #[test]
    fn streams_write_until_they_are_done() {
        let r = raw(server(), "GET /stream HTTP/1.1\r\n\r\n");
        assert!(
            r.contains("Content-Type: text/plain") && !r.contains("Content-Length"),
            "{r}"
        );
        assert!(r.ends_with("\r\n\r\none\ntwo\n"), "{r}");
    }

    #[test]
    fn notices_a_client_that_left() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server_side, _) = listener.accept().unwrap();
        assert!(!peer_closed(&server_side));
        drop(client);
        thread::sleep(Duration::from_millis(50));
        assert!(peer_closed(&server_side));
    }
}
