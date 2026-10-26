//! Pixel-art warp (puppet / mesh warp): bends a cel or selection with a grid of control handles and
//! optional pin handles, and resamples it by **majority vote** so the result has no blur and no new
//! colors.
//!
//! The deformation is a displacement field over a region (the selection bounds, or the canvas): a
//! `cols` x `rows` grid of handles spread evenly over the region, each carrying a displacement in
//! pixels, interpolated bilinearly (or with a smooth Catmull-Rom spline), plus any number of pins
//! (a point, a displacement and a radius with a smooth falloff). A handle's displacement is where the
//! content at that handle goes: the forward map is `p + displacement(p)`.
//!
//! Resampling walks the output: every output pixel is cut into `samples` x `samples` points, each
//! point is mapped back to the source (the forward map is inverted by fixed-point iteration), and the
//! most frequent source color wins. Transparent counts as a color (so shapes keep crisp edges), a tie
//! goes to a visible color over transparent and then to the color of the sample closest to the pixel
//! center (scan order among equals), nothing is ever blended. Without a selection the whole canvas is warped and pixels the source does not cover
//! turn transparent; with one, only selected pixels are read and written, so everything outside it
//! stays exactly as it was and is never dragged in.
//!
//! The FFI wrappers at the bottom make an apply one undo step, and offer a preview that leaves the
//! document and its history alone.

use crate::document::{Editor, Sprite};
use crate::paint::filters::{self, Target};
use crate::paint::selection::Selection;

/// Most handles along either axis (the grid is 2x2 up to this).
pub const MAX_GRID: u32 = 8;
/// Most sample points per pixel along either axis.
pub const MAX_SAMPLES: u32 = 8;
/// Most pins in one warp.
pub const MAX_PINS: usize = 32;
/// Displacements are clamped to this many pixels.
const MAX_OFFSET: f64 = 4096.0;
/// Fixed-point steps spent inverting the forward map for one point.
const INVERT_STEPS: usize = 24;

type P2 = (f64, f64);

fn finite(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(-MAX_OFFSET, MAX_OFFSET)
    } else {
        0.0
    }
}

/// A point handle: content within `radius` of (`x`, `y`) (canvas coordinates, pixel corners are
/// integers) moves by up to (`dx`, `dy`), fully at the pin and fading out smoothly towards the radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pin {
    pub x: f64,
    pub y: f64,
    pub dx: f64,
    pub dy: f64,
    pub radius: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Warp {
    /// Where the handle grid lies: x, y, width, height (the first handle is the top-left corner, the
    /// last the bottom-right one). Outside it the field continues with the nearest edge value.
    pub region: (i32, i32, u32, u32),
    pub cols: u32,
    pub rows: u32,
    /// `cols * rows` displacements in pixels, row by row.
    pub offsets: Vec<P2>,
    pub pins: Vec<Pin>,
    /// Interpolate with a Catmull-Rom spline instead of bilinearly (smooth, passes through every handle).
    pub smooth: bool,
    /// Sample points per output pixel along each axis, 1..=`MAX_SAMPLES`.
    pub samples: u32,
}

fn add(a: P2, b: P2) -> P2 {
    (a.0 + b.0, a.1 + b.1)
}

fn sub(a: P2, b: P2) -> P2 {
    (a.0 - b.0, a.1 - b.1)
}

fn scale(a: P2, k: f64) -> P2 {
    (a.0 * k, a.1 * k)
}

fn catmull(p: [P2; 4], t: f64) -> P2 {
    let f = |a: f64, b: f64, c: f64, d: f64| {
        0.5 * (2.0 * b
            + (c - a) * t
            + (2.0 * a - 5.0 * b + 4.0 * c - d) * t * t
            + (3.0 * b - a - 3.0 * c + d) * t * t * t)
    };
    (f(p[0].0, p[1].0, p[2].0, p[3].0), f(p[0].1, p[1].1, p[2].1, p[3].1))
}

impl Warp {
    /// An identity warp with a `cols` x `rows` grid (each clamped to 2..=`MAX_GRID`) over `region`.
    pub fn new(cols: u32, rows: u32, region: (i32, i32, u32, u32)) -> Warp {
        let (cols, rows) = (cols.clamp(2, MAX_GRID), rows.clamp(2, MAX_GRID));
        Warp {
            region,
            cols,
            rows,
            offsets: vec![(0.0, 0.0); (cols * rows) as usize],
            pins: Vec::new(),
            smooth: false,
            samples: 4,
        }
    }

    /// Builds a warp from flat arrays, as the C ABI passes them: `offsets` holds `cols * rows` x, y
    /// pairs and `pins` holds x, y, dx, dy, radius for each pin. Returns `None` for a grid outside
    /// 2..=`MAX_GRID`, an offset list of the wrong length, a ragged pin list or too many pins.
    pub fn from_flat(
        region: (i32, i32, u32, u32),
        (cols, rows): (u32, u32),
        offsets: &[f64],
        samples: u32,
        smooth: bool,
        pins: &[f64],
    ) -> Option<Warp> {
        if !(2..=MAX_GRID).contains(&cols) || !(2..=MAX_GRID).contains(&rows) {
            return None;
        }
        if offsets.len() != (cols * rows * 2) as usize || !pins.len().is_multiple_of(5) || pins.len() / 5 > MAX_PINS {
            return None;
        }
        let mut w = Warp::new(cols, rows, region);
        w.offsets = offsets.chunks_exact(2).map(|c| (finite(c[0]), finite(c[1]))).collect();
        w.pins = pins
            .chunks_exact(5)
            .map(|c| Pin {
                x: finite(c[0]),
                y: finite(c[1]),
                dx: finite(c[2]),
                dy: finite(c[3]),
                radius: finite(c[4]).max(0.0),
            })
            .collect();
        w.smooth = smooth;
        w.samples = samples.clamp(1, MAX_SAMPLES);
        Some(w)
    }

