//! Pixmith Live (`docs/LIVE_PROTOCOL.md`): the server is started on a temporary app folder and a plain
//! `TcpStream` plays the client, while the test thread plays the UI thread by pumping the requests.

mod common;

use pixmith_core::formats::json::{self, Value};
use pixmith_desktop::app::PixmithApp;
use pixmith_desktop::doc::Doc;
use pixmith_desktop::live;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Rig {
    app: PixmithApp,
    dir: PathBuf,
}

impl Rig {
    fn new(name: &str) -> Rig {
        let dir = common::scratch(name);
        let mut app = PixmithApp::default();
        app.live.start_in(&dir.join("Pixmith"), None);
        assert!(app.live.server.is_some(), "{:?}", app.live.error);
        Rig { app, dir }
    }

    fn discovery(&self) -> Value {
        let path = self.app.live.server.as_ref().unwrap().discovery_path().to_path_buf();
        json::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn connect(&self) -> Client {
        let info = self.discovery();
        let port = info.get("port").unwrap().as_f64().unwrap() as u16;
        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(5))).unwrap();
        Client { stream, pending: Vec::new(), next: 1 }
    }

    /// A connection that already said hello.
    fn client(&mut self) -> Client {
        self.client_named("pixmith-mcp")
    }

    fn client_named(&mut self, name: &str) -> Client {
        let mut c = self.connect();
        let token = self.discovery().get("token").unwrap().as_str().unwrap().to_string();
        let r = c.call(
            &mut self.app,
            "hello",
            &json::to_string(&obj(&[
                ("token", Value::Str(token)),
                ("client", Value::Str(name.into())),
                ("client_version", Value::Str("9.9".into())),
                ("protocol", Value::Num(1.0)),
            ])),
        );
        assert!(r.get("result").is_some(), "{r:?}");
        c
    }

    /// Calls and returns `result`, failing the test on an error.
    fn ok(&mut self, c: &mut Client, method: &str, params: &str) -> Value {
        let r = c.call(&mut self.app, method, params);
        match r.get("result") {
            Some(v) => v.clone(),
            None => panic!("{method} failed: {r:?}"),
        }
    }

    /// Calls and returns the error code.
    fn fail(&mut self, c: &mut Client, method: &str, params: &str) -> i64 {
        let r = c.call(&mut self.app, method, params);
        match r.get("error") {
            Some(e) => e.get("code").unwrap().as_f64().unwrap() as i64,
            None => panic!("{method} should have failed: {r:?}"),
        }
    }
}

struct Client {
    stream: TcpStream,
    pending: Vec<u8>,
    next: i64,
}

impl Client {
    fn send(&mut self, line: &str) {
        self.stream.write_all(line.as_bytes()).unwrap();
        self.stream.write_all(b"\n").unwrap();
    }

    /// Reads one response line while pumping the app; `None` when the server closed the connection.
    fn read(&mut self, app: &mut PixmithApp) -> Option<Value> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            live::pump(app);
            if let Some(end) = self.pending.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=end).collect();
                return Some(json::parse(std::str::from_utf8(&line).unwrap()).unwrap());
            }
            assert!(Instant::now() < deadline, "no response");
            let mut buf = vec![0u8; 1 << 16];
            match self.stream.read(&mut buf) {
                Ok(0) => return None,
                Ok(n) => self.pending.extend_from_slice(&buf[..n]),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(_) => return None,
            }
        }
    }

    /// Like `call`, for methods the network layer answers itself: nothing pumps the app, so this can run on
    /// another thread (and block, as `game.capture` does).
    fn call_net(&mut self, method: &str, params: &Value) -> Value {
        let id = self.next;
        self.next += 1;
        let params = json::to_string(params);
        self.send(&format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#));
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(end) = self.pending.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=end).collect();
                let r = json::parse(std::str::from_utf8(&line).unwrap()).unwrap();
                assert_eq!(r.get("id").unwrap().as_f64(), Some(id as f64), "{r:?}");
                return r;
            }
            assert!(Instant::now() < deadline, "no response");
            let mut buf = vec![0u8; 1 << 16];
            match self.stream.read(&mut buf) {
                Ok(0) => panic!("connection closed"),
                Ok(n) => self.pending.extend_from_slice(&buf[..n]),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(e) => panic!("{e}"),
            }
        }
    }

    fn call(&mut self, app: &mut PixmithApp, method: &str, params: &str) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(&format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#));
        let r = self.read(app).expect("connection closed");
        assert_eq!(r.get("id").unwrap().as_f64(), Some(id as f64), "{r:?}");
        r
    }
}

fn obj(members: &[(&str, Value)]) -> Value {
    Value::Obj(members.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).unwrap_or_else(|| panic!("no {key} in {v:?}")).as_str().unwrap()
}

fn n(v: &Value, key: &str) -> i64 {
    v.get(key).unwrap_or_else(|| panic!("no {key} in {v:?}")).as_f64().unwrap() as i64
}

/// A document of `art()` drawn at 16 x 16, as .pxmt bytes in base64.
fn art_b64(seed: u32) -> String {
    art_b64_sized(seed, 16)
}

fn art_b64_sized(seed: u32, side: u32) -> String {
    let mut doc = common::art(seed, side, side);
    b64(&doc.encode().unwrap())
}

