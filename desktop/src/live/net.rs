//! The network half of Pixmith Live (`docs/LIVE_PROTOCOL.md`): a loopback TCP listener, newline-delimited
//! JSON-RPC 2.0 with the `hello` handshake, and the discovery file.
//!
//! Everything here runs on background threads and never touches a document. A request after `hello` is handed to
//! the UI thread through a channel ([`Server::take_requests`]) together with a reply channel; the connection's
//! thread then waits for the answer, which keeps the answers in order.

use pixmith_core::formats::json::{self, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// The protocol version this app speaks.
pub const PROTOCOL: u32 = 1;
/// The largest message, in bytes: 64 MiB.
pub const MAX_MESSAGE: usize = 64 << 20;
/// Most connections at once; a local tool needs one.
const MAX_CONNECTIONS: usize = 16;
/// How long a connection may stay silent before `hello`.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the UI thread may take to answer one request.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(120);
/// A client counts as a running game while it polled this recently.
const GAME_ALIVE: Duration = Duration::from_secs(1);
/// `game.capture` waits this long unless `timeout_ms` says otherwise, and never longer than the maximum.
const CAPTURE_DEFAULT_MS: u64 = 3000;
const CAPTURE_MAX_MS: u64 = 10_000;

pub const ERR_PARSE: i32 = -32700;
pub const ERR_INVALID_REQUEST: i32 = -32600;
pub const ERR_METHOD: i32 = -32601;
pub const ERR_PARAMS: i32 = -32602;
pub const ERR_UNAUTHORIZED: i32 = -32001;
pub const ERR_UNKNOWN_DOCUMENT: i32 = -32004;
pub const ERR_STALE: i32 = -32009;
pub const ERR_BUSY: i32 = -32010;
pub const ERR_UNSAVED: i32 = -32011;
pub const ERR_NO_GAME: i32 = -32012;
pub const ERR_OTHER: i32 = -32000;

#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

impl RpcError {
    pub fn new(code: i32, message: impl Into<String>) -> RpcError {
        RpcError { code, message: message.into() }
    }
}

pub type Reply = Result<Value, RpcError>;

/// A request waiting for the UI thread.
pub struct Request {
    pub method: String,
    pub params: Value,
    reply: Sender<Reply>,
}

impl Request {
    pub fn answer(self, reply: Reply) {
        let _ = self.reply.send(reply);
    }
}

/// A connected client, as `hello` introduced it.
#[derive(Clone, Debug, PartialEq)]
pub struct Client {
    pub conn: u64,
    pub name: String,
    pub version: String,
}

struct Shared {
    token: String,
    stop: AtomicBool,
    /// Disconnect was pressed: `hello` is refused until Live mode is switched off and on again.
    blocked: AtomicBool,
    ctx: Mutex<Option<eframe::egui::Context>>,
    clients: Mutex<Vec<Client>>,
    streams: Mutex<HashMap<u64, TcpStream>>,
    next_conn: AtomicU64,
    /// The game view relay; `game_changed` wakes a `game.capture` waiting for its frame.
    game: Mutex<Game>,
    game_changed: Condvar,
}

/// A client that calls `game.poll`: a running game.
struct Poller {
    conn: u64,
    name: String,
    at: Instant,
}

/// One `game.capture` in flight.
struct Slot {
    conn: u64,
    name: String,
    /// Handed out by a `game.poll` already.
    delivered: bool,
    frame: Option<Frame>,
}

struct Frame {
    png: String,
    w: Value,
    h: Value,
}

#[derive(Default)]
struct Game {
    pollers: Vec<Poller>,
    slots: HashMap<String, Slot>,
    next_request: u64,
}

impl Shared {
    fn game(&self) -> std::sync::MutexGuard<'_, Game> {
        self.game.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Forgets a connection's game state; waiting captures notice and fail.
    fn forget_game(&self, conn: Option<u64>) {
        self.game().pollers.retain(|p| conn.is_some_and(|c| p.conn != c));
        self.game_changed.notify_all();
    }

    fn wake(&self) {
        if let Some(ctx) = self.ctx.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            ctx.request_repaint();
        }
    }
}

