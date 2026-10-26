//! Embedded Lua scripting.
//!
//! A script sees two globals, `sprite` and `app`, plus a small `color` helper table. The full
//! reference is in `docs/LUA_API.md`; this is the summary.
//!
//! Conventions: pixel coordinates and palette indices are 0-based; layers, frames, tags and
//! lists are 1-based (as usual in Lua). Colors are `Color` objects or integers `0xRRGGBBAA`
//! (Aseprite pixel values `0xAABBGGRR` where Aseprite uses them: `Color(n)`, `app.pixelColor`,
//! `Image` pixels; see `color.rs`). Handles
//! (`sprite.layers[i]`, `sprite.frames[i]`, `sprite.tags[i]`, `sprite:cel(l, f)`) refer to a
//! position in the stack; after a call that deletes or moves layers, frames or tags, handles
//! obtained earlier raise "stale handle" instead of silently pointing at something else.
//!
//! ```lua
//! sprite.width, sprite.height, sprite.layerCount, sprite.frameCount, sprite.colorMode
//! sprite.activeLayer, sprite.activeFrame      -- 1-based; the defaults for the pixel calls
//!
//! -- Pixels
//! sprite:getPixel(x, y [, layer [, frame]])   -- color, 0 outside the canvas
//! sprite:setPixel(x, y, color [, layer [, frame]])
//! sprite:getPixels(layer, frame [, x, y, w, h])           -- flat row-major table of colors
//! sprite:setPixels(layer, frame, pixels [, x, y, w, h])   -- same layout; at most 4194304 pixels; pixels may also be
//!                                                         -- an Image or a string of raw RGBA bytes (whole canvas)
//! sprite:fillSelection(color [, layer [, frame]]) -> count
//!
//! -- Layers
//! sprite.layers[i] -> { name, visible, opacity (0..255), blendMode (0..18), isGroup, isTilemap,
//!                       isReference, index, level, parent, children }
//! (also: isVisible (effective, with parents), tileset, userData, getTile / setTile on tilemap layers)
//! sprite:newLayer(name), sprite:newGroup(name), sprite:deleteLayer(i), sprite:moveLayer(i, to) -> index
//! sprite:moveLayerIntoGroup(i, group) -> index, sprite:moveLayerOutOfGroup(i) -> index
//!
//! -- Tween rig (layers with a tween parent; see docs/LUA_API.md)
//! layer.tweenParent (read/write layer or nil), layer:setTweenKey(prop, frame, value [, ease]),
//! layer:setTweenPivot(x, y | nil), layer:tweenTransform([frame]) -> { matrix, local, rotation, scaleX, scaleY, pivotX,
//! pivotY } or nil, layer:tweenMap([frame,] x, y) -> x, y
//!
//! -- Frames and tags
//! sprite.frames[i] -> { duration (ms), index }
//! sprite:newFrame([copy]), sprite:duplicateFrame(i), sprite:deleteFrame(i)
//! sprite.tags[i] -> { name, from, to, direction, repeatCount, color, index, delete() }
//! sprite:newTag(name, from, to), sprite:deleteTag(i)
//!
//! -- Tilemaps (tile indices are 0-based, 0 is the empty tile)
//! sprite.tilesets[i] -> { name, tileWidth, tileHeight, count, index, getTilePixels(t), setTilePixels(t, px),
//!                         addTile([px]), duplicateTile(t), deleteTile(t) }
//! layer:getTile(x, y [, frame]) -> index, flipX, flipY, diagonal
//! layer:setTile(x, y, index [, flipX, flipY, diagonal [, frame]])    -- x, y are grid cells
//!
//! -- Slices
//! sprite.slices[i] -> { name, index, hasCenter, hasPivot, getBounds([frame]), setBounds(x, y, w, h [, frame]),
//!                       getCenter / setCenter / clearCenter, getPivot / setPivot / clearPivot, delete() }
//! sprite:newSlice(rect | name, x, y, w, h), sprite:deleteSlice(i)    -- slice.bounds / center / pivot too
//!
//! -- User data: { text, color } on sprite.userData, layer.userData, cel.userData, tag.userData, slice.userData
//!
//! -- Cels
//! sprite:cel(layer, frame) -> { opacity, zIndex, isEmpty, linked, linkedFrames, unlink(), userData }
//! sprite:clearCel(layer, frame), sprite:deleteCel(cel | layer, frame)
//! sprite:newCel(layer, frame [, image [, position]]), layer:cel(frame), cel.image / cel.position / cel.bounds
//!
//! -- Geometry and images (Aseprite compatible; see `geometry.rs` and `image.rs`)
//! Point(x, y), Size(w, h), Rectangle(x, y, w, h)     -- every call taking a point or rectangle accepts these
//! Image(w, h [, colorMode]), Image{ fromFile = "/absolute/path.png" }, image.bytes (raw RGBA), image:drawImage ...
//!
//! -- Selection (scripts only look at it through these calls)
//! sprite.selection -> { isEmpty, bounds, contains(x, y), selectRect(x, y, w, h [, mode]),
//!                       selectAll(), deselect(), invert() }
//!
//! -- Palette
//! sprite.paletteSize, sprite:getPaletteColor(i), sprite:setPaletteColor(i, color), sprite:addPaletteColor(color)
//!
//! app.frameDiff{ frame, layer, wrap, mask } -> { changed, appeared, disappeared, total, bounds, ... } (docs/LUA_API.md)
//! app.alert(msg), app.undoLabel(text), app.blend.Multiply ..., app.direction.PingPong ..., print(...)
//! app.version, app.apiVersion, app.activeLayer / app.activeFrame (handles; assign to select them)
//! color.rgba(r, g, b [, a]), color.split(c) -> r, g, b, a
//!
//! -- Aseprite compatibility (one file each): color.rs Color / app.pixelColor, sprites.rs Sprite(...) / app.open /
//! -- app.sprites / saveAs / close, fs.rs io / os / app.fs, access.rs the permission questions, commands.rs
//! -- app.command / app.transaction / Version / enumerations, palettes.rs Palette
//! ```
//!
//! The whole run is one undo step (sprite and selection), and a script that fails leaves the
//! document untouched. In Indexed and Grayscale sprites every cel the script changed is snapped
//! to the color mode when the run ends. The sandbox has the base, table, string, utf8, math and
//! coroutine libraries, plus `io`, `os`, `dofile`, `loadfile` and `app.fs`, which ask the front end
//! before touching a path or running a program (docs/LUA_HOST_API.md); there is no `package`,
//! `debug` or `require`, and `load` accepts text only. An instruction budget, a wall-clock limit
//! (not counting time spent in a permission dialog) and a memory limit stop runaway scripts. A single pathological string pattern match is one VM instruction and cannot be
//! interrupted.

use crate::document::color_mode;
use crate::document::{Editor, Sprite};
use crate::paint::selection::Selection;
pub mod access;
mod color;
mod commands;
#[cfg(test)]
mod compat_tests;
mod dialog;
mod events;
mod framediff;
mod fs;
mod geometry;
pub mod host;
mod image;
mod palettes;
mod plugins;
#[cfg(test)]
mod session_tests;
mod sprites;

pub use sprites::{ActiveDoc, NewDocument};

use mlua::chunk::ChunkMode;
use mlua::{
    Error, Function, Lua, LuaOptions, MultiValue, Result as LuaResult, StdLib, Table, UserData, UserDataFields,
    UserDataMethods, Value, VmState,
};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ffi::{c_char, CStr, CString};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Most pixels `getPixels` / `setPixels` move in one call (2048 x 2048).
pub const MAX_BULK_PIXELS: usize = 1 << 22;

/// Resource limits of one script run.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub instructions: u64,
    pub time: Duration,
    pub memory: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { instructions: 300_000_000, time: Duration::from_secs(10), memory: 128 << 20 }
    }
}

impl Limits {
    /// Limits with these budgets (the defaults are 300 million instructions, 10 seconds, 128 MiB).
    pub const fn new(instructions: u64, time: Duration, memory: usize) -> Limits {
        Limits { instructions, time, memory }
    }

    /// The same limits with another wall-clock budget (the instruction budget still applies).
    pub const fn with_time(mut self, time: Duration) -> Limits {
        self.time = time;
        self
    }

    pub const fn with_instructions(mut self, instructions: u64) -> Limits {
        self.instructions = instructions;
        self
    }

    pub const fn with_memory(mut self, bytes: usize) -> Limits {
        self.memory = bytes;
        self
    }
}

/// Set by the UI (`pixmith_script_cancel`) to stop the script that is running on a background thread.
static CANCEL: AtomicBool = AtomicBool::new(false);
static NEVER: AtomicBool = AtomicBool::new(false);

/// Control character that starts the structured line `pixmith_run_script*` appends to its messages.
/// It is stripped from script output, so scripts cannot forge the line.
const MARKER: char = '\u{1}';

/// The last line of the messages returned through the C ABI when a script changed the active
/// layer or frame: `"\u{1}PIXMITH-SELECT layer=<n> frame=<n>"` with 1-based numbers.
pub const SELECT_LINE: &str = "\u{1}PIXMITH-SELECT";

/// Scripts may grow a sprite to at most this many layers and frames.
const MAX_SCRIPT_LAYERS: usize = 1024;
const MAX_SCRIPT_FRAMES: usize = 4096;
const MAX_SCRIPT_TAGS: usize = 4096;
const MAX_SCRIPT_SLICES: usize = 4096;
/// Most pixel data a script may put into one tileset (the Rust side is not covered by the Lua
/// memory limit).
const MAX_TILESET_BYTES: usize = 64 << 20;

/// What a script run produced.
#[derive(Debug)]
pub struct Outcome {
    /// `app.alert` and `print` output in order.
    pub messages: Vec<String>,
    /// The (layer, frame), 0-based, the script asked the editor to select via `app.activeLayer`
    /// or `app.activeFrame`.
    pub select: Option<(usize, usize)>,
    /// The error that stopped the script, if any (the document is then unchanged).
    pub error: Option<String>,
    /// Documents the script created (`Sprite(...)`) or opened (`app.open`), in creation order, closed ones
    /// removed; empty when the script failed. The front end opens them as tabs.
    pub new_documents: Vec<NewDocument>,
    /// Which document is active after the run (see `docs/LUA_HOST_API.md`).
    pub active: ActiveDoc,
    /// The script closed the document it ran on (`sprite:close()`); the front end closes the tab.
    pub close_original: bool,
    /// The script saved the document it ran on with `sprite:saveAs(path)` to a document format: its new file.
    pub original_path: Option<std::path::PathBuf>,
    /// ... and the document has not changed since, so the front end can treat it as saved.
    pub original_saved: bool,
}

impl Outcome {
    fn failed(messages: Vec<String>, error: String) -> Outcome {
        Outcome {
            messages,
            select: None,
            error: Some(error),
            new_documents: Vec::new(),
            active: ActiveDoc::Unchanged,
            close_original: false,
            original_path: None,
            original_saved: false,
        }
    }
}

/// Settings of one run besides the budgets.
#[derive(Clone, Debug)]
pub struct RunConfig {
    /// Names the script in permission requests; "always allow" is remembered per name.
    pub script_name: String,
    /// The file of the document the script runs on (`sprite.filename`).
    pub doc_path: Option<std::path::PathBuf>,
    /// Path prefixes this run may read and write without asking (besides `access::set_allowed_paths`).
    pub allowed_paths: Vec<std::path::PathBuf>,
}

impl Default for RunConfig {
    fn default() -> Self {
        RunConfig { script_name: "script".into(), doc_path: None, allowed_paths: Vec::new() }
    }
}

struct Ctx {
    ed: *mut Editor,
    messages: RefCell<Vec<String>>,
    label: RefCell<Option<String>>,
    layer: Cell<usize>,
    frame: Cell<usize>,
    /// Set when the script assigned `app.activeLayer` / `app.activeFrame`.
    selected: Cell<bool>,
    /// Bumped by every call that deletes or moves layers, frames or tags; handles remember the
    /// value they were created under.
    epoch: Cell<u64>,
    /// The script's working selection, written back (undoably) when the run succeeds.
    mask: RefCell<Option<Arc<Selection>>>,
    /// The session runtime (dialogs, events, plugin) when the script runs inside a front end.
    rt: RefCell<Option<Rc<host::Rt>>>,
    /// False in a session transaction that has no document (plugin `init`/`exit`).
    has_doc: Cell<bool>,
    /// Bytes held by free-standing script images (see `image.rs`); they live outside the Lua heap.
    image_bytes: Cell<usize>,
    /// For a document the script created or opened: the editor `ed` points into.
    #[allow(dead_code)]
    owned: Option<Box<Editor>>,
    /// State shared by all documents of the run (see `sprites.rs`).
    shared: Rc<sprites::Shared>,
    /// This document's file and open/closed state.
    doc: sprites::Doc,
    /// What `app.command.Undo` / `Redo` step through (see `commands.rs`).
    undo: commands::UndoLog,
}

impl Ctx {
    /// A context for a session worker; the editor is replaced by a snapshot at the start of each transaction.
    fn new(ed: &mut Editor) -> Ctx {
        Ctx {
            ed: ed as *mut Editor,
            owned: None,
            shared: sprites::Shared::new(),
            doc: sprites::Doc::new(true, None),
            undo: Default::default(),
            messages: RefCell::new(Vec::new()),
            label: RefCell::new(None),
            layer: Cell::new(0),
            frame: Cell::new(0),
            selected: Cell::new(false),
            epoch: Cell::new(0),
            mask: RefCell::new(None),
            rt: RefCell::new(None),
            has_doc: Cell::new(true),
            image_bytes: Cell::new(0),
        }
    }

    /// A document the script creates or opens: its own editor, sharing the run's state with `parent`.
    fn child(parent: &Rc<Ctx>, sprite: Sprite, path: Option<std::path::PathBuf>) -> Rc<Ctx> {
        let mut ed = Box::new(Editor::new(sprite));
        let ptr: *mut Editor = &mut *ed;
        let ctx = Rc::new(Ctx {
            ed: ptr,
            owned: Some(ed),
            shared: parent.shared.clone(),
            doc: sprites::Doc::new(false, path),
            undo: Default::default(),
            messages: RefCell::new(Vec::new()),
            label: RefCell::new(None),
            layer: Cell::new(0),
            frame: Cell::new(0),
            selected: Cell::new(false),
            epoch: Cell::new(0),
            mask: RefCell::new(None),
            rt: RefCell::new(parent.rt.borrow().clone()),
            has_doc: Cell::new(true),
            image_bytes: Cell::new(0),
        });
        ctx.undo.reset(ctx.sprite().clone(), None);
        ctx
    }

    /// Time spent waiting for the front end (a permission dialog) does not count toward the time limit.
    fn add_wait(&self, d: Duration) {
        self.shared.waited.set(self.shared.waited.get() + d);
        if let Some(rt) = self.rt.borrow().as_ref() {
            rt.add_wait(d);
        }
    }

    #[allow(clippy::mut_from_ref)]
    fn editor(&self) -> &mut Editor {
        // SAFETY: as for `sprite`: the pointer is valid while the context lives.
        unsafe { &mut *self.ed }
    }

    #[allow(clippy::mut_from_ref)]
    fn sprite(&self) -> &mut Sprite {
        // The pointer is valid for the whole run: `run` owns the Lua state and drops it before
        // the editor is used again, and Lua is single-threaded so calls never overlap.
        unsafe { &mut (*self.ed).sprite }
    }

    fn layer_index(&self, v: Option<i64>) -> LuaResult<usize> {
        let n = self.sprite().layers.len();
        match v {
            None => Ok(self.layer.get().min(n - 1)),
            Some(i) if i >= 1 && i as usize <= n => Ok(i as usize - 1),
            Some(i) => Err(Error::runtime(format!("layer {i} does not exist (1..{n})"))),
        }
    }

    fn frame_index(&self, v: Option<i64>) -> LuaResult<usize> {
        let n = self.sprite().frame_count();
        match v {
            None => Ok(self.frame.get().min(n - 1)),
            Some(i) if i >= 1 && i as usize <= n => Ok(i as usize - 1),
            Some(i) => Err(Error::runtime(format!("frame {i} does not exist (1..{n})"))),
        }
    }

    /// A layer given as a 1-based number or as a layer handle (0-based result).
    fn layer_of(&self, v: &Value) -> LuaResult<usize> {
        if let Value::UserData(u) = v {
            let h = u.borrow::<LayerH>().map_err(|_| Error::runtime("expected a layer or a layer number"))?;
            h.get()?;
            return Ok(h.1);
        }
        if let Value::String(name) = v {
            let name = name.to_string_lossy();
            return self
                .sprite()
                .layers
                .iter()
                .position(|l| l.name == name)
                .ok_or_else(|| Error::runtime(format!("no layer is named \"{name}\"")));
        }
        match geometry::int_of(v) {
            Some(i) => self.layer_index(Some(i)),
            None => Err(Error::runtime("expected a layer, a layer number or a layer name")),
        }
    }

    /// A frame given as a 1-based number or as a frame handle (0-based result).
    fn frame_of(&self, v: &Value) -> LuaResult<usize> {
        if let Value::UserData(u) = v {
            let h = u.borrow::<FrameH>().map_err(|_| Error::runtime("expected a frame or a frame number"))?;
            return h.check();
        }
        match geometry::int_of(v) {
            Some(i) => self.frame_index(Some(i)),
            None => Err(Error::runtime("expected a frame or a frame number")),
        }
    }

    fn tag_index(&self, v: i64) -> LuaResult<usize> {
        let n = self.sprite().tags.len();
        if v >= 1 && v as usize <= n {
            Ok(v as usize - 1)
        } else {
            Err(Error::runtime(format!("tag {v} does not exist (1..{n})")))
        }
    }

    /// A drawable (non-group) layer.
    fn drawable_layer(&self, v: Option<i64>) -> LuaResult<usize> {
        let l = self.layer_index(v)?;
        if self.sprite().layers[l].is_group() {
            return Err(Error::runtime("groups have no pixels"));
        }
        Ok(l)
    }

    /// The drawing colors (`app.fgColor`, `app.bgColor`) as the front end last reported them.
    fn colors(&self) -> (u32, u32) {
        self.rt.borrow().as_ref().map_or((0x0000_00FF, 0xFFFF_FFFF), |r| r.colors())
    }

    fn invalidate_handles(&self) {
        self.epoch.set(self.epoch.get() + 1);
    }

    fn check(&self, epoch: u64, what: &str) -> LuaResult<()> {
        if epoch == self.epoch.get() {
            Ok(())
        } else {
            Err(Error::runtime(format!(
                "stale {what} handle: layers, frames or tags were deleted or moved since it was obtained; fetch it again"
            )))
        }
    }

    fn mask(&self) -> Option<Arc<Selection>> {
        self.mask.borrow().clone()
    }

    fn set_mask(&self, m: Selection) {
        *self.mask.borrow_mut() = crate::paint::selection::nonempty(m);
    }
}

fn color_arg(v: i64) -> LuaResult<[u8; 4]> {
    u32::try_from(v).map(u32::to_be_bytes).map_err(|_| Error::runtime(format!("color {v} is not a 0xRRGGBBAA value")))
}

fn pack(c: &[u8]) -> i64 {
    u32::from_be_bytes([c[0], c[1], c[2], c[3]]) as i64
}

/// Index of the last layer at nesting level 0: new layers go above it, never inside a group.
fn top_level_layer(s: &Sprite) -> usize {
    s.layers.iter().rposition(|l| l.child_level == 0).unwrap_or(s.layers.len() - 1)
}

/// Clamped canvas rectangle from optional x, y, w, h arguments (default: the whole canvas).
fn rect_args(
    s: &Sprite,
    r: (Option<i64>, Option<i64>, Option<i64>, Option<i64>),
) -> LuaResult<(usize, usize, usize, usize)> {
    rect_args_cap(s, r, MAX_BULK_PIXELS)
}

/// `rect_args` with a different pixel cap (buffers and images may fill the whole canvas).
fn rect_args_cap(
    s: &Sprite,
    r: (Option<i64>, Option<i64>, Option<i64>, Option<i64>),
    cap: usize,
) -> LuaResult<(usize, usize, usize, usize)> {
    let (cw, ch) = (s.width as i64, s.height as i64);
    let (x, y) = (r.0.unwrap_or(0), r.1.unwrap_or(0));
    let (w, h) = (r.2.unwrap_or(cw - x), r.3.unwrap_or(ch - y));
    if w < 0 || h < 0 || x < 0 || y < 0 || x + w > cw || y + h > ch {
        return Err(Error::runtime(format!("rectangle {x},{y} {w}x{h} does not fit the {cw}x{ch} canvas")));
    }
    if (w as usize).saturating_mul(h as usize) > cap {
        return Err(Error::runtime(format!("at most {cap} pixels per call")));
    }
    Ok((x as usize, y as usize, w as usize, h as usize))
}

/// An optional layer/frame argument (number or handle) taken off `a`; the active one when absent (or nil).
/// With `required`, a missing argument is an error instead.
fn take_layer(ctx: &Ctx, a: &mut MultiValue, required: bool) -> LuaResult<usize> {
    match a.pop_front() {
        None if required => Err(Error::runtime("a layer is required")),
        None | Some(Value::Nil) => ctx.layer_index(None),
        Some(v) => ctx.layer_of(&v),
    }
}

fn take_frame_idx(ctx: &Ctx, a: &mut MultiValue, required: bool) -> LuaResult<usize> {
    match a.pop_front() {
        None if required => Err(Error::runtime("a frame is required")),
        None | Some(Value::Nil) => ctx.frame_index(None),
        Some(v) => ctx.frame_of(&v),
    }
}

/// `[layer [, frame]]`, both optional.
fn take_site(ctx: &Ctx, a: &mut MultiValue) -> LuaResult<(usize, usize)> {
    Ok((take_layer(ctx, a, false)?, take_frame_idx(ctx, a, false)?))
}

/// `[x, y, w, h]` (each optional) or one Rectangle, as the optional bounds of the bulk pixel calls.
fn take_rect_opts(a: &mut MultiValue) -> LuaResult<(Option<i64>, Option<i64>, Option<i64>, Option<i64>)> {
    match a.front() {
        None | Some(Value::Nil) => Ok((None, None, None, None)),
        Some(Value::Integer(_) | Value::Number(_)) => {
            let mut v = [None; 4];
            for slot in v.iter_mut() {
                *slot = geometry::take_opt_int(a, "rectangle")?;
            }
            Ok((v[0], v[1], v[2], v[3]))
        }
        Some(_) => {
            let r = geometry::take_rect(a, "rectangle")?;
            Ok((Some(r.x), Some(r.y), Some(r.w), Some(r.h)))
        }
    }
}

