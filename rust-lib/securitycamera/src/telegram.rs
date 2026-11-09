//! Talks to a Telegram chat through the Bot API.
//!
//! Errors never contain the URL of a request: it has the bot token in it.

use std::io::Read;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

const DEFAULT_API: &str = "https://api.telegram.org";

/// What the rest of the program needs from Telegram. [`Bot`] is the real one;
/// the tests use a fake.
pub trait Chat: Send + Sync {
    /// Posts text and returns the id of the message, which is what reactions
    /// and replies refer to.
    fn send(&self, text: &str) -> Result<i64, String>;
    fn send_message(&self, text: &str) -> Result<(), String> {
        self.send(text).map(|_| ())
    }
    fn send_photo(&self, jpeg: &[u8], caption: &str) -> Result<(), String>;
    fn send_animation(&self, gif: &[u8], caption: &str) -> Result<(), String>;
    /// Like `send_animation`, as a reply to the message `reply_to`.
    fn send_animation_reply(&self, gif: &[u8], caption: &str, reply_to: i64) -> Result<(), String>;
    /// Posts text with a button that opens the mini app at `url`. Telegram
    /// only allows these buttons in private chats.
    fn send_web_app_button(&self, text: &str, label: &str, url: &str) -> Result<(), String>;
}

/// A text message received in the configured chat.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Message {
    pub id: i64,
    pub text: String,
    /// The sender's username, or their first name if they have none.
    pub from: String,
    /// Seconds since the Unix epoch.
    pub time: i64,
}

impl Message {
    pub fn new(text: &str) -> Message {
        Message { text: text.to_owned(), ..Message::default() }
    }

    /// The command in a message like "/snap" or "/snap@mybot arg" without the
    /// slash, or "" if the message is not a command.
    pub fn command(&self) -> &str {
        if !self.text.starts_with('/') {
            return "";
        }
        let word = self.text.split_whitespace().next().unwrap_or("");
        let word = word.split('@').next().unwrap_or("");
        &word[1..]
    }

    /// The words after the command, e.g. ["0.1"] for "/threshold 0.1".
    pub fn args(&self) -> Vec<&str> {
        if !self.text.starts_with('/') {
            return Vec::new();
        }
        self.text.split_whitespace().skip(1).collect()
    }

    /// Everything after the command, trimmed, with its line breaks kept, e.g.
    /// a JSON document for "/edit_config {...}". It is "" if there is none.
    pub fn payload(&self) -> &str {
        if !self.text.starts_with('/') {
            return "";
        }
        match self.text.find([' ', '\t', '\r', '\n']) {
            Some(i) => self.text[i..].trim(),
            None => "",
        }
    }
}

/// A reaction someone added to a message of the configured chat.
#[derive(Clone, Debug, PartialEq)]
pub struct Reaction {
    /// The message that was reacted to.
    pub message_id: i64,
    /// The emoji of the reaction, e.g. "👍".
    pub emoji: String,
    /// The username of who reacted, or their first name.
    pub from: String,
}

type MessageListener = Box<dyn Fn(&Message) + Send + Sync>;
type ReactionListener = Box<dyn Fn(&Reaction) + Send + Sync>;

/// A Telegram bot that talks to one chat.
pub struct Bot {
    token: String,
    chat_id: String,
    api: String,
    client: ureq::Agent,
    // uploads take longer than the other requests, the HD GIFs are big
    upload_client: ureq::Agent,
    // answering "is this user in the group" must not take long
    check_client: ureq::Agent,
    // long polling needs a timeout longer than the poll
    poll_client: ureq::Agent,
    poll_timeout: Duration,

    listeners: RwLock<Vec<MessageListener>>,
    reactions: RwLock<Vec<ReactionListener>>,
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(timeout).build()
}

const REQUEST_FAILED: &str = "telegram: request failed";

/// The status and the start of the body of a reply, whatever the status.
/// `Err` is only a request that got no reply, without saying why: that text
/// could have the URL, and so the token.
fn reply(r: Result<ureq::Response, ureq::Error>, limit: u64) -> Result<(u16, String), String> {
    match r {
        Ok(resp) | Err(ureq::Error::Status(_, resp)) => {
            let status = resp.status();
            let mut body = Vec::new();
            let _ = resp.into_reader().take(limit).read_to_end(&mut body);
            Ok((status, String::from_utf8_lossy(&body).into_owned()))
        }
        Err(_) => Err(REQUEST_FAILED.into()),
    }
}