/// A running Live server. Dropping it closes the port and removes the discovery file.
pub struct Server {
    pub port: u16,
    pub token: String,
    requests: Receiver<Request>,
    shared: Arc<Shared>,
    discovery: PathBuf,
    accepting: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    /// Binds `127.0.0.1:0`, starts accepting and writes `<app_dir>/Live/<pid>.json`.
    pub fn start(app_dir: &Path, ctx: Option<eframe::egui::Context>) -> std::io::Result<Server> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let addr = listener.local_addr()?;
        let token = random_token()?;
        let shared = Arc::new(Shared {
            token: token.clone(),
            stop: AtomicBool::new(false),
            blocked: AtomicBool::new(false),
            ctx: Mutex::new(ctx),
            clients: Mutex::new(Vec::new()),
            streams: Mutex::new(HashMap::new()),
            next_conn: AtomicU64::new(1),
            game: Mutex::new(Game::default()),
            game_changed: Condvar::new(),
        });
        let discovery = write_discovery(app_dir, addr.port(), &token)?;
        let (tx, requests) = mpsc::channel();
        let accept_shared = shared.clone();
        let accepting = std::thread::Builder::new()
            .name("pixmith-live-accept".into())
            .spawn(move || accept(listener, accept_shared, tx))?;
        Ok(Server { port: addr.port(), token, requests, shared, discovery, accepting: Some(accepting) })
    }

    /// Gives the network threads a way to wake the UI when a request arrives.
    pub fn set_context(&self, ctx: eframe::egui::Context) {
        *self.shared.ctx.lock().unwrap_or_else(|e| e.into_inner()) = Some(ctx);
    }

    /// Every request that arrived since the last call.
    pub fn take_requests(&self) -> Vec<Request> {
        self.requests.try_iter().collect()
    }

    /// The clients that completed `hello` and are still connected.
    pub fn clients(&self) -> Vec<Client> {
        self.shared.clients.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Closes every connection (the port stays open; a client may connect again). The clients are forgotten right
    /// away: on Windows a shutdown does not always wake a connection thread blocked in a read.
    pub fn disconnect_all(&self) {
        for s in self.shared.streams.lock().unwrap_or_else(|e| e.into_inner()).drain().map(|(_, s)| s) {
            let _ = s.shutdown(Shutdown::Both);
        }
        self.shared.clients.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.shared.forget_game(None);
        self.shared.wake();
    }

    /// The Disconnect button: closes every connection and refuses later `hello`s (`-32001`, "disconnected by the
    /// user") until Live mode is turned off and on again, which starts a new server.
    pub fn disconnect_by_user(&self) {
        self.shared.blocked.store(true, Ordering::SeqCst);
        self.disconnect_all();
    }

    pub fn discovery_path(&self) -> &Path {
        &self.discovery
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.disconnect_all();
        // Wake the accept loop so it sees the flag and drops the listener.
        let _ = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], self.port)), Duration::from_millis(200));
        if let Some(thread) = self.accepting.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.discovery);
    }
}

fn random_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

// MARK: Discovery

/// The folder holding the discovery files.
pub fn discovery_dir(app_dir: &Path) -> PathBuf {
    app_dir.join("Live")
}

fn write_discovery(app_dir: &Path, port: u16, token: &str) -> std::io::Result<PathBuf> {
    let dir = discovery_dir(app_dir);
    std::fs::create_dir_all(&dir)?;
    remove_stale(&dir);
    let started = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let num = |n: u64| Value::Num(n as f64);
    let text = json::to_string(&Value::Obj(vec![
        ("protocol".into(), num(PROTOCOL as u64)),
        ("pid".into(), num(std::process::id() as u64)),
        ("port".into(), num(port as u64)),
        ("token".into(), Value::Str(token.into())),
        ("app".into(), Value::Str("Pixmith".into())),
        ("version".into(), Value::Str(env!("CARGO_PKG_VERSION").into())),
        ("front_end".into(), Value::Str("egui".into())),
        ("platform".into(), Value::Str(platform().into())),
        ("started".into(), num(started)),
    ]));
    let path = dir.join(format!("{}.json", std::process::id()));
    let tmp = dir.join(format!("{}.json.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(text.as_bytes())?;
    drop(file);
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Deletes discovery files of instances that are gone (their port refuses connections).
fn remove_stale(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let port = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| json::parse(&t).ok())
            .and_then(|v| v.get("port").and_then(Value::as_f64));
        let alive = port.is_some_and(|p| {
            TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], p as u16)), Duration::from_millis(150))
                .is_ok()
        });
        if !alive {
            let _ = std::fs::remove_file(&path);
        }
    }
}