/// Maps colors to the sprite's color mode, remembering earlier answers.
struct Snap<'a> {
    s: &'a Sprite,
    cache: std::collections::HashMap<[u8; 4], [u8; 4]>,
}

impl<'a> Snap<'a> {
    fn new(s: &'a Sprite) -> Self {
        Snap { s, cache: Default::default() }
    }

    fn get(&mut self, c: [u8; 4]) -> [u8; 4] {
        if self.s.color_mode == color_mode::ColorMode::Rgba {
            return c;
        }
        let s = self.s;
        *self.cache.entry(c).or_insert_with(|| color_mode::snap_color(s, c))
    }
}

/// Writes colors into a rectangle of a cel (replacing what was there), creating the cel if
/// needed, and keeps tilemap layers in sync with their tiles.
fn write_rect(
    s: &mut Sprite,
    layer: usize,
    frame: usize,
    rect: (usize, usize, usize, usize),
    colors: &mut dyn Iterator<Item = [u8; 4]>,
) {
    let (x0, y0, w, h) = rect;
    if w == 0 || h == 0 {
        return;
    }
    let cw = s.width as usize;
    let snapped: Vec<[u8; 4]> = {
        let mut snap = Snap::new(s);
        colors.map(|c| snap.get(c)).collect()
    };
    let Some(cel) = s.cel_mut(layer, frame) else { return };
    for (i, c) in snapped.iter().enumerate().take(w * h) {
        let (x, y) = (x0 + i % w, y0 + i / w);
        cel.pixels[(y * cw + x) * 4..][..4].copy_from_slice(c);
    }
    if s.layers[layer].kind == crate::document::tilemap::KIND_TILEMAP {
        let bb = (x0 as i32, y0 as i32, (x0 + w) as i32, (y0 + h) as i32);
        crate::document::tilemap::sync_cel(s, layer, frame, Some(bb));
    }
    s.flush_links();
}

struct SpriteH(Rc<Ctx>);
struct LayerH(Rc<Ctx>, usize, u64);
struct FrameH(Rc<Ctx>, usize, u64);
struct TagH(Rc<Ctx>, usize, u64);
struct CelH(Rc<Ctx>, usize, usize, u64);
struct SelectionH(Rc<Ctx>);

impl LayerH {
    fn new(c: &Rc<Ctx>, i: usize) -> Self {
        LayerH(c.clone(), i, c.epoch.get())
    }

    fn get(&self) -> LuaResult<&mut crate::document::Layer> {
        self.0.check(self.2, "layer")?;
        self.0.sprite().layers.get_mut(self.1).ok_or_else(|| Error::runtime("layer does not exist"))
    }
}

impl FrameH {
    fn new(c: &Rc<Ctx>, i: usize) -> Self {
        FrameH(c.clone(), i, c.epoch.get())
    }

    fn check(&self) -> LuaResult<usize> {
        self.0.check(self.2, "frame")?;
        if self.1 < self.0.sprite().frame_count() {
            Ok(self.1)
        } else {
            Err(Error::runtime("frame does not exist"))
        }
    }
}

impl TagH {
    fn new(c: &Rc<Ctx>, i: usize) -> Self {
        TagH(c.clone(), i, c.epoch.get())
    }

    fn get(&self) -> LuaResult<&mut crate::document::Tag> {
        self.0.check(self.2, "tag")?;
        self.0.sprite().tags.get_mut(self.1).ok_or_else(|| Error::runtime("tag does not exist"))
    }
}

impl UserData for SpriteH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("width", |_, t| Ok(t.0.sprite().width));
        f.add_field_method_get("height", |_, t| Ok(t.0.sprite().height));
        f.add_field_method_get("layerCount", |_, t| Ok(t.0.sprite().layers.len()));
        f.add_field_method_get("frameCount", |_, t| Ok(t.0.sprite().frame_count()));
        f.add_field_method_get("paletteSize", |_, t| Ok(t.0.sprite().palette.len()));
        f.add_field_method_get("activeLayer", |_, t| Ok(t.0.layer.get() + 1));
        f.add_field_method_get("activeFrame", |_, t| Ok(t.0.frame.get() + 1));
        sprites::add_fields(f);
        palettes::add_fields(f);
        f.add_field_method_get("layers", |lua, t| {
            let n = t.0.sprite().layers.len();
            lua.create_sequence_from((0..n).map(|i| LayerH::new(&t.0, i)))
        });
        f.add_field_method_get("frames", |lua, t| {
            let n = t.0.sprite().frame_count();
            lua.create_sequence_from((0..n).map(|i| FrameH::new(&t.0, i)))
        });
        f.add_field_method_get("tags", |lua, t| {
            let n = t.0.sprite().tags.len();
            lua.create_sequence_from((0..n).map(|i| TagH::new(&t.0, i)))
        });
        f.add_field_method_get("selection", |_, t| Ok(SelectionH(t.0.clone())));
        f.add_field_method_get("events", |_, t| Ok(events::EventsH { ctx: t.0.clone(), scope: "sprite" }));
        f.add_field_method_get("tilesets", |lua, t| {
            let n = t.0.sprite().tilesets.len();
            lua.create_sequence_from((0..n).map(|i| TilesetH(t.0.clone(), i)))
        });
        f.add_field_method_get("slices", |lua, t| {
            let n = t.0.sprite().extra.slices.len();
            lua.create_sequence_from((0..n).map(|i| SliceH::new(&t.0, i)))
        });
        f.add_field_method_get("userData", |_, t| Ok(UserDataH::new(&t.0, 0, 0, 0)));
        f.add_field_method_get("bounds", |_, t| {
            let s = t.0.sprite();
            Ok(geometry::RectV::new(0, 0, s.width as i64, s.height as i64))
        });
        f.add_field_method_get("size", |_, t| {
            let s = t.0.sprite();
            Ok(geometry::SizeV { w: s.width as i64, h: s.height as i64 })
        });
        // Every cel that has an image, frame by frame, bottom layer first within a frame.
        f.add_field_method_get("cels", |lua, t| {
            let s = t.0.sprite();
            let mut out = Vec::new();
            for f in 0..s.frame_count() {
                for l in 0..s.layers.len() {
                    if s.cels[f][l].is_some() {
                        out.push(CelH(t.0.clone(), l, f, t.0.epoch.get()));
                    }
                }
            }
            lua.create_sequence_from(out)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        sprites::add_methods(m);
        commands::add_methods(m);
        palettes::add_methods(m);
        // MARK: Pixels
        // getPixel(x, y [, layer [, frame]]) or getPixel(point [, layer [, frame]])
        m.add_method("getPixel", |_, t, mut a: MultiValue| {
            let p = geometry::take_point(&mut a, "getPixel")?;
            let (l, f) = take_site(&t.0, &mut a)?;
            let s = t.0.sprite();
            if p.x < 0 || p.y < 0 || p.x >= s.width as i64 || p.y >= s.height as i64 {
                return Ok(0i64);
            }
            let i = (p.y as usize * s.width as usize + p.x as usize) * 4;
            Ok(s.cels[f][l].as_ref().map_or(0, |c| pack(&c.pixels[i..i + 4])))
        });
        // setPixel(x, y, color [, layer [, frame]]) or setPixel(point, color [, layer [, frame]])
        m.add_method("setPixel", |_, t, mut a: MultiValue| {
            let p = geometry::take_point(&mut a, "setPixel")?;
            let c = color::arg(&a.pop_front().unwrap_or(Value::Nil))
                .map_err(|_| Error::runtime("setPixel needs a color (a Color or a 0xRRGGBBAA number)"))?;
            let (l, f) = take_site(&t.0, &mut a)?;
            let s = t.0.sprite();
            if s.layers[l].is_group() {
                return Err(Error::runtime("cannot draw on a group layer"));
            }
            if p.x < 0 || p.y < 0 || p.x >= s.width as i64 || p.y >= s.height as i64 {
                return Ok(());
            }
            let c = color_mode::snap_color(s, c);
            s.draw_line(l, f, p.x as i32, p.y as i32, p.x as i32, p.y as i32, c, 1);
            s.flush_links(); // an edit shows in every frame that shares the cel right away
            Ok(())
        });
        // getPixels(layer, frame [, x, y, w, h]) or getPixels(layer, frame, rectangle)
        m.add_method("getPixels", |lua, t, mut a: MultiValue| {
            let (l, f) = (take_layer(&t.0, &mut a, true)?, take_frame_idx(&t.0, &mut a, true)?);
            let s = t.0.sprite();
            let (x0, y0, w, h) = rect_args(s, take_rect_opts(&mut a)?)?;
            let cw = s.width as usize;
            let cel = s.cels[f][l].as_ref();
            lua.create_sequence_from((0..w * h).map(|i| {
                cel.map_or(0, |c| {
                    let (px, py) = (x0 + i % w, y0 + i / w);
                    pack(&c.pixels[(py * cw + px) * 4..][..4])
                })
            }))
        });
        // setPixels(layer, frame, pixels [, x, y, w, h]) or setPixels(layer, frame, pixels, rectangle).
        // pixels: a flat table of colors (at most MAX_BULK_PIXELS), or an Image / a string of raw RGBA bytes
        // (row-major, 4 bytes per pixel; up to the whole canvas), written in Rust.
        m.add_method("setPixels", |_, t, mut a: MultiValue| {
            let layer = take_layer(&t.0, &mut a, true)?;
            if t.0.sprite().layers[layer].is_group() {
                return Err(Error::runtime("groups have no pixels"));
            }
            let f = take_frame_idx(&t.0, &mut a, true)?;
            let pixels = a.pop_front().unwrap_or(Value::Nil);
            let s = t.0.sprite();
            let bulk = match &pixels {
                Value::String(b) => Some((b.as_bytes().to_vec(), None)),
                other => image::image_of(other).map(|img| (Vec::new(), Some(img))),
            };
            let (x, y, w, h) = take_rect_opts(&mut a)?;
            if let Some((bytes, img)) = bulk {
                let (data, iw, ih) = match img {
                    Some(img) => {
                        let (iw, ih) = img.size()?;
                        (img.snapshot()?, Some(iw), Some(ih))
                    }
                    None => (bytes, None, None),
                };
                let (w, h) = (w.or(iw.map(|v| v as i64)), h.or(ih.map(|v| v as i64)));
                let rect = rect_args_cap(s, (x, y, w, h), usize::MAX)?;
                if data.len() != rect.2 * rect.3 * 4 || iw.is_some_and(|v| v != rect.2) {
                    return Err(Error::runtime(format!(
                        "setPixels needs {} bytes (RGBA) for a {}x{} rectangle, got {}",
                        rect.2 * rect.3 * 4,
                        rect.2,
                        rect.3,
                        data.len()
                    )));
                }
                if rect.2 > 0 && rect.3 > 0 {
                    image::blit(s, layer, f, &data, rect.2, rect.3, rect.0 as i64, rect.1 as i64)?;
                }
                return Ok(());
            }
            let Value::Table(pixels) = pixels else {
                return Err(Error::runtime(
                    "setPixels: pixels must be a table of colors, an Image or a string of RGBA bytes",
                ));
            };
            let rect = rect_args(s, (x, y, w, h))?;
            let count = pixels.raw_len();
            if count != rect.2 * rect.3 {
                return Err(Error::runtime(format!(
                    "setPixels needs {} colors for a {}x{} rectangle, got {count}",
                    rect.2 * rect.3,
                    rect.2,
                    rect.3
                )));
            }
            let mut colors = Vec::with_capacity(count);
            for v in pixels.sequence_values::<color::PxColor>() {
                colors.push(v?.0);
            }
            write_rect(s, layer, f, rect, &mut colors.into_iter());
            Ok(())
        });
        m.add_method("fillSelection", |_, t, (color, layer, frame): (color::PxColor, Option<i64>, Option<i64>)| {
            let c = color.0;
            let l = t.0.drawable_layer(layer)?;
            let f = t.0.frame_index(frame)?;
            let Some(mask) = t.0.mask() else { return Ok(0usize) };
            let s = t.0.sprite();
            if (mask.w, mask.h) != (s.width, s.height) {
                return Ok(0);
            }
            let c = color_mode::snap_color(s, c);
            let (cw, ch) = (s.width as usize, s.height as usize);
            let Some(cel) = s.cel_mut(l, f) else { return Ok(0) };
            let mut n = 0;
            for (i, on) in mask.bits.iter().enumerate() {
                if *on {
                    cel.pixels[i * 4..i * 4 + 4].copy_from_slice(&c);
                    n += 1;
                }
            }
            debug_assert!(cel.pixels.len() == cw * ch * 4);
            if s.layers[l].kind == crate::document::tilemap::KIND_TILEMAP {
                crate::document::tilemap::sync_cel(s, l, f, None);
            }
            s.flush_links();
            Ok(n)
        });

        // MARK: Layers
        m.add_method("newLayer", |_, t, name: Option<String>| {
            let s = t.0.sprite();
            if s.layers.len() >= MAX_SCRIPT_LAYERS {
                return Err(Error::runtime(format!("too many layers (at most {MAX_SCRIPT_LAYERS})")));
            }
            let i = s.add_layer(top_level_layer(s));
            if let Some(n) = name {
                s.layers[i].name = n;
            }
            Ok(LayerH::new(&t.0, i))
        });
        m.add_method("newGroup", |_, t, name: Option<String>| {
            let s = t.0.sprite();
            if s.layers.len() >= MAX_SCRIPT_LAYERS {
                return Err(Error::runtime(format!("too many layers (at most {MAX_SCRIPT_LAYERS})")));
            }
            let i = s.add_group(top_level_layer(s));
            if let Some(n) = name {
                s.layers[i].name = n;
            }
            Ok(LayerH::new(&t.0, i))
        });
        m.add_method("deleteLayer", |_, t, v: Value| {
            let l = t.0.layer_of(&v)?;
            if !t.0.sprite().remove_layer(l) {
                return Err(Error::runtime("the last drawable layer cannot be deleted"));
            }
            t.0.invalidate_handles();
            Ok(())
        });
        m.add_method("moveLayerIntoGroup", |_, t, (i, group): (i64, i64)| {
            let (l, g) = (t.0.layer_index(Some(i))?, t.0.layer_index(Some(group))?);
            let n = t.0.sprite().move_into_group(l, g).ok_or_else(|| {
                Error::runtime("the layer cannot move into that group (it must be a group, not the layer itself or inside it, and not already the parent)")
            })?;
            t.0.invalidate_handles();
            Ok(n + 1)
        });
        m.add_method("moveLayerOutOfGroup", |_, t, i: i64| {
            let l = t.0.layer_index(Some(i))?;
            let n =
                t.0.sprite().move_out_of_group(l).ok_or_else(|| Error::runtime("the layer is not inside a group"))?;
            t.0.invalidate_handles();
            Ok(n + 1)
        });
        m.add_method("moveLayer", |_, t, (i, to): (i64, i64)| {
            let mut cur = t.0.layer_index(Some(i))?;
            let target = t.0.layer_index(Some(to))?;
            let s = t.0.sprite();
            let up = target > cur;
            for _ in 0..s.layers.len() * 2 {
                if cur == target {
                    break;
                }
                // One step at a time, like dragging it through the layer list.
                let Some(n) = s.move_layer(cur, up) else { break };
                if n == cur {
                    break;
                }
                let passed = if up { n > target } else { n < target };
                cur = n;
                if passed {
                    break;
                }
            }
            t.0.invalidate_handles();
            Ok(cur + 1)
        });

        // MARK: Frames
        // newFrame([copy]) appends an empty frame (a copy of the last one with `true`); newFrame(frame) inserts a
        // copy of that frame after it, as in Aseprite.
        m.add_method("newFrame", |_, t, arg: Option<Value>| {
            let s = t.0.sprite();
            if s.frame_count() >= MAX_SCRIPT_FRAMES || !s.cels_fit_budget(s.width as u64, s.height as u64) {
                return Err(Error::runtime(format!(
                    "too many frames (at most {MAX_SCRIPT_FRAMES}) or not enough memory"
                )));
            }
            let (after, copy) = match arg {
                None | Some(Value::Nil) => (s.frame_count() - 1, false),
                Some(Value::Boolean(b)) => (s.frame_count() - 1, b),
                Some(v) => (t.0.frame_of(&v)?, true),
            };
            let i = s.add_frame(after, copy);
            Ok(FrameH::new(&t.0, i))
        });
        // newEmptyFrame([frameNumber]): an empty frame inserted at that position (default: the end).
        m.add_method("newEmptyFrame", |_, t, arg: Option<Value>| {
            let s = t.0.sprite();
            if s.frame_count() >= MAX_SCRIPT_FRAMES || !s.cels_fit_budget(s.width as u64, s.height as u64) {
                return Err(Error::runtime(format!(
                    "too many frames (at most {MAX_SCRIPT_FRAMES}) or not enough memory"
                )));
            }
            let at = match arg {
                None | Some(Value::Nil) => s.frame_count(),
                Some(v) => t.0.frame_of(&v)?,
            };
            let i = if at == 0 {
                // Before the first frame: insert after it and swap in place by moving it to the front.
                let n = s.add_frame(0, false);
                s.move_frame(n, 0);
                0
            } else {
                s.add_frame(at - 1, false)
            };
            t.0.invalidate_handles();
            Ok(FrameH::new(&t.0, i))
        });
        m.add_method("duplicateFrame", |_, t, i: i64| {
            let f = t.0.frame_index(Some(i))?;
            let n = t.0.sprite().add_frame(f, true);
            t.0.invalidate_handles();
            Ok(FrameH::new(&t.0, n))
        });
        m.add_method("deleteFrame", |_, t, v: Value| {
            let f = t.0.frame_of(&v)?;
            if !t.0.sprite().remove_frame(f) {
                return Err(Error::runtime("the last frame cannot be deleted"));
            }
            t.0.invalidate_handles();
            Ok(())
        });

        // MARK: Tags
        m.add_method("newTag", |_, t, (name, from, to): (String, i64, i64)| {
            let (a, b) = (t.0.frame_index(Some(from))?, t.0.frame_index(Some(to))?);
            let s = t.0.sprite();
            if s.tags.len() >= MAX_SCRIPT_TAGS {
                return Err(Error::runtime(format!("too many tags (at most {MAX_SCRIPT_TAGS})")));
            }
            let i = s.add_tag(a, b);
            s.tags[i].name = name;
            Ok(TagH::new(&t.0, i))
        });
        // newSlice(rectangle), newSlice(x, y, w, h) as in Aseprite (the slice gets the name "Slice N"), or
        // newSlice(name [, rectangle]) / newSlice(name, x, y, w, h). Without bounds the slice covers the canvas.
        m.add_method("newSlice", |_, t, mut a: MultiValue| {
            let name = match a.front() {
                Some(Value::String(n)) => {
                    let n = n.to_string_lossy();
                    a.pop_front();
                    Some(n)
                }
                _ => None,
            };
            let s = t.0.sprite();
            let r = if a.is_empty() || matches!(a.front(), Some(Value::Nil)) {
                geometry::RectV::new(0, 0, s.width as i64, s.height as i64)
            } else {
                geometry::take_rect(&mut a, "newSlice")?
            };
            let r = slice_rect(r.x, r.y, r.w, r.h)?;
            if s.extra.slices.len() >= MAX_SCRIPT_SLICES {
                return Err(Error::runtime(format!("too many slices (at most {MAX_SCRIPT_SLICES})")));
            }
            let name = name.unwrap_or_else(|| {
                let mut n = s.extra.slices.len() + 1;
                while s.extra.slices.iter().any(|x| x.name == format!("Slice {n}")) {
                    n += 1;
                }
                format!("Slice {n}")
            });
            let key = crate::document::slices::SliceKey { bounds: r, ..Default::default() };
            s.extra.slices.push(crate::document::slices::Slice { name, keys: vec![key], ..Default::default() });
            Ok(SliceH::new(&t.0, s.extra.slices.len() - 1))
        });
        m.add_method("deleteSlice", |_, t, v: Value| {
            let n = t.0.sprite().extra.slices.len();
            let i = match &v {
                Value::UserData(u) => {
                    let h = u
                        .borrow::<SliceH>()
                        .map_err(|_| Error::runtime("expected a slice, a slice number or a slice name"))?;
                    h.1
                }
                Value::String(name) => {
                    let name = name.to_string_lossy();
                    t.0.sprite()
                        .extra
                        .slices
                        .iter()
                        .position(|x| x.name == name)
                        .ok_or_else(|| Error::runtime(format!("no slice is named \"{name}\"")))?
                }
                other => {
                    let i = geometry::int_of(other)
                        .ok_or_else(|| Error::runtime("expected a slice, a slice number or a slice name"))?;
                    usize::try_from(i - 1)
                        .ok()
                        .filter(|&i| i < n)
                        .ok_or_else(|| Error::runtime(format!("slice {i} does not exist (1..{n})")))?
                }
            };
            if i >= n {
                return Err(Error::runtime("slice does not exist"));
            }
            t.0.sprite().extra.slices.remove(i);
            t.0.invalidate_handles();
            Ok(())
        });
        m.add_method("deleteTag", |_, t, v: Value| {
            let i = match &v {
                Value::UserData(u) => {
                    let h =
                        u.borrow::<TagH>().map_err(|_| Error::runtime("expected a tag, a tag number or a tag name"))?;
                    h.get()?;
                    h.1
                }
                Value::String(name) => {
                    let name = name.to_string_lossy();
                    t.0.sprite()
                        .tags
                        .iter()
                        .position(|x| x.name == name)
                        .ok_or_else(|| Error::runtime(format!("no tag is named \"{name}\"")))?
                }
                other => t.0.tag_index(
                    geometry::int_of(other)
                        .ok_or_else(|| Error::runtime("expected a tag, a tag number or a tag name"))?,
                )?,
            };
            t.0.sprite().remove_tag(i);
            t.0.invalidate_handles();
            Ok(())
        });

        // MARK: Cels
        m.add_method("cel", |_, t, (layer, frame): (Value, Value)| {
            let (l, f) = (t.0.layer_of(&layer)?, t.0.frame_of(&frame)?);
            if t.0.sprite().layers[l].is_group() {
                return Err(Error::runtime("groups have no cels"));
            }
            Ok(CelH(t.0.clone(), l, f, t.0.epoch.get()))
        });
        m.add_method("clearCel", |_, t, (layer, frame): (Value, Value)| {
            let l = t.0.layer_of(&layer)?;
            if t.0.sprite().layers[l].is_group() {
                return Err(Error::runtime("groups have no pixels"));
            }
            let f = t.0.frame_of(&frame)?;
            Ok(t.0.sprite().clear_cels((l, l), (f, f)))
        });
        // deleteCel(cel) or deleteCel(layer, frame), as in Aseprite.
        m.add_method("deleteCel", |_, t, (layer, frame): (Value, Option<Value>)| {
            let (l, f) = match (&layer, frame) {
                (Value::UserData(u), None) => {
                    let c =
                        u.borrow::<CelH>().map_err(|_| Error::runtime("deleteCel expects a cel or (layer, frame)"))?;
                    c.check()?;
                    (c.1, c.2)
                }
                (_, Some(frame)) => (t.0.layer_of(&layer)?, t.0.frame_of(&frame)?),
                _ => return Err(Error::runtime("deleteCel expects a cel or (layer, frame)")),
            };
            if t.0.sprite().layers[l].is_group() {
                return Err(Error::runtime("groups have no cels"));
            }
            t.0.sprite().clear_cels((l, l), (f, f));
            Ok(())
        });
        // newCel(layer, frame [, image [, position]]): puts the image (RGBA, copied; kept whole even beyond the canvas) into
        // the cel with its top-left at `position` (default 0,0), replacing the cel, in one call. Without an
        // image the cel is empty. `layer` and `frame` are numbers or handles.
        m.add_method("newCel", |_, t, (layer, frame, img, pos): (Value, Value, Option<Value>, Option<Value>)| {
            let l = t.0.layer_of(&layer)?;
            if t.0.sprite().layers[l].is_group() {
                return Err(Error::runtime("groups have no cels"));
            }
            let f = t.0.frame_of(&frame)?;
            let img = match img {
                None | Some(Value::Nil) => None,
                Some(v) => Some(
                    image::image_of(&v).ok_or_else(|| Error::runtime("newCel: the third argument must be an Image"))?,
                ),
            };
            let at = match pos {
                None | Some(Value::Nil) => (0, 0),
                Some(v) => {
                    geometry::point_of(&v).ok_or_else(|| Error::runtime("newCel: the position must be a Point"))?
                }
            };
            image::place_cel(&t.0, l, f, img.as_ref(), at)?;
            Ok(CelH(t.0.clone(), l, f, t.0.epoch.get()))
        });

        // MARK: Palette
        m.add_method("getPaletteColor", |_, t, i: i64| {
            let s = t.0.sprite();
            s.palette
                .get(usize::try_from(i).unwrap_or(usize::MAX))
                .map(|c| u32::from_be_bytes(*c) as i64)
                .ok_or_else(|| Error::runtime(format!("palette index {i} does not exist (0..{})", s.palette.len())))
        });
        m.add_method("setPaletteColor", |_, t, (i, color): (i64, color::PxColor)| {
            let c = color.0;
            let s = t.0.sprite();
            let idx = usize::try_from(i).unwrap_or(usize::MAX);
            if idx == s.palette.len() && idx < 256 {
                s.palette.push(c);
                Ok(())
            } else if color_mode::set_palette_entry(s, idx, c) {
                Ok(())
            } else {
                Err(Error::runtime(format!("palette index {i} does not exist (0..{})", s.palette.len())))
            }
        });
        m.add_method("addPaletteColor", |_, t, color: color::PxColor| {
            let c = color.0;
            let s = t.0.sprite();
            if let Some(i) = s.palette.iter().position(|&p| p == c) {
                return Ok(i);
            }
            if s.palette.len() >= 256 {
                return Err(Error::runtime("the palette is full"));
            }
            s.palette.push(c);
            Ok(s.palette.len() - 1)
        });
    }
}

