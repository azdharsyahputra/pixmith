//! Pixmith Live (`docs/LIVE_PROTOCOL.md`): lets a local tool such as the Pixmith MCP server read and edit the
//! documents open in the app.
//!
//! [`net`] runs the TCP server on background threads. This module is the UI-thread half: once per frame
//! [`frame`] hands every waiting request to [`handle`], so all document access happens on the UI thread, between
//! frames. The state lives in [`LiveState`], one per app.

mod base64;
pub mod net;

use crate::app::{DocTab, PixmithApp, Tool};
use crate::doc::Doc;
use crate::ui::gamedev::paths;
use eframe::egui;
use net::{ERR_BUSY, ERR_OTHER, ERR_PARAMS, ERR_STALE, ERR_UNKNOWN_DOCUMENT, ERR_UNSAVED, Reply, RpcError, Server};
use pixmith_core::ffi::live::{Region, SelectionChange};
use pixmith_core::formats::json::{self, Value};
use std::path::{Path, PathBuf};

/// Live mode state of the app: the server (while the preference is on) and the ids and revisions of the tabs.
#[derive(Default)]
pub struct LiveState {
    /// The running server; `None` while the preference is off or the port could not be opened.
    pub server: Option<Server>,
    /// Follow the preference (start and stop the server with it). Only the real app does; tests and screenshot
    /// scenarios leave it off and start a server explicitly.
    pub managed: bool,
    /// Why the server could not start, for the Preferences window.
    pub error: Option<String>,
    /// Where the discovery files go (`<app data>`); the real app folder unless a test says otherwise.
    app_dir: Option<PathBuf>,
    docs: Vec<Tracked>,
    next_id: u64,
    /// Documents (`Doc::uid`) a Live request created or opened. Their tabs are never reused as a blank tab, so
    /// two quick `documents.new` calls cannot swallow each other's document.
    created: Vec<u64>,
}

/// What Live knows about one open document.
struct Tracked {
    uid: u64,
    id: String,
    revision: u64,
    /// The tab's revision and history state as of the last look; a difference means the document changed.
    seen: (u64, u64),
}

impl LiveState {
    /// Number of clients that completed `hello`.
    pub fn connected(&self) -> Vec<net::Client> {
        self.server.as_ref().map(Server::clients).unwrap_or_default()
    }