// MARK: Connections

fn accept(listener: TcpListener, shared: Arc<Shared>, tx: Sender<Request>) {
    for stream in listener.incoming() {
        if shared.stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        if shared.streams.lock().unwrap_or_else(|e| e.into_inner()).len() >= MAX_CONNECTIONS {
            continue;
        }
        let (shared, tx) = (shared.clone(), tx.clone());
        let conn = shared.next_conn.fetch_add(1, Ordering::SeqCst);
        if let Ok(copy) = stream.try_clone() {
            shared.streams.lock().unwrap_or_else(|e| e.into_inner()).insert(conn, copy);
        }
        let _ = std::thread::Builder::new().name("pixmith-live-conn".into()).spawn(move || {
            serve(conn, stream, &shared, &tx);
            shared.streams.lock().unwrap_or_else(|e| e.into_inner()).remove(&conn);
            shared.clients.lock().unwrap_or_else(|e| e.into_inner()).retain(|c| c.conn != conn);
            shared.forget_game(Some(conn));
            shared.wake();
        });
    }
}

enum Line {
    Message,
    Closed,
    TooLong,
}

fn read_message(reader: &mut BufReader<TcpStream>, buf: &mut Vec<u8>) -> Line {
    buf.clear();
    match reader.by_ref().take(MAX_MESSAGE as u64 + 1).read_until(b'\n', buf) {
        Ok(0) | Err(_) => Line::Closed,
        Ok(_) if buf.last() == Some(&b'\n') => {
            buf.pop();
            if buf.len() > MAX_MESSAGE { Line::TooLong } else { Line::Message }
        }
        Ok(_) if buf.len() > MAX_MESSAGE => Line::TooLong,
        Ok(_) => Line::Closed, // cut off in the middle of a message
    }
}

fn response(id: &Value, reply: &Reply) -> String {
    let mut members = vec![("jsonrpc".to_string(), Value::Str("2.0".into())), ("id".to_string(), id.clone())];
    match reply {
        Ok(result) => members.push(("result".into(), result.clone())),
        Err(e) => members.push((
            "error".into(),
            Value::Obj(vec![
                ("code".into(), Value::Num(e.code as f64)),
                ("message".into(), Value::Str(e.message.clone())),
            ]),
        )),
    }
    let mut text = json::to_string(&Value::Obj(members));
    text.push('\n');
    text
}