fn check_ok((status, body): (u16, String)) -> Result<String, String> {
    if status == 200 {
        return Ok(body);
    }
    let short: String = body.chars().take(512).collect();
    Err(format!("telegram: status {status}: {short}"))
}

impl Bot {
    pub fn new(token: &str, chat_id: &str) -> Result<Bot, String> {
        Bot::with_api(token, chat_id, DEFAULT_API)
    }

    /// A bot that talks to another server than Telegram's.
    pub fn with_api(token: &str, chat_id: &str, api: &str) -> Result<Bot, String> {
        if token.is_empty() || chat_id.is_empty() {
            return Err("telegram: token and chat id are required".into());
        }
        Ok(Bot {
            token: token.to_owned(),
            chat_id: chat_id.to_owned(),
            api: api.trim_end_matches('/').to_owned(),
            client: agent(Duration::from_secs(10)),
            upload_client: agent(Duration::from_secs(120)),
            check_client: agent(Duration::from_secs(5)),
            poll_client: agent(Duration::from_secs(40)),
            poll_timeout: Duration::from_secs(30),
            listeners: RwLock::new(Vec::new()),
            reactions: RwLock::new(Vec::new()),
        })
    }

    /// How long a poll for updates waits on Telegram's side.
    pub fn set_poll_timeout(&mut self, d: Duration) {
        self.poll_timeout = d;
    }