    /// Starts the server (if it is not running) with discovery files under `app_dir`.
    pub fn start_in(&mut self, app_dir: &Path, ctx: Option<egui::Context>) {
        if self.server.is_some() {
            return;
        }
        self.app_dir = Some(app_dir.to_path_buf());
        match Server::start(app_dir, ctx) {
            Ok(s) => {
                self.server = Some(s);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Live mode could not start: {e}")),
        }
    }

    /// Closes the port and removes the discovery file.
    pub fn stop(&mut self) {
        self.server = None;
    }

    /// Notes which tabs exist and bumps the revision of every one that changed since the last look.
    fn update(&mut self, current: &[(u64, (u64, u64))]) {
        self.docs.retain(|d| current.iter().any(|(uid, _)| *uid == d.uid));
        self.created.retain(|c| current.iter().any(|(uid, _)| uid == c));
        for &(uid, seen) in current {
            match self.docs.iter_mut().find(|d| d.uid == uid) {
                Some(d) if d.seen != seen => {
                    d.seen = seen;
                    d.revision += 1;
                }
                Some(_) => {}
                None => {
                    self.next_id += 1;
                    self.docs.push(Tracked { uid, id: format!("doc-{}", self.next_id), revision: 1, seen });
                }
            }
        }
    }

    /// Remembers that a Live request made this document, so its tab is not reused as a blank one.
    fn mark_created(&mut self, uid: u64) {
        if !self.created.contains(&uid) {
            self.created.push(uid);
        }
    }

    /// Whether a Live request created or opened this document.
    pub fn was_created(&self, uid: u64) -> bool {
        self.created.contains(&uid)
    }

    fn tracked(&self, uid: u64) -> Option<&Tracked> {
        self.docs.iter().find(|d| d.uid == uid)
    }
}

fn snapshot(tabs: &[DocTab]) -> Vec<(u64, (u64, u64))> {
    tabs.iter().map(|t| (t.doc.uid(), (t.revision, t.doc.state_id()))).collect()
}

/// Brings the revisions up to date with the tabs.
fn refresh(app: &mut PixmithApp) {
    let current = snapshot(&app.tabs);
    app.live.update(&current);
}

/// Called at the start of every frame: follows the preference, then answers the waiting requests.
pub fn frame(app: &mut PixmithApp, ctx: &egui::Context) {
    if app.live.managed {
        let wanted = app.menus.prefs.live_enabled;
        if wanted && app.live.server.is_none() && app.live.error.is_none() {
            app.live.start_in(&paths::app_dir(), Some(ctx.clone()));
        } else if !wanted && app.live.server.is_some() {
            app.live.stop();
        }
        if wanted != app.menus.prefs.live_enabled || (!wanted && app.live.error.is_some()) {
            app.live.error = None;
        }
    }
    if app.live.server.is_none() {
        return;
    }
    if pump(app) {
        ctx.request_repaint();
    }
}

/// Answers every request that is waiting. Returns whether there were any.
pub fn pump(app: &mut PixmithApp) -> bool {
    let Some(server) = &app.live.server else { return false };
    let requests = server.take_requests();
    let any = !requests.is_empty();
    for request in requests {
        let reply = handle(app, &request.method, &request.params);
        request.answer(reply);
    }
    any
}

// MARK: Requests

fn err(code: i32, message: impl Into<String>) -> RpcError {
    RpcError::new(code, message)
}

fn obj(members: Vec<(&str, Value)>) -> Value {
    Value::Obj(members.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn num(n: impl Into<f64>) -> Value {
    Value::Num(n.into())
}

fn string(s: impl Into<String>) -> Value {
    Value::Str(s.into())
}

fn required<'a>(params: &'a Value, key: &str) -> Result<&'a Value, RpcError> {
    params.get(key).filter(|v| **v != Value::Null).ok_or_else(|| err(ERR_PARAMS, format!("missing \"{key}\"")))
}

fn str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, RpcError> {
    required(params, key)?.as_str().ok_or_else(|| err(ERR_PARAMS, format!("\"{key}\" must be a string")))
}

fn int_value(v: &Value, key: &str) -> Result<i64, RpcError> {
    match v {
        Value::Num(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Ok(*n as i64),
        _ => Err(err(ERR_PARAMS, format!("\"{key}\" must be an integer"))),
    }
}

fn int_param(params: &Value, key: &str) -> Result<i64, RpcError> {
    int_value(required(params, key)?, key)
}

fn opt_int(params: &Value, key: &str) -> Result<Option<i64>, RpcError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => int_value(v, key).map(Some),
    }
}

fn bool_param(params: &Value, key: &str) -> Result<bool, RpcError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        _ => Err(err(ERR_PARAMS, format!("\"{key}\" must be true or false"))),
    }
}

