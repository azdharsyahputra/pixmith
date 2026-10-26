//! Documents of a script run: `Sprite(...)`, `app.open`, `app.sprites`, `app.activeSprite`,
//! `sprite:saveAs` / `saveCopyAs` / `close`, `sprite.filename`, and the result a run reports to the front end
//! (docs/LUA_HOST_API.md, section 2).
//!
//! The document a script runs on (the "original") is edited in place and committed as one undo step. Every
//! other document lives in its own [`Ctx`] (with its own `Editor`) until the run ends; then the survivors are
//! handed to the front end as [`NewDocument`]s.

use super::access::{self, Access};
use super::color::ColorH;
use super::{color_mode, Ctx, FrameH, LayerH, SpriteH};
use crate::document::{Cel, Sprite};
use crate::formats::export_plan::{EncodeOptions, ExportOptions, Format};
use crate::formats::raster::{ImageFormat, ImageOptions};
use mlua::{Error, Lua, MetaMethod, MultiValue, Result as LuaResult, Table, UserDataFields, UserDataMethods, Value};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

/// Most documents one run may create or open (besides the one it runs on).
pub const MAX_NEW_DOCUMENTS: usize = 32;
/// Most pixel bytes (canvas x 4 per sprite, at creation) the documents of one run may start with.
const MAX_NEW_DOCUMENT_BYTES: usize = 512 << 20;
/// Largest file `app.open` reads.
const MAX_OPEN_BYTES: u64 = 512 << 20;

/// A document the script created or opened, handed to the front end when the run succeeds.
#[derive(Debug)]
pub struct NewDocument {
    pub sprite: Sprite,
    /// Set if the script opened the sprite from a file or saved it with `saveAs`.
    pub path: Option<PathBuf>,
    /// False when `sprite` is exactly what `path` holds.
    pub modified: bool,
}

/// Which document is active after a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActiveDoc {
    /// The script did not switch documents.
    #[default]
    Unchanged,
    /// The document the script ran on.
    Original,
    /// Index into `new_documents`.
    New(usize),
}

/// The document-related part of a run's result.
#[derive(Debug, Default)]
pub struct Docs {
    pub new_documents: Vec<NewDocument>,
    pub active: ActiveDoc,
    pub close_original: bool,
    pub original_path: Option<PathBuf>,
    pub original_saved: bool,
}

/// Per-document state.
pub(super) struct Doc {
    pub original: bool,
    pub path: RefCell<Option<PathBuf>>,
    /// The sprite as of its last save to `path` (None: never saved or opened from a file).
    pub saved: RefCell<Option<Sprite>>,
    pub closed: Cell<bool>,
}

impl Doc {
    pub fn new(original: bool, path: Option<PathBuf>) -> Doc {
        Doc { original, path: RefCell::new(path), saved: RefCell::new(None), closed: Cell::new(false) }
    }
}

/// State shared by all documents of a run.
pub(super) struct Shared {
    /// Documents created or opened by the script, in creation order.
    pub docs: RefCell<Vec<Rc<Ctx>>>,
    pub original: RefCell<Weak<Ctx>>,
    pub active: RefCell<Weak<Ctx>>,
    pub active_changed: Cell<bool>,
    pub access: access::RunAccess,
    pub waited: Cell<Duration>,
    pub close_original: Cell<bool>,
    pub original_saved_as: RefCell<Option<PathBuf>>,
    pub bytes: Cell<usize>,
}

impl Shared {
    pub fn new() -> Rc<Shared> {
        Rc::new(Shared {
            docs: RefCell::new(Vec::new()),
            original: RefCell::new(Weak::new()),
            active: RefCell::new(Weak::new()),
            active_changed: Cell::new(false),
            access: Default::default(),
            waited: Cell::new(Duration::ZERO),
            close_original: Cell::new(false),
            original_saved_as: RefCell::new(None),
            bytes: Cell::new(0),
        })
    }

    /// Starts a new run (a session transaction): forgets the previous run's documents and answers.
    pub fn reset_run(&self) {
        self.docs.borrow_mut().clear();
        *self.active.borrow_mut() = Weak::new();
        self.active_changed.set(false);
        self.close_original.set(false);
        *self.original_saved_as.borrow_mut() = None;
        self.bytes.set(0);
        self.access.reset_run();
    }
}

