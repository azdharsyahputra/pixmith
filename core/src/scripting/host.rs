//! Script sessions: Lua states that live on their own thread and talk to a front end through messages
//! (docs/LUA_DIALOG.md). This file holds the session runtime, the host that owns all sessions, and the
//! C ABI (`pixmith_script_start`, `pixmith_script_poll`, ...).
//!
//! A session never touches a `PixmithDoc`. Every call that can run Lua carries a snapshot of the sprite
//! and selection; the worker runs the code against it and, when it changed, parks the result as a
//! *commit* that the front end applies on its own thread with `pixmith_script_apply`.

use super::dialog::{self, Dlg, Reaction};
use super::events::{event_bit, EventsH, Listener};
use super::plugins::{self, Command, Prefs};
use super::{clean_error, conform_changed_cels, new_sandbox, Ctx, Limits, SpriteH, MARKER};
use crate::document::{Editor, Sprite};
use crate::formats::json::{self, Value as J};
use crate::paint::selection::Selection;
use mlua::chunk::ChunkMode;
use mlua::{AnyUserData, Error, Function, IntoLuaMulti, Lua, Result as LuaResult, Table, Value, VmState};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Sessions that may run at once.
pub const MAX_SESSIONS: usize = 64;
/// Results kept for the front end to apply (all sessions together); older ones are dropped.
const MAX_COMMITS: usize = 64;
/// Messages kept for a front end that does not poll.
const MAX_QUEUED: usize = 50_000;
const WORKER_STACK: usize = 16 << 20;

// MARK: Messages and commits

/// A copy of the sprite and selection plus where it came from, handed to a session with an event.
pub struct DocSnap {
    sprite: Sprite,
    mask: Option<Arc<Selection>>,
    tag: u64,
    layer: usize,
    frame: usize,
}

impl DocSnap {
    pub fn of(e: &Editor, tag: u64, layer: u32, frame: u32) -> DocSnap {
        DocSnap {
            sprite: e.sprite.clone(),
            mask: e.sel.mask.clone(),
            tag,
            layer: layer as usize,
            frame: frame as usize,
        }
    }
}

enum Cmd {
    Event(J, Option<DocSnap>),
    Stop,
}

struct Commit {
    base: Sprite,
    base_mask: Option<Arc<Selection>>,
    sprite: Sprite,
    mask: Option<Arc<Selection>>,
    label: String,
}

pub(super) struct Shared {
    id: u64,
    tx: Sender<Cmd>,
    cancel: Arc<AtomicBool>,
    pub(super) listens: AtomicU32,
}

struct Inner {
    next_id: AtomicU64,
    sessions: Mutex<Vec<Arc<Shared>>>,
    queue: Mutex<VecDeque<String>>,
    cv: Condvar,
    /// Finished transactions waiting for the front end to apply them, by (session, txn). They outlive the
    /// session: its `txn` and `session.end` messages usually arrive in the same poll.
    commits: Mutex<VecDeque<((u64, u64), Commit)>>,
    /// `fg << 32 | bg`
    colors: AtomicU64,
    /// Budgets of one transaction.
    limits: Limits,
}

impl Inner {
    fn push(&self, msg: J) {
        let text = json::to_string(&msg);
        let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        if q.len() >= MAX_QUEUED {
            q.pop_front();
        }
        q.push_back(text);
        self.cv.notify_all();
    }

    fn session(&self, id: u64) -> Option<Arc<Shared>> {
        self.sessions.lock().unwrap_or_else(|p| p.into_inner()).iter().find(|s| s.id == id).cloned()
    }
}

fn obj(ty: &str, session: u64, fields: Vec<(&str, J)>) -> J {
    let mut o = vec![("type".to_string(), J::Str(ty.into())), ("session".to_string(), J::Num(session as f64))];
    o.extend(fields.into_iter().map(|(k, v)| (k.to_string(), v)));
    J::Obj(o)
}

fn jstr<'a>(v: &'a J, key: &str) -> Option<&'a str> {
    v.get(key).and_then(J::as_str)
}

// MARK: Host

/// All running sessions and their message queue. The C ABI uses one global host; tests make their own.
#[derive(Clone)]
pub struct Host(Arc<Inner>);

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}

/// What a session runs.
pub enum Spec {
    Script { source: String, name: String },
    Plugin { source: String, name: String, dir: Option<String>, prefs_path: Option<String> },
}

impl Host {
    pub fn new() -> Host {
        Host::with_limits(Limits::default())
    }

