//! Aseprite's `Image` for Lua scripts: free-standing RGBA images (`Image(w, h)`,
//! `Image{ fromFile = "/abs/path.png" }`), and live views of a cel (`cel.image`).
//!
//! Everything that touches many pixels (`image.bytes`, `drawImage`, `newCel`, `cel.image = img`,
//! `sprite:setPixels(layer, frame, image)`) runs in Rust, so a 1024 x 1024 image costs a few VM
//! instructions however it is used. Images are always RGBA in memory; the color mode a script passes
//! to `Image(w, h, colorMode)` is only remembered (cels snap to the sprite's color mode when written).
//!
//! Like in Aseprite a cel may extend beyond the canvas: `cel.position` and `cel.bounds` describe the box
//! around all of the cel's non-transparent pixels (negative coordinates and sizes reaching past the
//! canvas included), and `cel.image` is a view of exactly that box. Moving a cel off the canvas and
//! back loses nothing (see `document::offcanvas`).

use super::color::{self, PixelArg};
use super::geometry::{int_of, size_of, take_opt_int, take_point, take_rect, PointV, RectV, SizeV};
use super::{CelH, Ctx, Snap, SpriteH};
use crate::document::color_mode::ColorMode;
use crate::document::Sprite;
use mlua::{Error, Lua, MultiValue, Result as LuaResult, Table, UserData, UserDataFields, UserDataMethods, Value};
use std::cell::{Cell, RefCell};
use std::io::Read;
use std::rc::Rc;

/// Most pixels one image may hold (the largest canvas).
const MAX_IMAGE_PIXELS: usize = crate::document::MAX_CANVAS_PIXELS;
/// Most image memory (Rust side, not counted by the Lua memory limit) a run may keep alive at once.
const MAX_LIVE_IMAGE_BYTES: usize = 1 << 29;
/// Largest PNG file `Image{ fromFile = ... }` reads.
const MAX_PNG_FILE_BYTES: u64 = 256 << 20;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

enum Kind {
    /// A free-standing image.
    Own { w: usize, h: usize, px: Vec<u8> },
    /// A window onto a cel: pixel (x, y) of the image is pixel (ox + x, oy + y) of the cel, in canvas
    /// coordinates (which may lie beyond the canvas).
    View { layer: usize, frame: usize, ox: i64, oy: i64, w: usize, h: usize, epoch: u64 },
}

pub(super) struct ImageInner {
    ctx: Rc<Ctx>,
    mode: Cell<i64>,
    kind: RefCell<Kind>,
}

impl Drop for ImageInner {
    fn drop(&mut self) {
        if let Kind::Own { px, .. } = self.kind.get_mut() {
            self.ctx.image_bytes.set(self.ctx.image_bytes.get().saturating_sub(px.len()));
        }
    }
}

#[derive(Clone)]
pub(super) struct ImageH(Rc<ImageInner>);

fn reserve(ctx: &Ctx, w: usize, h: usize) -> LuaResult<usize> {
    if w == 0 || h == 0 {
        return Err(Error::runtime("an image needs a width and a height of at least 1"));
    }
    let pixels = w
        .checked_mul(h)
        .filter(|&n| n <= MAX_IMAGE_PIXELS)
        .ok_or_else(|| Error::runtime(format!("image {w}x{h} is too large (at most {MAX_IMAGE_PIXELS} pixels)")))?;
    let bytes = pixels * 4;
    if ctx.image_bytes.get().saturating_add(bytes) > MAX_LIVE_IMAGE_BYTES {
        return Err(Error::runtime(format!(
            "a script may keep at most {} MB of images alive at once",
            MAX_LIVE_IMAGE_BYTES >> 20
        )));
    }
    ctx.image_bytes.set(ctx.image_bytes.get() + bytes);
    Ok(bytes)
}

impl ImageH {
    /// A free-standing image; `px` must hold `w * h * 4` bytes, or `None` for a transparent one.
    fn own(ctx: &Rc<Ctx>, w: usize, h: usize, px: Option<Vec<u8>>, mode: i64) -> LuaResult<ImageH> {
        let bytes = reserve(ctx, w, h)?;
        let px = px.unwrap_or_else(|| vec![0; bytes]);
        debug_assert_eq!(px.len(), bytes);
        Ok(ImageH(Rc::new(ImageInner {
            ctx: ctx.clone(),
            mode: Cell::new(mode),
            kind: RefCell::new(Kind::Own { w, h, px }),
        })))
    }

    fn view(ctx: &Rc<Ctx>, layer: usize, frame: usize, r: Box4) -> ImageH {
        ImageH(Rc::new(ImageInner {
            ctx: ctx.clone(),
            mode: Cell::new(0),
            kind: RefCell::new(Kind::View { layer, frame, ox: r.0, oy: r.1, w: r.2, h: r.3, epoch: ctx.epoch.get() }),
        }))
    }

    pub(super) fn size(&self) -> LuaResult<(usize, usize)> {
        match &*self.0.kind.borrow() {
            Kind::Own { w, h, .. } => Ok((*w, *h)),
            Kind::View { w, h, epoch, .. } => {
                self.0.ctx.check(*epoch, "image")?;
                Ok((*w, *h))
            }
        }
    }

    fn color_mode(&self) -> i64 {
        match &*self.0.kind.borrow() {
            Kind::Own { .. } => self.0.mode.get(),
            Kind::View { .. } => match self.0.ctx.sprite().color_mode {
                ColorMode::Rgba => 0,
                ColorMode::Grayscale => 1,
                ColorMode::Indexed => 2,
            },
        }
    }

    /// Calls `f(pixels, w, h)` with the image's RGBA bytes.
    fn read<R>(&self, f: impl FnOnce(&[u8], usize, usize) -> R) -> LuaResult<R> {
        let (w, h) = self.size()?;
        match &*self.0.kind.borrow() {
            Kind::Own { px, .. } => Ok(f(px, w, h)),
            Kind::View { layer, frame, ox, oy, .. } => {
                let s = self.0.ctx.sprite();
                let out = match s.cels[*frame][*layer].as_ref() {
                    Some(cel) => cel.read_region(s.width as usize, s.height as usize, (*ox as i32, *oy as i32, w, h)),
                    None => vec![0u8; w * h * 4],
                };
                Ok(f(&out, w, h))
            }
        }
    }

    /// Calls `f(pixels, w, h)` with mutable RGBA bytes; for a cel view the result is written back.
    fn modify<R>(&self, f: impl FnOnce(&mut [u8], usize, usize) -> R) -> LuaResult<R> {
        let (w, h) = self.size()?;
        let mut kind = self.0.kind.borrow_mut();
        match &mut *kind {
            Kind::Own { px, .. } => Ok(f(px, w, h)),
            Kind::View { layer, frame, ox, oy, .. } => {
                let (layer, frame, ox, oy) = (*layer, *frame, *ox, *oy);
                drop(kind);
                let mut data = self.read(|p, _, _| p.to_vec())?;
                let r = f(&mut data, w, h);
                blit_ex(self.0.ctx.sprite(), layer, frame, &data, w, h, ox, oy, true)?;
                Ok(r)
            }
        }
    }

    /// The pixel value `getPixel` reports for RGBA `c`, in this image's color mode (Aseprite's formats:
    /// RGB `0xAABBGGRR`, grayscale `gray | alpha << 8`, indexed: the palette index).
    fn value_of(&self, c: [u8; 4]) -> i64 {
        match self.color_mode() {
            1 => (crate::document::color_mode::luma(c) as i64) | (c[3] as i64) << 8,
            2 => {
                let a = super::sprites::active_ctx(&self.0.ctx);
                let s = a.sprite();
                if c[3] == 0 {
                    s.transparent_index as i64
                } else {
                    color::nearest_index(&s.palette, c) as i64
                }
            }
            _ => color::px_to_pixel(c) as i64,
        }
    }