impl UserData for LayerH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("cels", |lua, t| {
            t.get()?;
            let s = t.0.sprite();
            let out: Vec<CelH> = (0..s.frame_count())
                .filter(|&f| s.cels[f][t.1].is_some())
                .map(|f| CelH(t.0.clone(), t.1, f, t.0.epoch.get()))
                .collect();
            lua.create_sequence_from(out)
        });

        f.add_field_method_get("name", |_, t| Ok(t.get()?.name.clone()));
        f.add_field_method_set("name", |_, t, v: String| {
            t.get()?.name = v;
            Ok(())
        });
        f.add_field_method_get("visible", |_, t| Ok(t.get()?.visible));
        f.add_field_method_set("visible", |_, t, v: bool| {
            t.get()?.visible = v;
            Ok(())
        });
        f.add_field_method_get("opacity", |_, t| Ok(t.get()?.opacity));
        f.add_field_method_set("opacity", |_, t, v: Value| {
            let v = whole("layer.opacity", &v)?;
            t.get()?.opacity = v.clamp(0, 255) as u8;
            Ok(())
        });
        // Aseprite's BlendMode numbering (see `commands.rs`).
        f.add_field_method_get("blendMode", |_, t| Ok(commands::blend_to_aseprite(t.get()?.blend_mode)));
        f.add_field_method_set("blendMode", |_, t, v: Value| {
            let v = whole("layer.blendMode", &v)?;
            t.get()?.blend_mode = commands::blend_from_aseprite(v)?;
            Ok(())
        });
        f.add_field_method_get("stackIndex", |_, t| {
            t.get()?;
            Ok(t.1 + 1)
        });
        f.add_field_method_get("isImage", |_, t| Ok(t.get()?.kind == 0));
        f.add_field_method_get("isBackground", |_, t| Ok(t.get()?.flags & 8 != 0));
        f.add_field_method_get("isEditable", |_, t| Ok(!t.get()?.is_locked()));
        f.add_field_method_get("sprite", |_, t| {
            t.get()?;
            Ok(SpriteH(t.0.clone()))
        });
        add_user_data_fields(f, |t: &LayerH| UserDataH::new(&t.0, 1, t.1, 0));
        f.add_field_method_get("isGroup", |_, t| Ok(t.get()?.is_group()));
        f.add_field_method_get("isTilemap", |_, t| Ok(t.get()?.kind == crate::document::tilemap::KIND_TILEMAP));
        f.add_field_method_get("isReference", |_, t| Ok(t.get()?.is_reference()));
        f.add_field_method_set("isReference", |_, t, v: bool| {
            t.get()?;
            if t.0.sprite().set_layer_reference(t.1, v) {
                Ok(())
            } else {
                Err(Error::runtime("a group cannot be a reference layer"))
            }
        });
        f.add_field_method_get("index", |_, t| {
            t.get()?;
            Ok(t.1 + 1)
        });
        f.add_field_method_get("level", |_, t| Ok(t.get()?.child_level));
        // Whether the layer is shown: its own flag and those of all its parent groups.
        f.add_field_method_get("isVisible", |_, t| {
            t.get()?;
            Ok(t.0.sprite().is_layer_shown(t.1))
        });
        // Assigning sets the layer's own visibility (Aseprite's `layer.isVisible = false`).
        f.add_field_method_set("isVisible", |_, t, v: bool| {
            t.get()?.visible = v;
            Ok(())
        });
        f.add_field_method_get("tileset", |_, t| {
            let l = t.get()?;
            let ti = l.tileset as usize;
            Ok((l.kind == crate::document::tilemap::KIND_TILEMAP && ti < t.0.sprite().tilesets.len())
                .then(|| TilesetH(t.0.clone(), ti)))
        });
        f.add_field_method_get("userData", |_, t| {
            t.get()?;
            Ok(UserDataH::new(&t.0, 1, t.1, 0))
        });
        f.add_field_method_get("parent", |_, t| {
            t.get()?;
            Ok(t.0.sprite().parent_layer(t.1).map(|p| LayerH::new(&t.0, p)))
        });
        // The tween parent (docs/LUA_API.md, Tween rig): assigning nil unlinks; a cycle is an error.
        f.add_field_method_get("tweenParent", |_, t| {
            t.get()?;
            Ok(crate::gamedev::rig::parent_of(t.0.sprite(), t.1).map(|p| LayerH::new(&t.0, p)))
        });
        f.add_field_method_set("tweenParent", |_, t, v: Value| {
            t.get()?;
            let parent = if matches!(v, Value::Nil) { None } else { Some(t.0.layer_of(&v)?) };
            if crate::gamedev::rig::set_parent_raw(t.0.sprite(), t.1, parent) {
                Ok(())
            } else {
                Err(Error::runtime(
                    "tweenParent: both layers must be normal or group layers, a layer cannot be its own ancestor, \
                     and a group cannot be linked to a layer inside it",
                ))
            }
        });
        f.add_field_method_get("children", |lua, t| {
            t.get()?;
            let s = t.0.sprite();
            let level = s.layers[t.1].child_level;
            let kids: Vec<LayerH> = if s.layers[t.1].is_group() {
                (t.1 + 1..s.subtree_end(t.1))
                    .filter(|&j| s.layers[j].child_level == level + 1)
                    .map(|j| LayerH::new(&t.0, j))
                    .collect()
            } else {
                Vec::new()
            };
            lua.create_sequence_from(kids)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(mlua::MetaMethod::Eq, |_, t, o: Value| {
            Ok(match o {
                Value::UserData(u) => u.borrow::<LayerH>().is_ok_and(|o| Rc::ptr_eq(&o.0, &t.0) && o.1 == t.1),
                _ => false,
            })
        });
        m.add_meta_method(mlua::MetaMethod::ToString, |_, t, ()| Ok(format!("Layer(\"{}\")", t.get()?.name)));
        // layer:setTweenKey("rotation", frame, 30 [, "linear"]): props x, y, rotation, scaleX, scaleY, opacity.
        m.add_method("setTweenKey", |_, t, (prop, frame, value, ease): (String, Value, f64, Option<String>)| {
            use crate::gamedev::tween::{set_key_raw, Ease, Prop};
            t.get()?;
            let f = t.0.frame_of(&frame)?;
            let prop = match prop.as_str() {
                "x" => Prop::PosX,
                "y" => Prop::PosY,
                "rotation" => Prop::Rotation,
                "scaleX" => Prop::ScaleX,
                "scaleY" => Prop::ScaleY,
                "opacity" => Prop::Opacity,
                other => {
                    return Err(Error::runtime(format!(
                        "unknown tween property '{other}' (x, y, rotation, scaleX, scaleY, opacity)"
                    )))
                }
            };
            let ease = match ease.as_deref().unwrap_or("linear") {
                "linear" => Ease::Linear,
                "hold" => Ease::Hold,
                "inQuad" => Ease::InQuad,
                "outQuad" => Ease::OutQuad,
                "inOutQuad" => Ease::InOutQuad,
                "inOutCubic" => Ease::InOutCubic,
                "outBack" => Ease::OutBack,
                "outBounce" => Ease::OutBounce,
                other => return Err(Error::runtime(format!("unknown easing '{other}'"))),
            };
            if !set_key_raw(t.0.sprite(), t.1, prop, f as u32, value as f32, ease) {
                return Err(Error::runtime("setTweenKey: the layer cannot be tweened or the value is not finite"));
            }
            Ok(())
        });
        // layer:setTweenPivot(x, y) in canvas coordinates (pixel corners), or setTweenPivot(nil) for the content center.
        m.add_method("setTweenPivot", |_, t, (x, y): (Option<f64>, Option<f64>)| {
            t.get()?;
            let pivot = match (x, y) {
                (Some(x), Some(y)) => Some((x as f32, y as f32)),
                (None, None) => None,
                _ => return Err(Error::runtime("setTweenPivot needs x and y, or nil")),
            };
            if crate::gamedev::tween::set_pivot_raw(t.0.sprite(), t.1, pivot) {
                Ok(())
            } else {
                Err(Error::runtime("setTweenPivot: the layer cannot be tweened or the pivot is not finite"))
            }
        });
        // layer:tweenTransform([frame]) -> { matrix = {a,b,c,d,e,f}, local = {...}, rotation, scaleX, scaleY, pivotX, pivotY },
        // or nil when the layer is not tweened (no keys of its own or of an ancestor).
        m.add_method("tweenTransform", |lua, t, frame: Option<Value>| {
            t.get()?;
            let f = match frame {
                Some(v) if !matches!(v, Value::Nil) => t.0.frame_of(&v)?,
                _ => t.0.frame_index(None)?,
            };
            let Some(c) = crate::gamedev::rig::composed_transform(t.0.sprite(), t.1, f as u32) else { return Ok(None) };
            let out = lua.create_table()?;
            out.set("matrix", lua.create_sequence_from(c.matrix)?)?;
            out.set("local", lua.create_sequence_from(c.local)?)?;
            out.set("rotation", c.rotation)?;
            out.set("scaleX", c.scale_x)?;
            out.set("scaleY", c.scale_y)?;
            out.set("pivotX", c.pivot_world.0)?;
            out.set("pivotY", c.pivot_world.1)?;
            Ok(Some(out))
        });
        // layer:tweenMap([frame,] x, y) -> x, y: a point of the layer's rest pose where it is drawn at that frame.
        m.add_method("tweenMap", |_, t, mut a: MultiValue| {
            t.get()?;
            let nums = a.iter().filter(|v| matches!(v, Value::Integer(_) | Value::Number(_))).count();
            let f = if nums >= 3 { take_frame_idx(&t.0, &mut a, false)? } else { t.0.frame_index(None)? };
            let num = |v: Option<Value>| match v {
                Some(Value::Integer(i)) => Ok(i as f64),
                Some(Value::Number(n)) => Ok(n),
                _ => Err(Error::runtime("tweenMap needs x and y")),
            };
            let (x, y) = (num(a.pop_front())?, num(a.pop_front())?);
            let m = crate::gamedev::rig::world_matrix(t.0.sprite(), t.1, f as u32)
                .ok_or_else(|| Error::runtime("tweenMap: the layer is not tweened"))?;
            Ok(crate::gamedev::rig::map_point(&m, x, y))
        });
        // layer:cel(frame) -> the cel, or nil when the layer has no image there (as in Aseprite).
        m.add_method("cel", |_, t, frame: Value| {
            t.get()?;
            let f = t.0.frame_of(&frame)?;
            Ok(t.0.sprite().cels[f][t.1].is_some().then(|| CelH(t.0.clone(), t.1, f, t.0.epoch.get())))
        });

        // Tiles are addressed by grid cell: (x, y) is the cell whose top-left pixel is
        // (x * tileWidth, y * tileHeight). Tile 0 is the empty tile.
        // getTile(x, y [, frame]) or getTile(point [, frame])
        m.add_method("getTile", |_, t, mut a: MultiValue| {
            let geometry::PointV { x, y } = geometry::take_point(&mut a, "getTile")?;
            let (tw, th) = t.tile_size()?;
            let f = take_frame_idx(&t.0, &mut a, false)?;
            let s = t.0.sprite();
            let (px, py) = (x.saturating_mul(tw as i64), y.saturating_mul(th as i64));
            if px < 0 || py < 0 || px >= s.width as i64 || py >= s.height as i64 {
                return Ok((0u32, false, false, false));
            }
            let v = crate::document::tilemap::tile_at(s, t.1, f, px as i32, py as i32);
            Ok((
                v & crate::document::tilemap::TILE_ID_MASK,
                v & crate::document::tilemap::FLIP_X != 0,
                v & crate::document::tilemap::FLIP_Y != 0,
                v & crate::document::tilemap::FLIP_D != 0,
            ))
        });
        // setTile(x, y, index [, flipX, flipY, diagonal [, frame]]) or setTile(point, index [, ...])
        m.add_method("setTile", |_, t, mut a: MultiValue| {
            let geometry::PointV { x, y } = geometry::take_point(&mut a, "setTile")?;
            let index = geometry::take_opt_int(&mut a, "setTile index")?
                .ok_or_else(|| Error::runtime("setTile needs a tile index"))?;
            let flag = |a: &mut MultiValue| match a.pop_front() {
                None | Some(Value::Nil) => Ok(None),
                Some(Value::Boolean(b)) => Ok(Some(b)),
                Some(_) => Err(Error::runtime("setTile: flip flags must be booleans")),
            };
            let (fx, fy, fd) = (flag(&mut a)?, flag(&mut a)?, flag(&mut a)?);
            let frame = a.pop_front();
            {
                let f = match frame {
                    None | Some(Value::Nil) => t.0.frame_index(None)?,
                    Some(v) => t.0.frame_of(&v)?,
                };
                let (tw, th) = t.tile_size()?;
                let s = t.0.sprite();
                let ti = s.layers[t.1].tileset as usize;
                let count = s.tilesets[ti].tiles.len();
                let id = u32::try_from(index).ok().filter(|&i| (i as usize) < count).ok_or_else(|| {
                    Error::runtime(format!("tile {index} does not exist in the tileset (0..{})", count - 1))
                })?;
                let (px, py) = (x.saturating_mul(tw as i64), y.saturating_mul(th as i64));
                if px < 0 || py < 0 || px >= s.width as i64 || py >= s.height as i64 {
                    return Err(Error::runtime(format!("cell {x},{y} is outside the canvas")));
                }
                let value = id
                    | if fx == Some(true) { crate::document::tilemap::FLIP_X } else { 0 }
                    | if fy == Some(true) { crate::document::tilemap::FLIP_Y } else { 0 }
                    | if fd == Some(true) { crate::document::tilemap::FLIP_D } else { 0 };
                s.begin_cel_edit(t.1, f);
                crate::document::tilemap::stamp(s, t.1, f, px as i32, py as i32, value);
                s.flush_links();
                Ok(())
            }
        });
    }
}

impl LayerH {
    /// Tile size of a tilemap layer's tileset.
    fn tile_size(&self) -> LuaResult<(u32, u32)> {
        let l = self.get()?;
        let ts = self.0.sprite().tilesets.get(l.tileset as usize);
        match ts {
            Some(ts) if l.kind == crate::document::tilemap::KIND_TILEMAP => Ok((ts.tile_w, ts.tile_h)),
            _ => Err(Error::runtime("this is not a tilemap layer")),
        }
    }
}

impl UserData for FrameH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("duration", |_, t| Ok(t.0.sprite().durations[t.check()?]));
        f.add_field_method_set("duration", |_, t, v: Value| {
            let v = whole("frame.duration", &v)?;
            let i = t.check()?;
            t.0.sprite().durations[i] = v.clamp(1, u16::MAX as i64) as u16;
            Ok(())
        });
        f.add_field_method_get("index", |_, t| Ok(t.check()? + 1));
        f.add_field_method_get("frameNumber", |_, t| Ok(t.check()? + 1));
        f.add_field_method_get("sprite", |_, t| {
            t.check()?;
            Ok(SpriteH(t.0.clone()))
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(mlua::MetaMethod::Eq, |_, t, o: Value| {
            Ok(match o {
                Value::UserData(u) => u.borrow::<FrameH>().is_ok_and(|o| Rc::ptr_eq(&o.0, &t.0) && o.1 == t.1),
                _ => false,
            })
        });
        m.add_meta_method(mlua::MetaMethod::ToString, |_, t, ()| Ok(format!("Frame({})", t.1 + 1)));
    }
}

impl UserData for TagH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("name", |_, t| Ok(t.get()?.name.clone()));
        f.add_field_method_set("name", |_, t, v: String| {
            t.get()?.name = v;
            Ok(())
        });
        f.add_field_method_get("from", |_, t| Ok(t.get()?.from as usize + 1));
        f.add_field_method_set("from", |_, t, v: Value| {
            let v = whole("tag.from", &v)?;
            let f = t.0.frame_index(Some(v))?;
            let tag = t.get()?;
            let (a, b) = (f as u16, tag.to);
            (tag.from, tag.to) = (a.min(b), a.max(b));
            Ok(())
        });
        f.add_field_method_get("to", |_, t| Ok(t.get()?.to as usize + 1));
        f.add_field_method_set("to", |_, t, v: Value| {
            let v = whole("tag.to", &v)?;
            let f = t.0.frame_index(Some(v))?;
            let tag = t.get()?;
            let (a, b) = (tag.from, f as u16);
            (tag.from, tag.to) = (a.min(b), a.max(b));
            Ok(())
        });
        f.add_field_method_get("direction", |_, t| Ok(t.get()?.direction));
        f.add_field_method_set("direction", |_, t, v: Value| {
            let v = whole("tag.direction", &v)?;
            if !(0..=3).contains(&v) {
                return Err(Error::runtime(format!("direction {v} is not in 0..3 (see app.direction)")));
            }
            t.get()?.direction = v as u8;
            Ok(())
        });
        f.add_field_method_get("repeatCount", |_, t| Ok(t.get()?.repeat));
        f.add_field_method_set("repeatCount", |_, t, v: Value| {
            let v = whole("tag.repeatCount", &v)?;
            t.get()?.repeat = v.clamp(0, u16::MAX as i64) as u16;
            Ok(())
        });
        f.add_field_method_get("color", |_, t| {
            let c = t.get()?.color;
            Ok(color::ColorH::from_rgba(&t.0, [c[0], c[1], c[2], 255]))
        });
        f.add_field_method_set("color", |_, t, v: color::PxColor| {
            let c = v.0;
            t.get()?.color = [c[0], c[1], c[2]];
            Ok(())
        });
        f.add_field_method_get("aniDir", |_, t| Ok(t.get()?.direction));
        f.add_field_method_set("aniDir", |_, t, v: Value| {
            let v = whole("tag.aniDir", &v)?;
            if !(0..=3).contains(&v) {
                return Err(Error::runtime(format!("direction {v} is not an AniDir value")));
            }
            t.get()?.direction = v as u8;
            Ok(())
        });
        f.add_field_method_get("fromFrame", |_, t| {
            let from = t.get()?.from as usize;
            Ok(FrameH::new(&t.0, from))
        });
        f.add_field_method_get("toFrame", |_, t| {
            let to = t.get()?.to as usize;
            Ok(FrameH::new(&t.0, to))
        });
        f.add_field_method_get("frames", |_, t| {
            let tag = t.get()?;
            Ok(tag.to as usize - tag.from as usize + 1)
        });
        f.add_field_method_get("data", |_, t| {
            t.get()?;
            Ok(UserDataH::new(&t.0, 3, t.1, 0).read()?.text.unwrap_or_default())
        });
        f.add_field_method_set("data", |_, t, v: Option<String>| {
            t.get()?;
            UserDataH::new(&t.0, 3, t.1, 0).write(|u| u.text = v.filter(|s| !s.is_empty()))
        });
        f.add_field_method_get("userData", |_, t| {
            t.get()?;
            Ok(UserDataH::new(&t.0, 3, t.1, 0))
        });
        f.add_field_method_get("index", |_, t| {
            t.get()?;
            Ok(t.1 + 1)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("delete", |_, t, ()| {
            t.get()?;
            t.0.sprite().remove_tag(t.1);
            t.0.invalidate_handles();
            Ok(())
        });
    }
}