    fn url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", self.api, self.token, method)
    }

    /// Calls a method with a JSON body and returns the body of the reply.
    fn post(&self, method: &str, payload: Value) -> Result<String, String> {
        let r = self.client.post(&self.url(method)).send_json(payload);
        check_ok(reply(r, 64 << 10)?)
    }

    /// Whether the Telegram user is in the configured chat. Telegram has no
    /// call that lists the members of a group, only one that tells about a
    /// single user: this is it (getChatMember). It is only guaranteed to work
    /// for other users if the bot is an administrator of the group. A user
    /// Telegram does not know counts as not being a member; any other failure
    /// is an error, and says nothing about the user.
    pub fn is_member(&self, user_id: i64) -> Result<bool, String> {
        #[derive(Deserialize, Default)]
        struct Out {
            #[serde(default)]
            description: String,
            #[serde(default)]
            result: Member,
        }
        #[derive(Deserialize, Default)]
        struct Member {
            #[serde(default)]
            status: String,
            #[serde(default)]
            is_member: bool,
        }
        let r = self.check_client.post(&self.url("getChatMember")).send_json(json!({"chat_id": self.chat_id, "user_id": user_id}));
        let (status, body) = reply(r, 4096)?;
        // the description of an error is in the body, which may not be JSON (a proxy's page)
        let out: Out = serde_json::from_str(&body).unwrap_or_default();
        if status == 400 && out.description.to_lowercase().contains("user not found") {
            return Ok(false);
        }
        if status != 200 {
            return Err(format!("telegram: getChatMember: status {status}: {}", out.description));
        }
        match out.result.status.as_str() {
            "creator" | "administrator" | "member" => Ok(true),
            // still in the chat, but muted or limited: is_member says whether
            "restricted" => Ok(out.result.is_member),
            "left" | "kicked" => Ok(false),
            other => Err(format!("telegram: getChatMember: unknown status {other:?}")),
        }
    }

    /// Makes the menu button next to the message box open the mini app at
    /// `url`, in every private chat with the bot. It is the bot's default menu
    /// button, not the one of the configured chat: that may be a group, and a
    /// group has no menu button.
    pub fn set_menu_web_app(&self, label: &str, url: &str) -> Result<(), String> {
        self.post("setChatMenuButton", json!({"menu_button": {"type": "web_app", "text": label, "web_app": {"url": url}}})).map(|_| ())
    }

    /// Whether the Telegram API answers at all. Any HTTP reply counts, even an
    /// error one: it is the connection that is being tested.
    pub fn reachable(&self) -> bool {
        matches!(self.client.get(&self.url("getMe")).call(), Ok(_) | Err(ureq::Error::Status(..)))
    }

    /// Shows "typing…" in the chat while a GIF is on its way. It is only a
    /// courtesy: if it fails the GIF is sent all the same.
    fn send_typing(&self) {
        let _ = self.post("sendChatAction", json!({"chat_id": self.chat_id, "action": "typing"}));
    }

    fn upload(&self, method: &str, field: &str, filename: &str, data: &[u8], caption: &str, reply_to: i64) -> Result<(), String> {
        let boundary = format!("----securitycamera{:016x}", boundary_seed());
        let mut body: Vec<u8> = Vec::with_capacity(data.len() + 512);
        let mut text_field = |name: &str, value: &str| {
            body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
        };
        text_field("chat_id", &self.chat_id);
        if !caption.is_empty() {
            text_field("caption", caption);
        }
        if reply_to != 0 {
            text_field("reply_parameters", &format!(r#"{{"message_id":{reply_to},"allow_sending_without_reply":true}}"#));
        }
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let r = self
            .upload_client
            .post(&self.url(method))
            .set("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
            .send_bytes(&body);
        check_ok(reply(r, 512)?).map(|_| ())
    }

    /// Registers a listener for messages sent to the configured chat.
    /// Messages from other chats are ignored. It is safe to call while `run`
    /// is active.
    pub fn add_listener(&self, l: impl Fn(&Message) + Send + Sync + 'static) {
        self.listeners.write().unwrap_or_else(|e| e.into_inner()).push(Box::new(l));
    }

    /// Registers a listener for the reactions added to messages in the
    /// configured chat, one call per emoji added (removing one is not
    /// reported). Telegram only sends reactions to a bot that is an
    /// administrator of a group; in a private chat it always does.
    pub fn add_reaction_listener(&self, l: impl Fn(&Reaction) + Send + Sync + 'static) {
        self.reactions.write().unwrap_or_else(|e| e.into_inner()).push(Box::new(l));
    }

    /// Long-polls for new messages and dispatches the ones from the configured
    /// chat to the listeners, in registration order, until `stop` is set.
    /// Listeners run synchronously, so a slow one delays the following
    /// messages. Transient request failures are retried after a short pause.
    pub fn run(&self, stop: &AtomicBool) {
        let mut offset = 0i64;
        while !stop.load(Ordering::SeqCst) {
            let updates = match self.get_updates(offset) {
                Ok(u) => u,
                Err(e) => {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    eprintln!("{e}");
                    let until = Instant::now() + Duration::from_secs(5);
                    while Instant::now() < until && !stop.load(Ordering::SeqCst) {
                        thread::sleep(Duration::from_millis(50));
                    }
                    continue;
                }
            };
            for u in updates {
                offset = u.update_id + 1;
                if let Some(r) = &u.message_reaction {
                    eprintln!("telegram: reaction update on message {} in chat {} (configured chat {})", r.message_id, r.chat.id, self.chat_id);
                    if r.chat.id.to_string() == self.chat_id {
                        let from = r.user.name();
                        for emoji in added_emoji(&r.old_reaction, &r.new_reaction) {
                            self.dispatch_reaction(&Reaction { message_id: r.message_id, emoji, from: from.clone() });
                        }
                    }
                }
                let Some(m) = &u.message else { continue };
                if m.text.is_empty() || m.chat.id.to_string() != self.chat_id {
                    continue;
                }
                self.dispatch(&Message { id: m.message_id, text: m.text.clone(), from: m.from.name(), time: m.date });
            }
        }
    }

    fn dispatch_reaction(&self, r: &Reaction) {
        for l in self.reactions.read().unwrap_or_else(|e| e.into_inner()).iter() {
            l(r);
        }
    }

    fn dispatch(&self, m: &Message) {
        for l in self.listeners.read().unwrap_or_else(|e| e.into_inner()).iter() {
            l(m);
        }
    }

    fn get_updates(&self, offset: i64) -> Result<Vec<Update>, String> {
        let url = format!(
            "{}?offset={offset}&timeout={}&allowed_updates=%5B%22message%22,%22message_reaction%22%5D",
            self.url("getUpdates"),
            self.poll_timeout.as_secs()
        );
        let body = check_ok(reply(self.poll_client.get(&url).call(), 8 << 20)?)?;
        #[derive(Deserialize)]
        struct Out {
            #[serde(default)]
            result: Vec<Update>,
        }
        serde_json::from_str::<Out>(&body).map(|o| o.result).map_err(|e| format!("telegram: decode updates: {e}"))
    }
}

/// A seed for the boundary of a multipart body: it only has to not be in the data.
fn boundary_seed() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    RandomState::new().hash_one(Instant::now())
}

impl Chat for Bot {
    fn send(&self, text: &str) -> Result<i64, String> {
        let body = self.post("sendMessage", json!({"chat_id": self.chat_id, "text": text}))?;
        // the message is sent, a reply that can't be read only costs its id
        #[derive(Deserialize, Default)]
        struct Out {
            #[serde(default)]
            result: Sent,
        }
        #[derive(Deserialize, Default)]
        struct Sent {
            #[serde(default)]
            message_id: i64,
        }
        Ok(serde_json::from_str::<Out>(&body).unwrap_or_default().result.message_id)
    }

    fn send_photo(&self, jpeg: &[u8], caption: &str) -> Result<(), String> {
        self.upload("sendPhoto", "photo", "snap.jpg", jpeg, caption, 0)
    }

    fn send_animation(&self, gif: &[u8], caption: &str) -> Result<(), String> {
        self.send_typing();
        self.upload("sendAnimation", "animation", "alarm.gif", gif, caption, 0)
    }

    fn send_animation_reply(&self, gif: &[u8], caption: &str, reply_to: i64) -> Result<(), String> {
        self.send_typing();
        self.upload("sendAnimation", "animation", "alarm.gif", gif, caption, reply_to)
    }

    fn send_web_app_button(&self, text: &str, label: &str, url: &str) -> Result<(), String> {
        self.post(
            "sendMessage",
            json!({
                "chat_id": self.chat_id,
                "text": text,
                "reply_markup": {"inline_keyboard": [[{"text": label, "web_app": {"url": url}}]]},
            }),
        )
        .map(|_| ())
    }
}

impl crate::miniapp::Members for Bot {
    fn is_member(&self, user_id: i64) -> Result<bool, String> {
        Bot::is_member(self, user_id)
    }
}

/// One reaction as Telegram describes it: custom emoji and paid reactions
/// have no `emoji`.
#[derive(Deserialize, Default, Clone)]
struct ReactionType {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    emoji: String,
}

#[derive(Deserialize, Default)]
struct ChatRef {
    #[serde(default)]
    id: i64,
}

#[derive(Deserialize, Default)]
struct UserRef {
    #[serde(default)]
    username: String,
    #[serde(default)]
    first_name: String,
}

impl UserRef {
    fn name(&self) -> String {
        if self.username.is_empty() { self.first_name.clone() } else { self.username.clone() }
    }
}

#[derive(Deserialize)]
struct Update {
    update_id: i64,
    /// Sent when somebody changes their reactions to a message.
    message_reaction: Option<ReactionUpdate>,
    message: Option<IncomingMessage>,
}

#[derive(Deserialize)]
struct ReactionUpdate {
    #[serde(default)]
    chat: ChatRef,
    #[serde(default)]
    message_id: i64,
    #[serde(default)]
    user: UserRef,
    #[serde(default)]
    old_reaction: Vec<ReactionType>,
    #[serde(default)]
    new_reaction: Vec<ReactionType>,
}

#[derive(Deserialize)]
struct IncomingMessage {
    #[serde(default)]
    message_id: i64,
    #[serde(default)]
    date: i64,
    #[serde(default)]
    text: String,
    #[serde(default)]
    chat: ChatRef,
    #[serde(default)]
    from: UserRef,
}

/// The emoji in `new` that are not in `old`.
fn added_emoji(old: &[ReactionType], new: &[ReactionType]) -> Vec<String> {
    new.iter()
        .filter(|n| n.kind == "emoji")
        .filter(|n| !old.iter().any(|o| o.kind == "emoji" && o.emoji == n.emoji))
        .map(|n| n.emoji.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::httpd::{self, Request, Response};
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    /// A fake Telegram: records what it is asked and answers `answer`.
    struct Fake {
        seen: Arc<Mutex<Vec<Seen>>>,
        addr: std::net::SocketAddr,
    }

    #[derive(Clone, Debug)]
    struct Seen {
        path: String,
        query: String,
        body: Vec<u8>,
    }

    impl Seen {
        fn json(&self) -> Value {
            serde_json::from_slice(&self.body).unwrap_or(Value::Null)
        }
    }

    fn fake(answer: impl Fn(&Request) -> Response + Send + Sync + 'static) -> Fake {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let addr = httpd::spawn(
            "127.0.0.1:0",
            1 << 20,
            Arc::new(move |req: &Request| {
                let query = req.query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
                log.lock().unwrap().push(Seen { path: req.path.clone(), query, body: req.body.clone() });
                answer(req)
            }),
        )
        .unwrap();
        Fake { seen, addr }
    }

    impl Fake {
        fn bot(&self, chat: &str) -> Bot {
            Bot::with_api("TOKEN", chat, &format!("http://{}", self.addr)).unwrap()
        }
        fn requests(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }
        fn last(&self) -> Seen {
            self.requests().pop().expect("no request")
        }
    }

    fn ok(body: &str) -> Response {
        Response::new(200, "application/json", body)
    }

    fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
        hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|i| i + from)
    }

    /// The part `name` of a multipart body, as bytes.
    fn part(body: &[u8], name: &str) -> Option<Vec<u8>> {
        let start = find(body, format!("name=\"{name}\"").as_bytes(), 0)?;
        let from = find(body, b"\r\n\r\n", start)? + 4;
        let boundary_end = find(body, b"\r\n", 0)?;
        let boundary = &body[..boundary_end]; // --boundary
        let to = find(body, &[b"\r\n", boundary].concat(), from)?;
        Some(body[from..to].to_vec())
    }

    #[test]
    fn send_message_posts_json() {
        let f = fake(|_| ok("{}"));
        f.bot("42").send_message("hi").unwrap();
        let r = f.last();
        assert_eq!(r.path, "/botTOKEN/sendMessage");
        assert_eq!(r.json(), json!({"chat_id": "42", "text": "hi"}));
        assert!(Bot::new("", "42").is_err());
        assert!(Bot::new("t", "").is_err());
    }

    #[test]
    fn send_returns_the_message_id() {
        let f = fake(|_| ok(r#"{"ok":true,"result":{"message_id":321}}"#));
        assert_eq!(f.bot("42").send("hi"), Ok(321));
        let f = fake(|_| ok("not json"));
        assert_eq!(f.bot("42").send("hi"), Ok(0));
    }

    #[test]
    fn errors_have_the_status_but_not_the_token() {
        let f = fake(|_| Response::text(500, "boom"));
        let e = f.bot("42").send("hi").unwrap_err();
        assert!(e.contains("status 500") && e.contains("boom") && !e.contains("TOKEN"), "{e}");
        let bot = Bot::with_api("SECRETTOKEN", "42", "http://127.0.0.1:1").unwrap();
        let e = bot.send("hi").unwrap_err();
        assert!(!e.contains("SECRETTOKEN") && e.contains("request failed"), "{e}");
    }

    #[test]
    fn send_animation_uploads_after_a_typing_action() {
        let f = fake(|_| ok("{}"));
        f.bot("-42").send_animation(b"GIF89a", "cap").unwrap();
        let reqs = f.requests();
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].path, "/botTOKEN/sendChatAction");
        assert_eq!(reqs[0].json()["action"], "typing");
        assert_eq!(reqs[1].path, "/botTOKEN/sendAnimation");
        assert_eq!(part(&reqs[1].body, "chat_id").unwrap(), b"-42");
        assert_eq!(part(&reqs[1].body, "caption").unwrap(), b"cap");
        assert_eq!(part(&reqs[1].body, "animation").unwrap(), b"GIF89a");
        assert!(part(&reqs[1].body, "reply_parameters").is_none());
    }

    #[test]
    fn send_animation_reply_names_the_message() {
        let f = fake(|_| ok("{}"));
        f.bot("42").send_animation_reply(b"GIF89a", "cap", 55).unwrap();
        let reply = part(&f.last().body, "reply_parameters").unwrap();
        assert!(String::from_utf8(reply).unwrap().contains(r#""message_id":55"#));
    }

    #[test]
    fn send_photo_uploads_the_jpeg() {
        let f = fake(|_| ok("{}"));
        f.bot("1").send_photo(b"JPG", "").unwrap();
        let r = f.last();
        assert_eq!(r.path, "/botTOKEN/sendPhoto");
        assert_eq!(part(&r.body, "photo").unwrap(), b"JPG");
        assert!(part(&r.body, "caption").is_none());
        // binary data survives the multipart body
        let bytes: Vec<u8> = (0..=255).collect();
        f.bot("1").send_photo(&bytes, "x").unwrap();
        assert_eq!(part(&f.last().body, "photo").unwrap(), bytes);
    }

    #[test]
    fn commands_args_and_payload() {
        for (text, want) in [("/snap", "snap"), ("/snap@mybot now", "snap"), ("hello", ""), ("/", "")] {
            assert_eq!(Message::new(text).command(), want, "{text:?}");
        }
        for (text, want) in [("/threshold 0.1", "0.1"), ("/threshold@mybot 0.1 x", "0.1 x"), ("/snap", ""), ("hello there", "")] {
            assert_eq!(Message::new(text).args().join(" "), want, "{text:?}");
        }
        for (text, want) in [
            ("/edit_config", ""),
            ("/edit_config   ", ""),
            ("/edit_config {\"a\": 1}", "{\"a\": 1}"),
            ("/edit_config@bot\n{\n  \"a\": 1\n}", "{\n  \"a\": 1\n}"),
            ("hello there", ""),
        ] {
            assert_eq!(Message::new(text).payload(), want, "{text:?}");
        }
    }

    #[test]
    fn web_app_buttons() {
        let f = fake(|_| ok("{}"));
        let bot = f.bot("42");
        bot.send_web_app_button("hello", "Open", "https://x.ts.net").unwrap();
        let r = f.last();
        assert_eq!(r.path, "/botTOKEN/sendMessage");
        let b = r.json();
        assert_eq!(b["text"], "hello");
        assert_eq!(b["reply_markup"]["inline_keyboard"][0][0], json!({"text": "Open", "web_app": {"url": "https://x.ts.net"}}));

        bot.set_menu_web_app("Camera", "https://x.ts.net").unwrap();
        let r = f.last();
        assert_eq!(r.path, "/botTOKEN/setChatMenuButton");
        let b = r.json();
        assert!(b.get("chat_id").is_none(), "the menu button must be the default one: {b}");
        assert_eq!(b["menu_button"], json!({"type": "web_app", "text": "Camera", "web_app": {"url": "https://x.ts.net"}}));
    }

    #[test]
    fn is_member_by_status() {
        for (status, want) in [("creator", true), ("administrator", true), ("member", true), ("left", false), ("kicked", false)] {
            let f = fake(move |_| ok(&format!(r#"{{"ok":true,"result":{{"status":"{status}"}}}}"#)));
            assert_eq!(f.bot("-100123").is_member(42), Ok(want), "{status}");
            let r = f.last();
            assert_eq!(r.path, "/botTOKEN/getChatMember");
            assert_eq!(r.json(), json!({"chat_id": "-100123", "user_id": 42}));
        }
        // restricted users are in the chat only if is_member says so
        for want in [true, false] {
            let f = fake(move |_| ok(&format!(r#"{{"ok":true,"result":{{"status":"restricted","is_member":{want}}}}}"#)));
            assert_eq!(f.bot("-1").is_member(1), Ok(want));
        }
    }

    #[test]
    fn is_member_unknown_user_is_a_no_and_other_failures_are_errors() {
        let f = fake(|_| Response::new(400, "application/json", r#"{"ok":false,"description":"Bad Request: user not found"}"#));
        assert_eq!(f.bot("-1").is_member(1), Ok(false));
        for (name, status, body) in [
            ("bad chat", 400, r#"{"ok":false,"description":"Bad Request: chat not found"}"#),
            ("not admin", 403, r#"{"ok":false,"description":"Forbidden: bot is not a member"}"#),
            ("server", 500, "oops"),
            ("unknown status", 200, r#"{"ok":true,"result":{"status":"alien"}}"#),
        ] {
            let f = fake(move |_| Response::new(status, "application/json", body));
            assert!(f.bot("-1").is_member(1).is_err(), "{name}");
        }
    }

    #[test]
    fn reachable_counts_any_reply() {
        let f = fake(|_| Response::text(401, "unauthorized"));
        assert!(f.bot("1").reachable());
        assert!(!Bot::with_api("T", "1", "http://127.0.0.1:1").unwrap().reachable());
    }

    /// Runs the bot against a server that hands out `first` and then nothing,
    /// until `done` says it saw what it wanted.
    fn poll(chat: &str, first: &'static str, listen: impl FnOnce(&Bot, mpsc::Sender<()>)) -> Fake {
        let calls = Arc::new(AtomicUsize::new(0));
        let f = fake(move |_| ok(if calls.fetch_add(1, Ordering::SeqCst) == 0 { first } else { r#"{"ok":true,"result":[]}"# }));
        let mut bot = f.bot(chat);
        bot.set_poll_timeout(Duration::ZERO);
        let (tx, rx) = mpsc::channel();
        listen(&bot, tx);
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let t = thread::spawn(move || bot.run(&stop2));
        // the second poll comes after the first one was dispatched
        while f.requests().len() < 2 {
            thread::sleep(Duration::from_millis(5));
        }
        let _ = rx.try_recv();
        stop.store(true, Ordering::SeqCst);
        t.join().unwrap();
        f
    }

    #[test]
    fn listeners_get_the_messages_of_the_chat() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        let f = poll(
            "-42",
            r#"{"ok":true,"result":[
                {"update_id":10,"message":{"message_id":1,"date":100,"text":"hello","chat":{"id":-42},"from":{"username":"bob"}}},
                {"update_id":11,"message":{"message_id":2,"date":101,"text":"other chat","chat":{"id":7},"from":{"first_name":"Eve"}}},
                {"update_id":12,"message":{"message_id":3,"date":102,"chat":{"id":-42},"from":{"first_name":"Al"}}}]}"#,
            move |bot, _| bot.add_listener(move |m| g.lock().unwrap().push(m.clone())),
        );
        assert_eq!(*got.lock().unwrap(), [Message { id: 1, text: "hello".into(), from: "bob".into(), time: 100 }]);
        let reqs = f.requests();
        assert_eq!(reqs[0].path, "/botTOKEN/getUpdates");
        assert!(reqs[0].query.contains("offset=0"), "{}", reqs[0].query);
        assert!(reqs[1].query.contains("offset=13"), "the next poll skips what was seen: {}", reqs[1].query);
    }

    #[test]
    fn reaction_listeners_get_only_new_emoji_of_the_chat() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        let f = poll(
            "-42",
            r#"{"ok":true,"result":[
                {"update_id":1,"message_reaction":{"chat":{"id":-42},"message_id":9,"user":{"username":"bob"},
                    "old_reaction":[{"type":"emoji","emoji":"❤"}],
                    "new_reaction":[{"type":"emoji","emoji":"❤"},{"type":"emoji","emoji":"👍"},{"type":"custom_emoji"}]}},
                {"update_id":2,"message_reaction":{"chat":{"id":7},"message_id":9,"user":{"first_name":"Eve"},
                    "old_reaction":[],"new_reaction":[{"type":"emoji","emoji":"👍"}]}},
                {"update_id":3,"message_reaction":{"chat":{"id":-42},"message_id":9,"user":{"first_name":"Al"},
                    "old_reaction":[{"type":"emoji","emoji":"👍"}],"new_reaction":[]}}]}"#,
            move |bot, _| bot.add_reaction_listener(move |r| g.lock().unwrap().push(r.clone())),
        );
        assert_eq!(*got.lock().unwrap(), [Reaction { message_id: 9, emoji: "👍".into(), from: "bob".into() }]);
        assert!(f.requests()[0].query.contains("message_reaction"), "getUpdates does not ask for reactions");
    }

    #[test]
    fn run_stops_when_asked_even_after_errors() {
        let f = fake(|_| Response::text(500, "down"));
        let mut bot = f.bot("1");
        bot.set_poll_timeout(Duration::ZERO);
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let t = thread::spawn(move || bot.run(&stop2));
        while f.requests().is_empty() {
            thread::sleep(Duration::from_millis(5));
        }
        stop.store(true, Ordering::SeqCst);
        t.join().unwrap();
    }
}
