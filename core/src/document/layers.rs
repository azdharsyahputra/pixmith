//! Layer hierarchy operations, blend modes, tags, frame reordering, linked cels, and their C ABI.
//!
//! Linked cels are cels of one layer that share the same `Arc<Cel>`. Editing one replaces its
//! buffer (copy on write), so `begin_cel_edit` remembers its siblings and `flush_links` points
//! them at the edited buffer afterwards.

use crate::document::{blend_over, mul_un8, Cel, Editor, Layer, Sprite, Tag};
use std::ffi::{c_char, CString};
use std::ptr::null_mut;
use std::sync::Arc;

/// Cels waiting to be re-shared after an edit: (layer, edited frame, sibling frames).
#[derive(Clone, Debug, Default)]
pub struct PendingLinks(Option<(usize, usize, Vec<usize>)>);

/// Pending bookkeeping never affects document equality.
impl PartialEq for PendingLinks {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// A copy of a cel that shares no buffer with the original.
pub fn independent_cel(c: &Option<Arc<Cel>>) -> Option<Arc<Cel>> {
    c.as_ref().map(|c| Arc::new((**c).clone()))
}

// MARK: Blend modes

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(mut c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    if n < 0.0 {
        for v in &mut c {
            *v = l + (*v - l) * l / (l - n);
        }
    }
    if x > 1.0 {
        for v in &mut c {
            *v = l + (*v - l) * (1.0 - l) / (x - l);
        }
    }
    c
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

/// Sets the saturation of `c`. On ties Aseprite's min/mid/max channel selection can pick the same
/// channel twice, leaving another channel untouched; the selection below reproduces that so grays and
/// two-equal-channel colors blend like Aseprite (its output for such colors is visible in the
/// `blend_saturation_bug` fixture).
fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let [r, g, b] = c;
    let min = if r < g.min(b) {
        0
    } else if g < b {
        1
    } else {
        2
    };
    let max = if r > g.max(b) {
        0
    } else if g > b {
        1
    } else {
        2
    };
    let mid = if r > g {
        if g > b {
            1
        } else if r > b {
            2
        } else {
            0
        }
    } else if g > b {
        if b > r {
            2
        } else {
            0
        }
    } else {
        1
    };
    let mut out = c;
    if c[max] > c[min] {
        out[mid] = (c[mid] - c[min]) * s / (c[max] - c[min]);
        out[max] = s;
    } else {
        out[mid] = 0.0;
        out[max] = 0.0;
    }
    out[min] = 0.0;
    out
}

fn hard_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        cb * 2.0 * cs
    } else {
        let s = 2.0 * cs - 1.0;
        cb + s - cb * s
    }
}

fn soft_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
    } else {
        let d = if cb <= 0.25 { ((16.0 * cb - 12.0) * cb + 4.0) * cb } else { cb.sqrt() };
        cb + (2.0 * cs - 1.0) * (d - cb)
    }
}

/// Blend function B(backdrop, source) for one channel of a separable mode, in 0..=1.
fn separable(mode: u16, cb: f32, cs: f32) -> f32 {
    match mode {
        1 => cb * cs,
        2 => cb + cs - cb * cs,
        3 => hard_light(cs, cb),
        4 => cb.min(cs),
        5 => cb.max(cs),
        6 => {
            if cb == 0.0 {
                0.0
            } else if cs >= 1.0 {
                1.0
            } else {
                (cb / (1.0 - cs)).min(1.0)
            }
        }
        7 => {
            if cb >= 1.0 {
                1.0
            } else if cs <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - cb) / cs).min(1.0)
            }
        }
        8 => hard_light(cb, cs),
        9 => soft_light(cb, cs),
        10 => (cb - cs).abs(),
        11 => cb + cs - 2.0 * cb * cs,
        16 => (cb + cs).min(1.0),
        17 => (cb - cs).max(0.0),
        18 => {
            if cb == 0.0 {
                0.0
            } else if cb >= cs {
                1.0
            } else {
                cb / cs
            }
        }
        _ => cs,
    }
}

fn blend_rgb(mode: u16, cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    match mode {
        12 => set_lum(set_sat(cs, sat(cb)), lum(cb)),
        13 => set_lum(set_sat(cb, sat(cs)), lum(cb)),
        14 => set_lum(cs, lum(cb)),
        15 => set_lum(cb, lum(cs)),
        _ => [0, 1, 2].map(|i| separable(mode, cb[i], cs[i])),
    }
}

/// Composites straight-alpha source `s` onto backdrop `d` with the Aseprite blend mode `mode`.
/// `opacity` (0..=255) scales the source alpha. Unknown modes act as normal.
pub fn blend_pixel(d: &mut [u8], s: &[u8], opacity: u32, mode: u16) {
    if mode == 0 || mode > 18 {
        return blend_over(d, s, opacity);
    }
    let sa = mul_un8(s[3] as u32, opacity);
    if sa == 0 {
        return;
    }
    if d[3] == 0 {
        d.copy_from_slice(&[s[0], s[1], s[2], sa as u8]);
        return;
    }
    let ba = d[3] as u32;
    let cb = [d[0], d[1], d[2]].map(|v| v as f32 / 255.0);
    let cs = [s[0], s[1], s[2]].map(|v| v as f32 / 255.0);
    let mixed = blend_rgb(mode, cb, cs);
    if sa == 255 && ba == 255 {
        // Opaque over opaque: the general formula below reduces exactly to the blended color.
        for i in 0..3 {
            d[i] = (mixed[i] * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
        }
        return;
    }
    let ra = sa + ba - mul_un8(ba, sa);
    for i in 0..3 {
        // The blended color is mixed into the source by the backdrop's alpha, and (matching the output of
        // Aseprite for translucent backdrops) once more by the product of both alphas; the result is then
        // composited like normal blending. For an opaque backdrop this is exactly `blended`.
        let blended = (mixed[i] * 255.0 + 0.5).clamp(0.0, 255.0) as i32;
        let once = s[i] as i32 + mul_signed(ba as i32, blended - s[i] as i32);
        let src = once + mul_signed(mul_un8(ba, sa) as i32, blended - once);
        let b = d[i] as i32;
        d[i] = (b + (src - b) * sa as i32 / ra as i32).clamp(0, 255) as u8;
    }
    d[3] = ra as u8;
}

/// `a * b / 255` rounded, for a signed `b`.
fn mul_signed(a: i32, b: i32) -> i32 {
    let t = a * b + 0x80;
    ((t >> 8) + t) >> 8
}

// MARK: Rendering

/// Composites the visible layers of a frame. A group is rendered on its own and then blended onto what lies
/// below it with the group's opacity and blend mode. Groups that would not change the result are flattened.
pub fn render_frame(s: &Sprite, frame: usize) -> Vec<u8> {
    render_frame_rect(s, frame, (0, 0, s.width as usize, s.height as usize))
}

/// Like `render_frame`, for the pixels of the rectangle `(x, y, w, h)` only; the result is `w * h * 4`
/// bytes, tightly packed, and identical to cropping the full render. The rectangle is clipped to the canvas.
pub fn render_frame_rect(s: &Sprite, frame: usize, rect: (usize, usize, usize, usize)) -> Vec<u8> {
    render_frame_rect_with(s, frame, rect, 0)
}

/// `render_frame_rect` for an editor view: `references` is how opaque reference layers are drawn, in percent
/// (0 leaves them out, which is what every export wants).
pub fn render_frame_rect_with(s: &Sprite, frame: usize, rect: (usize, usize, usize, usize), references: u8) -> Vec<u8> {
    let (cw, ch) = (s.width as usize, s.height as usize);
    let (x, y) = (rect.0.min(cw), rect.1.min(ch));
    let region =
        Region { x, y, w: rect.2.min(cw - x), h: rect.3.min(ch - y), stride: cw, references: references.min(100) };
    let mut out = vec![0u8; region.w * region.h * 4];
    if frame < s.cels.len() && !out.is_empty() {
        composite_range_in(s, frame, 0, s.layers.len(), region, &mut out, 0);
    }
    out
}

/// A rectangle of the canvas; `stride` is the canvas width in pixels. Compositing targets are `w * h` pixel
/// buffers that cover exactly this rectangle.
#[derive(Clone, Copy)]
pub(crate) struct Region {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    stride: usize,
    /// How opaque reference layers are drawn, in percent; 0 skips them.
    references: u8,
}

impl Region {
    fn full(s: &Sprite) -> Region {
        Region { x: 0, y: 0, w: s.width as usize, h: s.height as usize, stride: s.width as usize, references: 0 }
    }

    /// Where the source rows start: canvas-sized images (cels) are read at the region's offset, images that
    /// are themselves region-sized (isolated groups) from their top-left.
    fn canvas_geom(self) -> Geom {
        Geom { w: self.w, stride: self.stride, x: self.x, y: self.y }
    }

    fn buffer_geom(self) -> Geom {
        Geom { w: self.w, stride: self.w, x: 0, y: 0 }
    }
}

/// Which pixels of a source image land on the target: `w` pixels per row starting at (`x`, `y`) of an image
/// that is `stride` pixels wide.
#[derive(Clone, Copy)]
struct Geom {
    w: usize,
    stride: usize,
    x: usize,
    y: usize,
}

/// Groups nested deeper than this are flattened (their blend mode and opacity still scale their
/// children's opacity, but they are not rendered in isolation). Bounds recursion and the number
/// of canvas-sized temporary buffers alive at once, whatever shape the layer tree has.
const MAX_ISOLATED_DEPTH: usize = 8;

/// Deepest nesting a loaded layer tree may have (see `normalize_levels`).
pub const MAX_GROUP_DEPTH: u16 = 64;

/// Makes `child_level` describe a well-formed tree: the first layer is at level 0, a layer is at
/// most one level deeper than the layer before it, and only groups can have children. Levels are
/// capped at `MAX_GROUP_DEPTH`. Returns whether anything changed.
pub fn normalize_levels(layers: &mut [Layer]) -> bool {
    let mut changed = false;
    let mut max = 0u16;
    for l in layers.iter_mut() {
        let level = l.child_level.min(max).min(MAX_GROUP_DEPTH);
        changed |= level != l.child_level;
        l.child_level = level;
        max = if l.is_group() { (level + 1).min(MAX_GROUP_DEPTH) } else { level };
    }
    changed
}

/// Composites a cel (or an isolated group's image) onto `out`, row by row. Gives the same bytes as
/// calling `blend_pixel` for every pixel, but skips transparent rows and copies opaque pixels.
fn composite_image(out: &mut [u8], src: &[u8], g: Geom, opacity: u32, mode: u16) {
    if opacity == 0 || g.w == 0 {
        return;
    }
    for (row, d) in out.chunks_exact_mut(g.w * 4).enumerate() {
        let start = ((g.y + row) * g.stride + g.x) * 4;
        let Some(s) = src.get(start..start + g.w * 4) else { break };
        // Fully transparent rows (common in sparse cels) are skipped after one cheap pass.
        if s.chunks_exact(4).fold(0u8, |acc, p| acc | p[3]) == 0 {
            continue;
        }
        let normal = mode == 0 || mode > 18;
        for (d, s) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
            match s[3] {
                0 => {}
                255 if normal && opacity == 255 => d.copy_from_slice(s),
                _ => blend_pixel(d, s, opacity, mode),
            }
        }
    }
}