    /// A host whose sessions use these per-transaction budgets (tests; the C ABI uses the defaults).
    pub fn with_limits(limits: Limits) -> Host {
        Host(Arc::new(Inner {
            limits,
            next_id: AtomicU64::new(1),
            sessions: Mutex::new(Vec::new()),
            queue: Mutex::new(VecDeque::new()),
            cv: Condvar::new(),
            commits: Mutex::new(VecDeque::new()),
            colors: AtomicU64::new(0x000000FF_FFFFFFFF),
        }))
    }

    pub fn set_colors(&self, fg: u32, bg: u32) {
        self.0.colors.store((fg as u64) << 32 | bg as u64, Ordering::Relaxed);
    }

    /// Starts a session; `doc` is the snapshot the main chunk (or `init`) sees.
    pub fn start(&self, spec: Spec, doc: Option<DocSnap>) -> Result<u64, String> {
        let (tx, rx) = channel();
        let shared;
        {
            let mut list = self.0.sessions.lock().unwrap_or_else(|p| p.into_inner());
            if list.len() >= MAX_SESSIONS {
                return Err(format!("at most {MAX_SESSIONS} scripts can run at once"));
            }
            let id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
            shared = Arc::new(Shared { id, tx, cancel: Arc::new(AtomicBool::new(false)), listens: AtomicU32::new(0) });
            list.push(shared.clone());
        }
        let (inner, sh) = (self.0.clone(), shared.clone());
        let spawned = std::thread::Builder::new()
            .name(format!("pixmith-script-{}", shared.id))
            .stack_size(WORKER_STACK)
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| worker(&inner, &sh, rx, spec, doc)));
                if let Err(p) = outcome {
                    let text = p
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| p.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".into());
                    inner.push(obj(
                        "txn",
                        sh.id,
                        vec![
                            ("txn", J::Num(0.0)),
                            ("doc", J::Num(0.0)),
                            ("ok", J::Bool(false)),
                            ("error", J::Str(format!("internal error: {text}"))),
                            ("messages", J::Arr(vec![])),
                            ("commit", J::Bool(false)),
                        ],
                    ));
                }
                inner.sessions.lock().unwrap_or_else(|p| p.into_inner()).retain(|s| s.id != sh.id);
                inner.push(obj("session.end", sh.id, vec![]));
            });
        match spawned {
            Ok(_) => Ok(shared.id),
            Err(e) => {
                self.0.sessions.lock().unwrap_or_else(|p| p.into_inner()).retain(|s| s.id != shared.id);
                Err(format!("could not start the script thread: {e}"))
            }
        }
    }

    /// All queued messages as a JSON array, waiting up to `timeout` for the first one.
    pub fn poll(&self, timeout: Duration) -> String {
        let mut q = self.0.queue.lock().unwrap_or_else(|p| p.into_inner());
        if q.is_empty() && !timeout.is_zero() {
            q = self.0.cv.wait_timeout_while(q, timeout, |q| q.is_empty()).unwrap_or_else(|p| p.into_inner()).0;
        }
        let mut out = String::from("[");
        for (i, m) in q.drain(..).enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&m);
        }
        out.push(']');
        out
    }

    /// Delivers one event; false when the session is gone.
    pub fn send(&self, session: u64, event: J, doc: Option<DocSnap>) -> bool {
        match self.0.session(session) {
            Some(s) => s.tx.send(Cmd::Event(event, doc)).is_ok(),
            None => false,
        }
    }

    /// Delivers an `app.event` / `sprite.event` to every session listening for it; `doc` is snapshotted per
    /// receiving session. Returns how many sessions received it.
    pub fn broadcast(&self, event: &J, doc: Option<&Editor>, tag: u64, layer: u32, frame: u32) -> u32 {
        let scope = match jstr(event, "type") {
            Some("app.event") => "app",
            Some("sprite.event") => "sprite",
            _ => return 0,
        };
        let Some(bit) = jstr(event, "name").and_then(|n| event_bit(scope, n)) else { return 0 };
        let list: Vec<Arc<Shared>> = self.0.sessions.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let mut n = 0;
        for s in list.iter().filter(|s| s.listens.load(Ordering::Relaxed) & bit != 0) {
            let snap = doc.map(|e| DocSnap::of(e, tag, layer, frame));
            if s.tx.send(Cmd::Event(event.clone(), snap)).is_ok() {
                n += 1;
            }
        }
        n
    }

    /// Applies a finished transaction to `doc` as one undo step: 1 applied, 0 nothing, -1 conflict, -2 unknown.
    pub fn apply(&self, session: u64, txn: u64, doc: Option<&mut Editor>) -> i32 {
        let commit = {
            let mut c = self.0.commits.lock().unwrap_or_else(|p| p.into_inner());
            match c.iter().position(|(k, _)| *k == (session, txn)) {
                Some(i) => c.remove(i).map(|x| x.1),
                None => None,
            }
        };
        let Some(commit) = commit else { return -2 };
        let Some(e) = doc else { return 0 };
        if e.sprite != commit.base || e.sel.mask != commit.base_mask {
            crate::ffi::set_error("The document changed while the script was running; its result was discarded.");
            return -1;
        }
        e.checkpoint();
        e.sprite = commit.sprite;
        e.sel.mask = commit.mask;
        e.label_last(&commit.label);
        e.dirty.mark_all();
        1
    }

    /// Interrupts a session; it runs `exit`, closes its dialogs and ends.
    pub fn stop(&self, session: u64) {
        if let Some(s) = self.0.session(session) {
            s.cancel.store(true, Ordering::Relaxed);
            let _ = s.tx.send(Cmd::Stop);
        }
    }

    pub fn count(&self) -> u32 {
        self.0.sessions.lock().unwrap_or_else(|p| p.into_inner()).len() as u32
    }
}