impl CelH {
    fn check(&self) -> LuaResult<()> {
        self.0.check(self.3, "cel")
    }
}

impl UserData for CelH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("opacity", |_, t| {
            t.check()?;
            Ok(t.0.sprite().cels[t.2][t.1].as_ref().map_or(255, |c| c.opacity))
        });
        f.add_field_method_set("opacity", |_, t, v: Value| {
            let v = whole("cel.opacity", &v)?;
            t.check()?;
            if t.0.sprite().set_cel_opacity(t.1, t.2, v.clamp(0, 255) as u8) {
                Ok(())
            } else {
                Err(Error::runtime("the cel is empty; draw something first"))
            }
        });
        f.add_field_method_get("zIndex", |_, t| {
            t.check()?;
            Ok(t.0.sprite().cel_z_index(t.1, t.2).unwrap_or(0))
        });
        f.add_field_method_set("zIndex", |_, t, v: Value| {
            let v = whole("cel.zIndex", &v)?;
            t.check()?;
            let z = v.clamp(i16::MIN as i64, i16::MAX as i64) as i32;
            if t.0.sprite().set_cel_z_index(t.1, t.2, z) {
                Ok(())
            } else {
                Err(Error::runtime("the cel is empty; draw something first"))
            }
        });
        f.add_field_method_get("isEmpty", |_, t| {
            t.check()?;
            Ok(t.0.sprite().cels[t.2][t.1]
                .as_ref()
                .is_none_or(|c| c.outside.is_none() && c.pixels.chunks_exact(4).all(|p| p[3] == 0)))
        });
        f.add_field_method_get("linked", |_, t| {
            t.check()?;
            Ok(t.0.sprite().is_cel_linked(t.1, t.2))
        });
        f.add_field_method_get("linkedFrames", |lua, t| {
            t.check()?;
            let frames: Vec<usize> = t.0.sprite().link_siblings(t.1, t.2).into_iter().map(|f| f + 1).collect();
            lua.create_sequence_from(frames)
        });
        f.add_field_method_get("userData", |_, t| {
            t.check()?;
            Ok(UserDataH::new(&t.0, 2, t.2, t.1))
        });
        // Objects as in Aseprite (`cel.frame.frameNumber`); `cel.frameNumber` is the plain number.
        f.add_field_method_get("layer", |_, t| {
            t.check()?;
            Ok(LayerH::new(&t.0, t.1))
        });
        f.add_field_method_get("frame", |_, t| {
            t.check()?;
            Ok(FrameH::new(&t.0, t.2))
        });
        f.add_field_method_get("frameNumber", |_, t| Ok(t.2 + 1));
        f.add_field_method_get("sprite", |_, t| {
            t.check()?;
            Ok(SpriteH(t.0.clone()))
        });
        add_user_data_fields(f, |t: &CelH| UserDataH::new(&t.0, 2, t.2, t.1));
        // The box around all of the cel's non-transparent pixels, also those beyond the canvas edges.
        f.add_field_method_get("bounds", |_, t| {
            t.check()?;
            Ok(image::cel_rect(t.0.sprite(), t.1, t.2))
        });
        f.add_field_method_get("position", |_, t| {
            t.check()?;
            let r = image::cel_rect(t.0.sprite(), t.1, t.2);
            Ok(geometry::PointV { x: r.x, y: r.y })
        });
        f.add_field_method_set("position", |_, t, v: Value| {
            t.check()?;
            let (x, y) = geometry::point_of(&v).ok_or_else(|| Error::runtime("position must be a Point"))?;
            image::set_cel_position(&t.0, t.1, t.2, geometry::PointV { x, y })
        });
        // A live view of the cel's pixels (nil for an empty cel): drawing on it draws on the cel.
        f.add_field_method_get("image", |_, t| {
            t.check()?;
            Ok(image::cel_image(&t.0, t.1, t.2))
        });
        f.add_field_method_set("image", |_, t, v: Value| {
            t.check()?;
            let img = image::image_of(&v).ok_or_else(|| Error::runtime("cel.image must be set to an Image"))?;
            image::set_cel_image(&t.0, t.1, t.2, &img)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("unlink", |_, t, ()| {
            t.check()?;
            Ok(t.0.sprite().unlink_cel(t.1, t.2))
        });
    }
}

/// Parses a selection mode: "replace" (default), "add", "subtract" or 0, 1, 2.
fn mode_arg(v: Option<Value>) -> LuaResult<u8> {
    Ok(match v {
        None | Some(Value::Nil) => 0,
        Some(Value::String(s)) => match s.to_string_lossy().as_str() {
            "replace" => 0,
            "add" => 1,
            "subtract" => 2,
            other => return Err(Error::runtime(format!("selection mode '{other}' is not replace, add or subtract"))),
        },
        Some(Value::Integer(i)) if (0..=2).contains(&i) => i as u8,
        Some(_) => return Err(Error::runtime("selection mode must be \"replace\", \"add\" or \"subtract\"")),
    })
}

/// Combines a rectangle (clipped to the canvas) with the script's selection. `mode`: 0 replace, 1 add,
/// 2 subtract, 3 intersect.
fn select_with_rect(ctx: &Ctx, rect: &geometry::RectV, mode: u8) {
    let s = ctx.sprite();
    let (cw, ch) = (s.width as i64, s.height as i64);
    let mut r = Selection::new(s.width, s.height, false);
    let (x0, y0) = (rect.x.max(0), rect.y.max(0));
    let (x1, y1) = (rect.x.saturating_add(rect.w).min(cw), rect.y.saturating_add(rect.h).min(ch));
    for py in y0..y1.max(y0) {
        for px in x0..x1.max(x0) {
            r.bits[(py * cw + px) as usize] = true;
        }
    }
    let combined = match (mode, ctx.mask()) {
        (1, Some(base)) => {
            let mut c = (*base).clone();
            c.bits.iter_mut().zip(&r.bits).for_each(|(a, b)| *a |= *b);
            c
        }
        (2, Some(base)) => {
            let mut c = (*base).clone();
            c.bits.iter_mut().zip(&r.bits).for_each(|(a, b)| *a &= !*b);
            c
        }
        (2 | 3, None) => Selection::new(s.width, s.height, false),
        (3, Some(base)) => {
            let mut c = (*base).clone();
            c.bits.iter_mut().zip(&r.bits).for_each(|(a, b)| *a &= *b);
            c
        }
        _ => r,
    };
    ctx.set_mask(combined);
}

impl UserData for SelectionH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("isEmpty", |_, t| Ok(t.0.mask().is_none()));
        f.add_field_method_get("bounds", |_, t| {
            Ok(t.0
                .mask()
                .and_then(|m| m.bounds())
                .map(|(x, y, w, h)| geometry::RectV::new(x as i64, y as i64, w as i64, h as i64)))
        });
        // The top-left of the selection (0,0 when empty); assigning a Point moves the selection.
        f.add_field_method_get("origin", |_, t| {
            let (x, y) = t.0.mask().and_then(|m| m.bounds()).map_or((0, 0), |b| (b.0 as i64, b.1 as i64));
            Ok(geometry::PointV { x, y })
        });
        f.add_field_method_set("origin", |_, t, v: Value| {
            let (x, y) = geometry::point_of(&v).ok_or_else(|| Error::runtime("origin must be a Point"))?;
            let Some(mask) = t.0.mask() else { return Ok(()) };
            let Some((bx, by, _, _)) = mask.bounds() else { return Ok(()) };
            let (dx, dy) = (x.saturating_sub(bx as i64), y.saturating_sub(by as i64));
            let (w, h) = (mask.w as i64, mask.h as i64);
            let mut moved = Selection::new(mask.w, mask.h, false);
            for (i, on) in mask.bits.iter().enumerate() {
                if *on {
                    let (nx, ny) = ((i as i64 % w).saturating_add(dx), (i as i64 / w).saturating_add(dy));
                    if nx >= 0 && ny >= 0 && nx < w && ny < h {
                        moved.bits[(ny * w + nx) as usize] = true;
                    }
                }
            }
            t.0.set_mask(moved);
            Ok(())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        // contains(x, y) or contains(point)
        m.add_method("contains", |_, t, mut a: MultiValue| {
            let p = geometry::take_point(&mut a, "contains")?;
            Ok(match t.0.mask() {
                Some(m)
                    if p.x >= i32::MIN as i64
                        && p.x <= i32::MAX as i64
                        && p.y >= i32::MIN as i64
                        && p.y <= i32::MAX as i64 =>
                {
                    m.get(p.x as i32, p.y as i32)
                }
                _ => false,
            })
        });
        // selectRect(x, y, w, h [, mode]) or selectRect(rectangle [, mode]); mode: "replace", "add", "subtract"
        m.add_method("selectRect", |_, t, mut a: MultiValue| {
            let r = geometry::take_rect(&mut a, "selectRect")?;
            let mode = mode_arg(a.pop_front())?;
            select_with_rect(&t.0, &r, mode);
            Ok(())
        });
        // Aseprite's names: select(rect) replaces; add, subtract and intersect combine.
        for (name, mode) in [("select", 0u8), ("add", 1), ("subtract", 2), ("intersect", 3)] {
            m.add_method(name, move |_, t, mut a: MultiValue| {
                let r = geometry::take_rect(&mut a, name)?;
                select_with_rect(&t.0, &r, mode);
                Ok(())
            });
        }
        m.add_method("selectAll", |_, t, ()| {
            let s = t.0.sprite();
            t.0.set_mask(Selection::new(s.width, s.height, true));
            Ok(())
        });
        m.add_method("deselect", |_, t, ()| {
            *t.0.mask.borrow_mut() = None;
            Ok(())
        });
        m.add_method("invert", |_, t, ()| {
            let s = t.0.sprite();
            let mut inv = t.0.mask().map_or_else(|| Selection::new(s.width, s.height, false), |m| (*m).clone());
            inv.bits.iter_mut().for_each(|b| *b = !*b);
            t.0.set_mask(inv);
            Ok(())
        });
    }
}

// MARK: User data, tilesets and slices

/// Text and color attached to the sprite, a layer, a cel, a tag or a slice.
struct UserDataH {
    ctx: Rc<Ctx>,
    /// 0 sprite, 1 layer (`a`), 2 cel (`a` frame, `b` layer), 3 tag (`a`), 4 slice (`a`): the codes of
    /// `slices::target`.
    kind: u32,
    a: u32,
    b: u32,
    epoch: u64,
}

impl UserDataH {
    fn new(ctx: &Rc<Ctx>, kind: u32, a: usize, b: usize) -> Self {
        UserDataH { ctx: ctx.clone(), kind, a: a as u32, b: b as u32, epoch: ctx.epoch.get() }
    }

    fn read(&self) -> LuaResult<crate::document::slices::UserData> {
        self.ctx.check(self.epoch, "user data")?;
        Ok(crate::document::slices::target(self.ctx.sprite(), self.kind, self.a, self.b).cloned().unwrap_or_default())
    }

    fn write(&self, f: impl FnOnce(&mut crate::document::slices::UserData)) -> LuaResult<()> {
        self.ctx.check(self.epoch, "user data")?;
        let s = self.ctx.sprite();
        let (a, b) = (self.a as usize, self.b as usize);
        match self.kind {
            0 => f(&mut s.extra.sprite),
            1 => f(&mut s.layers.get_mut(a).ok_or_else(|| Error::runtime("layer does not exist"))?.user_data),
            2 => {
                let cel = s.cel_mut(b, a).ok_or_else(|| Error::runtime("cannot attach user data to this cel"))?;
                f(&mut cel.user_data);
                s.flush_links();
            }
            3 => f(&mut s.tags.get_mut(a).ok_or_else(|| Error::runtime("tag does not exist"))?.user_data),
            _ => f(&mut s.extra.slices.get_mut(a).ok_or_else(|| Error::runtime("slice does not exist"))?.user),
        }
        Ok(())
    }
}

/// Aseprite's `layer.data` (string) and `layer.color` (Color) shortcuts for the user data of `T`.
fn add_user_data_fields<T: 'static, F: UserDataFields<T>>(f: &mut F, make: fn(&T) -> UserDataH) {
    f.add_field_method_get("data", move |_, t| Ok(make(t).read()?.text.unwrap_or_default()));
    f.add_field_method_set("data", move |_, t, v: Option<String>| {
        make(t).write(|u| u.text = v.filter(|s| !s.is_empty()))
    });
    f.add_field_method_get("color", move |_, t| {
        let u = make(t);
        Ok(color::ColorH::from_rgba(&u.ctx, u.read()?.color.unwrap_or([0, 0, 0, 0])))
    });
    f.add_field_method_set("color", move |_, t, v: Option<color::PxColor>| {
        let c = v.map(|c| c.0);
        make(t).write(|u| u.color = c)
    });
}

impl UserData for UserDataH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("text", |_, t| Ok(t.read()?.text));
        f.add_field_method_set("text", |_, t, v: Option<String>| t.write(|u| u.text = v.filter(|s| !s.is_empty())));
        f.add_field_method_get("color", |_, t| Ok(t.read()?.color.map(|c| color::ColorH::from_rgba(&t.ctx, c))));
        f.add_field_method_set("color", |_, t, v: Option<color::PxColor>| {
            let c = v.map(|c| c.0);
            t.write(|u| u.color = c)
        });
    }
}

struct TilesetH(Rc<Ctx>, usize);

impl TilesetH {
    fn get(&self) -> LuaResult<&mut crate::document::tilemap::Tileset> {
        self.0.sprite().tilesets.get_mut(self.1).ok_or_else(|| Error::runtime("tileset does not exist"))
    }

    fn tile_index(&self, i: i64) -> LuaResult<usize> {
        let n = self.get()?.tiles.len();
        usize::try_from(i)
            .ok()
            .filter(|&i| i < n)
            .ok_or_else(|| Error::runtime(format!("tile {i} does not exist (0..{})", n - 1)))
    }

    fn check_room(&self) -> LuaResult<()> {
        let ts = self.get()?;
        if ts.tiles.len() * ts.tile_w as usize * ts.tile_h as usize * 4 >= MAX_TILESET_BYTES {
            return Err(Error::runtime("the tileset is full"));
        }
        Ok(())
    }

    fn write_tile(&self, i: usize, pixels: &Table) -> LuaResult<()> {
        let s = self.0.sprite();
        let (tw, th, zero_empty) = {
            let ts = self.get()?;
            (ts.tile_w as usize, ts.tile_h as usize, ts.zero_is_empty())
        };
        if i == 0 && zero_empty {
            return Err(Error::runtime("tile 0 is the empty tile and cannot be changed"));
        }
        if pixels.raw_len() != tw * th {
            return Err(Error::runtime(format!(
                "a tile needs {} colors ({tw}x{th}), got {}",
                tw * th,
                pixels.raw_len()
            )));
        }
        let mut snap = Snap::new(s);
        let mut data = Vec::with_capacity(tw * th * 4);
        for v in pixels.sequence_values::<color::PxColor>() {
            data.extend(snap.get(v?.0));
        }
        self.get()?.tiles[i] = Arc::new(data);
        crate::document::tilemap::refresh_tileset_users(s, self.1);
        Ok(())
    }
}

impl UserData for TilesetH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("name", |_, t| Ok(t.get()?.name.clone()));
        f.add_field_method_set("name", |_, t, v: String| {
            t.get()?.name = v;
            Ok(())
        });
        f.add_field_method_get("tileWidth", |_, t| Ok(t.get()?.tile_w));
        f.add_field_method_get("tileHeight", |_, t| Ok(t.get()?.tile_h));
        f.add_field_method_get("count", |_, t| Ok(t.get()?.tiles.len()));
        f.add_field_method_get("index", |_, t| {
            t.get()?;
            Ok(t.1 + 1)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("getTilePixels", |lua, t, i: i64| {
            let i = t.tile_index(i)?;
            let tile = t.get()?.tiles[i].clone();
            lua.create_sequence_from(tile.chunks_exact(4).map(pack))
        });
        m.add_method("setTilePixels", |_, t, (i, pixels): (i64, Table)| {
            let i = t.tile_index(i)?;
            t.write_tile(i, &pixels)
        });
        m.add_method("addTile", |_, t, pixels: Option<Table>| {
            t.check_room()?;
            let i = t.get()?.add_blank() as usize;
            if let Some(p) = pixels {
                if let Err(e) = t.write_tile(i, &p) {
                    t.get()?.tiles.pop();
                    return Err(e);
                }
            }
            Ok(i)
        });
        m.add_method("duplicateTile", |_, t, i: i64| {
            let i = t.tile_index(i)?;
            t.check_room()?;
            crate::document::tilemap::duplicate_tile(t.0.sprite(), t.1, i as u32)
                .ok_or_else(|| Error::runtime("cannot duplicate this tile"))
        });
        m.add_method("deleteTile", |_, t, i: i64| {
            let i = t.tile_index(i)?;
            if crate::document::tilemap::delete_tile(t.0.sprite(), t.1, i as u32) {
                Ok(())
            } else {
                Err(Error::runtime(
                    "this tile cannot be deleted (tile 0 is the empty tile, and a tileset keeps at least one tile)",
                ))
            }
        });
    }
}

struct SliceH(Rc<Ctx>, usize, u64);

impl SliceH {
    fn new(c: &Rc<Ctx>, i: usize) -> Self {
        SliceH(c.clone(), i, c.epoch.get())
    }

    fn get(&self) -> LuaResult<&mut crate::document::slices::Slice> {
        self.0.check(self.2, "slice")?;
        self.0.sprite().extra.slices.get_mut(self.1).ok_or_else(|| Error::runtime("slice does not exist"))
    }
}

fn slice_rect(x: i64, y: i64, w: i64, h: i64) -> LuaResult<crate::document::slices::Rect> {
    const LIMIT: i64 = 1 << 20;
    if x.abs() > LIMIT || y.abs() > LIMIT || !(0..=LIMIT).contains(&w) || !(0..=LIMIT).contains(&h) {
        return Err(Error::runtime("slice rectangle out of range"));
    }
    Ok(crate::document::slices::Rect { x: x as i32, y: y as i32, w: w as u32, h: h as u32 })
}

/// An optional trailing frame argument (a number or a frame handle); the active frame when absent.
/// An integer field value. Lua numbers with a fraction would otherwise fail with mlua's bare
/// "error converting Lua number to i64 (out of range)".
fn whole(field: &str, v: &Value) -> LuaResult<i64> {
    match v {
        Value::Integer(i) => Ok(*i),
        Value::Number(n) if n.fract() == 0.0 && n.abs() < 9.0e18 => Ok(*n as i64),
        Value::Number(n) if field == "frame.duration" && n.is_finite() && n.fract() != 0.0 => {
            Err(Error::runtime(format!(
            "frame.duration is whole milliseconds (1-65535); got {n} \u{2014} Aseprite uses seconds, multiply by 1000"
        )))
        }
        Value::Number(n) if n.is_finite() && n.fract() != 0.0 => {
            Err(Error::runtime(format!("{field} must be a whole number, got {n}")))
        }
        Value::Number(n) => Err(Error::runtime(format!("{field} is out of range, got {n}"))),
        other => Err(Error::runtime(format!("{field} must be a whole number, got {}", other.type_name()))),
    }
}

fn take_frame(ctx: &Ctx, a: &mut MultiValue) -> LuaResult<u32> {
    match a.pop_front() {
        None | Some(Value::Nil) => Ok(ctx.frame_index(None)? as u32),
        Some(v) => Ok(ctx.frame_of(&v)? as u32),
    }
}

fn rect_value(r: crate::document::slices::Rect) -> geometry::RectV {
    geometry::RectV::new(r.x as i64, r.y as i64, r.w as i64, r.h as i64)
}

