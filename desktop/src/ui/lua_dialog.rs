//! Lua dialogs, events and plugin commands in the egui app (`docs/LUA_DIALOG.md`; the macOS twin is
//! `App/ScriptHost.swift`).
//!
//! The core runs every script or plugin as a session on its own thread and describes what the user should see as
//! JSON messages. This module owns the front-end half of that contract:
//!
//! * [`LuaState`] keeps the sessions, the open dialogs and the plugin commands, and turns the polled messages into
//!   changes of them (`dialog.open`, `dialog.modify`, `dialog.close`, `state`, `txn`, `app.set`, `command.*`,
//!   `session.end`);
//! * `render` draws the dialogs with the design-system controls and `model` holds their data and events;
//! * the hooks at the end send the app's events to the scripts (`sitechange`, `fgcolorchange`, `beforecommand`,
//!   `sprite.change`, ...) and run plugin commands from File > Scripts.
//!
//! The only code that talks to the core is `calls::script_ui`.

pub mod model;
pub mod render;

use crate::app::PixmithApp;
use crate::calls::script_ui::{CoreHost, ScriptHost};
use crate::theme::{Colors, radius, space};
use crate::ui::gamedev::Notice;
use crate::ui::menus::commands::Cmd;
use crate::ui::widgets::{self, DialogAction};
use eframe::egui::{self, Id};
use model::{Dialog, Event};
use pixmith_core::formats::json::{self, Value};
use render::View;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// A script that has been running this long without a modal dialog gets a progress sheet with Cancel.
const PROGRESS_AFTER: Duration = Duration::from_millis(300);
/// How often to look for messages while the core is working on something we asked.
const POLL_EVERY: Duration = Duration::from_millis(8);

/// What a session is doing (`state` messages).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Running,
    /// Inside a modal `show`, waiting for the user.
    Waiting,
    Idle,
}

/// One script run or plugin.
pub struct Session {
    pub name: String,
    /// The document it was started on (`doc_tag`).
    pub tag: u64,
    pub state: SessionState,
    /// When it last entered `Running`.
    since: Instant,
    /// We sent it something and have not seen its next `state` yet.
    poked: bool,
    pub plugin: bool,
    /// Its Lua run locks the document (scripts, and plugin commands the user started); the housekeeping events
    /// the app sends to idle plugins do not.
    locking: bool,
    cancelling: bool,
}

impl Session {
    fn new(name: &str, tag: u64, plugin: bool) -> Session {
        Session {
            name: name.to_string(),
            tag,
            state: SessionState::Running,
            since: Instant::now(),
            poked: false,
            plugin,
            locking: !plugin,
            cancelling: false,
        }
    }
}

/// A dialog that is open, with what the renderer remembers about it.
pub struct Open {
    pub dialog: Dialog,
    view: View,
}

/// A command a plugin added to File > Scripts.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginCommand {
    pub session: u64,
    pub plugin: String,
    pub id: String,
    pub title: String,
    pub group: String,
    pub enabled: bool,
    pub checked: bool,
}

/// The drawing colors, layer and frame last reported to the scripts, to notice changes.
#[derive(Default)]
struct Seen {
    site: Option<(u64, u32, u32)>,
    colors: Option<(u32, u32)>,
    /// The revision of each document (by tag) when its scripts last heard about it.
    revisions: BTreeMap<u64, u64>,
    undone: BTreeMap<u64, u32>,
}

/// Everything the Lua side keeps in the app; one field of `GamedevState`.
pub struct LuaState {
    host: Box<dyn ScriptHost>,
    sessions: BTreeMap<u64, Session>,
    dialogs: Vec<Open>,
    commands: Vec<PluginCommand>,
    seen: Seen,
    /// A plugin should re-evaluate its commands' `onenabled` / `onchecked` (the site or the document changed).
    refresh_dirty: bool,
    last_refresh: Instant,
    /// Undo or Redo ran this frame (for `sprite.change`'s `fromUndo`).
    from_undo: bool,
    /// Notices produced while the app was borrowed, shown by the next `show`.
    notices: Vec<Notice>,
}

impl Default for LuaState {
    fn default() -> LuaState {
        LuaState::with_host(Box::new(CoreHost::isolated()))
    }
}

