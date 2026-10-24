//! Blur and Jumble: brush tools that rework the pixels under the brush instead of painting a color.
//!
//! * **Blur** replaces each pixel under the brush by the average of its 3x3 neighbourhood (alpha-weighted, so a
//!   transparent neighbour does not darken the colors), mixed with the old pixel by `strength` percent. Every
//!   dab reads the pixels as they were before that dab, so the result does not depend on scan order.
//! * **Jumble** swaps each pixel under the brush with a random pixel up to `1 + size / 2` away (each pixel with
//!   probability `strength` percent). Equal seeds give equal results.
//!
//! The brush is the global brush of `paint::ink` (square, round or a custom brush image), `size` is clamped to
//! 1..=64, and mirroring uses the same flags as the other strokes (bit 0 left/right, bit 1 top/bottom). Ink
//! modes and colors play no part. Like the pencil, a stroke interpolates a line of dabs between mouse events,
//! never leaves the canvas, is clipped to the selection, and is one undo step when the caller opened one with
//! `pixmith_begin_stroke`. C ABI: `pixmith_retouch_begin`, `pixmith_retouch_to`, `pixmith_retouch_end_doc`.

use crate::document::{Editor, Sprite};
use crate::paint::ink;
use crate::paint::shapes::{line_points, Pt, SYM_X, SYM_Y};
use std::cell::RefCell;
use std::collections::HashMap;

pub const KIND_BLUR: u32 = 0;
pub const KIND_JUMBLE: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Blur,
    Jumble,
}

impl Kind {
    pub fn from_u32(k: u32) -> Kind {
        if k == KIND_JUMBLE {
            Kind::Jumble
        } else {
            Kind::Blur
        }
    }
}

type Rect = (i32, i32, i32, i32);

/// An in-progress Blur or Jumble stroke.
pub struct Retouch {
    layer: usize,
    frame: usize,
    kind: Kind,
    size: i32,
    strength: u32,
    sym: u32,
    rng: u32,
    last: Pt,
    /// Changed area since the last `take_dirty`, end exclusive.
    dirty: Option<Rect>,
}

fn xorshift(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

fn union(a: Option<Rect>, b: Rect) -> Option<Rect> {
    Some(a.map_or(b, |a| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))))
}

/// Pixels under the brush centered on `p`, unclipped (the same shape the pencil stamps).
fn brush_cells(size: i32, p: Pt) -> Vec<Pt> {
    let settings = ink::settings();
    if let Some(b) = &settings.custom {
        let (ox, oy) = (p.0 - b.w / 2, p.1 - b.h / 2);
        return b.cache().pixels.iter().map(|&(x, y, _)| (ox + x, oy + y)).collect();
    }
    let half = (size - 1) / 2;
    let mut out = Vec::with_capacity((size * size) as usize);
    for row in 0..size {
        // Same rounding as `Painter::dot`: a round brush trims `l` pixels from both ends of each row.
        let l = if settings.circle && size > 2 {
            let r = size as f64 / 2.0;
            let dy = row as f64 + 0.5 - r;
            ((r - (r * r - dy * dy).max(0.0).sqrt() - 0.5 - 1e-9).ceil() as i32).clamp(0, size - 1)
        } else {
            0
        };
        for col in l..size - l {
            out.push((p.0 - half + col, p.1 - half + row));
        }
    }
    out
}

