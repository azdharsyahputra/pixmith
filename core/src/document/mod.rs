//! In-memory sprite model and editing operations.

pub mod color_mode;
pub mod layers;
pub mod offcanvas;
pub mod slices;
pub mod spritedata;
pub mod tilemap;

use crate::document::color_mode::ColorMode;
use crate::history::store::{compact_top, inverse_of, settle_all, Entry, Witness};
use std::sync::Arc;

pub use crate::document::offcanvas::OffCanvas;

/// PICO-8 palette, used for new sprites and files without a palette.
const DEFAULT_PALETTE: [u32; 16] = [
    0x000000, 0x1D2B53, 0x7E2553, 0x008751, 0xAB5236, 0x5F574F, 0xC2C3C7, 0xFFF1E8, 0xFF004D, 0xFFA300, 0xFFEC27,
    0x00E436, 0x29ADFF, 0x83769C, 0xFF77A8, 0xFFCCAA,
];

/// Largest canvas (width x height) that Pixmith will create or load; bigger ones would need
/// hundreds of megabytes per cel.
pub const MAX_CANVAS_PIXELS: usize = 1 << 26;

/// Memory budget for all distinct cel buffers of a sprite when an operation would resize them.
pub const MAX_CEL_MEMORY: usize = 1 << 32;