    /// RGBA for a pixel value or `Color` argument of the pixel functions (the inverse of `value_of`).
    fn rgba_of(&self, a: PixelArg) -> [u8; 4] {
        match a {
            PixelArg::Rgba(c) => c,
            PixelArg::Value(v) => match self.color_mode() {
                1 => {
                    let g = (v & 0xFF) as u8;
                    [g, g, g, ((v >> 8) & 0xFF) as u8]
                }
                2 => {
                    let act = super::sprites::active_ctx(&self.0.ctx);
                    let s = act.sprite();
                    if v == s.transparent_index as u32 {
                        [0; 4]
                    } else {
                        s.palette.get(v as usize).copied().unwrap_or([0, 0, 0, 0])
                    }
                }
                _ => color::pixel_to_px(v),
            },
        }
    }

    fn arg(&self, v: &Value) -> LuaResult<[u8; 4]> {
        Ok(self.rgba_of(color::pixel_arg(v)?))
    }

    pub(super) fn snapshot(&self) -> LuaResult<Vec<u8>> {
        self.read(|p, _, _| p.to_vec())
    }

    /// Replaces every pixel with `data` (`w * h * 4` bytes).
    fn replace_all(&self, data: &[u8]) -> LuaResult<()> {
        let (w, h) = self.size()?;
        if data.len() != w * h * 4 {
            return Err(Error::runtime(format!(
                "image.bytes needs {} bytes ({w} x {h} x 4, RGBA), got {}",
                w * h * 4,
                data.len()
            )));
        }
        let mut kind = self.0.kind.borrow_mut();
        match &mut *kind {
            Kind::Own { px, .. } => {
                px.copy_from_slice(data);
                Ok(())
            }
            Kind::View { layer, frame, ox, oy, .. } => {
                let (layer, frame, ox, oy) = (*layer, *frame, *ox, *oy);
                drop(kind);
                blit_ex(self.0.ctx.sprite(), layer, frame, data, w, h, ox, oy, true)
            }
        }
    }

    fn get(&self, x: i64, y: i64) -> LuaResult<[u8; 4]> {
        let (w, h) = self.size()?;
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
            return Ok([0; 4]);
        }
        let (x, y) = (x as usize, y as usize);
        Ok(match &*self.0.kind.borrow() {
            Kind::Own { px, .. } => px[(y * w + x) * 4..][..4].try_into().unwrap_or([0; 4]),
            Kind::View { layer, frame, ox, oy, .. } => {
                let s = self.0.ctx.sprite();
                s.cels[*frame][*layer]
                    .as_ref()
                    .map_or([0; 4], |c| c.pixel_at(s.width as usize, s.height as usize, ox + x as i64, oy + y as i64))
            }
        })
    }

    /// Replaces (or, with `blend`, composites) one pixel; out of range is ignored.
    fn put(&self, x: i64, y: i64, c: [u8; 4], blend: bool) -> LuaResult<()> {
        let (w, h) = self.size()?;
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
            return Ok(());
        }
        let c = if blend {
            let mut d = self.get(x, y)?;
            crate::document::blend_over(&mut d, &c, 255);
            d
        } else {
            c
        };
        let (xu, yu) = (x as usize, y as usize);
        let mut kind = self.0.kind.borrow_mut();
        match &mut *kind {
            Kind::Own { px, .. } => {
                px[(yu * w + xu) * 4..][..4].copy_from_slice(&c);
                Ok(())
            }
            Kind::View { layer, frame, ox, oy, .. } => {
                let (layer, frame, cx, cy) = (*layer, *frame, *ox + xu as i64, *oy + yu as i64);
                drop(kind);
                blit_ex(self.0.ctx.sprite(), layer, frame, &c, 1, 1, cx, cy, true)
            }
        }
    }
}

/// A box (x, y, w, h) in canvas coordinates; x and y may be negative.
type Box4 = (i64, i64, usize, usize);

/// Copies `src` (`sw * sh` RGBA) onto `dst` (`dw * dh` RGBA) with its top-left at (px, py), clipped. Replaces.
#[allow(clippy::too_many_arguments)]
fn copy_clipped(dst: &mut [u8], dw: usize, dh: usize, src: &[u8], sw: usize, sh: usize, px: i64, py: i64) {
    let x0 = px.max(0);
    let y0 = py.max(0);
    let x1 = px.saturating_add(sw as i64).min(dw as i64);
    let y1 = py.saturating_add(sh as i64).min(dh as i64);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let n = (x1 - x0) as usize * 4;
    for y in y0..y1 {
        let s = (((y - py) as usize) * sw + (x0 - px) as usize) * 4;
        let d = (y as usize * dw + x0 as usize) * 4;
        dst[d..d + n].copy_from_slice(&src[s..s + n]);
    }
}

/// Puts `src` (`sw * sh` RGBA) into the cel of `layer`/`frame` with its top-left at canvas (px, py),
/// clipped to the canvas and replacing what was there. Creates the cel; snaps colors to the sprite's
/// color mode; keeps tilemap layers and linked cels in sync.
#[allow(clippy::too_many_arguments)]
pub(super) fn blit(
    s: &mut Sprite,
    layer: usize,
    frame: usize,
    src: &[u8],
    sw: usize,
    sh: usize,
    px: i64,
    py: i64,
) -> LuaResult<()> {
    blit_ex(s, layer, frame, src, sw, sh, px, py, false)
}

/// `blit`; with `beyond` the part that falls outside the canvas is kept in the cel instead of being
/// clipped (except on tilemap layers, which have no content beyond the canvas).
#[allow(clippy::too_many_arguments)]
pub(super) fn blit_ex(
    s: &mut Sprite,
    layer: usize,
    frame: usize,
    src: &[u8],
    sw: usize,
    sh: usize,
    px: i64,
    py: i64,
    beyond: bool,
) -> LuaResult<()> {
    debug_assert_eq!(src.len(), sw * sh * 4);
    let snapped: Vec<u8>;
    let data: &[u8] = if s.color_mode == ColorMode::Rgba {
        src
    } else {
        let mut snap = Snap::new(s);
        snapped = src.chunks_exact(4).flat_map(|c| snap.get([c[0], c[1], c[2], c[3]])).collect();
        &snapped
    };
    let (cw, ch) = (s.width as usize, s.height as usize);
    let tilemap = s.layers[layer].kind == crate::document::tilemap::KIND_TILEMAP;
    let Some(cel) = s.cel_mut(layer, frame) else {
        return Err(Error::runtime("this layer cannot be drawn on (a group, or a locked reference layer)"));
    };
    let inside =
        px >= 0 && py >= 0 && px.saturating_add(sw as i64) <= cw as i64 && py.saturating_add(sh as i64) <= ch as i64;
    if beyond && !tilemap && !inside {
        let (Ok(x), Ok(y)) = (i32::try_from(px), i32::try_from(py)) else {
            return Err(Error::runtime("the position is too far from the canvas"));
        };
        cel.write_region(cw, ch, (x, y, sw, sh), data)
            .map_err(|e| Error::runtime(format!("cannot place the image: {e}")))?;
    } else {
        copy_clipped(&mut cel.pixels, cw, ch, data, sw, sh, px, py);
    }
    if tilemap {
        let (x0, y0) = (px.clamp(0, cw as i64) as i32, py.clamp(0, ch as i64) as i32);
        let x1 = px.saturating_add(sw as i64).clamp(0, cw as i64) as i32;
        let y1 = py.saturating_add(sh as i64).clamp(0, ch as i64) as i32;
        crate::document::tilemap::sync_cel(s, layer, frame, Some((x0, y0, x1, y1)));
    }
    s.flush_links();
    Ok(())
}

// MARK: Cels

/// The box around all of the cel's non-transparent pixels, beyond the canvas included: (x, y, w, h).
pub(super) fn cel_bbox(s: &Sprite, layer: usize, frame: usize) -> Option<Box4> {
    let cel = s.cels.get(frame)?.get(layer)?.as_ref()?;
    cel.full_bounds(s.width as usize, s.height as usize).map(|(x, y, w, h)| (x as i64, y as i64, w, h))
}

pub(super) fn cel_rect(s: &Sprite, layer: usize, frame: usize) -> RectV {
    cel_bbox(s, layer, frame).map_or_else(RectV::default, |(x, y, w, h)| RectV::new(x, y, w as i64, h as i64))
}