    /// Canvas position of handle (`i`, `j`).
    pub fn handle_pos(&self, i: u32, j: u32) -> P2 {
        let (x, y, w, h) = self.region;
        (
            x as f64 + w as f64 * i as f64 / (self.cols - 1) as f64,
            y as f64 + h as f64 * j as f64 / (self.rows - 1) as f64,
        )
    }

    /// True when nothing moves.
    pub fn is_identity(&self) -> bool {
        self.offsets.iter().all(|o| *o == (0.0, 0.0)) && self.pins.iter().all(|p| p.dx == 0.0 && p.dy == 0.0)
    }

    /// Displacement of handle (`i`, `j`); handles beyond the grid continue the edge linearly.
    fn at(&self, i: i32, j: i32) -> P2 {
        let (c, r) = (self.cols as i32, self.rows as i32);
        if i < 0 {
            return sub(scale(self.at(0, j), 2.0), self.at(1, j));
        }
        if i >= c {
            return sub(scale(self.at(c - 1, j), 2.0), self.at(c - 2, j));
        }
        if j < 0 {
            return sub(scale(self.at(i, 0), 2.0), self.at(i, 1));
        }
        if j >= r {
            return sub(scale(self.at(i, r - 1), 2.0), self.at(i, r - 2));
        }
        self.offsets.get((j * c + i) as usize).copied().unwrap_or((0.0, 0.0))
    }

    /// The displacement field at source point `p`.
    pub fn displacement(&self, p: P2) -> P2 {
        let (x, y, w, h) = self.region;
        let axis = |v: f64, lo: i32, len: u32, n: u32| {
            if len == 0 {
                0.0
            } else {
                ((v - lo as f64) / len as f64 * (n - 1) as f64).clamp(0.0, (n - 1) as f64)
            }
        };
        let (u, v) = (axis(p.0, x, w, self.cols), axis(p.1, y, h, self.rows));
        let (i0, j0) = ((u.floor() as i32).min(self.cols as i32 - 2), (v.floor() as i32).min(self.rows as i32 - 2));
        let (tx, ty) = (u - i0 as f64, v - j0 as f64);
        let mut d = if self.smooth {
            let row =
                |j: i32| catmull([self.at(i0 - 1, j), self.at(i0, j), self.at(i0 + 1, j), self.at(i0 + 2, j)], tx);
            catmull([row(j0 - 1), row(j0), row(j0 + 1), row(j0 + 2)], ty)
        } else {
            let lerp = |a: P2, b: P2, t: f64| add(scale(a, 1.0 - t), scale(b, t));
            lerp(
                lerp(self.at(i0, j0), self.at(i0 + 1, j0), tx),
                lerp(self.at(i0, j0 + 1), self.at(i0 + 1, j0 + 1), tx),
                ty,
            )
        };
        for pin in &self.pins {
            let dist = ((p.0 - pin.x).powi(2) + (p.1 - pin.y).powi(2)).sqrt();
            if dist < pin.radius {
                let t = 1.0 - dist / pin.radius;
                let weight = t * t * (3.0 - 2.0 * t);
                d = add(d, (pin.dx * weight, pin.dy * weight));
            }
        }
        d
    }

    /// The source point that lands on output point `out`: solves `q + displacement(q) = out` by
    /// fixed-point iteration, which is exact for shifts and converges for any bend that does not fold.
    pub fn source_of(&self, out: P2) -> P2 {
        let mut q = out;
        for _ in 0..INVERT_STEPS {
            let d = self.displacement(q);
            let next = (out.0 - d.0, out.1 - d.1);
            let moved = (next.0 - q.0).abs().max((next.1 - q.1).abs());
            q = next;
            if moved < 1e-7 {
                break;
            }
        }
        q
    }

    /// Warps a straight-alpha RGBA canvas buffer (`w` x `h`) by majority vote. With a `mask`, only its
    /// pixels are read or written; the rest of the result is a copy of `src`.
    pub fn run(&self, src: &[u8], w: usize, h: usize, mask: Option<&Selection>) -> Vec<u8> {
        let mut out = src.to_vec();
        if self.is_identity() || src.len() != w * h * 4 {
            return out;
        }
        let n = self.samples.clamp(1, MAX_SAMPLES) as usize;
        let order = sample_order(n);
        let key_at = |x: i64, y: i64| -> u32 {
            if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 || mask.is_some_and(|m| !m.get(x as i32, y as i32)) {
                return 0;
            }
            let i = (y as usize * w + x as usize) * 4;
            if src[i + 3] == 0 {
                0
            } else {
                u32::from_be_bytes([src[i], src[i + 1], src[i + 2], src[i + 3]])
            }
        };
        let mut tally: Vec<(u32, u32)> = Vec::with_capacity(order.len());
        for y in 0..h {
            for x in 0..w {
                if mask.is_some_and(|m| !m.get(x as i32, y as i32)) {
                    continue;
                }
                tally.clear();
                for &(ox, oy) in &order {
                    let q = self.source_of((x as f64 + ox, y as f64 + oy));
                    let key = key_at(q.0.floor() as i64, q.1.floor() as i64);
                    match tally.iter_mut().find(|t| t.0 == key) {
                        Some(t) => t.1 += 1,
                        None => tally.push((key, 1)),
                    }
                }
                // Most votes win. A tie goes to a visible color over transparent (so thin lines do not
                // lose pixels where they straddle two output pixels), then to the color met first, i.e.
                // nearest to the pixel center.
                let mut best = tally[0];
                for t in &tally[1..] {
                    if t.1 > best.1 || (t.1 == best.1 && best.0 == 0 && t.0 != 0) {
                        best = *t;
                    }
                }
                out[(y * w + x) * 4..][..4].copy_from_slice(&best.0.to_be_bytes());
            }
        }
        out
    }
}