fn serve(conn: u64, stream: TcpStream, shared: &Shared, tx: &Sender<Request>) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(HELLO_TIMEOUT));
    let Ok(mut out) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    let mut authed = false;
    let mut name = String::new();
    let send = |out: &mut TcpStream, id: &Value, reply: &Reply| out.write_all(response(id, reply).as_bytes()).is_ok();
    loop {
        match read_message(&mut reader, &mut buf) {
            Line::Closed => return,
            Line::TooLong => {
                let e = RpcError::new(ERR_INVALID_REQUEST, "message larger than 64 MiB");
                send(&mut out, &Value::Null, &Err(e));
                return;
            }
            Line::Message => {}
        }
        // Disconnected while this thread waited in the read.
        if !shared.streams.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&conn) {
            return;
        }
        if buf.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let parsed =
            std::str::from_utf8(&buf).map_err(|_| "the message is not UTF-8".to_string()).and_then(json::parse);
        let message = match parsed {
            Ok(m) => m,
            Err(e) => {
                send(&mut out, &Value::Null, &Err(RpcError::new(ERR_PARSE, e)));
                if authed {
                    continue;
                }
                return;
            }
        };
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str).map(str::to_string);
        let params = message.get("params").cloned().unwrap_or(Value::Obj(Vec::new()));
        let (Some(method), true) = (method, matches!(message, Value::Obj(_))) else {
            send(
                &mut out,
                &id.unwrap_or(Value::Null),
                &Err(RpcError::new(ERR_INVALID_REQUEST, "not a JSON-RPC request")),
            );
            if authed {
                continue;
            }
            return;
        };
        // Notifications are not used in protocol 1.
        let Some(id) = id else {
            if authed {
                continue;
            }
            return;
        };
        if !authed {
            let client = (method == "hello").then(|| hello(&params, shared)).transpose();
            match client {
                Ok(Some(c)) => {
                    authed = true;
                    name = c.name.clone();
                    let _ = reader.get_ref().set_read_timeout(None);
                    shared.clients.lock().unwrap_or_else(|e| e.into_inner()).push(Client { conn, ..c });
                    shared.wake();
                    if !send(&mut out, &id, &Ok(hello_result())) {
                        return;
                    }
                }
                Ok(None) => {
                    send(&mut out, &id, &Err(RpcError::new(ERR_UNAUTHORIZED, "send hello with the token first")));
                    return;
                }
                Err(e) => {
                    send(&mut out, &id, &Err(e));
                    return;
                }
            }
            continue;
        }
        if method == "hello" {
            if !send(&mut out, &id, &Ok(hello_result())) {
                return;
            }
            continue;
        }
        if matches!(method.as_str(), "game.poll" | "game.frame" | "game.capture") {
            let reply = match method.as_str() {
                "game.poll" => Ok(game_poll(shared, conn, &name)),
                "game.frame" => game_frame(shared, &params),
                _ => game_capture(shared, &params),
            };
            if !send(&mut out, &id, &reply) {
                return;
            }
            continue;
        }
        let (reply_tx, reply_rx) = mpsc::channel();
        if tx.send(Request { method, params, reply: reply_tx }).is_err() {
            return;
        }
        shared.wake();
        let waited = std::time::Instant::now();
        let reply = loop {
            match reply_rx.recv_timeout(Duration::from_millis(250)) {
                Ok(r) => break r,
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {
                    if shared.stop.load(Ordering::SeqCst) {
                        return;
                    }
                    if waited.elapsed() > ANSWER_TIMEOUT {
                        break Err(RpcError::new(ERR_OTHER, "Pixmith did not answer in time"));
                    }
                }
            }
        };
        if !send(&mut out, &id, &reply) {
            return;
        }
    }
}

// MARK: Game view

/// `game.poll`: the caller is a running game; hands it the oldest capture request not yet delivered.
fn game_poll(shared: &Shared, conn: u64, name: &str) -> Value {
    let mut game = shared.game();
    match game.pollers.iter_mut().find(|p| p.conn == conn) {
        Some(p) => (p.at, p.name) = (Instant::now(), name.to_string()),
        None => game.pollers.push(Poller { conn, name: name.to_string(), at: Instant::now() }),
    }
    // Request ids count up, so the smallest number is the oldest.
    let next = game
        .slots
        .iter()
        .filter(|(_, s)| s.conn == conn && !s.delivered && s.frame.is_none())
        .min_by_key(|(id, _)| id.trim_start_matches("cap-").parse::<u64>().unwrap_or(u64::MAX))
        .map(|(id, _)| id.clone());
    if let Some(id) = &next {
        game.slots.get_mut(id).unwrap().delivered = true;
    }
    Value::Obj(vec![("capture".into(), next.map_or(Value::Null, Value::Str))])
}