// MARK: Runtime inside the worker

pub(super) struct Budget {
    executed: Cell<u64>,
    started: Cell<Instant>,
    waited: Cell<Duration>,
    /// While `exit(plugin)` runs, a cancel request no longer interrupts Lua.
    exiting: Cell<bool>,
}

impl Budget {
    fn reset(&self) {
        self.executed.set(0);
        self.started.set(Instant::now());
        self.waited.set(Duration::ZERO);
    }
}

/// The worker side of a session. Lives in an `Rc` shared by the Lua closures of that session.
pub(super) struct Rt {
    host: Arc<Inner>,
    pub(super) shared: Arc<Shared>,
    rx: Receiver<Cmd>,
    pending: RefCell<VecDeque<Cmd>>,
    ctx: RefCell<Weak<Ctx>>,
    dialogs: RefCell<Vec<Dlg>>,
    dialog_seq: Cell<u32>,
    pub(super) listeners: RefCell<Vec<Listener>>,
    pub(super) listener_seq: Cell<u64>,
    pub(super) commands: RefCell<Vec<Command>>,
    state: Cell<&'static str>,
    txn_seq: Cell<u64>,
    in_txn: Cell<bool>,
    pub(super) cur_tag: Cell<u64>,
    budget: Rc<Budget>,
    plugin: bool,
    stopping: Cell<bool>,
    pub(super) prefs: RefCell<Option<Prefs>>,
}

pub(super) struct Report {
    pub(super) fatal: bool,
    pub(super) ok: bool,
}

impl Rt {
    pub fn ui_available(&self) -> bool {
        true
    }

    fn id(&self) -> u64 {
        self.shared.id
    }

    pub(super) fn ctx(&self) -> Rc<Ctx> {
        self.ctx.borrow().upgrade().expect("script context outlives its runtime")
    }

    pub(super) fn emit(&self, ty: &str, fields: Vec<(&str, J)>) {
        self.host.push(obj(ty, self.id(), fields));
    }

    pub(super) fn set_state(&self, s: &'static str) {
        if self.state.get() != s {
            self.state.set(s);
            self.emit("state", vec![("state", J::Str(s.into()))]);
        }
    }

    pub fn colors(&self) -> (u32, u32) {
        let c = self.host.colors.load(Ordering::Relaxed);
        ((c >> 32) as u32, c as u32)
    }

    pub fn emit_app_set(&self, fg: Option<u32>, bg: Option<u32>) {
        let mut f = Vec::new();
        if let Some(c) = fg {
            f.push(("fgColor", J::Num(c as f64)));
            let cur = self.host.colors.load(Ordering::Relaxed);
            self.host.colors.store((c as u64) << 32 | (cur & 0xFFFF_FFFF), Ordering::Relaxed);
        }
        if let Some(c) = bg {
            f.push(("bgColor", J::Num(c as f64)));
            let cur = self.host.colors.load(Ordering::Relaxed);
            self.host.colors.store(cur & !0xFFFF_FFFF | c as u64, Ordering::Relaxed);
        }
        self.emit("app.set", f);
    }

    pub fn next_dialog_id(&self) -> String {
        self.dialog_seq.set(self.dialog_seq.get() + 1);
        format!("s{}.d{}", self.id(), self.dialog_seq.get())
    }

    pub fn emit_dialog_modify(&self, dialog: &str, patches: Vec<(String, Vec<(String, J)>)>, title: Option<&str>) {
        let widgets: Vec<(String, J)> =
            patches.into_iter().filter(|(_, p)| !p.is_empty()).map(|(id, p)| (id, J::Obj(p))).collect();
        let mut f = vec![("dialog", J::Str(dialog.into())), ("widgets", J::Obj(widgets))];
        if let Some(t) = title {
            f.push(("title", J::Str(t.into())));
        }
        self.emit("dialog.modify", f);
    }