pub(crate) fn composite_range(s: &Sprite, frame: usize, start: usize, end: usize, out: &mut [u8], depth: usize) {
    composite_range_in(s, frame, start, end, Region::full(s), out, depth)
}

/// `composite_range` for a region of the canvas; `out` holds just that region.
pub(crate) fn composite_range_in(
    s: &Sprite,
    frame: usize,
    start: usize,
    end: usize,
    r: Region,
    out: &mut [u8],
    depth: usize,
) {
    for (i, next) in drawing_order(s, frame, start, end) {
        let layer = &s.layers[i];
        if layer.visible && (!layer.is_reference() || r.references > 0) {
            if layer.is_group() {
                let plain = layer.opacity == 255 && s.layers[i..next].iter().all(|l| l.blend_mode == 0);
                if let Some(tw) = crate::gamedev::tween::tweened(s, frame, i) {
                    composite_image(
                        out,
                        &tw.pixels,
                        r.canvas_geom(),
                        layer.opacity as u32 * tw.opacity as u32 / 255,
                        layer.blend_mode,
                    );
                } else if depth >= MAX_ISOLATED_DEPTH {
                    composite_flat(s, frame, i, next, r, out);
                } else if plain {
                    composite_range_in(s, frame, i + 1, next, r, out, depth + 1);
                } else if has_visible_cels(s, frame, i + 1, next, r) {
                    let mut group = vec![0u8; out.len()];
                    composite_range_in(s, frame, i + 1, next, r, &mut group, depth + 1);
                    composite_image(out, &group, r.buffer_geom(), layer.opacity as u32, layer.blend_mode);
                }
            } else if let Some(cel) =
                crate::gamedev::tween::tweened(s, frame, i).as_ref().or_else(|| s.cel_for_render(frame, i))
            {
                let opacity = reference_scaled(layer, r, layer.opacity as u32 * cel.opacity as u32 / 255);
                composite_image(out, &cel.pixels, r.canvas_geom(), opacity, layer.blend_mode);
            }
        }
    }
}

/// Opacity of a layer's cel once the editor's reference layer opacity is applied.
fn reference_scaled(layer: &Layer, r: Region, opacity: u32) -> u32 {
    if layer.is_reference() {
        opacity * r.references as u32 / 100
    } else {
        opacity
    }
}

/// The entries of one group (or of the root) as `(first layer, one past its subtree)` in drawing order. Layers
/// draw bottom to top, except that a cel with a z-index draws as if its layer sat that many places higher (or
/// lower, when negative): entries are ordered by layer index + z-index, and ties keep the lower layer index
/// first. Layer indexes are positions in the whole layer list (groups count), like Aseprite's. Groups have no
/// z-index.
fn drawing_order(s: &Sprite, frame: usize, start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut entries = Vec::new();
    let mut custom = false;
    let mut i = start;
    while i < end {
        let layer = &s.layers[i];
        let next = if layer.is_group() { s.subtree_end(i).min(end) } else { i + 1 };
        let z = if layer.is_group() { 0 } else { s.cel_for_render(frame, i).map_or(0, |c| c.z_index as i64) };
        custom |= z != 0;
        entries.push((i, next, i as i64 + z));
        i = next;
    }
    if custom {
        entries.sort_by_key(|&(i, _, order)| (order, i));
    }
    entries.into_iter().map(|(i, next, _)| (i, next)).collect()
}

/// Whether any layer in `start..end` could draw something in `frame`.
fn has_visible_cels(s: &Sprite, frame: usize, start: usize, end: usize, r: Region) -> bool {
    (start..end).any(|i| {
        let l = &s.layers[i];
        !l.is_group() && l.visible && (!l.is_reference() || r.references > 0) && s.cel_for_render(frame, i).is_some()
    })
}

/// Composites the subtree `start..end` without isolating groups, in one pass without recursion: a
/// group only hides its children or scales their opacity.
fn composite_flat(s: &Sprite, frame: usize, start: usize, end: usize, r: Region, out: &mut [u8]) {
    // Enclosing groups: (level, accumulated opacity, shown).
    let mut groups: Vec<(u16, u32, bool)> = Vec::new();
    for i in start..end {
        let layer = &s.layers[i];
        while groups.last().is_some_and(|g| g.0 >= layer.child_level) {
            groups.pop();
        }
        let (parent_opacity, parent_shown) = groups.last().map_or((255, true), |g| (g.1, g.2));
        let shown = parent_shown && layer.visible;
        let opacity = parent_opacity * layer.opacity as u32 / 255;
        if layer.is_group() {
            groups.push((layer.child_level, opacity, shown));
        } else if shown && (!layer.is_reference() || r.references > 0) {
            if let Some(cel) = s.cel_for_render(frame, i) {
                composite_image(
                    out,
                    &cel.pixels,
                    r.canvas_geom(),
                    reference_scaled(layer, r, opacity * cel.opacity as u32 / 255),
                    layer.blend_mode,
                );
            }
        }
    }
}

// MARK: Layer hierarchy

impl Sprite {
    /// The cel to draw at (frame, layer). A linked cel that was just edited has not been re-shared with its
    /// siblings yet (`flush_links` runs at the next edit), so they must show the edited image.
    fn cel_for_render(&self, frame: usize, layer: usize) -> Option<&Arc<Cel>> {
        if let Some((l, f, sibs)) = &self.links.0 {
            if *l == layer && sibs.contains(&frame) {
                return self.cels.get(*f)?.get(*l)?.as_ref();
            }
        }
        self.cels[frame][layer].as_ref()
    }

    /// Whether an edited linked cel has not been re-shared with its siblings yet (they show the edit too).
    pub fn has_pending_links(&self) -> bool {
        self.links.0.is_some()
    }

    /// A layer's opacity multiplied by those of all its parent groups (0..=255).
    pub fn effective_opacity(&self, i: usize) -> u32 {
        let mut opacity = self.layers[i].opacity as u32;
        let mut level = self.layers[i].child_level;
        for parent in self.layers[..i].iter().rev() {
            if level == 0 {
                break;
            }
            if parent.child_level == level - 1 {
                opacity = opacity * parent.opacity as u32 / 255;
                level -= 1;
            }
        }
        opacity
    }

    /// One past the last layer of `i`'s subtree (`i` itself plus, for a group, its descendants).
    pub fn subtree_end(&self, i: usize) -> usize {
        let level = self.layers[i].child_level;
        let mut end = i + 1;
        while end < self.layers.len() && self.layers[end].child_level > level {
            end += 1;
        }
        end
    }

    /// The group directly containing layer `i`.
    pub fn parent_layer(&self, i: usize) -> Option<usize> {
        let level = self.layers.get(i)?.child_level;
        if level == 0 {
            return None;
        }
        (0..i).rev().find(|&j| self.layers[j].child_level == level - 1)
    }

    /// Where a new layer above `above` goes, and its nesting level.
    fn insert_point(&self, above: usize) -> (usize, u16) {
        let above = above.min(self.layers.len() - 1);
        (self.subtree_end(above), self.layers[above].child_level)
    }

    /// Moves the subtree at `i` so it starts at `dest` (an index in the list without it).
    fn relocate(&mut self, i: usize, dest: usize, level: u16) -> usize {
        let end = self.subtree_end(i);
        let base = self.layers[i].child_level;
        let mut block: Vec<Layer> = self.layers.drain(i..end).collect();
        for l in &mut block {
            l.child_level = l.child_level - base + level;
        }
        self.layers.splice(dest..dest, block);
        for row in &mut self.cels {
            let cels: Vec<_> = row.drain(i..end).collect();
            row.splice(dest..dest, cels);
        }
        dest
    }

    /// Moves a layer (with its children) up or down among its siblings. At the edge of a group it
    /// leaves the group instead. Returns the new index.
    pub fn move_layer(&mut self, i: usize, up: bool) -> Option<usize> {
        if i >= self.layers.len() {
            return None;
        }
        self.flush_links();
        let (end, level) = (self.subtree_end(i), self.layers[i].child_level);
        let n = end - i;
        if up {
            if end < self.layers.len() && self.layers[end].child_level == level {
                let sibling = self.subtree_end(end) - end;
                return Some(self.relocate(i, i + sibling, level));
            }
        } else if let Some(start) = (0..i)
            .rev()
            .take_while(|&j| self.layers[j].child_level >= level)
            .find(|&j| self.layers[j].child_level == level)
        {
            return Some(self.relocate(i, start, level));
        }
        let parent = self.parent_layer(i)?;
        let plevel = self.layers[parent].child_level;
        let dest = if up { self.subtree_end(parent) - n } else { parent };
        Some(self.relocate(i, dest, plevel))
    }

    /// Moves a layer to the top of `group`.
    pub fn move_into_group(&mut self, i: usize, group: usize) -> Option<usize> {
        if i >= self.layers.len() || group >= self.layers.len() || !self.layers[group].is_group() {
            return None;
        }
        let end = self.subtree_end(i);
        if (i..end).contains(&group) || self.parent_layer(i) == Some(group) {
            return None;
        }
        self.flush_links();
        let group_end = self.subtree_end(group);
        let dest = if group_end > i { group_end - (end - i) } else { group_end };
        Some(self.relocate(i, dest, self.layers[group].child_level + 1))
    }

    /// Moves a layer out of its group, to just above it.
    pub fn move_out_of_group(&mut self, i: usize) -> Option<usize> {
        let parent = self.parent_layer(i)?;
        self.flush_links();
        let n = self.subtree_end(i) - i;
        let dest = self.subtree_end(parent) - n;
        Some(self.relocate(i, dest, self.layers[parent].child_level))
    }

