//! The core's script host as the front end sees it (`docs/LUA_DIALOG.md` section 4): sessions, polled JSON
//! messages, events, and applying a transaction's result to a document.
//!
//! This is the only file that talks to `pixmith_core::scripting::host`. Everything else goes through the
//! [`ScriptHost`] trait, so the renderer and the menus are tested with [`FakeHost`] and run unchanged on the real
//! core ([`CoreHost`]).

use crate::doc::Doc;
use pixmith_core::Editor;
use pixmith_core::formats::json::{self, Value};
use pixmith_core::scripting::host;
use std::collections::{BTreeMap, VecDeque};
use std::ffi::{CString, c_char};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The calls of `pixmith_script_ui.h`, with `doc` as a safe handle instead of a raw pointer.
///
/// `layer` and `frame` are 0-based; `tag` is any number that identifies the document again (the front end uses
/// the document pointer).
pub trait ScriptHost {
    /// False when no script host is linked in (nothing can be started).
    fn available(&self) -> bool;
    /// `pixmith_script_start`: returns the session id.
    fn start(
        &mut self,
        doc: Option<&Doc>,
        tag: u64,
        source: &str,
        name: &str,
        layer: u32,
        frame: u32,
    ) -> Result<u64, String>;
    /// `pixmith_plugin_load`: returns the session id.
    fn plugin_load(
        &mut self,
        source: &str,
        name: &str,
        dir: Option<&str>,
        prefs_path: Option<&str>,
    ) -> Result<u64, String>;
    /// `pixmith_script_poll(0)`: the messages queued for the sessions this host started, oldest first.
    fn poll(&mut self) -> Vec<Value>;
    /// `pixmith_script_send`.
    fn send(&mut self, session: u64, doc: Option<&Doc>, tag: u64, layer: u32, frame: u32, event_json: &str) -> bool;
    /// `pixmith_script_broadcast`.
    fn broadcast(&mut self, doc: Option<&Doc>, tag: u64, layer: u32, frame: u32, event_json: &str) -> u32;
    /// `pixmith_script_apply`: 1 applied, 0 nothing to apply, -1 conflict, -2 unknown transaction. `None`
    /// discards the result.
    fn apply(&mut self, session: u64, txn: u64, doc: Option<&mut Doc>) -> i32;
    /// `pixmith_script_set_colors`.
    fn set_colors(&mut self, fg: u32, bg: u32);
    /// `pixmith_script_stop`.
    fn stop(&mut self, session: u64);
    /// `pixmith_script_count`: live sessions.
    fn count(&self) -> u32;
}

/// Messages polled from the core's one global queue, sorted by session, and who may take them. The queue is
/// shared by the whole process, so every host takes only the sessions it started (tests run several apps at once);
/// the app's own host also adopts sessions nobody started through it (the extension loader calls the core itself).
#[derive(Default)]
struct Mailboxes {
    mail: BTreeMap<u64, Vec<Value>>,
    owner: BTreeMap<u64, u64>,
}

static MAILBOXES: Mutex<Mailboxes> = Mutex::new(Mailboxes { mail: BTreeMap::new(), owner: BTreeMap::new() });
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

fn lock() -> std::sync::MutexGuard<'static, Mailboxes> {
    MAILBOXES.lock().unwrap_or_else(|e| e.into_inner())
}

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

fn opt_cstr(s: Option<&str>) -> Option<CString> {
    s.map(cstr)
}

fn ptr_of(c: &Option<CString>) -> *const c_char {
    c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr())
}

fn doc_ptr(doc: Option<&Doc>) -> *mut Editor {
    doc.map_or(null_mut(), Doc::raw)
}

/// The real host: the Lua sessions of `pixmith-core` (`scripting::host`).
pub struct CoreHost {
    owner: u64,
    adopt: bool,
}

impl Default for CoreHost {
    /// The app's host: it also takes sessions started elsewhere in the process.
    fn default() -> CoreHost {
        CoreHost { owner: NEXT_OWNER.fetch_add(1, Ordering::Relaxed), adopt: true }
    }
}