    pub fn open_dialog(&self, d: &Dlg) {
        self.dialogs.borrow_mut().push(d.clone());
        self.emit("dialog.open", vec![("dialog", d.borrow().to_json())]);
    }

    /// Closes a dialog (from the script, or because the user closed the window) and runs `onclose`.
    pub fn close_dialog(&self, lua: &Lua, d: &Dlg, tell_front_end: bool) -> LuaResult<()> {
        let (was_open, id, onclose) = {
            let mut st = d.borrow_mut();
            let was = st.open;
            st.open = false;
            (was, st.id.clone(), st.onclose.clone())
        };
        if !was_open {
            return Ok(());
        }
        self.dialogs.borrow_mut().retain(|x| !Rc::ptr_eq(x, d));
        if tell_front_end {
            self.emit("dialog.close", vec![("dialog", J::Str(id))]);
        }
        if let Some(f) = onclose {
            self.run_callback(lua, &f, ())?;
        }
        Ok(())
    }

    /// Calls a Lua callback; when it fails, what it did to the sprite and selection is undone.
    pub(super) fn run_callback(&self, _lua: &Lua, f: &Function, args: impl IntoLuaMulti) -> LuaResult<()> {
        let ctx = self.ctx();
        let (sprite, mask) = (ctx.sprite().clone(), ctx.mask.borrow().clone());
        let r = f.call::<()>(args);
        if r.is_err() {
            *ctx.sprite() = sprite;
            *ctx.mask.borrow_mut() = mask;
            ctx.invalidate_handles();
        }
        r
    }

    // MARK: Commands from the front end

    /// The next command: a queued one, or wait up to `timeout` for one. Bursts of `change` events of one
    /// widget collapse into the last.
    fn next_cmd(&self, timeout: Duration) -> Option<Cmd> {
        if self.pending.borrow().is_empty() {
            let first = match self.rx.recv_timeout(timeout) {
                Ok(c) => c,
                Err(RecvTimeoutError::Timeout) => return None,
                Err(RecvTimeoutError::Disconnected) => Cmd::Stop,
            };
            let mut p = self.pending.borrow_mut();
            p.push_back(first);
            while let Ok(c) = self.rx.try_recv() {
                let merged = match (p.back_mut(), &c) {
                    (Some(Cmd::Event(old, _)), Cmd::Event(new, _)) if is_change(old) && same_widget(old, new) => {
                        *old = new.clone();
                        true
                    }
                    _ => false,
                };
                if !merged {
                    p.push_back(c);
                }
            }
        }
        self.pending.borrow_mut().pop_front()
    }

    /// Waits inside a modal `show` until `cond` turns false, serving events on the way.
    pub fn pump(&self, lua: &Lua, cond: impl Fn() -> bool) -> LuaResult<()> {
        self.set_state("waiting");
        let r = (|| {
            while cond() {
                if self.shared.cancel.load(Ordering::Relaxed) {
                    return Err(cancelled());
                }
                let t = Instant::now();
                let cmd = self.next_cmd(Duration::from_millis(50));
                self.budget.waited.set(self.budget.waited.get() + t.elapsed());
                match cmd {
                    Some(Cmd::Event(ev, _)) => {
                        // Modal: the transaction already has its snapshot, a new one is ignored.
                        if let Err(e) = self.dispatch(lua, &ev) {
                            if is_fatal(&e) {
                                return Err(e);
                            }
                            self.ctx().messages.borrow_mut().push(format!("error: {}", clean_error(&e)));
                        }
                    }
                    Some(Cmd::Stop) => {
                        self.stopping.set(true);
                        return Err(cancelled());
                    }
                    None => {}
                }
            }
            Ok(())
        })();
        self.set_state("running");
        r
    }

    fn dispatch(&self, lua: &Lua, ev: &J) -> LuaResult<()> {
        match jstr(ev, "type") {
            Some("widget") => self.on_widget(lua, ev),
            Some("dialog.closed") => {
                let id = jstr(ev, "dialog").unwrap_or("");
                let d = self.dialogs.borrow().iter().find(|d| d.borrow().id == id).cloned();
                match d {
                    Some(d) => self.close_dialog(lua, &d, false),
                    None => Ok(()),
                }
            }
            Some("command") => self.on_command(lua, ev),
            Some("app.event") | Some("sprite.event") => self.on_app_event(lua, ev),
            _ => Ok(()),
        }
    }

