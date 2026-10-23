//! Free transform of the selected pixels (or the whole cel): scale, rotate, skew and move with a
//! live preview, plus a RotSprite-style resampler for pixel art.
//!
//! A session lifts the selected pixels out of the cel. Every parameter change re-renders the lifted
//! pixels over the rest of the cel, so the document always shows the preview. The session starts
//! with one checkpoint: applying just ends the session, cancelling undoes that checkpoint.

use crate::document::{blend_over, Editor};
use crate::paint::selection::{cel_pixels, drawable, nonempty, px_index, Selection};
use std::f64::consts::PI;
use std::ffi::c_double;

type M2 = [[f64; 2]; 2];
type P2 = (f64, f64);

const MAX_SKEW: f64 = 80.0 * PI / 180.0;
const MIN_SCALE: f64 = 0.01;
/// Largest source area (pixels) the RotSprite resampler handles; bigger ones fall back to nearest.
const ROTSPRITE_MAX_AREA: usize = 1 << 18;

fn mul(a: M2, b: M2) -> M2 {
    [
        [a[0][0] * b[0][0] + a[0][1] * b[1][0], a[0][0] * b[0][1] + a[0][1] * b[1][1]],
        [a[1][0] * b[0][0] + a[1][1] * b[1][0], a[1][0] * b[0][1] + a[1][1] * b[1][1]],
    ]
}

fn apply(m: M2, v: P2) -> P2 {
    (m[0][0] * v.0 + m[0][1] * v.1, m[1][0] * v.0 + m[1][1] * v.1)
}

fn invert(m: M2) -> Option<M2> {
    let det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
    (det.abs() > 1e-12).then(|| [[m[1][1] / det, -m[0][1] / det], [-m[1][0] / det, m[0][0] / det]])
}

fn sub(a: P2, b: P2) -> P2 {
    (a.0 - b.0, a.1 - b.1)
}

fn add(a: P2, b: P2) -> P2 {
    (a.0 + b.0, a.1 + b.1)
}

fn norm_angle(a: f64) -> f64 {
    let a = a.rem_euclid(2.0 * PI);
    if a > PI {
        a - 2.0 * PI
    } else {
        a
    }
}

fn clamp_scale(s: f64) -> f64 {
    if s.abs() < MIN_SCALE {
        MIN_SCALE.copysign(if s == 0.0 { 1.0 } else { s })
    } else {
        s
    }
}

/// Handle directions `(hx, hy)` in drawing order: corners and edge midpoints.
pub const HANDLES: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/// Maps source point `v` to `dp + R * K * S * (v - sp)`: scale and skew first, then rotation, all
/// around the pivot, which sits at `sp` in the source and lands on `dp`. Coordinates are
/// continuous canvas coordinates (pixel corners are integers).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xform {
    /// Source box: x, y, width, height.
    pub src: (f64, f64, f64, f64),
    pub sp: P2,
    pub dp: P2,
    pub sx: f64,
    pub sy: f64,
    /// Rotation in radians, clockwise on screen.
    pub angle: f64,
    /// Skew angles in radians: x shifts with y, y shifts with x.
    pub kx: f64,
    pub ky: f64,
}

impl Xform {
    pub fn identity(x: f64, y: f64, w: f64, h: f64) -> Xform {
        let c = (x + w / 2.0, y + h / 2.0);
        Xform { src: (x, y, w, h), sp: c, dp: c, sx: 1.0, sy: 1.0, angle: 0.0, kx: 0.0, ky: 0.0 }
    }

    fn rot(&self) -> M2 {
        let (s, c) = self.angle.sin_cos();
        [[c, -s], [s, c]]
    }

    fn rk(&self) -> M2 {
        mul(self.rot(), [[1.0, self.kx.tan()], [self.ky.tan(), 1.0]])
    }

    fn lin(&self) -> M2 {
        mul(self.rk(), [[self.sx, 0.0], [0.0, self.sy]])
    }

    pub fn forward(&self, v: P2) -> P2 {
        add(apply(self.lin(), sub(v, self.sp)), self.dp)
    }

    pub fn inverse(&self, p: P2) -> Option<P2> {
        Some(add(apply(invert(self.lin())?, sub(p, self.dp)), self.sp))
    }

    /// Source point of a handle (or the box center for `(0, 0)`).
    pub fn handle_src(&self, (hx, hy): (i32, i32)) -> P2 {
        let (x, y, w, h) = self.src;
        (x + (hx + 1) as f64 / 2.0 * w, y + (hy + 1) as f64 / 2.0 * h)
    }

    pub fn handle(&self, h: (i32, i32)) -> P2 {
        self.forward(self.handle_src(h))
    }

    /// Corners in the order top-left, top-right, bottom-right, bottom-left.
    pub fn corners(&self) -> [P2; 4] {
        [(-1, -1), (1, -1), (1, 1), (-1, 1)].map(|h| self.handle(h))
    }