impl Retouch {
    /// Starts a stroke with its first dab at `p`.
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        s: &mut Sprite,
        layer: usize,
        frame: usize,
        p: Pt,
        kind: Kind,
        size: u32,
        strength: u32,
        seed: u32,
        flags: u32,
    ) -> Retouch {
        let mut r = Retouch {
            layer,
            frame,
            kind,
            size: size.clamp(1, 64) as i32,
            strength: strength.clamp(1, 100),
            sym: flags & (SYM_X | SYM_Y),
            rng: seed ^ 0x9E37_79B9 | 1,
            last: p,
            dirty: None,
        };
        r.dab(s, p);
        r
    }

    /// Changes the brush size for the rest of the stroke.
    pub fn set_size(&mut self, size: u32) {
        self.size = size.clamp(1, 64) as i32;
    }

    /// The area changed since the previous call, for incremental re-rendering.
    pub fn take_dirty(&mut self) -> Option<Rect> {
        self.dirty.take()
    }

    /// Dabs along a straight line from the previous point to `p`.
    pub fn to(&mut self, s: &mut Sprite, p: Pt) {
        let from = self.last;
        self.last = p;
        for q in line_points(from, p).into_iter().skip(1) {
            self.dab(s, q);
        }
    }

    /// The brush pixels at `p` plus their mirror images, inside the canvas, in scan order without duplicates.
    fn cells(&self, dims: (i32, i32), p: Pt) -> Vec<usize> {
        let (w, h) = dims;
        let mut out = Vec::new();
        for (x, y) in brush_cells(self.size, p) {
            let mut add = |x: i32, y: i32| {
                if x >= 0 && y >= 0 && x < w && y < h {
                    out.push((y * w + x) as usize);
                }
            };
            add(x, y);
            if self.sym & SYM_X != 0 {
                add(w - 1 - x, y);
            }
            if self.sym & SYM_Y != 0 {
                add(x, h - 1 - y);
            }
            if self.sym == SYM_X | SYM_Y {
                add(w - 1 - x, h - 1 - y);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    fn dab(&mut self, s: &mut Sprite, p: Pt) {
        let dims = s.dims();
        let cells = self.cells(dims, p);
        if cells.is_empty() {
            return;
        }
        let w = dims.0 as usize;
        let reach = if self.kind == Kind::Jumble { 1 + self.size / 2 } else { 1 };
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &i in &cells {
            let (x, y) = ((i % w) as i32, (i / w) as i32);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let Some(cel) = s.cel_mut(self.layer, self.frame) else { return };
        match self.kind {
            Kind::Blur => blur(&mut cel.pixels, dims, &cells, self.strength),
            Kind::Jumble => jumble(&mut cel.pixels, dims, &cells, reach, self.strength, &mut self.rng),
        }
        let grown =
            ((x0 - reach).max(0), (y0 - reach).max(0), (x1 + reach + 1).min(dims.0), (y1 + reach + 1).min(dims.1));
        self.dirty = union(self.dirty, grown);
    }
}

/// Averages each cell with its 3x3 neighbourhood, reading from a copy taken before the dab.
fn blur(px: &mut [u8], dims: (i32, i32), cells: &[usize], strength: u32) {
    let (w, h) = (dims.0 as usize, dims.1 as usize);
    // Snapshot of the rows the dab can read: its bounding box grown by one pixel.
    let (mut y0, mut y1) = (usize::MAX, 0);
    for &i in cells {
        y0 = y0.min(i / w);
        y1 = y1.max(i / w);
    }
    let (y0, y1) = (y0.saturating_sub(1), (y1 + 1).min(h - 1));
    let base = y0 * w * 4;
    let snap = px[base..(y1 + 1) * w * 4].to_vec();
    let at = |x: usize, y: usize| -> [u32; 4] {
        let i = ((y - y0) * w + x) * 4;
        [snap[i] as u32, snap[i + 1] as u32, snap[i + 2] as u32, snap[i + 3] as u32]
    };
    let s = strength as u64;
    for &i in cells {
        let (x, y) = (i % w, i / w);
        let (mut n, mut sa, mut pr, mut pg, mut pb) = (0u64, 0u64, 0u64, 0u64, 0u64);
        for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
            for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                let [r, g, b, a] = at(nx, ny);
                n += 1;
                sa += a as u64;
                pr += (r * a) as u64;
                pg += (g * a) as u64;
                pb += (b * a) as u64;
            }
        }
        let [or, og, ob, oa] = at(x, y);
        let mix = |old: u64, sum: u64| (old * (100 - s) * n + sum * s + 50 * n) / (100 * n);
        let a = mix(oa as u64, sa);
        let out = &mut px[i * 4..i * 4 + 4];
        if a == 0 {
            out.fill(0);
            continue;
        }
        let chan = |old: u32, sum: u64| {
            let p = mix(old as u64 * oa as u64, sum);
            ((p + a / 2) / a).min(255) as u8
        };
        out.copy_from_slice(&[chan(or, pr), chan(og, pg), chan(ob, pb), a as u8]);
    }
}

/// Swaps each cell with a random pixel within `reach`, in scan order.
fn jumble(px: &mut [u8], dims: (i32, i32), cells: &[usize], reach: i32, strength: u32, rng: &mut u32) {
    let w = dims.0 as usize;
    let span = 2 * reach as u32 + 1;
    for &i in cells {
        // Always draw the same three numbers so a stroke's randomness does not depend on the canvas edge.
        let (dx, dy, roll) = (xorshift(rng), xorshift(rng), xorshift(rng));
        if roll % 100 >= strength {
            continue;
        }
        let (x, y) = ((i % w) as i32 + (dx % span) as i32 - reach, (i / w) as i32 + (dy % span) as i32 - reach);
        if x < 0 || y < 0 || x >= dims.0 || y >= dims.1 {
            continue;
        }
        let j = y as usize * w + x as usize;
        if j != i {
            for c in 0..4 {
                px.swap(i * 4 + c, j * 4 + c);
            }
        }
    }
}

thread_local! {
    /// Running strokes by document pointer, so documents used alternately never share one.
    static RETOUCH: RefCell<HashMap<usize, Retouch>> = RefCell::new(HashMap::new());
}

/// Drops the running stroke of a document that is being freed or whose strokes all end.
pub(crate) fn forget(doc: *mut Editor) {
    RETOUCH.with(|s| {
        s.borrow_mut().remove(&(doc as usize));
    });
}

pub(crate) fn forget_all() {
    RETOUCH.with(|s| s.borrow_mut().clear());
}

/// Starts a Blur (`kind` 0) or Jumble (1) stroke at `(x, y)`; call `pixmith_begin_stroke` first for undo.
/// `strength` is 1..=100, `seed` drives Jumble's randomness, `flags` are the mirror bits.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_retouch_begin(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    kind: u32,
    size: u32,
    strength: u32,
    seed: u32,
    flags: u32,
) {
    crate::ffi::guarded("pixmith_retouch_begin", doc, || {
        let Some(e) = crate::ffi::editor_quiet(doc) else { return };
        let (x, y) = (crate::ffi::clampc(x), crate::ffi::clampc(y));
        let (l, f) = (layer as usize, frame as usize);
        let shared = e.cel_shared(l, f);
        let mut st = Retouch::begin(&mut e.sprite, l, f, (x, y), Kind::from_u32(kind), size, strength, seed, flags);
        crate::paint::selection::clip_to_selection(e, l, f);
        match st.take_dirty() {
            Some(r) => e.mark_cel_rect(l, f, r, shared),
            None => e.dirty.mark_all(),
        }
        RETOUCH.with(|s| {
            s.borrow_mut().insert(doc as usize, st);
        });
    })
}