impl UserData for SliceH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("name", |_, t| Ok(t.get()?.name.clone()));
        f.add_field_method_set("name", |_, t, v: String| {
            t.get()?.name = v;
            Ok(())
        });
        f.add_field_method_get("index", |_, t| {
            t.get()?;
            Ok(t.1 + 1)
        });
        f.add_field_method_get("hasCenter", |_, t| Ok(t.get()?.nine_patch));
        f.add_field_method_get("hasPivot", |_, t| Ok(t.get()?.has_pivot));
        f.add_field_method_get("userData", |_, t| {
            t.get()?;
            Ok(UserDataH::new(&t.0, 4, t.1, 0))
        });
        // As in Aseprite, `bounds`, `center` and `pivot` are the values of the slice's first frame; use
        // getBounds / setBounds ... to address another frame. `center` is relative to `bounds`.
        f.add_field_method_get("bounds", |_, t| {
            let sl = t.get()?;
            Ok(sl.key_at(0).or(sl.keys.first()).map(|k| rect_value(k.bounds)))
        });
        f.add_field_method_set("bounds", |_, t, v: Value| {
            let r = geometry::rect_of(&v).ok_or_else(|| Error::runtime("bounds must be a Rectangle"))?;
            let r = slice_rect(r.x, r.y, r.w, r.h)?;
            t.get()?;
            crate::document::slices::set_bounds(t.0.sprite(), t.1, 0, r);
            Ok(())
        });
        f.add_field_method_get("center", |_, t| {
            let sl = t.get()?;
            if !sl.nine_patch {
                return Ok(None);
            }
            Ok(sl.key_at(0).or(sl.keys.first()).map(|k| rect_value(k.center)))
        });
        f.add_field_method_set("center", |_, t, v: Value| {
            let sl = t.get()?;
            if matches!(v, Value::Nil) {
                sl.nine_patch = false;
                return Ok(());
            }
            let r = geometry::rect_of(&v).ok_or_else(|| Error::runtime("center must be a Rectangle or nil"))?;
            let r = slice_rect(r.x, r.y, r.w, r.h)?;
            crate::document::slices::key_for_edit(sl, 0).center = r;
            sl.nine_patch = true;
            Ok(())
        });
        f.add_field_method_get("pivot", |_, t| {
            let sl = t.get()?;
            Ok(sl.pivot_at(0).map(|p| geometry::PointV { x: p.0 as i64, y: p.1 as i64 }))
        });
        f.add_field_method_set("pivot", |_, t, v: Value| {
            let sl = t.get()?;
            if matches!(v, Value::Nil) {
                sl.has_pivot = false;
                sl.pivot_from = 0;
                return Ok(());
            }
            let (x, y) = geometry::point_of(&v).ok_or_else(|| Error::runtime("pivot must be a Point or nil"))?;
            if x.abs() > 1 << 20 || y.abs() > 1 << 20 {
                return Err(Error::runtime("pivot out of range"));
            }
            sl.set_pivot_all((x as i32, y as i32));
            Ok(())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        // Geometry is per frame: a key set on frame N applies from N until the next key.
        m.add_method("getBounds", |_, t, frame: Option<Value>| {
            let f = match frame {
                None | Some(Value::Nil) => t.0.frame_index(None)?,
                Some(v) => t.0.frame_of(&v)?,
            } as u32;
            let sl = t.get()?;
            Ok(sl.key_at(f).or(sl.keys.first()).map(|k| rect_value(k.bounds)))
        });
        // setBounds(x, y, w, h [, frame]) or setBounds(rectangle [, frame])
        m.add_method("setBounds", |_, t, mut a: MultiValue| {
            let r = geometry::take_rect(&mut a, "setBounds")?;
            let f = take_frame(&t.0, &mut a)?;
            let r = slice_rect(r.x, r.y, r.w, r.h)?;
            t.get()?;
            crate::document::slices::set_bounds(t.0.sprite(), t.1, f, r);
            Ok(())
        });
        m.add_method("getCenter", |_, t, frame: Option<Value>| {
            let f = match frame {
                None | Some(Value::Nil) => t.0.frame_index(None)?,
                Some(v) => t.0.frame_of(&v)?,
            } as u32;
            let sl = t.get()?;
            if !sl.nine_patch {
                return Ok(None);
            }
            Ok(sl.key_at(f).or(sl.keys.first()).map(|k| rect_value(k.center)))
        });
        // setCenter(x, y, w, h [, frame]) or setCenter(rectangle [, frame])
        m.add_method("setCenter", |_, t, mut a: MultiValue| {
            let r = geometry::take_rect(&mut a, "setCenter")?;
            let f = take_frame(&t.0, &mut a)?;
            let r = slice_rect(r.x, r.y, r.w, r.h)?;
            let sl = t.get()?;
            crate::document::slices::key_for_edit(sl, f).center = r;
            sl.nine_patch = true;
            Ok(())
        });
        m.add_method("clearCenter", |_, t, ()| {
            t.get()?.nine_patch = false;
            Ok(())
        });
        m.add_method("getPivot", |_, t, frame: Option<Value>| {
            let f = match frame {
                None | Some(Value::Nil) => t.0.frame_index(None)?,
                Some(v) => t.0.frame_of(&v)?,
            } as u32;
            let p = t.get()?.pivot_at(f);
            Ok((p.map(|p| p.0), p.map(|p| p.1)))
        });
        // setPivot(x, y [, frame]) or setPivot(point [, frame]). Without a frame, a slice with no pivot yet gets it on
        // all its keys (so every frame has it); with a frame the pivot starts there.
        m.add_method("setPivot", |_, t, mut a: MultiValue| {
            let p = geometry::take_point(&mut a, "setPivot")?;
            let f = match a.pop_front() {
                None | Some(Value::Nil) => None,
                Some(v) => Some(t.0.frame_of(&v)? as u32),
            };
            if p.x.abs() > 1 << 20 || p.y.abs() > 1 << 20 {
                return Err(Error::runtime("pivot out of range"));
            }
            let sl = t.get()?;
            let p = (p.x as i32, p.y as i32);
            match f {
                Some(f) => sl.set_pivot_at(f, p),
                None => sl.set_pivot_all(p),
            }
            Ok(())
        });
        m.add_method("clearPivot", |_, t, ()| {
            let sl = t.get()?;
            sl.has_pivot = false;
            sl.pivot_from = 0;
            Ok(())
        });
        m.add_method("delete", |_, t, ()| {
            t.get()?;
            t.0.sprite().extra.slices.remove(t.1);
            t.0.invalidate_handles();
            Ok(())
        });
    }
}

const BLEND_NAMES: [&str; 19] = [
    "Normal",
    "Multiply",
    "Screen",
    "Overlay",
    "Darken",
    "Lighten",
    "ColorDodge",
    "ColorBurn",
    "HardLight",
    "SoftLight",
    "Difference",
    "Exclusion",
    "Hue",
    "Saturation",
    "Color",
    "Luminosity",
    "Addition",
    "Subtract",
    "Divide",
];

fn setup(lua: &Lua, ctx: &Rc<Ctx>) -> LuaResult<()> {
    let g = lua.globals();
    // `dofile` / `loadfile` and the file functions of `io` / `os` ask first (see `fs.rs`); `load` is limited
    // to text chunks.
    let load: Function = g.get("load")?;
    g.set(
        "load",
        lua.create_function(move |lua, (chunk, name, _mode, rest): (Value, Option<String>, Value, MultiValue)| {
            let mut args = MultiValue::new();
            args.push_back(chunk);
            args.push_back(match name {
                Some(n) => Value::String(lua.create_string(&n)?),
                None => Value::Nil,
            });
            args.push_back(Value::String(lua.create_string("t")?));
            args.extend(rest);
            load.call::<MultiValue>(args)
        })?,
    )?;

    // Errors raised by API calls reach `pcall` as error objects; hand scripts plain strings
    // so `err:find(...)` and string concatenation work.
    lua.load(
        r#"
        local pcall0, xpcall0, pack, unpack, tostring = pcall, xpcall, table.pack, table.unpack, tostring
        local function plain(r)
          if not r[1] and type(r[2]) == "userdata" then r[2] = tostring(r[2]) end
          return unpack(r, 1, r.n)
        end
        function pcall(f, ...) return plain(pack(pcall0(f, ...))) end
        function xpcall(f, handler, ...)
          local function wrap(e) if type(e) == "userdata" then e = tostring(e) end return handler(e) end
          return plain(pack(xpcall0(f, wrap, ...)))
        end
        "#,
    )
    .set_name("=sandbox")
    .exec()?;

    let print_ctx = ctx.clone();
    g.set(
        "print",
        lua.create_function(move |lua, args: MultiValue| {
            let tostring: Function = lua.globals().get("tostring")?;
            let mut parts = Vec::new();
            for a in args {
                parts.push(tostring.call::<String>(a)?);
            }
            print_ctx.messages.borrow_mut().push(parts.join("\t").replace(MARKER, ""));
            Ok(())
        })?,
    )?;

    let app = lua.create_table()?;
    let c = ctx.clone();
    app.set(
        "alert",
        lua.create_function(move |_, msg: Value| {
            let text = match msg {
                Value::String(s) => s.to_string_lossy(),
                Value::Nil => String::new(),
                other => other.to_string()?,
            };
            c.messages.borrow_mut().push(text.replace(MARKER, ""));
            Ok(())
        })?,
    )?;
    let c = ctx.clone();
    app.set(
        "undoLabel",
        lua.create_function(move |_, text: String| {
            *c.label.borrow_mut() = Some(text);
            Ok(())
        })?,
    )?;
    // Blend modes and tag directions use Aseprite's numbering (BlendMode.* / AniDir.* are the same tables).
    let blend = lua.create_table()?;
    for (i, n) in BLEND_NAMES.iter().enumerate() {
        blend.set(*n, commands::blend_to_aseprite(i as u16))?;
    }
    app.set("blend", blend)?;
    let direction = lua.create_table()?;
    for (i, n) in ["Forward", "Reverse", "PingPong", "PingPongReverse"].iter().enumerate() {
        direction.set(*n, i)?;
    }
    app.set("direction", direction)?;
    // Bumped when the API grows, so scripts can check `app.apiVersion >= 2`. `app.version` is a Version object.
    app.set("apiVersion", commands::API_VERSION)?;

    // `app.activeLayer` / `app.activeFrame` / `app.activeSprite` and the other properties that depend on the
    // documents of the run are served by `sprites::app_get` / `app_set`; assigning `app.activeLayer` or
    // `app.activeFrame` also asks the editor to select that layer and frame when the run ends.
    let meta = lua.create_table()?;
    let c = ctx.clone();
    meta.set(
        "__index",
        lua.create_function(move |lua, (_t, key): (Value, Value)| {
            let name = if let Value::String(s) = &key { s.to_string_lossy() } else { String::new() };
            Ok(sprites::app_get(lua, &c, &name)?.unwrap_or(Value::Nil))
        })?,
    )?;
    let c = ctx.clone();
    meta.set(
        "__newindex",
        lua.create_function(move |_, (t, key, value): (Table, Value, Value)| {
            let name = if let Value::String(s) = &key { s.to_string_lossy() } else { String::new() };
            if sprites::app_set(&c, &name, &value)? {
                return Ok(());
            }
            match name.as_str() {
                "fgColor" | "bgColor" => {
                    let col = u32::from_be_bytes(color::arg(&value)?);
                    if let Some(rt) = c.rt.borrow().as_ref() {
                        if name == "fgColor" {
                            rt.emit_app_set(Some(col), None);
                        } else {
                            rt.emit_app_set(None, Some(col));
                        }
                    }
                }
                _ => t.raw_set(key, value)?,
            }
            Ok(())
        })?,
    )?;
    app.set_metatable(Some(meta))?;
    g.set("app", app)?;

    g.set("color", lua.create_table()?)?;

    g.set("sprite", SpriteH(ctx.clone()))?;
    geometry::install(lua)?;
    image::install(lua, ctx)?;
    color::install(lua, ctx)?;
    sprites::install(lua, ctx)?;
    palettes::install(lua, ctx)?;
    fs::install(lua, ctx)?;
    commands::install(lua, ctx)?;
    framediff::install(lua, ctx)?;
    host::install(lua, ctx)?;
    Ok(())
}

/// Runs `source` against the editor. On success the changes form a single undo step; on failure
/// the document is restored. `active` is the (layer, frame) pair, 0-based, scripts default to.
pub fn run(e: &mut Editor, source: &str, active: (usize, usize), limits: Limits) -> Outcome {
    run_with(e, source, active, limits, &NEVER)
}

/// `run` with the layer and frame the C ABI defaults to (the top drawable layer, frame 1): the entry point
/// for hosts that only need to choose the `Limits` (e.g. a longer timeout for a script that builds big art).
pub fn run_with_limits(e: &mut Editor, source: &str, limits: Limits) -> Outcome {
    let layer = e.sprite.layers.iter().rposition(|l| !l.is_group()).unwrap_or(0);
    run(e, source, (layer, 0), limits)
}

/// Like `run`; the script stops with "script cancelled" soon after `cancel` becomes true.
pub fn run_with(
    e: &mut Editor,
    source: &str,
    active: (usize, usize),
    limits: Limits,
    cancel: &'static AtomicBool,
) -> Outcome {
    run_in(e, source, active, limits, cancel, &RunConfig::default())
}

/// Like `run_with`, with the script's name, the document's file and run-specific permissions.
pub fn run_in(
    e: &mut Editor,
    source: &str,
    active: (usize, usize),
    limits: Limits,
    cancel: &'static AtomicBool,
    cfg: &RunConfig,
) -> Outcome {
    let before = e.sprite.clone();
    let (w, h) = (e.sprite.width, e.sprite.height);
    let old_mask = e.sel.mask.clone().filter(|m| m.w == w && m.h == h);
    let ctx = Rc::new(Ctx::new(e));
    ctx.layer.set(active.0.min(e.sprite.layers.len() - 1));
    ctx.frame.set(active.1.min(e.sprite.frame_count() - 1));
    *ctx.mask.borrow_mut() = old_mask.clone();
    *ctx.shared.original.borrow_mut() = Rc::downgrade(&ctx);
    *ctx.doc.path.borrow_mut() = cfg.doc_path.clone();
    *ctx.shared.access.script.borrow_mut() = cfg.script_name.clone();
    *ctx.shared.access.extra.borrow_mut() = cfg.allowed_paths.clone();
    ctx.undo.reset(before.clone(), old_mask.clone());
    let result = exec(&ctx, source, limits, cancel);
    let messages = ctx.messages.borrow().clone();
    let label = ctx.label.borrow().clone();
    let new_mask = ctx.mask.borrow().clone();
    // The layer or frame count may have changed after the script chose its selection.
    let select = ctx.selected.get().then(|| {
        let s = &e.sprite;
        let mut l = ctx.layer.get().min(s.layers.len() - 1);
        if s.layers[l].is_group() {
            l = s.layers.iter().rposition(|x| !x.is_group()).unwrap_or(0);
        }
        (l, ctx.frame.get().min(s.frame_count() - 1))
    });
    let outcome = match result {
        Ok(()) => {
            conform_changed_cels(&mut e.sprite, &before);
            e.sprite.flush_links();
            let docs = sprites::finish(&ctx, true);
            if ctx.shared.close_original.get() {
                // The document is closed: nothing of it is committed.
                e.sprite = before;
            } else if e.sprite != before || new_mask != old_mask {
                let after = std::mem::replace(&mut e.sprite, before);
                e.checkpoint(); // records the old selection and sprite
                e.sprite = after;
                e.sel.mask = new_mask;
                e.label_last(label.as_deref().unwrap_or("Script"));
            }
            Outcome {
                messages,
                select,
                error: None,
                new_documents: docs.new_documents,
                active: docs.active,
                close_original: docs.close_original,
                original_path: docs.original_path,
                original_saved: docs.original_saved,
            }
        }
        Err(err) => {
            e.sprite = before;
            sprites::finish(&ctx, false);
            Outcome::failed(messages, clean_error(&err))
        }
    };
    drop(ctx);
    outcome
}

/// Snaps the cels a script created or modified to the sprite's color mode (Indexed, Grayscale).
fn conform_changed_cels(s: &mut Sprite, before: &Sprite) {
    if s.color_mode == color_mode::ColorMode::Rgba {
        return;
    }
    let old: HashSet<*const crate::document::Cel> = before.cels.iter().flatten().flatten().map(Arc::as_ptr).collect();
    let changed: Vec<(usize, usize)> = (0..s.cels.len())
        .flat_map(|f| (0..s.layers.len()).map(move |l| (f, l)))
        .filter(|&(f, l)| s.cels[f].get(l).and_then(|c| c.as_ref()).is_some_and(|c| !old.contains(&Arc::as_ptr(c))))
        .collect();
    for (f, l) in changed {
        color_mode::conform_cel(s, l, f);
    }
}

fn clean_error(e: &Error) -> String {
    let s = e.to_string();
    let s = s.split("\nstack traceback:").next().unwrap_or(&s);
    s.trim().to_string()
}

/// A Lua state with the sandboxed standard libraries and the Pixmith API installed.
fn new_sandbox(ctx: &Rc<Ctx>, memory: usize) -> LuaResult<Lua> {
    let libs =
        StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH | StdLib::COROUTINE | StdLib::IO | StdLib::OS;
    let lua = Lua::new_with(libs, LuaOptions::default())?;
    lua.set_memory_limit(memory)?;
    setup(&lua, ctx)?;
    Ok(lua)
}

fn exec(ctx: &Rc<Ctx>, source: &str, limits: Limits, cancel: &'static AtomicBool) -> LuaResult<()> {
    let lua = new_sandbox(ctx, limits.memory)?;

    const STEP: u32 = 1000;
    let started = Instant::now();
    let executed = std::cell::Cell::new(0u64);
    let shared = ctx.shared.clone();
    lua.set_hook(mlua::HookTriggers::new().every_nth_instruction(STEP), move |_, _| {
        executed.set(executed.get() + STEP as u64);
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::runtime("script cancelled"));
        }
        if executed.get() > limits.instructions {
            return Err(Error::runtime("script stopped: instruction limit exceeded"));
        }
        if started.elapsed().saturating_sub(shared.waited.get()) > limits.time {
            return Err(Error::runtime("script stopped: time limit exceeded"));
        }
        Ok(VmState::Continue)
    })?;
    lua.load(source).set_name("=script").set_mode(ChunkMode::Text).exec()
}

/// Example scripts shipped with the app (copied from `scripts/lua-examples/`).
pub const EXAMPLES: [(&str, &str); 6] = [
    ("outline-layer.lua", include_str!("../../scripts/lua-examples/outline-layer.lua")),
    ("color-ramp.lua", include_str!("../../scripts/lua-examples/color-ramp.lua")),
    ("flip-all-frames.lua", include_str!("../../scripts/lua-examples/flip-all-frames.lua")),
    ("palette-swap.lua", include_str!("../../scripts/lua-examples/palette-swap.lua")),
    ("tag-every-4-frames.lua", include_str!("../../scripts/lua-examples/tag-every-4-frames.lua")),
    ("palette-ramp-dialog.lua", include_str!("../../scripts/lua-examples/palette-ramp-dialog.lua")),
];

// MARK: C ABI

#[no_mangle]
pub extern "C" fn pixmith_example_script_count() -> u32 {
    crate::ffi::guarded("pixmith_example_script_count", std::ptr::null_mut(), || EXAMPLES.len() as u32)
}

/// File name of an example script; release with `pixmith_string_free`.
#[no_mangle]
pub extern "C" fn pixmith_example_script_name(index: u32) -> *mut c_char {
    crate::ffi::guarded("pixmith_example_script_name", std::ptr::null_mut(), || {
        EXAMPLES
            .get(index as usize)
            .and_then(|e| CString::new(e.0).ok())
            .map_or(std::ptr::null_mut(), CString::into_raw)
    })
}

/// Source of an example script; release with `pixmith_string_free`.
#[no_mangle]
pub extern "C" fn pixmith_example_script_source(index: u32) -> *mut c_char {
    crate::ffi::guarded("pixmith_example_script_source", std::ptr::null_mut(), || {
        EXAMPLES
            .get(index as usize)
            .and_then(|e| CString::new(e.1).ok())
            .map_or(std::ptr::null_mut(), CString::into_raw)
    })
}

unsafe fn run_ffi(
    doc: *mut Editor,
    source: *const c_char,
    active: Option<(usize, usize)>,
    out: *mut *mut c_char,
) -> bool {
    run_ffi_limits(doc, source, active, Limits::default(), out)
}

unsafe fn run_ffi_limits(
    doc: *mut Editor,
    source: *const c_char,
    active: Option<(usize, usize)>,
    limits: Limits,
    out: *mut *mut c_char,
) -> bool {
    run_ffi_cfg(doc, source, active, limits, &RunConfig::default(), out)
}

unsafe fn run_ffi_cfg(
    doc: *mut Editor,
    source: *const c_char,
    active: Option<(usize, usize)>,
    limits: Limits,
    cfg: &RunConfig,
    out: *mut *mut c_char,
) -> bool {
    host::clear_sync_docs();
    if !out.is_null() {
        *out = std::ptr::null_mut();
    }
    let (Some(e), false) = (doc.as_mut(), source.is_null()) else { return false };
    e.dirty.mark_all();
    let src = CStr::from_ptr(source).to_string_lossy().into_owned();
    let active = active.unwrap_or_else(|| (e.sprite.layers.iter().rposition(|l| !l.is_group()).unwrap_or(0), 0));
    let mut outcome = run_in(e, &src, active, limits, &CANCEL, cfg);
    host::store_sync_docs(&mut outcome);
    if !out.is_null() {
        let mut lines = outcome.messages.clone();
        if let Some((l, f)) = outcome.select {
            lines.push(format!("{SELECT_LINE} layer={} frame={}", l + 1, f + 1));
        }
        *out = CString::new(lines.join("\n").replace('\0', "")).map_or(std::ptr::null_mut(), CString::into_raw);
    }
    match outcome.error {
        None => true,
        Some(msg) => {
            crate::ffi::set_error(msg);
            false
        }
    }
}

/// Asks the running script to stop (true) or clears the request before a new run (false). Safe to call
/// from the UI thread while a script runs on another thread.
#[no_mangle]
pub extern "C" fn pixmith_script_cancel(cancel: bool) {
    crate::ffi::guarded("pixmith_script_cancel", std::ptr::null_mut(), || {
        CANCEL.store(cancel, Ordering::Relaxed);
    })
}

/// Runs a Lua script (one undo step). Alerts and `print` output come back in `*out_messages`,
/// newline separated (release with `pixmith_string_free`). On failure the document is unchanged and
/// `pixmith_last_error` holds the error. Scripts default to the top drawable layer and frame 1.
#[no_mangle]
pub unsafe extern "C" fn pixmith_run_script(
    doc: *mut Editor,
    source: *const c_char,
    out_messages: *mut *mut c_char,
) -> bool {
    crate::ffi::guarded("pixmith_run_script", doc, || run_ffi(doc, source, None, out_messages))
}

/// Like `pixmith_run_script`, with the active layer and frame (0-based) scripts default to.
#[no_mangle]
pub unsafe extern "C" fn pixmith_run_script_at(
    doc: *mut Editor,
    source: *const c_char,
    layer: u32,
    frame: u32,
    out_messages: *mut *mut c_char,
) -> bool {
    crate::ffi::guarded("pixmith_run_script_at", doc, || {
        run_ffi(doc, source, Some((layer as usize, frame as usize)), out_messages)
    })
}

/// Like `pixmith_run_script_at`, with the script's name (shown in permission requests, and the key of "always allow";
/// NULL = "script") and the file of the document (`sprite.filename`; may be NULL). Documents the script created or
/// opened are available afterwards from `pixmith_script_outcome(0, 0)` and `pixmith_script_take_doc(0, 0, i)`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_run_script_named(
    doc: *mut Editor,
    source: *const c_char,
    layer: u32,
    frame: u32,
    script_name: *const c_char,
    doc_path: *const c_char,
    out_messages: *mut *mut c_char,
) -> bool {
    crate::ffi::guarded("pixmith_run_script_named", doc, || {
        let text = |p: *const c_char| (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned());
        let cfg = RunConfig {
            script_name: text(script_name).unwrap_or_else(|| "script".into()),
            doc_path: text(doc_path).map(std::path::PathBuf::from),
            allowed_paths: Vec::new(),
        };
        run_ffi_cfg(doc, source, Some((layer as usize, frame as usize)), Limits::default(), &cfg, out_messages)
    })
}