/// `cel.image`: a live view of the cel's pixels, or `None` for an empty cel.
pub(super) fn cel_image(ctx: &Rc<Ctx>, layer: usize, frame: usize) -> Option<ImageH> {
    cel_bbox(ctx.sprite(), layer, frame).map(|b| ImageH::view(ctx, layer, frame, b))
}

/// `cel.image = img`: the cel now shows `img` at the cel's current position (the top-left if it was empty).
pub(super) fn set_cel_image(ctx: &Rc<Ctx>, layer: usize, frame: usize, img: &ImageH) -> LuaResult<()> {
    let at = cel_bbox(ctx.sprite(), layer, frame).map_or((0, 0), |b| (b.0, b.1));
    place_cel(ctx, layer, frame, Some(img), at)
}

/// Replaces the cel with `img` (or an empty cel) at canvas position `at`, which may be beyond the canvas.
pub(super) fn place_cel(
    ctx: &Rc<Ctx>,
    layer: usize,
    frame: usize,
    img: Option<&ImageH>,
    at: (i64, i64),
) -> LuaResult<()> {
    let (data, w, h) = match img {
        Some(i) => {
            let (w, h) = i.size()?;
            (i.snapshot()?, w, h)
        }
        None => (Vec::new(), 0, 0),
    };
    let s = ctx.sprite();
    s.flush_links();
    s.cels[frame][layer] = None;
    if s.cel_mut(layer, frame).is_none() {
        return Err(Error::runtime("this layer cannot be drawn on (a group, or a locked reference layer)"));
    }
    if w > 0 {
        blit_ex(s, layer, frame, &data, w, h, at.0, at.1, true)?;
    } else {
        s.flush_links();
    }
    Ok(())
}

/// `cel.position = p`: moves the cel's content. What leaves the canvas is kept and comes back when
/// the cel is moved back.
pub(super) fn set_cel_position(ctx: &Rc<Ctx>, layer: usize, frame: usize, p: PointV) -> LuaResult<()> {
    let s = ctx.sprite();
    if s.layers[layer].kind == crate::document::tilemap::KIND_TILEMAP {
        return Err(Error::runtime("a tilemap cel cannot be moved; use layer:setTile"));
    }
    let Some((x, y, _, _)) = cel_bbox(s, layer, frame) else { return Ok(()) };
    if p.x == x && p.y == y {
        return Ok(());
    }
    let (dx, dy) = (p.x.checked_sub(x), p.y.checked_sub(y));
    let (Some(dx), Some(dy)) = (dx.and_then(|v| i32::try_from(v).ok()), dy.and_then(|v| i32::try_from(v).ok())) else {
        return Err(Error::runtime("the position is too far from the canvas"));
    };
    let (cw, ch) = (s.width as usize, s.height as usize);
    let Some(cel) = s.cel_mut(layer, frame) else {
        return Err(Error::runtime("this layer cannot be drawn on (a group, or a locked reference layer)"));
    };
    cel.translate(cw, ch, dx, dy).map_err(|_| Error::runtime("the position is too far from the canvas"))?;
    s.flush_links();
    Ok(())
}

// MARK: PNG files

/// Reads and decodes a PNG file. Only absolute paths are accepted and only PNG data is decoded.
fn load_png(path: &str) -> LuaResult<(usize, usize, Vec<u8>)> {
    let p = std::path::Path::new(path);
    if !p.is_absolute() {
        return Err(Error::runtime(format!("Image{{ fromFile }} needs an absolute path, got \"{path}\"")));
    }
    let fail = |e: std::io::Error| Error::runtime(format!("cannot read {path}: {e}"));
    // Windows cannot open a directory as a file, so ask before opening to give the same answer everywhere.
    if !std::fs::metadata(p).map_err(fail)?.is_file() {
        return Err(Error::runtime(format!("{path} is not a file")));
    }
    let file = std::fs::File::open(p).map_err(fail)?;
    let meta = file.metadata().map_err(fail)?;
    if !meta.is_file() {
        return Err(Error::runtime(format!("{path} is not a file")));
    }
    if meta.len() > MAX_PNG_FILE_BYTES {
        return Err(Error::runtime(format!("{path} is larger than {} MB", MAX_PNG_FILE_BYTES >> 20)));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_PNG_FILE_BYTES + 1).read_to_end(&mut bytes).map_err(fail)?;
    if !bytes.starts_with(&PNG_SIGNATURE) {
        // Other image formats load through the general reader (the first frame); a file named .png must be a PNG.
        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) {
            return Err(Error::runtime(format!("{path} is not a PNG file")));
        }
        let s = crate::formats::raster::read_any(&bytes, path)
            .map_err(|e| Error::runtime(format!("{path} is not a PNG file or another supported image: {e}")))?;
        let (w, h) = (s.width as usize, s.height as usize);
        return Ok((w, h, s.render(0)));
    }
    crate::formats::import::decode_png(&bytes).map_err(|e| Error::runtime(format!("{path}: {e}")))
}

// MARK: Lua API

fn image_arg(v: &Value, what: &str) -> LuaResult<ImageH> {
    match v {
        Value::UserData(u) => {
            u.borrow::<ImageH>().map(|i| i.clone()).map_err(|_| Error::runtime(format!("{what}: expected an Image")))
        }
        _ => Err(Error::runtime(format!("{what}: expected an Image"))),
    }
}

/// The image or raw RGBA string a bulk pixel call was given, if it is one.
pub(super) fn image_of(v: &Value) -> Option<ImageH> {
    image_arg(v, "").ok()
}

fn mode_value(v: &Value) -> i64 {
    int_of(v).filter(|m| (0..=4).contains(m)).unwrap_or(0)
}

fn image_ctor(ctx: &Rc<Ctx>, mut a: MultiValue) -> LuaResult<ImageH> {
    let bad =
        || Error::runtime("Image: expected (width, height [, colorMode]), {fromFile=path}, an ImageSpec or an Image");
    match a.front() {
        Some(Value::Integer(_) | Value::Number(_)) => {
            let w = a.pop_front().as_ref().and_then(int_of).ok_or_else(bad)?;
            let h = a.pop_front().as_ref().and_then(int_of).ok_or_else(bad)?;
            let mode = a.pop_front().as_ref().map_or(0, mode_value);
            let (w, h) = (usize::try_from(w).map_err(|_| bad())?, usize::try_from(h).map_err(|_| bad())?);
            ImageH::own(ctx, w, h, None, mode)
        }
        Some(Value::Table(t)) => {
            if let Ok(Value::String(path)) = t.raw_get::<Value>("fromFile") {
                let path = path.to_str()?;
                // Reading a file asks first (docs/LUA_HOST_API.md); relative paths are refused below.
                if std::path::Path::new(&*path).is_absolute() {
                    super::access::check(ctx, &path, super::access::Access::Read)?;
                }
                let (w, h, px) = load_png(&path)?;
                return ImageH::own(ctx, w, h, Some(px), 0);
            }
            let named = |k: &str| t.raw_get::<Value>(k).ok().as_ref().and_then(int_of);
            let (Some(w), Some(h)) = (named("width").or_else(|| named("w")), named("height").or_else(|| named("h")))
            else {
                return Err(bad());
            };
            let mode = t.raw_get::<Value>("colorMode").as_ref().map_or(0, mode_value);
            ImageH::own(ctx, usize::try_from(w).map_err(|_| bad())?, usize::try_from(h).map_err(|_| bad())?, None, mode)
        }
        Some(Value::UserData(u)) => {
            if let Ok(src) = u.borrow::<ImageH>() {
                let (w, h) = src.size()?;
                return ImageH::own(ctx, w, h, Some(src.snapshot()?), src.color_mode());
            }
            let (w, h) = size_of(&Value::UserData(u.clone())).ok_or_else(bad)?;
            ImageH::own(ctx, usize::try_from(w).map_err(|_| bad())?, usize::try_from(h).map_err(|_| bad())?, None, 0)
        }
        _ => Err(bad()),
    }
}

