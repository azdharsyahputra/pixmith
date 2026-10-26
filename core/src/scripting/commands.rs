//! `app.command.*`, `app.transaction`, `Version`, and the enumerations Aseprite scripts use
//! (`BlendMode`, `FlipType`, `AniDir`, `SpriteSheetType`, `ColorMode`, ...).
//!
//! `app.command.X{ ... }` runs on the active sprite (see `sprites::active_ctx`) and maps to the Pixmith
//! operation of the same meaning; commands that belong to the user interface (zoom, dialogs, the clipboard,
//! brushes, ...) do not exist and raise an error that says so.

use super::access::{self, Access};
use super::color;
use super::sprites::active_ctx;
use super::{Ctx, SpriteH};
use crate::document::color_mode::ColorMode;
use crate::document::{Cel, Layer, Sprite};
use crate::paint::filters::{self, Filter, Target};
use crate::paint::selection::Selection;
use mlua::{
    Error, Function, Lua, MetaMethod, MultiValue, ObjectLike, Result as LuaResult, Table, UserData, UserDataFields,
    UserDataMethods, Value,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// `app.apiVersion`: the level of the Aseprite scripting API this implementation follows.
pub(super) const API_VERSION: i64 = 21;

// MARK: Blend modes

/// Aseprite's `BlendMode` value of each Pixmith (file format) blend mode number.
const ASE_OF_PIX: [i64; 19] = [0, 2, 5, 8, 1, 4, 6, 3, 10, 9, 11, 12, 15, 16, 17, 18, 7, 13, 14];

pub(super) fn blend_to_aseprite(pixmith: u16) -> i64 {
    ASE_OF_PIX.get(pixmith as usize).copied().unwrap_or(0)
}

pub(super) fn blend_from_aseprite(v: i64) -> LuaResult<u16> {
    ASE_OF_PIX
        .iter()
        .position(|&a| a == v)
        .map(|p| p as u16)
        .ok_or_else(|| Error::runtime(format!("blend mode {v} is not a BlendMode value (0..18)")))
}

const BLEND_MODE_NAMES: [(&str, i64); 19] = [
    ("NORMAL", 0),
    ("DARKEN", 1),
    ("MULTIPLY", 2),
    ("COLOR_BURN", 3),
    ("LIGHTEN", 4),
    ("SCREEN", 5),
    ("COLOR_DODGE", 6),
    ("ADDITION", 7),
    ("OVERLAY", 8),
    ("SOFT_LIGHT", 9),
    ("HARD_LIGHT", 10),
    ("DIFFERENCE", 11),
    ("EXCLUSION", 12),
    ("SUBTRACT", 13),
    ("DIVIDE", 14),
    ("HSL_HUE", 15),
    ("HSL_SATURATION", 16),
    ("HSL_COLOR", 17),
    ("HSL_LUMINOSITY", 18),
];

fn enum_table(lua: &Lua, items: &[(&str, i64)]) -> LuaResult<Table> {
    let t = lua.create_table()?;
    for (k, v) in items {
        t.set(*k, *v)?;
    }
    Ok(t)
}

// MARK: Version

#[derive(Clone, Debug, PartialEq, Eq)]
struct VersionH {
    major: i64,
    minor: i64,
    patch: i64,
    label: String,
    number: i64,
}

impl VersionH {
    fn parse(s: &str) -> Option<VersionH> {
        let s = s.trim();
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (s, None),
        };
        let mut nums = core.split('.').map(|p| p.parse::<i64>().ok());
        let major = nums.next()??;
        let minor = nums.next().unwrap_or(Some(0))?;
        let patch = nums.next().unwrap_or(Some(0))?;
        if nums.next().is_some() {
            return None;
        }
        let (label, number) = match pre {
            None => (String::new(), 0),
            Some(p) => {
                let p = p.split(['-', '+']).next().unwrap_or("");
                let split = p.find(|c: char| c.is_ascii_digit()).unwrap_or(p.len());
                (p[..split].to_string(), p[split..].parse().unwrap_or(0))
            }
        };
        Some(VersionH { major, minor, patch, label, number })
    }

    fn rank(&self) -> i64 {
        match self.label.as_str() {
            "" => 100,
            "dev" => 1,
            "alpha" | "a" => 2,
            "beta" | "b" => 3,
            "rc" => 4,
            _ => 5,
        }
    }

    fn key(&self) -> (i64, i64, i64, i64, i64) {
        (self.major, self.minor, self.patch, self.rank(), self.number)
    }

    fn text(&self) -> String {
        let mut s = format!("{}.{}.{}", self.major, self.minor, self.patch);
        if !self.label.is_empty() {
            s.push_str(&format!(
                "-{}{}",
                self.label,
                if self.number > 0 { self.number.to_string() } else { String::new() }
            ));
        }
        s
    }
}

impl UserData for VersionH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("major", |_, t| Ok(t.major));
        f.add_field_method_get("minor", |_, t| Ok(t.minor));
        f.add_field_method_get("patch", |_, t| Ok(t.patch));
        f.add_field_method_get("prereleaseLabel", |_, t| Ok(t.label.clone()));
        f.add_field_method_get("prereleaseNumber", |_, t| Ok(t.number));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, t, ()| Ok(t.text()));
        m.add_meta_method(MetaMethod::Concat, |_, t, o: Value| {
            Ok(match o {
                Value::String(s) => format!("{}{}", t.text(), s.to_string_lossy()),
                _ => t.text(),
            })
        });
        m.add_meta_method(MetaMethod::Eq, |_, t, o: mlua::UserDataRef<VersionH>| Ok(t.key() == o.key()));
        m.add_meta_method(MetaMethod::Lt, |_, t, o: mlua::UserDataRef<VersionH>| Ok(t.key() < o.key()));
        m.add_meta_method(MetaMethod::Le, |_, t, o: mlua::UserDataRef<VersionH>| Ok(t.key() <= o.key()));
    }
}