/// Dispatches one request. Reads work whatever the user is doing; everything else needs the app to be free of
/// open sheets and shape sessions, and finishes a stroke in progress first.
pub fn handle(app: &mut PixmithApp, method: &str, params: &Value) -> Reply {
    refresh(app);
    match method {
        "documents.list" => return Ok(list(app)),
        "documents.get" => return get(app, params),
        "godot.atlas" => return godot_atlas(app, params),
        "view.get" => return Ok(view(app, app.active)),
        "documents.apply" | "documents.new" | "documents.open" | "documents.save" | "documents.activate"
        | "documents.close" | "documents.undo" | "documents.redo" | "view.set" => {}
        _ => return Err(err(net::ERR_METHOD, format!("unknown method \"{method}\""))),
    }
    // A document that does not exist is reported before the app being busy.
    let target = match method {
        "documents.new" | "documents.open" => app.active,
        "view.set" if matches!(params.get("id"), None | Some(Value::Null)) => app.active,
        _ => find(app, params)?,
    };
    if let Some(what) = busy(app) {
        return Err(err(ERR_BUSY, format!("Pixmith is busy: {what}")));
    }
    if crate::ui::canvas::finish_gesture(app) {
        refresh(app);
    }
    let result = match method {
        "documents.apply" => apply(app, target, params),
        "documents.new" => new_document(app, params),
        "documents.open" => open(app, params),
        "documents.save" => save(app, target, params),
        "documents.activate" => {
            app.active = target;
            app.show_editor();
            Ok(obj(vec![("document", doc_json(app, target))]))
        }
        "documents.close" => close(app, target, params),
        "documents.undo" => step_history(app, target, params, false),
        "documents.redo" => step_history(app, target, params, true),
        _ => set_view(app, target, params),
    };
    refresh(app);
    result
}

fn tab_of(app: &PixmithApp, id: &str) -> Option<usize> {
    app.tabs.iter().position(|t| app.live.tracked(t.doc.uid()).is_some_and(|d| d.id == id))
}

fn find(app: &PixmithApp, params: &Value) -> Result<usize, RpcError> {
    let id = str_param(params, "id")?;
    tab_of(app, id).ok_or_else(|| err(ERR_UNKNOWN_DOCUMENT, format!("no open document \"{id}\"")))
}

/// Why the app cannot take a change right now, if it cannot.
fn busy(app: &PixmithApp) -> Option<&'static str> {
    let (f, m) = (&app.files, &app.menus);
    if f.renaming.is_some() {
        Some("a tab is being renamed")
    } else if f.new_sheet.is_some()
        || f.prompt.is_some()
        || f.export.is_some()
        || f.sheet_import.is_some()
        || m.filter.is_some()
        || m.scale.is_some()
        || m.canvas_size.is_some()
        || m.cel.is_some()
        || app.selection.modify.is_some()
        || app.timeline.sheet_open()
        || app.tilemap.sheet.is_some()
        || app.tilemap.slice_sheet.is_some()
        || app.tilemap.user_sheet.is_some()
        || app.gamedev.godot.is_some()
        || app.gamedev.tileset.is_some()
        || app.error.is_some()
    {
        Some("a sheet is open")
    } else if app.canvas.tools.draw.session.is_some() {
        Some("a shape is being drawn")
    } else {
        None
    }
}

fn doc_json(app: &PixmithApp, i: usize) -> Value {
    let tab = &app.tabs[i];
    let tracked = app.live.tracked(tab.doc.uid());
    obj(vec![
        ("id", string(tracked.map_or("", |d| d.id.as_str()))),
        ("name", string(app.tab_name(i))),
        ("path", tab.doc.path.as_ref().map_or(Value::Null, |p| string(p.to_string_lossy()))),
        ("width", num(tab.doc.width())),
        ("height", num(tab.doc.height())),
        ("frames", num(tab.doc.frame_count())),
        ("layers", num(tab.doc.layer_count())),
        ("revision", num(tracked.map_or(0, |d| d.revision) as f64)),
        ("dirty", Value::Bool(tab.doc.is_dirty())),
        ("active", Value::Bool(i == app.active)),
        ("layer", num(tab.layer)),
        ("frame", num(tab.frame)),
    ])
}

fn list(app: &PixmithApp) -> Value {
    let docs = (0..app.tabs.len()).map(|i| doc_json(app, i)).collect();
    let active = doc_json(app, app.active).get("id").cloned().unwrap_or(Value::Null);
    obj(vec![("documents", Value::Arr(docs)), ("active", active)])
}