impl LuaState {
    pub fn with_host(host: Box<dyn ScriptHost>) -> LuaState {
        LuaState {
            host,
            sessions: BTreeMap::new(),
            dialogs: Vec::new(),
            commands: Vec::new(),
            seen: Seen::default(),
            refresh_dirty: false,
            last_refresh: Instant::now(),
            from_undo: false,
            notices: Vec::new(),
        }
    }

    /// Turns on what only the real app does: the host also takes sessions that other parts of the app start with
    /// the core directly (the extension loader).
    pub fn start(&mut self) {
        self.host = Box::new(CoreHost::default());
    }

    /// Replaces the host (tests use a `FakeHost`).
    pub fn set_host(&mut self, host: Box<dyn ScriptHost>) {
        *self = LuaState::with_host(host);
    }

    /// Whether scripts can run at all.
    pub fn available(&self) -> bool {
        self.host.available()
    }

    pub fn sessions(&self) -> impl Iterator<Item = (&u64, &Session)> {
        self.sessions.iter()
    }

    pub fn dialogs(&self) -> impl Iterator<Item = &Dialog> {
        self.dialogs.iter().map(|o| &o.dialog)
    }

    pub fn dialog(&self, id: &str) -> Option<&Dialog> {
        self.dialogs().find(|d| d.id == id)
    }

    pub fn commands(&self) -> &[PluginCommand] {
        &self.commands
    }

    /// A modal dialog is open: the window waits for it like for any sheet.
    pub fn modal_open(&self) -> bool {
        self.dialogs.iter().any(|o| o.dialog.modal)
    }

    /// A script is running Lua right now (a modal dialog waiting for the user does not count).
    pub fn busy(&self) -> bool {
        self.sessions.values().any(|s| s.locking && (s.state == SessionState::Running || s.poked))
    }

    /// Whether anything needs the core polled soon.
    fn polling(&self) -> bool {
        self.sessions.values().any(|s| s.state == SessionState::Running || s.poked)
    }

    fn has_modal_of(&self, session: u64) -> bool {
        self.dialogs.iter().any(|o| o.dialog.modal && o.dialog.session == session)
    }

    /// Takes the text of the notices produced since the last call.
    fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }

    /// Stops every session (quit) and gives plugins a moment to run their `exit`.
    pub fn stop_all(&mut self) {
        let ids: Vec<u64> = self.sessions.keys().copied().collect();
        for id in &ids {
            self.host.stop(*id);
        }
        let until = Instant::now() + Duration::from_millis(400);
        while self.host.count() > 0 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
            self.host.poll();
        }
        self.sessions.clear();
        self.dialogs.clear();
        self.commands.clear();
    }

    fn notice(&mut self, title: String, text: String, warning: bool) {
        self.notices.push(Notice { title, text, warning });
    }
}

