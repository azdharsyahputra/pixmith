//! Shape, stroke and spray tools with symmetry, plus their C ABI.
//!
//! Option flags shared by the FFI calls: bit 0 mirror left/right, bit 1 mirror top/bottom,
//! bit 2 pixel-perfect, bit 3 filled, bit 4 constrain (45° lines, squares and circles).

use crate::document::{Cel, Editor, Sprite};
use crate::paint::ink::{self, Brush, Ink, Settings};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

pub const SYM_X: u32 = 1;
pub const SYM_Y: u32 = 2;
pub const PIXEL_PERFECT: u32 = 4;
pub const FILLED: u32 = 8;
pub const CONSTRAIN: u32 = 16;
/// Polygons: draw the path without closing it (and without filling).
pub const OPEN: u32 = 32;

pub type Pt = (i32, i32);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Line,
    Rect,
    Ellipse,
}

/// Pixels of a Bresenham line, endpoints included.
pub fn line_points(a: Pt, b: Pt) -> Vec<Pt> {
    let (dx, dy) = ((b.0 - a.0).abs(), -(b.1 - a.1).abs());
    let (sx, sy) = (if a.0 < b.0 { 1 } else { -1 }, if a.1 < b.1 { 1 } else { -1 });
    let (mut x, mut y, mut err) = (a.0, a.1, dx + dy);
    let mut out = Vec::new();
    loop {
        out.push((x, y));
        if (x, y) == b {
            return out;
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

/// Snaps the end of a line to the nearest multiple of 45°.
pub fn constrain_line(a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (ax, ay) = (dx.abs(), dy.abs());
    if ax > 2 * ay {
        (b.0, a.1)
    } else if ay > 2 * ax {
        (a.0, b.1)
    } else {
        let m = ax.max(ay);
        (a.0 + m * if dx < 0 { -1 } else { 1 }, a.1 + m * if dy < 0 { -1 } else { 1 })
    }
}

/// Makes the box from `a` to `b` square, growing along the longer side.
pub fn constrain_square(a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let m = dx.abs().max(dy.abs());
    (a.0 + m * if dx < 0 { -1 } else { 1 }, a.1 + m * if dy < 0 { -1 } else { 1 })
}

fn normalize(a: Pt, b: Pt) -> (Pt, Pt) {
    ((a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1)))
}

/// Perimeter pixels of the box spanned by two corners.
pub fn rect_outline(a: Pt, b: Pt) -> Vec<Pt> {
    let ((x0, y0), (x1, y1)) = normalize(a, b);
    let mut out: Vec<Pt> = (x0..=x1).map(|x| (x, y0)).collect();
    if y1 > y0 {
        out.extend((x0..=x1).map(|x| (x, y1)));
    }
    for y in y0 + 1..y1 {
        out.push((x0, y));
        if x1 > x0 {
            out.push((x1, y));
        }
    }
    out
}

/// Rows `(y, left, right)` of the ellipse inscribed in the box spanned by two corners.
pub fn ellipse_spans(a: Pt, b: Pt) -> Vec<(i32, i32, i32)> {
    const EPS: f64 = 1e-9;
    let ((x0, y0), (x1, y1)) = normalize(a, b);
    let (w, h) = ((x1 - x0 + 1) as f64, (y1 - y0 + 1) as f64);
    let (cx, cy) = (x0 as f64 + w / 2.0, y0 as f64 + h / 2.0);
    (y0..=y1)
        .map(|y| {
            let t = (y as f64 + 0.5 - cy) / (h / 2.0);
            let xr = w / 2.0 * (1.0 - t * t).max(0.0).sqrt();
            let mut l = (cx - xr - 0.5 - EPS).ceil() as i32;
            let mut r = (cx + xr - 0.5 + EPS).floor() as i32;
            if l > r {
                l = (cx - 0.5).floor() as i32;
                r = (cx - 0.5).ceil() as i32;
            }
            (y, l.max(x0), r.min(x1))
        })
        .collect()
}

/// One-pixel-wide outline of the ellipse inscribed in the box.
pub fn ellipse_outline(a: Pt, b: Pt) -> Vec<Pt> {
    let spans = ellipse_spans(a, b);
    let mut out = Vec::new();
    for (i, &(y, l, r)) in spans.iter().enumerate() {
        let (up, down) = (i.checked_sub(1).map(|j| spans[j]), spans.get(i + 1).copied());
        let (Some(up), Some(down)) = (up, down) else {
            out.extend((l..=r).map(|x| (x, y)));
            continue;
        };
        let lref = up.1.max(down.1);
        let rref = up.2.min(down.2);
        out.extend((l..=(lref - 1).max(l).min(r)).map(|x| (x, y)));
        out.extend(((rref + 1).max(l).min(r)..=r).map(|x| (x, y)));
    }
    out
}

fn is_corner(a: Pt, b: Pt, c: Pt) -> bool {
    (a.0 - b.0).abs() + (a.1 - b.1).abs() == 1
        && (b.0 - c.0).abs() + (b.1 - c.1).abs() == 1
        && (a.0 - c.0).abs() == 1
        && (a.1 - c.1).abs() == 1
}

/// Writes brush stamps into a cel through the current ink, mirrored across the canvas center.
pub(crate) struct Painter<'a> {
    px: &'a mut [u8],
    w: i32,
    h: i32,
    color: [u8; 4],
    size: i32,
    sym: u32,
    settings: Rc<Settings>,
    /// Pixels already painted in this gesture; used by inks that must not stack on themselves.
    touched: Option<&'a mut [bool]>,
    use_brush: bool,
    /// Union of everything written so far, `(x0, y0, x1, y1)` with the end exclusive.
    pub(crate) bounds: Option<(i32, i32, i32, i32)>,
    /// Where the previous brush stamp went (and whether it erased), so a one-pixel step only has
    /// to paint what the stamp newly covers.
    last_stamp: Option<(Pt, bool)>,
}

impl<'a> Painter<'a> {
    pub(crate) fn new(cel: &'a mut Cel, s: (i32, i32), color: [u8; 4], size: u32, sym: u32) -> Self {
        Painter {
            px: &mut cel.pixels,
            w: s.0,
            h: s.1,
            color,
            size: size.clamp(1, 64) as i32,
            sym,
            settings: ink::settings(),
            touched: None,
            use_brush: true,
            bounds: None,
            last_stamp: None,
        }
    }

    pub(crate) fn tracked(mut self, touched: Option<&'a mut [bool]>) -> Self {
        if self.settings.ink != Ink::Simple {
            self.touched = touched;
        }
        self
    }

    /// Ignore circle and custom brushes (spray dots are always single pixels).
    pub(crate) fn plain(mut self) -> Self {
        self.use_brush = false;
        self
    }

    fn untouch(&mut self, x: i32, y: i32) {
        if let Some(t) = self.touched.as_deref_mut() {
            t[(y * self.w + x) as usize] = false;
        }
    }

    /// Adds the pixel rectangle (inclusive, clipped to the canvas) to `bounds`.
    fn note(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let (x0, y0, x1, y1) = (x0.max(0), y0.max(0), x1.min(self.w - 1) + 1, y1.min(self.h - 1) + 1);
        if x0 < x1 && y0 < y1 {
            self.bounds =
                Some(self.bounds.map_or((x0, y0, x1, y1), |b| (b.0.min(x0), b.1.min(y0), b.2.max(x1), b.3.max(y1))));
        }
    }

    fn raw(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        self.note(x0, y0, x1, y1);
        for y in y0.max(0)..=y1.min(self.h - 1) {
            for x in x0.max(0)..=x1.min(self.w - 1) {
                let i = (y * self.w + x) as usize;
                if let Some(t) = self.touched.as_deref_mut() {
                    if std::mem::replace(&mut t[i], true) {
                        continue;
                    }
                }
                ink::paint(&self.settings, &mut self.px[i * 4..i * 4 + 4], self.color);
            }
        }
    }

    /// Fills an inclusive rectangle plus its mirror images.
    pub(crate) fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let (mx0, mx1, my0, my1) = (self.w - 1 - x1, self.w - 1 - x0, self.h - 1 - y1, self.h - 1 - y0);
        self.raw(x0, y0, x1, y1);
        if self.sym & SYM_X != 0 {
            self.raw(mx0, y0, mx1, y1);
        }
        if self.sym & SYM_Y != 0 {
            self.raw(x0, my0, x1, my1);
        }
        if self.sym & SYM_X != 0 && self.sym & SYM_Y != 0 {
            self.raw(mx0, my0, mx1, my1);
        }
    }

    /// Stamps the brush centered on `p`.
    pub(crate) fn dot(&mut self, p: Pt) {
        let settings = self.settings.clone();
        if let (true, Some(b)) = (self.use_brush, &settings.custom) {
            return self.stamp(b, p);
        }
        let (size, half) = (self.size, (self.size - 1) / 2);
        if self.use_brush && settings.circle && size > 2 {
            let r = size as f64 / 2.0;
            for row in 0..size {
                let dy = row as f64 + 0.5 - r;
                let l = ((r - (r * r - dy * dy).max(0.0).sqrt() - 0.5 - 1e-9).ceil() as i32).clamp(0, size - 1);
                let y = p.1 - half + row;
                self.rect(p.0 - half + l, y, p.0 - half + size - 1 - l, y);
            }
        } else {
            self.rect(p.0 - half, p.1 - half, p.0 - half + size - 1, p.1 - half + size - 1);
        }
    }

    /// Paints one pixel plus its mirror images in the same order as `rect`.
    fn put(&mut self, x: i32, y: i32, color: [u8; 4]) {
        let (mx, my) = (self.w - 1 - x, self.h - 1 - y);
        let one = |s: &mut Self, x: i32, y: i32| {
            if x < 0 || y < 0 || x >= s.w || y >= s.h {
                return;
            }
            s.note(x, y, x, y);
            let i = (y * s.w + x) as usize;
            if let Some(t) = s.touched.as_deref_mut() {
                if std::mem::replace(&mut t[i], true) {
                    return;
                }
            }
            ink::paint(&s.settings, &mut s.px[i * 4..i * 4 + 4], color);
        };
        one(self, x, y);
        if self.sym & SYM_X != 0 {
            one(self, mx, y);
        }
        if self.sym & SYM_Y != 0 {
            one(self, x, my);
        }
        if self.sym & SYM_X != 0 && self.sym & SYM_Y != 0 {
            one(self, mx, my);
        }
    }

    fn stamp(&mut self, b: &Brush, p: Pt) {
        let cache = b.cache();
        let (ox, oy) = (p.0 - b.w / 2, p.1 - b.h / 2);
        let erase = self.color[3] == 0;
        let prev = self.last_stamp.replace((p, erase));
        // Repainting a single-colored brush with the plain ink changes nothing where the previous
        // stamp already painted, so a one-pixel step only needs the pixels it newly covers.
        let edge = match prev {
            Some((q, e))
                if cache.uniform.is_some()
                    && self.settings.ink == Ink::Simple
                    && self.touched.is_none()
                    && e == erase =>
            {
                let (dx, dy) = (p.0 - q.0, p.1 - q.1);
                (dx.abs() <= 1 && dy.abs() <= 1).then(|| &cache.edges[ink::BrushCache::step_index(dx, dy)])
            }
            _ => None,
        };
        // Stamps that are entirely off the canvas paint nothing (mirrored ones may still land on it).
        if self.sym == 0 && (ox + b.w <= 0 || oy + b.h <= 0 || ox >= self.w || oy >= self.h) {
            return;
        }
        let paint =
            |s: &mut Self, (bx, by, c): (i32, i32, [u8; 4])| s.put(ox + bx, oy + by, if erase { [0; 4] } else { c });
        match edge {
            Some(list) => {
                for &i in list {
                    paint(self, cache.pixels[i as usize]);
                }
            }
            None => {
                for &px in &cache.pixels {
                    paint(self, px);
                }
            }
        }
    }
}