    /// Bounding box of the transformed box: min x, min y, max x, max y.
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        let c = self.corners();
        let f =
            |g: fn(&P2) -> f64| (c.iter().map(g).fold(f64::MAX, f64::min), c.iter().map(g).fold(f64::MIN, f64::max));
        let (x0, x1) = f(|p| p.0);
        let (y0, y1) = f(|p| p.1);
        (x0, y0, x1, y1)
    }

    /// Changes the destination so that source point `a` lands on `at`.
    fn pin(&mut self, a: P2, at: P2) {
        self.dp = sub(at, apply(self.lin(), sub(a, self.sp)));
    }

    pub fn translate(&self, d: P2) -> Xform {
        Xform { dp: add(self.dp, d), ..*self }
    }

    /// Drags a scale handle to `p`, keeping the opposite side fixed.
    pub fn scale_handle(&self, h: (i32, i32), p: P2, keep_aspect: bool) -> Xform {
        let a = self.handle_src((-h.0, -h.1));
        let d_a = self.forward(a);
        let Some(inv) = invert(self.rk()) else { return *self };
        let q = apply(inv, sub(p, d_a));
        let t = sub(self.handle_src(h), a);
        let mut sx = if h.0 != 0 && t.0.abs() > 1e-9 { q.0 / t.0 } else { self.sx };
        let mut sy = if h.1 != 0 && t.1.abs() > 1e-9 { q.1 / t.1 } else { self.sy };
        if keep_aspect {
            let (fx, fy) = ((sx / self.sx).abs(), (sy / self.sy).abs());
            let f = match (h.0 != 0, h.1 != 0) {
                (true, true) => fx.max(fy),
                (true, false) => fx,
                _ => fy,
            };
            sx = (self.sx.abs() * f).copysign(if h.0 != 0 { sx } else { self.sx });
            sy = (self.sy.abs() * f).copysign(if h.1 != 0 { sy } else { self.sy });
        }
        // Whole-pixel sizes along the dragged axes, whatever the rotation or skew.
        let whole = |s: f64, len: f64| {
            if len > 0.0 {
                clamp_scale(((s.abs() * len).round().max(1.0) / len).copysign(s))
            } else {
                s
            }
        };
        let (sx, sy) =
            (if h.0 != 0 { whole(sx, self.src.2) } else { sx }, if h.1 != 0 { whole(sy, self.src.3) } else { sy });
        let mut out = Xform { sx: clamp_scale(sx), sy: clamp_scale(sy), ..*self };
        out.pin(a, d_a);
        out
    }

    /// For a pure quarter-turn of a box with whole-pixel size, moves the result onto the pixel
    /// grid. A half-pixel offset (odd size difference) rounds toward the source box origin, like
    /// the plain quarter-turn operation of the selection tools.
    pub fn snapped(&self) -> Xform {
        let quarter = (self.angle / (PI / 2.0)).round();
        if self.kx != 0.0 || self.ky != 0.0 || (self.angle - quarter * PI / 2.0).abs() > 1e-9 {
            return *self;
        }
        let (x0, y0, x1, y1) = self.bbox();
        let whole = |v: f64| (v - v.round()).abs() < 1e-6;
        if !whole(x1 - x0) || !whole(y1 - y0) {
            return *self;
        }
        let round = |v: f64, toward: f64| {
            let f = v.floor();
            if ((v - f) - 0.5).abs() < 1e-9 {
                if toward <= f {
                    f
                } else {
                    f + 1.0
                }
            } else {
                v.round()
            }
        };
        let (rx, ry) = (round(x0, self.src.0), round(y0, self.src.1));
        self.translate((rx - x0, ry - y0))
    }

    /// Rotates by the angle the pointer swept around the pivot from `start` to `p`.
    pub fn rotate_by_drag(&self, start: P2, p: P2, snap: bool) -> Xform {
        let a0 = (start.1 - self.dp.1).atan2(start.0 - self.dp.0);
        let a1 = (p.1 - self.dp.1).atan2(p.0 - self.dp.0);
        let mut angle = self.angle + (a1 - a0);
        if snap {
            let step = 15f64.to_radians();
            angle = (angle / step).round() * step;
        }
        Xform { angle: norm_angle(angle), ..*self }.snapped()
    }

    /// Skews by dragging an edge handle: top/bottom edges skew x, left/right edges skew y.
    pub fn skew_handle(&self, h: (i32, i32), p: P2) -> Xform {
        if (h.0 == 0) == (h.1 == 0) {
            return *self;
        }
        let a = self.handle_src((-h.0, -h.1));
        let d_a = self.forward(a);
        let Some(inv) = invert(self.rot()) else { return *self };
        let q = apply(inv, sub(p, d_a));
        let t = sub(self.handle_src(h), a);
        let mut out = *self;
        if h.1 != 0 {
            let denom = self.sy * t.1;
            if denom.abs() > 1e-9 {
                out.kx = (q.0 / denom).atan().clamp(-MAX_SKEW, MAX_SKEW);
            }
        } else {
            let denom = self.sx * t.0;
            if denom.abs() > 1e-9 {
                out.ky = (q.1 / denom).atan().clamp(-MAX_SKEW, MAX_SKEW);
            }
        }
        out.pin(a, d_a);
        out
    }

    /// Moves the pivot to canvas point `q` without changing the result.
    pub fn move_pivot(&self, q: P2) -> Xform {
        let sp = self.inverse(q).unwrap_or(self.sp);
        Xform { sp, dp: q, ..*self }
    }

    /// Numeric fields: x, y, width, height of the bounding box, rotation and skews in degrees.
    pub fn fields(&self) -> [f64; 7] {
        let (x0, y0, x1, y1) = self.bbox();
        [x0, y0, x1 - x0, y1 - y0, self.angle.to_degrees(), self.kx.to_degrees(), self.ky.to_degrees()]
    }

    /// Sets one numeric field (see `fields`). Width and height set the scale (the box keeps its
    /// top-left corner); position moves the result.
    pub fn with_field(&self, field: u32, v: f64) -> Xform {
        self.with_field_anchored(field, v, (-1, -1))
    }

    /// Like `with_field`; width and height changes keep the box point `anchor` (a handle
    /// direction, `(0, 0)` for the center) where it is.
    pub fn with_field_anchored(&self, field: u32, v: f64, anchor: (i32, i32)) -> Xform {
        let mut out = *self;
        match field {
            0 | 1 => {
                let (x0, y0, ..) = self.bbox();
                out.dp = add(self.dp, if field == 0 { (v - x0, 0.0) } else { (0.0, v - y0) });
            }
            2 | 3 => {
                let (_, _, w, h) = self.src;
                let a = self.handle_src(anchor);
                let at = self.forward(a);
                let (bx0, by0, bx1, by1) = self.bbox();
                // Scale relative to the current bounding box size, which equals the scaled size
                // when there is no rotation or skew.
                if field == 2 {
                    let cur = bx1 - bx0;
                    out.sx = clamp_scale(if cur > 1e-9 { self.sx * v / cur } else { v / w.max(1.0) });
                } else {
                    let cur = by1 - by0;
                    out.sy = clamp_scale(if cur > 1e-9 { self.sy * v / cur } else { v / h.max(1.0) });
                }
                out.pin(a, at);
            }
            4 => out = Xform { angle: norm_angle(v.to_radians()), ..out }.snapped(),
            5 => out.kx = v.to_radians().clamp(-MAX_SKEW, MAX_SKEW),
            6 => out.ky = v.to_radians().clamp(-MAX_SKEW, MAX_SKEW),
            _ => {}
        }
        out
    }
}

// MARK: Resampling

fn pack(p: &[u8]) -> u32 {
    if p[3] == 0 {
        0
    } else {
        u32::from_be_bytes([p[0], p[1], p[2], p[3]])
    }
}