    /// Copies a layer (with its children) just above it. Returns the copy's index.
    pub fn duplicate_layer(&mut self, i: usize) -> Option<usize> {
        if i >= self.layers.len() {
            return None;
        }
        self.flush_links();
        let end = self.subtree_end(i);
        let mut block: Vec<Layer> = self.layers[i..end].to_vec();
        block[0].name.push_str(" copy");
        self.layers.splice(end..end, block);
        for row in &mut self.cels {
            let copies: Vec<_> = row[i..end].iter().map(independent_cel).collect();
            row.splice(end..end, copies);
        }
        Some(end)
    }

    /// Flattens a layer into the image layer directly below it, keeping the rendered result.
    pub fn merge_down(&mut self, i: usize) -> Option<usize> {
        if i == 0 || i >= self.layers.len() {
            return None;
        }
        let (top, below) = (&self.layers[i], &self.layers[i - 1]);
        if top.is_group()
            || below.is_group()
            || top.is_reference()
            || below.is_reference()
            || top.child_level != below.child_level
        {
            return None;
        }
        self.flush_links();
        let (opacity, mode, visible) =
            (self.layers[i].opacity as u32, self.layers[i].blend_mode, self.layers[i].visible);
        let len = self.width as usize * self.height as usize * 4;
        for f in 0..self.frame_count() {
            let src = self.cels[f][i].take();
            let dst = self.cels[f][i - 1].take();
            let (Some(src), true) = (src, visible) else {
                self.cels[f][i - 1] = dst;
                continue;
            };
            let dst_user_data = dst.as_ref().map(|d| d.user_data.clone()).unwrap_or_default();
            let mut pixels = match dst {
                Some(d) => {
                    let mut px = d.pixels.clone();
                    if d.opacity != 255 {
                        for p in px.chunks_exact_mut(4) {
                            p[3] = (p[3] as u32 * d.opacity as u32 / 255) as u8;
                        }
                    }
                    px
                }
                None => vec![0; len],
            };
            let op = opacity * src.opacity as u32 / 255;
            for (d, s) in pixels.chunks_exact_mut(4).zip(src.pixels.chunks_exact(4)) {
                blend_pixel(d, s, op, mode);
            }
            self.cels[f][i - 1] =
                Some(Arc::new(Cel { opacity: 255, pixels, user_data: dst_user_data, ..Default::default() }));
        }
        self.layers.remove(i);
        for row in &mut self.cels {
            row.remove(i);
        }
        for f in 0..self.frame_count() {
            crate::document::color_mode::conform_cel(self, i - 1, f);
        }
        if self.layers[i - 1].kind == crate::document::tilemap::KIND_TILEMAP {
            for f in 0..self.frame_count() {
                crate::document::tilemap::sync_cel(self, i - 1, f, None);
            }
        }
        Some(i - 1)
    }

    /// Adds an empty group above `above` (above its children if it is a group).
    pub fn add_group(&mut self, above: usize) -> usize {
        self.flush_links();
        let (at, level) = self.insert_point(above);
        let mut group = Layer::new(format!("Group {}", self.layers.iter().filter(|l| l.is_group()).count() + 1));
        group.kind = 1;
        group.child_level = level;
        self.insert_layer(at, group)
    }

    /// Wraps a layer and its children in a new group. Returns the group's index.
    pub fn group_layer(&mut self, i: usize) -> Option<usize> {
        if i >= self.layers.len() {
            return None;
        }
        self.flush_links();
        let end = self.subtree_end(i);
        let mut group = Layer::new(format!("Group {}", self.layers.iter().filter(|l| l.is_group()).count() + 1));
        group.kind = 1;
        group.child_level = self.layers[i].child_level;
        for l in &mut self.layers[i..end] {
            l.child_level += 1;
        }
        Some(self.insert_layer(i, group))
    }

    fn insert_layer(&mut self, at: usize, layer: Layer) -> usize {
        self.layers.insert(at, layer);
        for row in &mut self.cels {
            row.insert(at, None);
        }
        at
    }

    pub fn set_layer_props(&mut self, i: usize, name: &str, opacity: u8, blend_mode: u16, visible: bool) -> bool {
        let Some(l) = self.layers.get_mut(i) else { return false };
        l.name = name.to_string();
        l.opacity = opacity;
        l.blend_mode = if blend_mode <= 18 { blend_mode } else { 0 };
        l.visible = visible;
        true
    }

    // MARK: Frames and cels

    /// Moves frame `from` so it ends up at index `to`. Tags keep their frame ranges.
    pub fn move_frame(&mut self, from: usize, to: usize) -> bool {
        let n = self.frame_count();
        if from >= n || to >= n || from == to {
            return false;
        }
        self.flush_links();
        let row = self.cels.remove(from);
        self.cels.insert(to, row);
        let d = self.durations.remove(from);
        self.durations.insert(to, d);
        self.extra.frame_moved(from, to);
        true
    }

    /// Frames of `layer` whose cel shares its buffer with the cel at `frame`.
    pub fn link_siblings(&self, layer: usize, frame: usize) -> Vec<usize> {
        let Some(Some(cel)) = self.cels.get(frame).and_then(|r| r.get(layer)) else { return Vec::new() };
        (0..self.frame_count())
            .filter(|&f| f != frame && matches!(&self.cels[f][layer], Some(o) if Arc::ptr_eq(o, cel)))
            .collect()
    }

    pub fn is_cel_linked(&self, layer: usize, frame: usize) -> bool {
        !self.link_siblings(layer, frame).is_empty()
    }

    pub(crate) fn begin_cel_edit(&mut self, layer: usize, frame: usize) {
        // Still editing the same linked cel: its siblings stay on the old image until the next flush, so the
        // buffer is unique and further writes do not clone it again (one clone per gesture, not per point).
        if matches!(&self.links.0, Some((l, f, _)) if (*l, *f) == (layer, frame)) {
            return;
        }
        self.flush_links();
        let sibs = self.link_siblings(layer, frame);
        if !sibs.is_empty() {
            self.links.0 = Some((layer, frame, sibs));
        }
    }

    /// Re-shares the most recently edited cel with the cels it was linked to.
    pub fn flush_links(&mut self) {
        let Some((layer, frame, sibs)) = self.links.0.take() else { return };
        let Some(Some(cel)) = self.cels.get(frame).and_then(|r| r.get(layer)).cloned() else { return };
        for f in sibs {
            if let Some(slot) = self.cels.get_mut(f).and_then(|r| r.get_mut(layer)) {
                *slot = Some(cel.clone());
            }
        }
    }

    /// Makes the cels of `layer` in frames `from..=to` share the cel at `from`.
    pub fn link_cels(&mut self, layer: usize, from: usize, to: usize) -> bool {
        let to = to.min(self.frame_count().saturating_sub(1));
        if layer >= self.layers.len() || self.layers[layer].is_group() || from >= to {
            return false;
        }
        self.flush_links();
        let Some(src) = self.cels[from][layer].clone() else { return false };
        for f in from + 1..=to {
            self.cels[f][layer] = Some(src.clone());
        }
        true
    }

    /// Gives a linked cel its own copy of the shared image.
    pub fn unlink_cel(&mut self, layer: usize, frame: usize) -> bool {
        self.flush_links();
        if !self.is_cel_linked(layer, frame) {
            return false;
        }
        self.cels[frame][layer] = independent_cel(&self.cels[frame][layer]);
        true
    }

    /// Sets a cel's opacity (shared by linked cels). Returns false if the cel is empty.
    pub fn set_cel_opacity(&mut self, layer: usize, frame: usize, opacity: u8) -> bool {
        if !matches!(self.cels.get(frame).and_then(|r| r.get(layer)), Some(Some(_))) {
            return false;
        }
        match self.cel_mut_any(layer, frame) {
            Some(c) => c.opacity = opacity,
            None => return false,
        }
        self.flush_links();
        true
    }

    // MARK: Tags