/// Sample points inside a pixel, `n` x `n` of them, nearest to the center first (scan order among equals).
fn sample_order(n: usize) -> Vec<P2> {
    let mut pts: Vec<P2> =
        (0..n * n).map(|k| (((k % n) as f64 + 0.5) / n as f64, ((k / n) as f64 + 0.5) / n as f64)).collect();
    pts.sort_by(|a, b| {
        let da = (a.0 - 0.5).powi(2) + (a.1 - 0.5).powi(2);
        let db = (b.0 - 0.5).powi(2) + (b.1 - 0.5).powi(2);
        da.total_cmp(&db)
    });
    pts
}

/// Where the handle grid goes by default: the bounds of the selection, or the whole canvas.
pub fn region(sel: Option<&Selection>, width: u32, height: u32) -> (i32, i32, u32, u32) {
    match sel.and_then(|s| s.bounds()) {
        Some((x, y, w, h)) => (x, y, w as u32, h as u32),
        None => (0, 0, width, height),
    }
}

/// Warps the cel at (`layer`, `frame`), limited to `mask`. Cels linked to it follow and stay linked;
/// indexed sprites keep their palette. Returns whether any pixel changed.
pub fn apply(sprite: &mut Sprite, warp: &Warp, layer: usize, frame: usize, mask: Option<&Selection>) -> bool {
    let mask = mask.filter(|m| (m.w, m.h) == (sprite.width, sprite.height));
    filters::apply_with(sprite, &|px, w, h| warp.run(px, w, h, mask), Target::Cel, layer, frame, None)
}

/// Warps the cel inside the editor's selection as one undo step; nothing is recorded when no pixel changes.
pub fn apply_editor(e: &mut Editor, warp: &Warp, layer: usize, frame: usize) -> bool {
    let mut next = e.sprite.clone();
    let (w, h) = (next.width, next.height);
    if !apply(&mut next, warp, layer, frame, e.sel.active(w, h)) {
        return false;
    }
    crate::document::tilemap::reconcile(&mut next);
    e.checkpoint();
    e.sprite = next;
    true
}

/// The flattened `frame` as it would look after the warp, leaving the editor untouched.
pub fn preview(e: &Editor, warp: &Warp, layer: usize, frame: usize) -> Vec<u8> {
    let mut s = e.sprite.clone();
    let (w, h) = (s.width, s.height);
    apply(&mut s, warp, layer, frame, e.sel.active(w, h));
    s.render(frame)
}

// MARK: C ABI

unsafe fn slice<'a, T>(p: *const T, n: usize) -> &'a [T] {
    if p.is_null() || n == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(p, n)
    }
}

/// The warp a call describes, with its grid over the selection bounds (or the canvas).
#[allow(clippy::too_many_arguments)]
unsafe fn warp_of(
    e: &Editor,
    (cols, rows): (u32, u32),
    offsets: *const f64,
    samples: u32,
    smooth: bool,
    pins: *const f64,
    pin_count: usize,
) -> Option<Warp> {
    if cols > MAX_GRID || rows > MAX_GRID || pin_count > MAX_PINS || (offsets.is_null() && cols * rows > 0) {
        return None;
    }
    let region = region(e.sel.active(e.sprite.width, e.sprite.height), e.sprite.width, e.sprite.height);
    Warp::from_flat(
        region,
        (cols, rows),
        slice(offsets, (cols * rows * 2) as usize),
        samples,
        smooth,
        slice(pins, pin_count * 5),
    )
}

/// Writes the handle grid's region, `x, y, width, height`: the selection bounds, or the canvas.
#[no_mangle]
pub unsafe extern "C" fn pixmith_warp_bounds(doc: *const Editor, out: *mut i32) -> bool {
    crate::ffi::guarded("pixmith_warp_bounds", std::ptr::null_mut(), || {
        let (Some(e), false) = (doc.as_ref(), out.is_null()) else { return false };
        let (x, y, w, h) = region(e.sel.active(e.sprite.width, e.sprite.height), e.sprite.width, e.sprite.height);
        std::slice::from_raw_parts_mut(out, 4).copy_from_slice(&[x, y, w as i32, h as i32]);
        true
    })
}