/// Scale2x / EPX: doubles an image, smoothing diagonal edges without inventing colors.
pub fn epx(src: &[u32], w: usize, h: usize) -> Vec<u32> {
    let at = |x: isize, y: isize, fallback: u32| -> u32 {
        if x < 0 || y < 0 || x >= w as isize || y >= h as isize {
            fallback
        } else {
            src[y as usize * w + x as usize]
        }
    };
    let mut out = vec![0u32; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (xi, yi) = (x as isize, y as isize);
            let p = src[y * w + x];
            let (a, b) = (at(xi, yi - 1, p), at(xi + 1, yi, p));
            let (c, d) = (at(xi - 1, yi, p), at(xi, yi + 1, p));
            let (mut p1, mut p2, mut p3, mut p4) = (p, p, p, p);
            if c == a && c != d && a != b {
                p1 = a;
            }
            if a == b && a != c && b != d {
                p2 = b;
            }
            if d == c && d != b && c != a {
                p3 = c;
            }
            if b == d && b != a && d != c {
                p4 = d;
            }
            let o = y * 2 * w * 2 + x * 2;
            out[o] = p1;
            out[o + 1] = p2;
            out[o + w * 2] = p3;
            out[o + w * 2 + 1] = p4;
        }
    }
    out
}

/// Source image upscaled 8x with three EPX passes. Transparent pixels are 0.
pub fn upscale8(src: &[u32], w: usize, h: usize) -> Vec<u32> {
    let a = epx(src, w, h);
    let b = epx(&a, w * 2, h * 2);
    epx(&b, w * 4, h * 4)
}

// MARK: Session

/// A free-transform session over one cel.
pub struct Session {
    layer: usize,
    frame: usize,
    /// Cel pixels with the lifted pixels cleared.
    base: Vec<u8>,
    lift: Vec<u8>,
    lmask: Vec<bool>,
    bw: usize,
    bh: usize,
    had_selection: bool,
    pub xf: Xform,
    drag: Option<Xform>,
    rotsprite: bool,
    /// Box point (handle direction) that typed width and height changes keep fixed.
    anchor: (i32, i32),
    up8: Option<Vec<u32>>,
}

/// Starts a transform of the selected pixels (or the whole cel) as one undo step.
pub fn begin(e: &mut Editor, layer: usize, frame: usize, rotsprite: bool) -> bool {
    if !drawable(&e.sprite, layer, frame) || e.sprite.cels[frame][layer].is_none() {
        return false;
    }
    let (w, h) = (e.sprite.width, e.sprite.height);
    let sel = e.sel.active(w, h).cloned();
    let (bx, by, bw, bh) = sel.as_ref().and_then(|s| s.bounds()).unwrap_or((0, 0, w as i32, h as i32));
    e.checkpoint();
    let mut base = cel_pixels(&e.sprite, layer, frame);
    let (bwu, bhu) = (bw as usize, bh as usize);
    let mut lift = vec![0u8; bwu * bhu * 4];
    let mut lmask = vec![false; bwu * bhu];
    for y in 0..bh {
        for x in 0..bw {
            if sel.as_ref().is_none_or(|s| s.get(bx + x, by + y)) {
                let (i, c) = ((y * bw + x) as usize, px_index(w, bx + x, by + y));
                lmask[i] = true;
                lift[i * 4..i * 4 + 4].copy_from_slice(&base[c..c + 4]);
                base[c..c + 4].fill(0);
            }
        }
    }
    e.sel.xform = Some(Session {
        layer,
        frame,
        base,
        lift,
        lmask,
        bw: bwu,
        bh: bhu,
        had_selection: sel.is_some(),
        xf: Xform::identity(bx as f64, by as f64, bw as f64, bh as f64),
        drag: None,
        rotsprite,
        anchor: (0, 0),
        up8: None,
    });
    true
}

pub fn active(e: &Editor) -> bool {
    e.sel.xform.is_some()
}

/// Cel (layer, frame) of the session in progress.
pub(crate) fn pending_target(e: &Editor) -> Option<(usize, usize)> {
    e.sel.xform.as_ref().map(|s| (s.layer, s.frame))
}

/// Keeps the previewed result.
pub fn commit(e: &mut Editor) {
    if let Some(t) = pending_target(e) {
        crate::paint::selection::sync_tiles(e, t.0, t.1);
    }
    e.sel.xform = None;
}

/// Discards the preview and restores the cel and selection from before `begin`, leaving no redo step.
pub fn cancel(e: &mut Editor) -> bool {
    e.sel.xform.is_some() && e.rollback()
}

/// Chooses which box point typed width and height changes keep fixed (`(0, 0)` is the center).
pub fn set_anchor(e: &mut Editor, anchor: (i32, i32)) {
    if let Some(s) = e.sel.xform.as_mut() {
        s.anchor = (anchor.0.clamp(-1, 1), anchor.1.clamp(-1, 1));
    }
}

fn with_session(e: &mut Editor, f: impl FnOnce(&mut Session)) {
    if let Some(s) = e.sel.xform.as_mut() {
        f(s);
        render(e);
    }
}

pub fn set_xform(e: &mut Editor, xf: Xform) {
    with_session(e, |s| s.xf = xf);
}

pub fn set_rotsprite(e: &mut Editor, on: bool) {
    with_session(e, |s| s.rotsprite = on);
}

pub fn current(e: &Editor) -> Option<Xform> {
    e.sel.xform.as_ref().map(|s| s.xf)
}