pub(crate) fn mirrors(p: Pt, s: (i32, i32), sym: u32) -> Vec<Pt> {
    let mut out = vec![p];
    if sym & SYM_X != 0 {
        out.push((s.0 - 1 - p.0, p.1));
    }
    if sym & SYM_Y != 0 {
        out.push((p.0, s.1 - 1 - p.1));
    }
    if sym & SYM_X != 0 && sym & SYM_Y != 0 {
        out.push((s.0 - 1 - p.0, s.1 - 1 - p.1));
    }
    out
}

/// A per-gesture painted-pixel map, only needed for inks that would otherwise stack.
pub(crate) fn tracking(dims: (i32, i32)) -> Option<Vec<bool>> {
    (ink::settings().ink != Ink::Simple).then(|| vec![false; (dims.0 * dims.1) as usize])
}

fn xorshift(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

impl Sprite {
    pub(crate) fn dims(&self) -> (i32, i32) {
        (self.width as i32, self.height as i32)
    }

    /// Draws a line, rectangle or ellipse between two corners. `flags` as in the module docs.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_shape(
        &mut self,
        layer: usize,
        frame: usize,
        shape: Shape,
        a: Pt,
        mut b: Pt,
        color: [u8; 4],
        size: u32,
        flags: u32,
    ) {
        let dims = self.dims();
        let Some(cel) = self.cel_mut(layer, frame) else { return };
        if flags & CONSTRAIN != 0 {
            b = if shape == Shape::Line { constrain_line(a, b) } else { constrain_square(a, b) };
        }
        let mut touched = tracking(dims);
        let mut p = Painter::new(cel, dims, color, size, flags).tracked(touched.as_deref_mut());
        let ((x0, y0), (x1, y1)) = normalize(a, b);
        let filled = flags & FILLED != 0;
        let outline = match shape {
            Shape::Line => line_points(a, b),
            Shape::Rect => {
                if filled {
                    p.rect(x0, y0, x1, y1);
                }
                rect_outline(a, b)
            }
            Shape::Ellipse => {
                if filled {
                    for (y, l, r) in ellipse_spans(a, b) {
                        p.rect(l, y, r, y);
                    }
                }
                ellipse_outline(a, b)
            }
        };
        for pt in outline {
            p.dot(pt);
        }
    }

    /// Scatters random dots inside a disc. Equal seeds give equal results.
    #[allow(clippy::too_many_arguments)]
    pub fn spray(
        &mut self,
        layer: usize,
        frame: usize,
        center: Pt,
        color: [u8; 4],
        radius: u32,
        density: u32,
        seed: u32,
        sym: u32,
    ) {
        let dims = self.dims();
        let Some(cel) = self.cel_mut(layer, frame) else { return };
        let r = radius.clamp(1, 256) as i32;
        let area = std::f64::consts::PI * (r * r) as f64;
        let count = ((area * density.clamp(1, 100) as f64 / 200.0).round() as u32).max(1);
        let mut state = seed ^ 0x9E37_79B9 | 1;
        let mut touched = tracking(dims);
        let mut p = Painter::new(cel, dims, color, 1, sym).plain().tracked(touched.as_deref_mut());
        for _ in 0..count {
            for _ in 0..16 {
                let dx = (xorshift(&mut state) % (2 * r as u32 + 1)) as i32 - r;
                let dy = (xorshift(&mut state) % (2 * r as u32 + 1)) as i32 - r;
                if dx * dx + dy * dy <= r * r {
                    p.dot((center.0 + dx, center.1 + dy));
                    break;
                }
            }
        }
    }

    /// Replaces every pixel in the cel that matches the one at `(x, y)` (non-contiguous fill).
    pub fn fill_all(&mut self, layer: usize, frame: usize, x: i32, y: i32, color: [u8; 4]) {
        let (w, h) = self.dims();
        if x < 0 || y < 0 || x >= w || y >= h {
            return;
        }
        let Some(cel) = self.cel_mut(layer, frame) else { return };
        let same = |a: &[u8], b: &[u8]| a == b || (a[3] == 0 && b[3] == 0);
        let i = (y * w + x) as usize * 4;
        let target: [u8; 4] = cel.pixels[i..i + 4].try_into().unwrap();
        if same(&target, &color) {
            return;
        }
        for px in cel.pixels.chunks_exact_mut(4) {
            if same(px, &target) {
                px.copy_from_slice(&color);
            }
        }
    }
}