    fn on_widget(&self, lua: &Lua, ev: &J) -> LuaResult<()> {
        let (Some(dialog), Some(id), Some(event)) = (jstr(ev, "dialog"), jstr(ev, "id"), jstr(ev, "event")) else {
            return Ok(());
        };
        let Some(d) = self.dialogs.borrow().iter().find(|d| d.borrow().id == dialog).cloned() else { return Ok(()) };
        let color = ev.get("color").and_then(J::as_f64).filter(|c| (0.0..=4294967295.0).contains(c)).map(|c| c as u32);
        let (reaction, patches, value) = {
            let mut st = d.borrow_mut();
            dialog::apply_widget_event(&mut st, id, event, ev.get("value"), color)
        };
        if !patches.is_empty() {
            self.emit_dialog_modify(dialog, patches, None);
        }
        match reaction {
            Reaction::None => Ok(()),
            Reaction::Close => self.close_dialog(lua, &d, true),
            Reaction::Callback(f, extra) => {
                let t = lua.create_table()?;
                t.set("id", id)?;
                let mut v = value.unwrap_or(Value::Nil);
                if matches!(v, Value::Nil) {
                    v = dialog::text_value(&d.borrow(), id, lua);
                }
                t.set("value", v)?;
                if let Some((k, v)) = extra {
                    t.set(k, v)?;
                }
                self.run_callback(lua, &f, t)
            }
        }
    }

    fn alive(&self) -> bool {
        self.plugin || self.has_listeners() || self.dialogs.borrow().iter().any(|d| d.borrow().open)
    }

    // MARK: Transactions

    /// Runs `body` as one transaction against `doc`, then reports it to the front end.
    pub(super) fn run_txn(&self, lua: &Lua, doc: Option<DocSnap>, body: impl FnOnce() -> LuaResult<()>) -> Report {
        let ctx = self.ctx();
        let ed = ctx.editor();
        let (has_doc, tag) = (doc.is_some(), doc.as_ref().map_or(0, |d| d.tag));
        let base_mask;
        match doc {
            Some(d) => {
                ed.sprite = d.sprite;
                base_mask = d.mask;
                ctx.layer.set(d.layer.min(ed.sprite.layers.len() - 1));
                ctx.frame.set(d.frame.min(ed.sprite.frame_count() - 1));
            }
            None => {
                ed.sprite = Sprite::new(1, 1);
                base_mask = None;
                ctx.layer.set(0);
                ctx.frame.set(0);
            }
        }
        let before = ed.sprite.clone();
        let (w, h) = (ed.sprite.width, ed.sprite.height);
        let old_mask = base_mask.clone().filter(|m| m.w == w && m.h == h);
        *ctx.mask.borrow_mut() = old_mask.clone();
        ctx.has_doc.set(has_doc);
        ctx.messages.borrow_mut().clear();
        *ctx.label.borrow_mut() = None;
        ctx.selected.set(false);
        ctx.invalidate_handles();
        self.budget.reset();
        self.cur_tag.set(tag);
        self.txn_seq.set(self.txn_seq.get() + 1);
        let txn = self.txn_seq.get();
        let sprite_value = if has_doc {
            lua.create_userdata(SpriteH(ctx.clone())).map(Value::UserData).unwrap_or(Value::Nil)
        } else {
            Value::Nil
        };
        let _ = lua.globals().set("sprite", sprite_value.clone());
        if let Ok(app) = lua.globals().get::<Table>("app") {
            let _ = app.raw_set("sprite", sprite_value);
        }
        self.in_txn.set(true);
        let result = catch_unwind(AssertUnwindSafe(|| {
            body()?;
            self.refresh_commands();
            Ok::<(), Error>(())
        }));
        self.in_txn.set(false);
        if matches!(result, Ok(Ok(()))) {
            self.save_prefs();
        }

        let messages: Vec<J> = ctx.messages.borrow_mut().drain(..).map(J::Str).collect();
        let label = ctx.label.borrow().clone();
        let new_mask = ctx.mask.borrow().clone();
        let select = (ctx.selected.get() && has_doc).then(|| {
            let s = &ed.sprite;
            let mut l = ctx.layer.get().min(s.layers.len() - 1);
            if s.layers[l].is_group() {
                l = s.layers.iter().rposition(|x| !x.is_group()).unwrap_or(0);
            }
            (l, ctx.frame.get().min(s.frame_count() - 1))
        });
        let mut fatal = false;
        let (ok, error, commit) = match result {
            Ok(Ok(())) => {
                let mut changed = false;
                if has_doc {
                    conform_changed_cels(&mut ed.sprite, &before);
                    ed.sprite.flush_links();
                    changed = ed.sprite != before || new_mask != old_mask;
                }
                if changed {
                    let commit = Commit {
                        base: before.clone(),
                        base_mask: base_mask.clone(),
                        sprite: ed.sprite.clone(),
                        mask: new_mask,
                        label: label.clone().unwrap_or_else(|| "Script".into()),
                    };
                    let mut c = self.host.commits.lock().unwrap_or_else(|p| p.into_inner());
                    c.push_back(((self.id(), txn), commit));
                    while c.len() > MAX_COMMITS {
                        c.pop_front();
                    }
                }
                (true, None, changed)
            }
            Ok(Err(e)) => (false, Some(clean_error(&e)), false),
            Err(p) => {
                fatal = true;
                let text = p
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| p.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".into());
                (false, Some(format!("internal error: {text}")), false)
            }
        };
        ed.sprite = before;
        let mut fields = vec![
            ("txn", J::Num(txn as f64)),
            ("doc", J::Num(tag as f64)),
            ("ok", J::Bool(ok)),
            ("error", error.clone().map_or(J::Null, J::Str)),
            ("messages", J::Arr(messages)),
            ("commit", J::Bool(commit)),
        ];
        if commit {
            fields.push(("label", J::Str(label.unwrap_or_else(|| "Script".into()))));
        }
        if let (Some((l, f)), true) = (select, ok) {
            fields.push((
                "select",
                J::Obj(vec![("layer".into(), J::Num((l + 1) as f64)), ("frame".into(), J::Num((f + 1) as f64))]),
            ));
        }
        self.emit("txn", fields);
        Report { fatal: fatal || (error.is_some() && self.stopping.get()), ok }
    }
}

