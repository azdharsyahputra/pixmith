//! Selection mask, selection-aware drawing, move/transform, clipboard and canvas resizing.

use crate::document::{blend_over, Cel, Editor, Sprite, UNDO_LIMIT};
use crate::history::store::PackedMask;
use std::sync::Arc;

const MAX_DIM: i32 = 16384;
/// Most points accepted for one selection polygon or drawn polygon.
pub(crate) const MAX_POLY_POINTS: usize = 1 << 22;

/// A canvas-sized mask of selected pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    pub w: u32,
    pub h: u32,
    pub bits: Vec<bool>,
}

impl Selection {
    pub(crate) fn new(w: u32, h: u32, on: bool) -> Self {
        Selection { w, h, bits: vec![on; w as usize * h as usize] }
    }

    /// Inclusive corners in any order, clamped to the canvas.
    fn rect(w: u32, h: u32, a: (i32, i32), b: (i32, i32)) -> Self {
        let mut s = Selection::new(w, h, false);
        let (x0, x1) = (a.0.min(b.0).max(0), a.0.max(b.0).min(w as i32 - 1));
        let (y0, y1) = (a.1.min(b.1).max(0), a.1.max(b.1).min(h as i32 - 1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                s.bits[y as usize * w as usize + x as usize] = true;
            }
        }
        s
    }

    pub fn get(&self, x: i32, y: i32) -> bool {
        x >= 0
            && y >= 0
            && x < self.w as i32
            && y < self.h as i32
            && self.bits[y as usize * self.w as usize + x as usize]
    }

    /// Bounding box as (x, y, width, height), or None when nothing is selected.
    pub fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, -1, -1);
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                if self.get(x, y) {
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    y0 = y0.min(y);
                    y1 = y1.max(y);
                }
            }
        }
        (x1 >= 0).then_some((x0, y0, x1 - x0 + 1, y1 - y0 + 1))
    }

    /// Boundary segments `[x0, y0, x1, y1]` in pixel-corner coordinates, with collinear runs merged.
    pub fn outline(&self) -> Vec<[i32; 4]> {
        let (w, h) = (self.w as i32, self.h as i32);
        let mut out = Vec::new();
        for y in 0..=h {
            let mut start = None;
            for x in 0..=w {
                let edge = x < w && self.get(x, y) != self.get(x, y - 1);
                match (edge, start) {
                    (true, None) => start = Some(x),
                    (false, Some(s)) => {
                        out.push([s, y, x, y]);
                        start = None;
                    }
                    _ => {}
                }
            }
        }
        for x in 0..=w {
            let mut start = None;
            for y in 0..=h {
                let edge = y < h && self.get(x, y) != self.get(x - 1, y);
                match (edge, start) {
                    (true, None) => start = Some(y),
                    (false, Some(s)) => {
                        out.push([x, s, x, y]);
                        start = None;
                    }
                    _ => {}
                }
            }
        }
        out
    }

    /// Square-neighborhood dilation by `n` pixels; `outside` is the value assumed beyond the canvas.
    fn dilated_with(&self, n: i32, outside: bool) -> Selection {
        let (w, h) = (self.w as usize, self.h as usize);
        let n = n.max(0) as usize;
        let pass = |src: &[bool], len: usize, lines: usize, stride_line: usize, stride_item: usize| -> Vec<bool> {
            let mut out = vec![false; src.len()];
            for l in 0..lines {
                let mut prefix = vec![0usize; len + 1];
                for i in 0..len {
                    prefix[i + 1] = prefix[i] + src[l * stride_line + i * stride_item] as usize;
                }
                for i in 0..len {
                    let (lo, hi) = (i.saturating_sub(n), (i + n + 1).min(len));
                    let beyond = outside && (i < n || i + n >= len);
                    out[l * stride_line + i * stride_item] = beyond || prefix[hi] > prefix[lo];
                }
            }
            out
        };
        let rows = pass(&self.bits, w, h, w, 1);
        Selection { w: self.w, h: self.h, bits: pass(&rows, h, w, 1, w) }
    }

    fn dilated(&self, n: i32) -> Selection {
        self.dilated_with(n, false)
    }

    fn eroded(&self, n: i32) -> Selection {
        let mut inv = self.clone();
        inv.bits.iter_mut().for_each(|b| *b = !*b);
        let mut out = inv.dilated_with(n, true);
        out.bits.iter_mut().for_each(|b| *b = !*b);
        out
    }

    /// Area enclosed by a closed path (even-odd, sampled at pixel centers) plus the path's own pixels.
    fn polygon(w: u32, h: u32, pts: &[(i32, i32)]) -> Selection {
        let mut s = Selection::new(w, h, false);
        let mut set = |x: i32, y: i32| {
            if x >= 0 && y >= 0 && x < w as i32 && y < h as i32 {
                s.bits[y as usize * w as usize + x as usize] = true;
            }
        };
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            let (dx, dy) = ((b.0 - a.0).abs(), -(b.1 - a.1).abs());
            let (sx, sy) = (if a.0 < b.0 { 1 } else { -1 }, if a.1 < b.1 { 1 } else { -1 });
            let (mut x, mut y, mut err) = (a.0, a.1, dx + dy);
            loop {
                set(x, y);
                if (x, y) == b {
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
        }
        if pts.len() >= 3 {
            for y in 0..h as i32 {
                let cy = y as f64 + 0.5;
                let mut xs: Vec<f64> = Vec::new();
                for i in 0..pts.len() {
                    let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                    let (ay, by) = (a.1 as f64 + 0.5, b.1 as f64 + 0.5);
                    if (ay <= cy) != (by <= cy) {
                        xs.push(a.0 as f64 + 0.5 + (cy - ay) * (b.0 - a.0) as f64 / (by - ay));
                    }
                }
                xs.sort_by(|a, b| a.total_cmp(b));
                for pair in xs.chunks_exact(2) {
                    for x in 0..w as i32 {
                        let cx = x as f64 + 0.5;
                        if cx >= pair[0] && cx < pair[1] {
                            s.bits[y as usize * w as usize + x as usize] = true;
                        }
                    }
                }
            }
        }
        s
    }

    fn shifted(&self, dx: i32, dy: i32) -> Selection {
        let mut s = Selection::new(self.w, self.h, false);
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let (nx, ny) = (x + dx, y + dy);
                if self.get(x, y) && nx >= 0 && ny >= 0 && nx < self.w as i32 && ny < self.h as i32 {
                    s.bits[ny as usize * self.w as usize + nx as usize] = true;
                }
            }
        }
        s
    }
}

/// How a new shape combines with the existing selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Replace,
    Add,
    Subtract,
}

impl Mode {
    pub fn from_u32(m: u32) -> Mode {
        match m {
            1 => Mode::Add,
            2 => Mode::Subtract,
            _ => Mode::Replace,
        }
    }
}

/// A move in progress: edits are recomputed from the pixels as they were when the drag began.
enum Drag {
    Pixels {
        layer: usize,
        frame: usize,
        base: Vec<u8>,
        /// The cel's off-canvas part when the drag began (a whole-cel move carries it along).
        base_outside: Option<Arc<crate::document::OffCanvas>>,
        mask: Selection,
        had_selection: bool,
    },
    Float {
        x: i32,
        y: i32,
    },
}

/// An interactive selection (marquee, lasso) that recombines its shape with `base` on every update.
struct Session {
    base: Option<Arc<Selection>>,
    mode: Mode,
}

/// Pasted pixels that stay repositionable until committed. They are already composited into the
/// cel (over `base`), so saving and rendering need no special case.
struct Float {
    layer: usize,
    frame: usize,
    base: Vec<u8>,
    w: i32,
    h: i32,
    data: Vec<u8>,
    x: i32,
    y: i32,
}

/// Selection state kept beside the editor; its undo stacks mirror the sprite's.
#[derive(Default)]
pub struct State {
    pub(crate) mask: Option<Arc<Selection>>,
    /// Free-transform session (see `paint::transform`); any checkpoint ends it, keeping the preview.
    pub(crate) xform: Option<crate::paint::transform::Session>,
    drag: Option<Drag>,
    session: Option<Session>,
    float: Option<Float>,
    undo: Vec<MaskSnap>,
    redo: Vec<MaskSnap>,
}

/// A selection mask in a history chain (see `history_store`): the entry nearest the live
/// document holds the mask itself, older ones a compact form.
enum MaskSnap {
    None,
    /// The mask itself (nearest entry only).
    Full(Arc<Selection>),
    /// The same mask as the next entry nearer the live document.
    Same,
    Packed(Arc<PackedMask>),
}

impl MaskSnap {
    /// Bytes and address for memory accounting.
    fn info(&self) -> Option<(usize, usize)> {
        match self {
            MaskSnap::Full(m) => Some((Arc::as_ptr(m) as usize, m.bits.len())),
            MaskSnap::Packed(p) => Some((Arc::as_ptr(p) as usize, p.bytes())),
            MaskSnap::None | MaskSnap::Same => None,
        }
    }
}