pub(super) fn install(lua: &Lua, ctx: &Rc<Ctx>) -> LuaResult<()> {
    let g = lua.globals();
    let c = ctx.clone();
    g.set("Image", lua.create_function(move |_, a: MultiValue| image_ctor(&c, a))?)?;
    g.set(
        "ImageSpec",
        lua.create_function(|lua, t: Option<Table>| {
            let spec = lua.create_table()?;
            spec.set("width", 0)?;
            spec.set("height", 0)?;
            spec.set("colorMode", 0)?;
            spec.set("transparentColor", 0)?;
            if let Some(t) = t {
                for k in ["width", "height", "colorMode", "transparentColor"] {
                    if let Ok(v @ (Value::Integer(_) | Value::Number(_))) = t.raw_get::<Value>(k) {
                        spec.set(k, v)?;
                    }
                }
            }
            Ok(spec)
        })?,
    )?;
    let mode = lua.create_table()?;
    for (name, v) in [("RGB", 0), ("GRAY", 1), ("GRAYSCALE", 1), ("INDEXED", 2), ("TILEMAP", 3)] {
        mode.set(name, v)?;
    }
    g.set("ColorMode", mode)?;
    Ok(())
}

/// Composites `src` onto `dst` with its top-left at (px, py).
#[allow(clippy::too_many_arguments)]
fn composite(
    dst: &mut [u8],
    dw: usize,
    dh: usize,
    src: &[u8],
    sw: usize,
    sh: usize,
    (px, py): (i64, i64),
    opacity: u32,
    mode: u16,
) {
    let x0 = px.max(0);
    let y0 = py.max(0);
    let x1 = px.saturating_add(sw as i64).min(dw as i64);
    let y1 = py.saturating_add(sh as i64).min(dh as i64);
    for y in y0..y1.max(y0) {
        for x in x0..x1.max(x0) {
            let s = (((y - py) as usize) * sw + (x - px) as usize) * 4;
            let d = (y as usize * dw + x as usize) * 4;
            crate::document::layers::blend_pixel(&mut dst[d..d + 4], &src[s..s + 4], opacity, mode);
        }
    }
}

/// Clips a rectangle to `w` x `h`.
fn clip(r: &RectV, w: usize, h: usize) -> (i64, i64, i64, i64) {
    let c = RectV::new(0, 0, w as i64, h as i64).intersection(r);
    (c.x, c.y, c.x + c.w, c.y + c.h)
}