fn b64(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let v = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(A[(v >> 18) as usize & 63] as char);
        out.push(A[(v >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { A[(v >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { A[v as usize & 63] as char } else { '=' });
    }
    out
}

fn pixels(app: &PixmithApp, tab: usize) -> Vec<u8> {
    let mut out = Vec::new();
    assert!(app.tabs[tab].doc.render(0, &mut out));
    out
}

#[test]
fn discovery_file_describes_the_server_and_goes_away() {
    let rig = Rig::new("live-discovery");
    let path = rig.app.live.server.as_ref().unwrap().discovery_path().to_path_buf();
    assert_eq!(path.parent().unwrap().file_name().unwrap(), "Live");
    assert_eq!(path.file_name().unwrap().to_string_lossy(), format!("{}.json", std::process::id()));
    let info = rig.discovery();
    assert_eq!(n(&info, "protocol"), 1);
    assert_eq!(n(&info, "pid"), std::process::id() as i64);
    assert!(n(&info, "port") > 0);
    assert_eq!(s(&info, "token").len(), 32);
    assert!(s(&info, "token").bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    assert_eq!(s(&info, "app"), "Pixmith");
    assert_eq!(s(&info, "front_end"), "egui");
    assert!(["macos", "windows", "linux"].contains(&s(&info, "platform")));
    assert!(n(&info, "started") > 1_700_000_000);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let port = n(&info, "port") as u16;
    let Rig { app, .. } = rig;
    drop(app);
    assert!(!path.exists(), "the discovery file is removed with the server");
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err(), "the port is closed");
}

#[test]
fn stopping_the_server_removes_the_file_and_stale_files_are_cleaned() {
    let mut rig = Rig::new("live-stop");
    let live_dir = rig.dir.join("Pixmith").join("Live");
    let stale = live_dir.join("1.json");
    std::fs::write(&stale, r#"{"protocol":1,"pid":1,"port":1}"#).unwrap();
    let path = rig.app.live.server.as_ref().unwrap().discovery_path().to_path_buf();
    rig.app.live.stop();
    assert!(!path.exists());
    rig.app.live.start_in(&rig.dir.join("Pixmith"), None);
    assert!(!stale.exists(), "a file whose port refuses connections is removed");
    assert!(rig.app.live.server.as_ref().unwrap().discovery_path().exists());
}

#[test]
fn hello_is_required_and_the_token_must_match() {
    let mut rig = Rig::new("live-hello");
    // Anything before hello.
    let mut c = rig.connect();
    let r = c.call(&mut rig.app, "documents.list", "{}");
    assert_eq!(n(r.get("error").unwrap(), "code"), -32001);
    assert!(c.read(&mut rig.app).is_none(), "the connection is closed");
    // A wrong token.
    let mut c = rig.connect();
    let r = c.call(&mut rig.app, "hello", r#"{"token":"nope","client":"x","protocol":1}"#);
    assert_eq!(n(r.get("error").unwrap(), "code"), -32001);
    assert!(c.read(&mut rig.app).is_none());
    // Garbage before hello.
    let mut c = rig.connect();
    c.send("this is not json");
    let r = c.read(&mut rig.app).unwrap();
    assert_eq!(n(r.get("error").unwrap(), "code"), -32700);
    assert!(c.read(&mut rig.app).is_none());
    // The real thing.
    let token = rig.discovery().get("token").unwrap().as_str().unwrap().to_string();
    let mut c = rig.connect();
    let r = c.call(
        &mut rig.app,
        "hello",
        &format!(r#"{{"token":"{token}","client":"pixmith-mcp","client_version":"1.2","protocol":1}}"#),
    );
    let result = r.get("result").unwrap();
    assert_eq!(s(result, "app"), "Pixmith");
    assert_eq!(s(result, "front_end"), "egui");
    assert_eq!(n(result, "protocol"), 1);
    assert_eq!(s(r.get("result").unwrap(), "version"), env!("CARGO_PKG_VERSION"));
    let clients = rig.app.live.connected();
    assert_eq!((clients[0].name.as_str(), clients[0].version.as_str()), ("pixmith-mcp", "1.2"));
}

#[test]
fn protocol_errors() {
    let mut rig = Rig::new("live-errors");
    let mut c = rig.client();
    assert_eq!(rig.fail(&mut c, "no.such.method", "{}"), -32601);
    assert_eq!(rig.fail(&mut c, "documents.get", "{}"), -32602);
    assert_eq!(rig.fail(&mut c, "documents.get", r#"{"id":7}"#), -32602);
    assert_eq!(rig.fail(&mut c, "documents.get", r#"{"id":"doc-99"}"#), -32004);
    c.send("{oops");
    assert_eq!(n(c.read(&mut rig.app).unwrap().get("error").unwrap(), "code"), -32700);
    c.send(r#"[1,2]"#);
    assert_eq!(n(c.read(&mut rig.app).unwrap().get("error").unwrap(), "code"), -32600);
    c.send(r#"{"jsonrpc":"2.0","id":5}"#);
    assert_eq!(n(c.read(&mut rig.app).unwrap().get("error").unwrap(), "code"), -32600);
    // Notifications get no answer; the next request is answered normally.
    c.send(r#"{"jsonrpc":"2.0","method":"documents.list"}"#);
    rig.ok(&mut c, "documents.list", "{}");
    // Requests are answered in order, also when sent back to back.
    c.send(r#"{"jsonrpc":"2.0","id":"a","method":"documents.list","params":{}}"#);
    c.send(r#"{"jsonrpc":"2.0","id":"b","method":"view.get","params":{}}"#);
    assert_eq!(c.read(&mut rig.app).unwrap().get("id").unwrap().as_str(), Some("a"));
    assert_eq!(c.read(&mut rig.app).unwrap().get("id").unwrap().as_str(), Some("b"));
}

#[test]
fn a_message_over_64_mib_is_refused() {
    let mut rig = Rig::new("live-big");
    let mut c = rig.client();
    // 64 MiB and one byte without a newline.
    let chunk = vec![b'a'; 1 << 20];
    let mut sent = 0;
    while sent <= 64 << 20 {
        if c.stream.write_all(&chunk).is_err() {
            break;
        }
        sent += chunk.len();
    }
    let r = c.read(&mut rig.app);
    assert!(r.is_none_or(|r| n(r.get("error").unwrap(), "code") == -32600));
}

#[test]
fn list_get_and_view() {
    let mut rig = Rig::new("live-list");
    let mut c = rig.client();
    let list = rig.ok(&mut c, "documents.list", "{}");
    let docs = list.get("documents").unwrap().as_array().unwrap();
    assert_eq!(docs.len(), 1);
    let d = &docs[0];
    assert_eq!(s(d, "id"), "doc-1");
    assert_eq!(s(&list, "active"), "doc-1");
    assert_eq!((n(d, "width"), n(d, "height"), n(d, "frames"), n(d, "layers")), (32, 32, 1, 1));
    assert_eq!(n(d, "revision"), 1);
    assert_eq!(d.get("path"), Some(&Value::Null));
    assert_eq!(d.get("dirty"), Some(&Value::Bool(false)));
    assert_eq!(d.get("active"), Some(&Value::Bool(true)));
    assert_eq!((n(d, "layer"), n(d, "frame")), (0, 0));

    let got = rig.ok(&mut c, "documents.get", r#"{"id":"doc-1"}"#);
    let bytes = pixmith_core::formats::pxmt::read_document(&decode(s(&got, "pxmt"))).unwrap();
    assert_eq!((bytes.width, bytes.height), (32, 32));
    assert_eq!(n(got.get("document").unwrap(), "revision"), 1);

    let v = rig.ok(&mut c, "view.get", "{}");
    assert_eq!(s(&v, "document"), "doc-1");
    assert_eq!((n(&v, "layer"), n(&v, "frame")), (0, 0));
    assert_eq!(s(&v, "tool"), "pencil");
    assert!(v.get("zoom").unwrap().as_f64().unwrap() > 0.0);
}

fn decode(text: &str) -> Vec<u8> {
    const A: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0);
    for ch in text.chars().filter(|c| *c != '=') {
        acc = acc << 6 | A.find(ch).unwrap() as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

#[test]
fn apply_is_one_labelled_undo_step() {
    let mut rig = Rig::new("live-apply");
    // The tab's art is 32 x 32; the client sends a 16 x 16 drawing, which also changes the canvas size.
    let mut c = rig.client();
    let before = pixels(&rig.app, 0);
    let applied = rig.ok(
        &mut c,
        "documents.apply",
        &format!(r#"{{"id":"doc-1","base_revision":1,"pxmt":"{}","label":"Draw a face"}}"#, art_b64(2)),
    );
    let d = applied.get("document").unwrap();
    assert_eq!((n(d, "width"), n(d, "height")), (16, 16));
    assert_eq!(n(d, "revision"), 2);
    assert_eq!(d.get("dirty"), Some(&Value::Bool(true)));
    assert_eq!(rig.app.tabs[0].doc.history_counts(), (1, 0));
    // The history panel shows the label.
    let label = unsafe {
        let p = pixmith_core::history::pixmith_history_label(rig.app.tabs[0].doc.raw(), 0);
        let text = std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned();
        pixmith_core::ffi::pixmith_string_free(p);
        text
    };
    assert_eq!(label, "AI: Draw a face");
    // Undo restores the old document, redo brings the edit back, both are changes (the revision goes up).
    let edited = pixels(&rig.app, 0);
    assert_ne!(edited, before);
    assert!(rig.app.tabs[0].doc.undo());
    rig.app.tabs[0].changed();
    assert_eq!(pixels(&rig.app, 0), before);
    assert_eq!(rig.app.tabs[0].doc.width(), 32);
    let list = rig.ok(&mut c, "documents.list", "{}");
    assert_eq!(n(&list.get("documents").unwrap().as_array().unwrap()[0], "revision"), 3);
    assert!(rig.app.tabs[0].doc.redo());
    rig.app.tabs[0].changed();
    assert_eq!(pixels(&rig.app, 0), edited);
}

#[test]
fn stale_revision_bad_bytes_and_layer_frame() {
    let mut rig = Rig::new("live-stale");
    let mut c = rig.client();
    // Stale: the base revision is not the current one.
    let before = pixels(&rig.app, 0);
    let params = format!(r#"{{"id":"doc-1","base_revision":7,"pxmt":"{}","label":"x"}}"#, art_b64(1));
    assert_eq!(rig.fail(&mut c, "documents.apply", &params), -32009);
    assert_eq!(pixels(&rig.app, 0), before);
    assert_eq!(rig.app.tabs[0].doc.history_counts(), (0, 0));
    // The user draws: the revision goes up even though the client never sent anything.
    rig.app.tabs[0].doc.begin_edit("Pencil");
    rig.app.tabs[0].doc.stroke_end();
    rig.app.tabs[0].doc.touch();
    rig.app.tabs[0].changed();
    let rev = n(rig.ok(&mut c, "documents.get", r#"{"id":"doc-1"}"#).get("document").unwrap(), "revision");
    assert_eq!(rev, 2);
    assert_eq!(
        rig.fail(&mut c, "documents.apply", &format!(r#"{{"id":"doc-1","base_revision":1,"pxmt":"{}"}}"#, art_b64(1))),
        -32009
    );
    // Bad bytes: not base64, and base64 that is not a document.
    let undo_steps = rig.app.tabs[0].doc.history_counts();
    assert_eq!(rig.fail(&mut c, "documents.apply", r#"{"id":"doc-1","base_revision":2,"pxmt":"@@@"}"#), -32602);
    assert_eq!(rig.fail(&mut c, "documents.apply", r#"{"id":"doc-1","base_revision":2,"pxmt":"aGVsbG8="}"#), -32000);
    assert_eq!(rig.fail(&mut c, "documents.apply", r#"{"id":"doc-1","pxmt":"aGVsbG8="}"#), -32602);
    assert_eq!(rig.fail(&mut c, "documents.apply", r#"{"id":"doc-9","base_revision":2,"pxmt":"aGVsbG8="}"#), -32004);
    assert_eq!(rig.app.tabs[0].doc.history_counts(), undo_steps);
    assert_eq!(pixels(&rig.app, 0), before);
    // Layer and frame are clamped to the new document.
    let r = rig.ok(
        &mut c,
        "documents.apply",
        &format!(r#"{{"id":"doc-1","base_revision":2,"pxmt":"{}","label":"y","layer":99,"frame":-3}}"#, art_b64(3)),
    );
    let d = r.get("document").unwrap();
    assert_eq!((n(d, "layer"), n(d, "frame")), (0, 0));
    assert_eq!(n(d, "revision"), 3);
}

#[test]
fn back_to_back_new_documents_all_stay_open() {
    let mut rig = Rig::new("live-parallel-new");
    let mut c = rig.client();
    // Only the app's own blank tab is reused; the tab the first request made is not blank for the second.
    let first = rig.ok(&mut c, "documents.new", r#"{"width":8,"height":8,"name":"a"}"#);
    let second = rig.ok(&mut c, "documents.new", r#"{"width":10,"height":6,"name":"b"}"#);
    let third = rig.ok(&mut c, "documents.new", r#"{"width":4,"height":4}"#);
    let ids: Vec<&str> = [&first, &second, &third].iter().map(|r| s(r.get("document").unwrap(), "id")).collect();
    assert_ne!(ids[0], ids[1]);
    assert_ne!(ids[1], ids[2]);
    assert_ne!(ids[0], ids[2]);
    assert_eq!(rig.app.tabs.len(), 3);
    let list = rig.ok(&mut c, "documents.list", "{}");
    let docs = list.get("documents").unwrap().as_array().unwrap();
    assert_eq!(docs.len(), 3);
    assert_eq!((n(&docs[0], "width"), n(&docs[0], "height")), (8, 8));
    assert_eq!((n(&docs[1], "width"), n(&docs[1], "height")), (10, 6));
}

#[test]
fn new_open_save_activate_close() {
    let mut rig = Rig::new("live-files");
    let mut c = rig.client();
    // New: a blank untitled tab is reused, a second one gets its own tab.
    let first = rig.ok(&mut c, "documents.new", r#"{"width":12,"height":9,"name":"hero"}"#);
    let d1 = first.get("document").unwrap().clone();
    assert_eq!((n(&d1, "width"), n(&d1, "height")), (12, 9));
    assert_eq!(s(&d1, "name"), "hero");
    assert_eq!(d1.get("active"), Some(&Value::Bool(true)));
    assert_eq!(rig.app.tabs.len(), 1);
    // Make the first one non-pristine so the next `new` opens a tab.
    let id1 = s(&d1, "id").to_string();
    let applied = rig.ok(
        &mut c,
        "documents.apply",
        &format!(r#"{{"id":"{id1}","base_revision":{},"pxmt":"{}","label":"art"}}"#, n(&d1, "revision"), art_b64(4)),
    );
    assert_eq!(n(applied.get("document").unwrap(), "width"), 16);
    let second = rig.ok(&mut c, "documents.new", r#"{"width":8,"height":8}"#);
    let d2 = second.get("document").unwrap();
    assert_eq!(rig.app.tabs.len(), 2);
    assert_ne!(s(d2, "id"), id1);
    assert_eq!(rig.fail(&mut c, "documents.new", r#"{"width":0,"height":8}"#), -32602);
    assert_eq!(rig.fail(&mut c, "documents.new", r#"{"width":9999,"height":8}"#), -32602);
    let list = rig.ok(&mut c, "documents.list", "{}");
    assert_eq!(list.get("documents").unwrap().as_array().unwrap().len(), 2);
    assert_eq!(s(&list, "active"), s(d2, "id"));

    // Activate the first again.
    let a = rig.ok(&mut c, "documents.activate", &format!(r#"{{"id":"{id1}"}}"#));
    assert_eq!(a.get("document").unwrap().get("active"), Some(&Value::Bool(true)));
    assert_eq!(rig.app.active, 0);

    // Save: no file yet needs a path; the extension picks the format.
    assert_eq!(rig.fail(&mut c, "documents.save", &format!(r#"{{"id":"{id1}"}}"#)), -32602);
    let out = rig.dir.join("hero.pxmt");
    assert_eq!(rig.fail(&mut c, "documents.save", &format!(r#"{{"id":"{id1}","path":"relative.pxmt"}}"#)), -32602);
    assert_eq!(
        rig.fail(
            &mut c,
            "documents.save",
            &format!(
                r#"{{"id":"{id1}","path":{}}}"#,
                json::to_string(&Value::Str(rig.dir.join("x.png").display().to_string()))
            )
        ),
        -32602
    );
    let saved = rig.ok(
        &mut c,
        "documents.save",
        &format!(r#"{{"id":"{id1}","path":{}}}"#, json::to_string(&Value::Str(out.display().to_string()))),
    );
    let sd = saved.get("document").unwrap();
    assert_eq!(s(sd, "name"), "hero.pxmt");
    assert_eq!(s(sd, "path"), out.display().to_string());
    assert_eq!(sd.get("dirty"), Some(&Value::Bool(false)));
    assert!(pixmith_core::formats::pxmt::is_pxmt(&std::fs::read(&out).unwrap()));
    // Saving again without a path writes to its own file; an .aseprite path writes that format.
    rig.ok(&mut c, "documents.save", &format!(r#"{{"id":"{id1}"}}"#));
    let ase = rig.dir.join("hero.aseprite");
    rig.ok(
        &mut c,
        "documents.save",
        &format!(r#"{{"id":"{id1}","path":{}}}"#, json::to_string(&Value::Str(ase.display().to_string()))),
    );
    assert!(!pixmith_core::formats::pxmt::is_pxmt(&std::fs::read(&ase).unwrap()));

    // Open: a file, and the tab that already has it.
    let other = rig.dir.join("other.pxmt");
    common::art(5, 20, 10).save(&other).unwrap();
    let opened = rig.ok(
        &mut c,
        "documents.open",
        &format!(r#"{{"path":{}}}"#, json::to_string(&Value::Str(other.display().to_string()))),
    );
    let od = opened.get("document").unwrap();
    assert_eq!((n(od, "width"), n(od, "height")), (20, 10));
    assert_eq!(s(od, "name"), "other.pxmt");
    let tabs = rig.app.tabs.len();
    let again = rig.ok(
        &mut c,
        "documents.open",
        &format!(r#"{{"path":{}}}"#, json::to_string(&Value::Str(other.display().to_string()))),
    );
    assert_eq!(s(again.get("document").unwrap(), "id"), s(od, "id"));
    assert_eq!(rig.app.tabs.len(), tabs);
    assert_eq!(
        rig.fail(
            &mut c,
            "documents.open",
            &format!(
                r#"{{"path":{}}}"#,
                json::to_string(&Value::Str(rig.dir.join("missing.pxmt").display().to_string()))
            )
        ),
        -32000
    );
    assert_eq!(
        rig.fail(
            &mut c,
            "documents.open",
            &format!(r#"{{"path":{}}}"#, json::to_string(&Value::Str(rig.dir.display().to_string())))
        ),
        -32602
    );

    // Close: unsaved changes need `discard`.
    let oid = s(od, "id").to_string();
    let dirty = format!(
        r#"{{"id":"{}","base_revision":{},"pxmt":"{}","label":"z"}}"#,
        oid,
        n(rig.ok(&mut c, "documents.get", &format!(r#"{{"id":"{oid}"}}"#)).get("document").unwrap(), "revision"),
        art_b64(6)
    );
    rig.ok(&mut c, "documents.apply", &dirty);
    assert_eq!(rig.fail(&mut c, "documents.close", &format!(r#"{{"id":"{oid}"}}"#)), -32011);
    assert_eq!(rig.fail(&mut c, "documents.close", &format!(r#"{{"id":"{oid}","discard":false}}"#)), -32011);
    let before = rig.app.tabs.len();
    rig.ok(&mut c, "documents.close", &format!(r#"{{"id":"{oid}","discard":true}}"#));
    assert_eq!(rig.app.tabs.len(), before - 1);
    assert_eq!(rig.fail(&mut c, "documents.get", &format!(r#"{{"id":"{oid}"}}"#)), -32004);
    // A saved document closes without discard.
    rig.ok(&mut c, "documents.close", &format!(r#"{{"id":"{id1}"}}"#));
    assert!(!Path::new(&out).metadata().unwrap().is_dir());
}

#[test]
fn view_set_moves_the_current_layer_and_frame() {
    let mut rig = Rig::new("live-view");
    let mut c = rig.client();
    // Two layers and two frames, made through the core like the UI does.
    {
        let doc = &mut rig.app.tabs[0].doc;
        doc.add_layer(0);
        doc.add_frame(0, false);
        rig.app.tabs[0].changed();
    }
    assert_eq!((rig.app.tabs[0].doc.layer_count(), rig.app.tabs[0].doc.frame_count()), (2, 2));
    let v = rig.ok(&mut c, "view.set", r#"{"layer":0,"frame":1}"#);
    assert_eq!((n(&v, "layer"), n(&v, "frame")), (0, 1));
    assert_eq!((rig.app.tabs[0].layer, rig.app.tabs[0].frame), (0, 1));
    let v = rig.ok(&mut c, "view.set", r#"{"id":"doc-1","layer":50}"#);
    assert_eq!((n(&v, "layer"), n(&v, "frame")), (1, 1));
    assert_eq!(s(&v, "document"), "doc-1");
    assert_eq!(rig.fail(&mut c, "view.set", r#"{"id":"doc-4","layer":1}"#), -32004);
    assert_eq!(rig.fail(&mut c, "view.set", r#"{"layer":"one"}"#), -32602);
    let got = rig.ok(&mut c, "view.get", "{}");
    assert_eq!((n(&got, "layer"), n(&got, "frame")), (1, 1));
}

#[test]
fn busy_while_a_sheet_is_open_but_reads_work() {
    let mut rig = Rig::new("live-busy");
    let mut c = rig.client();
    rig.app.request_new_sprite();
    assert!(rig.app.files.new_sheet.is_some());
    rig.ok(&mut c, "documents.list", "{}");
    rig.ok(&mut c, "documents.get", r#"{"id":"doc-1"}"#);
    let params = format!(r#"{{"id":"doc-1","base_revision":1,"pxmt":"{}","label":"x"}}"#, art_b64(1));
    assert_eq!(rig.fail(&mut c, "documents.apply", &params), -32010);
    assert_eq!(rig.fail(&mut c, "documents.new", r#"{"width":4,"height":4}"#), -32010);
    assert_eq!(rig.fail(&mut c, "documents.close", r#"{"id":"doc-1","discard":true}"#), -32010);
    assert_eq!(rig.fail(&mut c, "documents.get", r#"{"id":"nope"}"#), -32004);
    assert_eq!(rig.app.tabs[0].doc.history_counts(), (0, 0));
    rig.app.files.new_sheet = None;
    rig.ok(&mut c, "documents.apply", &params);
}

/// `{"id": id}` for a request, with the id escaped like any JSON string.
fn id_params(id: &str) -> String {
    json::to_string(&Value::Obj(vec![("id".to_string(), Value::Str(id.to_string()))]))
}

#[test]
fn godot_atlas_lays_out_the_frames_and_works_while_busy() {
    let mut rig = Rig::new("live-godot-atlas");
    let mut c = rig.client();
    rig.app.tabs[0].doc.add_frame(0, false);
    rig.app.tabs[0].changed();
    let params = id_params("doc-1");
    let r = rig.ok(&mut c, "godot.atlas", &params);
    assert_eq!(s(r.get("document").unwrap(), "id"), "doc-1");
    // Never exported to Godot: unlinked, laid out with the default options.
    assert_eq!(r.get("linked"), Some(&Value::Bool(false)));
    let res = &r.get("resources").unwrap().as_array().unwrap()[0];
    assert_eq!(res.get("resource"), Some(&Value::Null));
    let page = &res.get("pages").unwrap().as_array().unwrap()[0];
    assert!(n(page, "w") > 0 && n(page, "h") > 0);
    assert_eq!(&decode(s(page, "png"))[1..4], b"PNG");
    let anims = res.get("animations").unwrap().as_array().unwrap();
    let frames = anims[0].get("frames").unwrap().as_array().unwrap();
    assert_eq!(frames.len(), 2);
    for f in frames {
        let region = f.get("region").unwrap().as_array().unwrap();
        assert_eq!(region.len(), 4);
        assert!(region.iter().all(|v| v.as_f64().is_some()));
    }

    // A read: answered while a sheet is open; an unknown document is reported either way.
    rig.app.request_new_sprite();
    assert!(rig.app.files.new_sheet.is_some());
    rig.ok(&mut c, "godot.atlas", &params);
    assert_eq!(rig.fail(&mut c, "godot.atlas", &id_params("nope")), -32004);
    assert_eq!(rig.fail(&mut c, "godot.atlas", "{}"), -32602);
}

#[test]
fn disconnect_closes_connections_and_the_port_stays_open() {
    let mut rig = Rig::new("live-disconnect");
    let mut c = rig.client();
    rig.ok(&mut c, "documents.list", "{}");
    assert_eq!(rig.app.live.connected().len(), 1);
    rig.app.live.server.as_ref().unwrap().disconnect_all();
    assert!(c.read(&mut rig.app).is_none());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !rig.app.live.connected().is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    // A client may connect again.
    let mut again = rig.client();
    rig.ok(&mut again, "documents.list", "{}");
}

#[test]
fn drawing_through_the_ui_changes_the_revision() {
    let mut rig = Rig::new("live-revision");
    let mut c = rig.client();
    let rev = |rig: &mut Rig, c: &mut Client| {
        let l = rig.ok(c, "documents.list", "{}");
        n(&l.get("documents").unwrap().as_array().unwrap()[0], "revision")
    };
    let r0 = rev(&mut rig, &mut c);
    assert_eq!(rev(&mut rig, &mut c), r0, "reading changes nothing");
    // An edit that only the core knows about (no `changed()` call) is still noticed through the history.
    rig.app.tabs[0].doc.begin_edit("Pencil");
    rig.app.tabs[0].doc.stroke_end();
    let r1 = rev(&mut rig, &mut c);
    assert!(r1 > r0);
    assert!(rig.app.tabs[0].doc.undo());
    let r2 = rev(&mut rig, &mut c);
    assert!(r2 > r1, "undo is a change too");
    let _ = Doc::new(1, 1);
}

#[test]
fn undo_and_redo_walk_the_history_the_user_sees() {
    let mut rig = Rig::new("live-undo");
    let mut c = rig.client();
    let blank = pixels(&rig.app, 0);
    for (i, label) in ["one", "two", "three"].iter().enumerate() {
        let rev = n(rig.ok(&mut c, "documents.get", r#"{"id":"doc-1"}"#).get("document").unwrap(), "revision");
        rig.ok(
            &mut c,
            "documents.apply",
            &format!(r#"{{"id":"doc-1","base_revision":{rev},"pxmt":"{}","label":"{label}"}}"#, art_b64(i as u32)),
        );
    }
    assert_eq!(rig.app.tabs[0].doc.history_counts(), (3, 0));
    let rev = |rig: &mut Rig, c: &mut Client| {
        n(rig.ok(c, "documents.get", r#"{"id":"doc-1"}"#).get("document").unwrap(), "revision")
    };
    let r0 = rev(&mut rig, &mut c);
    let r = rig.ok(&mut c, "documents.undo", r#"{"id":"doc-1"}"#);
    assert_eq!(n(&r, "steps"), 1);
    assert_eq!(rig.app.tabs[0].doc.history_counts(), (2, 1));
    assert!(n(r.get("document").unwrap(), "revision") > r0, "undo is a change");
    let r = rig.ok(&mut c, "documents.undo", r#"{"id":"doc-1","steps":10}"#);
    assert_eq!(n(&r, "steps"), 2, "only as many as there are");
    assert_eq!(pixels(&rig.app, 0), blank);
    let r = rig.ok(&mut c, "documents.undo", r#"{"id":"doc-1"}"#);
    assert_eq!(n(&r, "steps"), 0);
    let r = rig.ok(&mut c, "documents.redo", r#"{"id":"doc-1","steps":2}"#);
    assert_eq!(n(&r, "steps"), 2);
    assert_eq!(rig.app.tabs[0].doc.history_counts(), (2, 1));
    assert_eq!(n(&rig.ok(&mut c, "documents.redo", r#"{"id":"doc-1","steps":5}"#), "steps"), 1);
    assert_eq!(rig.fail(&mut c, "documents.undo", r#"{"id":"doc-1","steps":-1}"#), -32602);
    assert_eq!(rig.fail(&mut c, "documents.undo", r#"{"id":"doc-7"}"#), -32004);
    assert_eq!(rig.fail(&mut c, "documents.undo", "{}"), -32602);
}

#[test]
fn selection_comes_with_get_and_goes_back_with_apply() {
    let mut rig = Rig::new("live-selection");
    let mut c = rig.client();
    let got = rig.ok(&mut c, "documents.get", r#"{"id":"doc-1"}"#);
    assert_eq!(got.get("selection"), Some(&Value::Null));
    pixmith_core::paint::selection::select_rect(unsafe { &mut *rig.app.tabs[0].doc.raw() }, (3, 2), (5, 3));
    rig.app.tabs[0].changed();
    let got = rig.ok(&mut c, "documents.get", r#"{"id":"doc-1"}"#);
    let sel = got.get("selection").unwrap();
    assert_eq!((n(sel, "x"), n(sel, "y"), n(sel, "w"), n(sel, "h")), (3, 2, 3, 2));
    assert_eq!(decode(s(sel, "mask")), vec![1u8; 6]);
    let rev = n(got.get("document").unwrap(), "revision");
    // Absent keeps it; an object replaces it; null clears it. Each is one undo step.
    let apply = |rig: &mut Rig, c: &mut Client, rev: i64, extra: &str| {
        rig.ok(
            c,
            "documents.apply",
            &format!(
                r#"{{"id":"doc-1","base_revision":{rev},"pxmt":"{}","label":"sel"{extra}}}"#,
                art_b64_sized(1, 32)
            ),
        )
    };
    let r = apply(&mut rig, &mut c, rev, "");
    let rev = n(r.get("document").unwrap(), "revision");
    assert_eq!(n(rig.ok(&mut c, "documents.get", r#"{"id":"doc-1"}"#).get("selection").unwrap(), "w"), 3);
    let mask = b64(&[1, 0, 1, 1]);
    let r = apply(&mut rig, &mut c, rev, &format!(r#","selection":{{"x":1,"y":1,"w":2,"h":2,"mask":"{mask}"}}"#));
    let rev = n(r.get("document").unwrap(), "revision");
    let sel = rig.ok(&mut c, "documents.get", r#"{"id":"doc-1"}"#).get("selection").unwrap().clone();
    assert_eq!((n(&sel, "x"), n(&sel, "y"), n(&sel, "w"), n(&sel, "h")), (1, 1, 2, 2));
    assert_eq!(decode(s(&sel, "mask")), vec![1u8, 0, 1, 1]);
    assert_eq!(rig.app.tabs[0].doc.history_counts().0, 3);
    // A bad mask is refused and changes nothing.
    let params = format!(
        r#"{{"id":"doc-1","base_revision":{rev},"pxmt":"{}","selection":{{"x":0,"y":0,"w":2,"h":2,"mask":"AQ=="}}}}"#,
        art_b64_sized(1, 32)
    );
    assert_eq!(rig.fail(&mut c, "documents.apply", &params), -32602);
    assert_eq!(rig.app.tabs[0].doc.history_counts().0, 3);
    let r = apply(&mut rig, &mut c, rev, r#","selection":null"#);
    assert_eq!(rig.ok(&mut c, "documents.get", r#"{"id":"doc-1"}"#).get("selection"), Some(&Value::Null));
    let _ = r;
    // Undo brings the selection of the step before back.
    rig.ok(&mut c, "documents.undo", r#"{"id":"doc-1"}"#);
    let sel = rig.ok(&mut c, "documents.get", r#"{"id":"doc-1"}"#).get("selection").unwrap().clone();
    assert_eq!((n(&sel, "w"), n(&sel, "h")), (2, 2));
}

#[test]
fn the_users_disconnect_blocks_hello_until_live_mode_is_restarted() {
    let mut rig = Rig::new("live-user-disconnect");
    let mut c = rig.client();
    rig.ok(&mut c, "documents.list", "{}");
    rig.app.live.server.as_ref().unwrap().disconnect_by_user();
    assert!(c.read(&mut rig.app).is_none());
    // The client tries again with the right token.
    let token = rig.discovery().get("token").unwrap().as_str().unwrap().to_string();
    let mut again = rig.connect();
    let r = again.call(&mut rig.app, "hello", &format!(r#"{{"token":"{token}","client":"pixmith-mcp","protocol":1}}"#));
    let e = r.get("error").unwrap();
    assert_eq!(n(e, "code"), -32001);
    assert_eq!(s(e, "message"), "disconnected by the user");
    assert!(again.read(&mut rig.app).is_none());
    // Switching Live mode off and on again lets clients in.
    rig.app.live.stop();
    rig.app.live.start_in(&rig.dir.join("Pixmith"), None);
    let mut fresh = rig.client();
    rig.ok(&mut fresh, "documents.list", "{}");
}

// MARK: Through the real UI loop

mod ui_loop {
    use super::*;
    use eframe::egui;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use pixmith_desktop::theme;

    fn harness(app: PixmithApp) -> Harness<'static, PixmithApp> {
        let mut installed = false;
        let mut h = Harness::builder().with_size([1280.0, 800.0]).build_ui_state(
            move |ui, app: &mut PixmithApp| {
                if !installed {
                    theme::install(ui.ctx());
                    theme::apply(ui.ctx(), app.dark);
                    installed = true;
                    ui.ctx().request_discard("fonts installed");
                    return;
                }
                let full = ui.ctx().content_rect();
                let mut root = ui.new_child(egui::UiBuilder::new().max_rect(full));
                root.set_clip_rect(full);
                app.ui(&mut root);
            },
            app,
        );
        h.run();
        h
    }

    /// Reads one response while the harness runs frames.
    fn answer(h: &mut Harness<'static, PixmithApp>, c: &mut Client) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            h.step();
            if let Some(end) = c.pending.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = c.pending.drain(..=end).collect();
                return json::parse(std::str::from_utf8(&line).unwrap()).unwrap();
            }
            assert!(Instant::now() < deadline, "no response");
            let mut buf = vec![0u8; 1 << 16];
            if let Ok(n) = c.stream.read(&mut buf) {
                c.pending.extend_from_slice(&buf[..n]);
            }
        }
    }

    #[test]
    fn requests_are_answered_between_frames_and_the_chip_shows() {
        let rig = Rig::new("live-ui");
        let Rig { app, dir } = rig;
        let info_path = app.live.server.as_ref().unwrap().discovery_path().to_path_buf();
        let info = json::parse(&std::fs::read_to_string(&info_path).unwrap()).unwrap();
        let token = s(&info, "token").to_string();
        // Connect only once the (slow to build) window exists: a connection has 10 s to say hello.
        let mut h = harness(app);
        let stream = TcpStream::connect(("127.0.0.1", n(&info, "port") as u16)).unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(5))).unwrap();
        let mut c = Client { stream, pending: Vec::new(), next: 1 };
        assert!(h.query_by_label("AI connected").is_none());
        c.send(&format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"hello","params":{{"token":"{token}","client":"pixmith-mcp","client_version":"1","protocol":1}}}}"#
        ));
        assert!(answer(&mut h, &mut c).get("result").is_some());
        h.run();
        assert!(h.query_by_label("AI connected").is_some(), "the chip shows while a client is connected");
        // A request is served by a frame of the real UI.
        c.send(r#"{"jsonrpc":"2.0","id":2,"method":"documents.list","params":{}}"#);
        let r = answer(&mut h, &mut c);
        assert_eq!(s(&r.get("result").unwrap().get("documents").unwrap().as_array().unwrap()[0], "id"), "doc-1");
        let before = h.state().tabs[0].doc.history_counts();
        c.send(&format!(
            r#"{{"jsonrpc":"2.0","id":3,"method":"documents.apply","params":{{"id":"doc-1","base_revision":1,"pxmt":"{}","label":"Face"}}}}"#,
            art_b64(2)
        ));
        let r = answer(&mut h, &mut c);
        assert!(r.get("result").is_some(), "{r:?}");
        assert_eq!(h.state().tabs[0].doc.history_counts().0, before.0 + 1);
        // Disconnect drops the connection and the chip.
        h.state().live.server.as_ref().unwrap().disconnect_all();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !h.state().live.connected().is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        h.run();
        assert!(h.query_by_label("AI connected").is_none());
        let _ = dir;
    }

    #[test]
    fn the_preference_switch_stops_the_server() {
        let rig = Rig::new("live-switch");
        let Rig { mut app, dir } = rig;
        let path = app.live.server.as_ref().unwrap().discovery_path().to_path_buf();
        app.live.managed = true;
        app.menus.prefs.live_enabled = false;
        let mut h = harness(app);
        h.run();
        assert!(h.state().live.server.is_none());
        assert!(!path.exists());
        let _ = dir;
    }

    #[test]
    fn preferences_have_the_switch_on_by_default() {
        let mut app = PixmithApp::default();
        app.menus.prefs_open = true;
        let mut h = harness(app);
        h.run();
        assert!(h.query_by_label("Allow AI agents to control Pixmith").is_some());
        assert!(pixmith_desktop::ui::menus::Prefs::default().live_enabled);
        let encoded = pixmith_desktop::ui::menus::Prefs { live_enabled: false, ..Default::default() }.encode();
        assert!(!pixmith_desktop::ui::menus::Prefs::decode(&encoded).live_enabled);
    }
}

// MARK: Game view

/// A 1x1 PNG.
const TINY_PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";

fn error_code(r: &Value) -> i64 {
    r.get("error").unwrap_or_else(|| panic!("should have failed: {r:?}")).get("code").unwrap().as_f64().unwrap() as i64
}

fn frame_params(request: &str) -> Value {
    obj(&[
        ("request", Value::Str(request.into())),
        ("png", Value::Str(TINY_PNG.into())),
        ("w", Value::Num(1.0)),
        ("h", Value::Num(1.0)),
    ])
}

#[test]
fn game_capture_relays_the_frame_from_the_polling_game() {
    let mut rig = Rig::new("live-game-capture");
    let mut game = rig.client_named("pixmith-godot");
    let mut mcp = rig.client();
    // Nothing asked yet.
    assert_eq!(game.call_net("game.poll", &obj(&[])).get("result").unwrap().get("capture"), Some(&Value::Null));
    let asker = std::thread::spawn(move || mcp.call_net("game.capture", &obj(&[("timeout_ms", Value::Num(8000.0))])));
    let deadline = Instant::now() + Duration::from_secs(10);
    let request = loop {
        let r = game.call_net("game.poll", &obj(&[]));
        if let Some(id) = r.get("result").unwrap().get("capture").and_then(Value::as_str) {
            break id.to_string();
        }
        assert!(Instant::now() < deadline, "the capture was never offered to the game");
        std::thread::sleep(Duration::from_millis(20));
    };
    // Offered once.
    assert_eq!(game.call_net("game.poll", &obj(&[])).get("result").unwrap().get("capture"), Some(&Value::Null));
    let ok = game.call_net("game.frame", &frame_params(&request));
    assert_eq!(ok.get("result"), Some(&Value::Obj(Vec::new())), "{ok:?}");
    let got = asker.join().unwrap();
    let result = got.get("result").unwrap_or_else(|| panic!("{got:?}"));
    assert_eq!(s(result, "client"), "pixmith-godot");
    assert_eq!(s(result, "png"), TINY_PNG);
    assert_eq!((n(result, "w"), n(result, "h")), (1, 1));
    // A duplicate and an unknown request are dropped with {}.
    for request in [request.as_str(), "cap-999", ""] {
        let r = game.call_net("game.frame", &frame_params(request));
        assert_eq!(r.get("result"), Some(&Value::Obj(Vec::new())), "{r:?}");
    }
    // Bad params are still reported.
    let r = game.call_net("game.frame", &obj(&[("request", Value::Str("cap-1".into()))]));
    assert_eq!(error_code(&r), -32602);
}

#[test]
fn game_capture_without_a_game_fails_fast() {
    let mut rig = Rig::new("live-game-none");
    let mut c = rig.client();
    let started = Instant::now();
    assert_eq!(error_code(&c.call_net("game.capture", &obj(&[]))), -32012);
    assert!(started.elapsed() < Duration::from_secs(1));
    // A game that polled more than a second ago is gone.
    let mut game = rig.client_named("pixmith-godot");
    game.call_net("game.poll", &obj(&[]));
    std::thread::sleep(Duration::from_millis(1150));
    assert_eq!(error_code(&c.call_net("game.capture", &obj(&[]))), -32012);
    // A bad timeout is a parameter error.
    assert_eq!(error_code(&c.call_net("game.capture", &obj(&[("timeout_ms", Value::Str("x".into()))]))), -32602);
    // Like every method, the game ones need hello first.
    let mut raw = rig.connect();
    assert_eq!(error_code(&raw.call_net("game.poll", &obj(&[]))), -32001);
}

#[test]
fn game_capture_times_out_when_the_game_never_answers() {
    let mut rig = Rig::new("live-game-timeout");
    let mut game = rig.client_named("pixmith-godot");
    let mut c = rig.client();
    game.call_net("game.poll", &obj(&[]));
    let started = Instant::now();
    let r = c.call_net("game.capture", &obj(&[("timeout_ms", Value::Num(300.0))]));
    assert_eq!(error_code(&r), -32012);
    assert!(started.elapsed() >= Duration::from_millis(280) && started.elapsed() < Duration::from_secs(5));
    // The late frame is stale now.
    let request = game.call_net("game.poll", &obj(&[]));
    assert_eq!(request.get("result").unwrap().get("capture"), Some(&Value::Null), "{request:?}");
    let r = game.call_net("game.frame", &frame_params("cap-1"));
    assert_eq!(r.get("result"), Some(&Value::Obj(Vec::new())), "{r:?}");
}

#[test]
fn game_capture_fails_when_the_game_disconnects() {
    let mut rig = Rig::new("live-game-gone");
    let mut game = rig.client_named("pixmith-godot");
    let mut c = rig.client();
    game.call_net("game.poll", &obj(&[]));
    let asker = std::thread::spawn(move || c.call_net("game.capture", &obj(&[("timeout_ms", Value::Num(8000.0))])));
    std::thread::sleep(Duration::from_millis(200));
    drop(game);
    let started = Instant::now();
    assert_eq!(error_code(&asker.join().unwrap()), -32012);
    assert!(started.elapsed() < Duration::from_secs(5));
}