/// Continues the running Blur or Jumble stroke of `doc` with a line of dabs to `(x, y)`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_retouch_to(doc: *mut Editor, x: i32, y: i32) {
    crate::ffi::guarded("pixmith_retouch_to", doc, || {
        let Some(e) = crate::ffi::editor_quiet(doc) else { return };
        RETOUCH.with(|s| {
            if let Some(st) = s.borrow_mut().get_mut(&(doc as usize)) {
                let shared = e.cel_shared(st.layer, st.frame);
                st.to(&mut e.sprite, (crate::ffi::clampc(x), crate::ffi::clampc(y)));
                crate::paint::selection::clip_to_selection(e, st.layer, st.frame);
                if let Some(r) = st.take_dirty() {
                    e.mark_cel_rect(st.layer, st.frame, r, shared);
                }
            }
        });
    })
}

/// Changes the brush size of the running stroke of `doc` (pen pressure).
#[no_mangle]
pub unsafe extern "C" fn pixmith_retouch_size(doc: *mut Editor, size: u32) {
    crate::ffi::guarded("pixmith_retouch_size", doc, || {
        RETOUCH.with(|s| {
            if let Some(st) = s.borrow_mut().get_mut(&(doc as usize)) {
                st.set_size(size);
            }
        });
    })
}