/// Like `pixmith_run_script_at`, with explicit budgets: `instructions` VM instructions (0 = the default of
/// 300 million), `time_ms` milliseconds (0 = the default of 10 s) and `memory_mb` MiB of Lua memory
/// (0 = the default of 128). Meant for hosts such as an MCP server that run long scripts.
#[no_mangle]
pub unsafe extern "C" fn pixmith_run_script_limits(
    doc: *mut Editor,
    source: *const c_char,
    layer: u32,
    frame: u32,
    instructions: u64,
    time_ms: u64,
    memory_mb: u32,
    out_messages: *mut *mut c_char,
) -> bool {
    crate::ffi::guarded("pixmith_run_script_limits", doc, || {
        let d = Limits::default();
        let limits = Limits {
            instructions: if instructions == 0 { d.instructions } else { instructions },
            time: if time_ms == 0 { d.time } else { Duration::from_millis(time_ms) },
            memory: if memory_mb == 0 { d.memory } else { (memory_mb as usize).saturating_mul(1 << 20) },
        };
        run_ffi_limits(doc, source, Some((layer as usize, frame as usize)), limits, out_messages)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];

    fn run_src(e: &mut Editor, src: &str) -> Outcome {
        run(e, src, (0, 0), Limits::default())
    }

    fn ok(e: &mut Editor, src: &str) -> Outcome {
        let o = run_src(e, src);
        assert_eq!(o.error, None, "script failed");
        o
    }

    #[test]
    fn tween_rig_from_lua() {
        let mut s = Sprite::new(40, 40);
        s.add_frame(0, false);
        s.add_frame(1, false);
        s.add_layer(0);
        s.draw_line(0, 0, 10, 10, 10, 10, RED, 1);
        s.draw_line(1, 0, 20, 10, 20, 10, [0, 255, 0, 255], 1);
        let mut e = Editor::new(s);
        ok(
            &mut e,
            r#"
            local parent, child = sprite.layers[1], sprite.layers[2]
            assert(child.tweenParent == nil and parent.tweenParent == nil)
            parent:setTweenKey("x", 1, 0)
            parent:setTweenKey("x", 3, 4)
            parent:setTweenKey("rotation", 1, 0, "hold")
            parent:setTweenKey("rotation", 3, 90)
            parent:setTweenPivot(10, 10)
            assert(parent:tweenTransform(1).rotation == 0)
            child.tweenParent = parent
            assert(child.tweenParent == parent and #parent.children == 0)
            -- the child has no keys, it follows: at frame 3 the parent is translated by (4,0) and turned 90 degrees about (10,10)
            local t = child:tweenTransform(3)
            assert(math.abs(t.rotation - 90) < 1e-9, t.rotation)
            assert(math.abs(t.matrix[1]) < 1e-9 and math.abs(t.matrix[2] - 1) < 1e-9)
            -- the child's pixel (20,10) has its center at (20.5, 10.5): (10.5, 0.5) from the pivot, turned to (-0.5, 10.5), plus 4 to the right
            local x, y = child:tweenMap(3, 20.5, 10.5)
            assert(math.abs(x - 13.5) < 1e-9 and math.abs(y - 20.5) < 1e-9, x .. "," .. y)
            local x2, y2 = child:tweenMap(20.5, 10.5)   -- the active frame is 1: nothing moved
            assert(x2 == 20.5 and y2 == 10.5)
            -- cycles and non-layers are errors
            assert(not pcall(function() parent.tweenParent = child end))
            assert(not pcall(function() parent.tweenParent = parent end))
            assert(not pcall(function() parent:setTweenKey("shear", 1, 0) end))
            child.tweenParent = nil
            assert(child.tweenParent == nil and child:tweenTransform(3) == nil)
            child.tweenParent = parent
            "#,
        );
        assert_eq!(crate::gamedev::rig::parent_of(&e.sprite, 1), Some(0));
        let img = e.sprite.render(2);
        let at = |x: usize, y: usize| &img[(y * 40 + x) * 4..][..4];
        assert_eq!(at(13, 20), &[0, 255, 0, 255]);
        assert_eq!(e.history_len().0, 1, "the whole script is one undo step");
    }

    #[test]
    fn reads_and_writes_pixels_layers_and_frames() {
        let mut e = Editor::new(Sprite::new(4, 3));
        let o = ok(
            &mut e,
            r#"
            assert(sprite.width == 4 and sprite.height == 3)
            sprite:setPixel(1, 2, 0xFF0000FF)
            assert(sprite:getPixel(1, 2) == 0xFF0000FF)
            assert(sprite:getPixel(0, 0) == 0)
            assert(sprite:getPixel(99, 0) == 0)          -- outside the canvas
            local top = sprite:newLayer("Top")
            top.opacity = 128
            top.blendMode = app.blend.Multiply
            top.visible = false
            local f = sprite:newFrame(true)
            f.duration = 250
            assert(sprite.layerCount == 2 and sprite.frameCount == 2)
            assert(sprite:getPixel(1, 2, 1, 2) == 0xFF0000FF, "copied frame keeps pixels")
            sprite:setPixel(0, 0, color.rgba(0, 255, 0), top.index, 2)
            print(sprite.layers[2].name, sprite.frames[2].duration)
            "#,
        );
        assert_eq!(o.messages, vec!["Top\t250"]);
        let s = &e.sprite;
        assert_eq!((s.layers[1].opacity, s.layers[1].blend_mode, s.layers[1].visible), (128, 1, false));
        assert_eq!(s.durations, vec![100, 250]);
        assert_eq!(&s.cels[1][1].as_ref().unwrap().pixels[..4], &[0, 255, 0, 255]);
        assert_eq!(&s.cels[0][0].as_ref().unwrap().pixels[(2 * 4 + 1) * 4..][..4], &RED);
    }

    #[test]
    fn setting_a_pivot_without_a_frame_covers_every_key() {
        let mut e = Editor::new(Sprite::new(16, 16));
        ok(
            &mut e,
            r#"
            for _ = 1, 5 do sprite:newFrame() end
            -- without a frame, a slice with no pivot gets it on every key; later keys inherit it
            local u = sprite:newSlice("u", 0, 0, 2, 2)
            app.activeFrame = 4
            u:setPivot(Point(9, 8))
            u:setBounds(1, 1, 2, 2, 3)
            local fx, fy = u:getPivot(1)
            assert(fx == 9 and fy == 8 and u.pivot == Point(9, 8))
            local gx, gy = u:getPivot(3)
            assert(gx == 9 and gy == 8)
            -- with a pivot already, the frameless call changes frame 1 only
            u:setPivot(1, 2)
            assert(select(1, u:getPivot(1)) == 1 and select(1, u:getPivot(3)) == 9)
            "#,
        );
    }

    #[test]
    fn integer_fields_explain_a_fractional_value() {
        let mut e = Editor::new(Sprite::new(2, 2));
        let o = run_src(&mut e, "sprite.frames[1].duration = 0.1");
        let err = o.error.unwrap_or_default();
        assert!(err.contains("frame.duration is whole milliseconds (1-65535); got 0.1"), "{err}");
        let o = run_src(&mut e, "sprite.layers[1].opacity = 12.5");
        let err = o.error.unwrap_or_default();
        assert!(err.contains("layer.opacity must be a whole number, got 12.5"), "{err}");
        ok(
            &mut e,
            "sprite.frames[1].duration = 120.0 sprite.layers[1].opacity = 200 assert(sprite.frames[1].duration == 120)",
        );
        assert_eq!(e.sprite.durations, vec![120]);
    }

    #[test]
    fn frame_diff_reports_counts_bounds_and_a_mask() {
        let mut e = Editor::new(Sprite::new(8, 4));
        let o = ok(
            &mut e,
            r#"
            -- Top half of a body moves one pixel right, the bottom half stays: a torn character.
            local top = sprite:newLayer("Top")
            sprite:newFrame()
            for y = 0, 3 do
              for x = 1, 4 do
                local dx = (y < 2) and 1 or 0
                sprite:setPixel(x, y, 0xFF0000FF, 1, 1)
                sprite:setPixel(x + dx, y, 0xFF0000FF, 1, 2)
              end
            end
            sprite:setPixel(7, 3, 0x0000FFFF, top.index, 2)   -- only on the top layer, frame 2
            local d = app.frameDiff{ frame = 2, mask = true }
            print(d.frame, d.previous, d.changed, d.appeared, d.disappeared, d.total)
            print(d.bounds.x, d.bounds.y, d.bounds.width, d.bounds.height)
            print(d.mask.width, d.mask.height, d.mask:getPixel(5, 0), d.mask:getPixel(1, 0), d.mask:getPixel(2, 0))
            local l = app.frameDiff{ frame = 2, layer = 2 }
            print(l.layer, l.total, l.mask)
            local w = app.frameDiff{ frame = 1 }
            local n = app.frameDiff{ frame = 1, wrap = false }
            print(w.previous, w.total, n.previous, n.total, n.bounds)
            "#,
        );
        assert_eq!(
            o.messages,
            vec!["2\t1\t0\t3\t2\t5", "1\t0\t7\t4", "8\t4\t4278255360\t4278190335\t0", "2\t1\tnil", "2\t5\tnil\t0\tnil",]
        );
        let o = run_src(&mut e, "app.frameDiff{ frame = 9 }");
        assert!(o.error.is_some_and(|m| m.contains("frame 9 does not exist")));
        let o = run_src(&mut e, "sprite:newLayer('x') local g = sprite:newGroup('g') app.frameDiff{ layer = g }");
        assert!(o.error.is_some_and(|m| m.contains("groups have no pixels")));
    }

    #[test]
    fn whole_run_is_one_undo_step_with_a_label() {
        let mut e = Editor::new(Sprite::new(2, 2));
        ok(
            &mut e,
            "for i = 0, 1 do sprite:setPixel(i, i, 0x00FF00FF) end sprite:newLayer('x') app.undoLabel('My script')",
        );
        assert_eq!(e.history_len().0, 1);
        assert_eq!(e.history_label(0), Some("My script"));
        assert_eq!(e.sprite.layers.len(), 2);
        assert!(e.undo());
        assert_eq!(e.sprite.layers.len(), 1);
        assert_eq!(e.sprite.render(0), vec![0; 16]);
        assert!(!e.undo(), "nothing else to undo");
        // Unlabeled runs are named "Script"; runs that change nothing add no history.
        ok(&mut e, "sprite:setPixel(0, 0, 0xFFFFFFFF)");
        assert_eq!(e.history_label(0), Some("Script"));
        let n = e.history_len().0;
        ok(&mut e, "app.alert('hi')");
        assert_eq!(e.history_len().0, n);
    }

    #[test]
    fn failing_scripts_leave_the_document_alone() {
        let mut e = Editor::new(Sprite::new(2, 2));
        let before = e.sprite.clone();
        let o =
            run_src(&mut e, "sprite:setPixel(0, 0, 0xFF0000FF) sprite:newLayer('x') app.alert('before') error('boom')");
        assert!(o.error.as_deref().unwrap().contains("boom"), "{:?}", o.error);
        assert_eq!(o.messages, vec!["before"]);
        assert_eq!(e.sprite, before);
        assert_eq!(e.history_len().0, 0);

        for bad in [
            "sprite:getPixel(0, 0, 5)",
            "sprite:setPixel(0, 0, -1)",
            "sprite:setPixel(0, 0, 0x1FFFFFFFF)",
            "sprite.layers[1].blendMode = 40",
            "sprite:getPaletteColor(999)",
            "this is not lua",
            "sprite:setPixel('a', 0, 1)",
        ] {
            let o = run_src(&mut e, bad);
            assert!(o.error.is_some(), "{bad}");
            assert_eq!(e.sprite, before);
        }
    }

    #[test]
    fn sandbox_has_no_system_access() {
        let mut e = Editor::new(Sprite::new(1, 1));
        let o = ok(
            &mut e,
            r#"
            for _, name in ipairs({ "package", "debug", "require" }) do
              assert(_G[name] == nil, name .. " must not exist")
            end
            -- io, os, dofile and loadfile exist but ask before touching anything (see the access tests).
            assert(io.stdin == nil and os.exit and os.setlocale == nil)
            assert(not pcall(io.open, "/etc/hosts"), "no permission handler: denied")
            assert(not pcall(os.execute, "echo hi"))
            assert(not pcall(os.remove, "/tmp/pixmith-never"))
            assert(not pcall(dofile, "/etc/hosts") and not pcall(loadfile, "/etc/hosts"))
            assert(os.getenv("PIXMITH_SECRET_NOT_ALLOWED") == nil and not pcall(os.exit))
            assert(load("return 1 + 1")() == 2, "text chunks load")
            local f = load(string.dump(function() return 1 end))
            assert(f == nil, "bytecode must not load")
            local g = load(string.dump(function() return 1 end), "x", "b")
            assert(g == nil, "the mode argument cannot re-enable bytecode")
            assert(math.floor(2.5) == 2 and #string.rep("ab", 3) == 6 and utf8.char(65) == "A")
            "#,
        );
        assert!(o.messages.is_empty());
    }

    #[test]
    fn scripts_can_be_cancelled_from_another_thread() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let mut e = Editor::new(Sprite::new(1, 1));
        let stopper = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(50));
            FLAG.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let o = run_with(&mut e, "local n = 0 while true do n = n + 1 end", (0, 0), Limits::default(), &FLAG);
        stopper.join().unwrap();
        assert!(o.error.unwrap().contains("script cancelled"));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(e.history_len().0, 0);
    }

    #[test]
    fn runaway_scripts_are_stopped() {
        let mut e = Editor::new(Sprite::new(1, 1));
        let small = Limits { instructions: 1_000_000, ..Limits::default() };
        let o = run(&mut e, "while true do end", (0, 0), small);
        assert!(o.error.unwrap().contains("instruction limit"));

        let slow = Limits { instructions: u64::MAX, time: Duration::from_millis(100), ..Limits::default() };
        let started = Instant::now();
        let o = run(&mut e, "local n = 0 while true do n = n + 1 end", (0, 0), slow);
        assert!(o.error.unwrap().contains("time limit"));
        assert!(started.elapsed() < Duration::from_secs(5));

        let tiny = Limits { memory: 8 << 20, ..Limits::default() };
        let o = run(&mut e, "local s = string.rep('x', 1 << 30) return #s", (0, 0), tiny);
        assert!(o.error.is_some());
        let o = run(&mut e, "local t = {} while true do t[#t + 1] = {} end", (0, 0), tiny);
        assert!(o.error.unwrap().to_lowercase().contains("memory"));
    }

    #[test]
    fn palette_calls_work() {
        let mut e = Editor::new(Sprite::new(2, 1));
        let n = e.sprite.palette.len();
        ok(
            &mut e,
            &format!(
                r#"
            assert(sprite.paletteSize == {n})
            local i = sprite:addPaletteColor(0x123456FF)
            assert(i == {n} and sprite:getPaletteColor(i) == 0x123456FF)
            assert(sprite:addPaletteColor(0x123456FF) == i, "duplicates are reused")
            sprite:setPaletteColor(0, 0x010203FF)
            sprite:setPaletteColor(sprite.paletteSize, 0xABCDEFFF)  -- appends
            "#
            ),
        );
        assert_eq!(e.sprite.palette[0], [1, 2, 3, 255]);
        assert_eq!(e.sprite.palette.len(), n + 2);
        assert_eq!(e.sprite.palette[n + 1], [0xAB, 0xCD, 0xEF, 255]);
    }

    #[test]
    fn set_pixel_edits_tilemap_layers_through_their_tiles() {
        let mut e = Editor::new(Sprite::new(8, 8));
        let l = crate::document::tilemap::add_tilemap_layer(&mut e.sprite, 0, 4, 4);
        let o = run(&mut e, &format!("sprite:setPixel(1, 1, 0xFF0000FF, {})", l + 1), (0, 0), Limits::default());
        assert_eq!(o.error, None);
        assert_eq!(e.sprite.tilesets[0].tiles.len(), 2);
        assert_eq!(&e.sprite.tilesets[0].tiles[1][(4 + 1) * 4..][..4], &RED);
        let o = run_src(&mut e, "sprite:setPixel(0, 0, 1, 99)");
        assert!(o.error.unwrap().contains("layer 99"));
    }

    #[test]
    fn active_layer_and_frame_are_the_defaults() {
        let mut s = Sprite::new(2, 2);
        s.add_layer(0);
        s.add_frame(0, false);
        let mut e = Editor::new(s);
        let o = run(
            &mut e,
            "sprite:setPixel(1, 1, 0x00FF00FF) print(sprite.activeLayer, sprite.activeFrame)",
            (1, 1),
            Limits::default(),
        );
        assert_eq!(o.error, None);
        assert_eq!(o.messages, vec!["2\t2"]);
        assert_eq!(&e.sprite.cels[1][1].as_ref().unwrap().pixels[12..16], &[0, 255, 0, 255]);
        assert!(e.sprite.cels[0][0].is_none());
    }

    fn solid_blob() -> Editor {
        let mut s = Sprite::new(5, 5);
        s.draw_line(0, 0, 2, 2, 2, 2, RED, 1);
        Editor::new(s)
    }

    #[test]
    fn example_outline_layer() {
        let mut e = solid_blob();
        let o = ok(&mut e, include_str!("../../scripts/lua-examples/outline-layer.lua"));
        assert_eq!(o.messages, vec!["Outlined 4 pixels"]);
        let px = e.sprite.render(0);
        let at = |x: usize, y: usize| px[(y * 5 + x) * 4..][..4].to_vec();
        assert_eq!(at(2, 2), RED.to_vec());
        for (x, y) in [(1, 2), (3, 2), (2, 1), (2, 3)] {
            assert_eq!(at(x, y), vec![0, 0, 0, 255]);
        }
        assert_eq!(at(1, 1), vec![0; 4], "diagonals are not outlined");
        assert_eq!(e.history_label(0), Some("Outline layer"));
    }

    #[test]
    fn example_color_ramp() {
        let mut e = Editor::new(Sprite::new(5, 2));
        ok(&mut e, include_str!("../../scripts/lua-examples/color-ramp.lua"));
        assert_eq!(e.sprite.layers.len(), 2);
        assert_eq!(e.sprite.layers[1].name, "Color ramp");
        let px = &e.sprite.cels[0][1].as_ref().unwrap().pixels;
        assert_eq!(&px[..4], &[0x1D, 0x2B, 0x53, 255]);
        assert_eq!(&px[4 * 4..][..4], &[0xFF, 0xA3, 0x00, 255]);
        assert_eq!(&px[(5 + 4) * 4..][..4], &[0xFF, 0xA3, 0x00, 255]);
        let mid = &px[2 * 4..][..4];
        assert!(mid[0] > 0x1D && mid[0] < 0xFF);
    }

    #[test]
    fn example_flip_all_frames() {
        let mut s = Sprite::new(4, 1);
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        s.add_frame(0, true);
        s.link_cels(0, 0, 1);
        let top = s.add_layer(0);
        s.draw_line(top, 1, 1, 0, 1, 0, [0, 0, 255, 255], 1);
        let mut e = Editor::new(s);
        let o = ok(&mut e, include_str!("../../scripts/lua-examples/flip-all-frames.lua"));
        // Layer 1's two frames share one image, which is flipped once; the other layer has one cel.
        assert_eq!(o.messages, vec!["Flipped 2 cels"]);
        for f in 0..2 {
            assert_eq!(&e.sprite.cels[f][0].as_ref().unwrap().pixels[12..16], &RED);
            assert_eq!(&e.sprite.cels[f][0].as_ref().unwrap().pixels[..4], &[0; 4]);
        }
        assert_eq!(&e.sprite.cels[1][1].as_ref().unwrap().pixels[8..12], &[0, 0, 255, 255]);
        // Running it again restores the original.
        ok(&mut e, include_str!("../../scripts/lua-examples/flip-all-frames.lua"));
        assert_eq!(&e.sprite.cels[0][0].as_ref().unwrap().pixels[..4], &RED);
    }

    #[test]
    fn c_abi_reports_messages_and_errors() {
        unsafe {
            let doc = Box::into_raw(Box::new(Editor::new(Sprite::new(2, 2))));
            let mut out: *mut c_char = std::ptr::null_mut();
            let src = CString::new("app.alert('one') print('two') sprite:setPixel(0, 0, 0xFF0000FF)").unwrap();
            assert!(pixmith_run_script(doc, src.as_ptr(), &mut out));
            assert_eq!(CStr::from_ptr(out).to_str().unwrap(), "one\ntwo");
            crate::ffi::pixmith_string_free(out);
            let e = &mut *doc;
            assert_eq!(e.history_len().0, 1);

            let bad = CString::new("error('nope')").unwrap();
            assert!(!pixmith_run_script_at(doc, bad.as_ptr(), 0, 0, &mut out));
            assert!(CStr::from_ptr(crate::ffi::pixmith_last_error()).to_str().unwrap().contains("nope"));
            crate::ffi::pixmith_string_free(out);
            assert!(!pixmith_run_script(std::ptr::null_mut(), src.as_ptr(), &mut out));
            assert!(!pixmith_run_script(doc, std::ptr::null(), std::ptr::null_mut()));
            drop(Box::from_raw(doc));
        }
    }

    // MARK: Structure, tags, selection, cels, bulk pixels

    fn px_at(s: &Sprite, l: usize, f: usize, x: usize, y: usize) -> [u8; 4] {
        s.cels[f][l].as_ref().map_or([0; 4], |c| c.pixels[(y * s.width as usize + x) * 4..][..4].try_into().unwrap())
    }

    #[test]
    fn layers_can_be_created_nested_inspected_and_deleted() {
        let mut e = Editor::new(Sprite::new(2, 2));
        let o = ok(
            &mut e,
            r#"
            local g = sprite:newGroup("Props")
            assert(g.isGroup and g.parent == nil and #g.children == 0 and g.level == 0)
            local a = sprite:newLayer("A")
            assert(a.index == 3 and a.parent == nil)
            assert(#sprite.layers == 3 and sprite.layerCount == 3)
            print(g.name, a.name)
            "#,
        );
        assert_eq!(o.messages, vec!["Props\tA"]);
        // Make "A" a child of the group by hand, then look at it from Lua.
        e.sprite.move_into_group(2, 1);
        ok(
            &mut e,
            r#"
            local g, a = sprite.layers[2], sprite.layers[3]
            assert(a.level == 1 and a.parent.name == "Props" and a.parent.index == 2)
            assert(#g.children == 1 and g.children[1].name == "A")
            -- a new layer goes on top of the stack, never into the group
            local top = sprite:newLayer("Top")
            assert(top.level == 0 and top.index == 4)
            local newest = sprite:newGroup("G2")
            assert(newest.level == 0 and newest.index == 5)
            "#,
        );
        // Deleting a group removes its children; handles taken before are stale afterwards.
        let o = ok(
            &mut e,
            r#"
            local old = sprite.layers[1]
            sprite:deleteLayer(2)
            assert(sprite.layerCount == 3)
            assert(sprite.layers[2].name == "Top")
            local ok, err = pcall(function() return old.name end)
            assert(not ok and err:find("stale"), tostring(err))
            "#,
        );
        assert!(o.error.is_none());
        assert_eq!(e.sprite.layers.len(), 3);
        assert_eq!(e.history_label(e.history_len().0 - 1), Some("Script"));
        assert!(e.undo());
        assert_eq!(e.sprite.layers.len(), 5, "deleting is one undo step");

        for bad in ["sprite:deleteLayer(99)", "sprite:deleteLayer(0)"] {
            assert!(run_src(&mut e, bad).error.is_some(), "{bad}");
        }
        let mut one = Editor::new(Sprite::new(1, 1));
        let o = run_src(&mut one, "sprite:deleteLayer(1)");
        assert!(o.error.unwrap().contains("last drawable"));
        assert_eq!(one.sprite.layers.len(), 1);
    }

    #[test]
    fn move_layer_steps_through_the_stack() {
        let mut e = Editor::new(Sprite::new(1, 1));
        ok(&mut e, "sprite:newLayer('B') sprite:newLayer('C') sprite:newLayer('D')");
        let names = |e: &Editor| e.sprite.layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
        ok(&mut e, "assert(sprite:moveLayer(1, 3) == 3)");
        assert_eq!(names(&e)[2], "Layer 1");
        ok(&mut e, "assert(sprite:moveLayer(3, 1) == 1)");
        assert_eq!(names(&e)[0], "Layer 1");
        ok(&mut e, "assert(sprite:moveLayer(2, 2) == 2)");
        assert!(run_src(&mut e, "sprite:moveLayer(1, 9)").error.is_some());
        // A group counts as one step, children included.
        let mut g = Editor::new(Sprite::new(1, 1));
        ok(&mut g, "local grp = sprite:newGroup('G') sprite:newLayer('X')");
        g.sprite.move_into_group(0, 1);
        ok(&mut g, "assert(sprite:moveLayer(1, 2) >= 2)");
        assert!(g.undo());
        assert_eq!(g.sprite.layers.len(), 3);
    }

    #[test]
    fn frames_can_be_duplicated_and_deleted_and_tags_follow() {
        let mut e = Editor::new(Sprite::new(2, 1));
        let o = ok(
            &mut e,
            r#"
            sprite:setPixel(0, 0, 0xFF0000FF)
            sprite:newFrame()
            sprite:newFrame()
            sprite.frames[1].duration = 40
            local dup = sprite:duplicateFrame(1)
            assert(dup.index == 2 and dup.duration == 40 and sprite.frameCount == 4)
            assert(sprite:getPixel(0, 0, 1, 2) == 0xFF0000FF, "duplicate keeps pixels")
            local old = sprite.frames[4]
            sprite:deleteFrame(3)
            assert(sprite.frameCount == 3)
            local ok, err = pcall(function() return old.duration end)
            assert(not ok and err:find("stale"))
            print(sprite.frames[2].duration)
            "#,
        );
        assert_eq!(o.messages, vec!["40"]);
        assert_eq!(e.sprite.frame_count(), 3);
        let mut one = Editor::new(Sprite::new(1, 1));
        let o = run_src(&mut one, "sprite:deleteFrame(1)");
        assert!(o.error.unwrap().contains("last frame"));
        assert!(run_src(&mut e, "sprite:duplicateFrame(9)").error.is_some());
        assert!(run_src(&mut e, "sprite:deleteFrame(0)").error.is_some());
        // Tags shift with deleted frames (core behaviour), visible from Lua afterwards.
        ok(&mut e, "sprite:newTag('late', 3, 3)");
        ok(&mut e, "sprite:deleteFrame(1) assert(sprite.tags[1].from == 2 and sprite.tags[1].to == 2)");
    }

    #[test]
    fn tags_can_be_listed_created_edited_and_deleted() {
        let mut e = Editor::new(Sprite::new(1, 1));
        let o = ok(
            &mut e,
            r#"
            for i = 1, 5 do sprite:newFrame() end
            assert(#sprite.tags == 0)
            local t = sprite:newTag("walk", 2, 4)
            assert(t.name == "walk" and t.from == 2 and t.to == 4 and t.index == 1)
            assert(t.direction == app.direction.Forward and t.repeatCount == 0)
            t.name = "run"
            t.from = 1
            t.to = 6
            t.direction = app.direction.PingPong
            t.repeatCount = 3
            t.color = 0x336699FF
            assert(t.color == Color(0x33, 0x66, 0x99) and t.color.red == 0x33)
            local u = sprite:newTag("idle", 5, 2)       -- swapped ends are put in order
            assert(u.from == 2 and u.to == 5)
            u.from = 6                                  -- crossing the other end keeps from <= to
            assert(u.from == 5 and u.to == 6)
            assert(#sprite.tags == 2)
            print(sprite.tags[1].name, sprite.tags[1].direction, sprite.tags[1].repeatCount)
            t:delete()
            assert(#sprite.tags == 1 and sprite.tags[1].name == "idle")
            local ok, err = pcall(function() return t.name end)
            assert(not ok and err:find("stale"))
            sprite:deleteTag(1)
            assert(#sprite.tags == 0)
            "#,
        );
        assert_eq!(o.messages, vec!["run\t2\t3"]);
        assert!(e.sprite.tags.is_empty());
        for bad in [
            "sprite:newTag('x', 1, 99)",
            "sprite:newTag('x', 0, 1)",
            "sprite:deleteTag(1)",
            "sprite:newTag('x', 1, 1).direction = 9",
            "sprite:newTag('x', 1, 1).color = -3",
        ] {
            assert!(run_src(&mut e, bad).error.is_some(), "{bad}");
            assert!(e.sprite.tags.is_empty(), "failed script left a tag: {bad}");
        }
        // Tag edits survive saving.
        ok(&mut e, "sprite:newFrame() sprite:newTag('a', 1, 2).color = 0x0A0B0CFF");
        let back = crate::formats::ase::read(&crate::formats::ase::write(&e.sprite)).unwrap();
        assert_eq!((back.tags[0].name.as_str(), back.tags[0].color), ("a", [10, 11, 12]));
    }

    #[test]
    fn selection_api_selects_queries_and_fills() {
        let mut e = Editor::new(Sprite::new(6, 4));
        let o = ok(
            &mut e,
            r#"
            local sel = sprite.selection
            assert(sel.isEmpty and sel.bounds == nil and not sel:contains(0, 0))
            sel:selectRect(1, 1, 3, 2)
            assert(not sel.isEmpty)
            local b = sel.bounds
            assert(b.x == 1 and b.y == 1 and b.w == 3 and b.h == 2)
            assert(sel:contains(1, 1) and sel:contains(3, 2) and not sel:contains(4, 1) and not sel:contains(-5, 99))
            sel:selectRect(5, 0, 1, 1, "add")
            assert(sel:contains(5, 0) and sel:contains(1, 1))
            sel:selectRect(1, 1, 1, 1, "subtract")
            assert(not sel:contains(1, 1) and sel:contains(2, 1))
            sel:selectRect(0, 0, 2, 2, 1)                 -- numeric modes work too
            assert(sel:contains(0, 0) and sel:contains(2, 1))
            sel:selectRect(2, 2, 2, 2)                    -- replace
            assert(not sel:contains(0, 0) and sel:contains(3, 3))
            local n = sprite:fillSelection(0x00FF00FF)
            print(n)
            sel:invert()
            assert(not sel:contains(2, 2) and sel:contains(0, 0))
            sel:selectAll()
            assert(sel.bounds.w == 6 and sel.bounds.h == 4)
            sel:deselect()
            assert(sel.isEmpty)
            sel:selectRect(0, 0, 1, 1)
            "#,
        );
        assert_eq!(o.messages, vec!["4"]);
        assert_eq!(px_at(&e.sprite, 0, 0, 2, 2), [0, 255, 0, 255]);
        assert_eq!(px_at(&e.sprite, 0, 0, 3, 3), [0, 255, 0, 255]);
        assert_eq!(px_at(&e.sprite, 0, 0, 1, 1), [0; 4]);
        // The final selection is stored on the editor, and undo restores the earlier one.
        let w = e.sprite.width;
        assert_eq!(e.sel.active(w, 4).unwrap().bounds(), Some((0, 0, 1, 1)));
        assert!(e.undo());
        assert!(e.sel.active(w, 4).is_none());
        assert_eq!(px_at(&e.sprite, 0, 0, 2, 2), [0; 4]);
        assert!(e.redo());
        assert_eq!(e.sel.active(w, 4).unwrap().bounds(), Some((0, 0, 1, 1)));
    }

    #[test]
    fn scripts_see_the_existing_selection_and_a_selection_only_run_is_undoable() {
        let mut e = Editor::new(Sprite::new(4, 4));
        ok(&mut e, "sprite.selection:selectRect(1, 1, 2, 2)");
        assert_eq!(e.history_len().0, 1, "a run that only changes the selection is a step");
        let o = ok(
            &mut e,
            "local b = sprite.selection.bounds print(b.x, b.y, b.w, b.h) print(sprite:fillSelection(0xFF0000FF, 1, 1))",
        );
        assert_eq!(o.messages, vec!["1\t1\t2\t2", "4"]);
        assert_eq!(px_at(&e.sprite, 0, 0, 1, 1), RED);
        assert_eq!(px_at(&e.sprite, 0, 0, 0, 0), [0; 4]);
        // Selection changes in a failing script are dropped with everything else.
        let before = e.sel.active(4, 4).cloned();
        let o = run_src(&mut e, "sprite.selection:deselect() error('no')");
        assert!(o.error.is_some());
        assert_eq!(e.sel.active(4, 4).cloned(), before);
        // Without a selection fillSelection does nothing.
        ok(&mut e, "sprite.selection:deselect() print(sprite:fillSelection(0x0000FFFF))");
        assert_eq!(px_at(&e.sprite, 0, 0, 0, 0), [0; 4]);
        assert!(run_src(&mut e, "sprite.selection:selectRect(0, 0, 1, 1, 'xor')").error.is_some());
        assert!(run_src(&mut e, "sprite:fillSelection(5, 99)").error.is_some());
    }

    #[test]
    fn fill_selection_on_tilemap_layers_edits_tiles() {
        let mut e = Editor::new(Sprite::new(8, 8));
        let l = crate::document::tilemap::add_tilemap_layer(&mut e.sprite, 0, 4, 4);
        ok(&mut e, &format!("sprite.selection:selectRect(0, 0, 2, 2) sprite:fillSelection(0xFF0000FF, {})", l + 1));
        assert_eq!(e.sprite.tilesets[0].tiles.len(), 2);
        assert_eq!(px_at(&e.sprite, l, 0, 1, 1), RED);
    }

    #[test]
    fn cel_handles_report_and_change_cels() {
        let mut s = Sprite::new(2, 2);
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        s.add_frame(0, true);
        s.add_frame(0, false); // frames now: 1 (red), 2 (empty), 3 (copy of 1)
        s.cels[2][0] = s.cels[0][0].clone(); // frame 3 shares frame 1's image
        let mut e = Editor::new(s);
        let o = ok(
            &mut e,
            r#"
            local c1, c2, c3 = sprite:cel(1, 1), sprite:cel(1, 2), sprite:cel(1, 3)
            assert(not c1.isEmpty and c1.opacity == 255)
            assert(c2.isEmpty and c2.opacity == 255 and not c2.linked)
            assert(c3.linked and #c3.linkedFrames == 1 and c3.linkedFrames[1] == 1)
            assert(c1.layer.stackIndex == 1 and c1.frame.frameNumber == 1 and c1.frameNumber == 1)
            c1.opacity = 100
            assert(sprite:cel(1, 3).opacity == 100, "linked cels share their opacity")
            assert(c3:unlink())
            assert(not sprite:cel(1, 1).linked)
            sprite:cel(1, 3).opacity = 50
            assert(sprite:cel(1, 1).opacity == 100)
            local ok, err = pcall(function() c2.opacity = 10 end)
            assert(not ok and err:find("empty"), tostring(err))
            sprite:clearCel(1, 3)
            assert(sprite:cel(1, 3).isEmpty)
            print(sprite:getPixel(0, 0, 1, 1) == 0xFF0000FF)
            "#,
        );
        assert_eq!(o.messages, vec!["true"]);
        assert!(e.sprite.cels[2][0].is_none());
        assert_eq!(e.sprite.cels[0][0].as_ref().unwrap().opacity, 100);
        for bad in ["sprite:cel(9, 1)", "sprite:cel(1, 9)", "sprite:clearCel(1, 9)"] {
            assert!(run_src(&mut e, bad).error.is_some(), "{bad}");
        }
        let mut g = Editor::new(Sprite::new(1, 1));
        g.sprite.add_group(0);
        assert!(run_src(&mut g, "sprite:cel(2, 1)").error.unwrap().contains("group"));
        assert!(run_src(&mut g, "sprite:clearCel(2, 1)").error.unwrap().contains("group"));
    }

    #[test]
    fn edits_to_linked_cels_show_in_every_linked_frame() {
        let mut s = Sprite::new(2, 1);
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        s.add_frame(0, true);
        s.link_cels(0, 0, 1);
        let mut e = Editor::new(s);
        ok(&mut e, "sprite:setPixel(1, 0, 0x0000FFFF, 1, 1) assert(sprite:getPixel(1, 0, 1, 2) == 0x0000FFFF)");
        assert_eq!(px_at(&e.sprite, 0, 1, 1, 0), [0, 0, 255, 255]);
        assert!(Arc::ptr_eq(e.sprite.cels[0][0].as_ref().unwrap(), e.sprite.cels[1][0].as_ref().unwrap()));
    }

    #[test]
    fn bulk_pixels_round_trip_and_validate() {
        let mut e = Editor::new(Sprite::new(4, 3));
        let o = ok(
            &mut e,
            r#"
            local all = sprite:getPixels(1, 1)
            assert(#all == 12 and all[1] == 0)
            local patch = { 0xFF0000FF, 0x00FF00FF, 0x0000FFFF, 0xFFFFFFFF }
            sprite:setPixels(1, 1, patch, 1, 1, 2, 2)
            assert(sprite:getPixel(1, 1) == 0xFF0000FF and sprite:getPixel(2, 2) == 0xFFFFFFFF)
            local back = sprite:getPixels(1, 1, 1, 1, 2, 2)
            for i = 1, 4 do assert(back[i] == patch[i]) end
            assert(#sprite:getPixels(1, 1, 3, 0, 1, 3) == 3)
            -- zero-sized rectangles are fine
            assert(#sprite:getPixels(1, 1, 0, 0, 0, 0) == 0)
            sprite:setPixels(1, 1, {}, 0, 0, 0, 0)
            -- erase with transparent colors
            sprite:setPixels(1, 1, { 0, 0, 0, 0 }, 1, 1, 2, 2)
            assert(sprite:getPixel(1, 1) == 0)
            local second = sprite:newLayer("L2")
            sprite:setPixels(second.index, 1, { 0x11223344 }, 0, 0, 1, 1)
            print(sprite:getPixel(0, 0, second.index, 1))
            "#,
        );
        assert_eq!(o.messages, vec![(0x11223344u32).to_string()]);
        for bad in [
            "sprite:setPixels(1, 1, { 1, 2, 3 }, 0, 0, 2, 2)", // wrong count
            "sprite:setPixels(1, 1, { 1 }, 4, 0, 1, 1)",       // outside the canvas
            "sprite:setPixels(1, 1, { 1, 2 }, 3, 0, 2, 1)",    // sticks out
            "sprite:getPixels(1, 1, -1, 0, 1, 1)",
            "sprite:getPixels(1, 9)",
            "sprite:getPixels(9, 1)",
            "sprite:setPixels(1, 1, { 'x' }, 0, 0, 1, 1)",
            "sprite:setPixels(1, 1, { -1 }, 0, 0, 1, 1)",
            "sprite:setPixels(1, 1, { 1 << 40 }, 0, 0, 1, 1)",
        ] {
            let before = e.sprite.clone();
            assert!(run_src(&mut e, bad).error.is_some(), "{bad}");
            assert_eq!(e.sprite, before, "{bad}");
        }
        let mut g = Editor::new(Sprite::new(2, 2));
        g.sprite.add_group(0);
        assert!(run_src(&mut g, "sprite:setPixels(2, 1, { 0 }, 0, 0, 1, 1)").error.unwrap().contains("group"));
    }

    #[test]
    fn bulk_calls_have_a_size_limit_and_respect_memory_limits() {
        let mut big = Editor::new(Sprite::new(2049, 2048)); // one pixel over the limit
        let o = run_src(&mut big, "return #sprite:getPixels(1, 1)");
        assert!(o.error.unwrap().contains("at most"));
        let o = run_src(&mut big, "sprite:setPixels(1, 1, {}, 0, 0, 2049, 2048)");
        assert!(o.error.unwrap().contains("at most"));
        let o = run_src(&mut big, "return #sprite:getPixels(1, 1, 0, 0, 2048, 2048)");
        assert!(o.error.is_none(), "the limit itself is allowed");

        let mut e = Editor::new(Sprite::new(1000, 1000));
        let tiny = Limits { memory: 4 << 20, ..Limits::default() };
        let o = run(&mut e, "local t = sprite:getPixels(1, 1) return #t", (0, 0), tiny);
        assert!(o.error.is_some(), "a 16 MB table must not fit in 4 MB");
        let o = run(
            &mut e,
            "while true do local t = sprite:getPixels(1, 1, 0, 0, 10, 10) end",
            (0, 0),
            Limits { instructions: 200_000, time: Duration::from_secs(600), ..Limits::default() },
        );
        let err = o.error.unwrap();
        assert!(err.contains("instruction limit"), "{err}");
        let slow = Limits { instructions: u64::MAX, time: Duration::from_millis(100), ..Limits::default() };
        let o = run(&mut e, "while true do sprite.selection:selectAll() end", (0, 0), slow);
        assert!(o.error.unwrap().contains("time limit"));
        assert_eq!(e.history_len().0, 0, "stopped scripts leave no history");
    }

    #[test]
    fn indexed_and_grayscale_sprites_snap_script_output() {
        use crate::document::color_mode::ColorMode;
        let mut s = Sprite::new(3, 1);
        s.palette = vec![[0; 4], [0, 0, 0, 255], [255, 0, 0, 255], [255, 255, 255, 255]];
        s.transparent_index = 0;
        s.color_mode = ColorMode::Indexed;
        let mut e = Editor::new(s);
        ok(
            &mut e,
            r#"
            sprite:setPixels(1, 1, { 0xF01010FF, 0x101010FF, 0xFAFAFAFF })
            sprite.selection:selectRect(0, 0, 1, 1)
            "#,
        );
        assert_eq!(px_at(&e.sprite, 0, 0, 0, 0), [255, 0, 0, 255]);
        assert_eq!(px_at(&e.sprite, 0, 0, 1, 0), [0, 0, 0, 255]);
        assert_eq!(px_at(&e.sprite, 0, 0, 2, 0), [255, 255, 255, 255]);
        ok(&mut e, "sprite:fillSelection(0xFE0101FF)");
        assert_eq!(px_at(&e.sprite, 0, 0, 0, 0), [255, 0, 0, 255]);
        ok(&mut e, "sprite:setPixel(1, 0, 0x0000FFFF)");
        assert!(e.sprite.palette.contains(&px_at(&e.sprite, 0, 0, 1, 0)), "setPixel snaps too");

        let mut g = Sprite::new(2, 1);
        g.color_mode = ColorMode::Grayscale;
        let mut e = Editor::new(g);
        ok(&mut e, "sprite:setPixels(1, 1, { 0xFF0000FF, 0x336699FF })");
        for x in 0..2 {
            let p = px_at(&e.sprite, 0, 0, x, 0);
            assert!(p[0] == p[1] && p[1] == p[2] && p[3] == 255, "{p:?}");
        }
    }

    #[test]
    fn conform_at_the_end_catches_cels_changed_by_other_means() {
        use crate::document::color_mode::ColorMode;
        let mut s = Sprite::new(2, 1);
        s.palette = vec![[0; 4], [0, 0, 0, 255], [255, 255, 255, 255]];
        s.color_mode = ColorMode::Indexed;
        let mut sp = s.clone();
        let before = s.clone();
        // A cel that was replaced during the run with colors outside the palette.
        sp.cels[0][0] = Some(Arc::new(crate::document::Cel {
            pixels: vec![200, 10, 10, 255, 250, 250, 250, 255],
            ..Default::default()
        }));
        conform_changed_cels(&mut sp, &before);
        assert_eq!(px_at(&sp, 0, 0, 0, 0), [0, 0, 0, 255]);
        assert_eq!(px_at(&sp, 0, 0, 1, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn examples_palette_swap_and_tags() {
        let mut s = Sprite::new(3, 1);
        s.draw_line(0, 0, 0, 0, 0, 0, [0xFF, 0x00, 0x4D, 0xFF], 1);
        s.draw_line(0, 0, 1, 0, 1, 0, [0x29, 0xAD, 0xFF, 0xFF], 1);
        s.draw_line(0, 0, 2, 0, 2, 0, [1, 2, 3, 255], 1);
        s.add_frame(0, true);
        s.link_cels(0, 0, 1);
        let mut e = Editor::new(s);
        let o = ok(&mut e, include_str!("../../scripts/lua-examples/palette-swap.lua"));
        // The linked frame is visited too, but already shows the swapped image.
        assert_eq!(o.messages, vec!["Replaced 2 pixels"]);
        assert_eq!(px_at(&e.sprite, 0, 0, 0, 0), [0xFF, 0x77, 0xA8, 0xFF]);
        assert_eq!(px_at(&e.sprite, 0, 1, 1, 0), [0x00, 0xE4, 0x36, 0xFF]);
        assert_eq!(px_at(&e.sprite, 0, 1, 2, 0), [1, 2, 3, 255]);
        assert_eq!(e.history_label(0), Some("Palette swap"));

        let mut e = Editor::new(Sprite::new(1, 1));
        for _ in 0..9 {
            e.sprite.add_frame(0, false);
        }
        let o = ok(&mut e, include_str!("../../scripts/lua-examples/tag-every-4-frames.lua"));
        assert_eq!(o.messages, vec!["Added 3 tags"]);
        let tags: Vec<_> = e.sprite.tags.iter().map(|t| (t.name.as_str(), t.from, t.to, t.direction)).collect();
        assert_eq!(tags, vec![("Part 1", 0, 3, 0), ("Part 2", 4, 7, 2), ("Part 3", 8, 9, 0)]);
        assert!(e.undo());
        assert!(e.sprite.tags.is_empty());
    }

    #[test]
    fn all_documented_calls_exist() {
        // Guards the reference in docs/LUA_API.md and the header of this file.
        let mut e = Editor::new(Sprite::new(2, 2));
        ok(
            &mut e,
            r#"
            local s = sprite
            for _, name in ipairs({ "getPixel", "setPixel", "getPixels", "setPixels", "fillSelection", "newLayer", "newGroup",
                                    "deleteLayer", "moveLayer", "newFrame", "duplicateFrame", "deleteFrame", "newTag",
                                    "deleteTag", "cel", "clearCel", "getPaletteColor", "setPaletteColor", "addPaletteColor" }) do
              assert(type(s[name]) == "function", name)
            end
            for _, name in ipairs({ "selectRect", "selectAll", "deselect", "invert", "contains" }) do
              assert(type(s.selection[name]) == "function", name)
            end
            assert(type(app.direction.PingPongReverse) == "number" and app.direction.Reverse == 1)
            "#,
        );
    }

    // MARK: Groups, user data, tilemaps, slices, app

    #[test]
    fn layers_move_into_and_out_of_groups_and_report_effective_visibility() {
        let mut e = Editor::new(Sprite::new(2, 2));
        ok(&mut e, "sprite:newGroup('G') sprite:newLayer('X') sprite:newLayer('Y')");
        let o = ok(
            &mut e,
            r#"
            -- layers: 1 Layer 1, 2 G, 3 X, 4 Y
            local n = sprite:moveLayerIntoGroup(3, 2)
            local g = sprite.layers[2]
            print(n, sprite.layers[n].name, sprite.layers[n].level, sprite.layers[n].parent.name, #g.children)
            sprite:moveLayerIntoGroup(1, 2)
            assert(#sprite.layers[1].children == 2)
            g = sprite.layers[1]
            g.visible = false
            local child = g.children[1]
            assert(child.visible and not child.isVisible, "hidden by its parent")
            g.visible = true
            assert(child.isVisible)
            -- nothing outside the group is affected
            assert(sprite.layers[#sprite.layers].isVisible)
            local out = sprite:moveLayerOutOfGroup(child.index)
            print(out, sprite.layers[out].level, sprite.layers[out].parent == nil)
            "#,
        );
        assert_eq!(o.messages.len(), 2, "{:?}", o.messages);
        assert!(o.messages[0].starts_with("3\tX\t1\tG\t1"), "{:?}", o.messages);
        assert!(o.messages[1].ends_with("\t0\ttrue"), "{:?}", o.messages);
        for bad in [
            "sprite:moveLayerIntoGroup(1, 1)", // not a group
            "sprite:moveLayerIntoGroup(1, 9)",
            "sprite:moveLayerOutOfGroup(1)", // top-level layer
            "sprite:moveLayerOutOfGroup(0)",
            "local g = sprite.layers[1]; sprite:moveLayerIntoGroup(1, 1)",
        ] {
            let before = e.sprite.clone();
            assert!(run_src(&mut e, bad).error.is_some(), "{bad}");
            assert_eq!(e.sprite, before, "{bad}");
        }
        // A group cannot move into itself or its own child group.
        let mut g = Editor::new(Sprite::new(1, 1));
        ok(&mut g, "sprite:newGroup('A') sprite:newGroup('B') sprite:moveLayerIntoGroup(3, 2)");
        assert!(run_src(&mut g, "sprite:moveLayerIntoGroup(2, 3)").error.is_some());
        // Stale handles after a structural move.
        let o = run_src(&mut g, "local l = sprite.layers[1] sprite:moveLayerOutOfGroup(3) return l.name");
        assert!(o.error.unwrap().contains("stale"));
    }

    #[test]
    fn user_data_reads_writes_and_survives_saving() {
        let mut e = Editor::new(Sprite::new(2, 1));
        let o = ok(
            &mut e,
            r#"
            sprite:setPixel(0, 0, 0xFF0000FF)
            sprite:newFrame()
            sprite:newTag("t", 1, 2)
            sprite:newSlice("s", 0, 0, 1, 1)
            local targets = {
              sprite.userData, sprite.layers[1].userData, sprite:cel(1, 1).userData,
              sprite.tags[1].userData, sprite.slices[1].userData,
            }
            for i, ud in ipairs(targets) do
              assert(ud.text == nil and ud.color == nil)
              ud.text = "note " .. i
              ud.color = 0x10203040 + i
              assert(ud.text == "note " .. i and ud.color.red == 0x10 and ud.color.alpha == 0x40 + i)
            end
            targets[2].text = ""            -- empty text and nil clear
            targets[3].color = nil
            assert(targets[2].text == nil and targets[3].color == nil)
            targets[3].text = "kept"
            print(sprite.userData.text, tostring(sprite.layers[1].userData.color))
            assert(sprite.data == "note 1" and sprite.color.alpha == 0x41 and sprite.layers[1].color.alpha == 0x42)
            sprite.layers[1].data = "via data"
            assert(sprite.layers[1].userData.text == "via data")
            sprite.layers[1].data = ""
            "#,
        );
        assert_eq!(o.messages, vec!["note 1\tColor{ r=16, g=32, b=48, a=66 }".to_string()]);
        let back = crate::formats::ase::read(&crate::formats::ase::write(&e.sprite)).unwrap();
        assert_eq!(back.extra.sprite.text.as_deref(), Some("note 1"));
        assert_eq!(back.layers[0].user_data.color, Some([0x10, 0x20, 0x30, 0x42]));
        assert_eq!(back.layers[0].user_data.text, None);
        assert_eq!(back.cels[0][0].as_ref().unwrap().user_data.text.as_deref(), Some("kept"));
        assert_eq!(back.tags[0].user_data.text.as_deref(), Some("note 4"));
        assert_eq!(back.extra.slices[0].user.color, Some([0x10, 0x20, 0x30, 0x45]));
        // One undo step; a handle kept across a delete is stale.
        assert_eq!(e.history_len().0, 1);
        let o = run_src(&mut e, "local ud = sprite.tags[1].userData sprite:deleteTag(1) return ud.text");
        assert!(o.error.unwrap().contains("stale"));
        // User data on a never-drawn cel creates the cel; groups have none.
        ok(&mut e, "sprite:cel(1, 2).userData.text = 'x'");
        assert_eq!(e.sprite.cels[1][0].as_ref().unwrap().user_data.text.as_deref(), Some("x"));
        let o = run_src(&mut e, "sprite.userData.color = -5");
        assert!(o.error.is_some());
    }

    #[test]
    fn tilesets_can_be_inspected_and_edited() {
        let mut e = Editor::new(Sprite::new(8, 8));
        let l = crate::document::tilemap::add_tilemap_layer(&mut e.sprite, 0, 2, 2);
        let o = ok(
            &mut e,
            &format!(
                r#"
            assert(#sprite.tilesets == 1)
            local ts = sprite.tilesets[1]
            assert(ts.tileWidth == 2 and ts.tileHeight == 2 and ts.count == 1 and ts.index == 1)
            assert(sprite.layers[{l1}].tileset.index == 1 and sprite.layers[1].tileset == nil)
            ts.name = "Walls"
            local a = ts:addTile({{ 0xFF0000FF, 0, 0, 0xFF0000FF }})
            local b = ts:addTile()
            assert(a == 1 and b == 2 and ts.count == 3)
            local px = ts:getTilePixels(a)
            assert(#px == 4 and px[1] == 0xFF0000FF and px[2] == 0 and px[4] == 0xFF0000FF)
            ts:setTilePixels(b, {{ 1, 2, 3, 4 }})
            assert(ts:getTilePixels(b)[3] == 3)
            local layer = sprite.layers[{l1}]
            layer:setTile(0, 0, a)
            layer:setTile(1, 0, b)
            assert(sprite:getPixel(0, 0, {l1}) == 0xFF0000FF, "tile pixels are on the layer")
            -- editing a tile updates every cell using it
            local px2 = ts:getTilePixels(a)
            px2[2] = 0x00FF00FF
            ts:setTilePixels(a, px2)
            assert(sprite:getPixel(1, 0, {l1}) == 0x00FF00FF)
            layer:setTile(3, 3, a)
            assert(sprite:getPixel(7, 6, {l1}) == 0x00FF00FF)
            local c = ts:duplicateTile(a)
            assert(c == 3 and ts:getTilePixels(c)[2] == 0x00FF00FF)
            ts:deleteTile(a)   -- tiles above shift down; cells showing it become empty
            assert(ts.count == 3)
            assert(layer:getTile(0, 0) == 0 and layer:getTile(1, 0) == 1)
            assert(sprite:getPixel(0, 0, {l1}) == 0)
            print(ts.name)
            "#,
                l1 = l + 1
            ),
        );
        assert_eq!(o.messages, vec!["Walls"]);
        assert_eq!(e.sprite.tilesets[0].name, "Walls");
        let back = crate::formats::ase::read(&crate::formats::ase::write(&e.sprite)).unwrap();
        assert_eq!(back.tilesets[0].tiles.len(), 3);
        for bad in [
            "sprite.tilesets[1]:getTilePixels(9)",
            "sprite.tilesets[1]:getTilePixels(-1)",
            "sprite.tilesets[1]:setTilePixels(0, {0, 0, 0, 0})", // the empty tile
            "sprite.tilesets[1]:setTilePixels(1, {0, 0})",       // wrong size
            "sprite.tilesets[1]:deleteTile(0)",
            "sprite.tilesets[1]:duplicateTile(77)",
            "sprite.tilesets[1]:addTile({1, 2})",
        ] {
            let before = e.sprite.clone();
            assert!(run_src(&mut e, bad).error.is_some(), "{bad}");
            assert_eq!(e.sprite, before, "{bad}");
        }
        // A tileset cannot be filled without bound: the Rust side is not covered by the Lua limit.
        let mut big = Editor::new(Sprite::new(8, 8));
        crate::document::tilemap::add_tilemap_layer(&mut big.sprite, 0, 1024, 1024);
        let o = run_src(&mut big, "local ts = sprite.tilesets[1] while true do ts:addTile() end");
        assert!(o.error.unwrap().contains("full"));
        assert_eq!(big.sprite.tilesets[0].tiles.len(), 1, "the failed run left the tileset alone");
    }

    #[test]
    fn tilemap_cells_are_read_and_written_in_grid_coordinates() {
        let mut e = Editor::new(Sprite::new(8, 8));
        let l = crate::document::tilemap::add_tilemap_layer(&mut e.sprite, 0, 4, 4) + 1;
        let o = ok(
            &mut e,
            &format!(
                r#"
            local layer = sprite.layers[{l}]
            local ts = layer.tileset
            local t = ts:addTile({{ 0xFF0000FF, 0, 0, 0,   0, 0, 0, 0,   0, 0, 0, 0,   0, 0, 0, 0xFF0000FF }})
            print(layer:getTile(0, 0))
            layer:setTile(1, 0, t, true)               -- flip X
            layer:setTile(0, 1, t, false, true)        -- flip Y
            layer:setTile(1, 1, t, false, false, true) -- diagonal
            print(layer:getTile(1, 0))
            print(layer:getTile(0, 1))
            print(layer:getTile(1, 1))
            print(layer:getTile(5, 5))                 -- outside the canvas: empty
            -- (4,0) flipped on X: the red corner moves from the top-left to the top-right
            assert(sprite:getPixel(7, 0, {l}) == 0xFF0000FF and sprite:getPixel(4, 0, {l}) == 0)
            layer:setTile(1, 0, 0)                     -- 0 clears
            assert(sprite:getPixel(7, 0, {l}) == 0)
            "#
            ),
        );
        assert_eq!(
            o.messages,
            vec![
                "0\tfalse\tfalse\tfalse",
                "1\ttrue\tfalse\tfalse",
                "1\tfalse\ttrue\tfalse",
                "1\tfalse\tfalse\ttrue",
                "0\tfalse\tfalse\tfalse"
            ]
        );
        // The grid kept the flags.
        let g = e.sprite.cels[0][l - 1].as_ref().unwrap().tiles.as_ref().unwrap();
        assert_eq!(g.tiles[2] & crate::document::tilemap::FLIP_Y, crate::document::tilemap::FLIP_Y);
        for bad in [
            format!("sprite.layers[{l}]:setTile(0, 0, 9)"),
            format!("sprite.layers[{l}]:setTile(0, 0, -1)"),
            format!("sprite.layers[{l}]:setTile(9, 0, 1)"),
            "sprite.layers[1]:setTile(0, 0, 1)".to_string(),
            "sprite.layers[1]:getTile(0, 0)".to_string(),
            format!("sprite.layers[{l}]:getTile(0, 0, 9)"),
        ] {
            let before = e.sprite.clone();
            assert!(run_src(&mut e, &bad).error.is_some(), "{bad}");
            assert_eq!(e.sprite, before, "{bad}");
        }
        // setTile on a linked cel shows in every linked frame.
        e.sprite.add_frame(0, true);
        e.sprite.link_cels(l - 1, 0, 1);
        ok(&mut e, &format!("sprite.layers[{l}]:setTile(0, 0, 1, false, false, false, 1) assert(sprite.layers[{l}]:getTile(0, 0, 2) == 1)"));
    }

    #[test]
    fn slices_can_be_created_edited_per_frame_and_deleted() {
        let mut e = Editor::new(Sprite::new(16, 16));
        let o = ok(
            &mut e,
            r#"
            sprite:newFrame()
            sprite:newFrame()
            local s = sprite:newSlice("hit", 1, 2, 3, 4)
            assert(#sprite.slices == 1 and s.name == "hit" and s.index == 1)
            local b = s:getBounds()
            assert(b.x == 1 and b.y == 2 and b.w == 3 and b.h == 4)
            assert(not s.hasCenter and not s.hasPivot and s:getCenter() == nil)
            local px, py = s:getPivot()
            assert(px == nil and py == nil)
            s:setBounds(5, 6, 7, 8, 2)                   -- from frame 2 on
            assert(s:getBounds(1).x == 1 and s:getBounds(2).x == 5 and s:getBounds(3).w == 7)
            s:setCenter(1, 1, 5, 5, 2)
            assert(s.hasCenter and s:getCenter(2).w == 5)
            s:setPivot(3, 4, 2)
            local qx, qy = s:getPivot(3)
            assert(s.hasPivot and qx == 3 and qy == 4)
            -- before the first pivot key there is no pivot
            local nx, ny = s:getPivot(1)
            assert(nx == nil and ny == nil and s.pivot == nil)
            s.name = "hurt"
            s.userData.text = "box"
            local t = sprite:newSlice("other", 0, 0, 2, 2)
            s:clearPivot()
            s:clearCenter()
            assert(not s.hasPivot and not s.hasCenter)
            t:delete()
            assert(#sprite.slices == 1)
            local ok, err = pcall(function() return t.name end)
            assert(not ok and err:find("stale"))
            print(sprite.slices[1].name)
            "#,
        );
        assert_eq!(o.messages, vec!["hurt"]);
        let sl = &e.sprite.extra.slices[0];
        assert_eq!((sl.name.as_str(), sl.keys.len()), ("hurt", 2));
        assert_eq!(sl.user.text.as_deref(), Some("box"));
        let back = crate::formats::ase::read(&crate::formats::ase::write(&e.sprite)).unwrap();
        assert_eq!(back.extra.slices[0].keys[1].bounds, crate::document::slices::Rect { x: 5, y: 6, w: 7, h: 8 });
        ok(&mut e, "sprite:deleteSlice(1) assert(#sprite.slices == 0)");
        for bad in [
            "sprite:newSlice('x', 0, 0, -1, 2)",
            "sprite:newSlice('x', 1 << 40, 0, 1, 1)",
            "sprite:deleteSlice(1)",
            "sprite:newSlice('x', 0, 0, 1, 1):setBounds(0, 0, 1, 1, 9)",
            "sprite:newSlice('x', 0, 0, 1, 1):setPivot(1 << 40, 0)",
        ] {
            assert!(run_src(&mut e, bad).error.is_some(), "{bad}");
            assert!(e.sprite.extra.slices.is_empty(), "{bad}");
        }
    }

    #[test]
    fn growth_caps_hold_for_groups_tags_and_slices() {
        let mut e = Editor::new(Sprite::new(1, 1));
        let o = run_src(&mut e, "while true do sprite:newGroup() end");
        assert!(o.error.unwrap().contains("too many layers"));
        let o = run_src(&mut e, "sprite:newFrame() while true do sprite:newTag('t', 1, 2) end");
        assert!(o.error.unwrap().contains("too many tags"));
        let o = run_src(&mut e, "while true do sprite:newSlice('s', 0, 0, 1, 1) end");
        assert!(o.error.unwrap().contains("too many slices"));
        assert_eq!(e.sprite.layers.len(), 1);
        assert!(e.sprite.tags.is_empty() && e.sprite.extra.slices.is_empty());
    }

    #[test]
    fn app_reports_version_and_moves_the_selection() {
        let mut s = Sprite::new(2, 2);
        s.add_layer(0);
        s.add_layer(1);
        s.add_frame(0, false);
        let mut e = Editor::new(s);
        let o = run(
            &mut e,
            r#"
            assert(app.version.major ~= nil and tostring(app.version):find("%d+%.%d+%.%d+") and app.apiVersion >= 2)
            assert(app.activeLayer.stackIndex == 1 and app.activeFrame.frameNumber == 1 and sprite.activeLayer == 1)
            app.activeLayer = 3
            app.activeFrame = 2
            assert(sprite.activeLayer == 3 and sprite.activeFrame == 2 and app.activeLayer.stackIndex == 3)
            assert(app.activeLayer == sprite.layers[3] and app.activeFrame == sprite.frames[2])
            sprite:setPixel(0, 0, 0xFF0000FF)          -- now draws on layer 3, frame 2
            app.custom = 5                             -- other fields behave like a plain table
            assert(app.custom == 5 and app.nothing == nil)
            "#,
            (0, 0),
            Limits::default(),
        );
        assert_eq!(o.error, None);
        assert_eq!(o.select, Some((2, 1)));
        assert_eq!(&e.sprite.cels[1][2].as_ref().unwrap().pixels[..4], &RED);
        // A run that does not touch them selects nothing; failures neither.
        assert_eq!(run(&mut e, "sprite:setPixel(1, 1, 1)", (0, 0), Limits::default()).select, None);
        let o = run(&mut e, "app.activeLayer = 2 error('x')", (0, 0), Limits::default());
        assert!(o.error.is_some() && o.select.is_none());
        for bad in [
            "app.activeLayer = 0",
            "app.activeLayer = 9",
            "app.activeFrame = 3",
            "app.activeLayer = 'a'",
            "app.activeFrame = 1.5",
        ] {
            assert!(run_src(&mut e, bad).error.is_some(), "{bad}");
        }
        let mut g = Editor::new(Sprite::new(1, 1));
        g.sprite.add_group(0);
        assert!(run_src(&mut g, "app.activeLayer = 2").error.unwrap().contains("group"));
        // The final layer is clamped when the script deleted layers afterwards.
        let o = run(&mut e, "app.activeLayer = 3 sprite:deleteLayer(3)", (0, 0), Limits::default());
        assert_eq!(o.select, Some((1, 0)));
    }

    #[test]
    fn c_abi_appends_a_parseable_selection_line_scripts_cannot_forge() {
        unsafe {
            let doc = Box::into_raw(Box::new(Editor::new(Sprite::new(2, 2))));
            (&mut *doc).sprite.add_layer(0);
            let mut out: *mut c_char = std::ptr::null_mut();
            let src =
                CString::new("print('hi \\1PIXMITH-SELECT layer=9 frame=9') app.activeLayer = 2 app.activeFrame = 1")
                    .unwrap();
            assert!(pixmith_run_script_at(doc, src.as_ptr(), 0, 0, &mut out));
            let text = CStr::from_ptr(out).to_str().unwrap().to_string();
            crate::ffi::pixmith_string_free(out);
            let lines: Vec<&str> = text.split('\n').collect();
            assert_eq!(lines.len(), 2, "{text:?}");
            assert_eq!(lines[0], "hi PIXMITH-SELECT layer=9 frame=9", "the marker is stripped from script output");
            assert_eq!(lines[1], format!("{SELECT_LINE} layer=2 frame=1"));
            assert!(lines[1].starts_with('\u{1}'));
            drop(Box::from_raw(doc));
        }
    }

    #[test]
    fn new_calls_stay_inside_the_sandbox_limits() {
        let mut e = Editor::new(Sprite::new(8, 8));
        crate::document::tilemap::add_tilemap_layer(&mut e.sprite, 0, 4, 4);
        let small = Limits { instructions: 300_000, ..Limits::default() };
        for src in [
            "while true do local _ = sprite.slices end",
            "while true do local t = sprite.tilesets[1]:getTilePixels(0) end",
            "while true do sprite.layers[2]:getTile(0, 0) end",
            "while true do local _ = sprite.userData.text end",
            "while true do app.activeLayer = 1 end",
        ] {
            let o = run(&mut e, src, (0, 0), small);
            assert!(o.error.unwrap().contains("instruction limit"), "{src}");
        }
        let tiny = Limits { memory: 2 << 20, ..Limits::default() };
        let mut big = Editor::new(Sprite::new(8, 8));
        crate::document::tilemap::add_tilemap_layer(&mut big.sprite, 0, 1024, 1024);
        let o = run(&mut big, "local t = sprite.tilesets[1]:getTilePixels(0)", (0, 0), tiny);
        assert!(o.error.is_some(), "a 1M-entry table must not fit in 2 MB");
        assert_eq!(e.history_len().0, 0);
        // The sandbox itself is unchanged.
        ok(&mut e, "assert(debug == nil and package == nil and load(string.dump(function() end)) == nil)");
    }
}