impl UserData for ImageH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("width", |_, t| Ok(t.size()?.0));
        f.add_field_method_get("height", |_, t| Ok(t.size()?.1));
        f.add_field_method_get("bounds", |_, t| {
            let (w, h) = t.size()?;
            Ok(RectV::new(0, 0, w as i64, h as i64))
        });
        f.add_field_method_get("size", |_, t| {
            let (w, h) = t.size()?;
            Ok(SizeV { w: w as i64, h: h as i64 })
        });
        f.add_field_method_get("colorMode", |_, t| Ok(t.color_mode()));
        f.add_field_method_get("rowStride", |_, t| Ok(t.size()?.0 * 4));
        f.add_field_method_get("bytesPerPixel", |_, _| Ok(4));
        f.add_field_method_get("spec", |lua, t| {
            let (w, h) = t.size()?;
            let spec = lua.create_table()?;
            spec.set("width", w)?;
            spec.set("height", h)?;
            spec.set("colorMode", t.color_mode())?;
            spec.set("transparentColor", 0)?;
            Ok(spec)
        });
        f.add_field_method_get("cel", |_, t| {
            let kind = t.0.kind.borrow();
            Ok(match &*kind {
                Kind::View { layer, frame, .. } => Some(CelH(t.0.ctx.clone(), *layer, *frame, t.0.ctx.epoch.get())),
                Kind::Own { .. } => None,
            })
        });
        // Raw RGBA, row-major, one byte per channel; the string has exactly width * height * 4 bytes.
        f.add_field_method_get("bytes", |lua, t| t.read(|p, _, _| lua.create_string(p))?);
        f.add_field_method_set("bytes", |_, t, v: mlua::LuaString| t.replace_all(&v.as_bytes()));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("clone", |_, t, ()| {
            let (w, h) = t.size()?;
            ImageH::own(&t.0.ctx, w, h, Some(t.snapshot()?), t.color_mode())
        });
        // clear(), clear(color) or clear(rectangle): transparent (or `color`) over everything or the rectangle.
        m.add_method("clear", |_, t, mut a: MultiValue| {
            let (color, rect) = match a.front() {
                None | Some(Value::Nil) => ([0u8; 4], None),
                Some(Value::Integer(_) | Value::Number(_)) if a.len() < 4 => {
                    (t.arg(&a.pop_front().unwrap_or(Value::Nil))?, None)
                }
                Some(Value::UserData(u)) if u.borrow::<color::ColorH>().is_ok() => {
                    (t.arg(&a.pop_front().unwrap_or(Value::Nil))?, None)
                }
                Some(_) => {
                    let r = take_rect(&mut a, "clear")?;
                    (a.pop_front().map_or(Ok([0u8; 4]), |c| t.arg(&c))?, Some(r))
                }
            };
            t.modify(|p, w, h| {
                let (x0, y0, x1, y1) = rect.map_or((0, 0, w as i64, h as i64), |r| clip(&r, w, h));
                for y in y0..y1 {
                    for x in x0..x1 {
                        p[(y as usize * w + x as usize) * 4..][..4].copy_from_slice(&color);
                    }
                }
            })
        });
        // Pixel values are Aseprite's (see `color.rs`): RGB images 0xAABBGGRR, grayscale gray | alpha << 8, indexed
        // images the palette index. putPixel / drawPixel / clear / isPlain also take a Color.
        m.add_method("getPixel", |_, t, (x, y): (i64, i64)| Ok(t.value_of(t.get(x, y)?)));
        m.add_method("putPixel", |_, t, (x, y, c): (i64, i64, Value)| t.put(x, y, t.arg(&c)?, false));
        m.add_method("drawPixel", |_, t, (x, y, c): (i64, i64, Value)| t.put(x, y, t.arg(&c)?, true));
        // drawImage(image, position [, opacity [, blendMode]]) or drawImage(image, x, y [, opacity [, blendMode]])
        m.add_method("drawImage", |_, t, mut a: MultiValue| {
            let src = image_arg(&a.pop_front().unwrap_or(Value::Nil), "drawImage")?;
            let at = if a.is_empty() { PointV::default() } else { take_point(&mut a, "drawImage")? };
            let opacity = take_opt_int(&mut a, "drawImage opacity")?.unwrap_or(255).clamp(0, 255) as u32;
            let mode =
                super::commands::blend_from_aseprite(take_opt_int(&mut a, "drawImage blend mode")?.unwrap_or(0))?;
            let (sw, sh) = src.size()?;
            let data = src.snapshot()?;
            t.modify(|p, w, h| composite(p, w, h, &data, sw, sh, (at.x, at.y), opacity, mode))
        });
        // drawSprite(sprite, frame [, position]): the composited frame of the sprite.
        m.add_method("drawSprite", |_, t, mut a: MultiValue| {
            let Some(Value::UserData(u)) = a.pop_front() else {
                return Err(Error::runtime("drawSprite: expected a Sprite"));
            };
            let ctx = u.borrow::<SpriteH>()?.0.clone();
            let frame = match a.pop_front() {
                Some(Value::Integer(i)) => ctx.frame_index(Some(i))?,
                Some(Value::UserData(f)) => match f.borrow::<super::FrameH>() {
                    Ok(f) => f.check()?,
                    Err(_) => return Err(Error::runtime("drawSprite: expected a frame number")),
                },
                _ => ctx.frame.get(),
            };
            let at = if a.is_empty() { PointV::default() } else { take_point(&mut a, "drawSprite")? };
            let s = ctx.sprite();
            let (sw, sh) = (s.width as usize, s.height as usize);
            let data = s.render(frame);
            t.modify(|p, w, h| composite(p, w, h, &data, sw, sh, (at.x, at.y), 255, 0))
        });
        m.add_method("isEqual", |_, t, o: Value| {
            let o = image_arg(&o, "isEqual")?;
            if t.size()? != o.size()? {
                return Ok(false);
            }
            let other = o.snapshot()?;
            t.read(|p, _, _| p == other.as_slice())
        });
        m.add_method("isEmpty", |_, t, ()| t.read(|p, _, _| p.chunks_exact(4).all(|c| c[3] == 0)));
        m.add_method("isPlain", |_, t, c: Value| {
            let c = t.arg(&c)?;
            t.read(|p, _, _| p.chunks_exact(4).all(|px| px == c))
        });
        m.add_method("resize", |_, t, mut a: MultiValue| {
            let (nw, nh) = match a.front() {
                Some(Value::Integer(_) | Value::Number(_)) => {
                    let w = a.pop_front().as_ref().and_then(int_of);
                    let h = a.pop_front().as_ref().and_then(int_of);
                    (w, h)
                }
                Some(v) => size_of(v).map_or((None, None), |(w, h)| (Some(w), Some(h))),
                None => (None, None),
            };
            let (Some(nw), Some(nh)) = (nw, nh) else {
                return Err(Error::runtime("resize: expected (width, height) or a Size"));
            };
            let (nw, nh) = (usize::try_from(nw).unwrap_or(0), usize::try_from(nh).unwrap_or(0));
            let (w, h) = t.size()?;
            let mut kind = t.0.kind.borrow_mut();
            let Kind::Own { w: cw, h: ch, px } = &mut *kind else {
                return Err(Error::runtime(
                    "an image that belongs to a cel cannot be resized; assign a resized clone to cel.image",
                ));
            };
            let old_len = px.len();
            let mut moved = std::mem::take(px);
            t.0.ctx.image_bytes.set(t.0.ctx.image_bytes.get().saturating_sub(old_len));
            let bytes = match reserve(&t.0.ctx, nw, nh) {
                Ok(b) => b,
                Err(e) => {
                    t.0.ctx.image_bytes.set(t.0.ctx.image_bytes.get() + old_len);
                    *px = std::mem::take(&mut moved);
                    return Err(e);
                }
            };
            let mut out = vec![0u8; bytes];
            for y in 0..nh {
                let sy = y * h / nh;
                for x in 0..nw {
                    let sx = x * w / nw;
                    out[(y * nw + x) * 4..][..4].copy_from_slice(&moved[(sy * w + sx) * 4..][..4]);
                }
            }
            *px = out;
            (*cw, *ch) = (nw, nh);
            Ok(())
        });
        // pixels([rectangle]): `for it in image:pixels() do local c = it() it(newColor) end`; `it.x`, `it.y`.
        m.add_method("pixels", |lua, t, mut a: MultiValue| {
            let (w, h) = t.size()?;
            let rect = if a.is_empty() || matches!(a.front(), Some(Value::Nil)) {
                RectV::new(0, 0, w as i64, h as i64)
            } else {
                take_rect(&mut a, "pixels")?
            };
            let (x0, y0, x1, y1) = clip(&rect, w, h);
            let it =
                lua.create_userdata(PixelIt { img: t.clone(), rect: (x0, y0, x1, y1), pos: Cell::new((x0 - 1, y0)) })?;
            lua.create_function(move |_, ()| {
                let st = it.borrow::<PixelIt>()?;
                let (mut x, mut y) = st.pos.get();
                x += 1;
                if x >= st.rect.2 {
                    x = st.rect.0;
                    y += 1;
                }
                if y >= st.rect.3 || st.rect.2 <= st.rect.0 {
                    return Ok(Value::Nil);
                }
                st.pos.set((x, y));
                Ok(Value::UserData(it.clone()))
            })
        });
        // saveAs(path): any image format Pixmith writes (permission: write).
        m.add_method("saveAs", |_, t, path: String| {
            let (w, h) = t.size()?;
            super::sprites::save_image(&t.0.ctx, &path, t.snapshot()?, w, h)
        });
        // flip([FlipType]): in place; FlipType.HORIZONTAL (the default) or FlipType.VERTICAL.
        m.add_method("flip", |_, t, how: Option<i64>| {
            let vertical = match how.unwrap_or(0) {
                0 => false,
                1 => true,
                other => return Err(Error::runtime(format!("flip: {other} is not a FlipType"))),
            };
            t.modify(|p, w, h| {
                if vertical {
                    for y in 0..h / 2 {
                        let (top, bottom) = (y * w * 4, (h - 1 - y) * w * 4);
                        for i in 0..w * 4 {
                            p.swap(top + i, bottom + i);
                        }
                    }
                } else {
                    for row in p.chunks_exact_mut(w * 4) {
                        for x in 0..w / 2 {
                            for k in 0..4 {
                                row.swap(x * 4 + k, (w - 1 - x) * 4 + k);
                            }
                        }
                    }
                }
            })
        });
        // shrinkBounds([refColor]) -> Rectangle: the box around everything that is not the reference color
        // (default: fully transparent pixels; with a color: pixels different from it).
        m.add_method("shrinkBounds", |_, t, c: Option<Value>| {
            let reference = c.filter(|v| !matches!(v, Value::Nil)).map(|v| t.arg(&v)).transpose()?;
            let skip = |px: &[u8]| match reference {
                Some(r) => px == r,
                None => px[3] == 0,
            };
            t.read(|p, w, h| {
                let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
                for (y, row) in p.chunks_exact(w * 4).enumerate() {
                    if let Some(first) = row.chunks_exact(4).position(|px| !skip(px)) {
                        let last = row.chunks_exact(4).rposition(|px| !skip(px)).unwrap_or(first);
                        x0 = x0.min(first);
                        x1 = x1.max(last + 1);
                        y0 = y0.min(y);
                        y1 = y1.max(y + 1);
                    }
                }
                if x1 > x0 && y1 > y0 {
                    RectV::new(x0 as i64, y0 as i64, (x1 - x0) as i64, (y1 - y0) as i64)
                } else {
                    RectV::default()
                }
            })
        });
    }
}

/// The iterator object `image:pixels()` hands out (one object, advanced in place, as in Aseprite).
struct PixelIt {
    img: ImageH,
    /// x0, y0, x1, y1 (exclusive)
    rect: (i64, i64, i64, i64),
    pos: Cell<(i64, i64)>,
}