/// The active document: the one made active last, else the one the script runs on.
pub(super) fn active_ctx(ctx: &Rc<Ctx>) -> Rc<Ctx> {
    let sh = &ctx.shared;
    if let Some(a) = sh.active.borrow().upgrade().filter(|a| !a.doc.closed.get()) {
        return a;
    }
    sh.original.borrow().upgrade().unwrap_or_else(|| ctx.clone())
}

fn activate(ctx: &Rc<Ctx>, target: &Rc<Ctx>) {
    *ctx.shared.active.borrow_mut() = Rc::downgrade(target);
    ctx.shared.active_changed.set(true);
}

/// Every open document of the run that scripts can see: the original (if there is one), then the new ones.
fn all_docs(ctx: &Rc<Ctx>) -> Vec<Rc<Ctx>> {
    let mut v = Vec::new();
    if let Some(o) = ctx.shared.original.borrow().upgrade() {
        if o.has_doc.get() && !o.doc.closed.get() {
            v.push(o);
        }
    }
    v.extend(ctx.shared.docs.borrow().iter().filter(|d| !d.doc.closed.get()).cloned());
    v
}

fn live(ctx: &Ctx) -> LuaResult<()> {
    if ctx.doc.closed.get() {
        Err(Error::runtime("the sprite is closed"))
    } else {
        Ok(())
    }
}

fn mode_number(m: color_mode::ColorMode) -> i64 {
    match m {
        color_mode::ColorMode::Rgba => 0,
        color_mode::ColorMode::Grayscale => 1,
        color_mode::ColorMode::Indexed => 2,
    }
}

fn mode_from(n: i64) -> LuaResult<color_mode::ColorMode> {
    match n {
        0 => Ok(color_mode::ColorMode::Rgba),
        1 => Ok(color_mode::ColorMode::Grayscale),
        2 => Ok(color_mode::ColorMode::Indexed),
        _ => Err(Error::runtime(format!("color mode {n} is not supported (use ColorMode.RGB, GRAYSCALE or INDEXED)"))),
    }
}

// MARK: Creating documents

/// Registers a new document of the run and makes it the active one.
fn add_doc(ctx: &Rc<Ctx>, sprite: Sprite, path: Option<PathBuf>, saved: bool) -> LuaResult<Rc<Ctx>> {
    let sh = &ctx.shared;
    if sh.docs.borrow().len() >= MAX_NEW_DOCUMENTS {
        return Err(Error::runtime(format!("a script may create or open at most {MAX_NEW_DOCUMENTS} sprites")));
    }
    let bytes = (sprite.width as usize * sprite.height as usize * 4).saturating_mul(sprite.frame_count().max(1));
    if sh.bytes.get().saturating_add(bytes) > MAX_NEW_DOCUMENT_BYTES {
        return Err(Error::runtime(format!(
            "a script may create sprites of at most {} MB of pixels together",
            MAX_NEW_DOCUMENT_BYTES >> 20
        )));
    }
    sh.bytes.set(sh.bytes.get() + bytes);
    let child = Ctx::child(ctx, sprite, path);
    if saved {
        *child.doc.saved.borrow_mut() = Some(child.sprite().clone());
    }
    sh.docs.borrow_mut().push(child.clone());
    activate(ctx, &child);
    Ok(child)
}

/// A new, empty sprite: one transparent layer and frame, with an (empty) cel like Aseprite's.
fn blank(w: i64, h: i64, mode: color_mode::ColorMode) -> LuaResult<Sprite> {
    let bad = || {
        Error::runtime(format!(
            "Sprite: invalid size {w}x{h} (1..65535, at most {} pixels)",
            crate::document::MAX_CANVAS_PIXELS
        ))
    };
    if w < 1 || h < 1 || w > u16::MAX as i64 || h > u16::MAX as i64 {
        return Err(bad());
    }
    if (w as usize).saturating_mul(h as usize) > crate::document::MAX_CANVAS_PIXELS {
        return Err(bad());
    }
    let mut s = Sprite::new(w as u32, h as u32);
    s.cels[0][0] = Some(Arc::new(Cel { pixels: vec![0; w as usize * h as usize * 4], ..Default::default() }));
    if mode != color_mode::ColorMode::Rgba {
        color_mode::convert(&mut s, mode, 0);
    }
    Ok(s)
}