/// Appends `mask` (the live mask) as the nearest entry and compacts the previous nearest one.
fn push_mask(chain: &mut Vec<MaskSnap>, mask: Option<Arc<Selection>>) {
    if let Some(prev) = chain.pop() {
        chain.push(match prev {
            MaskSnap::Full(p) => match &mask {
                Some(m) if Arc::ptr_eq(&p, m) => MaskSnap::Same,
                _ => MaskSnap::Packed(Arc::new(PackedMask::pack(&p))),
            },
            other => other,
        });
    }
    chain.push(match mask {
        Some(m) => MaskSnap::Full(m),
        None => MaskSnap::None,
    });
}

/// Removes the nearest entry and makes the next one whole again. Returns the removed mask.
fn pop_mask(chain: &mut Vec<MaskSnap>) -> Option<Option<Arc<Selection>>> {
    let nearest = match chain.pop()? {
        MaskSnap::Full(m) => Some(m),
        _ => None,
    };
    if let Some(prev) = chain.last_mut() {
        match prev {
            MaskSnap::Same => *prev = nearest.clone().map_or(MaskSnap::None, MaskSnap::Full),
            MaskSnap::Packed(p) => *prev = MaskSnap::Full(Arc::new(p.unpack())),
            _ => {}
        }
    }
    Some(nearest)
}

impl State {
    /// The selection if it exists, is non-empty and matches the canvas size.
    pub fn active(&self, w: u32, h: u32) -> Option<&Selection> {
        self.mask.as_deref().filter(|m| m.w == w && m.h == h)
    }

    /// Address and byte size of the live mask, for memory accounting.
    pub(crate) fn mask_info(&self) -> Option<(usize, usize)> {
        self.mask.as_ref().map(|m| (Arc::as_ptr(m) as usize, m.bits.len()))
    }

    pub(crate) fn undo_mask_info(&self, step: usize) -> Option<(usize, usize)> {
        self.undo.get(step)?.info()
    }

    pub(crate) fn redo_mask_info(&self, step: usize) -> Option<(usize, usize)> {
        self.redo.get(step)?.info()
    }

    /// Forgets the oldest undo step's mask (the editor dropped that snapshot).
    pub(crate) fn drop_oldest_undo(&mut self) {
        if !self.undo.is_empty() {
            self.undo.remove(0);
        }
    }

    /// Forgets all undo and redo masks.
    pub(crate) fn drop_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub(crate) fn clear_redo(&mut self) {
        self.redo.clear();
    }

    pub(crate) fn on_checkpoint(&mut self) {
        // Any other edit ends a gesture in progress; its saved pixels would no longer match the cel.
        self.float = None;
        self.xform = None;
        self.drag = None;
        self.session = None;
        push_mask(&mut self.undo, self.mask.clone());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub(crate) fn on_undo(&mut self) {
        if let Some(prev) = pop_mask(&mut self.undo) {
            push_mask(&mut self.redo, std::mem::replace(&mut self.mask, prev));
        }
        self.reset_interaction();
    }

    /// Like `on_undo`, but the undone step is dropped instead of becoming redoable.
    pub(crate) fn on_rollback(&mut self) {
        if let Some(prev) = pop_mask(&mut self.undo) {
            self.mask = prev;
        }
        self.reset_interaction();
    }

    pub(crate) fn on_redo(&mut self) {
        if let Some(next) = pop_mask(&mut self.redo) {
            push_mask(&mut self.undo, std::mem::replace(&mut self.mask, next));
        }
        self.reset_interaction();
    }

    pub(crate) fn reset_interaction(&mut self) {
        self.float = None;
        self.xform = None;
        self.drag = None;
        self.session = None;
    }
}

/// Writes pixel edits of a tilemap layer back into its tiles (no-op for other layers).
pub(crate) fn sync_tiles(e: &mut Editor, layer: usize, frame: usize) {
    if e.sprite.layers.get(layer).is_some_and(|l| l.kind == crate::document::tilemap::KIND_TILEMAP) {
        crate::document::tilemap::sync_cel(&mut e.sprite, layer, frame, None);
        // Tiles allocated by this gesture and left unused again (erased, pixel-perfect corners) are not kept.
        let ts = e.sprite.layers[layer].tileset as usize;
        if let Some(before) = e.checkpoint_tile_count(ts) {
            crate::document::tilemap::trim_new_unused(&mut e.sprite, layer, before);
        }
    }
}

/// Called by `Editor::checkpoint`: previews that are about to become permanent (a floating paste
/// or a free transform) get their tile write-back first.
pub(crate) fn finish_pending(e: &mut Editor) {
    let mut targets = Vec::new();
    if let Some(f) = &e.sel.float {
        targets.push((f.layer, f.frame));
    }
    if let Some(t) = crate::paint::transform::pending_target(e) {
        targets.push(t);
    }
    for (layer, frame) in targets {
        sync_tiles(e, layer, frame);
    }
}

pub(crate) fn nonempty(m: Selection) -> Option<Arc<Selection>> {
    m.bits.contains(&true).then(|| Arc::new(m))
}

/// Sets the mask without recording undo (call `checkpoint` first).
fn set_mask(e: &mut Editor, m: Selection) {
    e.sel.mask = nonempty(m);
}

/// Replaces the selection as one undo step; does nothing when it would not change.
fn change_mask(e: &mut Editor, m: Selection) -> bool {
    let new = nonempty(m);
    let (w, h) = (e.sprite.width, e.sprite.height);
    if new.as_deref() == e.sel.active(w, h) {
        return false;
    }
    e.checkpoint();
    e.sel.mask = new;
    true
}

fn combine(base: Option<&Selection>, shape: Selection, mode: Mode) -> Selection {
    let Some(base) = base.filter(|b| b.w == shape.w && b.h == shape.h) else {
        return if mode == Mode::Subtract { Selection::new(shape.w, shape.h, false) } else { shape };
    };
    match mode {
        Mode::Replace => shape,
        Mode::Add => Selection { bits: base.bits.iter().zip(&shape.bits).map(|(a, b)| *a || *b).collect(), ..shape },
        Mode::Subtract => {
            Selection { bits: base.bits.iter().zip(&shape.bits).map(|(a, b)| *a && !*b).collect(), ..shape }
        }
    }
}

/// Starts an interactive selection: one undo step, then live updates via `apply_shape`.
pub fn begin_session(e: &mut Editor, mode: Mode) {
    e.checkpoint();
    let (w, h) = (e.sprite.width, e.sprite.height);
    let base = if e.sel.active(w, h).is_some() { e.sel.mask.clone() } else { None };
    e.sel.session = Some(Session { base, mode });
}

pub fn end_session(e: &mut Editor) {
    e.sel.session = None;
}

/// Updates the selection with a shape, combined with the session's base (or replacing it).
fn apply_shape(e: &mut Editor, shape: Selection) {
    if let Some(s) = &e.sel.session {
        e.sel.mask = nonempty(combine(s.base.as_deref(), shape, s.mode));
    } else {
        change_mask(e, shape);
    }
}

pub fn select_rect(e: &mut Editor, a: (i32, i32), b: (i32, i32)) {
    let m = Selection::rect(e.sprite.width, e.sprite.height, a, b);
    apply_shape(e, m);
}

/// Selects the area enclosed by a freehand or polygon path of pixel coordinates.
pub fn select_polygon(e: &mut Editor, pts: &[(i32, i32)]) {
    let m = Selection::polygon(e.sprite.width, e.sprite.height, pts);
    apply_shape(e, m);
}

pub fn select_all(e: &mut Editor) {
    change_mask(e, Selection::new(e.sprite.width, e.sprite.height, true));
}

pub fn deselect(e: &mut Editor) {
    change_mask(e, Selection::new(e.sprite.width, e.sprite.height, false));
}

pub fn invert(e: &mut Editor) {
    let (w, h) = (e.sprite.width, e.sprite.height);
    let mut m = e.sel.active(w, h).cloned().unwrap_or_else(|| Selection::new(w, h, false));
    m.bits.iter_mut().for_each(|b| *b = !*b);
    change_mask(e, m);
}

/// Magic wand: selects pixels of the current cel whose color is within `tolerance` (per channel,
/// 0-255) of the pixel at (x, y); transparent pixels all match each other. `contiguous` limits the
/// result to the 4-connected region around (x, y). With `contiguous` off this is "select by color".
#[allow(clippy::too_many_arguments)]
pub fn wand(
    e: &mut Editor,
    layer: usize,
    frame: usize,
    x: i32,
    y: i32,
    tolerance: u8,
    contiguous: bool,
    mode: Mode,
) -> bool {
    let (w, h) = (e.sprite.width, e.sprite.height);
    if !drawable(&e.sprite, layer, frame) || x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        return false;
    }
    let px = cel_pixels(&e.sprite, layer, frame);
    let at = |x: i32, y: i32| -> [u8; 4] { px[px_index(w, x, y)..][..4].try_into().unwrap() };
    let target = at(x, y);
    let matches =
        |c: [u8; 4]| (c[3] == 0 && target[3] == 0) || c.iter().zip(&target).all(|(a, b)| a.abs_diff(*b) <= tolerance);
    let mut shape = Selection::new(w, h, false);
    if contiguous {
        let mut stack = vec![(x, y)];
        while let Some((x, y)) = stack.pop() {
            if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                continue;
            }
            let i = y as usize * w as usize + x as usize;
            if shape.bits[i] || !matches(at(x, y)) {
                continue;
            }
            shape.bits[i] = true;
            stack.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
        }
    } else {
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                shape.bits[y as usize * w as usize + x as usize] = matches(at(x, y));
            }
        }
    }
    let combined = combine(e.sel.active(w, h), shape, mode);
    change_mask(e, combined);
    true
}