pub fn drag_begin(e: &mut Editor) {
    if let Some(s) = e.sel.xform.as_mut() {
        s.drag = Some(s.xf);
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Drag {
    Move,
    Scale { h: (i32, i32), keep_aspect: bool },
    Rotate { snap: bool },
    Skew { h: (i32, i32) },
    Pivot,
}

/// Applies a drag from `start` to `p` relative to the state at `drag_begin`.
pub fn drag(e: &mut Editor, kind: Drag, start: P2, p: P2) {
    with_session(e, |s| {
        let base = s.drag.unwrap_or(s.xf);
        s.xf = match kind {
            Drag::Move => base.translate(sub(p, start)),
            Drag::Scale { h, keep_aspect } => base.scale_handle(h, p, keep_aspect),
            Drag::Rotate { snap } => base.rotate_by_drag(start, p, snap),
            Drag::Skew { h } => base.skew_handle(h, p),
            Drag::Pivot => base.move_pivot(p),
        };
    });
}

// MARK: Rendering

/// Result pixel for destination pixel (x, y): color and whether it belongs to the selection.
fn sample_nearest(s: &Session, inv: &Xform, x: usize, y: usize) -> ([u8; 4], bool) {
    let Some(u) = inv.inverse((x as f64 + 0.5, y as f64 + 0.5)) else { return ([0; 4], false) };
    let (sx, sy) = ((u.0 - s.xf.src.0).floor(), (u.1 - s.xf.src.1).floor());
    if sx < 0.0 || sy < 0.0 || sx >= s.bw as f64 || sy >= s.bh as f64 {
        return ([0; 4], false);
    }
    let i = sy as usize * s.bw + sx as usize;
    if !s.lmask[i] {
        return ([0; 4], false);
    }
    (s.lift[i * 4..i * 4 + 4].try_into().unwrap(), true)
}

fn sample_rotsprite(s: &Session, up: &[u32], x: usize, y: usize) -> ([u8; 4], bool) {
    let (uw, uh) = (s.bw * 8, s.bh * 8);
    let mut votes = [0u32; 64];
    let mut in_mask = 0;
    for j in 0..8 {
        for i in 0..8 {
            let p = (x as f64 + (i as f64 + 0.5) / 8.0, y as f64 + (j as f64 + 0.5) / 8.0);
            let Some(u) = s.xf.inverse(p) else { continue };
            let (ux, uy) = (((u.0 - s.xf.src.0) * 8.0).floor(), ((u.1 - s.xf.src.1) * 8.0).floor());
            if ux < 0.0 || uy < 0.0 || ux >= uw as f64 || uy >= uh as f64 {
                continue;
            }
            let (ux, uy) = (ux as usize, uy as usize);
            if s.lmask[(uy / 8) * s.bw + ux / 8] {
                in_mask += 1;
                votes[j * 8 + i] = up[uy * uw + ux];
            }
        }
    }
    votes.sort_unstable();
    let (mut best, mut best_n, mut run) = (0u32, 0, 0);
    for k in 0..64 {
        run = if k > 0 && votes[k] == votes[k - 1] { run + 1 } else { 1 };
        if run > best_n {
            best_n = run;
            best = votes[k];
        }
    }
    (best.to_be_bytes(), in_mask >= 32)
}

/// Recomputes the cel (and selection) from the session's current transform.
fn render(e: &mut Editor) {
    let Some(mut s) = e.sel.xform.take() else { return };
    let (w, h) = (e.sprite.width as usize, e.sprite.height as usize);
    if s.base.len() != w * h * 4 {
        // The canvas changed under the session (inside a transaction): it can no longer be previewed.
        return;
    }
    let mut px = s.base.clone();
    let mut sel = Selection::new(w as u32, h as u32, false);
    let rotated = s.xf.angle != 0.0 || s.xf.kx != 0.0 || s.xf.ky != 0.0;
    let use_rotsprite = s.rotsprite && rotated && s.bw * s.bh <= ROTSPRITE_MAX_AREA;
    if use_rotsprite && s.up8.is_none() {
        let packed: Vec<u32> =
            (0..s.bw * s.bh).map(|i| if s.lmask[i] { pack(&s.lift[i * 4..i * 4 + 4]) } else { 0 }).collect();
        s.up8 = Some(upscale8(&packed, s.bw, s.bh));
    }
    if s.xf.inverse(s.xf.dp).is_some() {
        let (bx0, by0, bx1, by1) = s.xf.bbox();
        let (x0, y0) = ((bx0.floor().max(0.0)) as usize, (by0.floor().max(0.0)) as usize);
        let (x1, y1) = ((bx1.ceil().min(w as f64)).max(0.0) as usize, (by1.ceil().min(h as f64)).max(0.0) as usize);
        for y in y0..y1 {
            for x in x0..x1 {
                let (c, inside) = match (&s.up8, use_rotsprite) {
                    (Some(up), true) => sample_rotsprite(&s, up, x, y),
                    _ => sample_nearest(&s, &s.xf, x, y),
                };
                let i = y * w + x;
                if c[3] > 0 {
                    blend_over(&mut px[i * 4..i * 4 + 4], &c, 255);
                }
                sel.bits[i] = inside;
            }
        }
    }
    if let Some(cel) = e.sprite.cel_mut(s.layer, s.frame) {
        cel.pixels = px;
    }
    crate::document::color_mode::conform_cel(&mut e.sprite, s.layer, s.frame);
    if s.had_selection {
        e.sel.mask = nonempty(sel);
    }
    e.sel.xform = Some(s);
}

// MARK: C ABI

unsafe fn ed<'a>(doc: *mut Editor) -> Option<&'a mut Editor> {
    let e = doc.as_mut()?;
    e.dirty.mark_all();
    Some(e)
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_xf_begin(doc: *mut Editor, layer: u32, frame: u32, rotsprite: bool) -> bool {
    crate::ffi::guarded("pixmith_xf_begin", doc, || {
        ed(doc).is_some_and(|e| begin(e, layer as usize, frame as usize, rotsprite))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_xf_active(doc: *const Editor) -> bool {
    crate::ffi::guarded("pixmith_xf_active", std::ptr::null_mut(), || doc.as_ref().is_some_and(active))
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_xf_commit(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_xf_commit", doc, || {
        if let Some(e) = ed(doc) {
            commit(e);
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_xf_cancel(doc: *mut Editor) -> bool {
    crate::ffi::guarded("pixmith_xf_cancel", doc, || ed(doc).is_some_and(cancel))
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_xf_set_rotsprite(doc: *mut Editor, on: bool) {
    crate::ffi::guarded("pixmith_xf_set_rotsprite", doc, || {
        if let Some(e) = ed(doc) {
            set_rotsprite(e, on);
        }
    })
}

/// Sets the box point (`hx`, `hy` in -1..=1; 0, 0 is the center) typed width/height keep fixed.
#[no_mangle]
pub unsafe extern "C" fn pixmith_xf_set_anchor(doc: *mut Editor, hx: i32, hy: i32) {
    crate::ffi::guarded("pixmith_xf_set_anchor", doc, || {
        if let Some(e) = ed(doc) {
            set_anchor(e, (hx, hy));
        }
    })
}

/// Writes 18 doubles: the 8 handle positions in `HANDLES` order (x, y each), then the pivot.
#[no_mangle]
pub unsafe extern "C" fn pixmith_xf_geometry(doc: *const Editor, out: *mut c_double) -> bool {
    crate::ffi::guarded("pixmith_xf_geometry", std::ptr::null_mut(), || {
        let (Some(xf), false) = (doc.as_ref().and_then(current), out.is_null()) else { return false };
        let o = std::slice::from_raw_parts_mut(out, 18);
        for (i, h) in HANDLES.iter().enumerate() {
            let p = xf.handle(*h);
            o[i * 2] = p.0;
            o[i * 2 + 1] = p.1;
        }
        o[16] = xf.dp.0;
        o[17] = xf.dp.1;
        true
    })
}

/// Writes 7 doubles: x, y, width, height, angle, skew x, skew y (angles in degrees).
#[no_mangle]
pub unsafe extern "C" fn pixmith_xf_fields(doc: *const Editor, out: *mut c_double) -> bool {
    crate::ffi::guarded("pixmith_xf_fields", std::ptr::null_mut(), || {
        let (Some(xf), false) = (doc.as_ref().and_then(current), out.is_null()) else { return false };
        std::slice::from_raw_parts_mut(out, 7).copy_from_slice(&xf.fields());
        true
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_xf_set_field(doc: *mut Editor, field: u32, value: c_double) {
    crate::ffi::guarded("pixmith_xf_set_field", doc, || {
        if let Some(e) = ed(doc) {
            if !value.is_finite() {
                return;
            }
            if let Some(xf) = current(e) {
                let anchor = e.sel.xform.as_ref().map_or((0, 0), |s| s.anchor);
                set_xform(e, xf.with_field_anchored(field, value, anchor));
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_xf_drag_begin(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_xf_drag_begin", doc, || {
        if let Some(e) = ed(doc) {
            drag_begin(e);
        }
    })
}

/// `kind`: 0 move, 1 scale (`hx`, `hy` handle, `flag` keeps aspect), 2 rotate (`flag` snaps to
/// 15 degrees), 3 skew (`hx`, `hy` edge handle), 4 pivot. `(sx, sy)` is where the drag started and
/// `(px, py)` the pointer, in continuous canvas coordinates.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_xf_drag(
    doc: *mut Editor,
    kind: u32,
    hx: i32,
    hy: i32,
    flag: bool,
    sx: c_double,
    sy: c_double,
    px: c_double,
    py: c_double,
) {
    crate::ffi::guarded("pixmith_xf_drag", doc, || {
        let Some(e) = ed(doc) else { return };
        if ![sx, sy, px, py].iter().all(|v| v.is_finite()) {
            return;
        }
        let kind = match kind {
            0 => Drag::Move,
            1 => Drag::Scale { h: (hx, hy), keep_aspect: flag },
            2 => Drag::Rotate { snap: flag },
            3 => Drag::Skew { h: (hx, hy) },
            4 => Drag::Pivot,
            _ => return,
        };
        drag(e, kind, (sx, sy), (px, py));
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Sprite;
    use crate::paint::selection::{self, select_rect};

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];

    fn editor(w: u32, h: u32) -> Editor {
        Editor::new(Sprite::new(w, h))
    }

    fn dot(e: &mut Editor, x: i32, y: i32, c: [u8; 4]) {
        e.sprite.draw_line(0, 0, x, y, x, y, c, 1);
    }

    fn px(e: &Editor, x: i32, y: i32) -> [u8; 4] {
        e.sprite.cels[0][0]
            .as_ref()
            .map_or([0; 4], |c| c.pixels[px_index(e.sprite.width, x, y)..][..4].try_into().unwrap())
    }

    fn close(a: P2, b: P2) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
    }

    fn set(e: &mut Editor, f: impl FnOnce(Xform) -> Xform) {
        let xf = current(e).unwrap();
        set_xform(e, f(xf));
    }

    #[test]
    fn identity_keeps_pixels_and_move_shifts_them() {
        let mut e = editor(8, 8);
        dot(&mut e, 2, 2, RED);
        select_rect(&mut e, (1, 1), (3, 3));
        assert!(begin(&mut e, 0, 0, false));
        assert_eq!(px(&e, 2, 2), RED);
        set(&mut e, |x| x.translate((3.0, 2.0)));
        assert_eq!(px(&e, 2, 2)[3], 0);
        assert_eq!(px(&e, 5, 4), RED);
        assert_eq!(e.sel.active(8, 8).unwrap().bounds(), Some((4, 3, 3, 3)));
    }

    #[test]
    fn scaling_up_repeats_pixels_and_negative_scale_flips() {
        let mut e = editor(8, 4);
        dot(&mut e, 0, 0, RED);
        dot(&mut e, 1, 0, BLUE);
        select_rect(&mut e, (0, 0), (1, 0));
        begin(&mut e, 0, 0, false);
        // Pin the top-left corner and double the width.
        let xf = current(&e).unwrap();
        set_xform(&mut e, xf.with_field(2, 4.0));
        let row: Vec<_> = (0..4).map(|x| px(&e, x, 0)).collect();
        assert_eq!(row, vec![RED, RED, BLUE, BLUE]);
        // Flip: scale -1 around the pivot mirrors the pair.
        let mut e = editor(8, 4);
        dot(&mut e, 0, 0, RED);
        dot(&mut e, 1, 0, BLUE);
        select_rect(&mut e, (0, 0), (1, 0));
        begin(&mut e, 0, 0, false);
        set(&mut e, |x| Xform { sx: -1.0, ..x });
        assert_eq!((px(&e, 0, 0), px(&e, 1, 0)), (BLUE, RED));
    }

    #[test]
    fn rotation_by_90_matches_the_quarter_turn_operation() {
        let mut a = editor(5, 5);
        for (x, y, c) in [(1, 1, RED), (2, 1, BLUE), (3, 2, GREEN), (1, 3, RED)] {
            dot(&mut a, x, y, c);
        }
        select_rect(&mut a, (1, 1), (3, 3));
        let mut b = Editor::new(a.sprite.clone());
        select_rect(&mut b, (1, 1), (3, 3));
        assert!(selection::transform(&mut b, 0, 0, selection::Op::RotateCw));
        begin(&mut a, 0, 0, false);
        set(&mut a, |x| Xform { angle: PI / 2.0, ..x });
        for y in 0..5 {
            for x in 0..5 {
                assert_eq!(px(&a, x, y), px(&b, x, y), "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn rotsprite_rotates_exactly_by_quarter_turns_and_adds_no_colors() {
        let mut e = editor(9, 9);
        for (x, y, c) in [(3, 3, RED), (4, 3, RED), (5, 3, BLUE), (3, 5, GREEN)] {
            dot(&mut e, x, y, c);
        }
        select_rect(&mut e, (2, 2), (6, 6));
        begin(&mut e, 0, 0, true);
        set(&mut e, |x| Xform { angle: PI / 2.0, ..x });
        // (3,3) is up-left of center (4,4); a clockwise quarter turn sends it up-right.
        assert_eq!(px(&e, 5, 3), RED);
        assert_eq!(px(&e, 5, 4), RED);
        assert_eq!(px(&e, 5, 5), BLUE);
        assert_eq!(px(&e, 3, 3), GREEN);
        // At 30 degrees only source colors (or transparency) appear.
        set(&mut e, |x| Xform { angle: 30f64.to_radians(), ..x });
        for y in 0..9 {
            for x in 0..9 {
                let c = px(&e, x, y);
                assert!(c[3] == 0 || [RED, BLUE, GREEN].contains(&c), "{c:?} at {x},{y}");
            }
        }
        assert!((0..81).any(|i| px(&e, i % 9, i / 9)[3] > 0));
    }

    #[test]
    fn rotsprite_is_smoother_than_nearest_on_a_diagonal() {
        // A solid 8x8 square rotated 45 degrees keeps (nearly) its area either way.
        let count = |rot: bool| {
            let mut e = editor(24, 24);
            for y in 8..16 {
                for x in 8..16 {
                    dot(&mut e, x, y, RED);
                }
            }
            select_rect(&mut e, (8, 8), (15, 15));
            begin(&mut e, 0, 0, rot);
            set(&mut e, |x| Xform { angle: PI / 4.0, ..x });
            (0..24 * 24).filter(|i| px(&e, i % 24, i / 24)[3] > 0).count()
        };
        for n in [count(false), count(true)] {
            assert!((56..=72).contains(&n), "area {n}");
        }
    }

    #[test]
    fn epx_smooths_corners_only() {
        // A 3x3 diagonal: the lone corner pixels are replaced by their neighbors' color.
        let (a, b) = (1u32, 2u32);
        #[rustfmt::skip]
        let src = [
            a, a, b,
            a, b, b,
            b, b, b,
        ];
        let out = epx(&src, 3, 3);
        assert_eq!(out.len(), 36);
        // The center pixel borders the diagonal: only its top-left quadrant takes the other color.
        assert_eq!([out[2 * 6 + 2], out[2 * 6 + 3], out[3 * 6 + 2], out[3 * 6 + 3]], [a, b, b, b]);
        // Uniform regions are untouched.
        let flat = epx(&[a; 4], 2, 2);
        assert!(flat.iter().all(|c| *c == a));
        assert_eq!(upscale8(&[a, b], 2, 1).len(), 16 * 8);
        // Only existing colors appear.
        assert!(out.iter().all(|c| *c == a || *c == b));
    }

    #[test]
    fn handles_follow_rotation_and_skew() {
        let xf = Xform { angle: PI / 2.0, ..Xform::identity(0.0, 0.0, 4.0, 2.0) };
        // Center (2, 1); the top-left corner (0, 0) swings to the top-right.
        assert!(close(xf.forward((0.0, 0.0)), (3.0, -1.0)));
        assert!(close(xf.inverse(xf.forward((1.3, 0.4))).unwrap(), (1.3, 0.4)));
        let bb = xf.bbox();
        assert!(close((bb.0, bb.1), (1.0, -1.0)) && close((bb.2, bb.3), (3.0, 3.0)));
        let sk = Xform { kx: 45f64.to_radians(), ..Xform::identity(0.0, 0.0, 2.0, 2.0) };
        // x shifts by y: the top edge moves left by 1, the bottom edge right by 1.
        assert!(close(sk.forward((1.0, 0.0)), (0.0, 0.0)));
        assert!(close(sk.forward((1.0, 2.0)), (2.0, 2.0)));
    }

    #[test]
    fn scale_handle_keeps_the_opposite_corner_fixed() {
        let xf = Xform::identity(2.0, 2.0, 4.0, 4.0);
        let out = xf.scale_handle((1, 1), (10.0, 8.0), false);
        assert!(close(out.forward((2.0, 2.0)), (2.0, 2.0)));
        assert!(close(out.handle((1, 1)), (10.0, 8.0)));
        assert!((out.sx - 2.0).abs() < 1e-9 && (out.sy - 1.5).abs() < 1e-9);
        let uni = xf.scale_handle((1, 1), (10.0, 8.0), true);
        assert!((uni.sx - 2.0).abs() < 1e-9 && (uni.sy - 2.0).abs() < 1e-9);
        // An edge handle changes one axis only; dragging past the other side flips.
        let edge = xf.scale_handle((1, 0), (0.0, 99.0), false);
        assert!(edge.sy == 1.0 && edge.sx < 0.0);
        assert!(close(edge.forward((2.0, 3.0)), (2.0, 3.0)));
        // Rotated boxes scale along their own axes.
        let rot = Xform { angle: PI / 2.0, ..xf };
        let out = rot.scale_handle((1, 0), (4.0, 8.0), false);
        assert!(close(out.forward(rot.handle_src((-1, 0))), rot.forward(rot.handle_src((-1, 0)))));
        assert!(out.sx > 1.0);
    }

    #[test]
    fn rotate_drag_sweeps_around_the_pivot_and_snaps() {
        let xf = Xform::identity(0.0, 0.0, 4.0, 4.0);
        let out = xf.rotate_by_drag((6.0, 2.0), (2.0, 6.0), false);
        assert!((out.angle - PI / 2.0).abs() < 1e-9);
        let snapped = xf.rotate_by_drag((6.0, 2.0), (6.0, 2.0 + 4.0 * 20f64.to_radians().tan()), true);
        assert!((snapped.angle - 15f64.to_radians()).abs() < 1e-9);
        let wrap = Xform { angle: 3.0, ..xf }.rotate_by_drag((6.0, 2.0), (2.0, 6.0), false);
        assert!(wrap.angle > -PI && wrap.angle <= PI);
    }

    #[test]
    fn skew_drag_slides_the_edge_and_moving_the_pivot_changes_nothing() {
        let xf = Xform::identity(0.0, 0.0, 4.0, 4.0);
        // Drag the top edge 2 px right while the bottom edge stays put.
        let sk = xf.skew_handle((0, -1), (4.0, 0.0));
        assert!(close(sk.handle((0, -1)), (4.0, 0.0)));
        assert!(close(sk.handle((0, 1)), (2.0, 4.0)));
        let side = xf.skew_handle((1, 0), (4.0, 5.0));
        assert!(close(side.handle((1, 0)), (4.0, 5.0)));
        assert_eq!(xf.skew_handle((1, 1), (9.0, 9.0)), xf);
        let moved = Xform { angle: 0.4, sx: 1.5, kx: 0.2, ..xf }.move_pivot((7.0, 1.0));
        let orig = Xform { angle: 0.4, sx: 1.5, kx: 0.2, ..xf };
        for v in [(0.0, 0.0), (3.0, 1.0), (4.0, 4.0)] {
            assert!(close(moved.forward(v), orig.forward(v)));
        }
        assert!(close(moved.dp, (7.0, 1.0)));
    }

    #[test]
    fn numeric_fields_round_trip() {
        let xf = Xform::identity(2.0, 3.0, 4.0, 2.0);
        assert_eq!(xf.fields(), [2.0, 3.0, 4.0, 2.0, 0.0, 0.0, 0.0]);
        let moved = xf.with_field(0, 10.0).with_field(1, 0.0);
        assert!(close((moved.fields()[0], moved.fields()[1]), (10.0, 0.0)));
        let wide = xf.with_field(2, 8.0).with_field(3, 6.0);
        let f = wide.fields();
        assert!((f[2] - 8.0).abs() < 1e-9 && (f[3] - 6.0).abs() < 1e-9);
        assert!(close((f[0], f[1]), (2.0, 3.0)), "top-left stays: {f:?}");
        let turned = xf.with_field(4, 90.0).with_field(5, 20.0).with_field(6, -10.0);
        let f = turned.fields();
        assert!((f[4] - 90.0).abs() < 1e-9 && (f[5] - 20.0).abs() < 1e-9 && (f[6] + 10.0).abs() < 1e-9);
        assert!((xf.with_field(5, 200.0).fields()[5] - 80.0).abs() < 1e-9);
        assert_eq!(xf.with_field(9, 1.0), xf);
    }

    #[test]
    fn apply_cancel_and_undo_are_single_steps() {
        let mut e = editor(8, 8);
        dot(&mut e, 2, 2, RED);
        select_rect(&mut e, (1, 1), (3, 3));
        let before = e.sprite.clone();
        begin(&mut e, 0, 0, false);
        set(&mut e, |x| x.translate((2.0, 0.0)));
        set(&mut e, |x| x.translate((1.0, 0.0)));
        assert!(cancel(&mut e));
        assert!(!active(&e));
        assert_eq!(e.sprite.cels, before.cels);
        assert_eq!(e.sel.active(8, 8).unwrap().bounds(), Some((1, 1, 3, 3)));
        assert!(!cancel(&mut e));

        begin(&mut e, 0, 0, false);
        set(&mut e, |x| x.translate((2.0, 0.0)));
        commit(&mut e);
        assert_eq!(px(&e, 4, 2), RED);
        assert!(e.undo());
        assert_eq!(e.sprite.cels, before.cels);
        assert_eq!(e.sel.active(8, 8).unwrap().bounds(), Some((1, 1, 3, 3)));
        // Any other edit ends the session and keeps the preview.
        begin(&mut e, 0, 0, false);
        set(&mut e, |x| x.translate((1.0, 0.0)));
        e.checkpoint();
        assert!(!active(&e));
        assert_eq!(px(&e, 3, 2), RED);
    }

    #[test]
    fn whole_cel_transform_without_selection() {
        let mut e = editor(4, 4);
        dot(&mut e, 0, 0, RED);
        begin(&mut e, 0, 0, false);
        set(&mut e, |x| x.translate((2.0, 1.0)));
        assert_eq!(px(&e, 2, 1), RED);
        assert!(e.sel.active(4, 4).is_none());
        let mut empty = editor(4, 4);
        assert!(!begin(&mut empty, 0, 0, false));
    }

    #[test]
    fn drag_api_recomputes_from_the_start_state() {
        let mut e = editor(12, 12);
        for y in 4..8 {
            for x in 4..8 {
                dot(&mut e, x, y, RED);
            }
        }
        select_rect(&mut e, (4, 4), (7, 7));
        begin(&mut e, 0, 0, false);
        drag_begin(&mut e);
        drag(&mut e, Drag::Scale { h: (1, 1), keep_aspect: false }, (8.0, 8.0), (10.0, 10.0));
        drag(&mut e, Drag::Scale { h: (1, 1), keep_aspect: false }, (8.0, 8.0), (12.0, 12.0));
        let f = current(&e).unwrap().fields();
        assert!(close((f[2], f[3]), (8.0, 8.0)));
        assert_eq!(px(&e, 11, 11), RED);
        assert_eq!(px(&e, 3, 3)[3], 0);
        drag_begin(&mut e);
        drag(&mut e, Drag::Move, (5.0, 5.0), (4.0, 4.0));
        assert!(close((current(&e).unwrap().fields()[0], current(&e).unwrap().fields()[1]), (3.0, 3.0)));
    }

    #[test]
    fn cancel_leaves_no_redo_step() {
        let mut e = editor(8, 8);
        dot(&mut e, 2, 2, RED);
        select_rect(&mut e, (1, 1), (3, 3));
        let before = e.sprite.cels.clone();
        begin(&mut e, 0, 0, false);
        set(&mut e, |x| x.translate((2.0, 0.0)));
        assert!(cancel(&mut e));
        assert_eq!(e.sprite.cels, before);
        assert!(!e.redo(), "a cancelled transform must not be redoable");
        assert_eq!(e.history_len().1, 0);
        // The same for a floating paste.
        assert!(selection::paste(&mut e, 0, 0, 1, 1, &BLUE, 5, 5));
        assert!(selection::float_cancel(&mut e));
        assert_eq!(px(&e, 5, 5)[3], 0);
        assert!(!e.redo());
        // Earlier history is intact: the selection step is still undoable.
        assert!(e.undo());
    }

    #[test]
    fn non_square_quarter_turns_match_the_selection_operation() {
        for (bw, bh) in [(2, 1), (1, 2), (3, 2), (2, 3), (4, 1), (5, 2)] {
            for op in [selection::Op::RotateCw, selection::Op::RotateCcw] {
                let mut a = editor(10, 10);
                for y in 0..bh {
                    for x in 0..bw {
                        dot(&mut a, 3 + x, 3 + y, [(x * 40 + 10) as u8, (y * 60 + 10) as u8, 90, 255]);
                    }
                }
                select_rect(&mut a, (3, 3), (2 + bw, 2 + bh));
                let mut b = Editor::new(a.sprite.clone());
                select_rect(&mut b, (3, 3), (2 + bw, 2 + bh));
                assert!(selection::transform(&mut b, 0, 0, op));
                begin(&mut a, 0, 0, false);
                let turn = if matches!(op, selection::Op::RotateCw) { 90.0 } else { -90.0 };
                set(&mut a, |x| x.with_field(4, turn));
                for y in 0..10 {
                    for x in 0..10 {
                        assert_eq!(px(&a, x, y), px(&b, x, y), "{bw}x{bh} {op:?} at {x},{y}");
                    }
                }
            }
        }
    }

    #[test]
    fn scale_drag_gives_whole_pixel_sizes_even_when_rotated() {
        let rot = Xform { angle: 0.7, kx: 0.2, ..Xform::identity(2.0, 2.0, 5.0, 3.0) };
        for p in [(9.3, 7.7), (10.49, 8.1), (3.2, 3.9)] {
            let out = rot.scale_handle((1, 1), p, false);
            for (s, len) in [(out.sx, 5.0), (out.sy, 3.0)] {
                let size = s.abs() * len;
                assert!((size - size.round()).abs() < 1e-9 && size >= 1.0, "size {size}");
            }
        }
        // The axis that is not dragged keeps its scale.
        let edge = rot.scale_handle((1, 0), (9.3, 7.7), false);
        assert_eq!(edge.sy, rot.sy);
    }

    #[test]
    fn typed_size_keeps_the_chosen_anchor() {
        let xf = Xform::identity(4.0, 4.0, 4.0, 4.0);
        let c = xf.with_field_anchored(2, 8.0, (0, 0)).with_field_anchored(3, 2.0, (0, 0));
        let f = c.fields();
        assert!(close((f[0] + f[2] / 2.0, f[1] + f[3] / 2.0), (6.0, 6.0)), "{f:?}");
        assert!(close((f[2], f[3]), (8.0, 2.0)));
        let br = xf.with_field_anchored(2, 8.0, (1, 1));
        assert!(close(br.handle((1, 1)), (8.0, 8.0)));
        let tl = xf.with_field(3, 8.0);
        assert!(close(tl.handle((-1, -1)), (4.0, 4.0)));
    }

    #[test]
    fn session_anchor_applies_to_typed_size() {
        let mut e = editor(16, 16);
        dot(&mut e, 5, 5, RED);
        select_rect(&mut e, (4, 4), (7, 7));
        begin(&mut e, 0, 0, false);
        let xf = current(&e).unwrap();
        set_xform(&mut e, xf.with_field_anchored(2, 8.0, (0, 0)));
        let f = current(&e).unwrap().fields();
        assert!(close((f[0], f[2]), (2.0, 8.0)), "default anchor is the center: {f:?}");
        set_anchor(&mut e, (-1, -1));
        let xf = current(&e).unwrap();
        set_xform(&mut e, xf.with_field_anchored(3, 2.0, (-1, -1)));
        assert!((current(&e).unwrap().fields()[1] - 4.0).abs() < 1e-9);
    }

    fn tiles_in_sync(s: &Sprite) -> bool {
        matches!(crate::document::tilemap::normalized(s), std::borrow::Cow::Borrowed(_))
    }

    #[test]
    fn tilemap_pixels_are_written_back_when_a_preview_ends() {
        let mut s = Sprite::new(16, 16);
        let l = crate::document::tilemap::add_tilemap_layer(&mut s, 0, 8, 8);
        s.draw_line(l, 0, 1, 1, 2, 1, RED, 1);
        assert!(tiles_in_sync(&s));
        // Control: an unsynced pixel edit is detected.
        let mut dirty = s.clone();
        dirty.cel_mut(l, 0).unwrap().pixels[(5 * 16 + 5) * 4..][..4].copy_from_slice(&BLUE);
        assert!(!tiles_in_sync(&dirty));
        let mk = || {
            let mut e = Editor::new(s.clone());
            select_rect(&mut e, (0, 0), (5, 5));
            e
        };
        let px_l = |e: &Editor, x: i32, y: i32| -> [u8; 4] {
            e.sprite.cels[0][l].as_ref().unwrap().pixels[px_index(16, x, y)..][..4].try_into().unwrap()
        };
        // Free transform: committed explicitly...
        let mut e = mk();
        assert!(begin(&mut e, l, 0, false));
        set(&mut e, |x| x.translate((9.0, 9.0)));
        commit(&mut e);
        assert_eq!(px_l(&e, 10, 10), RED);
        assert!(tiles_in_sync(&e.sprite));
        // ...or ended by another edit.
        let mut e = mk();
        begin(&mut e, l, 0, false);
        set(&mut e, |x| x.translate((9.0, 9.0)));
        e.checkpoint();
        assert!(tiles_in_sync(&e.sprite));
        // Floating paste, committed or ended by an edit.
        let mut e = mk();
        assert!(selection::paste(&mut e, l, 0, 1, 1, &BLUE, 12, 12));
        selection::float_commit(&mut e);
        assert!(tiles_in_sync(&e.sprite));
        let mut e = mk();
        selection::paste(&mut e, l, 0, 1, 1, &BLUE, 12, 12);
        e.checkpoint();
        assert!(tiles_in_sync(&e.sprite));
        // Move, nudge, flip and delete.
        let mut e = mk();
        assert!(selection::nudge(&mut e, l, 0, 9, 9));
        assert!(tiles_in_sync(&e.sprite));
        let mut e = mk();
        assert!(selection::transform(&mut e, l, 0, selection::Op::FlipH));
        assert!(tiles_in_sync(&e.sprite));
        let mut e = mk();
        assert!(selection::delete(&mut e, l, 0));
        assert!(tiles_in_sync(&e.sprite));
    }

    #[test]
    fn merge_down_writes_back_to_a_tilemap_layer() {
        let mut s = Sprite::new(16, 16);
        let l = crate::document::tilemap::add_tilemap_layer(&mut s, 0, 8, 8);
        let top = s.add_layer(l);
        s.draw_line(top, 0, 3, 3, 4, 3, RED, 1);
        assert_eq!(s.merge_down(top), Some(l));
        assert!(tiles_in_sync(&s));
    }
}
