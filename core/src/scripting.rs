//! Embedded Lua scripting.
//!
//! A script sees two globals, `sprite` and `app`, plus a small `color` helper table. The full
//! reference is in `docs/LUA_API.md`; this is the summary.
//!
//! Conventions: pixel coordinates and palette indices are 0-based; layers, frames, tags and
//! lists are 1-based (as usual in Lua). Colors are integers `0xRRGGBBAA`. Handles
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
//! sprite:setPixels(layer, frame, pixels [, x, y, w, h])   -- same layout; at most 4194304 pixels
//! sprite:fillSelection(color [, layer [, frame]]) -> count
//!
//! -- Layers
//! sprite.layers[i] -> { name, visible, opacity (0..255), blendMode (0..18), isGroup, isTilemap,
//!                       index, level, parent, children }
//! (also: isVisible (effective, with parents), tileset, userData, getTile / setTile on tilemap layers)
//! sprite:newLayer(name), sprite:newGroup(name), sprite:deleteLayer(i), sprite:moveLayer(i, to) -> index
//! sprite:moveLayerIntoGroup(i, group) -> index, sprite:moveLayerOutOfGroup(i) -> index
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
//! sprite:newSlice(name, x, y, w, h), sprite:deleteSlice(i)
//!
//! -- User data: { text, color } on sprite.userData, layer.userData, cel.userData, tag.userData, slice.userData
//!
//! -- Cels
//! sprite:cel(layer, frame) -> { opacity, isEmpty, linked, linkedFrames, unlink(), userData }
//! sprite:clearCel(layer, frame)
//!
//! -- Selection (scripts only look at it through these calls)
//! sprite.selection -> { isEmpty, bounds, contains(x, y), selectRect(x, y, w, h [, mode]),
//!                       selectAll(), deselect(), invert() }
//!
//! -- Palette
//! sprite.paletteSize, sprite:getPaletteColor(i), sprite:setPaletteColor(i, color), sprite:addPaletteColor(color)
//!
//! app.alert(msg), app.undoLabel(text), app.blend.Multiply ..., app.direction.PingPong ..., print(...)
//! app.version, app.apiVersion, app.activeLayer / app.activeFrame (read, or assign to select them)
//! color.rgba(r, g, b [, a]), color.split(c) -> r, g, b, a
//! ```
//!
//! The whole run is one undo step (sprite and selection), and a script that fails leaves the
//! document untouched. In Indexed and Grayscale sprites every cel the script changed is snapped
//! to the color mode when the run ends. The sandbox has only the base, table, string, utf8, math
//! and coroutine libraries (no `os`, `io`, `package`, `debug`, `dofile`, `loadfile`; `load`
//! accepts text only). An instruction budget, a wall-clock limit and a memory limit stop runaway
//! scripts. A single pathological string pattern match is one VM instruction and cannot be
//! interrupted.

use crate::document::color_mode;
use crate::document::{Editor, Sprite};
use crate::paint::selection::Selection;
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
}