#[derive(Clone, Copy, Debug)]
pub enum Modify {
    Expand,
    Contract,
    Border,
}

/// Grows or shrinks the selection by `n` pixels (square neighborhood), or turns it into a border.
/// A border of width `n` straddles the old edge. Contracting also shrinks from the canvas edge.
pub fn modify(e: &mut Editor, kind: Modify, n: u32) -> bool {
    let (w, h) = (e.sprite.width, e.sprite.height);
    let Some(sel) = e.sel.active(w, h) else { return false };
    let n = n.min(1024) as i32;
    let result = match kind {
        Modify::Expand => sel.dilated(n),
        Modify::Contract => sel.eroded(n),
        Modify::Border => {
            let (outer, inner) = (sel.dilated((n + 1) / 2), sel.eroded(n / 2));
            Selection { bits: outer.bits.iter().zip(&inner.bits).map(|(o, i)| *o && !*i).collect(), ..outer }
        }
    };
    change_mask(e, result)
}

pub(crate) fn px_index(w: u32, x: i32, y: i32) -> usize {
    (y as usize * w as usize + x as usize) * 4
}

pub(crate) fn drawable(s: &Sprite, layer: usize, frame: usize) -> bool {
    frame < s.frame_count() && s.layers.get(layer).is_some_and(|l| !l.is_group())
}

// MARK: Clipped drawing

/// Like `Sprite::draw_line`, but only pixels inside the selection (if any) change.
pub fn draw_line(e: &mut Editor, layer: usize, frame: usize, a: (i32, i32), b: (i32, i32), color: [u8; 4], size: u32) {
    let (w, h) = (e.sprite.width, e.sprite.height);
    let Some(sel) = e.sel.active(w, h) else {
        return e.sprite.draw_line(layer, frame, a.0, a.1, b.0, b.1, color, size);
    };
    if !drawable(&e.sprite, layer, frame) {
        return;
    }
    let pad = size.clamp(1, 64) as i32;
    let (rx0, rx1) = ((a.0.min(b.0) - pad).max(0), (a.0.max(b.0) + pad).min(w as i32 - 1));
    let (ry0, ry1) = ((a.1.min(b.1) - pad).max(0), (a.1.max(b.1) + pad).min(h as i32 - 1));
    if rx0 > rx1 || ry0 > ry1 {
        return;
    }
    let old = e.sprite.cels[frame][layer].clone();
    let saved = |x: i32, y: i32| -> [u8; 4] {
        old.as_ref().map_or([0; 4], |c| c.pixels[px_index(w, x, y)..][..4].try_into().unwrap())
    };
    let before: Vec<[u8; 4]> =
        (ry0..=ry1).flat_map(|y| (rx0..=rx1).map(move |x| (x, y))).map(|(x, y)| saved(x, y)).collect();
    drop(old);
    e.sprite.draw_line(layer, frame, a.0, a.1, b.0, b.1, color, size);
    let Some(cel) = e.sprite.cel_mut(layer, frame) else { return };
    let mut it = before.into_iter();
    for y in ry0..=ry1 {
        for x in rx0..=rx1 {
            let prev = it.next().unwrap();
            if !sel.get(x, y) {
                cel.pixels[px_index(w, x, y)..][..4].copy_from_slice(&prev);
            }
        }
    }
    sync_tiles(e, layer, frame);
}

/// Like `Sprite::fill`, but the fill cannot leave the selection (if any).
pub fn fill(e: &mut Editor, layer: usize, frame: usize, x: i32, y: i32, color: [u8; 4]) {
    let (w, h) = (e.sprite.width, e.sprite.height);
    let Some(sel) = e.sel.active(w, h) else {
        return e.sprite.fill(layer, frame, x, y, color);
    };
    if !sel.get(x, y) {
        return;
    }
    let Some(cel) = e.sprite.cel_mut(layer, frame) else { return };
    let px = &mut cel.pixels;
    let get = |px: &[u8], x: i32, y: i32| -> [u8; 4] { px[px_index(w, x, y)..][..4].try_into().unwrap() };
    let same = |a: [u8; 4], b: [u8; 4]| a == b || (a[3] == 0 && b[3] == 0);
    let target = get(px, x, y);
    if same(target, color) {
        return;
    }
    let mut stack = vec![(x, y)];
    while let Some((x, y)) = stack.pop() {
        if !sel.get(x, y) || !same(get(px, x, y), target) {
            continue;
        }
        px[px_index(w, x, y)..][..4].copy_from_slice(&color);
        stack.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
    }
    sync_tiles(e, layer, frame);
}

// MARK: Move

pub(crate) fn cel_pixels(s: &Sprite, layer: usize, frame: usize) -> Vec<u8> {
    match &s.cels[frame][layer] {
        Some(c) => c.pixels.clone(),
        None => vec![0; s.width as usize * s.height as usize * 4],
    }
}

/// Starts an undoable move of the selected pixels (or the whole cel) on one layer.
/// Restores pixels outside the selection to their state at the last checkpoint, so tools that
/// draw without knowing about the selection still respect it, and writes tilemap edits back to
/// the tiles. Call it after every edit made with `Sprite::cel_mut`.
pub fn clip_to_selection(e: &mut Editor, layer: usize, frame: usize) {
    clip_pixels(e, layer, frame);
    // Every drawing tool ends here, so this is also where inks that blend (alpha, behind) and
    // gradients get snapped back onto the palette of Indexed and Grayscale sprites.
    crate::document::color_mode::conform_cel(&mut e.sprite, layer, frame);
    sync_tiles(e, layer, frame);
}

fn clip_pixels(e: &mut Editor, layer: usize, frame: usize) {
    let (w, h) = (e.sprite.width as usize, e.sprite.height as usize);
    let Some(mask) = e.sel.active(w as u32, h as u32).cloned() else { return };
    // The checkpoint may predate a canvas resize (inside a transaction): then there is nothing to restore from.
    let before = e.checkpoint_cel(frame, layer).filter(|c| c.pixels.len() == w * h * 4);
    let Some(cel) = e.sprite.cel_mut(layer, frame) else { return };
    for y in 0..h {
        for x in 0..w {
            if !mask.get(x as i32, y as i32) {
                let i = (y * w + x) * 4;
                let old = before.as_ref().map_or([0; 4], |c| c.pixels[i..i + 4].try_into().unwrap());
                cel.pixels[i..i + 4].copy_from_slice(&old);
            }
        }
    }
}

pub fn move_begin(e: &mut Editor, layer: usize, frame: usize) -> bool {
    if !drawable(&e.sprite, layer, frame) {
        return false;
    }
    match &e.sel.float {
        Some(f) if (f.layer, f.frame) == (layer, frame) => {
            e.sel.drag = Some(Drag::Float { x: f.x, y: f.y });
            return true;
        }
        _ => {}
    }
    e.checkpoint();
    let (w, h) = (e.sprite.width, e.sprite.height);
    let sel = e.sel.active(w, h).cloned();
    e.sel.drag = Some(Drag::Pixels {
        layer,
        frame,
        base: cel_pixels(&e.sprite, layer, frame),
        base_outside: e.sprite.cels[frame][layer].as_ref().and_then(|c| c.outside.clone()),
        had_selection: sel.is_some(),
        mask: sel.unwrap_or_else(|| Selection::new(w, h, true)),
    });
    true
}

/// Places the moved pixels at total offset (dx, dy) from where the move began.
pub fn move_to(e: &mut Editor, dx: i32, dy: i32) {
    let Some(drag) = e.sel.drag.take() else { return };
    let Drag::Pixels { layer, frame, ref base, ref base_outside, ref mask, had_selection } = drag else {
        if let Drag::Float { x, y } = drag {
            float_move(e, x + dx, y + dy);
        }
        e.sel.drag = Some(drag);
        return;
    };
    let (w, h) = (mask.w, mask.h);
    // The drag was begun on a canvas of another size (the sprite was replaced meanwhile).
    if (w, h) != (e.sprite.width, e.sprite.height) {
        e.sel.drag = None;
        return;
    }
    let tilemap = e.sprite.layers[layer].kind == crate::document::tilemap::KIND_TILEMAP;
    if !had_selection && !tilemap {
        // Moving the whole cel: its content keeps its place relative to the cel, so what leaves the
        // canvas is kept and what comes back is revealed (Aseprite behaviour).
        if let Some(cel) = e.sprite.cel_mut(layer, frame) {
            cel.pixels.clone_from(base);
            cel.outside = base_outside.clone();
            // Content that would reach too far for the file format is not moved.
            let _ = cel.translate(w as usize, h as usize, dx, dy);
        }
        e.sel.drag = Some(drag);
        return;
    }
    if let Some(cel) = e.sprite.cel_mut(layer, frame) {
        let mut px = base.clone();
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                if mask.get(x, y) {
                    px[px_index(w, x, y)..][..4].fill(0);
                }
            }
        }
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let (nx, ny) = (x + dx, y + dy);
                if mask.get(x, y) && nx >= 0 && ny >= 0 && nx < w as i32 && ny < h as i32 {
                    let i = px_index(w, nx, ny);
                    blend_over(&mut px[i..i + 4], &base[px_index(w, x, y)..][..4], 255);
                }
            }
        }
        cel.pixels = px;
        if had_selection {
            e.sel.mask = nonempty(mask.shifted(dx, dy));
        }
    }
    e.sel.drag = Some(drag);
}