/// An in-progress freehand stroke, optionally removing L-shaped corners.
pub struct Stroke {
    layer: usize,
    frame: usize,
    color: [u8; 4],
    size: u32,
    flags: u32,
    orig: Option<Arc<Cel>>,
    path: Vec<Pt>,
    touched: Option<Vec<bool>>,
    /// Pixels changed since the last `take_dirty`, `(x0, y0, x1, y1)` with the end exclusive.
    dirty: Option<(i32, i32, i32, i32)>,
    last_stamp: Option<(Pt, bool)>,
}

impl Stroke {
    /// The area changed since the previous call, for incremental re-rendering.
    pub fn take_dirty(&mut self) -> Option<(i32, i32, i32, i32)> {
        self.dirty.take()
    }

    /// Changes the color and brush size for the rest of the stroke (pen pressure).
    pub fn set_style(&mut self, color: [u8; 4], size: u32) {
        self.color = color;
        self.size = size;
    }

    pub fn begin(s: &mut Sprite, layer: usize, frame: usize, p: Pt, color: [u8; 4], size: u32, flags: u32) -> Stroke {
        let orig = s.cels.get(frame).and_then(|r| r.get(layer)).cloned().flatten();
        let touched = tracking(s.dims());
        let mut st =
            Stroke { layer, frame, color, size, flags, orig, path: Vec::new(), touched, dirty: None, last_stamp: None };
        st.add(s, p);
        st
    }

    /// Extends the stroke with a straight line to `p`.
    pub fn to(&mut self, s: &mut Sprite, p: Pt) {
        let Some(&last) = self.path.last() else { return };
        for q in line_points(last, p).into_iter().skip(1) {
            self.add(s, q);
        }
    }

    fn add(&mut self, s: &mut Sprite, q: Pt) {
        let dims = s.dims();
        let Some(cel) = s.cel_mut(self.layer, self.frame) else { return };
        let mut p = Painter::new(cel, dims, self.color, self.size, self.flags).tracked(self.touched.as_deref_mut());
        p.last_stamp = self.last_stamp;
        let n = self.path.len();
        let plain_pixel = self.size == 1 && p.settings.custom.is_none();
        if self.flags & PIXEL_PERFECT != 0 && plain_pixel && n >= 2 && is_corner(self.path[n - 2], self.path[n - 1], q)
        {
            let b = self.path.pop().unwrap();
            let gone = mirrors(b, dims, self.flags);
            for &(x, y) in &gone {
                if x < 0 || y < 0 || x >= dims.0 || y >= dims.1 {
                    continue;
                }
                p.untouch(x, y);
                p.note(x, y, x, y);
                let i = (y * dims.0 + x) as usize * 4;
                match &self.orig {
                    Some(o) => p.px[i..i + 4].copy_from_slice(&o.pixels[i..i + 4]),
                    None => p.px[i..i + 4].fill(0),
                }
            }
            for &k in self.path.iter().filter(|k| gone.contains(k)) {
                p.dot(k);
            }
        }
        self.path.push(q);
        p.dot(q);
        if let Some(b) = p.bounds {
            self.dirty = Some(self.dirty.map_or(b, |d| (d.0.min(b.0), d.1.min(b.1), d.2.max(b.2), d.3.max(b.3))));
        }
        self.last_stamp = p.last_stamp;
    }
}