fn cancelled() -> Error {
    Error::runtime("script cancelled")
}

/// Errors that end a transaction even inside a modal dialog (limits, cancel) instead of being reported.
pub(super) fn is_fatal(e: &Error) -> bool {
    matches!(e, Error::MemoryError(_)) || {
        let s = e.to_string();
        s.contains("script cancelled") || s.contains("script stopped:")
    }
}

fn is_change(ev: &J) -> bool {
    jstr(ev, "type") == Some("widget") && jstr(ev, "event") == Some("change")
}

fn same_widget(a: &J, b: &J) -> bool {
    is_change(b) && jstr(a, "dialog") == jstr(b, "dialog") && jstr(a, "id") == jstr(b, "id")
}

/// Adds `Dialog`, `app.events`, `app.isUIAvailable` and the table form of `app.alert` to a sandbox.
pub(super) fn install(lua: &Lua, ctx: &Rc<Ctx>) -> LuaResult<()> {
    let g = lua.globals();
    let c = ctx.clone();
    g.set("Dialog", lua.create_function(move |lua, arg: Value| dialog::create(lua, &c, arg))?)?;
    let app: Table = g.get("app")?;
    app.set("events", lua.create_userdata(EventsH { ctx: ctx.clone(), scope: "app" })?)?;
    app.set("isUIAvailable", ctx.rt.borrow().as_ref().is_some_and(|r| r.ui_available()))?;
    app.set("apiVersion", 3)?;
    lua.load(
        r#"
        local alert0 = app.alert
        function app.alert(a)
          if type(a) ~= "table" then return alert0(a) end
          local dlg = Dialog{ title = a.title or "" }
          local text = a.text
          if type(text) ~= "table" then text = { text or "" } end
          for _, line in ipairs(text) do dlg:label{ text = tostring(line) }:newrow() end
          local buttons = a.buttons or { "OK" }
          if type(buttons) ~= "table" then buttons = { buttons } end
          local pressed = 0
          for i, b in ipairs(buttons) do
            dlg:button{ id = "alert" .. i, text = tostring(b), focus = (i == 1),
                        onclick = function() pressed = i; dlg:close() end }
          end
          dlg:show{ wait = true }
          return pressed
        end
        "#,
    )
    .set_name("=sandbox")
    .exec()
}

// MARK: Worker