/// `game.frame`: the game's answer to a capture. An unknown or already answered request is dropped.
fn game_frame(shared: &Shared, params: &Value) -> Reply {
    let bad = |what: &str| RpcError::new(ERR_PARAMS, what.to_string());
    let request = params.get("request").and_then(Value::as_str).ok_or_else(|| bad("\"request\" must be a string"))?;
    let png = params.get("png").and_then(Value::as_str).ok_or_else(|| bad("\"png\" must be a string"))?;
    let dim = |key: &str| match params.get(key) {
        Some(v @ Value::Num(n)) if *n >= 0.0 && n.fract() == 0.0 => Ok(v.clone()),
        _ => Err(bad(&format!("\"{key}\" must be a whole number"))),
    };
    let (w, h) = (dim("w")?, dim("h")?);
    let mut game = shared.game();
    if let Some(slot) = game.slots.get_mut(request)
        && slot.frame.is_none()
    {
        slot.frame = Some(Frame { png: png.to_string(), w, h });
        shared.game_changed.notify_all();
    }
    Ok(Value::Obj(Vec::new()))
}

/// `game.capture`: asks the game that polled last for a screenshot and waits for its `game.frame`.
fn game_capture(shared: &Shared, params: &Value) -> Reply {
    let ms = match params.get("timeout_ms") {
        None | Some(Value::Null) => CAPTURE_DEFAULT_MS,
        Some(Value::Num(n)) if *n >= 0.0 && n.fract() == 0.0 => (*n as u64).min(CAPTURE_MAX_MS),
        Some(_) => return Err(RpcError::new(ERR_PARAMS, "\"timeout_ms\" must be a whole number of 0 or more")),
    };
    let no_game = |why: &str| RpcError::new(ERR_NO_GAME, why.to_string());
    let deadline = Instant::now() + Duration::from_millis(ms);
    let mut game = shared.game();
    let (conn, name) = game
        .pollers
        .iter()
        .filter(|p| p.at.elapsed() <= GAME_ALIVE)
        .max_by_key(|p| p.at)
        .map(|p| (p.conn, p.name.clone()))
        .ok_or_else(|| {
            no_game("no running game: nothing is polling (is the Pixmith Live Link addon on in a debug build?)")
        })?;
    game.next_request += 1;
    let id = format!("cap-{}", game.next_request);
    game.slots.insert(id.clone(), Slot { conn, name, delivered: false, frame: None });
    let outcome = loop {
        let slot = game.slots.get_mut(&id).unwrap();
        if let Some(frame) = slot.frame.take() {
            break Ok(Value::Obj(vec![
                ("png".into(), Value::Str(frame.png)),
                ("w".into(), frame.w),
                ("h".into(), frame.h),
                ("client".into(), Value::Str(slot.name.clone())),
            ]));
        }
        if !game.pollers.iter().any(|p| p.conn == conn) {
            break Err(no_game("the game disconnected before it sent a frame"));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || shared.stop.load(Ordering::SeqCst) {
            break Err(no_game("the game did not send a frame in time"));
        }
        game = shared
            .game_changed
            .wait_timeout(game, left.min(Duration::from_millis(250)))
            .unwrap_or_else(|e| e.into_inner())
            .0;
    };
    game.slots.remove(&id);
    outcome
}

/// Checks a `hello`; the client it describes (the connection number is filled in by the caller).
fn hello(params: &Value, shared: &Shared) -> Result<Client, RpcError> {
    if shared.blocked.load(Ordering::SeqCst) {
        return Err(RpcError::new(ERR_UNAUTHORIZED, "disconnected by the user"));
    }
    if params.get("token").and_then(Value::as_str) != Some(shared.token.as_str()) {
        return Err(RpcError::new(ERR_UNAUTHORIZED, "wrong token"));
    }
    if let Some(p) = params.get("protocol").and_then(Value::as_f64)
        && p as u32 != PROTOCOL
    {
        return Err(RpcError::new(ERR_PARAMS, format!("protocol {p} is not supported; this app speaks {PROTOCOL}")));
    }
    let text = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or_default().chars().take(80).collect();
    Ok(Client { conn: 0, name: text("client"), version: text("client_version") })
}

fn hello_result() -> Value {
    Value::Obj(vec![
        ("app".into(), Value::Str("Pixmith".into())),
        ("version".into(), Value::Str(env!("CARGO_PKG_VERSION").into())),
        ("front_end".into(), Value::Str("egui".into())),
        ("platform".into(), Value::Str(platform().into())),
        ("protocol".into(), Value::Num(PROTOCOL as f64)),
    ])
}