/// Rewinds to where the open preview session began (see `Editor::begin_preview`), so a preview
/// can be redrawn from scratch. Does nothing when no session is open.
pub fn reset_to_checkpoint(e: &mut Editor) {
    e.reset_current_preview();
}

/// Starts a preview session (also a checkpoint) and returns its id, which is never 0.
#[no_mangle]
pub unsafe extern "C" fn pixmith_preview_begin(doc: *mut Editor) -> u64 {
    crate::ffi::guarded("pixmith_preview_begin", doc, || crate::ffi::editor(doc).map_or(0, |e| e.begin_preview()))
}

/// Rewinds the document to where session `id` began. False (and no change) if `id` is not the
/// open session, for instance because the history moved since.
#[no_mangle]
pub unsafe extern "C" fn pixmith_preview_reset(doc: *mut Editor, id: u64) -> bool {
    crate::ffi::guarded("pixmith_preview_reset", doc, || crate::ffi::editor(doc).is_some_and(|e| e.reset_preview(id)))
}

/// Closes session `id`; what was drawn stays.
#[no_mangle]
pub unsafe extern "C" fn pixmith_preview_end(doc: *mut Editor, id: u64) {
    crate::ffi::guarded("pixmith_preview_end", doc, || {
        if let Some(e) = crate::ffi::editor(doc) {
            e.end_preview(id);
        }
    })
}

thread_local! {
    /// Running freehand strokes by document pointer, so documents used alternately never share one.
    static STROKE: RefCell<std::collections::HashMap<usize, Stroke>> = RefCell::new(std::collections::HashMap::new());
}

/// Drops the running stroke of a document that is being freed (its address may be reused).
pub(crate) fn forget_stroke(doc: *mut Editor) {
    STROKE.with(|s| {
        s.borrow_mut().remove(&(doc as usize));
    });
}

fn rgba(c: u32) -> [u8; 4] {
    c.to_be_bytes()
}

/// Draws a shape between two corners. With `restore`, first rewinds to the checkpoint taken by
/// `pixmith_begin_stroke`, so repeated calls while dragging show a live preview in a single undo step.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_shape(
    doc: *mut Editor,
    kind: u32,
    layer: u32,
    frame: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: u32,
    size: u32,
    flags: u32,
    restore: bool,
) {
    crate::ffi::guarded("pixmith_shape", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return };
        let shape = match kind {
            0 => Shape::Line,
            1 => Shape::Rect,
            _ => Shape::Ellipse,
        };
        if restore {
            reset_to_checkpoint(e);
        }
        let (a, b) =
            ((crate::ffi::clampc(x0), crate::ffi::clampc(y0)), (crate::ffi::clampc(x1), crate::ffi::clampc(y1)));
        e.sprite.draw_shape(layer as usize, frame as usize, shape, a, b, rgba(color), size, flags);
        crate::paint::selection::clip_to_selection(e, layer as usize, frame as usize);
    })
}