fn get(app: &mut PixmithApp, params: &Value) -> Reply {
    let i = find(app, params)?;
    let bytes = app.tabs[i].doc.encode().map_err(|e| err(ERR_OTHER, e))?;
    let selection = app.tabs[i].doc.live_selection().map_or(Value::Null, |r| {
        obj(vec![
            ("x", num(r.x)),
            ("y", num(r.y)),
            ("w", num(r.w)),
            ("h", num(r.h)),
            ("mask", string(base64::encode(&r.mask))),
        ])
    });
    Ok(obj(vec![("document", doc_json(app, i)), ("pxmt", string(base64::encode(&bytes))), ("selection", selection)]))
}

/// `godot.atlas`: the Godot Live Link layout of the document (see `docs/LIVE_PROTOCOL.md`) with its `Doc` added.
fn godot_atlas(app: &mut PixmithApp, params: &Value) -> Reply {
    let i = find(app, params)?;
    let text = app.tabs[i].doc.godot_link().map_err(|e| err(ERR_OTHER, e))?;
    let Value::Obj(mut members) = json::parse(&text).map_err(|e| err(ERR_OTHER, e))? else {
        return Err(err(ERR_OTHER, "the Godot layout is not an object"));
    };
    members.insert(0, ("document".to_string(), doc_json(app, i)));
    Ok(Value::Obj(members))
}

/// The optional `selection` of `documents.apply`: absent keeps the selection, null clears it, an object sets it.
fn selection_change(params: &Value) -> Result<SelectionChange, RpcError> {
    let bad = |what: &str| err(ERR_PARAMS, format!("\"selection\": {what}"));
    match params.get("selection") {
        None => Ok(SelectionChange::Keep),
        Some(Value::Null) => Ok(SelectionChange::Clear),
        Some(v @ Value::Obj(_)) => {
            let int =
                |k: &str| v.get(k).map(|n| int_value(n, k)).unwrap_or_else(|| Err(bad(&format!("missing \"{k}\""))));
            let (x, y, w, h) = (int("x")?, int("y")?, int("w")?, int("h")?);
            if w < 0 || h < 0 || w > 1 << 16 || h > 1 << 16 || x.abs() > 1 << 20 || y.abs() > 1 << 20 {
                return Err(bad("the rectangle is out of range"));
            }
            let mask = base64::decode(v.get("mask").and_then(Value::as_str).ok_or_else(|| bad("missing \"mask\""))?)
                .map_err(|e| bad(&format!("\"mask\" is {e}")))?;
            if mask.len() != (w * h) as usize {
                return Err(bad(&format!("\"mask\" has {} bytes, not w * h = {}", mask.len(), w * h)));
            }
            Ok(SelectionChange::Set(Region { x: x as i32, y: y as i32, w: w as u32, h: h as u32, mask }))
        }
        Some(_) => Err(bad("must be an object or null")),
    }
}

/// `documents.undo` and `documents.redo`: up to `steps` (default 1) steps of the history the user sees.
fn step_history(app: &mut PixmithApp, i: usize, params: &Value, redo: bool) -> Reply {
    let wanted = opt_int(params, "steps")?.unwrap_or(1);
    if wanted < 0 {
        return Err(err(ERR_PARAMS, "\"steps\" must be 0 or more"));
    }
    let tab = &mut app.tabs[i];
    let mut done = 0;
    while done < wanted.min(10_000) && (if redo { tab.doc.redo() } else { tab.doc.undo() }) {
        done += 1;
    }
    if done > 0 {
        tab.changed();
        clamp_position(tab);
    }
    refresh(app);
    Ok(obj(vec![("document", doc_json(app, i)), ("steps", num(done as f64))]))
}