pub fn default_palette() -> Vec<[u8; 4]> {
    DEFAULT_PALETTE
        .iter()
        .map(|c| {
            let [_, r, g, b] = c.to_be_bytes();
            [r, g, b, 255]
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
    pub opacity: u8,
    /// 0 normal, 1 group, 2 tilemap.
    pub kind: u16,
    /// Nesting depth; a group's children follow it with a higher level.
    pub child_level: u16,
    pub blend_mode: u16,
    /// Raw Aseprite layer flags except the visible bit, which lives in `visible`.
    pub flags: u16,
    pub user_data: crate::document::slices::UserData,
    /// Index into `Sprite::tilesets` (tilemap layers only).
    pub tileset: u32,
}

impl Layer {
    pub fn new(name: impl Into<String>) -> Self {
        Layer {
            name: name.into(),
            visible: true,
            opacity: 255,
            kind: 0,
            child_level: 0,
            blend_mode: 0,
            flags: 2, // editable
            user_data: Default::default(),
            tileset: 0,
        }
    }

    pub fn is_group(&self) -> bool {
        self.kind == 1
    }

    /// A reference layer (layer flag 0x40): shown in the editor, never part of exports.
    pub fn is_reference(&self) -> bool {
        self.flags & LAYER_REFERENCE != 0
    }

    /// Whether the layer's "editable" flag (0x2) is clear. Only reference layers enforce it.
    pub fn is_locked(&self) -> bool {
        self.flags & LAYER_EDITABLE == 0
    }
}

/// `.aseprite` layer flag: the layer can be edited.
pub const LAYER_EDITABLE: u16 = 2;
/// `.aseprite` layer flag: reference layer.
pub const LAYER_REFERENCE: u16 = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    pub name: String,
    pub from: u16,
    pub to: u16,
    pub direction: u8,
    pub repeat: u16,
    pub color: [u8; 3],
    pub user_data: crate::document::slices::UserData,
}

/// A full-canvas RGBA buffer with straight (non-premultiplied) alpha, plus the part of the cel's
/// content beyond the canvas edges (`outside`; see `document::offcanvas`).
///
/// Build cels with `Cel { opacity, pixels, ..Default::default() }` so that new fields do not
/// have to be added to every literal.
#[derive(Debug, PartialEq)]
pub struct Cel {
    pub opacity: u8,
    pub pixels: Vec<u8>,
    /// Tile grid of a tilemap layer's cel; `pixels` then holds its rendering.
    pub tiles: Option<crate::document::tilemap::TileGrid>,
    pub user_data: crate::document::slices::UserData,
    /// Palette indices loaded from an indexed file (full canvas); used on save wherever they still
    /// match the pixels, so duplicate palette colors keep their original index.
    pub indices: Option<Arc<Vec<u8>>>,
    /// Z-index of the cel chunk: the cel draws as if its layer sat `z_index` places higher (or lower
    /// when negative) in the stack; see `layers::composite_range_in`.
    pub z_index: i16,
    /// The part of the cel's content beyond the canvas edges, if any (see `document::offcanvas`):
    /// what is visible inside the canvas always comes from `pixels`.
    pub outside: Option<Arc<OffCanvas>>,
}

/// Copy-on-write of a cel shared with undo history goes through here: large pixel buffers reuse
/// pooled memory and are copied by several threads (see `bufpool`).
impl Clone for Cel {
    fn clone(&self) -> Self {
        Cel {
            opacity: self.opacity,
            pixels: crate::history::buffer_pool::copy_of(&self.pixels),
            tiles: self.tiles.clone(),
            user_data: self.user_data.clone(),
            indices: self.indices.clone(),
            z_index: self.z_index,
            outside: self.outside.clone(),
        }
    }
}

/// An empty, fully opaque cel (`derive(Default)` would make the opacity 0).
impl Default for Cel {
    fn default() -> Self {
        Cel {
            opacity: 255,
            pixels: Vec::new(),
            tiles: None,
            user_data: Default::default(),
            indices: None,
            z_index: 0,
            outside: None,
        }
    }
}

// Test-only count of cel buffers copied because they were shared.
#[cfg(test)]
thread_local! {
    pub(crate) static CEL_CLONES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sprite {
    pub width: u32,
    pub height: u32,
    /// Bottom to top.
    pub layers: Vec<Layer>,
    /// Per-frame duration in milliseconds.
    pub durations: Vec<u16>,
    /// `cels[frame][layer]`. Buffers are shared between undo states until modified.
    pub cels: Vec<Vec<Option<Arc<Cel>>>>,
    pub tags: Vec<Tag>,
    pub palette: Vec<[u8; 4]>,
    /// Bookkeeping that keeps linked cels in sync (see `document::layers`).
    pub links: crate::document::layers::PendingLinks,
    pub tilesets: Vec<crate::document::tilemap::Tileset>,
    /// Pixel encoding used when saving; cel buffers are always RGBA in memory.
    pub color_mode: ColorMode,
    /// Palette index treated as transparent in Indexed mode.
    pub transparent_index: u8,
    /// Slices and user data.
    pub extra: crate::document::slices::Extra,
    /// The file's color profile; kept and written back, pixels are never converted.
    pub color_profile: crate::document::spritedata::ColorProfile,
}

impl Sprite {
    pub fn new(width: u32, height: u32) -> Self {
        let width = width.clamp(1, u16::MAX as u32);
        let mut height = height.clamp(1, u16::MAX as u32);
        if width as usize * height as usize > MAX_CANVAS_PIXELS {
            height = (MAX_CANVAS_PIXELS / width as usize).max(1) as u32;
        }
        Sprite {
            width,
            height,
            layers: vec![Layer::new("Layer 1")],
            durations: vec![100],
            cels: vec![vec![None]],
            tags: Vec::new(),
            palette: default_palette(),
            links: Default::default(),
            tilesets: Vec::new(),
            color_mode: ColorMode::Rgba,
            transparent_index: 0,
            extra: Default::default(),
            color_profile: Default::default(),
        }
    }

    pub fn frame_count(&self) -> usize {
        self.durations.len()
    }

    /// Calls `f(address, bytes)` for every separately allocated, shareable buffer of the sprite:
    /// cel images (with their tile grids), index hints and tile images. Snapshots of one document
    /// share most of these, so callers count each address once.
    pub(crate) fn for_each_buffer(&self, mut f: impl FnMut(usize, usize)) {
        for cel in self.cels.iter().flatten().flatten() {
            let grid = cel.tiles.as_ref().map_or(0, |g| g.tiles.len() * 4);
            f(Arc::as_ptr(cel) as usize, cel.pixels.len() + grid);
            if let Some(o) = &cel.outside {
                f(Arc::as_ptr(o) as usize, o.rgba.len());
            }
            if let Some(i) = &cel.indices {
                f(Arc::as_ptr(i) as usize, i.len());
            }
        }
        for t in self.tilesets.iter().flat_map(|ts| &ts.tiles) {
            f(Arc::as_ptr(t) as usize, t.len());
        }
    }

    /// Checks the structural invariants the editing code relies on, in time proportional to the
    /// number of cels (not pixels): every frame has one cel slot per layer, buffers have the
    /// canvas size, off-canvas parts are within bounds, tile grids and tilesets are consistent, layer nesting is a proper tree.
    pub fn validate(&self) -> Result<(), String> {
        if self.width == 0 || self.height == 0 {
            return Err("the canvas has no pixels".into());
        }
        if self.layers.is_empty() || self.durations.is_empty() {
            return Err("the sprite has no layers or no frames".into());
        }
        if self.cels.len() != self.durations.len() {
            return Err(format!("{} frames have durations but {} have cels", self.durations.len(), self.cels.len()));
        }
        let (w, h) = (self.width as usize, self.height as usize);
        for (i, l) in self.layers.iter().enumerate() {
            let prev = i.checked_sub(1).map(|p| &self.layers[p]);
            let max_level = prev.map_or(0, |p| p.child_level + p.is_group() as u16);
            if l.child_level > max_level {
                return Err(format!(
                    "layer {} is nested {} levels deep below a layer that cannot hold it",
                    i + 1,
                    l.child_level
                ));
            }
            if l.kind == crate::document::tilemap::KIND_TILEMAP && (l.tileset as usize) >= self.tilesets.len() {
                return Err(format!("tilemap layer {} refers to a missing tileset", i + 1));
            }
        }
        for ts in &self.tilesets {
            if ts.tile_w == 0 || ts.tile_h == 0 || ts.tiles.is_empty() {
                return Err("a tileset is empty or has no tile size".into());
            }
            let len = ts.tile_w as usize * ts.tile_h as usize * 4;
            if ts.tiles.iter().any(|t| t.len() != len) {
                return Err("a tile image does not match its tileset's tile size".into());
            }
        }
        let mut outside_bytes = 0usize;
        for (f, row) in self.cels.iter().enumerate() {
            if row.len() != self.layers.len() {
                return Err(format!("frame {} has {} cel slots for {} layers", f + 1, row.len(), self.layers.len()));
            }
            for (l, cel) in row.iter().enumerate() {
                let Some(cel) = cel else { continue };
                if self.layers[l].is_group() {
                    return Err(format!("group layer {} has a cel in frame {}", l + 1, f + 1));
                }
                if cel.pixels.len() != w * h * 4 {
                    return Err(format!(
                        "the cel of layer {} in frame {} has {} bytes, expected {}",
                        l + 1,
                        f + 1,
                        cel.pixels.len(),
                        w * h * 4
                    ));
                }
                if let Some(o) = &cel.outside {
                    o.check().map_err(|e| format!("layer {} in frame {}: {e}", l + 1, f + 1))?;
                    if cel.tiles.is_some() {
                        return Err(format!(
                            "the tilemap cel of layer {} in frame {} has off-canvas pixels",
                            l + 1,
                            f + 1
                        ));
                    }
                    outside_bytes += o.rgba.len();
                    if outside_bytes > MAX_CEL_MEMORY {
                        return Err("the cels hold too many pixels beyond the canvas".into());
                    }
                }
                if cel.indices.as_ref().is_some_and(|i| i.len() != w * h) {
                    return Err("a cel's palette index hints do not match the canvas".into());
                }
                if let Some(g) = &cel.tiles {
                    if g.tiles.len() != g.w as usize * g.h as usize {
                        return Err("a tile grid does not hold w x h tiles".into());
                    }
                }
            }
        }
        Ok(())
    }

    /// Whether giving every distinct cel a `w` x `h` RGBA buffer stays within `MAX_NEW_CEL_BYTES`.
    /// Operations that rebuild all cels at a new size check this first, because the old buffers
    /// stay alive in the undo history while the new ones are allocated.
    pub(crate) fn cels_fit_budget(&self, w: u64, h: u64) -> bool {
        let mut seen = std::collections::HashSet::new();
        let distinct = self.cels.iter().flatten().flatten().filter(|c| seen.insert(Arc::as_ptr(c))).count() as u128;
        distinct * w as u128 * h as u128 * 4 <= MAX_NEW_CEL_BYTES as u128
    }

    /// Number of distinct cel buffers (linked cels count once).
    pub fn distinct_cel_count(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        seen.extend(self.cels.iter().flatten().flatten().map(Arc::as_ptr));
        seen.len()
    }

    fn pixel_len(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    /// Whether a layer and all of its parent groups are visible.
    pub fn is_layer_shown(&self, i: usize) -> bool {
        if !self.layers[i].visible {
            return false;
        }
        let mut level = self.layers[i].child_level;
        for parent in self.layers[..i].iter().rev() {
            if level == 0 {
                break;
            }
            if parent.child_level == level - 1 {
                if !parent.visible {
                    return false;
                }
                level -= 1;
            }
        }
        true
    }

    /// Composites the visible layers of a frame.
    pub fn render(&self, frame: usize) -> Vec<u8> {
        crate::document::layers::render_frame(self, frame)
    }

    /// The cel to draw on, created empty when the layer has none. A locked reference layer cannot be drawn on.
    pub(crate) fn cel_mut(&mut self, layer: usize, frame: usize) -> Option<&mut Cel> {
        let l = self.layers.get(layer)?;
        if l.is_reference() && l.is_locked() {
            return None;
        }
        self.cel_mut_any(layer, frame)
    }

    /// `cel_mut` for property edits (opacity, z-index), which a layer lock does not forbid.
    pub(crate) fn cel_mut_any(&mut self, layer: usize, frame: usize) -> Option<&mut Cel> {
        if frame >= self.frame_count() || self.layers.get(layer)?.is_group() {
            return None;
        }
        self.begin_cel_edit(layer, frame);
        let len = self.pixel_len();
        let cel = self.cels[frame][layer]
            .get_or_insert_with(|| Arc::new(Cel { opacity: 255, pixels: vec![0; len], ..Default::default() }));
        #[cfg(test)]
        if Arc::strong_count(cel) > 1 {
            CEL_CLONES.with(|c| c.set(c.get() + 1));
        }
        Some(Arc::make_mut(cel))
    }

    /// Draws a line with a square brush, replacing pixels (a transparent color erases).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_line(
        &mut self,
        layer: usize,
        frame: usize,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        color: [u8; 4],
        size: u32,
    ) {
        let (w, h) = (self.width as i32, self.height as i32);
        let Some(cel) = self.cel_mut(layer, frame) else { return };
        let size = size.clamp(1, 64) as i32;
        let half = (size - 1) / 2;
        let mut stamp = |cx: i32, cy: i32| {
            for y in (cy - half).max(0)..(cy - half + size).min(h) {
                for x in (cx - half).max(0)..(cx - half + size).min(w) {
                    let i = (y * w + x) as usize * 4;
                    cel.pixels[i..i + 4].copy_from_slice(&color);
                }
            }
        };
        let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
        let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
        let (mut x, mut y, mut err) = (x0, y0, dx + dy);
        loop {
            stamp(x, y);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
        if self.layers[layer].kind == crate::document::tilemap::KIND_TILEMAP {
            let r = size + 1;
            let bbox = (x0.min(x1) - r, y0.min(y1) - r, x0.max(x1) + r + 1, y0.max(y1) + r + 1);
            crate::document::tilemap::sync_cel(self, layer, frame, Some(bbox));
        }
        self.flush_links();
    }

    /// 4-connected flood fill on one layer. All fully transparent pixels count as equal.
    pub fn fill(&mut self, layer: usize, frame: usize, x: i32, y: i32, color: [u8; 4]) {
        let (w, h) = (self.width as i32, self.height as i32);
        if x < 0 || y < 0 || x >= w || y >= h {
            return;
        }
        let Some(cel) = self.cel_mut(layer, frame) else { return };
        let px = &mut cel.pixels;
        let get = |px: &[u8], x: i32, y: i32| -> [u8; 4] {
            let i = (y * w + x) as usize * 4;
            px[i..i + 4].try_into().unwrap()
        };
        let same = |a: [u8; 4], b: [u8; 4]| a == b || (a[3] == 0 && b[3] == 0);
        let target = get(px, x, y);
        if same(target, color) {
            return;
        }
        let mut stack = vec![(x, y)];
        while let Some((x, y)) = stack.pop() {
            if x < 0 || y < 0 || x >= w || y >= h || !same(get(px, x, y), target) {
                continue;
            }
            let i = (y * w + x) as usize * 4;
            px[i..i + 4].copy_from_slice(&color);
            stack.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
        }
        if self.layers[layer].kind == crate::document::tilemap::KIND_TILEMAP {
            crate::document::tilemap::sync_cel(self, layer, frame, None);
        }
        self.flush_links();
    }

    /// Inserts a layer above `above` (above its children if it is a group). Returns its index.
    pub fn add_layer(&mut self, above: usize) -> usize {
        let above = above.min(self.layers.len() - 1);
        let level = self.layers[above].child_level;
        let mut at = above + 1;
        while at < self.layers.len() && self.layers[at].child_level > level {
            at += 1;
        }
        let mut layer = Layer::new(format!("Layer {}", self.layers.len() + 1));
        layer.child_level = level;
        self.layers.insert(at, layer);
        for row in &mut self.cels {
            row.insert(at, None);
        }
        at
    }

    /// Removes a layer and its children. Refuses to remove the last drawable layer.
    pub fn remove_layer(&mut self, i: usize) -> bool {
        if i >= self.layers.len() {
            return false;
        }
        let level = self.layers[i].child_level;
        let mut end = i + 1;
        while end < self.layers.len() && self.layers[end].child_level > level {
            end += 1;
        }
        let drawable_left = self.layers.iter().enumerate().any(|(j, l)| !(i..end).contains(&j) && !l.is_group());
        if !drawable_left {
            return false;
        }
        self.layers.drain(i..end);
        for row in &mut self.cels {
            row.drain(i..end);
        }
        true
    }

    /// Inserts a frame after `after`, empty or copying its cels. Returns its index.
    pub fn add_frame(&mut self, after: usize, duplicate: bool) -> usize {
        let after = after.min(self.frame_count() - 1);
        let at = after + 1;
        let row = if duplicate {
            self.cels[after].iter().map(crate::document::layers::independent_cel).collect()
        } else {
            vec![None; self.layers.len()]
        };
        self.cels.insert(at, row);
        self.extra.frame_inserted(at);
        self.durations.insert(at, self.durations[after]);
        for t in &mut self.tags {
            if t.from >= at as u16 {
                t.from += 1;
            }
            if t.to >= at as u16 {
                t.to += 1;
            }
        }
        at
    }

    pub fn remove_frame(&mut self, i: usize) -> bool {
        if self.frame_count() <= 1 || i >= self.frame_count() {
            return false;
        }
        self.cels.remove(i);
        self.extra.frame_removed(i);
        self.durations.remove(i);
        self.extra.clamp_frames(self.durations.len());
        let i = i as u16;
        self.tags.retain(|t| !(t.from == i && t.to == i));
        for t in &mut self.tags {
            if t.from > i {
                t.from -= 1;
            }
            if t.to >= i && t.to > 0 {
                t.to -= 1;
            }
        }
        true
    }
}

/// Rounded `a * b / 255` for 8-bit values.
pub(crate) fn mul_un8(a: u32, b: u32) -> u32 {
    let t = a * b + 0x80;
    ((t >> 8) + t) >> 8
}

/// Source-over blend with straight alpha, in integer arithmetic with rounded multiplies so results
/// match what Aseprite renders for the same layers.
pub(crate) fn blend_over(d: &mut [u8], s: &[u8], opacity: u32) {
    let sa = mul_un8(s[3] as u32, opacity);
    if sa == 0 {
        return;
    }
    if d[3] == 0 {
        d.copy_from_slice(&[s[0], s[1], s[2], sa as u8]);
        return;
    }
    let ba = d[3] as u32;
    let ra = sa + ba - mul_un8(ba, sa);
    for c in 0..3 {
        let (b, s) = (d[c] as i32, s[c] as i32);
        d[c] = (b + (s - b) * sa as i32 / ra as i32) as u8;
    }
    d[3] = ra as u8;
}

pub(crate) const UNDO_LIMIT: usize = 100;

/// Default budget for the memory held only by undo history (see `Editor::set_undo_budget`).
pub const DEFAULT_UNDO_BUDGET: usize = 1 << 30;

static NEXT_UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Most pixel memory one resize or scale may allocate for the rebuilt cels.
pub(crate) const MAX_NEW_CEL_BYTES: u64 = 1 << 30;

/// A sprite plus undo history. Only the snapshot nearest the live document shares whole cel
/// buffers with it; older ones store just what changed (see `history_store`).
pub struct Editor {
    pub sprite: Sprite,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    pub sel: crate::paint::selection::State,
    pub states: crate::history::States,
    /// Labels parallel to `undo` / `redo` (empty = unlabeled), shown by the history panel.
    pub(crate) labels: Vec<String>,
    pub(crate) redo_labels: Vec<String>,
    /// Open transaction depth and whether the transaction has already taken its checkpoint.
    pub(crate) txn_depth: u32,
    pub(crate) txn_taken: bool,
    /// What changed since a viewer last asked (see `render::dirty`).
    pub dirty: crate::render::dirty::Dirty,
    /// Unique per editor for the life of the process (unlike its address, which can be reused).
    pub(crate) uid: u64,
    /// Inside a transaction: the state at the most recent (skipped) checkpoint, i.e. before the
    /// operation now running. `last_checkpoint` prefers it so selection clipping sees the right base.
    op_base: Option<Sprite>,
    /// Most bytes of buffers that only undo history keeps alive.
    undo_budget: usize,
    /// The live document's buffers as of the last undo, redo or rollback. While the live document
    /// still holds exactly these, the nearest history entries that are stored relative to the live
    /// document (`Entry::Compact`) can be applied to it in place.
    sync: Option<Witness>,
    /// How many times history had to be discarded because the live document was edited without a
    /// checkpoint while history relied on it (should stay 0).
    history_drops: u32,
    /// Number of checkpoints taken so far; lets the FFI guard tell whether an operation took its own.
    checkpoints_taken: u64,
    /// The drawing preview in progress, if any (see `begin_preview`).
    preview: Option<Preview>,
    next_preview: u64,
    /// How opaque the editor draws reference layers, in percent (0 hides them). A view setting: it is not
    /// saved and exports never show reference layers whatever it says.
    pub reference_display: u8,
}

/// A preview session: the document as it was when the gesture began, kept apart from the undo
/// history so that rewinding is right inside transactions and for any history layout. The stamp
/// records where the history was; if it moved (undo, redo, another edit) the session is stale.
struct Preview {
    id: u64,
    base: Sprite,
    stamp: (u64, usize, usize),
}

impl Editor {
    pub fn new(sprite: Sprite) -> Self {
        Editor {
            sprite,
            undo: Vec::new(),
            redo: Vec::new(),
            sel: Default::default(),
            states: Default::default(),
            labels: Vec::new(),
            redo_labels: Vec::new(),
            txn_depth: 0,
            txn_taken: false,
            dirty: Default::default(),
            uid: NEXT_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            op_base: None,
            undo_budget: DEFAULT_UNDO_BUDGET,
            sync: None,
            history_drops: 0,
            checkpoints_taken: 0,
            preview: None,
            next_preview: 0,
            reference_display: 100,
        }
    }

    /// Unique per editor for the life of the process (unlike its address, which can be reused).
    pub fn uid(&self) -> u64 {
        self.uid
    }

    /// How often history was discarded because of an edit that bypassed `checkpoint` (a bug
    /// indicator; tests assert it stays 0).
    pub fn history_drops(&self) -> u32 {
        self.history_drops
    }

    /// Whether the entries stored relative to the live document still match it. If not (the
    /// document was edited without a checkpoint), no entry can be trusted and history is dropped.
    fn history_in_sync(&mut self) -> bool {
        let relative =
            matches!(self.undo.last(), Some(Entry::Compact(_))) || matches!(self.redo.last(), Some(Entry::Compact(_)));
        if !relative || self.sync.as_ref().is_some_and(|w| w.matches(&self.sprite)) {
            return true;
        }
        self.undo.clear();
        self.redo.clear();
        self.labels.clear();
        self.redo_labels.clear();
        self.sel.drop_history();
        self.states.drop_history();
        self.sync = None;
        self.op_base = None;
        self.history_drops += 1;
        false
    }

    /// Forgets the redo steps (their entries are stored relative to the live document).
    fn drop_redo(&mut self) {
        self.redo.clear();
        self.redo_labels.clear();
        self.sel.clear_redo();
        self.states.clear_redo();
    }

    /// Turns the live sprite into the state `step` describes and returns the entry that restores
    /// the state it had. The nearest whole entry of the other chain (`redo` when undoing, `undo`
    /// when redoing), if any, is compacted against the outgoing content first.
    fn swap_live(&mut self, mut step: Entry, undoing: bool) -> Entry {
        // A finished background compaction is used as such (patched in place); an unfinished one
        // is used whole, which is just as quick.
        step.settle(false);
        let other = if undoing { &mut self.redo } else { &mut self.undo };
        compact_top(other, &self.sprite);
        if matches!(step, Entry::Compact(_)) {
            // Background compactions read the live cels; let them finish (normally they already
            // have) so the step below can patch those cels in place instead of copying them.
            settle_all(&mut self.undo);
            settle_all(&mut self.redo);
        }
        match step.into_whole() {
            Ok(prev) => {
                let old = std::mem::replace(&mut self.sprite, prev);
                // Compacting the outgoing state means comparing whole cels: the history thread
                // does it, and the entry is used whole until then.
                inverse_of(old, &self.sprite)
            }
            Err(c) => Entry::Compact(c.apply_in_place(&mut self.sprite)),
        }
    }

    /// Waits until the history thread has compacted every snapshot. Editing never needs this
    /// (unfinished compactions are used as whole snapshots); tests and memory reports do.
    pub fn wait_for_history(&mut self) {
        settle_all(&mut self.undo);
        settle_all(&mut self.redo);
    }

    /// Sets the byte budget of the undo history and trims it to fit right away. History is trimmed
    /// at each checkpoint, before the edit it protects has copied anything, so it may exceed the
    /// budget by the buffers that one edit copies.
    pub fn set_undo_budget(&mut self, bytes: usize) {
        self.undo_budget = bytes;
        self.enforce_undo_budget();
    }

    /// Bytes of buffers (cel images, tile images, index hints, selection masks) that only the
    /// undo and redo history keeps alive, each shared buffer counted once.
    pub fn history_bytes(&self) -> usize {
        self.history_bytes_per_step(true).1
    }

    /// Like `history_bytes`, but without the newest undo step. That one still holds whole cel
    /// buffers (the state before the latest edit), which is a temporary extra copy of each cel the
    /// edit touched; everything older is stored as deltas.
    pub fn history_bytes_beyond_newest(&self) -> usize {
        let (per, total) = self.history_bytes_per_step(true);
        total - per.last().copied().unwrap_or(0)
    }

    /// Bytes attributed to each undo step (index 0 is the oldest) and the total including redo
    /// steps. A buffer shared by several snapshots, or with the live sprite, is attributed to the
    /// newest one that holds it, so dropping the oldest step frees exactly its share.
    /// With `wait`, compactions still running on the history thread are waited for, so the counts
    /// are exact; without, they count as nothing yet.
    fn history_bytes_per_step(&self, wait: bool) -> (Vec<usize>, usize) {
        let mut seen = std::collections::HashSet::new();
        self.sprite.for_each_buffer(|p, _| {
            seen.insert(p);
        });
        if let Some((p, _)) = self.sel.mask_info() {
            seen.insert(p);
        }
        let mut total = 0;
        for (k, s) in self.redo.iter().enumerate().rev() {
            s.for_each_buffer(
                |p, b| {
                    if seen.insert(p) {
                        total += b;
                    }
                },
                wait,
            );
            if let Some((p, b)) = self.sel.redo_mask_info(k) {
                if seen.insert(p) {
                    total += b;
                }
            }
        }
        let mut per = vec![0usize; self.undo.len()];
        for k in (0..self.undo.len()).rev() {
            let mut bytes = 0;
            self.undo[k].for_each_buffer(
                |p, b| {
                    if seen.insert(p) {
                        bytes += b;
                    }
                },
                wait,
            );
            if let Some((p, b)) = self.sel.undo_mask_info(k) {
                if seen.insert(p) {
                    bytes += b;
                }
            }
            per[k] = bytes;
        }
        total += per.iter().sum::<usize>();
        (per, total)
    }

    /// Drops the oldest undo steps until the history fits the byte budget. The newest step always
    /// stays, so the last edit can be undone however large it is.
    fn enforce_undo_budget(&mut self) {
        if self.undo.len() <= 1 {
            return;
        }
        let (mut per, mut total) = self.history_bytes_per_step(false);
        if total > self.undo_budget {
            // Over budget counting only finished compactions: settle the running ones and count
            // exactly before dropping steps.
            settle_all(&mut self.undo);
            settle_all(&mut self.redo);
            (per, total) = self.history_bytes_per_step(true);
        }
        let mut drop = 0;
        while total > self.undo_budget && drop + 1 < per.len() {
            total -= per[drop];
            drop += 1;
        }
        for _ in 0..drop {
            self.undo.remove(0);
            self.labels.remove(0);
            self.sel.drop_oldest_undo();
            self.states.drop_oldest_undo();
        }
    }

    /// Records the current state so the next change can be undone.
    /// Inside a transaction only the first checkpoint is recorded, so the whole transaction undoes at once.
    pub fn checkpoint(&mut self) {
        if self.txn_depth > 0 {
            if self.txn_taken {
                self.op_base = Some(self.sprite.clone());
                // The undo step already exists, but the document is a new state again (it may
                // have been saved in the middle of the transaction).
                self.states.touch();
                return;
            }
            self.txn_taken = true;
        }
        self.history_in_sync();
        self.sync = None;
        self.sprite.flush_links();
        crate::paint::selection::finish_pending(self);
        self.op_base = None;
        self.sel.on_checkpoint();
        self.checkpoints_taken += 1;
        crate::history::store::push_nearest(&mut self.undo, self.sprite.clone());
        self.labels.push(String::new());
        let trimmed = self.undo.len() > UNDO_LIMIT;
        if trimmed {
            self.undo.remove(0);
            self.labels.remove(0);
        }
        self.states.on_checkpoint(trimmed);
        self.redo.clear();
        self.redo_labels.clear();
        self.enforce_undo_budget();
    }

    /// Starts (or nests inside) a transaction.
    pub fn begin_transaction(&mut self) {
        if self.txn_depth == 0 {
            self.txn_taken = false;
        }
        self.txn_depth += 1;
    }

    /// Ends the innermost transaction.
    pub fn end_transaction(&mut self) {
        self.txn_depth = self.txn_depth.saturating_sub(1);
        if self.txn_depth == 0 {
            self.txn_taken = false;
            self.op_base = None;
        }
    }

    /// Names the most recent undo step, unless it already has a name.
    pub fn label_last(&mut self, label: &str) {
        if let Some(l) = self.labels.last_mut().filter(|l| l.is_empty()) {
            *l = label.to_string();
        }
    }

    /// Undo steps, oldest first, followed by redo steps, next first.
    pub fn history_len(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }

    pub fn history_label(&self, i: usize) -> Option<&str> {
        let n = self.labels.len();
        if i < n {
            self.labels.get(i).map(String::as_str)
        } else {
            let r = self.redo_labels.len().checked_sub(1 + (i - n))?;
            self.redo_labels.get(r).map(String::as_str)
        }
    }

    /// Undoes or redoes until `position` steps are applied (0 = the original state).
    pub fn history_goto(&mut self, position: usize) -> bool {
        if position > self.undo.len() + self.redo.len() {
            return false;
        }
        while self.undo.len() > position {
            if !self.undo() {
                return false;
            }
        }
        while self.undo.len() < position {
            if !self.redo() {
                return false;
            }
        }
        true
    }

    pub(crate) fn checkpoint_count(&self) -> u64 {
        self.checkpoints_taken
    }

    /// Brings the document back to a consistent state after an operation panicked half-way.
    /// `before` is `checkpoint_count()` from when the operation started. If the operation took its own
    /// checkpoint, that step is dropped and the sprite returns to it (nothing the user did earlier is
    /// lost). Otherwise the last step is undone, which leaves the half-edited state on the redo stack.
    /// Returns whether the sprite was rewound.
    pub fn recover_after_panic(&mut self, before: u64) -> bool {
        self.txn_depth = 0;
        self.txn_taken = false;
        self.op_base = None;
        if self.checkpoints_taken != before {
            self.rollback()
        } else {
            self.undo()
        }
    }

    /// Undoes the most recent checkpoint without leaving a redo step (for cancelled previews).
    pub(crate) fn rollback(&mut self) -> bool {
        if self.undo.is_empty() || !self.history_in_sync() || self.undo.is_empty() {
            return false;
        }
        self.sync = None;
        // The step a transaction opened is gone: its next checkpoint must take a new one, or the
        // edits that follow would run unprotected against history stored relative to the document.
        self.txn_taken = false;
        // Redo steps are stored relative to the document being discarded; there should be none.
        self.drop_redo();
        match self.undo.pop().map(Entry::into_whole) {
            Some(Ok(prev)) => self.sprite = prev,
            Some(Err(c)) => {
                c.apply_in_place(&mut self.sprite);
            }
            None => return false,
        }
        self.sel.on_rollback();
        self.states.on_rollback();
        self.labels.pop();
        self.sync = Some(Witness::of(&self.sprite));
        true
    }

    fn preview_stamp(&self) -> (u64, usize, usize) {
        (self.checkpoints_taken, self.undo.len(), self.redo.len())
    }

    /// Starts a drawing preview: takes the checkpoint for the gesture (inside a transaction only
    /// the first one is recorded) and remembers the document as it is now. Returns the session id
    /// (never 0). A later `begin_preview` replaces the session.
    pub fn begin_preview(&mut self) -> u64 {
        self.checkpoint();
        self.next_preview += 1;
        let id = self.next_preview;
        self.preview = Some(Preview { id, base: self.sprite.clone(), stamp: self.preview_stamp() });
        id
    }

    fn live_preview(&self) -> Option<&Preview> {
        self.preview.as_ref().filter(|p| p.stamp == self.preview_stamp())
    }

    /// Whether `id` names the open preview session (it is stale once the history moves).
    pub fn preview_open(&self, id: u64) -> bool {
        self.live_preview().is_some_and(|p| p.id == id)
    }

    /// Rewinds the document to where the open preview began so it can be redrawn from scratch.
    /// Without an open session this does nothing, so it can never touch an unrelated step.
    pub fn reset_current_preview(&mut self) -> bool {
        let Some(base) = self.live_preview().map(|p| p.base.clone()) else { return false };
        self.sprite = base;
        true
    }

    /// Like `reset_current_preview`, but only for the session `id`.
    pub fn reset_preview(&mut self, id: u64) -> bool {
        self.preview_open(id) && self.reset_current_preview()
    }

    /// Closes the session `id` (the drawn result stays).
    pub fn end_preview(&mut self, id: u64) {
        if self.preview.as_ref().is_some_and(|p| p.id == id) {
            self.preview = None;
        }
    }

    /// Number of tiles of tileset `ts` in the state saved by the most recent checkpoint.
    pub(crate) fn checkpoint_tile_count(&self, ts: usize) -> Option<usize> {
        if let Some(base) = &self.op_base {
            return base.tilesets.get(ts).map(|t| t.tiles.len());
        }
        match self.undo.last()? {
            Entry::Compact(c) => c.tile_count(ts),
            whole => whole.whole()?.tilesets.get(ts).map(|t| t.tiles.len()),
        }
    }

    /// The cel at (frame, layer) in the state saved by the most recent checkpoint.
    pub(crate) fn checkpoint_cel(&self, frame: usize, layer: usize) -> Option<Arc<Cel>> {
        if let Some(base) = &self.op_base {
            return base.cels.get(frame)?.get(layer)?.clone();
        }
        match self.undo.last()? {
            // Stored relative to the live document, which must not have been edited since.
            Entry::Compact(c) if self.sync.as_ref().is_some_and(|w| w.matches(&self.sprite)) => {
                c.cel_at(&self.sprite, frame, layer)
            }
            Entry::Compact(_) => None,
            whole => whole.whole()?.cels.get(frame)?.get(layer)?.clone(),
        }
    }

    pub fn undo(&mut self) -> bool {
        if self.undo.is_empty() || !self.history_in_sync() || self.undo.is_empty() {
            return false;
        }
        // Release the references the witness holds, so that live buffers nobody else owns can be
        // patched in place.
        self.sync = None;
        self.op_base = None;
        self.txn_taken = false;
        let Some(step) = self.undo.pop() else { return false };
        let inverse = self.swap_live(step, true);
        self.redo.push(inverse);
        self.sel.on_undo();
        self.states.on_undo();
        let label = self.labels.pop().unwrap_or_default();
        self.redo_labels.push(label);
        self.sync = Some(Witness::of(&self.sprite));
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.redo.is_empty() || !self.history_in_sync() || self.redo.is_empty() {
            return false;
        }
        self.sync = None;
        self.op_base = None;
        self.txn_taken = false;
        let Some(step) = self.redo.pop() else { return false };
        let inverse = self.swap_live(step, false);
        self.undo.push(inverse);
        self.sel.on_redo();
        self.states.on_redo();
        let label = self.redo_labels.pop().unwrap_or_default();
        self.labels.push(label);
        self.sync = Some(Witness::of(&self.sprite));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    #[test]
    fn render_respects_visibility_and_opacity() {
        let mut s = Sprite::new(1, 1);
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        let top = s.add_layer(0);
        s.draw_line(top, 0, 0, 0, 0, 0, BLUE, 1);
        s.layers[top].opacity = 128;
        assert_eq!(s.render(0), vec![127, 0, 128, 255]);
        s.layers[top].visible = false;
        assert_eq!(s.render(0), RED.to_vec());
    }

    #[test]
    fn hidden_group_hides_children() {
        let mut s = Sprite::new(1, 1);
        let mut group = Layer::new("group");
        group.kind = 1;
        let mut child = Layer::new("child");
        child.child_level = 1;
        s.layers = vec![group, child];
        s.cels = vec![vec![None, None]];
        s.draw_line(1, 0, 0, 0, 0, 0, RED, 1);
        assert_eq!(s.render(0), RED.to_vec());
        s.layers[0].visible = false;
        assert_eq!(s.render(0), vec![0; 4]);
    }

    #[test]
    fn fill_stops_at_boundary() {
        let mut s = Sprite::new(5, 1);
        s.draw_line(0, 0, 2, 0, 2, 0, RED, 1);
        s.fill(0, 0, 0, 0, BLUE);
        let px = s.render(0);
        assert_eq!(&px[0..4], &BLUE);
        assert_eq!(&px[8..12], &RED);
        assert_eq!(&px[12..16], &[0; 4]);
    }

    #[test]
    fn undo_and_redo_restore_pixels() {
        let mut e = Editor::new(Sprite::new(2, 2));
        e.checkpoint();
        e.sprite.draw_line(0, 0, 0, 0, 1, 1, RED, 1);
        let drawn = e.sprite.render(0);
        assert!(e.undo());
        assert_eq!(e.sprite.render(0), vec![0; 16]);
        assert!(e.redo());
        assert_eq!(e.sprite.render(0), drawn);
    }

    #[test]
    fn removing_frames_adjusts_tags() {
        let mut s = Sprite::new(1, 1);
        for _ in 0..5 {
            s.add_frame(0, false);
        }
        s.tags.push(Tag {
            name: "a".into(),
            from: 2,
            to: 5,
            direction: 0,
            repeat: 0,
            color: [0; 3],
            user_data: Default::default(),
        });
        s.tags.push(Tag {
            name: "b".into(),
            from: 0,
            to: 0,
            direction: 0,
            repeat: 0,
            color: [0; 3],
            user_data: Default::default(),
        });
        s.remove_frame(3);
        s.remove_frame(0);
        assert_eq!(s.tags.len(), 1);
        assert_eq!((s.tags[0].from, s.tags[0].to), (1, 3));
    }

    /// Undo snapshots must share every cel the edit did not touch, however big the sprite is.
    #[test]
    fn undo_snapshots_share_untouched_cels() {
        let mut s = Sprite::new(512, 512);
        let top = s.add_layer(0);
        for _ in 0..9 {
            s.add_frame(0, false);
        }
        for f in 0..10 {
            s.draw_line(0, f, 0, 0, 511, 511, RED, 1);
            s.draw_line(top, f, 0, 511, 511, 0, BLUE, 1);
        }
        let mut e = Editor::new(s);
        for step in 0..20 {
            e.checkpoint();
            e.sprite.draw_line(0, 3, step, 0, step, 10, RED, 1);
        }
        assert_eq!(e.undo.len(), 20);
        // The newest snapshot is whole and shares all cels but (layer 0, frame 3) with the live sprite.
        let Some(Entry::Full(last)) = e.undo.last() else { panic!("the newest snapshot is whole") };
        for f in 0..10 {
            for l in 0..2 {
                let (a, b) = (e.sprite.cels[f][l].as_ref().unwrap(), last.cels[f][l].as_ref().unwrap());
                assert_eq!(Arc::ptr_eq(a, b), (f, l) != (3, 0), "cel ({f},{l})");
            }
        }
        // Older snapshots keep no whole cels at all: each holds only the pixels its edit changed.
        assert!(e.history_bytes_beyond_newest() < 64 * 1024, "{}", e.history_bytes_beyond_newest());
        // Operations that touch no pixels share everything.
        e.checkpoint();
        e.sprite.layers[0].name = "x".into();
        let Some(Entry::Full(last)) = e.undo.last() else { panic!("the newest snapshot is whole") };
        assert!(e
            .sprite
            .cels
            .iter()
            .flatten()
            .flatten()
            .zip(last.cels.iter().flatten().flatten())
            .all(|(a, b)| Arc::ptr_eq(a, b)));
    }
}