    pub fn add_tag(&mut self, from: usize, to: usize) -> usize {
        const COLORS: [[u8; 3]; 6] =
            [[59, 130, 246], [239, 68, 68], [34, 197, 94], [234, 179, 8], [168, 85, 247], [249, 115, 22]];
        let last = self.frame_count() - 1;
        let (from, to) = (from.min(last), to.min(last));
        self.tags.push(Tag {
            name: format!("Tag {}", self.tags.len() + 1),
            from: from.min(to) as u16,
            to: from.max(to) as u16,
            direction: 0,
            repeat: 0,
            color: COLORS[self.tags.len() % COLORS.len()],
            user_data: Default::default(),
        });
        self.tags.len() - 1
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_tag(
        &mut self,
        i: usize,
        name: &str,
        from: usize,
        to: usize,
        direction: u8,
        repeat: u16,
        color: [u8; 3],
    ) -> bool {
        let last = self.frame_count() - 1;
        let Some(t) = self.tags.get_mut(i) else { return false };
        let (from, to) = (from.min(last), to.min(last));
        *t = Tag {
            name: name.to_string(),
            from: from.min(to) as u16,
            to: from.max(to) as u16,
            direction: direction.min(3),
            repeat,
            color,
            user_data: std::mem::take(&mut t.user_data),
        };
        true
    }

    pub fn remove_tag(&mut self, i: usize) -> bool {
        if i >= self.tags.len() {
            return false;
        }
        self.tags.remove(i);
        true
    }

    /// The first tag that contains `frame`.
    pub fn tag_at(&self, frame: usize) -> Option<&Tag> {
        self.tags.iter().find(|t| (t.from as usize..=t.to as usize).contains(&frame))
    }

    /// The frame to show after `frame` during playback. With `within_tag` the animation loops inside
    /// the tag containing `frame`, honoring its direction and repeat count; once a finite repeat count is
    /// used up, playback continues with the frame after the tag.
    pub fn next_play_frame(&self, frame: usize, st: &mut PlayState, within_tag: bool) -> usize {
        let n = self.frame_count();
        let found = self.tags.iter().enumerate().find(|(_, t)| (t.from as usize..=t.to as usize).contains(&frame));
        let Some((ti, t)) = found.filter(|_| within_tag) else {
            *st = PlayState::default();
            return (frame + 1) % n;
        };
        let (from, to) = (t.from as usize, (t.to as usize).min(n - 1));
        if st.tag != ti as i32 {
            *st = PlayState { reversed: matches!(t.direction, 1 | 3), passes: 0, tag: ti as i32 };
        }
        let mut pass = false;
        let next = match t.direction {
            0 => {
                pass = frame >= to;
                if pass {
                    from
                } else {
                    frame + 1
                }
            }
            1 => {
                pass = frame <= from;
                if pass {
                    to
                } else {
                    frame - 1
                }
            }
            d => {
                if from >= to {
                    pass = true;
                    frame
                } else if st.reversed {
                    if frame > from {
                        frame - 1
                    } else {
                        st.reversed = false;
                        pass = d == 2;
                        frame + 1
                    }
                } else if frame < to {
                    frame + 1
                } else {
                    st.reversed = true;
                    pass = d == 3;
                    frame - 1
                }
            }
        };
        if pass {
            st.passes += 1;
            if t.repeat > 0 && st.passes >= t.repeat as u32 {
                *st = PlayState::default();
                return (to + 1) % n;
            }
        }
        next
    }

    // MARK: Cel grid

    fn cel_rect(&self, layers: (usize, usize), frames: (usize, usize)) -> bool {
        layers.0 <= layers.1 && layers.1 < self.layers.len() && frames.0 <= frames.1 && frames.1 < self.frame_count()
    }

    /// One byte per cel, `layer * frame_count + frame`: bit 0 has a cel, bit 1 has visible pixels, bit 2 is linked.
    pub fn cel_grid(&self) -> Vec<u8> {
        let nf = self.frame_count();
        let mut out = vec![0u8; self.layers.len() * nf];
        for l in 0..self.layers.len() {
            for f in 0..nf {
                let Some(c) = &self.cels[f][l] else { continue };
                let content = c.pixels.chunks_exact(4).any(|p| p[3] != 0);
                let linked = (0..nf).any(|g| g != f && matches!(&self.cels[g][l], Some(o) if Arc::ptr_eq(o, c)));
                out[l * nf + f] = 1 | (content as u8) << 1 | (linked as u8) << 2;
            }
        }
        out
    }

    /// Empties the cels in a layers x frames rectangle (inclusive bounds).
    pub fn clear_cels(&mut self, layers: (usize, usize), frames: (usize, usize)) -> bool {
        if !self.cel_rect(layers, frames) {
            return false;
        }
        self.flush_links();
        let mut any = false;
        for f in frames.0..=frames.1 {
            for l in layers.0..=layers.1 {
                any |= self.cels[f][l].take().is_some();
            }
        }
        any
    }

    /// Whether a cel of layer `src` may live on layer `dst`.
    fn cel_fits(&self, src: usize, dst: usize) -> bool {
        let (a, b) = (&self.layers[src], &self.layers[dst]);
        a.is_group() == b.is_group()
            && a.kind == b.kind
            && (a.kind != crate::document::tilemap::KIND_TILEMAP || a.tileset == b.tileset)
    }

    /// Moves (or copies) the cels of a rectangle by `dl` layers and `df` frames. Fails if the target leaves
    /// the sprite or maps a layer onto one of a different kind. Moved cels keep their links.
    pub fn move_cels(
        &mut self,
        layers: (usize, usize),
        frames: (usize, usize),
        dl: isize,
        df: isize,
        copy: bool,
    ) -> bool {
        if !self.cel_rect(layers, frames) || (dl == 0 && df == 0) {
            return false;
        }
        let shift = |v: usize, d: isize| v.checked_add_signed(d);
        let (Some(l0), Some(l1), Some(f0), Some(f1)) =
            (shift(layers.0, dl), shift(layers.1, dl), shift(frames.0, df), shift(frames.1, df))
        else {
            return false;
        };
        if !self.cel_rect((l0, l1), (f0, f1))
            || (layers.0..=layers.1).any(|l| !self.cel_fits(l, l.wrapping_add_signed(dl)))
        {
            return false;
        }
        self.flush_links();
        let mut moved = Vec::new();
        for f in frames.0..=frames.1 {
            for l in layers.0..=layers.1 {
                let cel = if copy { self.cels[f][l].clone() } else { self.cels[f][l].take() };
                moved.push((f.wrapping_add_signed(df), l.wrapping_add_signed(dl), cel));
            }
        }
        for (f, l, cel) in moved {
            self.cels[f][l] = if copy { independent_cel(&cel) } else { cel };
        }
        true
    }

    /// Copies a rectangle of cels to the cel clipboard.
    pub fn copy_cels(&self, layers: (usize, usize), frames: (usize, usize)) -> bool {
        if !self.cel_rect(layers, frames) {
            return false;
        }
        let cels = (frames.0..=frames.1).map(|f| self.cels[f][layers.0..=layers.1].to_vec()).collect();
        let kinds = (layers.0..=layers.1).map(|l| (self.layers[l].kind, self.layers[l].tileset)).collect();
        CLIP.with(|c| *c.borrow_mut() = Some(CelClip { size: (self.width, self.height), cels, kinds, source: None }));
        true
    }

    /// Pastes the cel clipboard with its top-left cel at (`layer`, `frame`), skipping cels that do not fit.
    pub fn paste_cels(&mut self, layer: usize, frame: usize) -> bool {
        self.paste_cels_as(layer, frame, None)
    }

    /// Like `paste_cels`, for the editor `editor_uid`. Tilemap cels copied from another document are skipped,
    /// because their tile indices refer to that document's tilesets.
    pub fn paste_cels_as(&mut self, layer: usize, frame: usize, editor_uid: Option<u64>) -> bool {
        let Some(clip) = CLIP.with(|c| c.borrow().clone()) else { return false };
        if clip.size != (self.width, self.height) || layer >= self.layers.len() || frame >= self.frame_count() {
            return false;
        }
        let foreign = clip.source.is_some() && editor_uid.is_some() && clip.source != editor_uid;
        self.flush_links();
        let mut any = false;
        for (df, row) in clip.cels.iter().enumerate() {
            for (dl, cel) in row.iter().enumerate() {
                let (l, f) = (layer + dl, frame + df);
                if l >= self.layers.len() || f >= self.frame_count() {
                    continue;
                }
                let dst = &self.layers[l];
                let (kind, tileset) = clip.kinds[dl];
                let tilemap_ok = kind != crate::document::tilemap::KIND_TILEMAP || (!foreign && dst.tileset == tileset);
                let compatible = !dst.is_group() && dst.kind == kind && tilemap_ok;
                if compatible {
                    self.cels[f][l] = independent_cel(cel);
                    if kind == crate::document::tilemap::KIND_TILEMAP {
                        crate::document::tilemap::repair_cel(self, l, f);
                    }
                    any = true;
                }
            }
        }
        any
    }
}

/// A copied rectangle of cels, indexed `[frame][layer offset]`.
#[derive(Clone)]
struct CelClip {
    size: (u32, u32),
    cels: Vec<Vec<Option<Arc<Cel>>>>,
    kinds: Vec<(u16, u32)>,
    /// `Editor::uid` of the document it was copied from, when copied through the C ABI.
    source: Option<u64>,
}

thread_local! {
    static CLIP: std::cell::RefCell<Option<CelClip>> = const { std::cell::RefCell::new(None) };
}

/// Playback bookkeeping carried between `next_play_frame` calls.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayState {
    /// Heading of a ping-pong or reverse tag.
    pub reversed: bool,
    /// Completed passes of the current tag.
    pub passes: u32,
    /// Index of the tag being played, or -1.
    pub tag: i32,
}

impl Default for PlayState {
    fn default() -> Self {
        PlayState { reversed: false, passes: 0, tag: -1 }
    }
}

// MARK: C ABI

/// Runs an edit on the sprite as one undo step. The edit must not touch the sprite when it fails.
pub(crate) fn undoable<R>(e: &mut Editor, f: impl FnOnce(&mut Sprite) -> Option<R>) -> Option<R> {
    let before = e.sprite.clone();
    match f(&mut e.sprite) {
        Some(r) => {
            let after = std::mem::replace(&mut e.sprite, before);
            e.checkpoint();
            e.sprite = after;
            Some(r)
        }
        None => {
            e.sprite = before;
            None
        }
    }
}

fn index(r: Option<usize>) -> i32 {
    r.map_or(-1, |i| i as i32)
}

/// A tag as seen from C. `color` is 0xRRGGBB.
#[repr(C)]
pub struct PixmithTag {
    pub from: u32,
    pub to: u32,
    pub direction: u8,
    pub repeat: u16,
    pub color: u32,
}