fn worker(inner: &Arc<Inner>, shared: &Arc<Shared>, rx: Receiver<Cmd>, spec: Spec, first: Option<DocSnap>) {
    let mut editor = Box::new(Editor::new(Sprite::new(1, 1)));
    let (source, name, plugin, dir, prefs_path) = match spec {
        Spec::Script { source, name } => (source, name, false, None, None),
        Spec::Plugin { source, name, dir, prefs_path } => (source, name, true, dir, prefs_path),
    };
    let ctx = Rc::new(Ctx::new(&mut editor));
    let limits = inner.limits;
    let budget = Rc::new(Budget {
        executed: Cell::new(0),
        started: Cell::new(Instant::now()),
        waited: Cell::new(Duration::ZERO),
        exiting: Cell::new(false),
    });
    let rt = Rc::new(Rt {
        host: inner.clone(),
        shared: shared.clone(),
        rx,
        pending: RefCell::new(VecDeque::new()),
        ctx: RefCell::new(Rc::downgrade(&ctx)),
        dialogs: RefCell::new(Vec::new()),
        dialog_seq: Cell::new(0),
        listeners: RefCell::new(Vec::new()),
        listener_seq: Cell::new(0),
        commands: RefCell::new(Vec::new()),
        state: Cell::new("none"),
        txn_seq: Cell::new(0),
        in_txn: Cell::new(false),
        cur_tag: Cell::new(0),
        budget: budget.clone(),
        plugin,
        stopping: Cell::new(false),
        prefs: RefCell::new(None),
    });
    *ctx.rt.borrow_mut() = Some(rt.clone());
    rt.set_state("running");

    let lua = match new_sandbox(&ctx, limits.memory) {
        Ok(l) => l,
        Err(e) => {
            end_with_error(&rt, &format!("could not start Lua: {e}"));
            return;
        }
    };
    {
        let cancel = shared.cancel.clone();
        let b = budget.clone();
        const STEP: u32 = 1000;
        let hooked = lua.set_hook(mlua::HookTriggers::new().every_nth_instruction(STEP), move |_, _| {
            if !b.exiting.get() && cancel.load(Ordering::Relaxed) {
                return Err(cancelled());
            }
            b.executed.set(b.executed.get() + STEP as u64);
            if b.executed.get() > limits.instructions {
                return Err(Error::runtime("script stopped: instruction limit exceeded"));
            }
            if b.started.get().elapsed().saturating_sub(b.waited.get()) > limits.time {
                return Err(Error::runtime("script stopped: time limit exceeded"));
            }
            Ok(VmState::Continue)
        });
        if let Err(e) = hooked {
            end_with_error(&rt, &format!("could not start Lua: {e}"));
            return;
        }
    }

    // The plugin object (preferences are loaded before `init`).
    let mut plugin_obj: Option<AnyUserData> = None;
    if plugin {
        let made = plugins::create(&rt, &lua, &ctx, &name, dir.as_deref(), prefs_path.as_deref());
        match made {
            Ok(p) => plugin_obj = Some(p),
            Err(e) => {
                end_with_error(&rt, &format!("could not start the plugin: {e}"));
                return;
            }
        }
    }

    let report = {
        let plugin_obj = plugin_obj.clone();
        rt.run_txn(&lua, first, || {
            lua.load(source.as_str()).set_name(format!("={name}")).set_mode(ChunkMode::Text).exec()?;
            if let Some(p) = &plugin_obj {
                plugins::call_init(&lua, p)?;
            }
            Ok(())
        })
    };
    // A script whose main chunk failed ends (what it registered is dropped); a plugin stays loaded.
    let mut keep_going = !report.fatal && (plugin || report.ok);
    while keep_going && rt.alive() {
        rt.update_listens();
        rt.set_state("idle");
        let cmd = loop {
            if let Some(c) = rt.next_cmd(Duration::from_millis(250)) {
                break c;
            }
        };
        match cmd {
            Cmd::Stop => break,
            Cmd::Event(ev, doc) => {
                rt.set_state("running");
                let r = rt.run_txn(&lua, doc, || rt.dispatch(&lua, &ev));
                keep_going = !r.fatal;
            }
        }
    }

    // Unload: `exit(plugin)`, preferences, then close what is still open.
    budget.exiting.set(true);
    if plugin {
        if let Some(p) = &plugin_obj {
            plugins::unload(&rt, &lua, p);
        }
        rt.save_prefs();
    }
    for d in rt.dialogs.borrow_mut().drain(..) {
        d.borrow_mut().open = false;
        rt.emit("dialog.close", vec![("dialog", J::Str(d.borrow().id.clone()))]);
    }
    plugins::drop_commands(&rt);
    rt.set_state("ended");
    // Break the reference cycle Lua -> closures -> Ctx -> Rt before the state is dropped.
    *ctx.rt.borrow_mut() = None;
    drop(lua);
}

fn end_with_error(rt: &Rc<Rt>, error: &str) {
    rt.emit(
        "txn",
        vec![
            ("txn", J::Num(0.0)),
            ("doc", J::Num(0.0)),
            ("ok", J::Bool(false)),
            ("error", J::Str(error.into())),
            ("messages", J::Arr(vec![])),
            ("commit", J::Bool(false)),
        ],
    );
    *rt.ctx().rt.borrow_mut() = None;
}

// MARK: C ABI

fn global() -> &'static Host {
    static HOST: OnceLock<Host> = OnceLock::new();
    HOST.get_or_init(Host::new)
}

unsafe fn text_arg(p: *const c_char) -> Option<String> {
    (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().replace(MARKER, ""))
}