fn tab_by_tag(app: &PixmithApp, tag: u64) -> Option<usize> {
    app.tabs.iter().position(|t| t.doc.raw() as usize as u64 == tag)
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn num_of(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

fn flag_of(v: &Value, key: &str, default: bool) -> bool {
    match v.get(key) {
        Some(Value::Bool(b)) => *b,
        _ => default,
    }
}

impl PixmithApp {
    /// Starts `source` as a script session on the active document (File > Scripts, Run Script...). The result
    /// arrives as messages: dialogs, then a transaction that is applied as one undo step.
    pub fn lua_start_script(&mut self, name: &str, source: &str) -> bool {
        let tab = &mut self.tabs[self.active];
        tab.playing = false;
        let (tag, layer, frame) = (tab.doc.raw() as usize as u64, tab.layer, tab.frame);
        let lua = &mut self.gamedev.lua;
        match lua.host.start(Some(&self.tabs[self.active].doc), tag, source, name, layer, frame) {
            Ok(id) => {
                lua.sessions.insert(id, Session::new(name, tag, false));
                true
            }
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }

    /// Loads a plugin script (`init(plugin)` runs once). Its commands appear in File > Scripts. Returns the
    /// session id. The extension loader calls this for every enabled extension script.
    pub fn lua_load_plugin(
        &mut self,
        source: &str,
        name: &str,
        dir: Option<&str>,
        prefs_path: Option<&str>,
    ) -> Result<u64, String> {
        let lua = &mut self.gamedev.lua;
        let id = lua.host.plugin_load(source, name, dir, prefs_path)?;
        lua.sessions.insert(id, Session::new(name, 0, true));
        Ok(id)
    }

    /// Unloads a plugin: its `exit` runs and its commands go.
    pub fn lua_unload_plugin(&mut self, session: u64) {
        self.gamedev.lua.host.stop(session);
    }

    /// The plugin command with this index in File > Scripts.
    pub fn lua_command(&self, index: usize) -> Option<&PluginCommand> {
        self.gamedev.lua.commands.get(index)
    }

    /// File > Scripts > a plugin command: sends `command` to its session on the active document.
    pub fn lua_run_command(&mut self, index: usize) {
        let Some(cmd) = self.gamedev.lua.commands.get(index).cloned() else { return };
        if !cmd.enabled || self.job_running() {
            return;
        }
        let event =
            Value::Obj(vec![("type".into(), Value::Str("command".into())), ("id".into(), Value::Str(cmd.id.clone()))]);
        if let Some(s) = self.gamedev.lua.sessions.get_mut(&cmd.session) {
            s.locking = true;
        }
        send_to_session(self, cmd.session, &json::to_string(&event), true);
    }

    /// Whether Lua is running something that locks the document (a progress sheet would show).
    pub fn lua_busy(&self) -> bool {
        self.gamedev.lua.busy()
    }
}

/// Sends `event` to a session with a snapshot of the document it works on (the active one for plugins).
fn send_to_session(app: &mut PixmithApp, session: u64, event: &str, poke: bool) -> bool {
    let tag = app.gamedev.lua.sessions.get(&session).map_or(0, |s| s.tag);
    let index = tab_by_tag(app, tag).unwrap_or(app.active);
    let (tag, layer, frame) = {
        let tab = &app.tabs[index];
        (tab.doc.raw() as usize as u64, tab.layer, tab.frame)
    };
    let lua = &mut app.gamedev.lua;
    let ok = lua.host.send(session, Some(&app.tabs[index].doc), tag, layer, frame, event);
    if let (true, true, Some(s)) = (ok, poke, lua.sessions.get_mut(&session)) {
        s.poked = true;
    }
    ok
}

// MARK: Messages from the core

/// Handles one polled message.
fn handle(app: &mut PixmithApp, msg: &Value) {
    let session = num_of(msg, "session").map_or(0, |n| n as u64);
    let kind = str_of(msg, "type");
    let lua = &mut app.gamedev.lua;
    // A session this front end did not start itself (a plugin loaded elsewhere) is adopted on first sight.
    if session != 0 && kind != "session.end" {
        lua.sessions.entry(session).or_insert_with(|| Session::new("script", 0, false));
    }
    match kind {
        "state" => {
            if let Some(s) = lua.sessions.get_mut(&session) {
                s.poked = false;
                let state = match str_of(msg, "state") {
                    "waiting" => SessionState::Waiting,
                    "idle" => SessionState::Idle,
                    _ => SessionState::Running,
                };
                if state == SessionState::Running && s.state != SessionState::Running {
                    s.since = Instant::now();
                }
                s.state = state;
                if state == SessionState::Idle {
                    s.locking = false;
                    if s.plugin {
                        lua.refresh_dirty = true;
                    }
                }
            }
        }
        "dialog.open" => {
            if let Some(d) = msg.get("dialog").and_then(|d| Dialog::from_json(session, d)) {
                lua.dialogs.retain(|o| o.dialog.id != d.id);
                lua.dialogs.push(Open { dialog: d, view: View::default() });
            }
        }
        "dialog.modify" => {
            let id = str_of(msg, "dialog");
            if let Some(open) = lua.dialogs.iter_mut().find(|o| o.dialog.id == id) {
                let focus = open.dialog.modify(msg);
                open.view.pending_focus.extend(focus);
            }
        }
        "dialog.close" => {
            let id = str_of(msg, "dialog");
            lua.dialogs.retain(|o| o.dialog.id != id);
        }
        "txn" => handle_txn(app, session, msg),
        "app.set" => {
            if let Some(c) = num_of(msg, "fgColor") {
                app.primary = c as u32;
            }
            if let Some(c) = num_of(msg, "bgColor") {
                app.secondary = c as u32;
            }
            // The scripts know already: do not echo it back as a change event.
            lua.seen.colors = Some((app.primary, app.secondary));
            lua.host.set_colors(app.primary, app.secondary);
        }
        "command.add" => {
            if let Some(s) = lua.sessions.get_mut(&session) {
                // A plugin another part of the app loaded: name it and keep it out of the busy state when idle.
                s.plugin = true;
                s.locking = false;
                if s.name == "script" && !str_of(msg, "plugin").is_empty() {
                    s.name = str_of(msg, "plugin").to_string();
                }
            }
            if let Some(c) = msg.get("command") {
                let command = PluginCommand {
                    session,
                    plugin: str_of(msg, "plugin").to_string(),
                    id: str_of(c, "id").to_string(),
                    title: str_of(c, "title").to_string(),
                    group: c.get("group").and_then(Value::as_str).unwrap_or("file_scripts").to_string(),
                    enabled: flag_of(c, "enabled", true),
                    checked: flag_of(c, "checked", false),
                };
                lua.commands.retain(|x| !(x.session == session && x.id == command.id));
                lua.commands.push(command);
            }
        }
        "command.update" => {
            if let Some(c) = msg.get("command") {
                let id = str_of(c, "id");
                if let Some(x) = lua.commands.iter_mut().find(|x| x.session == session && x.id == id) {
                    if let Some(t) = c.get("title").and_then(Value::as_str) {
                        x.title = t.to_string();
                    }
                    x.enabled = flag_of(c, "enabled", x.enabled);
                    x.checked = flag_of(c, "checked", x.checked);
                }
            }
        }
        "command.remove" => {
            let id = str_of(msg, "id");
            lua.commands.retain(|x| !(x.session == session && x.id == id));
        }
        "session.end" => {
            lua.sessions.remove(&session);
            lua.dialogs.retain(|o| o.dialog.session != session);
            lua.commands.retain(|x| x.session != session);
        }
        _ => {}
    }
}

/// A Lua call chain finished: apply its result to the document as one undo step and report errors and output.
fn handle_txn(app: &mut PixmithApp, session: u64, msg: &Value) {
    let txn = num_of(msg, "txn").map_or(0, |n| n as u64);
    let tag = num_of(msg, "doc").map_or(0, |n| n as u64);
    let ok = flag_of(msg, "ok", true);
    let error = msg.get("error").and_then(Value::as_str).map(str::to_string);
    let messages: Vec<String> = msg
        .get("messages")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|m| m.as_str().map(str::to_string))
        .collect();
    let name = app.gamedev.lua.sessions.get(&session).map_or_else(|| "script".to_string(), |s| s.name.clone());
    let index = tab_by_tag(app, tag);
    let mut conflict = false;
    let mut applied = false;
    if flag_of(msg, "commit", false) {
        let result = {
            let PixmithApp { tabs, gamedev, .. } = &mut *app;
            gamedev.lua.host.apply(session, txn, index.map(|i| &mut tabs[i].doc))
        };
        applied = result == 1;
        conflict = result == -1;
    }
    if let Some(i) = index {
        let tab = &mut app.tabs[i];
        if applied {
            if let Some(sel) = msg.get("select") {
                let layer = num_of(sel, "layer").map_or(0, |n| n as u32);
                let frame = num_of(sel, "frame").map_or(0, |n| n as u32);
                tab.layer = layer.saturating_sub(1);
                tab.frame = frame.saturating_sub(1);
            }
            // A script may have removed the current layer or frame.
            (tab.layer, tab.frame) = tab.doc.clamp_selection(tab.layer, tab.frame);
            tab.changed();
            // A script's own result is no user edit: its listeners must not hear it as `sprite.change`.
            app.gamedev.lua.seen.revisions.insert(tag, tab.revision);
        }
    }
    let lua = &mut app.gamedev.lua;
    let cancelled = error.as_deref() == Some("script cancelled");
    if conflict {
        lua.notice(name, "The document changed while the script was running; its result was discarded.".into(), true);
    } else if !ok && !cancelled {
        let mut parts = Vec::new();
        if !messages.is_empty() {
            parts.push(messages.join("\n"));
        }
        parts.push(format!("{}\nThe document was not changed.", error.unwrap_or_default()));
        lua.notice(format!("Script error in {name}"), parts.join("\n\n"), true);
    } else if ok && !messages.is_empty() {
        lua.notice(name, messages.join("\n"), false);
    }
}

/// Polls the core and handles what arrived. Returns true when there was anything.
fn pump(app: &mut PixmithApp) -> bool {
    let messages = app.gamedev.lua.host.poll();
    let any = !messages.is_empty();
    for m in &messages {
        handle(app, m);
    }
    any
}

// MARK: Events from the app

/// `{"type":"app.event","name":...}` with the given extra members.
fn app_event(name: &str, extra: Vec<(&str, Value)>) -> String {
    let mut members =
        vec![("type".to_string(), Value::Str("app.event".into())), ("name".to_string(), Value::Str(name.into()))];
    members.extend(extra.into_iter().map(|(k, v)| (k.to_string(), v)));
    json::to_string(&Value::Obj(members))
}

fn broadcast(app: &mut PixmithApp, event: &str) {
    let tab = &app.tabs[app.active];
    let (tag, layer, frame) = (tab.doc.raw() as usize as u64, tab.layer, tab.frame);
    app.gamedev.lua.host.broadcast(Some(&tab.doc), tag, layer, frame, event);
}

/// Tells the scripts what changed since the last frame: the site (document, layer, frame), the drawing colors and
/// the document itself. Cheap while no session exists.
fn send_app_events(app: &mut PixmithApp) {
    let (fg, bg) = (app.primary, app.secondary);
    let active = app.tab();
    let (tag, layer, frame) = (active.doc.raw() as usize as u64, active.layer, active.frame);
    let revision = active.revision;
    let undone = active.doc.history_counts().1;
    let lua = &mut app.gamedev.lua;
    let first_colors = lua.seen.colors.is_none();
    let colors_changed = lua.seen.colors != Some((fg, bg));
    if colors_changed {
        lua.host.set_colors(fg, bg);
    }
    let old_colors = lua.seen.colors.replace((fg, bg));
    let site_changed = lua.seen.site.is_some_and(|s| s != (tag, layer, frame));
    lua.seen.site = Some((tag, layer, frame));
    let previous_revision = lua.seen.revisions.insert(tag, revision);
    let previous_undone = lua.seen.undone.insert(tag, undone);
    let edited = previous_revision.is_some_and(|r| r != revision);
    let from_undo = std::mem::take(&mut lua.from_undo) || previous_undone.is_some_and(|u| undone > u);
    if lua.host.count() == 0 {
        return;
    }
    if site_changed || edited {
        lua.refresh_dirty = true;
    }
    if site_changed {
        let e = app_event(
            "sitechange",
            vec![("layer", Value::Num(f64::from(layer) + 1.0)), ("frame", Value::Num(f64::from(frame) + 1.0))],
        );
        broadcast(app, &e);
    }
    if let (false, Some((old_fg, old_bg))) = (first_colors, old_colors) {
        if old_fg != fg {
            broadcast(app, &app_event("fgcolorchange", vec![("color", Value::Num(f64::from(fg)))]));
        }
        if old_bg != bg {
            broadcast(app, &app_event("bgcolorchange", vec![("color", Value::Num(f64::from(bg)))]));
        }
    }
    if edited {
        let e = json::to_string(&Value::Obj(vec![
            ("type".into(), Value::Str("sprite.event".into())),
            ("name".into(), Value::Str("change".into())),
            ("fromUndo".into(), Value::Bool(from_undo)),
        ]));
        broadcast(app, &e);
    }
}

/// Called before a menu command runs (`beforecommand`).
pub fn before_command(app: &mut PixmithApp, cmd: Cmd) {
    if matches!(cmd, Cmd::Undo | Cmd::Redo) {
        app.gamedev.lua.from_undo = true;
    }
    command_event(app, cmd, "beforecommand");
}

/// Called after a menu command ran (`aftercommand`).
pub fn after_command(app: &mut PixmithApp, cmd: Cmd) {
    command_event(app, cmd, "aftercommand");
}

fn command_event(app: &mut PixmithApp, cmd: Cmd, name: &str) {
    if app.gamedev.lua.host.count() == 0 || matches!(cmd, Cmd::Script(_) | Cmd::RunScript | Cmd::PluginCommand(_)) {
        return;
    }
    let title = cmd.label(app);
    broadcast(app, &app_event(name, vec![("command", Value::Str(title))]));
}

/// The undo and redo keys are handled by the canvas: tell `sprite.change` that the next change is from undo.
pub fn note_undo(app: &mut PixmithApp) {
    app.gamedev.lua.from_undo = true;
}

/// The document was saved under another name (`filenamechange`).
pub fn note_file_renamed(app: &mut PixmithApp) {
    if app.gamedev.lua.host.count() > 0 {
        let e = json::to_string(&Value::Obj(vec![
            ("type".into(), Value::Str("sprite.event".into())),
            ("name".into(), Value::Str("filenamechange".into())),
        ]));
        broadcast(app, &e);
    }
}

// MARK: Frame

/// Everything the Lua side does each frame; call once per frame after the other dialogs.
pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    let lua = &app.gamedev.lua;
    if !lua.available() && lua.sessions.is_empty() {
        return;
    }
    let mut got = pump(app);
    send_app_events(app);
    refresh_plugins(app, ctx);
    let mut events = render_dialogs(app, ctx);
    for (session, dialog, event) in events.drain(..) {
        let json = event.to_json(&dialog);
        send_to_session(app, session, &json, true);
        got = true;
    }
    progress_sheet(app, ctx);
    show_notices(app, ctx);
    if got {
        ctx.request_repaint();
    }
    if app.gamedev.lua.polling() {
        ctx.request_repaint_after(POLL_EVERY);
    }
}

/// Plugins decide whether their commands are enabled from the document of the last event they got, so an idle
/// plugin hears a no-op event whenever the site or the document changed (at most four times a second).
fn refresh_plugins(app: &mut PixmithApp, ctx: &egui::Context) {
    const EVERY: Duration = Duration::from_millis(250);
    let lua = &mut app.gamedev.lua;
    if !lua.refresh_dirty {
        return;
    }
    if lua.last_refresh.elapsed() < EVERY {
        ctx.request_repaint_after(EVERY - lua.last_refresh.elapsed());
        return;
    }
    let ids: Vec<u64> = lua
        .sessions
        .iter()
        .filter(|(_, s)| s.plugin && s.state == SessionState::Idle && !s.locking && !s.poked)
        .map(|(id, _)| *id)
        .collect();
    if ids.is_empty() {
        return;
    }
    lua.refresh_dirty = false;
    lua.last_refresh = Instant::now();
    let event = app_event("refresh", Vec::new());
    for id in ids {
        send_to_session(app, id, &event, false);
    }
}

/// Moves the notices the Lua side collected into the app's notice sheet, joining them when one is already open.
fn show_notices(app: &mut PixmithApp, _ctx: &egui::Context) {
    for n in app.gamedev.lua.take_notices() {
        match &mut app.gamedev.notice {
            Some(existing) => {
                existing.title = n.title;
                existing.text = format!("{}\n\n{}", existing.text, n.text);
                existing.warning |= n.warning;
            }
            slot => *slot = Some(n),
        }
    }
}

type Outgoing = Vec<(u64, String, Event)>;

fn render_dialogs(app: &mut PixmithApp, ctx: &egui::Context) -> Outgoing {
    let mut outgoing = Outgoing::new();
    let mut dialogs = std::mem::take(&mut app.gamedev.lua.dialogs);
    let top_modal = dialogs.iter().rposition(|o| o.dialog.modal);
    let mut windows = 0;
    for (i, open) in dialogs.iter_mut().enumerate() {
        let mut events = Vec::new();
        if open.dialog.modal {
            show_modal(ctx, open, top_modal == Some(i), &mut events);
        } else {
            windows += 1;
            show_window(ctx, open, windows - 1, &mut events);
        }
        outgoing.extend(coalesce(events).into_iter().map(|e| (open.dialog.session, open.dialog.id.clone(), e)));
    }
    // Dialogs the core opened or closed while we were drawing (none today) are kept; the closed ones go.
    let closed: Vec<String> =
        outgoing.iter().filter(|(_, _, e)| *e == Event::Closed).map(|(_, d, _)| d.clone()).collect();
    dialogs.retain(|o| !closed.contains(&o.dialog.id));
    let added = std::mem::replace(&mut app.gamedev.lua.dialogs, dialogs);
    app.gamedev.lua.dialogs.extend(added);
    outgoing
}

/// Several `change`s of one widget in a frame collapse into the last (the core does the same while a callback
/// runs, this just saves the messages).
fn coalesce(events: Vec<Event>) -> Vec<Event> {
    let mut out: Vec<Event> = Vec::with_capacity(events.len());
    for e in events {
        if let Event::Change { id, .. } = &e
            && let Some(Event::Change { id: last, .. }) = out.last()
            && last == id
        {
            out.pop();
        }
        out.push(e);
    }
    out
}

fn frame_for(c: &Colors, margin: i8) -> egui::Frame {
    egui::Frame::new()
        .fill(c.panel)
        .stroke(egui::Stroke::new(1.0, c.separator))
        .corner_radius(egui::CornerRadius::same(radius::PANEL + 2))
        .inner_margin(egui::Margin::same(margin))
        .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: egui::Color32::from_black_alpha(90) })
}