impl CoreHost {
    /// A host that only sees the sessions it started (tests that run apps side by side).
    pub fn isolated() -> CoreHost {
        CoreHost { owner: NEXT_OWNER.fetch_add(1, Ordering::Relaxed), adopt: false }
    }

    fn started(id: u64) -> Result<u64, String> {
        if id == 0 { Err(crate::doc::last_error()) } else { Ok(id) }
    }
}

impl ScriptHost for CoreHost {
    fn available(&self) -> bool {
        true
    }

    fn start(
        &mut self,
        doc: Option<&Doc>,
        tag: u64,
        source: &str,
        name: &str,
        layer: u32,
        frame: u32,
    ) -> Result<u64, String> {
        let (source, name) = (cstr(source), cstr(name));
        // The claim is made while the mailboxes are locked, so no poll can hand the first messages to anyone else.
        let mut boxes = lock();
        // SAFETY: `doc` is live or null, the strings are NUL terminated and outlive the call.
        let id = unsafe { host::pixmith_script_start(doc_ptr(doc), tag, source.as_ptr(), name.as_ptr(), layer, frame) };
        if id != 0 {
            boxes.owner.insert(id, self.owner);
        }
        drop(boxes);
        CoreHost::started(id)
    }

    fn plugin_load(
        &mut self,
        source: &str,
        name: &str,
        dir: Option<&str>,
        prefs_path: Option<&str>,
    ) -> Result<u64, String> {
        let (source, name, dir, prefs) = (cstr(source), cstr(name), opt_cstr(dir), opt_cstr(prefs_path));
        let mut boxes = lock();
        // SAFETY: the strings are NUL terminated and outlive the call; the optional ones may be null.
        let id = unsafe { host::pixmith_plugin_load(source.as_ptr(), name.as_ptr(), ptr_of(&dir), ptr_of(&prefs)) };
        if id != 0 {
            boxes.owner.insert(id, self.owner);
        }
        drop(boxes);
        CoreHost::started(id)
    }

    fn poll(&mut self) -> Vec<Value> {
        let mut boxes = lock();
        let raw = host::pixmith_script_poll(0);
        // SAFETY: `raw` is null or a string the core allocated.
        let text = unsafe { crate::doc::take_string(raw) };
        if let Ok(Value::Arr(items)) = json::parse(&text) {
            for m in items {
                let session = m.get("session").and_then(Value::as_f64).map_or(0, |n| n as u64);
                boxes.mail.entry(session).or_default().push(m);
            }
        }
        let mine: Vec<u64> = boxes
            .mail
            .keys()
            .copied()
            .filter(|s| match boxes.owner.get(s) {
                Some(o) => *o == self.owner,
                None => self.adopt,
            })
            .collect();
        let mut out = Vec::new();
        for s in mine {
            if self.adopt {
                boxes.owner.insert(s, self.owner);
            }
            let msgs = boxes.mail.remove(&s).unwrap_or_default();
            if msgs.iter().any(|m| m.get("type").and_then(Value::as_str) == Some("session.end")) {
                boxes.owner.remove(&s);
            }
            out.extend(msgs);
        }
        out
    }

    fn send(&mut self, session: u64, doc: Option<&Doc>, tag: u64, layer: u32, frame: u32, event_json: &str) -> bool {
        let event = cstr(event_json);
        // SAFETY: `doc` is live or null; `event` is NUL terminated.
        unsafe { host::pixmith_script_send(session, doc_ptr(doc), tag, layer, frame, event.as_ptr()) }
    }

    fn broadcast(&mut self, doc: Option<&Doc>, tag: u64, layer: u32, frame: u32, event_json: &str) -> u32 {
        let event = cstr(event_json);
        // SAFETY: as for `send`.
        unsafe { host::pixmith_script_broadcast(doc_ptr(doc), tag, layer, frame, event.as_ptr()) }
    }