/// Starts a script session (docs/LUA_DIALOG.md). Returns its id, 0 on failure.
#[no_mangle]
pub unsafe extern "C" fn pixmith_script_start(
    doc: *mut Editor,
    doc_tag: u64,
    source: *const c_char,
    name: *const c_char,
    layer: u32,
    frame: u32,
) -> u64 {
    crate::ffi::guarded("pixmith_script_start", std::ptr::null_mut(), || {
        let Some(source) = text_arg(source) else { return 0 };
        let name = text_arg(name).unwrap_or_else(|| "script".into());
        let snap = crate::ffi::editor_quiet(doc).map(|e| DocSnap::of(e, doc_tag, layer, frame));
        match global().start(Spec::Script { source, name }, snap) {
            Ok(id) => id,
            Err(e) => {
                crate::ffi::set_error(e);
                0
            }
        }
    })
}

/// Loads a plugin: runs the source, then its global `init(plugin)`. Returns the session id, 0 on failure.
#[no_mangle]
pub unsafe extern "C" fn pixmith_plugin_load(
    source: *const c_char,
    name: *const c_char,
    dir: *const c_char,
    prefs_path: *const c_char,
) -> u64 {
    crate::ffi::guarded("pixmith_plugin_load", std::ptr::null_mut(), || {
        let Some(source) = text_arg(source) else { return 0 };
        let name = text_arg(name).unwrap_or_else(|| "plugin".into());
        match global().start(Spec::Plugin { source, name, dir: text_arg(dir), prefs_path: text_arg(prefs_path) }, None)
        {
            Ok(id) => id,
            Err(e) => {
                crate::ffi::set_error(e);
                0
            }
        }
    })
}

/// Queued messages of all sessions as a JSON array; free with `pixmith_string_free`.
#[no_mangle]
pub extern "C" fn pixmith_script_poll(timeout_ms: u32) -> *mut c_char {
    crate::ffi::guarded("pixmith_script_poll", std::ptr::null_mut(), || {
        let text = global().poll(Duration::from_millis(timeout_ms.min(10_000) as u64));
        CString::new(text.replace('\0', "")).map_or(std::ptr::null_mut(), CString::into_raw)
    })
}

/// Delivers one event to a session.
#[no_mangle]
pub unsafe extern "C" fn pixmith_script_send(
    session: u64,
    doc: *mut Editor,
    doc_tag: u64,
    layer: u32,
    frame: u32,
    event_json: *const c_char,
) -> bool {
    crate::ffi::guarded("pixmith_script_send", std::ptr::null_mut(), || {
        let Some(text) = text_arg(event_json) else { return false };
        let Ok(ev) = json::parse(&text) else {
            crate::ffi::set_error("the event is not valid JSON");
            return false;
        };
        let snap = crate::ffi::editor_quiet(doc).map(|e| DocSnap::of(e, doc_tag, layer, frame));
        global().send(session, ev, snap)
    })
}

/// Delivers an app/sprite event to every session that listens for it.
#[no_mangle]
pub unsafe extern "C" fn pixmith_script_broadcast(
    doc: *mut Editor,
    doc_tag: u64,
    layer: u32,
    frame: u32,
    event_json: *const c_char,
) -> u32 {
    crate::ffi::guarded("pixmith_script_broadcast", std::ptr::null_mut(), || {
        let Some(text) = text_arg(event_json) else { return 0 };
        let Ok(ev) = json::parse(&text) else { return 0 };
        global().broadcast(&ev, crate::ffi::editor_quiet(doc).map(|e| &*e), doc_tag, layer, frame)
    })
}

/// Applies the result of transaction `txn` to `doc`: 1 applied, 0 nothing, -1 conflict, -2 unknown.
#[no_mangle]
pub unsafe extern "C" fn pixmith_script_apply(session: u64, txn: u64, doc: *mut Editor) -> i32 {
    crate::ffi::guarded("pixmith_script_apply", doc, || global().apply(session, txn, crate::ffi::editor(doc)))
}

/// The drawing colors scripts read as `app.fgColor` / `app.bgColor`.
#[no_mangle]
pub extern "C" fn pixmith_script_set_colors(fg: u32, bg: u32) {
    crate::ffi::guarded("pixmith_script_set_colors", std::ptr::null_mut(), || global().set_colors(fg, bg))
}

/// Stops a session.
#[no_mangle]
pub extern "C" fn pixmith_script_stop(session: u64) {
    crate::ffi::guarded("pixmith_script_stop", std::ptr::null_mut(), || global().stop(session))
}

/// Number of live sessions.
#[no_mangle]
pub extern "C" fn pixmith_script_count() -> u32 {
    crate::ffi::guarded("pixmith_script_count", std::ptr::null_mut(), || global().count())
}