/// Moves a layer up or down. Returns its new index, or -1.
#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_move(doc: *mut Editor, layer: u32, up: bool) -> i32 {
    crate::ffi::guarded("pixmith_layer_move", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return -1 };
        index(undoable(e, |s| s.move_layer(layer as usize, up)))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_duplicate(doc: *mut Editor, layer: u32) -> i32 {
    crate::ffi::guarded("pixmith_layer_duplicate", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return -1 };
        index(undoable(e, |s| s.duplicate_layer(layer as usize)))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_merge_down(doc: *mut Editor, layer: u32) -> i32 {
    crate::ffi::guarded("pixmith_layer_merge_down", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return -1 };
        index(undoable(e, |s| s.merge_down(layer as usize)))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_can_merge_down(doc: *const Editor, layer: u32) -> bool {
    crate::ffi::guarded("pixmith_layer_can_merge_down", std::ptr::null_mut(), || {
        let Some(s) = doc.as_ref().map(|e| &e.sprite) else { return false };
        let i = layer as usize;
        i > 0
            && s.layers.get(i).zip(s.layers.get(i - 1)).is_some_and(|(a, b)| {
                !a.is_group()
                    && !b.is_group()
                    && !a.is_reference()
                    && !b.is_reference()
                    && a.child_level == b.child_level
            })
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_add_group(doc: *mut Editor, above: u32) -> i32 {
    crate::ffi::guarded("pixmith_layer_add_group", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return -1 };
        index(undoable(e, |s| Some(s.add_group(above as usize))))
    })
}

/// Wraps a layer in a new group. Returns the group's index, or -1.
#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_group(doc: *mut Editor, layer: u32) -> i32 {
    crate::ffi::guarded("pixmith_layer_group", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return -1 };
        index(undoable(e, |s| s.group_layer(layer as usize)))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_into_group(doc: *mut Editor, layer: u32, group: u32) -> i32 {
    crate::ffi::guarded("pixmith_layer_into_group", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return -1 };
        index(undoable(e, |s| s.move_into_group(layer as usize, group as usize)))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_out_of_group(doc: *mut Editor, layer: u32) -> i32 {
    crate::ffi::guarded("pixmith_layer_out_of_group", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return -1 };
        index(undoable(e, |s| s.move_out_of_group(layer as usize)))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_blend_mode(doc: *const Editor, layer: u32) -> u16 {
    crate::ffi::guarded("pixmith_layer_blend_mode", std::ptr::null_mut(), || {
        doc.as_ref().and_then(|e| e.sprite.layers.get(layer as usize)).map_or(0, |l| l.blend_mode)
    })
}

/// Sets name, opacity, blend mode and visibility as one undo step.
#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_set_props(
    doc: *mut Editor,
    layer: u32,
    name: *const c_char,
    opacity: u8,
    blend_mode: u16,
    visible: bool,
) -> bool {
    crate::ffi::guarded("pixmith_layer_set_props", doc, || {
        let (Some(e), Some(name)) = (crate::ffi::editor(doc), crate::ffi::path_arg(name)) else { return false };
        undoable(e, |s| s.set_layer_props(layer as usize, &name, opacity, blend_mode, visible).then_some(())).is_some()
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_frame_move(doc: *mut Editor, from: u32, to: u32) -> bool {
    crate::ffi::guarded("pixmith_frame_move", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return false };
        undoable(e, |s| s.move_frame(from as usize, to as usize).then_some(())).is_some()
    })
}

/// Cel opacity, or -1 for an empty cel.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cel_opacity(doc: *const Editor, layer: u32, frame: u32) -> i32 {
    crate::ffi::guarded("pixmith_cel_opacity", std::ptr::null_mut(), || {
        let cel =
            crate::ffi::synced(doc).and_then(|e| e.sprite.cels.get(frame as usize)?.get(layer as usize)?.as_ref());
        cel.map_or(-1, |c| c.opacity as i32)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_cel_set_opacity(doc: *mut Editor, layer: u32, frame: u32, opacity: u8) -> bool {
    crate::ffi::guarded("pixmith_cel_set_opacity", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return false };
        undoable(e, |s| s.set_cel_opacity(layer as usize, frame as usize, opacity).then_some(())).is_some()
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_cel_is_linked(doc: *const Editor, layer: u32, frame: u32) -> bool {
    crate::ffi::guarded("pixmith_cel_is_linked", std::ptr::null_mut(), || {
        crate::ffi::synced(doc).is_some_and(|e| e.sprite.is_cel_linked(layer as usize, frame as usize))
    })
}

/// Links the cels of a layer in frames `from..=to` to the cel at `from`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cel_link(doc: *mut Editor, layer: u32, from: u32, to: u32) -> bool {
    crate::ffi::guarded("pixmith_cel_link", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return false };
        undoable(e, |s| s.link_cels(layer as usize, from as usize, to as usize).then_some(())).is_some()
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_cel_unlink(doc: *mut Editor, layer: u32, frame: u32) -> bool {
    crate::ffi::guarded("pixmith_cel_unlink", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return false };
        undoable(e, |s| s.unlink_cel(layer as usize, frame as usize).then_some(())).is_some()
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tag_count(doc: *const Editor) -> u32 {
    crate::ffi::guarded("pixmith_tag_count", std::ptr::null_mut(), || {
        doc.as_ref().map_or(0, |e| e.sprite.tags.len() as u32)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tag_get(doc: *const Editor, index: u32, out: *mut PixmithTag) -> bool {
    crate::ffi::guarded("pixmith_tag_get", std::ptr::null_mut(), || {
        let Some(t) = doc.as_ref().and_then(|e| e.sprite.tags.get(index as usize)) else { return false };
        if out.is_null() {
            return false;
        }
        let [r, g, b] = t.color;
        *out = PixmithTag {
            from: t.from as u32,
            to: t.to as u32,
            direction: t.direction,
            repeat: t.repeat,
            color: u32::from_be_bytes([0, r, g, b]),
        };
        true
    })
}

/// Returns a string to release with `pixmith_string_free`, or null.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tag_name(doc: *const Editor, index: u32) -> *mut c_char {
    crate::ffi::guarded("pixmith_tag_name", std::ptr::null_mut(), || {
        doc.as_ref()
            .and_then(|e| e.sprite.tags.get(index as usize))
            .and_then(|t| CString::new(t.name.replace('\0', "")).ok())
            .map_or(null_mut(), CString::into_raw)
    })
}

/// Adds a tag. Returns its index, or -1.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tag_add(doc: *mut Editor, name: *const c_char, info: PixmithTag) -> i32 {
    crate::ffi::guarded("pixmith_tag_add", doc, || {
        let (Some(e), Some(name)) = (crate::ffi::editor(doc), crate::ffi::path_arg(name)) else { return -1 };
        let [_, r, g, b] = info.color.to_be_bytes();
        index(undoable(e, |s| {
            let i = s.add_tag(info.from as usize, info.to as usize);
            s.update_tag(i, &name, info.from as usize, info.to as usize, info.direction, info.repeat, [r, g, b]);
            Some(i)
        }))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tag_update(
    doc: *mut Editor,
    index: u32,
    name: *const c_char,
    info: PixmithTag,
) -> bool {
    crate::ffi::guarded("pixmith_tag_update", doc, || {
        let (Some(e), Some(name)) = (crate::ffi::editor(doc), crate::ffi::path_arg(name)) else { return false };
        let [_, r, g, b] = info.color.to_be_bytes();
        undoable(e, |s| {
            s.update_tag(
                index as usize,
                &name,
                info.from as usize,
                info.to as usize,
                info.direction,
                info.repeat,
                [r, g, b],
            )
            .then_some(())
        })
        .is_some()
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tag_remove(doc: *mut Editor, index: u32) -> bool {
    crate::ffi::guarded("pixmith_tag_remove", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return false };
        undoable(e, |s| s.remove_tag(index as usize).then_some(())).is_some()
    })
}

/// Resets playback state before starting.
#[no_mangle]
pub unsafe extern "C" fn pixmith_play_start(state: *mut PlayState) {
    crate::ffi::guarded("pixmith_play_start", std::ptr::null_mut(), || {
        if let Some(st) = state.as_mut() {
            *st = PlayState::default();
        }
    })
}

/// The next frame of playback; `state` is updated in place.
#[no_mangle]
pub unsafe extern "C" fn pixmith_play_next(
    doc: *const Editor,
    frame: u32,
    state: *mut PlayState,
    within_tag: bool,
) -> u32 {
    crate::ffi::guarded("pixmith_play_next", std::ptr::null_mut(), || {
        let (Some(e), Some(st)) = (doc.as_ref(), state.as_mut()) else { return frame };
        e.sprite.next_play_frame(frame as usize, st, within_tag) as u32
    })
}

/// Fills `out` with `Sprite::cel_grid` (layer-major, one byte per cel).
#[no_mangle]
pub unsafe extern "C" fn pixmith_cel_grid(doc: *const Editor, out: *mut u8, len: usize) -> bool {
    crate::ffi::guarded("pixmith_cel_grid", std::ptr::null_mut(), || {
        let Some(e) = crate::ffi::synced(doc) else { return false };
        let grid = e.sprite.cel_grid();
        if out.is_null() || len < grid.len() {
            return false;
        }
        std::ptr::copy_nonoverlapping(grid.as_ptr(), out, grid.len());
        true
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_cels_clear(doc: *mut Editor, l0: u32, l1: u32, f0: u32, f1: u32) -> bool {
    crate::ffi::guarded("pixmith_cels_clear", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return false };
        undoable(e, |s| s.clear_cels((l0 as usize, l1 as usize), (f0 as usize, f1 as usize)).then_some(())).is_some()
    })
}

/// Copies cels to the cel clipboard (not an edit).
#[no_mangle]
pub unsafe extern "C" fn pixmith_cels_copy(doc: *const Editor, l0: u32, l1: u32, f0: u32, f1: u32) -> bool {
    crate::ffi::guarded("pixmith_cels_copy", std::ptr::null_mut(), || {
        let Some(e) = doc.as_ref() else { return false };
        let ok = e.sprite.copy_cels((l0 as usize, l1 as usize), (f0 as usize, f1 as usize));
        if ok {
            CLIP.with(|c| {
                if let Some(clip) = c.borrow_mut().as_mut() {
                    clip.source = Some(e.uid);
                }
            });
        }
        ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_cels_has_clip() -> bool {
    crate::ffi::guarded("pixmith_cels_has_clip", std::ptr::null_mut(), || CLIP.with(|c| c.borrow().is_some()))
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_cels_paste(doc: *mut Editor, layer: u32, frame: u32) -> bool {
    crate::ffi::guarded("pixmith_cels_paste", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return false };
        let uid = e.uid;
        undoable(e, |s| s.paste_cels_as(layer as usize, frame as usize, Some(uid)).then_some(())).is_some()
    })
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_cels_move(
    doc: *mut Editor,
    l0: u32,
    l1: u32,
    f0: u32,
    f1: u32,
    dl: i32,
    df: i32,
    copy: bool,
) -> bool {
    crate::ffi::guarded("pixmith_cels_move", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return false };
        undoable(e, |s| {
            s.move_cels((l0 as usize, l1 as usize), (f0 as usize, f1 as usize), dl as isize, df as isize, copy)
                .then_some(())
        })
        .is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::ase;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn px(c: [u8; 4]) -> Sprite {
        let mut s = Sprite::new(1, 1);
        s.draw_line(0, 0, 0, 0, 0, 0, c, 1);
        s
    }

    fn names(s: &Sprite) -> Vec<(String, u16)> {
        s.layers.iter().map(|l| (l.name.clone(), l.child_level)).collect()
    }

    fn near(a: &[u8], b: &[u8]) -> bool {
        a.iter().zip(b).all(|(x, y)| x.abs_diff(*y) <= 1)
    }

    /// Two opaque layers on a 1x1 sprite; returns the result of blending `top` over `bottom`.
    fn blend(bottom: [u8; 4], top: [u8; 4], mode: u16) -> Vec<u8> {
        let mut d = bottom;
        blend_pixel(&mut d, &top, 255, mode);
        d.to_vec()
    }

    #[test]
    fn separable_blend_modes() {
        let (b, s) = ([200, 100, 50, 255], [100, 200, 255, 255]);
        assert!(near(&blend(b, s, 0), &s));
        assert!(near(&blend(b, s, 1), &[78, 78, 50, 255]));
        assert!(near(&blend(b, s, 2), &[222, 222, 255, 255]));
        assert!(near(&blend(b, s, 4), &[100, 100, 50, 255]));
        assert!(near(&blend(b, s, 5), &[200, 200, 255, 255]));
        assert!(near(&blend(b, s, 10), &[100, 100, 205, 255]));
        assert!(near(&blend(b, s, 16), &[255, 255, 255, 255]));
        assert!(near(&blend(b, s, 17), &[100, 0, 0, 255]));
        assert!(near(&blend(b, s, 18), &[255, 128, 50, 255]));
        // Overlay is hard light with the layers swapped.
        assert_eq!(blend(b, s, 3), blend(s, b, 8));
        assert!(near(&blend(b, s, 11), &[143, 143, 205, 255]));
    }

    #[test]
    fn dodge_burn_and_soft_light_edges() {
        let gray = [128, 128, 128, 255];
        assert!(near(&blend(gray, [0, 0, 0, 255], 6), &[128, 128, 128, 255]));
        assert!(near(&blend(gray, [255, 255, 255, 255], 6), &[255, 255, 255, 255]));
        assert!(near(&blend(gray, [255, 255, 255, 255], 7), &[128, 128, 128, 255]));
        assert!(near(&blend(gray, [0, 0, 0, 255], 7), &[0, 0, 0, 255]));
        // Soft light with a mid-gray source leaves the backdrop unchanged.
        assert!(near(&blend([40, 120, 220, 255], [128, 128, 128, 255], 9), &[40, 120, 220, 255]));
        assert!(blend([100, 100, 100, 255], [255, 255, 255, 255], 9)[0] > 100);
        assert!(blend([100, 100, 100, 255], [0, 0, 0, 255], 9)[0] < 100);
    }

    #[test]
    fn non_separable_blend_modes() {
        let (b, s) = ([200, 100, 50, 255], [30, 90, 250, 255]);
        let lum = |c: &[u8]| 0.3 * c[0] as f32 + 0.59 * c[1] as f32 + 0.11 * c[2] as f32;
        let sat = |c: &[u8]| (c[..3].iter().max().unwrap() - c[..3].iter().min().unwrap()) as f32;
        let color = blend(b, s, 14);
        assert!((lum(&color) - lum(&b)).abs() < 2.0);
        let luminosity = blend(b, s, 15);
        assert!((lum(&luminosity) - lum(&s)).abs() < 2.0);
        let hue = blend(b, s, 12);
        assert!((sat(&hue) - sat(&b)).abs() < 3.0 && (lum(&hue) - lum(&b)).abs() < 2.0);
        let saturation = blend(b, s, 13);
        assert!((sat(&saturation) - sat(&s)).abs() < 3.0 && (lum(&saturation) - lum(&b)).abs() < 2.0);
        // A gray source has no hue to apply: color mode yields gray with the backdrop's luminance.
        let c = blend(b, [128, 128, 128, 255], 14);
        assert!(c[0].abs_diff(c[1]) <= 1 && c[1].abs_diff(c[2]) <= 1);
    }

    #[test]
    fn blend_with_alpha() {
        // Transparent backdrop: the source passes through.
        assert_eq!(blend([9, 9, 9, 0], [10, 20, 30, 128], 1), vec![10, 20, 30, 128]);
        // Zero-alpha source changes nothing.
        assert_eq!(blend([9, 8, 7, 255], [10, 20, 30, 0], 1), vec![9, 8, 7, 255]);
        // Half-opaque multiply of white over opaque red keeps red.
        let mut d = RED;
        blend_pixel(&mut d, &[255, 255, 255, 255], 128, 1);
        assert_eq!(d, RED);
        let mut d = [255, 255, 255, 255];
        blend_pixel(&mut d, &RED, 128, 1);
        assert!(near(&d, &[255, 127, 127, 255]));
    }

    #[test]
    fn render_uses_layer_blend_mode_and_group_opacity() {
        let mut s = px([100, 200, 50, 255]);
        let top = s.add_layer(0);
        s.draw_line(top, 0, 0, 0, 0, 0, [128, 128, 128, 255], 1);
        s.layers[top].blend_mode = 1;
        assert!(near(&s.render(0), &[50, 100, 25, 255]));
        s.layers[top].blend_mode = 99; // unknown acts as normal
        assert_eq!(s.render(0), vec![128, 128, 128, 255]);

        let mut g = Sprite::new(1, 1);
        let mut group = Layer::new("g");
        group.kind = 1;
        group.opacity = 128;
        let mut child = Layer::new("c");
        child.child_level = 1;
        child.opacity = 128;
        g.layers = vec![group, child];
        g.cels = vec![vec![None, None]];
        g.draw_line(1, 0, 0, 0, 0, 0, RED, 1);
        assert_eq!(g.effective_opacity(1), 64);
        assert_eq!(g.render(0), vec![255, 0, 0, 64]);
    }

    /// Layers (bottom to top): a, G{b, H{c}, d}, e
    fn tree() -> Sprite {
        let mut s = Sprite::new(1, 1);
        let spec = [
            ("a", 0, false),
            ("G", 0, true),
            ("b", 1, false),
            ("H", 1, true),
            ("c", 2, false),
            ("d", 1, false),
            ("e", 0, false),
        ];
        s.layers = spec
            .iter()
            .map(|&(n, level, group)| {
                let mut l = Layer::new(n);
                l.child_level = level;
                l.kind = group as u16;
                l
            })
            .collect();
        s.cels = vec![vec![None; spec.len()]];
        s
    }

    fn order(s: &Sprite) -> String {
        s.layers.iter().map(|l| l.name.as_str()).collect()
    }

    fn n(name: &str, level: u16) -> (String, u16) {
        (name.to_string(), level)
    }

    #[test]
    fn move_layer_keeps_groups_together() {
        let mut s = tree();
        assert_eq!(s.move_layer(1, true), Some(2)); // G (with children) above e
        assert_eq!(order(&s), "aeGbHcd");
        assert_eq!(names(&s)[2..], [n("G", 0), n("b", 1), n("H", 1), n("c", 2), n("d", 1)]);
        assert_eq!(s.move_layer(2, false), Some(1));
        assert_eq!(order(&s), "aGbHcde");
        // Moving down past a group skips all of its children.
        assert_eq!(s.move_layer(6, false), Some(1));
        assert_eq!(order(&s), "aeGbHcd");
        // Siblings inside a group swap without leaving it.
        let mut s = tree();
        assert_eq!(s.move_layer(5, false), Some(3));
        assert_eq!(order(&s), "aGbdHce");
        assert_eq!(names(&s)[3], n("d", 1));
        // The top layer of a group moves out above the group.
        let mut s = tree();
        assert_eq!(s.move_layer(5, true), Some(5));
        assert_eq!(order(&s), "aGbHcde");
        assert_eq!(names(&s)[5], n("d", 0));
        // The bottom layer of a group moves out below it.
        let mut s = tree();
        assert_eq!(s.move_layer(2, false), Some(1));
        assert_eq!(order(&s), "abGHcde");
        assert_eq!(s.layers[1].child_level, 0);
        // Edges of the stack.
        assert_eq!(tree().move_layer(6, true), None);
        assert_eq!(tree().move_layer(0, false), None);
    }

    #[test]
    fn move_layer_moves_cels_too() {
        let mut s = px(RED);
        let top = s.add_layer(0);
        s.draw_line(top, 0, 0, 0, 0, 0, BLUE, 1);
        assert_eq!(s.render(0), BLUE.to_vec());
        assert_eq!(s.move_layer(top, false), Some(0));
        assert_eq!(s.render(0), RED.to_vec());
    }

    #[test]
    fn group_membership_changes() {
        let mut s = tree();
        assert_eq!(s.move_into_group(0, 1), Some(5)); // a -> top of G
        assert_eq!(order(&s), "GbHcdae");
        assert_eq!(s.layers[5].child_level, 1);
        // A layer above joins a nested group at its top.
        let mut s = tree();
        assert_eq!(s.move_into_group(6, 3), Some(5));
        assert_eq!(order(&s), "aGbHced");
        assert_eq!(s.layers[5].child_level, 2);
        // Leaving a group lands directly above it.
        let mut s = tree();
        assert_eq!(s.move_out_of_group(4), Some(4));
        assert_eq!(names(&s)[4], n("c", 1));
        assert_eq!(s.move_out_of_group(0), None);
        // Cannot move a group into itself or its descendants, or into a non-group.
        let mut s = tree();
        assert_eq!(s.move_into_group(1, 3), None);
        assert_eq!(s.move_into_group(1, 1), None);
        assert_eq!(s.move_into_group(0, 2), None);
        assert_eq!(s.move_into_group(3, 1), None); // H is already in G
                                                   // Moving a group re-levels its children.
        assert_eq!(s.move_out_of_group(3), Some(4));
        assert_eq!(order(&s), "aGbdHce");
        assert_eq!(names(&s)[4..], [n("H", 0), n("c", 1), n("e", 0)]);
    }

    #[test]
    fn duplicate_layer_is_independent() {
        let mut s = px(RED);
        s.add_frame(0, true);
        let copy = s.duplicate_layer(0).unwrap();
        assert_eq!(copy, 1);
        assert_eq!(s.layers[1].name, "Layer 1 copy");
        s.draw_line(1, 0, 0, 0, 0, 0, BLUE, 1);
        assert_eq!(s.cels[0][0].as_ref().unwrap().pixels, RED.to_vec());
        assert_eq!(s.render(1), RED.to_vec());
        // A group is copied with its children.
        let mut t = tree();
        assert_eq!(t.duplicate_layer(1), Some(6));
        assert_eq!(order(&t), "aGbHcdG copybHcde");
        assert_eq!(t.layers[6].name, "G copy");
        assert_eq!(t.cels[0].len(), 12);
    }

    #[test]
    fn merge_down_preserves_render() {
        let mut s = px([200, 100, 50, 255]);
        let top = s.add_layer(0);
        s.draw_line(top, 0, 0, 0, 0, 0, [100, 200, 255, 200], 1);
        s.layers[top].blend_mode = 1;
        s.layers[top].opacity = 180;
        let before = s.render(0);
        assert_eq!(s.merge_down(top), Some(0));
        assert_eq!(s.layers.len(), 1);
        assert_eq!(s.cels[0].len(), 1);
        assert!(near(&s.render(0), &before), "{:?} vs {:?}", s.render(0), before);
        // Groups and the bottom layer can't be merged.
        assert_eq!(s.merge_down(0), None);
        assert_eq!(tree().merge_down(1), None);
        assert_eq!(tree().merge_down(2), None);
    }

    #[test]
    fn merge_down_into_empty_cel() {
        let mut s = Sprite::new(1, 1);
        let top = s.add_layer(0);
        s.draw_line(top, 0, 0, 0, 0, 0, BLUE, 1);
        assert_eq!(s.merge_down(top), Some(0));
        assert_eq!(s.render(0), BLUE.to_vec());
    }

    #[test]
    fn groups_can_be_added_and_wrapped() {
        let mut s = tree();
        assert_eq!(s.add_group(1), 6); // above G and its children
        assert!(s.layers[6].is_group());
        assert_eq!(s.layers[6].child_level, 0);
        assert_eq!(s.cels[0].len(), 8);
        let mut s = tree();
        assert_eq!(s.group_layer(3), Some(3)); // wrap H
        assert_eq!(names(&s)[2..6], [n("b", 1), n("Group 3", 1), n("H", 2), n("c", 3)]);
        assert_eq!(s.parent_layer(4), Some(3));
    }

    #[test]
    fn move_frame_reorders_content() {
        let mut s = px(RED);
        s.add_frame(0, false);
        s.draw_line(0, 1, 0, 0, 0, 0, BLUE, 1);
        s.durations = vec![10, 20];
        assert!(s.move_frame(1, 0));
        assert_eq!(s.render(0), BLUE.to_vec());
        assert_eq!(s.render(1), RED.to_vec());
        assert_eq!(s.durations, vec![20, 10]);
        assert!(!s.move_frame(0, 0));
        assert!(!s.move_frame(0, 5));
    }

    #[test]
    fn tags_are_edited_and_clamped() {
        let mut s = Sprite::new(1, 1);
        for _ in 0..3 {
            s.add_frame(0, false);
        }
        let t = s.add_tag(3, 1);
        assert_eq!((s.tags[t].from, s.tags[t].to), (1, 3));
        assert!(s.update_tag(t, "run", 2, 99, 9, 4, [1, 2, 3]));
        assert_eq!(
            (s.tags[t].name.as_str(), s.tags[t].from, s.tags[t].to, s.tags[t].direction, s.tags[t].repeat),
            ("run", 2, 3, 3, 4)
        );
        assert!(!s.update_tag(7, "x", 0, 0, 0, 0, [0; 3]));
        assert!(s.remove_tag(t) && s.tags.is_empty() && !s.remove_tag(0));
    }

    fn walk(s: &Sprite, start: usize, steps: usize, within: bool) -> Vec<usize> {
        let mut st = PlayState::default();
        let mut f = start;
        (0..steps)
            .map(|_| {
                f = s.next_play_frame(f, &mut st, within);
                f
            })
            .collect()
    }

    #[test]
    fn playback_honors_tag_direction() {
        let mut s = Sprite::new(1, 1);
        for _ in 0..5 {
            s.add_frame(0, false);
        }
        let t = s.add_tag(1, 3);
        assert_eq!(walk(&s, 0, 8, true), vec![1, 2, 3, 1, 2, 3, 1, 2]);
        assert_eq!(walk(&s, 2, 6, true), vec![3, 1, 2, 3, 1, 2]);
        s.tags[t].direction = 1;
        assert_eq!(walk(&s, 2, 6, true), vec![1, 3, 2, 1, 3, 2]);
        s.tags[t].direction = 2;
        assert_eq!(walk(&s, 1, 7, true), vec![2, 3, 2, 1, 2, 3, 2]);
        s.tags[t].direction = 3;
        assert_eq!(walk(&s, 3, 7, true), vec![2, 1, 2, 3, 2, 1, 2]);
        // Ignoring tags plays every frame.
        assert_eq!(walk(&s, 2, 5, false), vec![3, 4, 5, 0, 1]);
        // A one-frame ping-pong tag stays put.
        s.tags[t].from = 2;
        s.tags[t].to = 2;
        s.tags[t].direction = 2;
        assert_eq!(walk(&s, 2, 2, true), vec![2, 2]);
    }

    #[test]
    fn add_frame_copies_are_independent_but_links_share() {
        let mut s = px(RED);
        s.add_frame(0, true);
        assert!(!s.is_cel_linked(0, 0));
        s.draw_line(0, 1, 0, 0, 0, 0, BLUE, 1);
        assert_eq!(s.render(0), RED.to_vec());
        assert_eq!(s.render(1), BLUE.to_vec());

        s.add_frame(1, false);
        assert!(s.link_cels(0, 0, 2));
        assert!(s.is_cel_linked(0, 0) && s.is_cel_linked(0, 2));
        assert_eq!(s.render(2), RED.to_vec());
        // Editing one linked cel updates all of them.
        s.draw_line(0, 2, 0, 0, 0, 0, BLUE, 1);
        assert_eq!(s.render(0), BLUE.to_vec());
        assert!(s.is_cel_linked(0, 0));
        // Opacity is shared as well.
        assert!(s.set_cel_opacity(0, 1, 100));
        assert!(s.set_cel_opacity(0, 0, 77));
        assert_eq!(s.cels[2][0].as_ref().unwrap().opacity, 77);
        assert_eq!(s.cels[1][0].as_ref().unwrap().opacity, 77);
        // Unlinking detaches one cel.
        assert!(s.unlink_cel(0, 2));
        assert!(!s.is_cel_linked(0, 2));
        s.draw_line(0, 2, 0, 0, 0, 0, RED, 1);
        assert_eq!(s.cels[0][0].as_ref().unwrap().pixels, BLUE.to_vec());
        assert!(s.unlink_cel(0, 1));
        assert!(!s.unlink_cel(0, 0));
    }

    #[test]
    fn linked_edit_through_cel_mut_is_flushed() {
        let mut s = px(RED);
        s.add_frame(0, false);
        assert!(s.link_cels(0, 0, 1));
        // Not via draw_line: the edit is finished by the next flush.
        s.cel_mut(0, 1).unwrap().pixels[..4].copy_from_slice(&BLUE);
        s.flush_links();
        assert_eq!(s.cels[0][0].as_ref().unwrap().pixels, BLUE.to_vec());
        assert!(s.is_cel_linked(0, 0));
        // Linking needs a source cel, and a range.
        let mut e = Sprite::new(1, 1);
        e.add_frame(0, false);
        assert!(!e.link_cels(0, 0, 1));
        assert!(!px(RED).link_cels(0, 0, 0));
    }

    #[test]
    fn stroke_on_a_linked_cel_copies_the_buffer_once() {
        let mut s = Sprite::new(32, 1);
        s.add_frame(0, false);
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        assert!(s.link_cels(0, 0, 1));
        crate::document::CEL_CLONES.with(|c| c.set(0));
        let mut st = crate::paint::shapes::Stroke::begin(&mut s, 0, 1, (0, 0), BLUE, 1, 0);
        for x in 1..20 {
            st.to(&mut s, (x, 0));
        }
        assert_eq!(crate::document::CEL_CLONES.with(|c| c.get()), 1);
        // The sibling already shows the stroke, and the link is restored by the next flush.
        assert_eq!(s.render(0)[19 * 4..20 * 4], BLUE);
        s.flush_links();
        assert!(s.is_cel_linked(0, 0));
        assert_eq!(s.cels[0][0].as_ref().unwrap().pixels[19 * 4..20 * 4], BLUE);
    }

    #[test]
    fn unflushed_linked_edit_is_visible_in_renders_and_flushed_by_checkpoint() {
        let mut s = px(RED);
        s.add_frame(0, false);
        assert!(s.link_cels(0, 0, 1));
        s.cel_mut(0, 1).unwrap().pixels[..4].copy_from_slice(&BLUE);
        assert_eq!(s.render(0), BLUE.to_vec());
        assert_eq!(s.render(1), BLUE.to_vec());
        let mut e = crate::document::Editor::new(s);
        e.checkpoint();
        assert_eq!(e.sprite.cels[0][0].as_ref().unwrap().pixels, BLUE.to_vec());
        assert!(e.sprite.links.0.is_none());
    }

    #[test]
    fn linked_cels_round_trip_through_files() {
        let mut s = px(RED);
        s.add_frame(0, true); // independent copy
        s.add_frame(1, false);
        s.add_frame(2, false);
        s.draw_line(0, 2, 0, 0, 0, 0, BLUE, 1);
        assert!(s.link_cels(0, 2, 3));
        let top = s.add_layer(0);
        s.draw_line(top, 0, 0, 0, 0, 0, BLUE, 1);
        s.durations = vec![100, 90, 80, 70];

        let loaded = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(loaded, s);
        let same = |a: usize, b: usize, l: usize| {
            Arc::ptr_eq(loaded.cels[a][l].as_ref().unwrap(), loaded.cels[b][l].as_ref().unwrap())
        };
        assert!(same(2, 3, 0), "linked cels stay linked");
        assert!(!same(0, 1, 0), "duplicated frames stay independent");
        assert!(!same(1, 2, 0));
        // And they stay linked when edited after loading.
        let mut loaded = loaded;
        loaded.draw_line(0, 3, 0, 0, 0, 0, RED, 1);
        assert_eq!(loaded.render(2), RED.to_vec());
        assert!(loaded.is_cel_linked(0, 2));
    }

    #[test]
    fn blend_mode_and_groups_round_trip() {
        let mut s = tree();
        s.layers[2].blend_mode = 9;
        s.layers[2].opacity = 77;
        s.layers[1].opacity = 90;
        s.layers[1].name = "group".into();
        s.move_layer(1, true);
        s.tags.push(Tag {
            name: "t".into(),
            from: 0,
            to: 0,
            direction: 3,
            repeat: 2,
            color: [9, 8, 7],
            user_data: Default::default(),
        });
        let loaded = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(loaded, s);
    }

    #[test]
    fn editing_through_the_editor_is_undoable() {
        let mut e = Editor::new(tree());
        let before = e.sprite.clone();
        let r = undoable(&mut e, |s| s.move_layer(1, true));
        assert_eq!(r, Some(2));
        assert_ne!(e.sprite, before);
        assert!(e.undo());
        assert_eq!(e.sprite, before);
        // A failed edit leaves no undo step.
        assert_eq!(undoable(&mut e, |s| s.merge_down(0)), None);
        assert!(!e.undo());
    }

    #[test]
    fn playback_honors_tag_repeat() {
        let mut s = Sprite::new(1, 1);
        for _ in 0..5 {
            s.add_frame(0, false);
        }
        let t = s.add_tag(1, 3);
        s.tags[t].repeat = 2;
        // Two passes over the tag, then on to the next frames; entering the tag again restarts the count.
        assert_eq!(walk(&s, 1, 10, true), vec![2, 3, 1, 2, 3, 4, 5, 0, 1, 2]);
        s.tags[t].repeat = 1;
        assert_eq!(walk(&s, 1, 5, true), vec![2, 3, 4, 5, 0]);
        // Ping-pong counts a full there-and-back as one pass.
        s.tags[t].repeat = 2;
        s.tags[t].direction = 2;
        assert_eq!(walk(&s, 1, 9, true), vec![2, 3, 2, 1, 2, 3, 2, 1, 4]);
        // Reverse tags exit after the repeats too.
        s.tags[t].direction = 1;
        s.tags[t].repeat = 1;
        assert_eq!(walk(&s, 3, 5, true), vec![2, 1, 4, 5, 0]);
        // A one-frame tag counts each step as a pass.
        s.tags[t].from = 2;
        s.tags[t].to = 2;
        s.tags[t].direction = 0;
        s.tags[t].repeat = 3;
        assert_eq!(walk(&s, 2, 3, true), vec![2, 2, 3]);
        // Infinite (0) never leaves; ignoring tags ignores repeats.
        s.tags[t].repeat = 0;
        assert_eq!(walk(&s, 2, 4, true), vec![2, 2, 2, 2]);
        s.tags[t].repeat = 1;
        assert_eq!(walk(&s, 2, 3, false), vec![3, 4, 5]);
    }

    /// 1x1 sprite with the given layers (level, kind, opacity, blend, cel color).
    fn composed(spec: &[(u16, u16, u8, u16, Option<[u8; 4]>)]) -> Sprite {
        let mut s = Sprite::new(1, 1);
        s.layers = spec
            .iter()
            .enumerate()
            .map(|(i, &(level, kind, opacity, blend, _))| {
                let mut l = Layer::new(format!("l{i}"));
                l.child_level = level;
                l.kind = kind;
                l.opacity = opacity;
                l.blend_mode = blend;
                l
            })
            .collect();
        s.cels = vec![vec![None; spec.len()]];
        for (i, &(_, _, _, _, color)) in spec.iter().enumerate() {
            if let Some(c) = color {
                s.draw_line(i, 0, 0, 0, 0, 0, c, 1);
            }
        }
        s
    }

    #[test]
    fn group_blend_mode_applies_to_the_composite() {
        // background, then a multiply group holding one gray layer
        let s = composed(&[
            (0, 0, 255, 0, Some([100, 200, 50, 255])),
            (0, 1, 255, 1, None),
            (1, 0, 255, 0, Some([128, 128, 128, 255])),
        ]);
        assert!(near(&s.render(0), &[50, 100, 25, 255]));
        // The group's opacity scales the whole composite.
        let mut s = composed(&[
            (0, 0, 255, 0, Some([100, 200, 50, 255])),
            (0, 1, 128, 1, None),
            (1, 0, 255, 0, Some([128, 128, 128, 255])),
        ]);
        assert!(near(&s.render(0), &[75, 150, 37, 255]), "{:?}", s.render(0));
        // A hidden group draws nothing, whatever its mode.
        s.layers[1].visible = false;
        assert_eq!(s.render(0), vec![100, 200, 50, 255]);
    }

    #[test]
    fn groups_are_isolated_composites() {
        // Ungrouped, multiply sees the red background and turns it black.
        let flat = composed(&[(0, 0, 255, 0, Some(RED)), (0, 0, 255, 1, Some(BLUE))]);
        assert_eq!(flat.render(0), vec![0, 0, 0, 255]);
        // Inside a group the multiply layer only sees the group's own content, so the group is plain blue.
        let grouped = composed(&[(0, 0, 255, 0, Some(RED)), (0, 1, 255, 0, None), (1, 0, 255, 1, Some(BLUE))]);
        assert_eq!(grouped.render(0), BLUE.to_vec());
        // Group opacity applies once to the composite, not to each child.
        let s = composed(&[(0, 1, 128, 0, None), (1, 0, 255, 0, Some(RED)), (1, 0, 255, 0, Some(BLUE))]);
        assert_eq!(s.render(0), vec![0, 0, 255, 128]);
        // A normal, opaque group of normal layers renders like the same layers without the group.
        let a = composed(&[
            (0, 0, 255, 0, Some(RED)),
            (0, 1, 255, 0, None),
            (1, 0, 100, 0, Some([0, 255, 0, 200])),
            (1, 0, 255, 0, Some([0, 0, 255, 90])),
        ]);
        let b = composed(&[
            (0, 0, 255, 0, Some(RED)),
            (0, 0, 100, 0, Some([0, 255, 0, 200])),
            (0, 0, 255, 0, Some([0, 0, 255, 90])),
        ]);
        assert_eq!(a.render(0), b.render(0));
        // Nested groups: a half-transparent inner group inside an outer multiply group.
        let n = composed(&[
            (0, 0, 255, 0, Some([200, 200, 200, 255])),
            (0, 1, 255, 1, None),
            (1, 1, 128, 0, None),
            (2, 0, 255, 0, Some([100, 100, 100, 255])),
        ]);
        let px = n.render(0);
        assert!(px[..3].iter().all(|&v| v.abs_diff(139) <= 2) && px[3] == 255, "{px:?}");
    }

    fn grid_sprite() -> Sprite {
        let mut s = Sprite::new(1, 1);
        s.add_layer(0);
        s.add_layer(1);
        s.add_frame(0, false);
        s.add_frame(1, false);
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        s.draw_line(1, 1, 0, 0, 0, 0, BLUE, 1);
        s.cels[2][0] = Some(Arc::new(Cel { opacity: 255, pixels: vec![0; 4], ..Default::default() }));
        s
    }

    #[test]
    fn cel_grid_reports_content_and_links() {
        let mut s = grid_sprite();
        let nf = s.frame_count();
        let g = s.cel_grid();
        assert_eq!(g.len(), 9);
        assert_eq!(g[0], 3); // cel with content
        assert_eq!(g[1], 0); // none
        assert_eq!(g[2], 1); // empty cel
        assert_eq!(g[nf + 1], 3);
        assert!(s.link_cels(0, 0, 1));
        let g = s.cel_grid();
        assert_eq!((g[0], g[1]), (7, 7));
        assert_eq!(g[nf + 1], 3);
    }

    #[test]
    fn cels_move_copy_and_clear() {
        let mut s = grid_sprite();
        // Move the blue cel from layer 1 / frame 1 to layer 2 / frame 2.
        assert!(s.move_cels((1, 1), (1, 1), 1, 1, false));
        assert!(s.cels[1][1].is_none());
        assert_eq!(s.cels[2][2].as_ref().unwrap().pixels, BLUE.to_vec());
        // Out of range and no-op moves fail without changes.
        let before = s.clone();
        assert!(!s.move_cels((2, 2), (2, 2), 1, 0, false));
        assert!(!s.move_cels((2, 2), (2, 2), -3, 0, false));
        assert!(!s.move_cels((0, 0), (0, 0), 0, 0, false));
        assert_eq!(s, before);
        // Copying leaves the source and makes an independent cel.
        assert!(s.move_cels((0, 0), (0, 0), 0, 1, true));
        assert!(s.cels[0][0].is_some() && s.cels[1][0].is_some());
        assert!(!s.is_cel_linked(0, 0));
        // Moving a whole rectangle keeps its shape.
        assert!(s.move_cels((0, 0), (0, 1), 1, 1, false));
        assert!(s.cels[0][0].is_none() && s.cels[1][0].is_none());
        assert_eq!(s.cels[1][1].as_ref().unwrap().pixels, RED.to_vec());
        assert_eq!(s.cels[2][1].as_ref().unwrap().pixels, RED.to_vec());
        // A move onto a group layer is refused.
        let mut g = tree();
        g.add_frame(0, false);
        g.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        assert!(!g.move_cels((0, 0), (0, 0), 1, 0, false));
        assert!(g.move_cels((0, 0), (0, 0), 0, 1, false));
        // Clearing.
        assert!(s.clear_cels((0, 2), (0, 2)));
        assert!(s.cel_grid().iter().all(|&b| b == 0));
        assert!(!s.clear_cels((0, 2), (0, 2)));
        assert!(!s.clear_cels((0, 9), (0, 0)));
    }

    #[test]
    fn cels_copy_paste_and_undo() {
        let mut e = Editor::new(grid_sprite());
        assert!(e.sprite.copy_cels((0, 1), (0, 1)));
        // Pasting at layer 1 / frame 1 puts layer 0's cels on layer 1 and layer 1's on layer 2.
        let before = e.sprite.clone();
        assert_eq!(undoable(&mut e, |s| s.paste_cels(1, 1).then_some(())), Some(()));
        let cel = |e: &Editor, l: usize, f: usize| e.sprite.cels[f][l].as_ref().map(|c| c.pixels.clone());
        assert_eq!(cel(&e, 1, 1), Some(RED.to_vec()));
        assert_eq!(cel(&e, 2, 2), Some(BLUE.to_vec()));
        assert_eq!(cel(&e, 2, 1), None);
        assert!(!e.sprite.is_cel_linked(1, 1));
        assert!(e.undo());
        assert_eq!(e.sprite, before);
        // One undo step per operation.
        assert_eq!(undoable(&mut e, |s| s.clear_cels((0, 0), (0, 0)).then_some(())), Some(()));
        assert_eq!(undoable(&mut e, |s| s.clear_cels((0, 0), (0, 0)).then_some(())), None);
        assert!(e.undo());
        assert_eq!(e.sprite, before);
        assert!(!e.undo());
        // Cels that would fall outside the sprite are dropped; group rows are skipped.
        assert!(e.sprite.paste_cels(2, 2));
        let mut t = tree();
        t.add_frame(0, false);
        t.paste_cels(1, 0);
        assert!(t.cels[0][1].is_none());
        let mut other = Sprite::new(2, 2);
        assert!(!other.paste_cels(0, 0));
    }
}