impl Ctx {
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
    let (cw, ch) = (s.width as i64, s.height as i64);
    let (x, y) = (r.0.unwrap_or(0), r.1.unwrap_or(0));
    let (w, h) = (r.2.unwrap_or(cw - x), r.3.unwrap_or(ch - y));
    if w < 0 || h < 0 || x < 0 || y < 0 || x + w > cw || y + h > ch {
        return Err(Error::runtime(format!("rectangle {x},{y} {w}x{h} does not fit the {cw}x{ch} canvas")));
    }
    if (w as usize).saturating_mul(h as usize) > MAX_BULK_PIXELS {
        return Err(Error::runtime(format!("at most {MAX_BULK_PIXELS} pixels per call")));
    }
    Ok((x as usize, y as usize, w as usize, h as usize))
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
        f.add_field_method_get("colorMode", |_, t| {
            Ok(match t.0.sprite().color_mode {
                color_mode::ColorMode::Rgba => "rgba",
                color_mode::ColorMode::Grayscale => "grayscale",
                color_mode::ColorMode::Indexed => "indexed",
            })
        });
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
        f.add_field_method_get("tilesets", |lua, t| {
            let n = t.0.sprite().tilesets.len();
            lua.create_sequence_from((0..n).map(|i| TilesetH(t.0.clone(), i)))
        });
        f.add_field_method_get("slices", |lua, t| {
            let n = t.0.sprite().extra.slices.len();
            lua.create_sequence_from((0..n).map(|i| SliceH::new(&t.0, i)))
        });
        f.add_field_method_get("userData", |_, t| Ok(UserDataH::new(&t.0, 0, 0, 0)));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        // MARK: Pixels
        m.add_method("getPixel", |_, t, (x, y, layer, frame): (i64, i64, Option<i64>, Option<i64>)| {
            let (l, f) = (t.0.layer_index(layer)?, t.0.frame_index(frame)?);
            let s = t.0.sprite();
            if x < 0 || y < 0 || x >= s.width as i64 || y >= s.height as i64 {
                return Ok(0i64);
            }
            let i = (y as usize * s.width as usize + x as usize) * 4;
            Ok(s.cels[f][l].as_ref().map_or(0, |c| pack(&c.pixels[i..i + 4])))
        });
        m.add_method("setPixel", |_, t, (x, y, color, layer, frame): (i64, i64, i64, Option<i64>, Option<i64>)| {
            let (l, f) = (t.0.layer_index(layer)?, t.0.frame_index(frame)?);
            let c = color_arg(color)?;
            let s = t.0.sprite();
            if s.layers[l].is_group() {
                return Err(Error::runtime("cannot draw on a group layer"));
            }
            if x < 0 || y < 0 || x >= s.width as i64 || y >= s.height as i64 {
                return Ok(());
            }
            let c = color_mode::snap_color(s, c);
            s.draw_line(l, f, x as i32, y as i32, x as i32, y as i32, c, 1);
            s.flush_links(); // an edit shows in every frame that shares the cel right away
            Ok(())
        });
        m.add_method(
            "getPixels",
            |lua, t, (layer, frame, x, y, w, h): (i64, i64, Option<i64>, Option<i64>, Option<i64>, Option<i64>)| {
                let (l, f) = (t.0.layer_index(Some(layer))?, t.0.frame_index(Some(frame))?);
                let s = t.0.sprite();
                let (x0, y0, w, h) = rect_args(s, (x, y, w, h))?;
                let cw = s.width as usize;
                let cel = s.cels[f][l].as_ref();
                lua.create_sequence_from((0..w * h).map(|i| {
                    cel.map_or(0, |c| {
                        let (px, py) = (x0 + i % w, y0 + i / w);
                        pack(&c.pixels[(py * cw + px) * 4..][..4])
                    })
                }))
            },
        );
        m.add_method(
            "setPixels",
            |_,
             t,
             (layer, frame, pixels, x, y, w, h): (
                i64,
                i64,
                Table,
                Option<i64>,
                Option<i64>,
                Option<i64>,
                Option<i64>,
            )| {
                let l = t.0.drawable_layer(Some(layer))?;
                let f = t.0.frame_index(Some(frame))?;
                let s = t.0.sprite();
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
                for v in pixels.sequence_values::<i64>() {
                    colors.push(color_arg(v?)?);
                }
                write_rect(s, l, f, rect, &mut colors.into_iter());
                Ok(())
            },
        );
        m.add_method("fillSelection", |_, t, (color, layer, frame): (i64, Option<i64>, Option<i64>)| {
            let c = color_arg(color)?;
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
        m.add_method("deleteLayer", |_, t, i: i64| {
            let l = t.0.layer_index(Some(i))?;
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
        m.add_method("newFrame", |_, t, copy: Option<bool>| {
            let s = t.0.sprite();
            if s.frame_count() >= MAX_SCRIPT_FRAMES || !s.cels_fit_budget(s.width as u64, s.height as u64) {
                return Err(Error::runtime(format!(
                    "too many frames (at most {MAX_SCRIPT_FRAMES}) or not enough memory"
                )));
            }
            let last = s.frame_count() - 1;
            let i = s.add_frame(last, copy.unwrap_or(false));
            Ok(FrameH::new(&t.0, i))
        });
        m.add_method("duplicateFrame", |_, t, i: i64| {
            let f = t.0.frame_index(Some(i))?;
            let n = t.0.sprite().add_frame(f, true);
            t.0.invalidate_handles();
            Ok(FrameH::new(&t.0, n))
        });
        m.add_method("deleteFrame", |_, t, i: i64| {
            let f = t.0.frame_index(Some(i))?;
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
        m.add_method("newSlice", |_, t, (name, x, y, w, h): (String, i64, i64, i64, i64)| {
            let r = slice_rect(x, y, w, h)?;
            let s = t.0.sprite();
            if s.extra.slices.len() >= MAX_SCRIPT_SLICES {
                return Err(Error::runtime(format!("too many slices (at most {MAX_SCRIPT_SLICES})")));
            }
            let key = crate::document::slices::SliceKey { bounds: r, ..Default::default() };
            s.extra.slices.push(crate::document::slices::Slice { name, keys: vec![key], ..Default::default() });
            Ok(SliceH::new(&t.0, s.extra.slices.len() - 1))
        });
        m.add_method("deleteSlice", |_, t, i: i64| {
            let n = t.0.sprite().extra.slices.len();
            let i = usize::try_from(i - 1)
                .ok()
                .filter(|&i| i < n)
                .ok_or_else(|| Error::runtime(format!("slice {i} does not exist (1..{n})")))?;
            t.0.sprite().extra.slices.remove(i);
            t.0.invalidate_handles();
            Ok(())
        });
        m.add_method("deleteTag", |_, t, i: i64| {
            let i = t.0.tag_index(i)?;
            t.0.sprite().remove_tag(i);
            t.0.invalidate_handles();
            Ok(())
        });

        // MARK: Cels
        m.add_method("cel", |_, t, (layer, frame): (i64, i64)| {
            let (l, f) = (t.0.layer_index(Some(layer))?, t.0.frame_index(Some(frame))?);
            if t.0.sprite().layers[l].is_group() {
                return Err(Error::runtime("groups have no cels"));
            }
            Ok(CelH(t.0.clone(), l, f, t.0.epoch.get()))
        });
        m.add_method("clearCel", |_, t, (layer, frame): (i64, i64)| {
            let l = t.0.drawable_layer(Some(layer))?;
            let f = t.0.frame_index(Some(frame))?;
            Ok(t.0.sprite().clear_cels((l, l), (f, f)))
        });

        // MARK: Palette
        m.add_method("getPaletteColor", |_, t, i: i64| {
            let s = t.0.sprite();
            s.palette
                .get(usize::try_from(i).unwrap_or(usize::MAX))
                .map(|c| u32::from_be_bytes(*c) as i64)
                .ok_or_else(|| Error::runtime(format!("palette index {i} does not exist (0..{})", s.palette.len())))
        });
        m.add_method("setPaletteColor", |_, t, (i, color): (i64, i64)| {
            let c = color_arg(color)?;
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
        m.add_method("addPaletteColor", |_, t, color: i64| {
            let c = color_arg(color)?;
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
        f.add_field_method_set("opacity", |_, t, v: i64| {
            t.get()?.opacity = v.clamp(0, 255) as u8;
            Ok(())
        });
        f.add_field_method_get("blendMode", |_, t| Ok(t.get()?.blend_mode));
        f.add_field_method_set("blendMode", |_, t, v: i64| {
            if !(0..=18).contains(&v) {
                return Err(Error::runtime(format!("blend mode {v} is not in 0..18")));
            }
            t.get()?.blend_mode = v as u16;
            Ok(())
        });
        f.add_field_method_get("isGroup", |_, t| Ok(t.get()?.is_group()));
        f.add_field_method_get("isTilemap", |_, t| Ok(t.get()?.kind == crate::document::tilemap::KIND_TILEMAP));
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
        // Tiles are addressed by grid cell: (x, y) is the cell whose top-left pixel is
        // (x * tileWidth, y * tileHeight). Tile 0 is the empty tile.
        m.add_method("getTile", |_, t, (x, y, frame): (i64, i64, Option<i64>)| {
            let (tw, th) = t.tile_size()?;
            let f = t.0.frame_index(frame)?;
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
        m.add_method(
            "setTile",
            |_,
             t,
             (x, y, index, fx, fy, fd, frame): (
                i64,
                i64,
                i64,
                Option<bool>,
                Option<bool>,
                Option<bool>,
                Option<i64>,
            )| {
                let (tw, th) = t.tile_size()?;
                let f = t.0.frame_index(frame)?;
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
            },
        );
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
        f.add_field_method_set("duration", |_, t, v: i64| {
            let i = t.check()?;
            t.0.sprite().durations[i] = v.clamp(1, u16::MAX as i64) as u16;
            Ok(())
        });
        f.add_field_method_get("index", |_, t| Ok(t.check()? + 1));
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
        f.add_field_method_set("from", |_, t, v: i64| {
            let f = t.0.frame_index(Some(v))?;
            let tag = t.get()?;
            let (a, b) = (f as u16, tag.to);
            (tag.from, tag.to) = (a.min(b), a.max(b));
            Ok(())
        });
        f.add_field_method_get("to", |_, t| Ok(t.get()?.to as usize + 1));
        f.add_field_method_set("to", |_, t, v: i64| {
            let f = t.0.frame_index(Some(v))?;
            let tag = t.get()?;
            let (a, b) = (tag.from, f as u16);
            (tag.from, tag.to) = (a.min(b), a.max(b));
            Ok(())
        });
        f.add_field_method_get("direction", |_, t| Ok(t.get()?.direction));
        f.add_field_method_set("direction", |_, t, v: i64| {
            if !(0..=3).contains(&v) {
                return Err(Error::runtime(format!("direction {v} is not in 0..3 (see app.direction)")));
            }
            t.get()?.direction = v as u8;
            Ok(())
        });
        f.add_field_method_get("repeatCount", |_, t| Ok(t.get()?.repeat));
        f.add_field_method_set("repeatCount", |_, t, v: i64| {
            t.get()?.repeat = v.clamp(0, u16::MAX as i64) as u16;
            Ok(())
        });
        f.add_field_method_get("color", |_, t| {
            let c = t.get()?.color;
            Ok(u32::from_be_bytes([c[0], c[1], c[2], 255]) as i64)
        });
        f.add_field_method_set("color", |_, t, v: i64| {
            let c = color_arg(v)?;
            t.get()?.color = [c[0], c[1], c[2]];
            Ok(())
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
        f.add_field_method_set("opacity", |_, t, v: i64| {
            t.check()?;
            if t.0.sprite().set_cel_opacity(t.1, t.2, v.clamp(0, 255) as u8) {
                Ok(())
            } else {
                Err(Error::runtime("the cel is empty; draw something first"))
            }
        });
        f.add_field_method_get("isEmpty", |_, t| {
            t.check()?;
            Ok(t.0.sprite().cels[t.2][t.1].as_ref().is_none_or(|c| c.pixels.chunks_exact(4).all(|p| p[3] == 0)))
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
        f.add_field_method_get("userData", |_, t| {
            t.check()?;
            Ok(UserDataH::new(&t.0, 2, t.2, t.1))
        });
        f.add_field_method_get("layer", |_, t| Ok(t.1 + 1));
        f.add_field_method_get("frame", |_, t| Ok(t.2 + 1));
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

impl UserData for SelectionH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("isEmpty", |_, t| Ok(t.0.mask().is_none()));
        f.add_field_method_get("bounds", |lua, t| {
            let Some((x, y, w, h)) = t.0.mask().and_then(|m| m.bounds()) else { return Ok(None) };
            let tb = lua.create_table()?;
            tb.set("x", x)?;
            tb.set("y", y)?;
            tb.set("w", w)?;
            tb.set("h", h)?;
            Ok(Some(tb))
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("contains", |_, t, (x, y): (i64, i64)| {
            Ok(match t.0.mask() {
                Some(m)
                    if x >= i32::MIN as i64 && x <= i32::MAX as i64 && y >= i32::MIN as i64 && y <= i32::MAX as i64 =>
                {
                    m.get(x as i32, y as i32)
                }
                _ => false,
            })
        });
        m.add_method("selectRect", |_, t, (x, y, w, h, mode): (i64, i64, i64, i64, Option<Value>)| {
            let mode = mode_arg(mode)?;
            let s = t.0.sprite();
            let (cw, ch) = (s.width as i64, s.height as i64);
            let mut r = Selection::new(s.width, s.height, false);
            for py in y.max(0)..(y + h).min(ch) {
                for px in x.max(0)..(x + w).min(cw) {
                    r.bits[(py * cw + px) as usize] = true;
                }
            }
            let combined = match (mode, t.0.mask()) {
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
                (2, None) => Selection::new(s.width, s.height, false),
                _ => r,
            };
            t.0.set_mask(combined);
            Ok(())
        });
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

impl UserData for UserDataH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("text", |_, t| Ok(t.read()?.text));
        f.add_field_method_set("text", |_, t, v: Option<String>| t.write(|u| u.text = v.filter(|s| !s.is_empty())));
        f.add_field_method_get("color", |_, t| Ok(t.read()?.color.map(|c| pack(&c))));
        f.add_field_method_set("color", |_, t, v: Option<i64>| {
            let c = v.map(color_arg).transpose()?;
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
        for v in pixels.sequence_values::<i64>() {
            data.extend(snap.get(color_arg(v?)?));
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

fn rect_table(lua: &Lua, r: crate::document::slices::Rect) -> LuaResult<Table> {
    let t = lua.create_table()?;
    t.set("x", r.x)?;
    t.set("y", r.y)?;
    t.set("w", r.w)?;
    t.set("h", r.h)?;
    Ok(t)
}

fn slice_rect(x: i64, y: i64, w: i64, h: i64) -> LuaResult<crate::document::slices::Rect> {
    const LIMIT: i64 = 1 << 20;
    if x.abs() > LIMIT || y.abs() > LIMIT || !(0..=LIMIT).contains(&w) || !(0..=LIMIT).contains(&h) {
        return Err(Error::runtime("slice rectangle out of range"));
    }
    Ok(crate::document::slices::Rect { x: x as i32, y: y as i32, w: w as u32, h: h as u32 })
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
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        // Geometry is per frame: a key set on frame N applies from N until the next key.
        m.add_method("getBounds", |lua, t, frame: Option<i64>| {
            let f = t.0.frame_index(frame)? as u32;
            let sl = t.get()?;
            sl.key_at(f).or(sl.keys.first()).map(|k| rect_table(lua, k.bounds)).transpose()
        });
        m.add_method("setBounds", |_, t, (x, y, w, h, frame): (i64, i64, i64, i64, Option<i64>)| {
            let f = t.0.frame_index(frame)? as u32;
            let r = slice_rect(x, y, w, h)?;
            t.get()?;
            crate::document::slices::set_bounds(t.0.sprite(), t.1, f, r);
            Ok(())
        });
        m.add_method("getCenter", |lua, t, frame: Option<i64>| {
            let f = t.0.frame_index(frame)? as u32;
            let sl = t.get()?;
            if !sl.nine_patch {
                return Ok(None);
            }
            sl.key_at(f).or(sl.keys.first()).map(|k| rect_table(lua, k.center)).transpose()
        });
        m.add_method("setCenter", |_, t, (x, y, w, h, frame): (i64, i64, i64, i64, Option<i64>)| {
            let f = t.0.frame_index(frame)? as u32;
            let r = slice_rect(x, y, w, h)?;
            let sl = t.get()?;
            crate::document::slices::key_for_edit(sl, f).center = r;
            sl.nine_patch = true;
            Ok(())
        });
        m.add_method("clearCenter", |_, t, ()| {
            t.get()?.nine_patch = false;
            Ok(())
        });
        m.add_method("getPivot", |_, t, frame: Option<i64>| {
            let f = t.0.frame_index(frame)? as u32;
            let sl = t.get()?;
            if !sl.has_pivot {
                return Ok((None, None));
            }
            let p = sl.key_at(f).or(sl.keys.first()).map(|k| k.pivot);
            Ok((p.map(|p| p.0), p.map(|p| p.1)))
        });
        m.add_method("setPivot", |_, t, (x, y, frame): (i64, i64, Option<i64>)| {
            let f = t.0.frame_index(frame)? as u32;
            if x.abs() > 1 << 20 || y.abs() > 1 << 20 {
                return Err(Error::runtime("pivot out of range"));
            }
            let sl = t.get()?;
            crate::document::slices::key_for_edit(sl, f).pivot = (x as i32, y as i32);
            sl.has_pivot = true;
            Ok(())
        });
        m.add_method("clearPivot", |_, t, ()| {
            t.get()?.has_pivot = false;
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
    // Nothing that touches files; `load` is limited to text chunks.
    g.set("dofile", Value::Nil)?;
    g.set("loadfile", Value::Nil)?;
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
    let blend = lua.create_table()?;
    for (i, n) in BLEND_NAMES.iter().enumerate() {
        blend.set(*n, i)?;
    }
    app.set("blend", blend)?;
    let direction = lua.create_table()?;
    for (i, n) in ["Forward", "Reverse", "PingPong", "PingPongReverse"].iter().enumerate() {
        direction.set(*n, i)?;
    }
    app.set("direction", direction)?;
    app.set("version", env!("CARGO_PKG_VERSION"))?;
    // Bumped when the API grows, so scripts can check `app.apiVersion >= 2`.
    app.set("apiVersion", 2)?;

    // `app.activeLayer` / `app.activeFrame` read and set the defaults of the pixel calls, and ask
    // the editor to select that layer and frame when the run ends.
    let meta = lua.create_table()?;
    let c = ctx.clone();
    meta.set(
        "__index",
        lua.create_function(move |_, (_t, key): (Value, Value)| {
            let name = if let Value::String(s) = &key { s.to_string_lossy() } else { String::new() };
            Ok(match name.as_str() {
                "activeLayer" => Some(c.layer.get() + 1),
                "activeFrame" => Some(c.frame.get() + 1),
                _ => None,
            })
        })?,
    )?;
    let c = ctx.clone();
    meta.set(
        "__newindex",
        lua.create_function(move |_, (t, key, value): (Table, Value, Value)| {
            let name = if let Value::String(s) = &key { s.to_string_lossy() } else { String::new() };
            let index = |v: &Value| match v {
                Value::Integer(i) => Ok(*i),
                Value::Number(n) if n.fract() == 0.0 && n.abs() < 1e15 => Ok(*n as i64),
                _ => Err(Error::runtime("expected a layer or frame number")),
            };
            match name.as_str() {
                "activeLayer" => {
                    let l = c.layer_index(Some(index(&value)?))?;
                    if c.sprite().layers[l].is_group() {
                        return Err(Error::runtime("a group cannot be the active layer"));
                    }
                    c.layer.set(l);
                    c.selected.set(true);
                }
                "activeFrame" => {
                    c.frame.set(c.frame_index(Some(index(&value)?))?);
                    c.selected.set(true);
                }
                _ => t.raw_set(key, value)?,
            }
            Ok(())
        })?,
    )?;
    app.set_metatable(Some(meta))?;
    g.set("app", app)?;

    let color = lua.create_table()?;
    color.set(
        "rgba",
        lua.create_function(|_, (r, g, b, a): (i64, i64, i64, Option<i64>)| {
            let ch = |v: i64| v.clamp(0, 255) as u32;
            Ok((ch(r) << 24 | ch(g) << 16 | ch(b) << 8 | ch(a.unwrap_or(255))) as i64)
        })?,
    )?;
    color.set(
        "split",
        lua.create_function(|_, c: i64| {
            let [r, g, b, a] = color_arg(c)?;
            Ok((r, g, b, a))
        })?,
    )?;
    g.set("color", color)?;

    g.set("sprite", SpriteH(ctx.clone()))?;
    Ok(())
}

/// Runs `source` against the editor. On success the changes form a single undo step; on failure
/// the document is restored. `active` is the (layer, frame) pair, 0-based, scripts default to.
pub fn run(e: &mut Editor, source: &str, active: (usize, usize), limits: Limits) -> Outcome {
    run_with(e, source, active, limits, &NEVER)
}

/// Like `run`; the script stops with "script cancelled" soon after `cancel` becomes true.
pub fn run_with(
    e: &mut Editor,
    source: &str,
    active: (usize, usize),
    limits: Limits,
    cancel: &'static AtomicBool,
) -> Outcome {
    let before = e.sprite.clone();
    let (w, h) = (e.sprite.width, e.sprite.height);
    let old_mask = e.sel.mask.clone().filter(|m| m.w == w && m.h == h);
    let ctx = Rc::new(Ctx {
        ed: e as *mut Editor,
        messages: RefCell::new(Vec::new()),
        label: RefCell::new(None),
        layer: Cell::new(active.0.min(e.sprite.layers.len() - 1)),
        frame: Cell::new(active.1.min(e.sprite.frame_count() - 1)),
        selected: Cell::new(false),
        epoch: Cell::new(0),
        mask: RefCell::new(old_mask.clone()),
    });
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
    drop(ctx);
    match result {
        Ok(()) => {
            conform_changed_cels(&mut e.sprite, &before);
            e.sprite.flush_links();
            if e.sprite != before || new_mask != old_mask {
                let after = std::mem::replace(&mut e.sprite, before);
                e.checkpoint(); // records the old selection and sprite
                e.sprite = after;
                e.sel.mask = new_mask;
                e.label_last(label.as_deref().unwrap_or("Script"));
            }
            Outcome { messages, select, error: None }
        }
        Err(err) => {
            e.sprite = before;
            Outcome { messages, select: None, error: Some(clean_error(&err)) }
        }
    }
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

fn exec(ctx: &Rc<Ctx>, source: &str, limits: Limits, cancel: &'static AtomicBool) -> LuaResult<()> {
    let libs = StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH | StdLib::COROUTINE;
    let lua = Lua::new_with(libs, LuaOptions::default())?;
    lua.set_memory_limit(limits.memory)?;
    setup(&lua, ctx)?;

    const STEP: u32 = 1000;
    let started = Instant::now();
    let executed = std::cell::Cell::new(0u64);
    lua.set_hook(mlua::HookTriggers::new().every_nth_instruction(STEP), move |_, _| {
        executed.set(executed.get() + STEP as u64);
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::runtime("script cancelled"));
        }
        if executed.get() > limits.instructions {
            return Err(Error::runtime("script stopped: instruction limit exceeded"));
        }
        if started.elapsed() > limits.time {
            return Err(Error::runtime("script stopped: time limit exceeded"));
        }
        Ok(VmState::Continue)
    })?;
    lua.load(source).set_name("=script").set_mode(ChunkMode::Text).exec()
}

/// Example scripts shipped with the app (copied from `scripts/lua-examples/`).
pub const EXAMPLES: [(&str, &str); 5] = [
    ("outline-layer.lua", include_str!("../../scripts/lua-examples/outline-layer.lua")),
    ("color-ramp.lua", include_str!("../../scripts/lua-examples/color-ramp.lua")),
    ("flip-all-frames.lua", include_str!("../../scripts/lua-examples/flip-all-frames.lua")),
    ("palette-swap.lua", include_str!("../../scripts/lua-examples/palette-swap.lua")),
    ("tag-every-4-frames.lua", include_str!("../../scripts/lua-examples/tag-every-4-frames.lua")),
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
    if !out.is_null() {
        *out = std::ptr::null_mut();
    }
    let (Some(e), false) = (doc.as_mut(), source.is_null()) else { return false };
    e.dirty.mark_all();
    let src = CStr::from_ptr(source).to_string_lossy().into_owned();
    let active = active.unwrap_or_else(|| (e.sprite.layers.iter().rposition(|l| !l.is_group()).unwrap_or(0), 0));
    let outcome = run_with(e, &src, active, Limits::default(), &CANCEL);
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
            for _, name in ipairs({ "os", "io", "package", "debug", "dofile", "loadfile", "require" }) do
              assert(_G[name] == nil, name .. " must not exist")
            end
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
            assert(t.color == 0x336699FF)
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
            assert(c1.layer == 1 and c1.frame == 1)
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
              assert(ud.text == "note " .. i and ud.color == 0x10203040 + i)
            end
            targets[2].text = ""            -- empty text and nil clear
            targets[3].color = nil
            assert(targets[2].text == nil and targets[3].color == nil)
            targets[3].text = "kept"
            print(sprite.userData.text, sprite.layers[1].userData.color)
            "#,
        );
        assert_eq!(o.messages, vec![format!("note 1\t{}", 0x10203042u32)]);
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
            assert(type(app.version) == "string" and app.apiVersion >= 2)
            assert(app.activeLayer == 1 and app.activeFrame == 1 and sprite.activeLayer == 1)
            app.activeLayer = 3
            app.activeFrame = 2
            assert(sprite.activeLayer == 3 and sprite.activeFrame == 2 and app.activeLayer == 3)
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
        ok(&mut e, "assert(os == nil and io == nil and debug == nil and package == nil and load(string.dump(function() end)) == nil)");
    }
}