/// Warps the cel at (`layer`, `frame`) as one undo step, limited to the selection. `offsets` holds
/// `cols * rows` x, y displacement pairs (rows top to bottom), `pins` holds x, y, dx, dy, radius
/// for each of `pin_count` pins. Returns false (and records nothing) for bad arguments or if no pixel changed.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_warp_apply(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    cols: u32,
    rows: u32,
    offsets: *const f64,
    samples: u32,
    smooth: bool,
    pins: *const f64,
    pin_count: usize,
) -> bool {
    crate::ffi::guarded("pixmith_warp_apply", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return false };
        let Some(warp) = warp_of(e, (cols, rows), offsets, samples, smooth, pins, pin_count) else { return false };
        apply_editor(e, &warp, layer as usize, frame as usize)
    })
}

/// Renders `frame` as it would look after the warp, leaving the document and undo history untouched.
/// `out` needs width * height * 4 bytes.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_warp_preview(
    doc: *const Editor,
    layer: u32,
    frame: u32,
    cols: u32,
    rows: u32,
    offsets: *const f64,
    samples: u32,
    smooth: bool,
    pins: *const f64,
    pin_count: usize,
    out: *mut u8,
    len: usize,
) -> bool {
    crate::ffi::guarded("pixmith_warp_preview", std::ptr::null_mut(), || {
        let Some(e) = doc.as_ref() else { return false };
        let Some(warp) = warp_of(e, (cols, rows), offsets, samples, smooth, pins, pin_count) else { return false };
        let px = preview(e, &warp, layer as usize, frame as usize);
        if out.is_null() || len < px.len() {
            return false;
        }
        std::ptr::copy_nonoverlapping(px.as_ptr(), out, px.len());
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Cel;
    use std::collections::HashSet;
    use std::sync::Arc;

    const A: [u8; 4] = [255, 0, 0, 255];
    const B: [u8; 4] = [0, 255, 0, 255];
    const C: [u8; 4] = [0, 0, 255, 255];
    const CLEAR: [u8; 4] = [0; 4];

    fn sprite(w: u32, h: u32, px: &[[u8; 4]]) -> Sprite {
        let mut s = Sprite::new(w, h);
        s.cels[0][0] = Some(Arc::new(Cel { opacity: 255, pixels: px.concat(), ..Default::default() }));
        s
    }

    fn buf(px: &[[u8; 4]]) -> Vec<u8> {
        px.concat()
    }

    /// Deterministic pseudo-random numbers.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 33
        }

        fn unit(&mut self) -> f64 {
            (self.next() % 2001) as f64 / 1000.0 - 1.0
        }
    }

    /// A busy test picture: several colors, translucent pixels and transparent ones.
    fn picture(w: usize, h: usize) -> Vec<u8> {
        let mut r = Lcg(7);
        let palette = [A, B, C, [255, 255, 0, 255], [10, 20, 30, 128], CLEAR, CLEAR];
        (0..w * h).flat_map(|_| palette[(r.next() % palette.len() as u64) as usize]).collect()
    }

    fn colors(b: &[u8]) -> HashSet<[u8; 4]> {
        b.chunks(4).map(|c| if c[3] == 0 { CLEAR } else { c.try_into().unwrap() }).collect()
    }

    fn shifted(src: &[u8], w: usize, h: usize, dx: i64, dy: i64) -> Vec<u8> {
        let mut out = vec![0u8; src.len()];
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let (sx, sy) = (x - dx, y - dy);
                if sx >= 0 && sy >= 0 && sx < w as i64 && sy < h as i64 {
                    let (s, d) = ((sy as usize * w + sx as usize) * 4, (y as usize * w + x as usize) * 4);
                    out[d..d + 4].copy_from_slice(&src[s..s + 4]);
                }
            }
        }
        out
    }

    fn uniform(cols: u32, rows: u32, region: (i32, i32, u32, u32), d: P2) -> Warp {
        let mut w = Warp::new(cols, rows, region);
        w.offsets.fill(d);
        w
    }

    #[test]
    fn identity_warp_is_lossless() {
        let src = picture(13, 9);
        for (cols, rows) in [(2, 2), (4, 3), (8, 8)] {
            let w = Warp::new(cols, rows, (0, 0, 13, 9));
            assert!(w.is_identity());
            assert_eq!(w.run(&src, 13, 9, None), src);
        }
        // A warp that moves less than a billionth of a pixel is not recognised as the identity and runs
        // the full resampler, which must give every pixel back, translucent ones included.
        for smooth in [false, true] {
            for samples in [1, 3, 4, 8] {
                let mut w = uniform(3, 3, (0, 0, 13, 9), (1e-9, -1e-9));
                w.smooth = smooth;
                w.samples = samples;
                assert!(!w.is_identity());
                assert_eq!(w.run(&src, 13, 9, None), src, "smooth {smooth} samples {samples}");
            }
        }
        // A pin with no displacement moves nothing either.
        let mut w = Warp::new(2, 2, (0, 0, 13, 9));
        w.pins.push(Pin { x: 6.0, y: 4.0, dx: 0.0, dy: 0.0, radius: 5.0 });
        assert_eq!(w.run(&src, 13, 9, None), src);
    }

    #[test]
    fn uniform_shift_equals_translate() {
        let src = picture(16, 12);
        for (dx, dy) in [(1, 0), (0, -1), (3, 2), (-4, 5), (20, 0)] {
            for (cols, rows) in [(2, 2), (5, 3)] {
                for smooth in [false, true] {
                    for samples in [1, 2, 4, 5] {
                        let mut w = uniform(cols, rows, (0, 0, 16, 12), (dx as f64, dy as f64));
                        w.smooth = smooth;
                        w.samples = samples;
                        assert_eq!(
                            w.run(&src, 16, 12, None),
                            shifted(&src, 16, 12, dx, dy),
                            "shift {dx},{dy} grid {cols}x{rows} smooth {smooth} samples {samples}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_shift_rounds_by_majority() {
        // [A B C A B C ...] shifted by three quarters of a pixel: 3 of 4 samples fall in the pixel to the
        // left, so the result is the stripes moved by a whole pixel; a quarter pixel changes nothing.
        let row: Vec<u8> = (0..9).flat_map(|i| [A, B, C][i % 3]).collect();
        let by = |d: f64, samples: u32| {
            let mut w = uniform(2, 2, (0, 0, 9, 1), (d, 0.0));
            w.samples = samples;
            w.run(&row, 9, 1, None)
        };
        assert_eq!(by(0.75, 4), shifted(&row, 9, 1, 1, 0));
        assert_eq!(by(0.25, 4), row);
        assert_eq!(by(-0.75, 4), shifted(&row, 9, 1, -1, 0));
        // One sample is nearest-neighbor sampling at the pixel center.
        assert_eq!(by(0.75, 1), shifted(&row, 9, 1, 1, 0));
        assert_eq!(by(0.25, 1), row);
    }

    #[test]
    fn a_tie_goes_to_the_sample_nearest_the_center() {
        // Half a pixel to the right: the left half of each output pixel reads the pixel to its left, the
        // right half reads the pixel itself; with 2 samples across it is a tie, and the top-left sample
        // (first in scan order among the equally central ones) decides: the pixel to the left.
        let row: Vec<u8> = (0..4).flat_map(|i| [A, B][i % 2]).collect();
        let mut w = uniform(2, 2, (0, 0, 4, 1), (0.5, 0.0));
        w.samples = 2;
        let out = w.run(&row, 4, 1, None);
        assert_eq!(&out[4..], &shifted(&row, 4, 1, 1, 0)[4..]);
        // With 3 samples the middle one reads the pixel itself and wins two to one: nothing moves.
        w.samples = 3;
        assert_eq!(&w.run(&row, 4, 1, None)[4..], &row[4..]);
        // The same warp always gives the same pixels.
        assert_eq!(w.run(&row, 4, 1, None), w.run(&row, 4, 1, None));
    }

    #[test]
    fn majority_vote_never_introduces_a_color() {
        let (w, h) = (24usize, 20usize);
        let src = picture(w, h);
        let allowed = colors(&src);
        let mut r = Lcg(99);
        for round in 0..12 {
            let (cols, rows) = (2 + (round % 4) as u32, 2 + (round % 3) as u32);
            let mut warp = Warp::new(cols, rows, (0, 0, w as u32, h as u32));
            for o in warp.offsets.iter_mut() {
                *o = (r.unit() * 3.0, r.unit() * 3.0);
            }
            warp.smooth = round % 2 == 1;
            warp.samples = 1 + (round % 5) as u32;
            if round % 3 == 0 {
                warp.pins.push(Pin { x: 10.0, y: 8.0, dx: 2.5, dy: -1.5, radius: 9.0 });
            }
            let out = warp.run(&src, w, h, None);
            assert!(colors(&out).is_subset(&allowed), "round {round} made a new color");
            // Fully transparent pixels come out as plain (0, 0, 0, 0).
            assert!(out.chunks(4).all(|c| c[3] != 0 || c == [0; 4]));
        }
    }

    fn components_8(b: &[u8], w: usize, h: usize) -> usize {
        let mut seen = vec![false; w * h];
        let mut count = 0;
        for start in 0..w * h {
            if seen[start] || b[start * 4 + 3] == 0 {
                continue;
            }
            count += 1;
            let mut stack = vec![start];
            seen[start] = true;
            while let Some(i) = stack.pop() {
                let (x, y) = ((i % w) as i64, (i / w) as i64);
                for (nx, ny) in
                    (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dx, dy))).map(|(dx, dy)| (x + dx, y + dy))
                {
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if !seen[j] && b[j * 4 + 3] != 0 {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        count
    }

    /// A 1 px, 8-connected circle outline.
    fn ring(w: usize, h: usize, r: f64) -> Vec<u8> {
        let mut out = vec![0u8; w * h * 4];
        let (cx, cy) = (w as f64 / 2.0 - 0.5, h as f64 / 2.0 - 0.5);
        for k in 0..720 {
            let a = k as f64 * std::f64::consts::PI / 360.0;
            let (x, y) = ((cx + r * a.cos()).round() as usize, (cy + r * a.sin()).round() as usize);
            out[(y * w + x) * 4..][..4].copy_from_slice(&A);
        }
        out
    }

    #[test]
    fn a_one_pixel_outline_stays_eight_connected_under_a_mild_bend() {
        // A 1 px line is 8-connected only while every output pixel on it still sees at least half line.
        // That holds for a bend of up to about half a pixel on a straight line and a third of a pixel on
        // a ring, which has diagonal stretches; a 1 px diagonal has just half the area of a horizontal
        // line, so a stronger bend can cut it (majority voting never invents pixels to bridge the gap).
        let (w, h) = (40usize, 40usize);
        let region = (0, 0, w as u32, h as u32);
        let mut hline = vec![0u8; w * h * 4];
        let mut vline = vec![0u8; w * h * 4];
        for t in 4..36 {
            hline[(20 * w + t) * 4..][..4].copy_from_slice(&A);
            vline[(t * w + 20) * 4..][..4].copy_from_slice(&A);
        }
        let pictures = [("ring", ring(w, h, 13.0), 0.3), ("hline", hline, 0.5), ("vline", vline, 0.5)];
        let mut r = Lcg(5);
        for (name, src, amp) in &pictures {
            assert_eq!(components_8(src, w, h), 1, "{name} source");
            for round in 0..40 {
                let mut warp = Warp::new(3, 3, region);
                // The border handles stay put; the middle one, and in half the rounds the edge midpoints, bend.
                for j in 0..3usize {
                    for i in 0..3usize {
                        if i == 1 && j == 1 || (round % 2 == 0 && (i == 1 || j == 1)) {
                            warp.offsets[j * 3 + i] = (r.unit() * amp, r.unit() * amp);
                        }
                    }
                }
                warp.smooth = round % 3 == 0;
                warp.samples = [3, 4, 5][round % 3];
                let out = warp.run(src, w, h, None);
                assert!(out.chunks(4).any(|c| c[3] != 0), "{name} round {round} vanished");
                assert_eq!(components_8(&out, w, h), 1, "{name} round {round} broke apart");
            }
        }
    }

    #[test]
    fn a_visible_color_wins_a_tie_against_transparent() {
        // One line pixel straddling two output pixels (half a pixel down): both keep it, none lose it.
        let mut src = vec![0u8; 3 * 4 * 4];
        src[4 * 4..][..4].copy_from_slice(&A);
        let mut w = uniform(2, 2, (0, 0, 3, 4), (0.0, 0.5));
        w.samples = 2;
        let out = w.run(&src, 3, 4, None);
        assert_eq!(&out[4 * 4..][..4], &A);
        assert_eq!(&out[7 * 4..][..4], &A);
        assert_eq!(out.chunks(4).filter(|c| c[3] != 0).count(), 2);
    }

    #[test]
    fn a_checker_keeps_its_two_colors() {
        let (w, h) = (12usize, 12usize);
        let src: Vec<u8> = (0..w * h).flat_map(|i| if (i % w + i / w) % 2 == 0 { A } else { B }).collect();
        let mut r = Lcg(21);
        for round in 0..10 {
            // Border handles stay still and the inside bends: the sources stay on the canvas.
            let mut warp = Warp::new(4, 4, (0, 0, w as u32, h as u32));
            for j in 1..3usize {
                for i in 1..3usize {
                    warp.offsets[j * 4 + i] = (r.unit() * 1.5, r.unit() * 1.5);
                }
            }
            warp.smooth = round % 2 == 0;
            warp.samples = 1 + round as u32 % 6;
            let out = warp.run(&src, w, h, None);
            assert_eq!(colors(&out).difference(&HashSet::from([A, B])).count(), 0, "round {round}");
        }
        // The same on a 2x1 region, i.e. only two pixels wide.
        let two: Vec<u8> = buf(&[A, B]);
        let w2 = uniform(2, 2, (0, 0, 2, 1), (0.4, 0.0));
        let out = w2.run(&two, 2, 1, None);
        assert!(colors(&out).iter().all(|c| *c == A || *c == B || *c == CLEAR));
    }

    #[test]
    fn the_selection_limits_reading_and_writing() {
        // Red outside, green inside a 4x4 selection; green moves 2 px right.
        let (w, h) = (10usize, 8usize);
        let mut sel = Selection::new(w as u32, h as u32, false);
        for y in 2..6 {
            for x in 3..7 {
                sel.bits[y * w + x] = true;
            }
        }
        let src: Vec<u8> = (0..w * h).flat_map(|i| if sel.bits[i] { B } else { A }).collect();
        let region = region(Some(&sel), w as u32, h as u32);
        assert_eq!(region, (3, 2, 4, 4));
        let warp = uniform(2, 2, region, (2.0, 0.0));
        let out = warp.run(&src, w, h, Some(&sel));
        for i in 0..w * h {
            let (x, y) = (i % w, i / w);
            let px: [u8; 4] = out[i * 4..i * 4 + 4].try_into().unwrap();
            if !sel.bits[i] {
                assert_eq!(px, A, "({x},{y}) outside the selection changed");
            } else if x < 5 {
                // Nothing of the selection reaches the first two columns, and the red outside is not pulled in.
                assert_eq!(px, CLEAR, "({x},{y})");
            } else {
                assert_eq!(px, B, "({x},{y})");
            }
        }
        // The same through a sprite, with a mask of the wrong size ignored.
        let mut s = sprite(w as u32, h as u32, &src.chunks(4).map(|c| c.try_into().unwrap()).collect::<Vec<_>>());
        assert!(apply(&mut s, &warp, 0, 0, Some(&sel)));
        assert_eq!(s.cels[0][0].as_ref().unwrap().pixels, out);
        let wrong = Selection::new(3, 3, false);
        let mut s2 = sprite(w as u32, h as u32, &src.chunks(4).map(|c| c.try_into().unwrap()).collect::<Vec<_>>());
        assert!(apply(&mut s2, &warp, 0, 0, Some(&wrong)));
        assert_ne!(s2.cels[0][0].as_ref().unwrap().pixels, out);
    }

    #[test]
    fn output_is_deterministic() {
        let src = picture(20, 16);
        let mut r = Lcg(3);
        let mut warp = Warp::new(5, 4, (0, 0, 20, 16));
        for o in warp.offsets.iter_mut() {
            *o = (r.unit() * 2.0, r.unit() * 2.0);
        }
        warp.smooth = true;
        warp.pins.push(Pin { x: 5.0, y: 5.0, dx: -2.0, dy: 1.0, radius: 6.0 });
        let first = warp.run(&src, 20, 16, None);
        for _ in 0..3 {
            assert_eq!(warp.run(&src, 20, 16, None), first);
        }
        assert_eq!(warp.clone().run(&src, 20, 16, None), first);
    }

    #[test]
    fn handles_move_the_content_under_them() {
        // Smooth and bilinear fields both give a handle its displacement exactly.
        for smooth in [false, true] {
            let mut w = Warp::new(4, 3, (10, 20, 30, 20));
            w.smooth = smooth;
            for (k, o) in w.offsets.iter_mut().enumerate() {
                *o = (k as f64 * 0.5 - 2.0, 3.0 - k as f64);
            }
            for j in 0..3 {
                for i in 0..4 {
                    let p = w.handle_pos(i, j);
                    let d = w.displacement(p);
                    let want = w.offsets[(j * 4 + i) as usize];
                    assert!(
                        (d.0 - want.0).abs() < 1e-9 && (d.1 - want.1).abs() < 1e-9,
                        "smooth {smooth} handle {i},{j}"
                    );
                    let q = w.source_of((p.0 + want.0, p.1 + want.1));
                    assert!((q.0 - p.0).abs() < 1e-5 && (q.1 - p.1).abs() < 1e-5, "inverse at handle {i},{j}");
                }
            }
        }
        // A linear field is reproduced exactly by both interpolations.
        for smooth in [false, true] {
            let mut w = Warp::new(5, 5, (0, 0, 40, 40));
            w.smooth = smooth;
            for j in 0..5usize {
                for i in 0..5usize {
                    w.offsets[j * 5 + i] = (i as f64 * 0.25, j as f64 * -0.5);
                }
            }
            let d = w.displacement((13.0, 27.0));
            assert!((d.0 - 13.0 / 40.0 * 4.0 * 0.25).abs() < 1e-9 && (d.1 + 27.0 / 40.0 * 4.0 * 0.5).abs() < 1e-9);
        }
    }

    #[test]
    fn a_pin_drags_its_neighborhood_only() {
        let (w, h) = (30usize, 30usize);
        let mut src = vec![0u8; w * h * 4];
        // A 3x3 block at (14..17, 14..17) under the pin and a pixel far away.
        for y in 14..17 {
            for x in 14..17 {
                src[(y * w + x) * 4..][..4].copy_from_slice(&A);
            }
        }
        src[(2 * w + 27) * 4..][..4].copy_from_slice(&B);
        let mut warp = Warp::new(2, 2, (0, 0, w as u32, h as u32));
        warp.pins.push(Pin { x: 15.5, y: 15.5, dx: 4.0, dy: 0.0, radius: 6.0 });
        let out = warp.run(&src, w, h, None);
        let at = |x: usize, y: usize| -> [u8; 4] { out[(y * w + x) * 4..][..4].try_into().unwrap() };
        // The center of the block moved right by most of 4 px and the far pixel did not move.
        assert_eq!(at(19, 15), A);
        assert_eq!(at(14, 15), CLEAR);
        assert_eq!(at(27, 2), B);
        // The pin never reaches beyond its radius.
        let far = Warp {
            pins: vec![Pin { x: 0.0, y: 0.0, dx: 5.0, dy: 5.0, radius: 3.0 }],
            ..Warp::new(2, 2, (0, 0, 30, 30))
        };
        assert_eq!(far.displacement((20.0, 20.0)), (0.0, 0.0));
        assert_eq!(far.displacement((0.0, 0.0)), (5.0, 5.0));
    }

    #[test]
    fn bad_arguments_are_refused_or_clamped() {
        let region = (0, 0, 8, 8);
        assert!(Warp::from_flat(region, (1, 2), &[0.0; 4], 4, false, &[]).is_none());
        assert!(Warp::from_flat(region, (9, 2), &[0.0; 36], 4, false, &[]).is_none());
        assert!(Warp::from_flat(region, (2, 2), &[0.0; 7], 4, false, &[]).is_none());
        assert!(Warp::from_flat(region, (2, 2), &[0.0; 8], 4, false, &[0.0; 4]).is_none());
        let w =
            Warp::from_flat(region, (2, 2), &[f64::NAN, 1e30, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 99, true, &[]).unwrap();
        assert_eq!((w.samples, w.offsets[0].0, w.offsets[0].1), (MAX_SAMPLES, 0.0, MAX_OFFSET));
        // Extreme but finite warps still finish and keep the buffer's size.
        let out = w.run(&picture(8, 8), 8, 8, None);
        assert_eq!(out.len(), 8 * 8 * 4);
        // A region without area has a constant field.
        let flat = uniform(2, 2, (3, 3, 0, 0), (1.0, 2.0));
        assert_eq!(flat.displacement((0.0, 0.0)), (1.0, 2.0));
    }

    #[test]
    fn linked_cels_follow_and_stay_linked() {
        let mut s = Sprite::new(6, 1);
        let cel = Arc::new(Cel { opacity: 255, pixels: buf(&[A, B, C, CLEAR, CLEAR, CLEAR]), ..Default::default() });
        s.cels[0][0] = Some(cel.clone());
        s.add_frame(0, false);
        s.cels[1][0] = Some(cel);
        let warp = uniform(2, 2, (0, 0, 6, 1), (1.0, 0.0));
        assert!(apply(&mut s, &warp, 0, 0, None));
        let (c0, c1) = (s.cels[0][0].as_ref().unwrap(), s.cels[1][0].as_ref().unwrap());
        assert!(Arc::ptr_eq(c0, c1));
        assert_eq!(c0.pixels, buf(&[CLEAR, A, B, C, CLEAR, CLEAR]));
        // Nothing to do: no change reported.
        assert!(!apply(&mut s, &Warp::new(2, 2, (0, 0, 6, 1)), 0, 0, None));
    }

    #[test]
    fn indexed_sprites_keep_their_palette() {
        // The result only holds colors of the source, so converting back to the palette is a no-op.
        let src = picture(10, 10);
        let w = uniform(3, 3, (0, 0, 10, 10), (0.5, 0.3));
        assert!(colors(&w.run(&src, 10, 10, None)).is_subset(&colors(&src)));
    }

    #[test]
    fn one_undo_step_and_nothing_for_a_no_op() {
        let mut e = Editor::new(sprite(4, 1, &[A, B, C, CLEAR]));
        e.checkpoint();
        let before = e.sprite.cels[0][0].as_ref().unwrap().pixels.clone();
        assert!(!apply_editor(&mut e, &Warp::new(2, 2, (0, 0, 4, 1)), 0, 0));
        let warp = uniform(2, 2, (0, 0, 4, 1), (1.0, 0.0));
        assert!(apply_editor(&mut e, &warp, 0, 0));
        assert_eq!(e.sprite.cels[0][0].as_ref().unwrap().pixels, buf(&[CLEAR, A, B, C]));
        assert!(e.undo());
        assert_eq!(e.sprite.cels[0][0].as_ref().unwrap().pixels, before);
        assert!(e.redo());
        assert_eq!(e.sprite.cels[0][0].as_ref().unwrap().pixels, buf(&[CLEAR, A, B, C]));
    }

    #[test]
    fn the_c_abi_applies_previews_and_validates() {
        let mut e = Box::new(Editor::new(sprite(4, 2, &[A, B, C, CLEAR, A, B, C, CLEAR])));
        e.checkpoint();
        let doc: *mut Editor = &mut *e;
        let offsets = [1.0f64, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        unsafe {
            let mut bounds = [0i32; 4];
            assert!(pixmith_warp_bounds(doc, bounds.as_mut_ptr()));
            assert_eq!(bounds, [0, 0, 4, 2]);
            assert!(!pixmith_warp_bounds(doc, std::ptr::null_mut()));

            let mut out = vec![0u8; 4 * 2 * 4];
            assert!(pixmith_warp_preview(
                doc,
                0,
                0,
                2,
                2,
                offsets.as_ptr(),
                4,
                false,
                std::ptr::null(),
                0,
                out.as_mut_ptr(),
                out.len()
            ));
            assert_eq!(out, buf(&[CLEAR, A, B, C, CLEAR, A, B, C]));
            // Previewing changes nothing.
            assert_eq!(e.sprite.cels[0][0].as_ref().unwrap().pixels, buf(&[A, B, C, CLEAR, A, B, C, CLEAR]));
            // Too small an output buffer, a bad grid, missing offsets: all refused.
            assert!(!pixmith_warp_preview(
                doc,
                0,
                0,
                2,
                2,
                offsets.as_ptr(),
                4,
                false,
                std::ptr::null(),
                0,
                out.as_mut_ptr(),
                3
            ));
            assert!(!pixmith_warp_apply(doc, 0, 0, 1, 2, offsets.as_ptr(), 4, false, std::ptr::null(), 0));
            assert!(!pixmith_warp_apply(doc, 0, 0, 2, 2, std::ptr::null(), 4, false, std::ptr::null(), 0));
            assert!(!pixmith_warp_apply(
                std::ptr::null_mut(),
                0,
                0,
                2,
                2,
                offsets.as_ptr(),
                4,
                false,
                std::ptr::null(),
                0
            ));
            assert!(!pixmith_warp_apply(doc, 5, 0, 2, 2, offsets.as_ptr(), 4, false, std::ptr::null(), 0));

            assert!(pixmith_warp_apply(doc, 0, 0, 2, 2, offsets.as_ptr(), 4, false, std::ptr::null(), 0));
            assert_eq!(e.sprite.cels[0][0].as_ref().unwrap().pixels, buf(&[CLEAR, A, B, C, CLEAR, A, B, C]));
            // A pin through the C ABI.
            let pin = [1.5f64, 0.5, 1.0, 0.0, 3.0];
            let zero = [0.0f64; 8];
            assert!(pixmith_warp_apply(doc, 0, 0, 2, 2, zero.as_ptr(), 4, true, pin.as_ptr(), 1));
        }
        assert!(e.undo() && e.undo());
        assert_eq!(e.sprite.cels[0][0].as_ref().unwrap().pixels, buf(&[A, B, C, CLEAR, A, B, C, CLEAR]));
    }
}