fn apply(app: &mut PixmithApp, i: usize, params: &Value) -> Reply {
    let base = int_param(params, "base_revision")?;
    let current = app.live.tracked(app.tabs[i].doc.uid()).map_or(0, |d| d.revision);
    if base != current as i64 {
        return Err(err(
            ERR_STALE,
            format!("stale revision: the document is at revision {current}, not {base}; fetch it again"),
        ));
    }
    let bytes = base64::decode(str_param(params, "pxmt")?).map_err(|e| err(ERR_PARAMS, format!("\"pxmt\": {e}")))?;
    let label = match params.get("label").and_then(Value::as_str).map(str::trim) {
        Some(l) if !l.is_empty() => l,
        _ => "edit",
    };
    let (layer, frame) = (opt_int(params, "layer")?, opt_int(params, "frame")?);
    let selection = selection_change(params)?;
    let tab = &mut app.tabs[i];
    tab.doc.apply_document_selection(&bytes, &format!("AI: {label}"), &selection).map_err(|e| err(ERR_OTHER, e))?;
    tab.changed();
    clamp_position(tab);
    if let Some(l) = layer {
        tab.layer = l.clamp(0, tab.doc.layer_count().saturating_sub(1) as i64) as u32;
    }
    if let Some(f) = frame {
        tab.frame = f.clamp(0, tab.doc.frame_count().saturating_sub(1) as i64) as u32;
    }
    refresh(app);
    Ok(obj(vec![("document", doc_json(app, i))]))
}

/// Keeps the current layer and frame inside the document.
fn clamp_position(tab: &mut DocTab) {
    tab.layer = tab.layer.min(tab.doc.layer_count().saturating_sub(1));
    tab.frame = tab.frame.min(tab.doc.frame_count().saturating_sub(1));
}

fn new_document(app: &mut PixmithApp, params: &Value) -> Reply {
    // Everything is checked and built before a tab exists, so a failing request never leaves a half-made one.
    let bytes = match params.get("pxmt") {
        None | Some(Value::Null) => None,
        Some(_) => {
            Some(base64::decode(str_param(params, "pxmt")?).map_err(|e| err(ERR_PARAMS, format!("\"pxmt\": {e}")))?)
        }
    };
    let dimension = |key: &str| -> Result<u32, RpcError> {
        let v = int_param(params, key)?;
        if (1..=4096).contains(&v) {
            Ok(v as u32)
        } else {
            Err(err(ERR_PARAMS, format!("\"{key}\" must be 1 to 4096")))
        }
    };
    let name = match params.get("name") {
        None | Some(Value::Null) => None,
        Some(Value::Str(s)) => {
            let s = s.trim();
            if s.contains(['/', '\\', ':']) {
                return Err(err(ERR_PARAMS, "\"name\" must not contain / \\ or :"));
            }
            (!s.is_empty()).then(|| s.to_string())
        }
        Some(_) => return Err(err(ERR_PARAMS, "\"name\" must be a string")),
    };
    let mut doc = match &bytes {
        Some(bytes) => Doc::from_bytes(bytes).map_err(|e| err(ERR_OTHER, e))?,
        None => Doc::new(dimension("width")?, dimension("height")?),
    };
    doc.name = name;
    // The app's own blank untitled tab is reused (once); a tab an earlier Live request made never is. Either
    // way the document ends up in the active tab.
    app.present(doc);
    mark_active_created(app);
    refresh(app);
    Ok(obj(vec![("document", doc_json(app, app.active))]))
}

/// The active tab holds what a Live request just created or opened: it is not a blank tab to reuse any more.
fn mark_active_created(app: &mut PixmithApp) {
    if let Some(tab) = app.tabs.get(app.active) {
        let uid = tab.doc.uid();
        app.live.mark_created(uid);
    }
}

fn absolute_path(params: &Value) -> Result<PathBuf, RpcError> {
    let path = PathBuf::from(str_param(params, "path")?);
    if path.is_absolute() { Ok(path) } else { Err(err(ERR_PARAMS, "\"path\" must be an absolute path")) }
}