pub fn move_end(e: &mut Editor) {
    if let Some(Drag::Pixels { layer, frame, .. }) = e.sel.drag.take() {
        sync_tiles(e, layer, frame);
    }
}

pub fn nudge(e: &mut Editor, layer: usize, frame: usize, dx: i32, dy: i32) -> bool {
    let ok = move_begin(e, layer, frame);
    if ok {
        move_to(e, dx, dy);
        move_end(e);
    }
    ok
}

// MARK: Clipboard

/// Clears the selected pixels (or the whole cel) of a layer.
pub fn delete(e: &mut Editor, layer: usize, frame: usize) -> bool {
    if !drawable(&e.sprite, layer, frame) || e.sprite.cels[frame][layer].is_none() {
        return false;
    }
    let (w, h) = (e.sprite.width, e.sprite.height);
    // Nothing visible in the region: no undo step.
    let anything = e.sprite.cels[frame][layer].as_ref().is_some_and(|c| {
        let sel = e.sel.active(w, h);
        // Without a selection the whole cel is cleared, including what lies beyond the canvas.
        (sel.is_none() && c.outside.is_some())
            || (0..h as i32).any(|y| {
                (0..w as i32).any(|x| sel.is_none_or(|s| s.get(x, y)) && c.pixels[px_index(w, x, y)..][..4] != [0; 4])
            })
    });
    if !anything {
        return false;
    }
    e.checkpoint();
    let sel = e.sel.active(w, h);
    let whole = sel.is_none();
    let Some(cel) = e.sprite.cel_mut(layer, frame) else { return false };
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if sel.is_none_or(|s| s.get(x, y)) {
                cel.pixels[px_index(w, x, y)..][..4].fill(0);
            }
        }
    }
    if whole {
        cel.outside = None;
    }
    sync_tiles(e, layer, frame);
    true
}

/// Bounding box (x, y, w, h) of the selection, or of the canvas without one.
pub fn copy_bounds(e: &Editor) -> (i32, i32, i32, i32) {
    let (w, h) = (e.sprite.width, e.sprite.height);
    e.sel.active(w, h).and_then(|s| s.bounds()).unwrap_or((0, 0, w as i32, h as i32))
}

/// RGBA pixels of `copy_bounds`; unselected pixels are transparent.
pub fn copy(e: &Editor, layer: usize, frame: usize) -> Option<Vec<u8>> {
    if !drawable(&e.sprite, layer, frame) {
        return None;
    }
    let (w, h) = (e.sprite.width, e.sprite.height);
    let sel = e.sel.active(w, h);
    let (bx, by, bw, bh) = copy_bounds(e);
    let cel = e.sprite.cels[frame][layer].as_ref();
    let mut out = vec![0u8; bw as usize * bh as usize * 4];
    if let Some(cel) = cel {
        for y in 0..bh {
            for x in 0..bw {
                if sel.is_none_or(|s| s.get(bx + x, by + y)) {
                    let d = (y * bw + x) as usize * 4;
                    out[d..d + 4].copy_from_slice(&cel.pixels[px_index(w, bx + x, by + y)..][..4]);
                }
            }
        }
    }
    Some(out)
}

/// Pastes an RGBA image onto a layer at (x, y) as a floating selection: it stays repositionable
/// (`move_begin` / `move_to`) until the next edit, `float_commit`, or `float_cancel`.
#[allow(clippy::too_many_arguments)]
pub fn paste(e: &mut Editor, layer: usize, frame: usize, iw: i32, ih: i32, data: &[u8], x: i32, y: i32) -> bool {
    if iw <= 0 || ih <= 0 || data.len() != iw as usize * ih as usize * 4 || !drawable(&e.sprite, layer, frame) {
        return false;
    }
    e.checkpoint();
    e.sel.float = Some(Float {
        layer,
        frame,
        base: cel_pixels(&e.sprite, layer, frame),
        w: iw,
        h: ih,
        data: data.to_vec(),
        x,
        y,
    });
    float_move(e, x, y);
    true
}

/// Re-composites the floating pixels at (x, y) over the cel as it was before the paste.
fn float_move(e: &mut Editor, x: i32, y: i32) {
    let Some(mut f) = e.sel.float.take() else { return };
    let (w, h) = (e.sprite.width, e.sprite.height);
    if f.base.len() != w as usize * h as usize * 4 {
        // The canvas changed under the paste (inside a transaction): drop the float.
        return;
    }
    f.x = x;
    f.y = y;
    if f.base.len() != w as usize * h as usize * 4 {
        return; // the canvas changed size under the float: drop it, the pixels stay as they are
    }
    if let Some(cel) = e.sprite.cel_mut(f.layer, f.frame) {
        cel.pixels.clone_from(&f.base);
        for sy in 0..f.h {
            for sx in 0..f.w {
                let (tx, ty) = (x + sx, y + sy);
                if tx >= 0 && ty >= 0 && tx < w as i32 && ty < h as i32 {
                    let i = px_index(w, tx, ty);
                    blend_over(&mut cel.pixels[i..i + 4], &f.data[(sy * f.w + sx) as usize * 4..][..4], 255);
                }
            }
        }
    }
    crate::document::color_mode::conform_cel(&mut e.sprite, f.layer, f.frame);
    set_mask(e, Selection::rect(w, h, (x, y), (x + f.w - 1, y + f.h - 1)));
    e.sel.float = Some(f);
}

pub fn float_active(e: &Editor) -> bool {
    e.sel.float.is_some()
}

/// Drops the floating state, leaving the pasted pixels in the cel.
pub fn float_commit(e: &mut Editor) {
    if let Some(f) = e.sel.float.take() {
        sync_tiles(e, f.layer, f.frame);
    }
}

/// Removes a floating paste entirely, as if it had never been pasted.
pub fn float_cancel(e: &mut Editor) -> bool {
    e.sel.float.is_some() && e.rollback()
}

// MARK: Transform

#[derive(Clone, Copy, Debug)]
pub enum Op {
    FlipH,
    FlipV,
    RotateCw,
    RotateCcw,
}

/// Flips or rotates the selected pixels (or the whole cel). Rotation keeps the region's center.
pub fn transform(e: &mut Editor, layer: usize, frame: usize, op: Op) -> bool {
    if !drawable(&e.sprite, layer, frame) || e.sprite.cels[frame][layer].is_none() {
        return false;
    }
    e.checkpoint();
    let (w, h) = (e.sprite.width, e.sprite.height);
    let sel = e.sel.active(w, h).cloned();
    let (bx, by, bw, bh) = sel.as_ref().and_then(|s| s.bounds()).unwrap_or((0, 0, w as i32, h as i32));
    if sel.is_none() && e.sprite.cels[frame][layer].as_ref().is_some_and(|c| c.outside.is_some()) {
        return transform_whole_cel(e, layer, frame, op);
    }
    let Some(cel) = e.sprite.cel_mut(layer, frame) else { return false };

    let (bwu, bhu) = (bw as usize, bh as usize);
    let mut lift = vec![0u8; bwu * bhu * 4];
    let mut lmask = vec![false; bwu * bhu];
    for y in 0..bh {
        for x in 0..bw {
            if sel.as_ref().is_none_or(|s| s.get(bx + x, by + y)) {
                let (i, c) = ((y * bw + x) as usize, px_index(w, bx + x, by + y));
                lmask[i] = true;
                lift[i * 4..i * 4 + 4].copy_from_slice(&cel.pixels[c..c + 4]);
                cel.pixels[c..c + 4].fill(0);
            }
        }
    }

    let rotate = matches!(op, Op::RotateCw | Op::RotateCcw);
    let (nw, nh) = if rotate { (bh, bw) } else { (bw, bh) };
    let (nx, ny) = if rotate { (bx + (bw - bh) / 2, by + (bh - bw) / 2) } else { (bx, by) };
    let mut new_sel = Selection::new(w, h, false);
    for y in 0..bh {
        for x in 0..bw {
            let i = (y * bw + x) as usize;
            if !lmask[i] {
                continue;
            }
            let (dx, dy) = match op {
                Op::FlipH => (bw - 1 - x, y),
                Op::FlipV => (x, bh - 1 - y),
                Op::RotateCw => (bh - 1 - y, x),
                Op::RotateCcw => (y, bw - 1 - x),
            };
            debug_assert!(dx < nw && dy < nh);
            let (tx, ty) = (nx + dx, ny + dy);
            if tx >= 0 && ty >= 0 && tx < w as i32 && ty < h as i32 {
                let c = px_index(w, tx, ty);
                blend_over(&mut cel.pixels[c..c + 4], &lift[i * 4..i * 4 + 4], 255);
                new_sel.bits[ty as usize * w as usize + tx as usize] = true;
            }
        }
    }
    if sel.is_some() {
        set_mask(e, new_sel);
    }
    sync_tiles(e, layer, frame);
    true
}