/// Ends only the running Blur or Jumble stroke of `doc`.
#[no_mangle]
pub extern "C" fn pixmith_retouch_end_doc(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_retouch_end_doc", doc, || forget(doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: [u8; 4] = [255, 0, 0, 255];
    const B: [u8; 4] = [0, 0, 255, 255];

    /// A sprite whose single cel is a red/blue checkerboard.
    fn checker(w: u32, h: u32) -> Sprite {
        let mut s = Sprite::new(w, h);
        let cel = s.cel_mut(0, 0).expect("cel");
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                cel.pixels[i..i + 4].copy_from_slice(if (x + y) % 2 == 0 { &R } else { &B });
            }
        }
        s
    }

    fn pixels(s: &mut Sprite) -> Vec<u8> {
        s.cel_mut(0, 0).expect("cel").pixels.clone()
    }

    fn stroke(s: &mut Sprite, kind: Kind, size: u32, strength: u32, seed: u32, flags: u32, path: &[Pt]) {
        let mut r = Retouch::begin(s, 0, 0, path[0], kind, size, strength, seed, flags);
        for &p in &path[1..] {
            r.to(s, p);
        }
    }

    #[test]
    fn blur_averages_the_neighbourhood() {
        let mut s = checker(8, 8);
        stroke(&mut s, Kind::Blur, 1, 100, 0, 0, &[(4, 4)]);
        let px = pixels(&mut s);
        let i = (4 * 8 + 4) * 4;
        // (4,4) is red; its 3x3 neighbourhood has five red and four blue pixels.
        assert_eq!(&px[i..i + 4], &[142, 0, 113, 255]);
        // Neighbours were not touched.
        assert_eq!(&px[i + 4..i + 8], &B);
    }

    #[test]
    fn blur_strength_mixes_with_the_old_pixel() {
        let mut full = checker(8, 8);
        let mut half = checker(8, 8);
        stroke(&mut full, Kind::Blur, 1, 100, 0, 0, &[(4, 4)]);
        stroke(&mut half, Kind::Blur, 1, 50, 0, 0, &[(4, 4)]);
        let i = (4 * 8 + 4) * 4;
        let (a, b) = (pixels(&mut full)[i], pixels(&mut half)[i]);
        assert!(b > a && b < 255, "half strength sits between the old red ({b}) and the average ({a})");
    }

    #[test]
    fn blur_keeps_flat_areas_and_does_not_darken_with_transparency() {
        let mut s = Sprite::new(6, 6);
        let cel = s.cel_mut(0, 0).unwrap();
        for y in 0..6usize {
            for x in 0..3usize {
                cel.pixels[(y * 6 + x) * 4..(y * 6 + x) * 4 + 4].copy_from_slice(&R);
            }
        }
        stroke(&mut s, Kind::Blur, 6, 100, 0, 0, &[(2, 2), (3, 3)]);
        let px = pixels(&mut s);
        for y in 0..6usize {
            for x in 0..6usize {
                let p = &px[(y * 6 + x) * 4..(y * 6 + x) * 4 + 4];
                if p[3] != 0 {
                    assert_eq!(&p[..3], &[255, 0, 0], "alpha-weighted blur keeps the red pure at ({x},{y})");
                }
            }
        }
        let edge = (2 * 6 + 3) * 4;
        assert!(px[edge + 3] > 0 && px[edge + 3] < 255, "the transparent side bleeds in partially");
    }

    #[test]
    fn blur_mirrors_symmetrically() {
        let mut s = Sprite::new(9, 9);
        let cel = s.cel_mut(0, 0).unwrap();
        for y in 0..9usize {
            for x in 0..9usize {
                // Symmetric input: a bright diagonal cross.
                if x == y || x + y == 8 {
                    cel.pixels[(y * 9 + x) * 4..(y * 9 + x) * 4 + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
        stroke(&mut s, Kind::Blur, 3, 100, 0, SYM_X | SYM_Y, &[(1, 1), (3, 2)]);
        let px = pixels(&mut s);
        let at = |x: usize, y: usize| px[(y * 9 + x) * 4..(y * 9 + x) * 4 + 4].to_vec();
        for y in 0..9usize {
            for x in 0..9usize {
                assert_eq!(at(x, y), at(8 - x, y), "left/right at ({x},{y})");
                assert_eq!(at(x, y), at(x, 8 - y), "top/bottom at ({x},{y})");
            }
        }
        assert_ne!(px, pixels(&mut checker(9, 9)), "something changed");
    }

    #[test]
    fn jumble_is_deterministic_per_seed_and_keeps_every_pixel() {
        let run = |seed| {
            let mut s = checker(16, 16);
            // Make colors unique so a swap is detectable.
            let cel = s.cel_mut(0, 0).unwrap();
            for (i, p) in cel.pixels.chunks_exact_mut(4).enumerate() {
                p.copy_from_slice(&[(i % 256) as u8, (i / 256) as u8, 7, 255]);
            }
            stroke(&mut s, Kind::Jumble, 5, 100, seed, 0, &[(3, 3), (10, 9), (4, 12)]);
            pixels(&mut s)
        };
        let (a, b, c) = (run(1), run(1), run(2));
        assert_eq!(a, b, "equal seeds, equal result");
        assert_ne!(a, c, "different seeds differ");
        let sorted = |v: &[u8]| {
            let mut px: Vec<[u8; 4]> = v.chunks_exact(4).map(|p| p.try_into().unwrap()).collect();
            px.sort();
            px
        };
        let mut orig = checker(16, 16);
        let cel = orig.cel_mut(0, 0).unwrap();
        for (i, p) in cel.pixels.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(&[(i % 256) as u8, (i / 256) as u8, 7, 255]);
        }
        assert_eq!(sorted(&a), sorted(&pixels(&mut orig)), "swaps only permute the pixels");
        assert_ne!(a, pixels(&mut orig));
    }

    #[test]
    fn jumble_only_touches_pixels_near_the_brush() {
        let mut s = checker(32, 32);
        let before = pixels(&mut s);
        stroke(&mut s, Kind::Jumble, 3, 100, 9, 0, &[(16, 16)]);
        let after = pixels(&mut s);
        let reach = 1 + 3 / 2 + 1; // brush half-width plus swap reach
        for y in 0..32i32 {
            for x in 0..32i32 {
                if (x - 16).abs() > reach + 1 || (y - 16).abs() > reach + 1 {
                    let i = ((y * 32 + x) * 4) as usize;
                    assert_eq!(before[i..i + 4], after[i..i + 4], "({x},{y}) is outside the reach");
                }
            }
        }
    }

    #[test]
    fn strokes_near_and_beyond_the_edges_stay_in_bounds() {
        for kind in [Kind::Blur, Kind::Jumble] {
            let mut s = checker(7, 5);
            stroke(&mut s, kind, 64, 100, 3, SYM_X | SYM_Y, &[(-50, -50), (3, 2), (900, 900), (-5, 4), (0, 0)]);
            assert_eq!(pixels(&mut s).len(), 7 * 5 * 4);
        }
    }

    #[test]
    fn dirty_rect_covers_all_changes() {
        let mut s = checker(24, 24);
        let before = pixels(&mut s);
        let mut r = Retouch::begin(&mut s, 0, 0, (12, 12), Kind::Jumble, 4, 100, 5, 0);
        r.to(&mut s, (16, 14));
        let (x0, y0, x1, y1) = r.take_dirty().expect("dirty");
        let after = pixels(&mut s);
        for y in 0..24i32 {
            for x in 0..24i32 {
                let i = ((y * 24 + x) * 4) as usize;
                if before[i..i + 4] != after[i..i + 4] {
                    assert!(x >= x0 && x < x1 && y >= y0 && y < y1, "({x},{y}) changed outside {x0},{y0},{x1},{y1}");
                }
            }
        }
    }

    #[test]
    fn round_brush_blurs_fewer_pixels_than_square() {
        let count = |circle| {
            ink::update_settings(|s| s.circle = circle);
            let mut s = checker(20, 20);
            let before = pixels(&mut s);
            stroke(&mut s, Kind::Blur, 9, 100, 0, 0, &[(10, 10)]);
            ink::update_settings(|s| s.circle = false);
            before.chunks_exact(4).zip(pixels(&mut s).chunks_exact(4)).filter(|(a, b)| a != b).count()
        };
        assert!(count(true) < count(false));
    }

    #[test]
    fn ffi_stroke_is_one_undo_step_and_respects_the_selection() {
        use crate::ffi::document::pixmith_begin_stroke;
        let doc = Box::into_raw(Box::new(Editor::new(checker(16, 16))));
        unsafe {
            let before = (*doc).sprite.cel_mut(0, 0).unwrap().pixels.clone();
            crate::paint::selection::pixmith_sel_rect(doc, 0, 0, 7, 15);
            pixmith_begin_stroke(doc);
            pixmith_retouch_begin(doc, 0, 0, 2, 8, KIND_BLUR, 5, 100, 1, 0);
            for x in 3..14 {
                pixmith_retouch_to(doc, x, 8);
            }
            pixmith_retouch_end_doc(doc);
            let after = (*doc).sprite.cel_mut(0, 0).unwrap().pixels.clone();
            assert_ne!(before, after, "the blur changed something");
            for y in 0..16usize {
                for x in 8..16usize {
                    let i = (y * 16 + x) * 4;
                    assert_eq!(before[i..i + 4], after[i..i + 4], "({x},{y}) is outside the selection");
                }
            }
            // A second stroke without a running session does nothing.
            pixmith_retouch_to(doc, 4, 4);
            assert_eq!((*doc).sprite.cel_mut(0, 0).unwrap().pixels, after);
            assert!(crate::ffi::document::pixmith_undo(doc));
            assert_eq!((*doc).sprite.cel_mut(0, 0).unwrap().pixels, before, "one undo reverts the whole stroke");
            drop(Box::from_raw(doc));
        }
    }
}