fn spec_of(t: &Table) -> Option<(i64, i64, i64)> {
    let int = |names: &[&str]| {
        names.iter().find_map(|n| match t.raw_get::<Value>(*n) {
            Ok(Value::Integer(i)) => Some(i),
            Ok(Value::Number(n)) if n.is_finite() => Some(n as i64),
            _ => None,
        })
    };
    Some((int(&["width", "w"])?, int(&["height", "h"])?, int(&["colorMode"]).unwrap_or(0)))
}

/// `Sprite(...)`: (w, h [, colorMode]), (ImageSpec), {fromFile = path}, (otherSprite) or an Image-like spec table.
fn sprite_ctor(lua: &Lua, ctx: &Rc<Ctx>, a: MultiValue) -> LuaResult<SpriteH> {
    let bad =
        || Error::runtime("Sprite: expected (width, height [, colorMode]), an ImageSpec, {fromFile=path} or a Sprite");
    let v: Vec<Value> = a.into_iter().collect();
    let child = match v.as_slice() {
        [Value::Integer(_) | Value::Number(_), Value::Integer(_) | Value::Number(_), rest @ ..] => {
            let n = |x: &Value| super::geometry::int_of(x).ok_or_else(bad);
            let mode = rest.first().map(n).transpose()?.unwrap_or(0);
            let s = blank(n(&v[0])?, n(&v[1])?, mode_from(mode)?)?;
            add_doc(ctx, s, None, false)?
        }
        [Value::Table(t)] => {
            if let Ok(Value::String(p)) = t.raw_get::<Value>("fromFile") {
                return open_file(lua, ctx, &p.to_str()?, true).map(|s| s.expect("errors are raised when required"));
            }
            let (w, h, m) = spec_of(t).ok_or_else(bad)?;
            add_doc(ctx, blank(w, h, mode_from(m)?)?, None, false)?
        }
        [Value::UserData(u)] => {
            if let Ok(src) = u.borrow::<SpriteH>() {
                live(&src.0)?;
                let mut s = src.0.sprite().clone();
                s.flush_links();
                add_doc(ctx, s, None, false)?
            } else if let Some((w, h)) = super::geometry::size_of(&Value::UserData(u.clone())) {
                add_doc(ctx, blank(w, h, color_mode::ColorMode::Rgba)?, None, false)?
            } else {
                // An ImageSpec created by ImageSpec() is a table, handled above; an Image has a spec property.
                match super::image::image_of(&Value::UserData(u.clone())) {
                    Some(img) => {
                        let (w, h) = img.size()?;
                        let mut s = blank(w as i64, h as i64, color_mode::ColorMode::Rgba)?;
                        let data = img.snapshot()?;
                        s.cels[0][0] = Some(Arc::new(Cel { pixels: data, ..Default::default() }));
                        add_doc(ctx, s, None, false)?
                    }
                    None => return Err(bad()),
                }
            }
        }
        _ => return Err(bad()),
    };
    Ok(SpriteH(child))
}

// MARK: Opening

/// Reads a sprite from a file (a document, or any image format).
pub(super) fn read_sprite_file(path: &Path) -> Result<Sprite, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    if meta.len() > MAX_OPEN_BYTES {
        return Err(format!("{} is larger than {} MB", path.display(), MAX_OPEN_BYTES >> 20));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let s = crate::formats::raster::read_any(&bytes, &path.to_string_lossy())?;
    if s.width as usize * s.height as usize > crate::document::MAX_CANVAS_PIXELS {
        return Err("the file's canvas is too large".into());
    }
    Ok(s)
}