/// `transform` of a whole cel that has content beyond the canvas: the full content is flipped or
/// rotated about the canvas, like the visible part.
fn transform_whole_cel(e: &mut Editor, layer: usize, frame: usize, op: Op) -> bool {
    let (w, h) = (e.sprite.width as usize, e.sprite.height as usize);
    let (bw, bh) = (w as i64, h as i64);
    let rotate = matches!(op, Op::RotateCw | Op::RotateCcw);
    let (nx, ny) = if rotate { ((bw - bh) / 2, (bh - bw) / 2) } else { (0, 0) };
    let map = |x: i64, y: i64| -> (i64, i64) {
        let (dx, dy) = match op {
            Op::FlipH => (bw - 1 - x, y),
            Op::FlipV => (x, bh - 1 - y),
            Op::RotateCw => (bh - 1 - y, x),
            Op::RotateCcw => (y, bw - 1 - x),
        };
        (nx + dx, ny + dy)
    };
    let Some(cel) = e.sprite.cel_mut(layer, frame) else { return false };
    let Some(full) = cel.full_image(w, h) else { return true };
    let (x0, y0) = (full.x as i64, full.y as i64);
    let (x1, y1) = (x0 + full.w as i64 - 1, y0 + full.h as i64 - 1);
    let (a, b) = (map(x0, y0), map(x1, y1));
    let (rx, ry) = (a.0.min(b.0), a.1.min(b.1));
    let (rw, rh) = ((a.0 - b.0).unsigned_abs() as usize + 1, (a.1 - b.1).unsigned_abs() as usize + 1);
    let (Ok(rx32), Ok(ry32)) = (i32::try_from(rx), i32::try_from(ry)) else { return false };
    let mut out = vec![0u8; rw * rh * 4];
    for sy in 0..full.h {
        for sx in 0..full.w {
            let (tx, ty) = map(x0 + sx as i64, y0 + sy as i64);
            let d = ((ty - ry) as usize * rw + (tx - rx) as usize) * 4;
            out[d..d + 4].copy_from_slice(&full.rgba[(sy * full.w + sx) * 4..][..4]);
        }
    }
    cel.set_full(w, h, (rx32, ry32, rw, rh), &out).is_ok()
}

// MARK: Canvas size

/// Resizes the canvas of every frame and layer; old content lands at (ox, oy) in the new canvas.
pub fn resize_canvas(e: &mut Editor, nw: i32, nh: i32, ox: i32, oy: i32) -> bool {
    let (nw, nh) = (nw.clamp(1, MAX_DIM), nh.clamp(1, MAX_DIM));
    // Offsets beyond this crop everything anyway; clamping keeps the arithmetic below in range.
    let (ox, oy) = (ox.clamp(-(1 << 20), 1 << 20), oy.clamp(-(1 << 20), 1 << 20));
    let (w, h) = (e.sprite.width as i32, e.sprite.height as i32);
    if (nw, nh, ox, oy) == (w, h, 0, 0) {
        return false;
    }
    let area = nw as usize * nh as usize;
    if area > crate::document::MAX_CANVAS_PIXELS || !e.sprite.cels_fit_budget(nw as u64, nh as u64) {
        crate::ffi::set_error(format!(
            "the canvas would need too much memory: {}",
            e.sprite.cel_budget_note(nw as u64, nh as u64)
        ));
        return false;
    }
    e.checkpoint();
    let mut done: std::collections::HashMap<*const Cel, Arc<Cel>> = std::collections::HashMap::new();
    // Isometric tilemaps are addressed in cells: their layer offset moves with the content, not their grid.
    let iso_layers = crate::document::tilemap::iso_layers(&e.sprite);
    for row in e.sprite.cels.iter_mut() {
        for (layer, cel) in row.iter_mut().enumerate() {
            let Some(cel) = cel else { continue };
            // Linked cels share one buffer; convert it once so they stay linked.
            if let Some(moved) = done.get(&Arc::as_ptr(cel)) {
                *cel = moved.clone();
                continue;
            }
            let key = Arc::as_ptr(cel);
            let mut px = vec![0u8; nw as usize * nh as usize * 4];
            let (x0, x1) = (ox.max(0), (ox + w).min(nw));
            // The full content (also what lies beyond the old canvas) is placed on the new canvas.
            let full = if cel.pixels.len() == w as usize * h as usize * 4 && cel.tiles.is_none() {
                // Content that was cropped away by this resize is kept beyond the new edges, and content
                // that was beyond the old edges can come into view.
                let full = cel.full_image(w as usize, h as usize);
                full.filter(|f| {
                    cel.outside.is_some()
                        || f.x as i64 + ox as i64 + f.w as i64 > nw as i64
                        || f.y as i64 + oy as i64 + f.h as i64 > nh as i64
                        || f.x as i64 + (ox as i64) < 0
                        || f.y as i64 + (oy as i64) < 0
                })
            } else {
                None
            };
            for y in oy.max(0)..(oy + h).min(nh) {
                if x0 < x1 && cel.pixels.len() == w as usize * h as usize * 4 {
                    let src = px_index(w as u32, x0 - ox, y - oy);
                    let dst = px_index(nw as u32, x0, y);
                    let n = (x1 - x0) as usize * 4;
                    px[dst..dst + n].copy_from_slice(&cel.pixels[src..src + n]);
                }
            }
            // Tile grids move with the content; their pixels are re-rendered below.
            let tiles = if iso_layers.get(layer).copied().unwrap_or(false) {
                cel.tiles.clone()
            } else {
                crate::document::tilemap::shifted(cel.tiles.as_ref(), ox, oy)
            };
            let mut moved = Cel {
                opacity: cel.opacity,
                pixels: px,
                tiles,
                user_data: cel.user_data.clone(),
                z_index: cel.z_index,
                ..Default::default()
            };
            if let Some(f) = full {
                // Fails only for content that would lie too far away for the file format, which is then dropped.
                let _ = moved.set_full(nw as usize, nh as usize, (f.x + ox, f.y + oy, f.w, f.h), &f.rgba);
            }
            let moved = Arc::new(moved);
            done.insert(key, moved.clone());
            *cel = moved;
        }
    }
    crate::document::tilemap::shift_iso_offsets(&mut e.sprite, ox, oy);
    e.sprite.extra.shifted(ox, oy);
    e.sprite.width = nw as u32;
    e.sprite.height = nh as u32;
    crate::document::tilemap::refresh_all(&mut e.sprite);
    e.sel.mask = None;
    e.sel.reset_interaction();
    true
}

pub fn crop_to_selection(e: &mut Editor) -> bool {
    let (w, h) = (e.sprite.width, e.sprite.height);
    let Some((x, y, bw, bh)) = e.sel.active(w, h).and_then(|s| s.bounds()) else { return false };
    resize_canvas(e, bw, bh, -x, -y)
}

// MARK: C ABI

unsafe fn ed<'a>(doc: *mut Editor) -> Option<&'a mut Editor> {
    let e = doc.as_mut()?;
    e.dirty.mark_all();
    Some(e)
}