/// A modal dialog: centered over a dimmed window that cannot be used meanwhile (`DSDialog`). Return confirms the
/// default button, Escape closes.
fn show_modal(ctx: &egui::Context, open: &mut Open, top: bool, out: &mut Vec<Event>) {
    let c = Colors::of(ctx);
    let width = render::natural_width(ctx, &open.dialog);
    egui::Modal::new(Id::new(("lua_modal", &open.dialog.id)))
        .frame(frame_for(&c, 20))
        .backdrop_color(egui::Color32::from_black_alpha(80))
        .show(ctx, |ui| {
            ui.set_width(width);
            if !open.dialog.notitlebar && !open.dialog.title.is_empty() {
                ui.label(egui::RichText::new(&open.dialog.title).font(widgets::strong(ui.ctx(), 15.0)));
                ui.add_space(space::SM);
            }
            if top {
                if let Some(button) = open.dialog.default_button().map(|b| b.id.clone())
                    && !egui::Popup::is_any_open(ui.ctx())
                    && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                {
                    out.push(Event::Click { id: button, value: None, color: None });
                }
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                    out.push(Event::Closed);
                }
            }
            render::body(ui, &mut open.dialog, &mut open.view, out);
        });
}

/// A non-modal dialog: a tool window the user can move, with a close button.
fn show_window(ctx: &egui::Context, open: &mut Open, index: usize, out: &mut Vec<Event>) {
    let c = Colors::of(ctx);
    let width = render::natural_width(ctx, &open.dialog);
    let step = 28.0 * index as f32;
    egui::Window::new(&open.dialog.id)
        .id(Id::new(("lua_window", &open.dialog.id)))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .movable(true)
        .default_pos(egui::pos2(96.0 + step, 120.0 + step))
        .frame(frame_for(&c, 16))
        .show(ctx, |ui| {
            ui.set_width(width);
            if !open.dialog.notitlebar {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&open.dialog.title).font(widgets::strong(ui.ctx(), 13.0)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let r = widgets::ds_icon_button(ui, egui_phosphor::regular::X, 24.0, 13.0, false, false);
                        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close"));
                        if r.clicked() {
                            out.push(Event::Closed);
                        }
                    });
                });
                ui.add_space(space::XS);
            }
            render::body(ui, &mut open.dialog, &mut open.view, out);
        });
}