impl UserData for PixelIt {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("x", |_, t| Ok(t.pos.get().0));
        f.add_field_method_get("y", |_, t| Ok(t.pos.get().1));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(mlua::MetaMethod::Call, |_, t, v: Option<Value>| {
            let (x, y) = t.pos.get();
            match v {
                None => Ok(Some(t.img.value_of(t.img.get(x, y)?))),
                Some(c) => {
                    t.img.put(x, y, t.img.arg(&c)?, false)?;
                    Ok(None)
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::document::color_mode::ColorMode;
    use crate::document::{Editor, Sprite};
    use crate::scripting::{run, run_with_limits, Limits};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn editor(w: u32, h: u32) -> Editor {
        Editor::new(Sprite::new(w, h))
    }

    fn ok(e: &mut Editor, src: &str) {
        let o = run(e, src, (0, 0), Limits::default());
        assert_eq!(o.error, None, "script failed:\n{src}");
    }

    fn fails(e: &mut Editor, src: &str, needle: &str) {
        let o = run(e, src, (0, 0), Limits::default());
        let err = o.error.unwrap_or_else(|| panic!("script should have failed:\n{src}"));
        assert!(err.contains(needle), "error {err:?} does not contain {needle:?}");
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let dir = std::env::temp_dir().join(format!("pixmith-lua-image-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            // These tests read files of their own directory: allow it on this thread.
            let prefix = dir.clone();
            crate::scripting::access::set_thread_permission_handler(Some(std::sync::Arc::new(move |r| {
                if std::path::Path::new(&r.path).starts_with(&prefix) {
                    crate::scripting::access::Permission::AllowOnce
                } else {
                    crate::scripting::access::Permission::Deny
                }
            })));
            TempDir(dir)
        }

        /// Writes a w x h PNG filled with `px` and returns its path as a Lua string literal.
        fn png(&self, name: &str, w: usize, h: usize, px: [u8; 4]) -> String {
            let rgba: Vec<u8> = px.iter().copied().cycle().take(w * h * 4).collect();
            let path = self.0.join(name);
            std::fs::write(&path, crate::formats::export::encode_png(&rgba, w, h).unwrap()).unwrap();
            format!("{:?}", path.to_string_lossy())
        }

        fn lua_path(&self, name: &str) -> String {
            format!("{:?}", self.0.join(name).to_string_lossy())
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn images_have_pixels_size_and_clone() {
        let mut e = editor(8, 8);
        ok(
            &mut e,
            r#"
            local img = Image(4, 3)
            assert(img.width == 4 and img.height == 3 and img.bounds == Rectangle(0, 0, 4, 3))
            assert(img.colorMode == ColorMode.RGB and img.rowStride == 16 and img.bytesPerPixel == 4)
            assert(img:isEmpty() and img:getPixel(0, 0) == 0 and img:getPixel(99, 0) == 0)
            img:putPixel(1, 2, 0xFF0000FF)
            img:drawPixel(0, 0, 0x8000FF00)                  -- Aseprite pixel values: 0xAABBGGRR
            assert(img:getPixel(1, 2) == 0xFF0000FF and img:getPixel(0, 0) == 0x8000FF00)
            img:putPixel(2, 2, Color(1, 2, 3))               -- a Color works too
            assert(img:getPixel(2, 2) == app.pixelColor.rgba(1, 2, 3, 255))
            img:putPixel(2, 2, app.pixelColor.rgba(9, 8, 7, 6))
            local px = img:getPixel(2, 2)
            assert(app.pixelColor.rgbaR(px) == 9 and app.pixelColor.rgbaG(px) == 8)
            assert(app.pixelColor.rgbaB(px) == 7 and app.pixelColor.rgbaA(px) == 6)
            img:putPixel(2, 2, 0)
            img:putPixel(-1, 0, 0xFFFFFFFF)                  -- outside: ignored
            local c = img:clone()
            c:putPixel(1, 2, 0)
            assert(img:getPixel(1, 2) == 0xFF0000FF and c:getPixel(1, 2) == 0)
            assert(img:isEqual(img:clone()) and not img:isEqual(c))
            assert(Image(img):isEqual(img))
            assert(Image(Size(5, 6)).width == 5 and Image(ImageSpec{ width = 2, height = 7 }).height == 7)
            assert(Image{ width = 3, height = 3, colorMode = ColorMode.INDEXED }.colorMode == ColorMode.INDEXED)
            assert(img.spec.width == 4 and img.spec.height == 3)
            local plain = Image(2, 2)
            plain:clear(0xFF332211)
            assert(plain:isPlain(0xFF332211) and plain:isPlain(Color(0x11, 0x22, 0x33)))
            plain:clear(Rectangle(0, 0, 1, 1))
            assert(plain:getPixel(0, 0) == 0 and plain:getPixel(1, 1) == 0xFF332211)
            plain:clear()
            assert(plain:isEmpty())
            assert(not pcall(Image, 0, 5) and not pcall(Image) and not pcall(Image, "x"))
            assert(not pcall(Image, 1 << 20, 1 << 20))
            local ok, err = pcall(function() img:saveAs("/tmp/pixmith-never-written.png") end)
            assert(not ok and err:find("permission denied"), err)
        "#,
        );
    }

    #[test]
    fn pixels_iterator_reads_and_writes() {
        let mut e = editor(4, 4);
        ok(
            &mut e,
            r#"
            local img = Image(3, 2)
            local n = 0
            for it in img:pixels() do
              it(0x01020304 + it.x * 256 + it.y)
              n = n + 1
            end
            assert(n == 6)
            assert(img:getPixel(2, 1) == 0x01020304 + 2 * 256 + 1)
            local seen = 0
            for it in img:pixels(Rectangle(1, 0, 2, 1)) do
              assert(it.y == 0 and it.x >= 1)
              assert(it() == img:getPixel(it.x, it.y))
              seen = seen + 1
            end
            assert(seen == 2)
            for it in img:pixels(Rectangle(10, 10, 1, 1)) do error("clipped away") end
        "#,
        );
    }

    #[test]
    fn bytes_get_and_set_are_checked_raw_rgba() {
        let mut e = editor(4, 4);
        ok(
            &mut e,
            r#"
            local img = Image(2, 2)
            img.bytes = "\xff\x00\x00\xff" .. "\x00\xff\x00\xff" .. "\x00\x00\xff\xff" .. "\x01\x02\x03\x04"
            assert(img:getPixel(0, 0) == 0xFF0000FF and img:getPixel(1, 0) == 0xFF00FF00)
            assert(img:getPixel(0, 1) == 0xFFFF0000 and img:getPixel(1, 1) == 0x04030201)
            assert(#img.bytes == 16 and img.bytes:byte(13) == 1 and img.bytes:byte(16) == 4)
            local ok, err = pcall(function() img.bytes = "short" end)
            assert(not ok and err:find("needs 16 bytes"), err)
            assert(not pcall(function() img.bytes = ("x"):rep(17) end))
            assert(not pcall(function() img.bytes = 5 end))
            -- round trip
            local other = Image(2, 2)
            other.bytes = img.bytes
            assert(other:isEqual(img))
        "#,
        );
    }

    #[test]
    fn draw_image_composites_with_position_opacity_and_clipping() {
        let mut e = editor(4, 4);
        ok(
            &mut e,
            r#"
            local dst = Image(4, 4)
            local src = Image(2, 2)
            src:clear(0xFF0000FF)
            dst:drawImage(src, Point(1, 1))
            assert(dst:getPixel(1, 1) == 0xFF0000FF and dst:getPixel(2, 2) == 0xFF0000FF)
            assert(dst:getPixel(0, 0) == 0 and dst:getPixel(3, 3) == 0)
            dst:drawImage(src, 3, 3)                          -- x, y form, clipped
            assert(dst:getPixel(3, 3) == 0xFF0000FF)
            dst:drawImage(src, Point(-1, -1))                 -- clipped on the other side
            assert(dst:getPixel(0, 0) == 0xFF0000FF)
            local half = Image(1, 1)
            half:drawImage(src, Point(0, 0), 128)
            local c = half:getPixel(0, 0)
            assert(app.pixelColor.rgbaR(c) == 255 and app.pixelColor.rgbaA(c) == 128, c)
            -- drawing over opaque pixels blends
            local base = Image(1, 1) base:clear(0xFFFF0000)
            local top = Image(1, 1) top:clear(0x800000FF)
            base:drawImage(top)
            local b = base:getPixel(0, 0)
            local P = app.pixelColor
            assert(P.rgbaA(b) == 255 and P.rgbaR(b) > 100 and P.rgbaB(b) > 100, b)
            dst:drawImage(dst, Point(0, 0))                   -- onto itself is fine
            assert(not pcall(function() dst:drawImage("nope", Point(0, 0)) end))
        "#,
        );
    }

    #[test]
    fn resize_scales_images_with_nearest_neighbour() {
        let mut e = editor(4, 4);
        ok(
            &mut e,
            r#"
            local img = Image(2, 1)
            img:putPixel(0, 0, 0xFF0000FF) img:putPixel(1, 0, 0xFF00FF00)
            img:resize(4, 2)
            assert(img.width == 4 and img.height == 2)
            assert(img:getPixel(0, 1) == 0xFF0000FF and img:getPixel(1, 0) == 0xFF0000FF)
            assert(img:getPixel(2, 0) == 0xFF00FF00 and img:getPixel(3, 1) == 0xFF00FF00)
            img:resize(Size(1, 1))
            assert(img.width == 1 and img:getPixel(0, 0) == 0xFF0000FF)
            assert(not pcall(function() img:resize(0, 1) end))
            assert(img.width == 1)                             -- unchanged after the failed call
        "#,
        );
    }

    #[test]
    fn new_cel_places_an_image_and_cels_expose_position_bounds_and_image() {
        let mut e = editor(8, 8);
        ok(
            &mut e,
            r#"
            local src = Image(3, 2)
            src:clear(0xFF0000FF)
            local layer = sprite:newLayer("art")
            local cel = sprite:newCel(layer, 1, src, Point(2, 3))
            assert(cel.position == Point(2, 3) and cel.bounds == Rectangle(2, 3, 3, 2))
            assert(sprite:getPixel(2, 3, 2, 1) == 0xFF0000FF and sprite:getPixel(4, 4, 2, 1) == 0xFF0000FF)
            assert(sprite:getPixel(1, 3, 2, 1) == 0 and sprite:getPixel(5, 3, 2, 1) == 0)
            assert(layer:cel(1) ~= nil and layer:cel(1).position == Point(2, 3))
            assert(layer:cel(1).frameNumber == 1 and #layer.cels == 1 and #sprite.cels >= 1)

            local img = cel.image
            assert(img.width == 3 and img.height == 2 and img:getPixel(0, 0) == 0xFF0000FF)
            assert(img.cel.layer.stackIndex == 2 and img.cel.frame.frameNumber == 1 and img.cel.frameNumber == 1)
            -- the image is a live view: drawing on it draws on the cel
            img:putPixel(1, 1, 0xFF00FF00)
            assert(sprite:getPixel(3, 4, 2, 1) == 0x00FF00FF)
            sprite:setPixel(2, 3, 0x0000FFFF, 2, 1)
            assert(img:getPixel(0, 0) == 0xFFFF0000)
            img:drawImage(src, Point(0, 0))
            assert(sprite:getPixel(2, 3, 2, 1) == 0xFF0000FF)

            -- replace the cel's image
            local big = Image(2, 2) big:clear(0xFFFFFF00)
            cel.image = big
            assert(cel.bounds == Rectangle(2, 3, 2, 2) and sprite:getPixel(4, 3, 2, 1) == 0)
            assert(sprite:getPixel(2, 3, 2, 1) == 0x00FFFFFF)

            -- move the cel
            cel.position = Point(0, 0)
            assert(cel.bounds == Rectangle(0, 0, 2, 2) and sprite:getPixel(2, 3, 2, 1) == 0)
            cel.position = Point(7, 7)                          -- sticks out of the canvas, nothing is lost
            assert(cel.bounds == Rectangle(7, 7, 2, 2))
            cel.position = Point(0, 0)
            assert(cel.bounds == Rectangle(0, 0, 2, 2) and sprite:getPixel(0, 0, 2, 1) == 0x00FFFFFF)

            -- frames, handles, replacing and errors
            sprite:newFrame()
            local c2 = sprite:newCel(layer, sprite.frames[2], src)
            assert(c2.frame.frameNumber == 2 and c2.frame == sprite.frames[2] and c2.position == Point(0, 0))
            sprite:newCel(1, 2, src, Point(100, 100))                         -- entirely beyond the canvas: kept
            assert(not sprite:cel(1, 2).isEmpty and sprite:cel(1, 2).bounds == Rectangle(100, 100, 3, 2))
            assert(sprite:cel(1, 2).image.width == 3)
            sprite:newCel(1, 2)
            assert(sprite:cel(1, 2).isEmpty and sprite:cel(1, 2).image == nil)
            assert(layer:cel(1).opacity == 255)
            assert(not pcall(sprite.newCel, sprite, 99, 1, src))
            assert(not pcall(sprite.newCel, sprite, layer, 1, "not an image"))
            assert(not pcall(sprite.newCel, sprite, layer, 1, src, 5))
            sprite:deleteCel(layer, 2)
            assert(layer:cel(2) == nil)
            sprite:deleteCel(layer:cel(1))
            assert(layer:cel(1) == nil)
            assert(sprite:cel(2, 1).image == nil)
        "#,
        );
        assert_eq!(e.sprite.cels[0][1], None);
    }

    #[test]
    fn new_cel_without_an_image_makes_an_empty_cel_and_respects_groups() {
        let mut e = editor(4, 4);
        ok(
            &mut e,
            r#"
            local layer = sprite:newLayer("a")
            local cel = sprite:newCel(layer, 1)
            assert(cel.isEmpty and cel.bounds.isEmpty and cel.position == Point(0, 0) and cel.image == nil)
            local group = sprite:newGroup("g")
            assert(not pcall(sprite.newCel, sprite, group, 1, Image(1, 1)))
            cel.position = Point(1, 1)                           -- empty cel: nothing to move
            local img = Image(1, 1) img:clear(0xFFFFFFFF)
            cel.image = img                                       -- an empty cel takes the image at 0,0
            assert(cel.bounds == Rectangle(0, 0, 1, 1))
        "#,
        );
    }

    #[test]
    fn set_pixels_takes_images_and_raw_bytes_and_rectangles() {
        let mut e = editor(4, 4);
        ok(
            &mut e,
            r#"
            local bytes = ("\xff\x00\x00\xff"):rep(16)
            sprite:setPixels(1, 1, bytes)
            assert(sprite:getPixel(0, 0) == 0xFF0000FF and sprite:getPixel(3, 3) == 0xFF0000FF)
            local img = Image(2, 2) img:clear(0xFF00FF00)
            sprite:setPixels(1, 1, img, 1, 1)                     -- at (1, 1), the image's size
            assert(sprite:getPixel(1, 1) == 0x00FF00FF and sprite:getPixel(2, 2) == 0x00FF00FF)
            assert(sprite:getPixel(0, 0) == 0xFF0000FF and sprite:getPixel(3, 3) == 0xFF0000FF)
            sprite:setPixels(1, 1, ("\x01\x02\x03\xff"):rep(4), Rectangle(2, 0, 2, 2))
            assert(sprite:getPixel(3, 1) == 0x010203FF and sprite:getPixel(1, 0) == 0xFF0000FF)
            sprite:setPixels(sprite.layers[1], sprite.frames[1], { 0xAABBCCDD }, Rectangle(0, 3, 1, 1))
            assert(sprite:getPixel(0, 3) == 0xAABBCCDD)
            assert(sprite:getPixel(Point(0, 3)) == 0xAABBCCDD)
            local px = sprite:getPixels(1, 1, Rectangle(0, 3, 2, 1))
            assert(#px == 2 and px[1] == 0xAABBCCDD)
            sprite:setPixel(Point(3, 3), 0x11223344)
            assert(sprite:getPixel(3, 3) == 0x11223344)
            assert(not pcall(sprite.setPixels, sprite, 1, 1, "short"))
            assert(not pcall(sprite.setPixels, sprite, 1, 1, img, 3, 3))      -- does not fit the canvas
            assert(not pcall(sprite.setPixels, sprite, 1, 1, Image(2, 2), 0, 0, 3, 3))
        "#,
        );
    }

    #[test]
    fn image_from_file_loads_png_by_absolute_path_only() {
        let dir = TempDir::new("fromfile");
        let png = dir.png("art.png", 5, 4, [10, 20, 30, 255]);
        let mut e = editor(8, 8);
        ok(
            &mut e,
            &format!(
                r#"
            local img = Image{{ fromFile = {png} }}
            assert(img.width == 5 and img.height == 4)
            assert(img:getPixel(0, 0) == 0xFF1E140A and img:getPixel(4, 3) == 0xFF1E140A)
            local cel = sprite:newCel(sprite.layers[1], 1, img, Point(1, 1))
            assert(cel.bounds == Rectangle(1, 1, 5, 4))
            sprite:setPixels(1, 1, Image{{ fromFile = {png} }}, 0, 0)
        "#
            ),
        );
        fails(&mut e, "Image{ fromFile = 'relative.png' }", "absolute path");
        fails(&mut e, &format!("Image{{ fromFile = {} }}", dir.lua_path("missing.png")), "cannot read");
        fails(&mut e, &format!("Image{{ fromFile = {} }}", dir.lua_path("")), "not a file");
        std::fs::write(dir.0.join("fake.png"), b"not a png at all").unwrap();
        fails(&mut e, &format!("Image{{ fromFile = {} }}", dir.lua_path("fake.png")), "not a PNG");
        // a PNG signature followed by garbage fails in the decoder, not in the script host
        let mut bad = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bad.extend_from_slice(&[0; 40]);
        std::fs::write(dir.0.join("bad.png"), bad).unwrap();
        fails(&mut e, &format!("Image{{ fromFile = {} }}", dir.lua_path("bad.png")), "bad.png");
    }

    #[test]
    fn big_images_cost_few_instructions_and_little_time() {
        let dir = TempDir::new("big");
        let png = dir.png("big.png", 1024, 1024, [200, 100, 50, 255]);
        let mut e = editor(1024, 1024);
        // A tiny instruction budget (the hook counts in steps of 1000): the work must happen in Rust.
        let tiny = Limits::default().with_instructions(20_000);

        let t = Instant::now();
        let o = run(
            &mut e,
            r#"
            local px = ("\x01\x02\x03\xff"):rep(1024 * 1024)
            local img = Image(1024, 1024)
            img.bytes = px
            sprite:newCel(sprite.layers[1], 1, img, Point(0, 0))
            assert(sprite:getPixel(1023, 1023) == 0x010203FF and #img.bytes == 4 * 1024 * 1024)
            local layer = sprite:newLayer("two")
            sprite:newCel(layer, 1, img)
            sprite:setPixels(1, 1, px)
            sprite:setPixels(1, 1, img)
        "#,
            (0, 0),
            tiny,
        );
        assert_eq!(o.error, None);
        let from_bytes = t.elapsed();

        let t = Instant::now();
        let o = run(
            &mut e,
            &format!(
                r#"
            local img = Image{{ fromFile = {png} }}
            sprite:newCel(sprite.layers[1], 1, img, Point(0, 0))
            assert(sprite:getPixel(10, 10) == 0xC86432FF)
        "#
            ),
            (0, 0),
            tiny,
        );
        assert_eq!(o.error, None);
        let from_png = t.elapsed();
        eprintln!("1024x1024: bytes+newCel+setPixels {from_bytes:?}, fromFile+newCel {from_png:?} (debug build)");
        assert!(from_bytes < Duration::from_secs(2), "{from_bytes:?}");
        assert!(from_png < Duration::from_secs(4), "{from_png:?}");
    }

    #[test]
    fn images_obey_size_and_memory_caps() {
        let mut e = editor(4, 4);
        // 8192 x 8192 images are fine one at a time but not all at once
        let o = run(
            &mut e,
            r#"
            local keep = {}
            local ok, err = pcall(function()
              for i = 1, 10 do keep[i] = Image(8192, 8192) end
            end)
            assert(not ok and err:find("at most 512 MB"), tostring(err))
            keep = nil
            collectgarbage() collectgarbage()
            local again = Image(8192, 8192)              -- the budget is released with the images
            assert(again.width == 8192)
            assert(not pcall(Image, 8193, 8193))          -- beyond the largest canvas
        "#,
            (0, 0),
            Limits::default(),
        );
        assert_eq!(o.error, None);
    }

    #[test]
    fn images_written_to_indexed_sprites_snap_to_the_palette() {
        let mut s = Sprite::new(4, 4);
        s.palette = vec![[0, 0, 0, 255], [255, 255, 255, 255]];
        s.color_mode = ColorMode::Indexed;
        let mut e = Editor::new(s);
        ok(
            &mut e,
            r#"
            local img = Image(1, 1) img:clear(0xFFFEFEFE)
            sprite:newCel(sprite.layers[1], 1, img)
            assert(sprite:getPixel(0, 0) == 0xFFFFFFFF, sprite:getPixel(0, 0))
        "#,
        );
    }

    #[test]
    fn the_embedding_entry_point_takes_limits() {
        let mut e = editor(4, 4);
        let src = "local n = 0 for i = 1, 3000000 do n = n + 1 end";
        let o = run_with_limits(&mut e, src, Limits::default().with_instructions(100_000));
        assert!(o.error.as_deref().is_some_and(|m| m.contains("instruction limit")), "{:?}", o.error);
        let o = run_with_limits(&mut e, src, Limits::default());
        assert_eq!(o.error, None);
        let o =
            run_with_limits(&mut e, "while true do end", Limits::new(u64::MAX, Duration::from_millis(100), 16 << 20));
        assert!(o.error.as_deref().is_some_and(|m| m.contains("time limit")), "{:?}", o.error);
        let l = Limits::default().with_time(Duration::from_secs(120)).with_memory(1 << 30);
        assert_eq!((l.instructions, l.time, l.memory), (300_000_000, Duration::from_secs(120), 1 << 30));
    }

    /// A script written for Aseprite, run unchanged: Rectangle / Point / newSlice / Image{ fromFile } / newCel.
    #[test]
    fn aseprite_style_script_runs_unchanged() {
        let dir = TempDir::new("aseprite");
        let png = dir.png("hero.png", 6, 6, [255, 128, 0, 255]);
        let mut e = editor(32, 32);
        ok(
            &mut e,
            &format!(
                r#"
            local spr = app.sprite
            local path = {png}
            local img = Image{{ fromFile = path }}
            assert(img.width == 6 and img.height == 6)

            -- one cel per frame, placed with a Point
            local layer = spr:newLayer()
            layer.name = "hero"
            local cel = spr:newCel(layer, spr.frames[1], img, Point(4, 5))
            assert(cel.position == Point(4, 5))
            assert(cel.bounds == Rectangle(4, 5, 6, 6))
            assert(cel.bounds:contains(Point(5, 6)) and not cel.bounds:contains(Point(10, 5)))

            -- slices from rectangles, with 9-patch center and pivot
            local slice = spr:newSlice(Rectangle(4, 5, 6, 6))
            slice.name = "hitbox"
            slice.center = Rectangle(1, 1, 4, 4)
            slice.pivot = Point(3, 3)
            assert(slice.bounds == Rectangle(4, 5, 6, 6))
            assert(slice.center == Rectangle(1, 1, 4, 4) and slice.pivot == Point(3, 3))
            local inner = slice.bounds:intersect(Rectangle(0, 0, 8, 8))
            assert(inner == Rectangle(4, 5, 4, 3), tostring(inner))
            assert(spr.bounds == Rectangle(0, 0, 32, 32) and spr.bounds:contains(slice.bounds))

            -- selection with rectangles
            local sel = spr.selection
            sel:select(Rectangle(2, 2, 10, 10))
            sel:subtract(Rectangle(2, 2, 5, 5))
            sel:add(Rectangle(20, 20, 2, 2))
            assert(sel.bounds == Rectangle(2, 2, 20, 20))
            assert(sel:contains(Point(21, 21)) and not sel:contains(Point(3, 3)))
            sel:select(Rectangle(0, 0, 4, 4))
            sel:intersect(Rectangle(2, 2, 4, 4))
            assert(sel.bounds == Rectangle(2, 2, 2, 2))
            sel.origin = Point(10, 10)
            assert(sel.bounds == Rectangle(10, 10, 2, 2) and sel.origin == Point(10, 10))
            sel:deselect()

            -- draw an image into another
            local canvas = Image(spr.width, spr.height)
            canvas:drawImage(cel.image, cel.position)
            assert(canvas:getPixel(4, 5) == 0xFF0080FF and canvas:getPixel(3, 5) == 0)
            print(spr.width .. "x" .. spr.height, tostring(cel.bounds))
        "#
            ),
        );
        let s = &e.sprite;
        assert_eq!(s.extra.slices.len(), 1);
        assert_eq!(s.extra.slices[0].name, "hitbox");
        assert!(s.extra.slices[0].nine_patch && s.extra.slices[0].has_pivot);
        assert_eq!(
            s.cels[0][1].as_ref().map(|c| c.pixels[((5 * 32 + 4) * 4)..][..4].to_vec()),
            Some(vec![255, 128, 0, 255])
        );
    }
}