/// `app.open(path)` / `Sprite{fromFile}`. With `required`, a file that cannot be read raises an error; otherwise
/// the answer is `None` (Aseprite's `app.open` returns nil).
fn open_file(_lua: &Lua, ctx: &Rc<Ctx>, raw: &str, required: bool) -> LuaResult<Option<SpriteH>> {
    let path = access::check(ctx, raw, Access::Read)?;
    // `app.open` of a file that is already open gives that sprite; `Sprite{ fromFile }` always loads a new one.
    if !required {
        for d in all_docs(ctx) {
            if d.doc.path.borrow().as_deref().is_some_and(|p| access::normalize(p) == path) {
                activate(ctx, &d);
                return Ok(Some(SpriteH(d)));
            }
        }
    }
    match read_sprite_file(&path) {
        Ok(s) => {
            let is_doc = is_document_path(&path);
            let child = add_doc(ctx, s, is_doc.then(|| path.clone()), is_doc)?;
            if !is_doc {
                // An image opened as a sprite is not "the file" any more once saved elsewhere; Aseprite keeps
                // its name, so do the same.
                *child.doc.path.borrow_mut() = Some(path);
            }
            Ok(Some(SpriteH(child)))
        }
        Err(e) if required => Err(Error::runtime(e)),
        Err(_) => Ok(None),
    }
}

fn is_document_path(p: &Path) -> bool {
    matches!(ext_of(p).as_str(), "aseprite" | "ase" | "pxmt")
}