    fn apply(&mut self, session: u64, txn: u64, doc: Option<&mut Doc>) -> i32 {
        let ptr = doc.map_or(null_mut(), |d| d.raw());
        // SAFETY: `ptr` is a live document owned by the UI thread (this one) or null.
        unsafe { host::pixmith_script_apply(session, txn, ptr) }
    }

    fn set_colors(&mut self, fg: u32, bg: u32) {
        host::pixmith_script_set_colors(fg, bg);
    }

    fn stop(&mut self, session: u64) {
        host::pixmith_script_stop(session);
    }

    fn count(&self) -> u32 {
        host::pixmith_script_count()
    }
}

/// What a [`FakeHost`] was asked to do, and the messages it will hand out.
#[derive(Default)]
pub struct FakeState {
    /// Messages the next `poll` returns (tests push them).
    pub inbox: VecDeque<Value>,
    /// `(session, event JSON)` of every `send`.
    pub sent: Vec<(u64, String)>,
    /// The event JSON of every `broadcast`.
    pub broadcasts: Vec<String>,
    /// `(session, txn, had a document)` of every `apply`.
    pub applied: Vec<(u64, u64, bool)>,
    /// What `apply` returns.
    pub apply_result: i32,
    /// `(name, source)` of every started script or plugin.
    pub started: Vec<(String, String)>,
    pub stopped: Vec<u64>,
    pub colors: Option<(u32, u32)>,
    pub next_session: u64,
    pub live: u32,
}

/// A host for tests and screenshots: records the calls and replays the messages a test pushes.
#[derive(Clone, Default)]
pub struct FakeHost(pub Arc<Mutex<FakeState>>);

impl FakeHost {
    pub fn new() -> FakeHost {
        let host = FakeHost::default();
        host.0.lock().unwrap().next_session = 1;
        host
    }

    /// Queues a message for the next poll.
    pub fn push(&self, json: &str) {
        let value = pixmith_core::formats::json::parse(json).expect("test message is valid JSON");
        self.0.lock().unwrap().inbox.push_back(value);
    }

    /// The events sent so far, with the `type` field read back for easy assertions.
    pub fn sent(&self) -> Vec<(u64, String)> {
        self.0.lock().unwrap().sent.clone()
    }
}

impl ScriptHost for FakeHost {
    fn available(&self) -> bool {
        true
    }
    fn start(&mut self, _: Option<&Doc>, _: u64, source: &str, name: &str, _: u32, _: u32) -> Result<u64, String> {
        let mut s = self.0.lock().unwrap();
        s.started.push((name.to_string(), source.to_string()));
        let id = s.next_session.max(1);
        s.next_session = id + 1;
        s.live += 1;
        Ok(id)
    }
    fn plugin_load(&mut self, source: &str, name: &str, _: Option<&str>, _: Option<&str>) -> Result<u64, String> {
        self.start(None, 0, source, name, 0, 0)
    }
    fn poll(&mut self) -> Vec<Value> {
        self.0.lock().unwrap().inbox.drain(..).collect()
    }
    fn send(&mut self, session: u64, _: Option<&Doc>, _: u64, _: u32, _: u32, event_json: &str) -> bool {
        self.0.lock().unwrap().sent.push((session, event_json.to_string()));
        true
    }
    fn broadcast(&mut self, _: Option<&Doc>, _: u64, _: u32, _: u32, event_json: &str) -> u32 {
        self.0.lock().unwrap().broadcasts.push(event_json.to_string());
        1
    }
    fn apply(&mut self, session: u64, txn: u64, doc: Option<&mut Doc>) -> i32 {
        let mut s = self.0.lock().unwrap();
        s.applied.push((session, txn, doc.is_some()));
        if doc.is_none() { 0 } else { s.apply_result }
    }
    fn set_colors(&mut self, fg: u32, bg: u32) {
        self.0.lock().unwrap().colors = Some((fg, bg));
    }
    fn stop(&mut self, session: u64) {
        let mut s = self.0.lock().unwrap();
        s.stopped.push(session);
        s.live = s.live.saturating_sub(1);
    }
    fn count(&self) -> u32 {
        self.0.lock().unwrap().live
    }
}