/// The sheet shown while a script keeps Lua busy for a while and has no modal dialog open (`ProgressSheet`).
fn progress_sheet(app: &mut PixmithApp, ctx: &egui::Context) {
    let lua = &app.gamedev.lua;
    let shown = lua
        .sessions
        .iter()
        .filter(|(id, s)| {
            s.locking
                && s.state == SessionState::Running
                && !lua.has_modal_of(**id)
                && s.since.elapsed() >= PROGRESS_AFTER
        })
        .map(|(id, s)| (*id, s.name.clone(), s.cancelling))
        .next();
    let Some((id, name, cancelling)) = shown else { return };
    let c = Colors::of(ctx);
    let title = format!("Running {name}\u{2026}");
    let action = widgets::dialog(ctx, "lua_job", 340.0, &title, None, |ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, radius::CHIP, c.field);
        let t = ui.input(|i| i.time) as f32;
        let seg = rect.width() * 0.3;
        let x = rect.left() - seg + (t * 0.9).fract() * (rect.width() + seg);
        let lit = egui::Rect::from_min_max(
            egui::pos2(x.max(rect.left()), rect.top()),
            egui::pos2((x + seg).min(rect.right()), rect.bottom()),
        );
        if lit.is_positive() {
            ui.painter().rect_filled(lit, radius::CHIP, c.accent);
        }
        ui.ctx().request_repaint();
        ui.add_space(space::XS);
        let mut action = DialogAction::None;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!cancelling, |ui| {
                    let label = if cancelling { "Cancelling\u{2026}" } else { "Cancel" };
                    let r = widgets::secondary_button(ui, label, false);
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
                    if r.clicked() {
                        action = DialogAction::Cancel;
                    }
                });
            });
        });
        if !cancelling && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            action = DialogAction::Cancel;
        }
        action
    });
    if action == DialogAction::Cancel {
        let lua = &mut app.gamedev.lua;
        if let Some(s) = lua.sessions.get_mut(&id) {
            s.cancelling = true;
        }
        lua.host.stop(id);
    }
}