unsafe fn ed_ref<'a>(doc: *const Editor) -> Option<&'a Editor> {
    doc.as_ref()
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_active(doc: *const Editor) -> bool {
    crate::ffi::guarded("pixmith_sel_active", std::ptr::null_mut(), || {
        ed_ref(doc).is_some_and(|e| e.sel.active(e.sprite.width, e.sprite.height).is_some())
    })
}

/// Writes the selection's bounding box (x, y, w, h) into `out[4]`; false when nothing is selected.
#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_bounds(doc: *const Editor, out: *mut i32) -> bool {
    crate::ffi::guarded("pixmith_sel_bounds", std::ptr::null_mut(), || {
        let Some(e) = ed_ref(doc) else { return false };
        let Some(b) = e.sel.active(e.sprite.width, e.sprite.height).and_then(|s| s.bounds()) else { return false };
        if !out.is_null() {
            std::slice::from_raw_parts_mut(out, 4).copy_from_slice(&[b.0, b.1, b.2, b.3]);
        }
        true
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_contains(doc: *const Editor, x: i32, y: i32) -> bool {
    crate::ffi::guarded("pixmith_sel_contains", std::ptr::null_mut(), || {
        ed_ref(doc).and_then(|e| e.sel.active(e.sprite.width, e.sprite.height)).is_some_and(|s| s.get(x, y))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_rect(doc: *mut Editor, x0: i32, y0: i32, x1: i32, y1: i32) {
    crate::ffi::guarded("pixmith_sel_rect", doc, || {
        if let Some(e) = ed(doc) {
            let c = crate::ffi::clampc;
            select_rect(e, (c(x0), c(y0)), (c(x1), c(y1)));
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_all(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_sel_all", doc, || {
        if let Some(e) = ed(doc) {
            select_all(e);
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_clear(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_sel_clear", doc, || {
        if let Some(e) = ed(doc) {
            deselect(e);
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_invert(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_sel_invert", doc, || {
        if let Some(e) = ed(doc) {
            invert(e);
        }
    })
}

/// Returns the number of outline segments; writes up to `cap` of them (4 ints each) into `out`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_outline(doc: *const Editor, out: *mut i32, cap: usize) -> usize {
    crate::ffi::guarded("pixmith_sel_outline", std::ptr::null_mut(), || {
        let Some(e) = ed_ref(doc) else { return 0 };
        let Some(s) = e.sel.active(e.sprite.width, e.sprite.height) else { return 0 };
        let segs = s.outline();
        if !out.is_null() {
            for (i, seg) in segs.iter().take(cap).enumerate() {
                std::slice::from_raw_parts_mut(out.add(i * 4), 4).copy_from_slice(seg);
            }
        }
        segs.len()
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_move_begin(doc: *mut Editor, layer: u32, frame: u32) -> bool {
    crate::ffi::guarded("pixmith_sel_move_begin", doc, || {
        ed(doc).is_some_and(|e| move_begin(e, layer as usize, frame as usize))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_move_to(doc: *mut Editor, dx: i32, dy: i32) {
    crate::ffi::guarded("pixmith_sel_move_to", doc, || {
        if let Some(e) = ed(doc) {
            move_to(e, crate::ffi::clampc(dx), crate::ffi::clampc(dy));
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_move_end(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_sel_move_end", doc, || {
        if let Some(e) = ed(doc) {
            move_end(e);
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_nudge(doc: *mut Editor, layer: u32, frame: u32, dx: i32, dy: i32) -> bool {
    crate::ffi::guarded("pixmith_sel_nudge", doc, || {
        ed(doc)
            .is_some_and(|e| nudge(e, layer as usize, frame as usize, crate::ffi::clampc(dx), crate::ffi::clampc(dy)))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_delete(doc: *mut Editor, layer: u32, frame: u32) -> bool {
    crate::ffi::guarded("pixmith_sel_delete", doc, || {
        ed(doc).is_some_and(|e| delete(e, layer as usize, frame as usize))
    })
}

/// Writes the copy region (x, y, w, h) into `out[4]`: the selection bounds, or the canvas.
#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_copy_bounds(doc: *const Editor, out: *mut i32) {
    crate::ffi::guarded("pixmith_sel_copy_bounds", std::ptr::null_mut(), || {
        if let (Some(e), false) = (ed_ref(doc), out.is_null()) {
            let b = copy_bounds(e);
            std::slice::from_raw_parts_mut(out, 4).copy_from_slice(&[b.0, b.1, b.2, b.3]);
        }
    })
}

/// Copies the region of `pixmith_sel_copy_bounds` as RGBA into `out`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_copy(
    doc: *const Editor,
    layer: u32,
    frame: u32,
    out: *mut u8,
    len: usize,
) -> bool {
    crate::ffi::guarded("pixmith_sel_copy", std::ptr::null_mut(), || {
        let Some(px) = ed_ref(doc).and_then(|e| copy(e, layer as usize, frame as usize)) else { return false };
        if out.is_null() || len < px.len() {
            return false;
        }
        std::ptr::copy_nonoverlapping(px.as_ptr(), out, px.len());
        true
    })
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_sel_paste(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    w: u32,
    h: u32,
    data: *const u8,
    x: i32,
    y: i32,
) -> bool {
    crate::ffi::guarded("pixmith_sel_paste", doc, || {
        let Some(e) = ed(doc) else { return false };
        if w > u16::MAX as u32 || h > u16::MAX as u32 {
            return false;
        }
        let (x, y) = (crate::ffi::clampc(x), crate::ffi::clampc(y));
        let (Some(len), false) = ((w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(4)), data.is_null())
        else {
            return false;
        };
        paste(e, layer as usize, frame as usize, w as i32, h as i32, std::slice::from_raw_parts(data, len), x, y)
    })
}

/// `op`: 0 flip horizontal, 1 flip vertical, 2 rotate clockwise, 3 rotate counter-clockwise.
#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_transform(doc: *mut Editor, layer: u32, frame: u32, op: u32) -> bool {
    crate::ffi::guarded("pixmith_sel_transform", doc, || {
        let op = match op {
            0 => Op::FlipH,
            1 => Op::FlipV,
            2 => Op::RotateCw,
            3 => Op::RotateCcw,
            _ => return false,
        };
        ed(doc).is_some_and(|e| transform(e, layer as usize, frame as usize, op))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_resize_canvas(doc: *mut Editor, w: u32, h: u32, ox: i32, oy: i32) -> bool {
    crate::ffi::guarded("pixmith_resize_canvas", doc, || {
        ed(doc).is_some_and(|e| {
            resize_canvas(
                e,
                w.min(i32::MAX as u32) as i32,
                h.min(i32::MAX as u32) as i32,
                crate::ffi::clampc(ox),
                crate::ffi::clampc(oy),
            )
        })
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_crop_to_selection(doc: *mut Editor) -> bool {
    crate::ffi::guarded("pixmith_crop_to_selection", doc, || ed(doc).is_some_and(crop_to_selection))
}

/// Starts an interactive selection (one undo step). `mode`: 0 replace, 1 add, 2 subtract.
#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_begin(doc: *mut Editor, mode: u32) {
    crate::ffi::guarded("pixmith_sel_begin", doc, || {
        if let Some(e) = ed(doc) {
            begin_session(e, Mode::from_u32(mode));
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_end(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_sel_end", doc, || {
        if let Some(e) = ed(doc) {
            end_session(e);
        }
    })
}

/// Selects the area inside a closed path of `n` points (x, y pairs in `pts`).
#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_poly(doc: *mut Editor, pts: *const i32, n: usize) {
    crate::ffi::guarded("pixmith_sel_poly", doc, || {
        let (Some(e), false) = (ed(doc), pts.is_null()) else { return };
        if n > MAX_POLY_POINTS {
            return;
        }
        let pts: Vec<(i32, i32)> = std::slice::from_raw_parts(pts, n * 2)
            .chunks_exact(2)
            .map(|p| (crate::ffi::clampc(p[0]), crate::ffi::clampc(p[1])))
            .collect();
        select_polygon(e, &pts);
    })
}

/// Magic wand on one cel; `contiguous` false selects every matching pixel (select by color).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_sel_wand(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    tolerance: u32,
    contiguous: bool,
    mode: u32,
) -> bool {
    crate::ffi::guarded("pixmith_sel_wand", doc, || {
        ed(doc).is_some_and(|e| {
            wand(e, layer as usize, frame as usize, x, y, tolerance.min(255) as u8, contiguous, Mode::from_u32(mode))
        })
    })
}

/// `kind`: 0 expand, 1 contract, 2 border.
#[no_mangle]
pub unsafe extern "C" fn pixmith_sel_modify(doc: *mut Editor, kind: u32, n: u32) -> bool {
    crate::ffi::guarded("pixmith_sel_modify", doc, || {
        let kind = match kind {
            0 => Modify::Expand,
            1 => Modify::Contract,
            2 => Modify::Border,
            _ => return false,
        };
        ed(doc).is_some_and(|e| modify(e, kind, n))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_float_active(doc: *const Editor) -> bool {
    crate::ffi::guarded("pixmith_float_active", std::ptr::null_mut(), || ed_ref(doc).is_some_and(float_active))
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_float_commit(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_float_commit", doc, || {
        if let Some(e) = ed(doc) {
            float_commit(e);
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_float_cancel(doc: *mut Editor) -> bool {
    crate::ffi::guarded("pixmith_float_cancel", doc, || ed(doc).is_some_and(float_cancel))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn px(e: &Editor, x: i32, y: i32) -> [u8; 4] {
        let c = e.sprite.cels[0][0]
            .as_ref()
            .map_or([0; 4], |c| c.pixels[px_index(e.sprite.width, x, y)..][..4].try_into().unwrap());
        c
    }

    fn editor(w: u32, h: u32) -> Editor {
        Editor::new(Sprite::new(w, h))
    }

    fn dot(e: &mut Editor, x: i32, y: i32, c: [u8; 4]) {
        e.sprite.draw_line(0, 0, x, y, x, y, c, 1);
    }

    #[test]
    fn rect_bounds_and_invert() {
        let mut e = editor(4, 4);
        select_rect(&mut e, (3, 2), (1, 1));
        assert_eq!(e.sel.active(4, 4).unwrap().bounds(), Some((1, 1, 3, 2)));
        invert(&mut e);
        let s = e.sel.active(4, 4).unwrap();
        assert!(s.get(0, 0) && !s.get(1, 1));
        assert_eq!(s.bits.iter().filter(|b| **b).count(), 16 - 6);
        select_all(&mut e);
        invert(&mut e);
        assert!(e.sel.active(4, 4).is_none());
    }

    #[test]
    fn outline_of_rect_is_four_segments() {
        let mut e = editor(8, 8);
        select_rect(&mut e, (1, 1), (3, 2));
        let mut o = e.sel.active(8, 8).unwrap().outline();
        o.sort();
        assert_eq!(o, vec![[1, 1, 1, 3], [1, 1, 4, 1], [1, 3, 4, 3], [4, 1, 4, 3]]);
    }

    #[test]
    fn drawing_is_clipped_to_selection() {
        let mut e = editor(8, 1);
        select_rect(&mut e, (2, 0), (4, 0));
        draw_line(&mut e, 0, 0, (0, 0), (7, 0), RED, 1);
        let got: Vec<bool> = (0..8).map(|x| px(&e, x, 0) == RED).collect();
        assert_eq!(got, vec![false, false, true, true, true, false, false, false]);
        draw_line(&mut e, 0, 0, (0, 0), (7, 0), [0; 4], 1);
        assert!((0..8).all(|x| px(&e, x, 0)[3] == 0 || !e.sel.active(8, 1).unwrap().get(x, 0)));
        assert!((2..=4).all(|x| px(&e, x, 0)[3] == 0));
    }

    #[test]
    fn fill_stays_inside_selection() {
        let mut e = editor(6, 1);
        select_rect(&mut e, (1, 0), (3, 0));
        fill(&mut e, 0, 0, 0, 0, RED);
        assert_eq!(px(&e, 0, 0)[3], 0);
        fill(&mut e, 0, 0, 2, 0, BLUE);
        let got: Vec<bool> = (0..6).map(|x| px(&e, x, 0) == BLUE).collect();
        assert_eq!(got, vec![false, true, true, true, false, false]);
    }

    #[test]
    fn move_selection_is_one_undo_step() {
        let mut e = editor(6, 1);
        dot(&mut e, 1, 0, RED);
        dot(&mut e, 5, 0, BLUE);
        select_rect(&mut e, (0, 0), (2, 0));
        assert!(move_begin(&mut e, 0, 0));
        move_to(&mut e, 1, 0);
        move_to(&mut e, 2, 0);
        move_end(&mut e);
        assert_eq!(px(&e, 3, 0), RED);
        assert_eq!(px(&e, 1, 0)[3], 0);
        assert_eq!(px(&e, 5, 0), BLUE);
        assert_eq!(e.sel.active(6, 1).unwrap().bounds(), Some((2, 0, 3, 1)));
        assert!(e.undo());
        assert_eq!(px(&e, 1, 0), RED);
        assert_eq!(px(&e, 3, 0)[3], 0);
    }

    #[test]
    fn move_whole_cel_without_selection_and_clips_at_edge() {
        let mut e = editor(4, 1);
        dot(&mut e, 3, 0, RED);
        dot(&mut e, 0, 0, BLUE);
        assert!(nudge(&mut e, 0, 0, 1, 0));
        assert_eq!(px(&e, 1, 0), BLUE);
        assert_eq!(px(&e, 3, 0)[3], 0);
        assert!(e.sel.active(4, 1).is_none());
    }

    #[test]
    fn move_back_restores_clipped_pixels_within_one_drag() {
        let mut e = editor(4, 1);
        dot(&mut e, 3, 0, RED);
        move_begin(&mut e, 0, 0);
        move_to(&mut e, 2, 0);
        assert!((0..4).all(|x| px(&e, x, 0)[3] == 0));
        move_to(&mut e, 0, 0);
        move_end(&mut e);
        assert_eq!(px(&e, 3, 0), RED);
    }

    #[test]
    fn copy_delete_paste_round_trip() {
        let mut e = editor(4, 4);
        dot(&mut e, 1, 1, RED);
        dot(&mut e, 2, 2, BLUE);
        select_rect(&mut e, (1, 1), (2, 2));
        assert_eq!(copy_bounds(&e), (1, 1, 2, 2));
        let data = copy(&e, 0, 0).unwrap();
        assert_eq!(&data[0..4], &RED);
        assert_eq!(&data[12..16], &BLUE);
        assert_eq!(&data[4..8], &[0; 4]);
        assert!(delete(&mut e, 0, 0));
        assert_eq!(px(&e, 1, 1)[3], 0);
        assert!(paste(&mut e, 0, 0, 2, 2, &data, 0, 2));
        assert_eq!(px(&e, 0, 2), RED);
        assert_eq!(px(&e, 1, 3), BLUE);
        assert_eq!(e.sel.active(4, 4).unwrap().bounds(), Some((0, 2, 2, 2)));
        assert!(!paste(&mut e, 0, 0, 2, 2, &data[..8], 0, 0));
    }

    #[test]
    fn flip_selection_in_place() {
        let mut e = editor(4, 1);
        dot(&mut e, 1, 0, RED);
        dot(&mut e, 2, 0, BLUE);
        select_rect(&mut e, (1, 0), (2, 0));
        assert!(transform(&mut e, 0, 0, Op::FlipH));
        assert_eq!(px(&e, 1, 0), BLUE);
        assert_eq!(px(&e, 2, 0), RED);
        assert!(e.undo());
        assert_eq!(px(&e, 1, 0), RED);
    }

    #[test]
    fn rotate_selection_cw_then_ccw_is_identity() {
        let mut e = editor(5, 5);
        dot(&mut e, 1, 1, RED);
        dot(&mut e, 2, 1, BLUE);
        select_rect(&mut e, (1, 1), (2, 1));
        assert!(transform(&mut e, 0, 0, Op::RotateCw));
        // 2x1 rotated clockwise becomes 1x2, red on top, centered on the old box.
        assert_eq!(e.sel.active(5, 5).unwrap().bounds().map(|b| (b.2, b.3)), Some((1, 2)));
        let (bx, by, _, _) = e.sel.active(5, 5).unwrap().bounds().unwrap();
        assert_eq!(px(&e, bx, by), RED);
        assert_eq!(px(&e, bx, by + 1), BLUE);
        assert!(transform(&mut e, 0, 0, Op::RotateCcw));
        assert_eq!(px(&e, 1, 1), RED);
        assert_eq!(px(&e, 2, 1), BLUE);
    }

    #[test]
    fn rotate_whole_cel_square() {
        let mut e = editor(2, 2);
        dot(&mut e, 0, 0, RED);
        assert!(transform(&mut e, 0, 0, Op::RotateCw));
        assert_eq!(px(&e, 1, 0), RED);
        assert_eq!(px(&e, 0, 0)[3], 0);
    }

    #[test]
    fn crop_to_selection_resizes_all_cels_and_undoes() {
        let mut e = editor(6, 6);
        e.sprite.add_frame(0, true);
        e.sprite.add_layer(0);
        dot(&mut e, 3, 4, RED);
        select_rect(&mut e, (2, 3), (4, 5));
        assert!(crop_to_selection(&mut e));
        assert_eq!((e.sprite.width, e.sprite.height), (3, 3));
        assert_eq!(px(&e, 1, 1), RED);
        assert_eq!(e.sprite.render(1).len(), 36);
        assert!(e.sel.active(3, 3).is_none());
        assert!(e.undo());
        assert_eq!((e.sprite.width, e.sprite.height), (6, 6));
        assert_eq!(px(&e, 3, 4), RED);
    }

    #[test]
    fn grow_canvas_with_offset() {
        let mut e = editor(2, 2);
        dot(&mut e, 0, 0, RED);
        assert!(resize_canvas(&mut e, 4, 3, 1, 1));
        assert_eq!(px(&e, 1, 1), RED);
        assert_eq!(e.sprite.cels[0][0].as_ref().unwrap().pixels.len(), 4 * 3 * 4);
        assert!(!resize_canvas(&mut e, 4, 3, 0, 0));
    }

    #[test]
    fn stale_selection_with_other_canvas_size_is_ignored() {
        let mut e = editor(4, 4);
        select_all(&mut e);
        e.sprite.width = 5;
        assert!(e.sel.active(5, 4).is_none());
    }

    #[test]
    fn clip_restores_pixels_outside_selection() {
        let mut e = Editor::new(Sprite::new(4, 1));
        e.checkpoint();
        e.sprite.draw_line(0, 0, 0, 0, 0, 0, [9, 9, 9, 255], 1);
        select_rect(&mut e, (1, 0), (2, 0));
        e.checkpoint();
        e.sprite.draw_line(0, 0, 0, 0, 3, 0, [255, 0, 0, 255], 1);
        clip_to_selection(&mut e, 0, 0);
        let px = e.sprite.render(0);
        assert_eq!(&px[0..4], &[9, 9, 9, 255]);
        assert_eq!(&px[4..12], &[255, 0, 0, 255, 255, 0, 0, 255]);
        assert_eq!(&px[12..16], &[0; 4]);
    }

    #[test]
    fn clip_inside_a_transaction_uses_the_state_before_the_current_operation() {
        let mut e = Editor::new(Sprite::new(4, 1));
        select_rect(&mut e, (1, 0), (2, 0));
        e.begin_transaction();
        // First operation: paints pixel 0, outside the selection (unclipped here on purpose).
        e.checkpoint();
        e.sprite.draw_line(0, 0, 0, 0, 0, 0, [9, 9, 9, 255], 1);
        // Second operation paints everything, then is clipped back to what the first left behind.
        e.checkpoint();
        e.sprite.draw_line(0, 0, 0, 0, 3, 0, RED, 1);
        clip_to_selection(&mut e, 0, 0);
        e.end_transaction();
        let px = e.sprite.render(0);
        assert_eq!(&px[0..4], &[9, 9, 9, 255]);
        assert_eq!(&px[4..12], &[255, 0, 0, 255, 255, 0, 0, 255]);
        assert_eq!(&px[12..16], &[0; 4]);
        // One undo step restores everything from before the transaction.
        assert!(e.undo());
        assert!(e.sprite.render(0).iter().all(|&b| b == 0));
        // After the transaction the normal checkpoint base is used again.
        e.checkpoint();
        e.sprite.draw_line(0, 0, 0, 0, 3, 0, BLUE, 1);
        clip_to_selection(&mut e, 0, 0);
        let px = e.sprite.render(0);
        assert_eq!(&px[0..4], &[0; 4]);
        assert_eq!(&px[4..8], &BLUE);
    }

    fn selected(e: &Editor, w: i32, h: i32) -> Vec<(i32, i32)> {
        let Some(s) = e.sel.active(e.sprite.width, e.sprite.height) else { return vec![] };
        (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|&(x, y)| s.get(x, y)).collect()
    }

    #[test]
    fn wand_contiguous_vs_global() {
        let mut e = editor(5, 1);
        for x in [0, 1, 3, 4] {
            dot(&mut e, x, 0, RED);
        }
        assert!(wand(&mut e, 0, 0, 0, 0, 0, true, Mode::Replace));
        assert_eq!(selected(&e, 5, 1), vec![(0, 0), (1, 0)]);
        assert!(wand(&mut e, 0, 0, 0, 0, 0, false, Mode::Replace));
        assert_eq!(selected(&e, 5, 1), vec![(0, 0), (1, 0), (3, 0), (4, 0)]);
        assert!(wand(&mut e, 0, 0, 2, 0, 0, true, Mode::Replace));
        assert_eq!(selected(&e, 5, 1), vec![(2, 0)]);
        assert!(!wand(&mut e, 0, 0, 9, 0, 0, true, Mode::Replace));
    }

    #[test]
    fn wand_tolerance_and_modes() {
        let mut e = editor(4, 1);
        dot(&mut e, 0, 0, [100, 0, 0, 255]);
        dot(&mut e, 1, 0, [110, 0, 0, 255]);
        dot(&mut e, 2, 0, [200, 0, 0, 255]);
        wand(&mut e, 0, 0, 0, 0, 5, true, Mode::Replace);
        assert_eq!(selected(&e, 4, 1), vec![(0, 0)]);
        wand(&mut e, 0, 0, 0, 0, 10, true, Mode::Replace);
        assert_eq!(selected(&e, 4, 1), vec![(0, 0), (1, 0)]);
        wand(&mut e, 0, 0, 2, 0, 0, true, Mode::Add);
        assert_eq!(selected(&e, 4, 1), vec![(0, 0), (1, 0), (2, 0)]);
        wand(&mut e, 0, 0, 1, 0, 0, true, Mode::Subtract);
        assert_eq!(selected(&e, 4, 1), vec![(0, 0), (2, 0)]);
        // Transparent pixels match each other regardless of their stored color.
        wand(&mut e, 0, 0, 3, 0, 0, false, Mode::Replace);
        assert_eq!(selected(&e, 4, 1), vec![(3, 0)]);
    }

    #[test]
    fn selection_changes_are_undoable() {
        let mut e = editor(4, 4);
        select_rect(&mut e, (0, 0), (1, 1));
        select_rect(&mut e, (2, 2), (3, 3));
        assert_eq!(selected(&e, 4, 4), vec![(2, 2), (3, 2), (2, 3), (3, 3)]);
        assert!(e.undo());
        assert_eq!(selected(&e, 4, 4), vec![(0, 0), (1, 0), (0, 1), (1, 1)]);
        assert!(e.undo());
        assert!(selected(&e, 4, 4).is_empty());
        assert!(e.redo() && e.redo());
        assert_eq!(selected(&e, 4, 4).len(), 4);
        deselect(&mut e);
        let steps = e.sel.undo.len();
        deselect(&mut e);
        assert_eq!(e.sel.undo.len(), steps, "no-op selection change must not add an undo step");
    }

    #[test]
    fn undo_restores_selection_after_move_and_crop() {
        let mut e = editor(6, 6);
        dot(&mut e, 1, 1, RED);
        select_rect(&mut e, (0, 0), (2, 2));
        assert!(nudge(&mut e, 0, 0, 2, 2));
        assert_eq!(e.sel.active(6, 6).unwrap().bounds(), Some((2, 2, 3, 3)));
        assert!(e.undo());
        assert_eq!(e.sel.active(6, 6).unwrap().bounds(), Some((0, 0, 3, 3)));
        assert_eq!(px(&e, 1, 1), RED);
        assert!(crop_to_selection(&mut e));
        assert_eq!((e.sprite.width, e.sprite.height), (3, 3));
        assert!(e.sel.active(3, 3).is_none());
        assert!(e.undo());
        assert_eq!(e.sel.active(6, 6).unwrap().bounds(), Some((0, 0, 3, 3)));
    }

    #[test]
    fn session_combines_with_base_and_is_one_undo_step() {
        let mut e = editor(6, 1);
        select_rect(&mut e, (0, 0), (2, 0));
        begin_session(&mut e, Mode::Add);
        select_rect(&mut e, (4, 0), (4, 0));
        select_rect(&mut e, (4, 0), (5, 0));
        end_session(&mut e);
        assert_eq!(selected(&e, 6, 1), vec![(0, 0), (1, 0), (2, 0), (4, 0), (5, 0)]);
        assert!(e.undo());
        assert_eq!(selected(&e, 6, 1), vec![(0, 0), (1, 0), (2, 0)]);
        begin_session(&mut e, Mode::Subtract);
        select_rect(&mut e, (1, 0), (5, 0));
        end_session(&mut e);
        assert_eq!(selected(&e, 6, 1), vec![(0, 0)]);
    }

    #[test]
    fn polygon_fills_interior_and_outline() {
        let mut e = editor(8, 8);
        select_polygon(&mut e, &[(1, 1), (5, 1), (5, 5), (1, 5)]);
        let b = e.sel.active(8, 8).unwrap().bounds();
        assert_eq!(b, Some((1, 1, 5, 5)));
        assert_eq!(selected(&e, 8, 8).len(), 25);
        select_polygon(&mut e, &[(0, 0), (6, 0), (0, 6)]);
        let s = e.sel.active(8, 8).unwrap();
        assert!(s.get(1, 1) && s.get(3, 0) && !s.get(5, 5) && !s.get(7, 7));
        select_polygon(&mut e, &[(2, 2), (6, 2)]);
        assert_eq!(selected(&e, 8, 8).len(), 5);
    }

    #[test]
    fn expand_contract_border() {
        let mut e = editor(9, 9);
        select_rect(&mut e, (3, 3), (5, 5));
        assert!(modify(&mut e, Modify::Expand, 1));
        assert_eq!(e.sel.active(9, 9).unwrap().bounds(), Some((2, 2, 5, 5)));
        assert!(modify(&mut e, Modify::Contract, 2));
        assert_eq!(selected(&e, 9, 9), vec![(4, 4)]);
        assert!(modify(&mut e, Modify::Contract, 1));
        assert!(e.sel.active(9, 9).is_none());
        assert!(!modify(&mut e, Modify::Expand, 1));
        select_rect(&mut e, (2, 2), (6, 6));
        assert!(modify(&mut e, Modify::Border, 1));
        let s = e.sel.active(9, 9).unwrap();
        assert!(s.get(1, 1) && s.get(4, 1) && !s.get(4, 2) && !s.get(4, 4) && !s.get(0, 0));
        assert_eq!(s.bounds(), Some((1, 1, 7, 7)));
    }

    #[test]
    fn contract_shrinks_from_canvas_edge() {
        let mut e = editor(4, 4);
        select_all(&mut e);
        assert!(modify(&mut e, Modify::Contract, 1));
        assert_eq!(e.sel.active(4, 4).unwrap().bounds(), Some((1, 1, 2, 2)));
    }

    #[test]
    fn floating_paste_moves_over_original_pixels() {
        let mut e = editor(6, 1);
        dot(&mut e, 1, 0, BLUE);
        let img = [RED, RED].concat();
        assert!(paste(&mut e, 0, 0, 2, 1, &img, 0, 0));
        assert!(float_active(&e));
        assert_eq!(px(&e, 1, 0), RED);
        assert!(move_begin(&mut e, 0, 0));
        move_to(&mut e, 3, 0);
        move_to(&mut e, 4, 0);
        move_end(&mut e);
        // The pixel under the original paste position is back, the paste sits at x = 4..6.
        assert_eq!(px(&e, 1, 0), BLUE);
        assert_eq!(px(&e, 0, 0)[3], 0);
        assert_eq!(px(&e, 4, 0), RED);
        assert_eq!(px(&e, 5, 0), RED);
        assert_eq!(e.sel.active(6, 1).unwrap().bounds(), Some((4, 0, 2, 1)));
        // A later edit commits the float as its own undo step.
        let before = e.sel.undo.len();
        assert!(delete(&mut e, 0, 0));
        assert!(!float_active(&e));
        assert_eq!(e.sel.undo.len(), before + 1);
        assert!(e.undo());
        assert_eq!(px(&e, 4, 0), RED);
    }

    #[test]
    fn float_cancel_and_undo_remove_the_paste() {
        let mut e = editor(4, 1);
        dot(&mut e, 0, 0, BLUE);
        select_rect(&mut e, (3, 0), (3, 0));
        assert!(paste(&mut e, 0, 0, 1, 1, &RED, 0, 0));
        assert_eq!(px(&e, 0, 0), RED);
        assert!(float_cancel(&mut e));
        assert_eq!(px(&e, 0, 0), BLUE);
        assert_eq!(selected(&e, 4, 1), vec![(3, 0)]);
        assert!(!float_cancel(&mut e));
        assert!(paste(&mut e, 0, 0, 1, 1, &RED, 0, 0));
        float_commit(&mut e);
        assert!(!float_cancel(&mut e));
        assert_eq!(px(&e, 0, 0), RED);
        assert!(e.undo());
        assert_eq!(px(&e, 0, 0), BLUE);
    }
}