fn version_ctor(a: MultiValue) -> LuaResult<VersionH> {
    let bad = || {
        Error::runtime(
            "Version: expected a string like \"1.3.10\" or \"1.3-rc1\", numbers (major, minor, patch) or a table",
        )
    };
    let v: Vec<Value> = a.into_iter().collect();
    match v.as_slice() {
        [Value::String(s)] => VersionH::parse(&s.to_str()?).ok_or_else(bad),
        [Value::Table(t)] => {
            let n = |k: &str| t.raw_get::<Option<i64>>(k).ok().flatten().unwrap_or(0);
            Ok(VersionH {
                major: n("major"),
                minor: n("minor"),
                patch: n("patch"),
                label: t.raw_get::<Option<String>>("prereleaseLabel").ok().flatten().unwrap_or_default(),
                number: n("prereleaseNumber"),
            })
        }
        nums if !nums.is_empty() && nums.len() <= 3 => {
            let mut x = [0i64; 3];
            for (i, n) in nums.iter().enumerate() {
                x[i] = super::geometry::int_of(n).ok_or_else(bad)?;
            }
            Ok(VersionH { major: x[0], minor: x[1], patch: x[2], label: String::new(), number: 0 })
        }
        _ => Err(bad()),
    }
}

// MARK: Undo

/// Snapshots `app.command.Undo` / `Redo` step through. The first snapshot is the document as the run found it;
/// one is added after every `app.command` and `app.transaction` (and before them, so that edits made with
/// the plain API in between form a step of their own).
#[derive(Default)]
pub(super) struct UndoLog {
    marks: RefCell<Vec<(Sprite, Option<Arc<Selection>>)>>,
    redo: RefCell<Vec<(Sprite, Option<Arc<Selection>>)>>,
}

impl UndoLog {
    pub fn reset(&self, s: Sprite, mask: Option<Arc<Selection>>) {
        *self.marks.borrow_mut() = vec![(s, mask)];
        self.redo.borrow_mut().clear();
    }

    fn boundary(&self, ctx: &Ctx) {
        let cur = (ctx.sprite().clone(), ctx.mask());
        let mut marks = self.marks.borrow_mut();
        if marks.last() != Some(&cur) {
            marks.push(cur);
            self.redo.borrow_mut().clear();
        }
    }

    fn restore(ctx: &Ctx, state: &(Sprite, Option<Arc<Selection>>)) {
        *ctx.sprite() = state.0.clone();
        *ctx.mask.borrow_mut() = state.1.clone();
        ctx.invalidate_handles();
        clamp_site(ctx);
    }

    fn undo(&self, ctx: &Ctx) -> bool {
        self.boundary(ctx);
        let mut marks = self.marks.borrow_mut();
        if marks.len() < 2 {
            return false;
        }
        let top = marks.pop().expect("two marks");
        self.redo.borrow_mut().push(top);
        Self::restore(ctx, marks.last().expect("one mark left"));
        true
    }

    fn redo(&self, ctx: &Ctx) -> bool {
        let Some(next) = self.redo.borrow_mut().pop() else { return false };
        Self::restore(ctx, &next);
        self.marks.borrow_mut().push(next);
        true
    }
}

fn clamp_site(ctx: &Ctx) {
    let s = ctx.sprite();
    let mut l = ctx.layer.get().min(s.layers.len() - 1);
    if s.layers[l].is_group() {
        l = s.layers.iter().rposition(|x| !x.is_group()).unwrap_or(0);
    }
    ctx.layer.set(l);
    ctx.frame.set(ctx.frame.get().min(s.frame_count() - 1));
}

// MARK: Parameters

struct Params(Option<Table>);

impl Params {
    fn get(&self, k: &str) -> Value {
        self.0.as_ref().and_then(|t| t.raw_get::<Value>(k).ok()).unwrap_or(Value::Nil)
    }

    fn str(&self, k: &str) -> Option<String> {
        match self.get(k) {
            Value::String(s) => Some(s.to_string_lossy()),
            _ => None,
        }
    }

    fn int(&self, k: &str) -> Option<i64> {
        super::geometry::int_of(&self.get(k))
    }

    fn num(&self, k: &str) -> Option<f64> {
        match self.get(k) {
            Value::Integer(i) => Some(i as f64),
            Value::Number(n) => Some(n),
            _ => None,
        }
    }

    fn flag(&self, k: &str) -> Option<bool> {
        match self.get(k) {
            Value::Boolean(b) => Some(b),
            _ => None,
        }
    }

    fn color(&self, k: &str) -> LuaResult<Option<[u8; 4]>> {
        match self.get(k) {
            Value::Nil => Ok(None),
            v => color::arg(&v).map(Some),
        }
    }
}

// MARK: Operations on a document