/// Starts a freehand stroke at `(x, y)`; call `pixmith_begin_stroke` first for undo.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_stroke_begin(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    color: u32,
    size: u32,
    flags: u32,
) {
    crate::ffi::guarded("pixmith_stroke_begin", doc, || {
        let Some(e) = crate::ffi::editor_quiet(doc) else { return };
        let (x, y) = (crate::ffi::clampc(x), crate::ffi::clampc(y));
        let shared = e.cel_shared(layer as usize, frame as usize);
        let mut st = Stroke::begin(&mut e.sprite, layer as usize, frame as usize, (x, y), rgba(color), size, flags);
        crate::paint::selection::clip_to_selection(e, layer as usize, frame as usize);
        match st.take_dirty() {
            Some(r) => e.mark_cel_rect(layer as usize, frame as usize, r, shared),
            None => e.dirty.mark_all(),
        }
        STROKE.with(|s| {
            s.borrow_mut().insert(doc as usize, st);
        });
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_stroke_to(doc: *mut Editor, x: i32, y: i32) {
    crate::ffi::guarded("pixmith_stroke_to", doc, || {
        let Some(e) = crate::ffi::editor_quiet(doc) else { return };
        STROKE.with(|s| {
            if let Some(st) = s.borrow_mut().get_mut(&(doc as usize)) {
                let shared = e.cel_shared(st.layer, st.frame);
                st.to(&mut e.sprite, (crate::ffi::clampc(x), crate::ffi::clampc(y)));
                crate::paint::selection::clip_to_selection(e, st.layer, st.frame);
                if let Some(r) = st.take_dirty() {
                    e.mark_cel_rect(st.layer, st.frame, r, shared);
                }
            } else {
                e.dirty.mark_all();
            }
        });
    })
}

/// Discards the most recent undo step and restores its state, leaving nothing to redo (cancelled
/// previews).
#[no_mangle]
pub unsafe extern "C" fn pixmith_rollback(doc: *mut Editor) -> bool {
    crate::ffi::guarded("pixmith_rollback", doc, || crate::ffi::editor(doc).is_some_and(|e| e.rollback()))
}

/// Updates the running stroke's color and size, for pressure-sensitive pens.
#[no_mangle]
pub unsafe extern "C" fn pixmith_stroke_style(doc: *mut Editor, color: u32, size: u32) {
    crate::ffi::guarded("pixmith_stroke_style", doc, || {
        STROKE.with(|s| {
            if let Some(st) = s.borrow_mut().get_mut(&(doc as usize)) {
                st.set_style(rgba(color), size);
            }
        });
    })
}

#[no_mangle]
pub extern "C" fn pixmith_stroke_end() {
    crate::ffi::guarded("pixmith_stroke_end", std::ptr::null_mut(), || {
        STROKE.with(|s| s.borrow_mut().clear());
    })
}

/// Ends only the running stroke of `doc`.
#[no_mangle]
pub extern "C" fn pixmith_stroke_end_doc(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_stroke_end_doc", doc, || {
        forget_stroke(doc);
    })
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_spray(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    color: u32,
    radius: u32,
    density: u32,
    seed: u32,
    flags: u32,
) {
    crate::ffi::guarded("pixmith_spray", doc, || {
        if let Some(e) = crate::ffi::editor(doc) {
            e.sprite.spray(
                layer as usize,
                frame as usize,
                (crate::ffi::clampc(x), crate::ffi::clampc(y)),
                rgba(color),
                radius,
                density,
                seed,
                flags,
            );
            crate::paint::selection::clip_to_selection(e, layer as usize, frame as usize);
        }
    })
}

/// Fill that can replace every matching pixel in the cel instead of only the connected area.
#[no_mangle]
pub unsafe extern "C" fn pixmith_fill_mode(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    color: u32,
    contiguous: bool,
) {
    crate::ffi::guarded("pixmith_fill_mode", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return };
        let (l, f) = (layer as usize, frame as usize);
        if contiguous {
            crate::paint::selection::fill(e, l, f, x, y, rgba(color));
        } else {
            e.sprite.fill_all(l, f, x, y, rgba(color));
            crate::paint::selection::clip_to_selection(e, l, f);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];

    fn lit(s: &Sprite) -> Vec<Pt> {
        let px = s.render(0);
        let w = s.width as usize;
        (0..px.len() / 4).filter(|&i| px[i * 4 + 3] != 0).map(|i| ((i % w) as i32, (i / w) as i32)).collect()
    }

    #[test]
    fn line_includes_endpoints() {
        let l = line_points((0, 0), (3, 1));
        assert_eq!((l[0], *l.last().unwrap()), ((0, 0), (3, 1)));
        assert_eq!(l.len(), 4);
        assert_eq!(line_points((2, 2), (2, 2)), vec![(2, 2)]);
    }

    #[test]
    fn constrain_snaps_to_octants() {
        assert_eq!(constrain_line((0, 0), (10, 2)), (10, 0));
        assert_eq!(constrain_line((0, 0), (2, -10)), (0, -10));
        assert_eq!(constrain_line((5, 5), (1, 2)), (1, 1));
        assert_eq!(constrain_square((0, 0), (3, -7)), (7, -7));
    }

    #[test]
    fn rect_outline_has_no_duplicates() {
        let o = rect_outline((1, 1), (4, 3));
        assert_eq!(o.len(), 4 + 4 + 2);
        let mut u = o.clone();
        u.sort();
        u.dedup();
        assert_eq!(u.len(), o.len());
        assert_eq!(rect_outline((2, 2), (2, 2)), vec![(2, 2)]);
        assert_eq!(rect_outline((0, 0), (0, 2)).len(), 3);
    }

    #[test]
    fn circle_spans_are_symmetric() {
        let widths: Vec<i32> = ellipse_spans((0, 0), (4, 4)).iter().map(|&(_, l, r)| r - l + 1).collect();
        assert_eq!(widths, vec![3, 5, 5, 5, 3]);
        for n in 1..14 {
            let spans = ellipse_spans((0, 0), (n, n + 3));
            let h = spans.len();
            for (i, &(_, l, r)) in spans.iter().enumerate() {
                assert_eq!(l, n - r, "mirror in x for width {}", n + 1);
                assert_eq!((l, r), (spans[h - 1 - i].1, spans[h - 1 - i].2), "mirror in y");
            }
        }
    }

    #[test]
    fn ellipse_outline_is_connected_and_thin() {
        let o = ellipse_outline((0, 0), (10, 6));
        let mut set: Vec<Pt> = o.clone();
        set.sort();
        set.dedup();
        for &(x, y) in &set {
            let n = set.iter().filter(|&&(a, b)| (a, b) != (x, y) && (a - x).abs() <= 1 && (b - y).abs() <= 1).count();
            assert!(n >= 2, "pixel {x},{y} has {n} neighbours");
        }
        assert!(set.contains(&(5, 0)) && set.contains(&(5, 6)) && set.contains(&(0, 3)) && set.contains(&(10, 3)));
        assert!(!set.contains(&(5, 3)));
    }

    #[test]
    fn filled_shapes_cover_interior() {
        let mut s = Sprite::new(8, 8);
        s.draw_shape(0, 0, Shape::Rect, (1, 1), (4, 3), RED, 1, FILLED);
        assert_eq!(lit(&s).len(), 12);
        let mut s = Sprite::new(8, 8);
        s.draw_shape(0, 0, Shape::Rect, (1, 1), (4, 3), RED, 1, 0);
        assert_eq!(lit(&s).len(), 10);
        let mut s = Sprite::new(8, 8);
        s.draw_shape(0, 0, Shape::Ellipse, (0, 0), (4, 4), RED, 1, FILLED);
        assert_eq!(lit(&s).len(), 3 + 5 + 5 + 5 + 3);
    }

    #[test]
    fn constrained_ellipse_is_a_circle() {
        let mut s = Sprite::new(16, 16);
        s.draw_shape(0, 0, Shape::Ellipse, (2, 2), (8, 4), RED, 1, FILLED | CONSTRAIN);
        let px = lit(&s);
        assert_eq!(px.iter().map(|p| p.0).max(), px.iter().map(|p| p.1).max());
    }

    #[test]
    fn symmetry_mirrors_across_center() {
        let mut s = Sprite::new(8, 6);
        s.draw_shape(0, 0, Shape::Line, (1, 1), (1, 1), RED, 1, SYM_X);
        assert_eq!(lit(&s), vec![(1, 1), (6, 1)]);
        let mut s = Sprite::new(8, 6);
        s.draw_shape(0, 0, Shape::Line, (1, 1), (1, 1), RED, 1, SYM_X | SYM_Y);
        assert_eq!(lit(&s), vec![(1, 1), (6, 1), (1, 4), (6, 4)]);
        let mut s = Sprite::new(8, 6);
        s.draw_shape(0, 0, Shape::Line, (0, 0), (0, 0), RED, 2, SYM_X);
        assert_eq!(lit(&s).len(), 8);
        assert!(lit(&s).contains(&(7, 1)));
    }

    #[test]
    fn pixel_perfect_removes_corners() {
        let mut s = Sprite::new(4, 4);
        let mut st = Stroke::begin(&mut s, 0, 0, (0, 0), RED, 1, 0);
        st.to(&mut s, (1, 0));
        st.to(&mut s, (1, 1));
        assert_eq!(lit(&s).len(), 3);

        let mut s = Sprite::new(4, 4);
        let mut st = Stroke::begin(&mut s, 0, 0, (0, 0), RED, 1, PIXEL_PERFECT);
        st.to(&mut s, (1, 0));
        st.to(&mut s, (1, 1));
        assert_eq!(lit(&s), vec![(0, 0), (1, 1)]);
        st.to(&mut s, (1, 2));
        assert_eq!(lit(&s), vec![(0, 0), (1, 1), (1, 2)]);
    }

    #[test]
    fn pixel_perfect_keeps_prior_pixels_and_mirrors() {
        let mut s = Sprite::new(6, 4);
        s.draw_shape(0, 0, Shape::Line, (1, 0), (1, 0), RED, 1, 0);
        let mut st = Stroke::begin(&mut s, 0, 0, (0, 0), [0, 0, 255, 255], 1, PIXEL_PERFECT);
        st.to(&mut s, (1, 0));
        st.to(&mut s, (1, 1));
        let px = s.render(0);
        assert_eq!(&px[4..8], &RED); // the removed corner gets its earlier color back
        let mut s = Sprite::new(6, 4);
        let mut st = Stroke::begin(&mut s, 0, 0, (0, 0), RED, 1, PIXEL_PERFECT | SYM_X);
        st.to(&mut s, (1, 0));
        st.to(&mut s, (1, 1));
        assert_eq!(lit(&s), vec![(0, 0), (5, 0), (1, 1), (4, 1)]);
    }

    #[test]
    fn spray_is_deterministic_and_bounded() {
        let mut a = Sprite::new(32, 32);
        let mut b = Sprite::new(32, 32);
        a.spray(0, 0, (16, 16), RED, 5, 50, 7, 0);
        b.spray(0, 0, (16, 16), RED, 5, 50, 7, 0);
        assert_eq!(lit(&a), lit(&b));
        assert!(!lit(&a).is_empty());
        assert!(lit(&a).iter().all(|&(x, y)| (x - 16).pow(2) + (y - 16).pow(2) <= 25));
        let mut c = Sprite::new(32, 32);
        c.spray(0, 0, (16, 16), RED, 5, 50, 8, 0);
        assert_ne!(lit(&a), lit(&c));
    }

    #[test]
    fn non_contiguous_fill_replaces_all_matches() {
        let mut s = Sprite::new(5, 1);
        s.draw_shape(0, 0, Shape::Line, (2, 0), (2, 0), RED, 1, 0);
        s.fill_all(0, 0, 0, 0, [0, 0, 255, 255]);
        let px = s.render(0);
        assert_eq!(&px[16..20], &[0, 0, 255, 255]);
        assert_eq!(&px[8..12], &RED);
        s.fill_all(0, 0, 2, 0, [0, 255, 0, 255]);
        assert_eq!(&s.render(0)[8..12], &[0, 255, 0, 255]);
    }

    #[test]
    fn alpha_ink_does_not_stack_within_a_stroke() {
        ink::update_settings(|s| s.ink = Ink::Alpha);
        let half = [255, 0, 0, 128];
        let mut s = Sprite::new(8, 8);
        let mut st = Stroke::begin(&mut s, 0, 0, (1, 1), half, 3, 0);
        st.to(&mut s, (5, 1));
        st.to(&mut s, (1, 1));
        assert_eq!(s.render(0)[(8 + 3) * 4 + 3], 128);
        let mut s = Sprite::new(8, 8);
        s.draw_shape(0, 0, Shape::Rect, (1, 1), (5, 5), half, 3, FILLED);
        assert_eq!(s.render(0)[(3 * 8 + 3) * 4 + 3], 128);
    }

    #[test]
    fn shading_steps_once_per_gesture() {
        let ramp = vec![[10, 0, 0, 255], [20, 0, 0, 255], [30, 0, 0, 255]];
        let mut s = Sprite::new(8, 8);
        s.draw_shape(0, 0, Shape::Rect, (1, 1), (5, 5), [10, 0, 0, 255], 1, FILLED);
        ink::update_settings(|s| {
            s.ink = Ink::Shading;
            s.ramp = ramp;
        });
        s.draw_shape(0, 0, Shape::Line, (1, 3), (5, 3), RED, 3, 0);
        let px = s.render(0);
        assert_eq!(px[(3 * 8 + 3) * 4], 20);
        assert_eq!(px[(2 * 8 + 3) * 4], 20);
        assert_eq!(px[(5 * 8 + 3) * 4], 10);
    }

    #[test]
    fn behind_and_lock_alpha_in_shapes() {
        let mut s = Sprite::new(6, 1);
        s.draw_shape(0, 0, Shape::Line, (2, 0), (2, 0), [0, 0, 255, 255], 1, 0);
        ink::update_settings(|s| s.ink = Ink::Behind);
        s.draw_shape(0, 0, Shape::Line, (0, 0), (5, 0), RED, 1, 0);
        let px = s.render(0);
        assert_eq!(&px[8..12], &[0, 0, 255, 255]);
        assert_eq!(&px[0..4], &RED);
        let mut s = Sprite::new(6, 1);
        s.draw_shape(0, 0, Shape::Line, (2, 0), (2, 0), [0, 0, 255, 255], 1, 0);
        ink::update_settings(|s| s.ink = Ink::LockAlpha);
        s.draw_shape(0, 0, Shape::Line, (0, 0), (5, 0), RED, 1, 0);
        assert_eq!(lit(&s), vec![(2, 0)]);
        assert_eq!(&s.render(0)[8..12], &RED);
    }

    #[test]
    fn circle_brush_rounds_the_corners() {
        ink::update_settings(|s| s.circle = true);
        let mut s = Sprite::new(8, 8);
        s.draw_shape(0, 0, Shape::Line, (3, 3), (3, 3), RED, 4, 0);
        assert_eq!(lit(&s).len(), 12);
        let mut s = Sprite::new(8, 8);
        s.draw_shape(0, 0, Shape::Line, (3, 3), (3, 3), RED, 3, 0);
        assert_eq!(lit(&s).len(), 9);
        let mut s = Sprite::new(16, 16);
        s.draw_shape(0, 0, Shape::Line, (8, 8), (8, 8), RED, 9, 0);
        let n = lit(&s).len();
        assert!((60..=70).contains(&n), "{n}");
    }

    #[test]
    fn custom_brush_stamps_its_pixels_and_mirrors() {
        let mut px = vec![0u8; 2 * 2 * 4];
        px[0..4].copy_from_slice(&RED);
        px[12..16].copy_from_slice(&[0, 0, 255, 255]);
        ink::update_settings(|s| s.custom = Brush::from_rgba(2, 2, &px).map(Rc::new));
        let mut s = Sprite::new(8, 4);
        s.draw_shape(0, 0, Shape::Line, (2, 2), (2, 2), [0, 255, 0, 255], 1, 0);
        assert_eq!(lit(&s), vec![(1, 1), (2, 2)]);
        let px = s.render(0);
        assert_eq!(&px[(8 + 1) * 4..][..4], &RED);
        assert_eq!(&px[(2 * 8 + 2) * 4..][..4], &[0, 0, 255, 255]);
        let mut s = Sprite::new(8, 4);
        s.draw_shape(0, 0, Shape::Line, (2, 2), (2, 2), RED, 1, SYM_X);
        assert_eq!(lit(&s).len(), 4);
        s.draw_shape(0, 0, Shape::Line, (2, 2), (2, 2), [0; 4], 1, 0);
        assert_eq!(lit(&s).len(), 2);
        // spray ignores the custom brush
        let mut s = Sprite::new(16, 16);
        s.spray(0, 0, (8, 8), RED, 3, 100, 1, 0);
        assert!(lit(&s).iter().all(|&(x, y)| (x - 8).pow(2) + (y - 8).pow(2) <= 9));
        assert!(s.render(0).chunks(4).all(|c| c[3] == 0 || c == RED));
    }

    #[test]
    fn preview_keeps_the_undo_step_label() {
        let mut e = Editor::new(Sprite::new(8, 8));
        e.begin_preview();
        e.label_last("Line");
        for end in [(2, 2), (6, 6)] {
            reset_to_checkpoint(&mut e);
            e.sprite.draw_shape(0, 0, Shape::Line, (0, 0), end, RED, 1, 0);
        }
        assert_eq!(e.history_len(), (1, 0));
        assert_eq!(e.history_label(0), Some("Line"));
        assert!(e.undo());
        assert!(lit(&e.sprite).is_empty());
    }

    #[test]
    fn preview_redraws_in_one_undo_step() {
        let mut e = Editor::new(Sprite::new(8, 8));
        e.begin_preview();
        for end in [(2, 2), (5, 5), (6, 1)] {
            reset_to_checkpoint(&mut e);
            e.sprite.draw_shape(0, 0, Shape::Line, (0, 0), end, RED, 1, 0);
        }
        assert_eq!(lit(&e.sprite).last(), Some(&(6, 1)));
        assert_eq!(lit(&e.sprite).len(), 7);
        assert!(e.undo());
        assert!(lit(&e.sprite).is_empty());
        assert!(!e.undo());
    }

    #[test]
    fn reset_without_an_open_session_changes_nothing() {
        let mut e = Editor::new(Sprite::new(8, 8));
        e.checkpoint();
        e.sprite.draw_shape(0, 0, Shape::Line, (0, 0), (3, 3), RED, 1, 0);
        let before = e.sprite.clone();
        // A checkpoint is not a preview session, so there is nothing to rewind to.
        reset_to_checkpoint(&mut e);
        assert_eq!(e.sprite, before);
        assert!(!e.reset_preview(1));
        assert_eq!(e.sprite, before);
        assert_eq!(e.history_len(), (1, 0));
    }

    #[test]
    fn preview_tokens_match_only_their_own_session() {
        let mut e = Editor::new(Sprite::new(8, 8));
        let first = e.begin_preview();
        e.sprite.draw_shape(0, 0, Shape::Line, (0, 0), (3, 3), RED, 1, 0);
        let second = e.begin_preview();
        assert_ne!(first, second);
        assert!(first != 0 && second != 0);
        e.sprite.draw_shape(0, 0, Shape::Line, (7, 0), (7, 5), RED, 1, 0);
        assert!(!e.reset_preview(first), "a replaced session is gone");
        assert_eq!(lit(&e.sprite).len(), 4 + 6);
        assert!(e.reset_preview(second));
        assert_eq!(lit(&e.sprite).len(), 4, "only the second stroke is rewound");
        // Ending the session, or moving the history, makes the token stale.
        e.sprite.draw_shape(0, 0, Shape::Line, (7, 0), (7, 5), RED, 1, 0);
        e.end_preview(second);
        assert!(!e.reset_preview(second));
        let third = e.begin_preview();
        assert!(e.undo());
        assert!(!e.reset_preview(third), "undo makes the session stale");
    }

    #[test]
    fn previews_inside_a_transaction_keep_earlier_strokes() {
        let mut e = Editor::new(Sprite::new(8, 8));
        e.begin_transaction();
        e.begin_preview();
        e.sprite.draw_shape(0, 0, Shape::Line, (0, 0), (3, 0), RED, 1, 0);
        e.begin_preview();
        for end in [(0, 3), (0, 6)] {
            reset_to_checkpoint(&mut e);
            e.sprite.draw_shape(0, 0, Shape::Line, (0, 7), end, RED, 1, 0);
        }
        e.end_transaction();
        let lit_now = lit(&e.sprite);
        assert!(lit_now.contains(&(3, 0)), "the first stroke survives the second one's preview");
        assert!(lit_now.contains(&(0, 6)) && lit_now.contains(&(0, 7)), "the final preview is drawn");
        assert!(!lit_now.contains(&(0, 3)), "the earlier preview of that stroke is gone");
        assert_eq!(e.history_len(), (1, 0), "the whole transaction is one undo step");
        assert!(e.undo());
        assert!(lit(&e.sprite).is_empty());
    }

    /// Paints every opaque brush pixel at every point, last stamp wins: the plain definition.
    fn naive_stamps(w: i32, h: i32, brush: &Brush, pts: &[Pt], erase: bool) -> Vec<u8> {
        let mut px = vec![0u8; (w * h * 4) as usize];
        for &(x, y) in pts {
            let (ox, oy) = (x - brush.w / 2, y - brush.h / 2);
            for by in 0..brush.h {
                for bx in 0..brush.w {
                    let c: [u8; 4] = brush.px[(by * brush.w + bx) as usize * 4..][..4].try_into().unwrap();
                    let (tx, ty) = (ox + bx, oy + by);
                    if c[3] != 0 && tx >= 0 && ty >= 0 && tx < w && ty < h {
                        let i = (ty * w + tx) as usize * 4;
                        px[i..i + 4].copy_from_slice(&if erase { [0; 4] } else { c });
                    }
                }
            }
        }
        px
    }

    fn brush_of(w: i32, h: i32, color: impl Fn(i32, i32) -> Option<[u8; 4]>) -> Brush {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.extend(color(x, y).unwrap_or([0; 4]));
            }
        }
        Brush::from_rgba(w, h, &px).unwrap()
    }

    #[test]
    fn long_strokes_with_custom_brushes_match_plain_stamping() {
        let uniform = brush_of(9, 7, |x, y| ((x * 3 + y) % 4 != 0 || x == 4).then_some([10, 200, 30, 255]));
        let multi = brush_of(8, 8, |x, y| ((x + y) % 5 != 1).then_some([x as u8 * 30 + 5, y as u8 * 30 + 5, 90, 255]));
        let paths: [Vec<Pt>; 3] = [
            line_points((-5, 3), (60, 40)),
            line_points((50, 2), (3, 45)).into_iter().chain(line_points((3, 45), (40, 45))).collect(),
            // Jumps (not one-pixel steps) fall back to full stamps.
            vec![(2, 2), (3, 3), (20, 20), (21, 20), (21, 21), (5, 40), (6, 40), (6, 40)],
        ];
        for brush in [uniform, multi] {
            for (n, pts) in paths.iter().enumerate() {
                for erase in [false, true] {
                    let (w, h) = (48, 48);
                    let mut s = Sprite::new(w as u32, h as u32);
                    // Start from pixels the erasing case can remove.
                    s.draw_shape(0, 0, Shape::Rect, (0, 0), (47, 47), [1, 2, 3, 255], 1, FILLED);
                    let base = s.cels[0][0].as_ref().unwrap().pixels.clone();
                    let mut expect = naive_stamps(w, h, &brush, pts, erase);
                    for (i, e) in expect.chunks_exact_mut(4).enumerate() {
                        if e[3] == 0 && !erase {
                            e.copy_from_slice(&base[i * 4..i * 4 + 4]);
                        } else if erase && e[3] == 0 {
                            // naive_stamps writes zeros for erased pixels and for untouched ones alike
                        }
                    }
                    if erase {
                        // Untouched pixels keep the base color; erased ones are transparent.
                        let touched = naive_stamps(w, h, &brush, pts, false);
                        for (i, e) in expect.chunks_exact_mut(4).enumerate() {
                            if touched[i * 4 + 3] == 0 {
                                e.copy_from_slice(&base[i * 4..i * 4 + 4]);
                            }
                        }
                    }
                    ink::update_settings(|st| {
                        st.custom = Some(Rc::new(Brush::new(brush.w, brush.h, brush.px.clone())))
                    });
                    let color = if erase { [0; 4] } else { [9, 9, 9, 255] };
                    let (dims, cel) = (s.dims(), s.cel_mut(0, 0).unwrap());
                    let mut p = Painter::new(cel, dims, color, 1, 0);
                    for &q in pts {
                        p.dot(q);
                    }
                    ink::update_settings(|st| st.custom = None);
                    assert_eq!(s.cels[0][0].as_ref().unwrap().pixels, expect, "path {n} erase {erase}");
                }
            }
        }
    }

    #[test]
    fn freehand_strokes_use_the_same_fast_path() {
        let brush = brush_of(6, 6, |x, y| ((x + y) % 7 != 0).then_some([200, 50, 50, 255]));
        let pts = line_points((2, 2), (40, 30));
        ink::update_settings(|st| st.custom = Some(Rc::new(Brush::new(brush.w, brush.h, brush.px.clone()))));
        let mut s = Sprite::new(48, 48);
        let mut st = Stroke::begin(&mut s, 0, 0, pts[0], [1, 1, 1, 255], 1, 0);
        st.to(&mut s, *pts.last().unwrap());
        ink::update_settings(|st| st.custom = None);
        assert_eq!(s.cels[0][0].as_ref().unwrap().pixels, naive_stamps(48, 48, &brush, &pts, false));
    }

    fn time_stroke(brush: Brush) -> std::time::Duration {
        ink::update_settings(|st| st.custom = Some(Rc::new(brush)));
        let mut s = Sprite::new(2100, 200);
        let started = std::time::Instant::now();
        s.draw_shape(0, 0, Shape::Line, (20, 100), (2020, 100), [0, 0, 0, 255], 1, 0);
        let took = started.elapsed();
        ink::update_settings(|st| st.custom = None);
        took
    }

    #[test]
    fn a_2000_pixel_stroke_with_a_64x64_brush_is_fast() {
        let uniform =
            brush_of(64, 64, |x, y| ((x - 32) * (x - 32) + (y - 32) * (y - 32) < 1000).then_some([5, 5, 5, 255]));
        let multi = brush_of(64, 64, |x, y| Some([x as u8 * 3, y as u8 * 3, 7, 255]));
        // Generous bounds for unoptimized builds; the release timing test below pins the real target.
        assert!(time_stroke(uniform).as_millis() < 1500);
        assert!(time_stroke(multi).as_millis() < 20000);
    }

    #[test]
    #[ignore = "timing: run with --release"]
    fn release_2000_pixel_stroke_with_a_64x64_brush_takes_under_50ms() {
        let uniform =
            brush_of(64, 64, |x, y| ((x - 32) * (x - 32) + (y - 32) * (y - 32) < 1000).then_some([5, 5, 5, 255]));
        let multi = brush_of(64, 64, |x, y| Some([x as u8 * 3, y as u8 * 3, 7, 255]));
        for (name, b) in [("uniform", uniform), ("multicolor", multi)] {
            let took = time_stroke(b);
            assert!(took.as_millis() < 50, "{name} brush: {took:?}");
        }
    }
}