fn ext_of(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

// MARK: Saving

/// Writes `s` to `path` in the format its extension names: a document (`.aseprite`, `.ase`, `.pxmt`) or an
/// image export (`.png`, `.gif`, `.webp`, `.jpg`, `.bmp`, `.tga`, `.ico`, `.pcx`, `.flc`). A multi-frame sprite
/// saved as a still format writes one file per frame, `<name>-001.png`, ...
pub(super) fn write_sprite_file(path: &Path, s: &mut Sprite) -> Result<(), String> {
    s.flush_links();
    let text = path.to_string_lossy().into_owned();
    match ext_of(path).as_str() {
        "aseprite" | "ase" => crate::formats::durable::write_file(&text, &crate::formats::ase::write(s)),
        "pxmt" => crate::formats::pxmt::save_file(&text, s),
        ext => {
            let img = ImageFormat::from_extension(ext)
                .ok_or_else(|| format!("cannot save \"{text}\": unknown file extension \".{ext}\""))?;
            let fmt = match img {
                ImageFormat::Png if s.frame_count() > 1 => Format::Sequence,
                ImageFormat::Png => Format::Png,
                ImageFormat::Gif => Format::Gif,
                ImageFormat::Webp => Format::Webp,
                ImageFormat::Jpeg => Format::Jpeg,
                ImageFormat::Bmp => Format::Bmp,
                ImageFormat::Tga => Format::Tga,
                ImageFormat::Ico => Format::Ico,
                ImageFormat::Pcx => Format::Pcx,
                ImageFormat::Flc => Format::Flc,
            };
            let enc = EncodeOptions {
                scale: 1,
                frame: 0,
                sheet: Default::default(),
                write_json: false,
                image: ImageOptions { webp_animated: s.frame_count() > 1, ..Default::default() },
            };
            let base = if fmt == Format::Sequence {
                path.with_extension("").to_string_lossy().into_owned()
            } else {
                text.clone()
            };
            let files =
                crate::formats::export_plan::render_outputs(s, fmt, &ExportOptions::default(), &enc, &base, None)?;
            for (p, data) in files {
                crate::formats::durable::write_file(&p, &data)?;
            }
            Ok(())
        }
    }
}

fn save_as(ctx: &Rc<Ctx>, raw: &str, copy: bool) -> LuaResult<()> {
    live(ctx)?;
    let path = access::check(ctx, raw, Access::Write)?;
    write_sprite_file(&path, ctx.sprite()).map_err(Error::runtime)?;
    if !copy && is_document_path(&path) {
        *ctx.doc.path.borrow_mut() = Some(path.clone());
        *ctx.doc.saved.borrow_mut() = Some(ctx.sprite().clone());
        if ctx.doc.original {
            *ctx.shared.original_saved_as.borrow_mut() = Some(path);
        }
    }
    Ok(())
}

/// Saves a free-standing image (`image:saveAs`).
pub(super) fn save_image(ctx: &Rc<Ctx>, raw: &str, rgba: Vec<u8>, w: usize, h: usize) -> LuaResult<()> {
    let path = access::check(ctx, raw, Access::Write)?;
    let mut s = blank(w as i64, h as i64, color_mode::ColorMode::Rgba)?;
    s.cels[0][0] = Some(Arc::new(Cel { pixels: rgba, ..Default::default() }));
    write_sprite_file(&path, &mut s).map_err(Error::runtime)
}

// MARK: Lua API

pub(super) fn install(lua: &Lua, ctx: &Rc<Ctx>) -> LuaResult<()> {
    let g = lua.globals();
    let c = ctx.clone();
    g.set("Sprite", lua.create_function(move |lua, a: MultiValue| sprite_ctor(lua, &c, a))?)?;
    let app: Table = g.get("app")?;
    let c = ctx.clone();
    app.set("open", lua.create_function(move |lua, p: String| open_file(lua, &c, &p, false))?)?;
    Ok(())
}

pub(super) fn add_fields<F: UserDataFields<SpriteH>>(f: &mut F) {
    f.add_field_method_get("filename", |_, t| {
        Ok(t.0.doc.path.borrow().as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default())
    });
    f.add_field_method_set("filename", |_, t, v: String| {
        live(&t.0)?;
        *t.0.doc.path.borrow_mut() = Some(PathBuf::from(&v));
        if t.0.doc.original {
            *t.0.shared.original_saved_as.borrow_mut() = Some(PathBuf::from(v));
        }
        *t.0.doc.saved.borrow_mut() = None;
        Ok(())
    });
    // An ImageSpec of the canvas: `Image(sprite.spec)`.
    f.add_field_method_get("spec", |lua, t| {
        let s = t.0.sprite();
        let spec = lua.create_table()?;
        spec.set("width", s.width)?;
        spec.set("height", s.height)?;
        spec.set("colorMode", mode_number(s.color_mode))?;
        spec.set("transparentColor", s.transparent_index)?;
        Ok(spec)
    });
    f.add_field_method_get("isModified", |_, t| Ok(t.0.doc.saved.borrow().as_ref().is_none_or(|s| s != t.0.sprite())));
    f.add_field_method_get("colorMode", |_, t| Ok(mode_number(t.0.sprite().color_mode)));
    super::add_user_data_fields(f, |t: &SpriteH| super::UserDataH::new(&t.0, 0, 0, 0));
    f.add_field_method_get("rootLayers", |lua, t| {
        let s = t.0.sprite();
        let roots: Vec<LayerH> = s
            .layers
            .iter()
            .enumerate()
            .filter(|(_, l)| l.child_level == 0)
            .map(|(i, _)| LayerH::new(&t.0, i))
            .collect();
        lua.create_sequence_from(roots)
    });
    f.add_field_method_get("backgroundLayer", |_, t| {
        let s = t.0.sprite();
        Ok(s.layers.iter().position(|l| l.flags & 8 != 0).map(|i| LayerH::new(&t.0, i)))
    });
    f.add_field_method_get("transparentColor", |_, t| Ok(t.0.sprite().transparent_index));
    f.add_field_method_set("transparentColor", |_, t, v: Value| {
        let v = super::whole("sprite.transparentColor", &v)?;
        t.0.sprite().transparent_index = v.clamp(0, 255) as u8;
        Ok(())
    });
}

pub(super) fn add_methods<M: UserDataMethods<SpriteH>>(m: &mut M) {
    m.add_meta_method(MetaMethod::Eq, |_, t, o: Value| {
        Ok(match o {
            Value::UserData(u) => u.borrow::<SpriteH>().is_ok_and(|o| Rc::ptr_eq(&o.0, &t.0)),
            _ => false,
        })
    });
    m.add_meta_method(MetaMethod::ToString, |_, t, ()| {
        let s = t.0.sprite();
        Ok(format!("Sprite({}x{})", s.width, s.height))
    });
    m.add_method("saveAs", |_, t, p: String| save_as(&t.0, &p, false));
    m.add_method("saveCopyAs", |_, t, p: String| save_as(&t.0, &p, true));
    m.add_method("close", |_, t, ()| {
        live(&t.0)?;
        t.0.doc.closed.set(true);
        if t.0.doc.original {
            t.0.shared.close_original.set(true);
        }
        let sh = &t.0.shared;
        let was_active = sh.active.borrow().upgrade().is_some_and(|a| Rc::ptr_eq(&a, &t.0)) || t.0.doc.original;
        if was_active {
            // The previous document takes over.
            let prev = all_docs(&t.0).pop();
            match prev {
                Some(p) => activate(&t.0, &p),
                None => sh.active_changed.set(true),
            }
        }
        Ok(())
    });
}

/// `app` properties that depend on the documents of the run. `None`: not one of them.
pub(super) fn app_get(lua: &Lua, ctx: &Rc<Ctx>, name: &str) -> LuaResult<Option<Value>> {
    let a = active_ctx(ctx);
    let has = a.has_doc.get() && !a.doc.closed.get();
    let userdata = |v: Option<mlua::AnyUserData>| v.map_or(Value::Nil, Value::UserData);
    Ok(Some(match name {
        "sprite" | "activeSprite" => {
            if has {
                Value::UserData(lua.create_userdata(SpriteH(a))?)
            } else {
                Value::Nil
            }
        }
        "sprites" => {
            let docs = all_docs(ctx);
            Value::Table(lua.create_sequence_from(docs.into_iter().map(SpriteH))?)
        }
        "activeLayer" if has => Value::UserData(lua.create_userdata(LayerH::new(&a, a.layer.get()))?),
        "activeFrame" if has => Value::UserData(lua.create_userdata(FrameH::new(&a, a.frame.get()))?),
        "activeCel" if has => {
            let s = a.sprite();
            let (l, f) = (a.layer.get().min(s.layers.len() - 1), a.frame.get().min(s.frame_count() - 1));
            userdata(match s.cels[f][l].is_some() {
                true => Some(lua.create_userdata(super::CelH(a.clone(), l, f, a.epoch.get()))?),
                false => None,
            })
        }
        "activeImage" if has => {
            let s = a.sprite();
            let (l, f) = (a.layer.get().min(s.layers.len() - 1), a.frame.get().min(s.frame_count() - 1));
            match super::image::cel_image(&a, l, f) {
                Some(img) => Value::UserData(lua.create_userdata(img)?),
                None => Value::Nil,
            }
        }
        "site" => {
            let t = lua.create_table()?;
            for k in ["sprite", "layer", "frame", "cel", "image"] {
                let key = match k {
                    "sprite" => "activeSprite",
                    "layer" => "activeLayer",
                    "frame" => "activeFrame",
                    "cel" => "activeCel",
                    _ => "activeImage",
                };
                if let Some(v) = app_get(lua, ctx, key)? {
                    t.set(k, v)?;
                }
            }
            if has {
                t.set("frameNumber", a.frame.get() + 1)?;
            }
            Value::Table(t)
        }
        // The first tag that covers the active frame.
        "tag" | "activeTag" if has => {
            let f = a.frame.get() as u16;
            match a.sprite().tags.iter().position(|t| t.from <= f && f <= t.to) {
                Some(i) => Value::UserData(lua.create_userdata(super::TagH::new(&a, i))?),
                None => Value::Nil,
            }
        }
        // The selected range is always the active cel (Pixmith has no timeline selection scripts can see).
        "range" if has => {
            let s = a.sprite();
            let (l, f) = (a.layer.get().min(s.layers.len() - 1), a.frame.get().min(s.frame_count() - 1));
            let t = lua.create_table()?;
            let cel = s.cels[f][l].is_some();
            t.set("type", if cel { 3 } else { 0 })?;
            t.set("isEmpty", !cel)?;
            t.set("layers", lua.create_sequence_from([LayerH::new(&a, l)])?)?;
            t.set("frames", lua.create_sequence_from([FrameH::new(&a, f)])?)?;
            if cel {
                t.set("cels", lua.create_sequence_from([super::CelH(a.clone(), l, f, a.epoch.get())])?)?;
                if let Some(img) = super::image::cel_image(&a, l, f) {
                    t.set("images", lua.create_sequence_from([img])?)?;
                }
            }
            for k in ["cels", "images", "colors", "slices"] {
                if !t.contains_key(k)? {
                    t.set(k, lua.create_table()?)?;
                }
            }
            Value::Table(t)
        }
        "activeLayer" | "activeFrame" | "activeCel" | "activeImage" | "tag" | "activeTag" | "range" => Value::Nil,
        "fgColor" | "bgColor" => {
            let (fg, bg) = ctx.colors();
            let c = if name == "fgColor" { fg } else { bg };
            Value::UserData(lua.create_userdata(ColorH::from_rgba(ctx, c.to_be_bytes()))?)
        }
        _ => return Ok(None),
    }))
}

/// Assignments to `app` properties of the documents; false when `name` is not one of them.
pub(super) fn app_set(ctx: &Rc<Ctx>, name: &str, value: &Value) -> LuaResult<bool> {
    match name {
        "sprite" | "activeSprite" => {
            let Value::UserData(u) = value else {
                return Err(Error::runtime("expected a sprite"));
            };
            let s = u.borrow::<SpriteH>().map_err(|_| Error::runtime("expected a sprite"))?;
            live(&s.0)?;
            activate(ctx, &s.0);
            Ok(true)
        }
        "activeLayer" | "activeFrame" => {
            if matches!(value, Value::Number(n) if n.fract() != 0.0) {
                return Err(Error::runtime("expected a layer or frame number"));
            }
            let a = active_ctx(ctx);
            if name == "activeLayer" {
                let l = a.layer_of(value)?;
                if a.sprite().layers[l].is_group() {
                    return Err(Error::runtime("a group cannot be the active layer"));
                }
                a.layer.set(l);
            } else {
                a.frame.set(a.frame_of(value)?);
            }
            a.selected.set(true);
            Ok(true)
        }
        _ => Ok(false),
    }
}

// MARK: Reporting

/// Ends the run's documents: hands the survivors over (when `ok`) and releases every document context, which
/// also breaks the reference cycle between the documents and the shared state.
pub(super) fn finish(ctx: &Rc<Ctx>, ok: bool) -> Docs {
    let sh = &ctx.shared;
    let docs: Vec<Rc<Ctx>> = std::mem::take(&mut *sh.docs.borrow_mut());
    let active = active_ctx(ctx);
    let changed = sh.active_changed.get();
    let live: Vec<Rc<Ctx>> = docs.iter().filter(|d| !d.doc.closed.get()).cloned().collect();
    let active_doc = if !ok || !changed {
        ActiveDoc::Unchanged
    } else if let Some(i) = live.iter().position(|d| Rc::ptr_eq(d, &active)) {
        ActiveDoc::New(i)
    } else if ctx.has_doc.get() && !sh.close_original.get() {
        ActiveDoc::Original
    } else {
        ActiveDoc::Unchanged
    };
    let original = sh.original.borrow().upgrade();
    let mut out = Docs::default();
    if ok {
        for d in &live {
            let mut sprite = std::mem::replace(&mut d.editor().sprite, Sprite::new(1, 1));
            if sprite.color_mode != color_mode::ColorMode::Rgba {
                color_mode::conform_all(&mut sprite);
            }
            sprite.flush_links();
            // An image opened as a sprite reaches the front end as an untitled document (like File > Open of a PNG).
            let path = d.doc.path.borrow().clone().filter(|p| is_document_path(p));
            let modified = d.doc.saved.borrow().as_ref().is_none_or(|s| *s != sprite);
            out.new_documents.push(NewDocument { sprite, path, modified });
        }
        out.active = active_doc;
        out.close_original = sh.close_original.get();
        out.original_path = sh.original_saved_as.borrow().clone();
        out.original_saved = match (&original, &out.original_path) {
            (Some(o), Some(_)) => o.doc.saved.borrow().as_ref().is_some_and(|s| s == o.sprite()),
            _ => false,
        };
    }
    for d in &docs {
        d.doc.closed.set(true);
        // Nothing of the session may be reachable from a document that outlives the run.
        *d.rt.borrow_mut() = None;
    }
    *sh.active.borrow_mut() = Weak::new();
    out
}