/// Runs `f` on an editor holding a copy of the document and its selection (the existing editor operations take an
/// `Editor`), then copies the result back; nothing of the scratch editor's history is kept.
fn scratch<R>(a: &Ctx, f: impl FnOnce(&mut crate::document::Editor) -> R) -> R {
    let mut ed = crate::document::Editor::new(a.sprite().clone());
    ed.sel.mask = a.mask();
    let r = f(&mut ed);
    *a.sprite() = std::mem::replace(&mut ed.sprite, Sprite::new(1, 1));
    let mask = ed.sel.mask.take();
    *a.mask.borrow_mut() = mask.filter(|m| m.w == a.sprite().width && m.h == a.sprite().height);
    a.invalidate_handles();
    clamp_site(a);
    r
}

fn drawable(a: &Ctx) -> LuaResult<(usize, usize)> {
    let l = a.layer.get().min(a.sprite().layers.len() - 1);
    if a.sprite().layers[l].is_group() {
        return Err(Error::runtime("the active layer is a group"));
    }
    Ok((l, a.frame.get().min(a.sprite().frame_count() - 1)))
}

/// Rebuilds every distinct cel (linked cels stay linked) with `f(pixels) -> new pixels`.
fn map_cels(s: &mut Sprite, f: impl Fn(&[u8]) -> Vec<u8>) {
    s.flush_links();
    let mut done: HashMap<*const Cel, Arc<Cel>> = HashMap::new();
    for cel in s.cels.iter_mut().flatten().flatten() {
        if let Some(n) = done.get(&Arc::as_ptr(cel)) {
            *cel = n.clone();
            continue;
        }
        let key = Arc::as_ptr(cel);
        let new = Arc::new(Cel {
            opacity: cel.opacity,
            pixels: f(&cel.pixels),
            user_data: cel.user_data.clone(),
            z_index: cel.z_index,
            ..Default::default()
        });
        done.insert(key, new.clone());
        *cel = new;
    }
}