fn open(app: &mut PixmithApp, params: &Value) -> Reply {
    let path = absolute_path(params)?;
    if path.is_dir() {
        return Err(err(ERR_PARAMS, "\"path\" is a folder; open a document file"));
    }
    app.open_file_in_tab(&path).map_err(|e| err(ERR_OTHER, e))?;
    mark_active_created(app);
    refresh(app);
    Ok(obj(vec![("document", doc_json(app, app.active))]))
}

fn save(app: &mut PixmithApp, i: usize, params: &Value) -> Reply {
    let path = match params.get("path") {
        None | Some(Value::Null) => app.tabs[i]
            .doc
            .path
            .clone()
            .ok_or_else(|| err(ERR_PARAMS, "the document has no file yet; pass \"path\""))?,
        Some(_) => absolute_path(params)?,
    };
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    if !matches!(ext.as_deref(), Some("pxmt" | "aseprite" | "ase")) {
        return Err(err(ERR_PARAMS, "save as .pxmt or .aseprite"));
    }
    // Another tab holds the file: with `replace_open` a tab without unsaved changes is closed first.
    let mut i = i;
    if bool_param(params, "replace_open")? {
        let uid = app.tabs[i].doc.uid();
        if let Some(j) = app.tab_holding(i, &path).filter(|&j| !app.tabs[j].doc.is_dirty()) {
            app.close_tab(j, None);
            i = app.tabs.iter().position(|t| t.doc.uid() == uid).unwrap_or(i);
        }
    }
    app.write_checked(i, &path).map_err(|e| err(ERR_OTHER, e))?;
    Ok(obj(vec![("document", doc_json(app, i))]))
}

fn close(app: &mut PixmithApp, i: usize, params: &Value) -> Reply {
    let discard = bool_param(params, "discard")?;
    if app.tabs[i].doc.is_dirty() && !discard {
        return Err(err(ERR_UNSAVED, format!("\"{}\" has unsaved changes", app.tab_name(i))));
    }
    app.close_tab(i, None);
    Ok(obj(vec![]))
}

fn tool_name(tool: Tool) -> &'static str {
    match tool {
        Tool::Marquee => "marquee",
        Tool::Lasso => "lasso",
        Tool::Polygon => "polygon",
        Tool::Wand => "wand",
        Tool::Move => "move",
        Tool::Pencil => "pencil",
        Tool::Eraser => "eraser",
        Tool::Spray => "spray",
        Tool::Fill => "fill",
        Tool::Gradient => "gradient",
        Tool::Line => "line",
        Tool::Rectangle => "rectangle",
        Tool::Ellipse => "ellipse",
        Tool::Curve => "curve",
        Tool::ShapePolygon => "shape_polygon",
        Tool::Contour => "contour",
        Tool::Text => "text",
        Tool::Picker => "picker",
        Tool::Slice => "slice",
        Tool::Blur => "blur",
        Tool::Jumble => "jumble",
    }
}

/// The `view.get` shape for tab `i`.
fn view(app: &PixmithApp, i: usize) -> Value {
    let tab = &app.tabs[i];
    let id = app.live.tracked(tab.doc.uid()).map_or(Value::Null, |d| string(d.id.as_str()));
    obj(vec![
        ("document", id),
        ("layer", num(tab.layer)),
        ("frame", num(tab.frame)),
        ("zoom", num(tab.zoom)),
        ("tool", string(tool_name(app.tool))),
    ])
}

fn set_view(app: &mut PixmithApp, i: usize, params: &Value) -> Reply {
    let (layer, frame) = (opt_int(params, "layer")?, opt_int(params, "frame")?);
    let tab = &mut app.tabs[i];
    if let Some(l) = layer {
        tab.layer = l.clamp(0, tab.doc.layer_count().saturating_sub(1) as i64) as u32;
    }
    if let Some(f) = frame {
        tab.frame = f.clamp(0, tab.doc.frame_count().saturating_sub(1) as i64) as u32;
        tab.playing = false;
    }
    Ok(view(app, i))
}