fn no_tilemaps(s: &Sprite, what: &str) -> LuaResult<()> {
    if s.layers.iter().any(|l| l.kind == crate::document::tilemap::KIND_TILEMAP) {
        return Err(Error::runtime(format!("{what} is not supported on sprites with tilemap layers")));
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Turn {
    FlipH,
    FlipV,
    Cw,
    Ccw,
    Half,
}

/// Flips or rotates the whole canvas (all layers and frames).
fn turn_sprite(a: &Ctx, how: Turn) -> LuaResult<()> {
    let s = a.sprite();
    no_tilemaps(s, "flipping or rotating the sprite")?;
    let (w, h) = (s.width as usize, s.height as usize);
    let (nw, nh) = if matches!(how, Turn::Cw | Turn::Ccw) { (h, w) } else { (w, h) };
    // (x, y) in the source -> (x, y) in the result
    let dest = move |x: usize, y: usize| match how {
        Turn::FlipH => (w - 1 - x, y),
        Turn::FlipV => (x, h - 1 - y),
        Turn::Half => (w - 1 - x, h - 1 - y),
        Turn::Cw => (h - 1 - y, x),
        Turn::Ccw => (y, w - 1 - x),
    };
    map_cels(s, |src| {
        let mut out = vec![0u8; nw * nh * 4];
        if src.len() == w * h * 4 {
            for y in 0..h {
                for x in 0..w {
                    let (dx, dy) = dest(x, y);
                    out[(dy * nw + dx) * 4..][..4].copy_from_slice(&src[(y * w + x) * 4..][..4]);
                }
            }
        }
        out
    });
    // Slices follow the canvas.
    let (wi, hi) = (w as i32, h as i32);
    for k in s.extra.slices.iter_mut().flat_map(|sl| sl.keys.iter_mut()) {
        let b = k.bounds;
        let (bw, bh) = (b.w as i32, b.h as i32);
        let (x, y, w2, h2) = match how {
            Turn::FlipH => (wi - b.x - bw, b.y, b.w, b.h),
            Turn::FlipV => (b.x, hi - b.y - bh, b.w, b.h),
            Turn::Half => (wi - b.x - bw, hi - b.y - bh, b.w, b.h),
            Turn::Cw => (hi - b.y - bh, b.x, b.h, b.w),
            Turn::Ccw => (b.y, wi - b.x - bw, b.h, b.w),
        };
        k.bounds = crate::document::slices::Rect { x, y, w: w2, h: h2 };
    }
    s.width = nw as u32;
    s.height = nh as u32;
    *a.mask.borrow_mut() = None;
    a.invalidate_handles();
    Ok(())
}

pub(super) fn flatten(a: &Ctx) -> LuaResult<()> {
    let s = a.sprite();
    s.flush_links();
    let name = s.layers.iter().find(|l| !l.is_group()).map_or_else(|| "Layer 1".to_string(), |l| l.name.clone());
    let frames = s.frame_count();
    let mut cels = Vec::with_capacity(frames);
    for f in 0..frames {
        let px = s.render(f);
        cels.push(vec![px
            .chunks_exact(4)
            .any(|p| p[3] != 0)
            .then(|| Arc::new(Cel { pixels: px, ..Default::default() }))]);
    }
    s.layers = vec![Layer::new(name)];
    s.cels = cels;
    s.tilesets.clear();
    if s.color_mode != ColorMode::Rgba {
        crate::document::color_mode::conform_all(s);
    }
    a.invalidate_handles();
    clamp_site(a);
    Ok(())
}

pub(super) fn resize(a: &Ctx, w: i64, h: i64) -> LuaResult<()> {
    if w < 1 || h < 1 || w > 8192 || h > 8192 {
        return Err(Error::runtime("the new size must be between 1x1 and 8192x8192"));
    }
    let s = a.sprite();
    if (w, h) == (s.width as i64, s.height as i64) {
        return Ok(());
    }
    if !s.cels_fit_budget(w as u64, h as u64) {
        return Err(Error::runtime("the sprite would need too much memory"));
    }
    s.flush_links();
    filters::scale_sprite(s, w as u32, h as u32);
    *a.mask.borrow_mut() = None;
    a.invalidate_handles();
    Ok(())
}

/// `sprite:crop(x, y, w, h)` / `CropSprite`: keeps that rectangle of the canvas.
pub(super) fn crop(a: &Ctx, x: i64, y: i64, w: i64, h: i64) -> LuaResult<()> {
    if w < 1 || h < 1 {
        return Err(Error::runtime("crop needs a rectangle with a size"));
    }
    let (sw, sh) = (a.sprite().width as i64, a.sprite().height as i64);
    if x.abs() > 1 << 20
        || y.abs() > 1 << 20
        || w > 8192
        || h > 8192
        || x.saturating_add(w) > sw + (1 << 20)
        || y.saturating_add(h) > sh + (1 << 20)
    {
        return Err(Error::runtime("the crop rectangle is too large"));
    }
    let ok = scratch(a, |e| crate::paint::selection::resize_canvas(e, w as i32, h as i32, -x as i32, -y as i32));
    if !ok && (w, h, x, y) != (sw, sh, 0, 0) {
        return Err(Error::runtime("the sprite would need too much memory"));
    }
    Ok(())
}

fn content_bounds(s: &Sprite) -> Option<(usize, usize, usize, usize)> {
    let (w, h) = (s.width as usize, s.height as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for cel in s.cels.iter().flatten().flatten() {
        if cel.pixels.len() != w * h * 4 {
            continue;
        }
        for (y, row) in cel.pixels.chunks_exact(w * 4).enumerate() {
            if let Some(first) = row.chunks_exact(4).position(|p| p[3] != 0) {
                let last = row.chunks_exact(4).rposition(|p| p[3] != 0).unwrap_or(first);
                x0 = x0.min(first);
                x1 = x1.max(last + 1);
                y0 = y0.min(y);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
}

/// Applies a filter to the active cel, limited to the selection.
fn filter_cel(a: &Ctx, filter: Filter) -> LuaResult<()> {
    let (l, f) = drawable(a)?;
    let s = a.sprite();
    let before = s.cels[f][l].as_ref().map(|c| c.pixels.clone());
    filters::apply(s, &filter, Target::Cel, l, f, None);
    if let (Some(mask), Some(before)) = (a.mask(), before) {
        if (mask.w, mask.h) == (s.width, s.height) {
            if let Some(cel) = s.cel_mut(l, f) {
                for (i, on) in mask.bits.iter().enumerate() {
                    if !*on {
                        cel.pixels[i * 4..i * 4 + 4].copy_from_slice(&before[i * 4..i * 4 + 4]);
                    }
                }
            }
        }
    }
    s.flush_links();
    Ok(())
}

fn check_frames(s: &Sprite) -> LuaResult<()> {
    if s.frame_count() >= super::MAX_SCRIPT_FRAMES || !s.cels_fit_budget(s.width as u64, s.height as u64) {
        return Err(Error::runtime(format!(
            "too many frames (at most {}) or not enough memory",
            super::MAX_SCRIPT_FRAMES
        )));
    }
    Ok(())
}

fn check_layers(s: &Sprite) -> LuaResult<()> {
    if s.layers.len() >= super::MAX_SCRIPT_LAYERS {
        return Err(Error::runtime(format!("too many layers (at most {})", super::MAX_SCRIPT_LAYERS)));
    }
    Ok(())
}

// MARK: app.command

const UNSUPPORTED_UI: &str = "it belongs to Aseprite's user interface (or to a feature Pixmith does not have)";

fn run_command(lua: &Lua, root: &Rc<Ctx>, name: &str, p: Params) -> LuaResult<()> {
    let a = active_ctx(root);
    if a.doc.closed.get() || !a.has_doc.get() {
        return Err(Error::runtime(format!("app.command.{name}: there is no active sprite")));
    }
    let a = &a;
    a.undo.boundary(a);
    let result = dispatch(lua, root, a, name, &p);
    if result.is_ok() {
        a.undo.boundary(a);
    }
    result
}

fn dispatch(lua: &Lua, root: &Rc<Ctx>, a: &Rc<Ctx>, name: &str, p: &Params) -> LuaResult<()> {
    let flipped = |p: &Params, default_orientation: Option<&str>| -> LuaResult<Turn> {
        let o = p.str("orientation").or(default_orientation.map(str::to_string)).unwrap_or_default();
        match o.to_ascii_lowercase().as_str() {
            "horizontal" => Ok(Turn::FlipH),
            "vertical" => Ok(Turn::FlipV),
            _ => Err(Error::runtime("Flip needs orientation = \"horizontal\" or \"vertical\"")),
        }
    };
    match name {
        // Frames and layers
        "NewFrame" => {
            check_frames(a.sprite())?;
            let dup = !matches!(p.str("content").as_deref(), Some("empty"));
            let last = a.sprite().frame_count() - 1;
            let i = a.sprite().add_frame(a.frame.get(), dup);
            a.frame.set(i);
            if i <= last {
                a.invalidate_handles(); // frames after the new one moved
            }
        }
        "RemoveFrame" => {
            let f = a.frame.get();
            if !a.sprite().remove_frame(f) {
                return Err(Error::runtime("RemoveFrame: the last frame cannot be removed"));
            }
            a.invalidate_handles();
            clamp_site(a);
        }
        "NewLayer" => {
            check_layers(a.sprite())?;
            let s = a.sprite();
            let above = a.layer.get();
            let i = if p.flag("group") == Some(true) { s.add_group(above) } else { s.add_layer(above) };
            if let Some(n) = p.str("name").filter(|n| !n.is_empty()) {
                s.layers[i].name = n;
            }
            if p.flag("reference") == Some(true) {
                s.set_layer_reference(i, true);
            }
            if !s.layers[i].is_group() {
                a.layer.set(i);
            }
            if i + 1 < s.layers.len() {
                a.invalidate_handles(); // layers above the new one moved
            }
        }
        "RemoveLayer" => {
            let l = a.layer.get();
            if !a.sprite().remove_layer(l) {
                return Err(Error::runtime("RemoveLayer: the last drawable layer cannot be removed"));
            }
            a.invalidate_handles();
            clamp_site(a);
        }
        "DuplicateLayer" => {
            check_layers(a.sprite())?;
            let l = a.layer.get();
            let n = a.sprite().duplicate_layer(l).ok_or_else(|| Error::runtime("DuplicateLayer failed"))?;
            a.layer.set(n);
            if n + 1 < a.sprite().layers.len() {
                a.invalidate_handles();
            }
            clamp_site(a);
        }
        "MergeDownLayer" => {
            let l = a.layer.get();
            let n = a.sprite().merge_down(l).ok_or_else(|| {
                Error::runtime("MergeDownLayer: the layer cannot be merged down (it needs an image layer below it)")
            })?;
            a.layer.set(n);
            a.invalidate_handles();
        }
        "FlattenLayers" => flatten(a)?,
        "GotoFirstFrame" => a.frame.set(0),
        "GotoLastFrame" => a.frame.set(a.sprite().frame_count() - 1),
        "GotoNextFrame" => a.frame.set((a.frame.get() + 1) % a.sprite().frame_count()),
        "GotoPreviousFrame" => {
            let n = a.sprite().frame_count();
            a.frame.set((a.frame.get() + n - 1) % n);
        }
        "GotoFrame" => {
            let f = p.int("frame").ok_or_else(|| Error::runtime("GotoFrame needs frame = <number>"))?;
            a.frame.set(a.frame_index(Some(f))?);
        }
        "GotoNextLayer" | "GotoPreviousLayer" => {
            let s = a.sprite();
            let n = s.layers.len();
            let mut l = a.layer.get();
            for _ in 0..n {
                l = if name == "GotoNextLayer" { (l + 1) % n } else { (l + n - 1) % n };
                if !s.layers[l].is_group() {
                    break;
                }
            }
            a.layer.set(l);
        }
        "Undo" => {
            if !a.undo.undo(a) {
                return Err(Error::runtime("Undo: nothing to undo in this script"));
            }
        }
        "Redo" => {
            if !a.undo.redo(a) {
                return Err(Error::runtime("Redo: nothing to redo in this script"));
            }
        }
        // Transformations
        "Flip" => {
            let t = flipped(p, None)?;
            match p.str("target").unwrap_or_default().as_str() {
                "canvas" | "sprite" | "all" => turn_sprite(a, t)?,
                _ => {
                    // The selection (or the whole cel) of the active cel.
                    let (l, f) = drawable(a)?;
                    let op = if t == Turn::FlipH {
                        crate::paint::selection::Op::FlipH
                    } else {
                        crate::paint::selection::Op::FlipV
                    };
                    scratch(a, |e| crate::paint::selection::transform(e, l, f, op));
                }
            }
        }
        "FlipHorizontal" => turn_sprite(a, Turn::FlipH)?,
        "FlipVertical" => turn_sprite(a, Turn::FlipV)?,
        "Rotate" => {
            let angle = p.int("angle").unwrap_or(90).rem_euclid(360);
            let how = match angle {
                90 => Turn::Cw,
                180 => Turn::Half,
                270 => Turn::Ccw,
                0 => return Ok(()),
                _ => return Err(Error::runtime("Rotate: angle must be 90, 180 or 270")),
            };
            match p.str("target").unwrap_or_else(|| "canvas".into()).as_str() {
                "mask" | "cel" => {
                    let (l, f) = drawable(a)?;
                    let turns = (angle / 90) as usize;
                    scratch(a, |e| {
                        for _ in 0..turns {
                            crate::paint::selection::transform(e, l, f, crate::paint::selection::Op::RotateCw);
                        }
                    });
                }
                _ => turn_sprite(a, how)?,
            }
        }
        "CanvasSize" => {
            let (l, r, t, b) = (
                p.int("left").unwrap_or(0),
                p.int("right").unwrap_or(0),
                p.int("top").unwrap_or(0),
                p.int("bottom").unwrap_or(0),
            );
            let s = a.sprite();
            let (nw, nh) = (s.width as i64 + l + r, s.height as i64 + t + b);
            if nw < 1 || nh < 1 || nw > 65535 || nh > 65535 {
                return Err(Error::runtime("CanvasSize: the resulting canvas must be between 1x1 and 65535x65535"));
            }
            if !scratch(a, |e| crate::paint::selection::resize_canvas(e, nw as i32, nh as i32, l as i32, t as i32))
                && (nw, nh, l, t) != (s.width as i64, s.height as i64, 0, 0)
            {
                return Err(Error::runtime("CanvasSize: the canvas would need too much memory"));
            }
        }
        "SpriteSize" => {
            let s = a.sprite();
            let (w, h) = match (p.int("width"), p.int("height"), p.num("scale")) {
                (Some(w), Some(h), _) => (w, h),
                (Some(w), None, _) => (w, (w as f64 * s.height as f64 / s.width as f64).round() as i64),
                (None, Some(h), _) => ((h as f64 * s.width as f64 / s.height as f64).round() as i64, h),
                (None, None, Some(k)) => ((s.width as f64 * k).round() as i64, (s.height as f64 * k).round() as i64),
                _ => return Err(Error::runtime("SpriteSize needs width and/or height (or scale)")),
            };
            resize(a, w, h)?;
        }
        "CropSprite" => {
            let (w, h) = (a.sprite().width, a.sprite().height);
            let (x, y, bw, bh) = a
                .mask()
                .filter(|m| (m.w, m.h) == (w, h))
                .and_then(|m| m.bounds())
                .map(|(x, y, bw, bh)| (x as i64, y as i64, bw as i64, bh as i64))
                .ok_or_else(|| Error::runtime("CropSprite needs a selection"))?;
            crop(a, x, y, bw, bh)?;
        }
        "AutocropSprite" | "Autocrop" => {
            if let Some((x, y, w, h)) = content_bounds(a.sprite()) {
                crop(a, x as i64, y as i64, w as i64, h as i64)?;
            }
        }
        "ChangePixelFormat" => {
            let mode = match p.str("format").unwrap_or_default().to_ascii_lowercase().as_str() {
                "rgb" | "rgba" => ColorMode::Rgba,
                "gray" | "grayscale" | "grey" => ColorMode::Grayscale,
                "indexed" => ColorMode::Indexed,
                _ => return Err(Error::runtime("ChangePixelFormat needs format = \"rgb\", \"gray\" or \"indexed\"")),
            };
            let dither = match p.str("dithering").unwrap_or_default().as_str() {
                "ordered" => 4,
                _ => 0,
            };
            a.sprite().flush_links();
            crate::document::color_mode::convert(a.sprite(), mode, dither);
        }
        // Filters (on the active cel, inside the selection)
        "ReplaceColor" => {
            let from = p.color("from")?.ok_or_else(|| Error::runtime("ReplaceColor needs from = <color>"))?;
            let to = p.color("to")?.ok_or_else(|| Error::runtime("ReplaceColor needs to = <color>"))?;
            let tolerance = p.int("tolerance").unwrap_or(0).clamp(0, 255) as u8;
            filter_cel(a, Filter::ReplaceColor { from, to, tolerance })?;
        }
        "Outline" => {
            let color = match p.color("color")? {
                Some(c) => c,
                None => a.colors().0.to_be_bytes(),
            };
            let inside = p.str("place").as_deref() == Some("inside");
            let eight = !matches!(p.str("matrix").as_deref(), Some("circle" | "horizontal" | "vertical"));
            filter_cel(a, Filter::Outline { color, inside, eight })?;
        }
        "InvertColor" => filter_cel(a, Filter::Invert)?,
        "Desaturate" => filter_cel(a, Filter::Desaturate)?,
        "BrightnessContrast" => {
            let g = |k: &str| p.int(k).unwrap_or(0).clamp(-100, 100) as i32;
            filter_cel(a, Filter::BrightnessContrast { brightness: g("brightness"), contrast: g("contrast") })?;
        }
        "HueSaturation" => {
            let g = |k: &str| p.int(k).unwrap_or(0).clamp(-100, 100) as i32;
            filter_cel(
                a,
                Filter::Hsl {
                    hue: p.int("hue").unwrap_or(0).clamp(-180, 180) as i32,
                    saturation: g("saturation"),
                    lightness: g("lightness"),
                },
            )?;
        }
        "Despeckle" => filter_cel(a, Filter::Despeckle { eight: true })?,
        // Selection and cels
        "SelectAll" => {
            let s = a.sprite();
            a.set_mask(Selection::new(s.width, s.height, true));
        }
        "DeselectMask" => *a.mask.borrow_mut() = None,
        "InvertMask" => {
            let s = a.sprite();
            let mut m = a.mask().map_or_else(|| Selection::new(s.width, s.height, false), |m| (*m).clone());
            for b in &mut m.bits {
                *b = !*b;
            }
            a.set_mask(m);
        }
        "MaskContent" => {
            let (l, f) = drawable(a)?;
            let s = a.sprite();
            let mut m = Selection::new(s.width, s.height, false);
            if let Some(c) = s.cels[f][l].as_ref() {
                for (i, px) in c.pixels.chunks_exact(4).enumerate() {
                    m.bits[i] = px[3] != 0;
                }
            }
            a.set_mask(m);
        }
        "ClearCel" => {
            let (l, f) = drawable(a)?;
            a.sprite().clear_cels((l, l), (f, f));
        }
        "Clear" => {
            let (l, f) = drawable(a)?;
            let s = a.sprite();
            let mask = a.mask().filter(|m| (m.w, m.h) == (s.width, s.height));
            match mask {
                None => {
                    s.clear_cels((l, l), (f, f));
                }
                Some(m) => {
                    if let Some(cel) = s.cel_mut(l, f) {
                        for (i, on) in m.bits.iter().enumerate() {
                            if *on {
                                cel.pixels[i * 4..i * 4 + 4].fill(0);
                            }
                        }
                    }
                    s.flush_links();
                }
            }
        }
        // Files and documents
        "NewFile" => {
            let w = p.int("width").unwrap_or(64);
            let h = p.int("height").unwrap_or(64);
            let m = match p.str("colorMode").as_deref() {
                Some("gray" | "grayscale") => 1,
                Some("indexed") => 2,
                _ => p.int("colorMode").unwrap_or(0),
            };
            let f: Function = lua.globals().get("Sprite")?;
            f.call::<Value>((w, h, m))?;
        }
        "OpenFile" => {
            let f = p.str("filename").ok_or_else(|| Error::runtime("OpenFile needs filename = <path>"))?;
            let open: Function = lua.globals().get::<Table>("app")?.get("open")?;
            open.call::<Value>(f)?;
        }
        "SaveFile" | "SaveFileAs" | "SaveFileCopyAs" => {
            let existing = a.doc.path.borrow().clone();
            let target = p
                .str("filename")
                .or_else(|| {
                    (name == "SaveFile").then(|| existing.as_ref().map(|e| e.to_string_lossy().into_owned())).flatten()
                })
                .ok_or_else(|| {
                    Error::runtime(format!("{name} needs filename = <path> (this sprite has no file yet)"))
                })?;
            let h = lua.create_userdata(SpriteH(a.clone()))?;
            let method = if name == "SaveFileCopyAs" { "saveCopyAs" } else { "saveAs" };
            h.call_method::<()>(method, target)?;
        }
        "CloseFile" => {
            let h = lua.create_userdata(SpriteH(a.clone()))?;
            h.call_method::<()>("close", ())?;
        }
        "DuplicateSprite" => {
            let f: Function = lua.globals().get("Sprite")?;
            let h = lua.create_userdata(SpriteH(a.clone()))?;
            f.call::<Value>(h)?;
        }
        "ExportSpriteSheet" => export_sheet(root, a, p)?,
        "Refresh" => {}
        other => {
            return Err(Error::runtime(format!("app.command.{other} is not available in Pixmith: {UNSUPPORTED_UI}")));
        }
    }
    Ok(())
}

fn export_sheet(root: &Rc<Ctx>, a: &Rc<Ctx>, p: &Params) -> LuaResult<()> {
    use crate::formats::sheet::{self, Layout};
    let kind = p.int("type").unwrap_or(1);
    let columns = p.int("columns").unwrap_or(0);
    let layout = match kind {
        1 => Layout::Horizontal,
        2 => Layout::Vertical,
        3 => Layout::Grid(columns.max(1) as usize),
        4 => {
            // Columns: as many rows as `rows`, or a square-ish grid.
            let n = a.sprite().frame_count();
            let rows = p.int("rows").unwrap_or(0);
            Layout::Grid(if rows > 0 { n.div_ceil(rows as usize).max(1) } else { (n as f64).sqrt().ceil() as usize })
        }
        5 => {
            let n = a.sprite().frame_count();
            Layout::Grid(((n as f64).sqrt().ceil() as usize).max(1))
        }
        _ => return Err(Error::runtime("ExportSpriteSheet: unknown SpriteSheetType")),
    };
    let opts = sheet::Options {
        layout,
        padding: p.int("shapePadding").unwrap_or(0).clamp(0, 256) as usize,
        scale: 1,
        border: p.int("borderPadding").map(|v| v.clamp(0, 256) as usize),
        extrude: if p.flag("extrude") == Some(true) { 1 } else { 0 },
    };
    let tex = p.str("textureFilename");
    let data = p.str("dataFilename");
    if tex.is_none() && data.is_none() {
        return Err(Error::runtime("ExportSpriteSheet needs textureFilename and/or dataFilename"));
    }
    let tex_path = tex.as_deref().map(|t| access::check(root, t, Access::Write)).transpose()?;
    let data_path = data.as_deref().map(|t| access::check(root, t, Access::Write)).transpose()?;
    let s = a.sprite();
    s.flush_links();
    let built = sheet::build(s, opts).map_err(Error::runtime)?;
    if let Some(tp) = &tex_path {
        let png = crate::formats::export::encode_png(&built.rgba, built.width, built.height).map_err(Error::runtime)?;
        crate::formats::durable::write_file(&tp.to_string_lossy(), &png).map_err(Error::runtime)?;
    }
    if let Some(dp) = &data_path {
        let name =
            tex_path.as_ref().and_then(|t| t.file_name()).map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let title =
            tex_path.as_ref().and_then(|t| t.file_stem()).map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let meta = crate::document::slices::json_meta(s, 1);
        let json = sheet::json_job(&built, &s.tags, &meta, &name, &title);
        crate::formats::durable::write_file(&dp.to_string_lossy(), json.as_bytes()).map_err(Error::runtime)?;
    }
    Ok(())
}

// MARK: Installation

pub(super) fn install(lua: &Lua, ctx: &Rc<Ctx>) -> LuaResult<()> {
    let g = lua.globals();
    g.set("BlendMode", enum_table(lua, &BLEND_MODE_NAMES)?)?;
    g.set("FlipType", enum_table(lua, &[("HORIZONTAL", 0), ("VERTICAL", 1)])?)?;
    g.set("AniDir", enum_table(lua, &[("FORWARD", 0), ("REVERSE", 1), ("PING_PONG", 2), ("PING_PONG_REVERSE", 3)])?)?;
    g.set(
        "SpriteSheetType",
        enum_table(lua, &[("HORIZONTAL", 1), ("VERTICAL", 2), ("ROWS", 3), ("COLUMNS", 4), ("PACKED", 5)])?,
    )?;
    g.set("SpriteSheetDataFormat", enum_table(lua, &[("JSON_HASH", 0), ("JSON_ARRAY", 1)])?)?;
    g.set("RangeType", enum_table(lua, &[("EMPTY", 0), ("LAYERS", 1), ("FRAMES", 2), ("CELS", 3)])?)?;
    g.set(
        "ColorMode",
        enum_table(lua, &[("RGB", 0), ("GRAY", 1), ("GRAYSCALE", 1), ("INDEXED", 2), ("BITMAP", 3), ("TILEMAP", 4)])?,
    )?;

    g.set("Version", lua.create_function(|_, a: MultiValue| version_ctor(a))?)?;
    let app: Table = g.get("app")?;
    app.set("version", VersionH::parse(env!("CARGO_PKG_VERSION")).expect("a valid package version"))?;

    // app.command.X{...}
    let command = lua.create_table()?;
    let meta = lua.create_table()?;
    let c = ctx.clone();
    meta.set(
        "__index",
        lua.create_function(move |lua, (_t, key): (Value, Value)| {
            let Value::String(name) = key else { return Ok(Value::Nil) };
            let name = name.to_string_lossy();
            let c = c.clone();
            let f = lua.create_function(move |lua, p: Option<Table>| run_command(lua, &c, &name, Params(p)))?;
            Ok(Value::Function(f))
        })?,
    )?;
    command.set_metatable(Some(meta))?;
    app.set("command", command)?;

    // app.refresh() has nothing to redraw; app.undo() / app.redo() are the Undo / Redo commands.
    app.set("refresh", lua.create_function(|_, ()| Ok(()))?)?;
    for (name, cmd) in [("undo", "Undo"), ("redo", "Redo")] {
        let c = ctx.clone();
        app.set(
            name,
            lua.create_function(move |lua, ()| {
                let a = active_ctx(&c);
                if a.doc.closed.get() || !a.has_doc.get() {
                    return Ok(());
                }
                // Nothing to step through is not an error here (Aseprite's app.undo() does nothing then).
                let _ = run_command(lua, &c, cmd, Params(None));
                Ok(())
            })?,
        )?;
    }

    // app.transaction([label,] fn): the changes are undone if `fn` fails.
    let c = ctx.clone();
    app.set(
        "transaction",
        lua.create_function(move |_, (a, b): (Value, Option<Value>)| {
            let (label, f) = match (a, b) {
                (Value::Function(f), _) => (None, f),
                (Value::String(l), Some(Value::Function(f))) => (Some(l.to_string_lossy()), f),
                _ => return Err(Error::runtime("app.transaction expects ([label,] function)")),
            };
            let act = active_ctx(&c);
            act.undo.boundary(&act);
            let (sprite, mask) = (act.sprite().clone(), act.mask());
            match f.call::<MultiValue>(()) {
                Ok(v) => {
                    act.undo.boundary(&act);
                    if let Some(l) = label {
                        *c.label.borrow_mut() = Some(l);
                    }
                    Ok(v)
                }
                Err(e) => {
                    *act.sprite() = sprite;
                    *act.mask.borrow_mut() = mask;
                    act.invalidate_handles();
                    clamp_site(&act);
                    Err(e)
                }
            }
        })?,
    )?;
    Ok(())
}

// MARK: Sprite methods

pub(super) fn add_methods<M: UserDataMethods<SpriteH>>(m: &mut M) {
    m.add_method("flatten", |_, t, ()| flatten(&t.0));
    m.add_method("resize", |_, t, mut a: MultiValue| {
        let (w, h) = match a.front() {
            Some(Value::Integer(_) | Value::Number(_)) => {
                let w = a.pop_front().as_ref().and_then(super::geometry::int_of);
                let h = a.pop_front().as_ref().and_then(super::geometry::int_of);
                (w, h)
            }
            Some(v) => super::geometry::size_of(v).map_or((None, None), |(w, h)| (Some(w), Some(h))),
            None => (None, None),
        };
        let (Some(w), Some(h)) = (w, h) else {
            return Err(Error::runtime("resize: expected (width, height) or a Size"));
        };
        resize(&t.0, w, h)
    });
    m.add_method("crop", |_, t, mut a: MultiValue| {
        let (x, y, w, h) = if a.is_empty() {
            let s = t.0.sprite();
            (0, 0, s.width as i64, s.height as i64)
        } else {
            let r = super::geometry::take_rect(&mut a, "crop")?;
            (r.x, r.y, r.w, r.h)
        };
        crop(&t.0, x, y, w, h)
    });
}
