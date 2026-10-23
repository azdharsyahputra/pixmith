//! Ink modes, custom brushes, gradients and dither-pattern fills, plus their C ABI.
//!
//! Ink and brush settings are per-thread state set by the UI before drawing; the painters in
//! `paint::shapes` read them when they start.

use crate::document::{blend_over, Editor, Sprite};
use std::cell::RefCell;
use std::rc::Rc;

pub type Color = [u8; 4];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Ink {
    /// Replace the pixel.
    #[default]
    Simple,
    /// Blend the color over the pixel.
    Alpha,
    /// Recolor only pixels that are not fully transparent, keeping their alpha.
    LockAlpha,
    /// Paint under the existing pixels.
    Behind,
    /// Move the pixel one step along the color ramp.
    Shading,
}

impl Ink {
    pub fn from_u32(v: u32) -> Ink {
        match v {
            1 => Ink::Alpha,
            2 => Ink::LockAlpha,
            3 => Ink::Behind,
            4 => Ink::Shading,
            _ => Ink::Simple,
        }
    }
}

/// A brush image stamped around the cursor; only pixels with alpha are painted.
#[derive(Debug, PartialEq)]
pub struct Brush {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u8>,
    cache: std::cell::OnceCell<BrushCache>,
}

/// Derived data that makes long strokes cheap: the visible pixels as a list, and for a brush of a
/// single color the pixels each one-pixel step newly covers.
#[derive(Debug, PartialEq, Default)]
pub struct BrushCache {
    /// Visible pixels as (x, y, color), brush-relative.
    pub pixels: Vec<(i32, i32, Color)>,
    /// The color shared by all visible pixels, if there is only one.
    pub uniform: Option<Color>,
    /// Per step `(dx, dy)` in -1..=1 (index `(dy + 1) * 3 + dx + 1`): indices into `pixels` of the
    /// pixels that a stamp moved by that step covers but the previous stamp did not.
    pub edges: [Vec<u32>; 9],
}

impl BrushCache {
    pub fn step_index(dx: i32, dy: i32) -> usize {
        ((dy + 1) * 3 + dx + 1) as usize
    }
}

impl Brush {
    pub fn new(w: i32, h: i32, px: Vec<u8>) -> Brush {
        Brush { w, h, px, cache: Default::default() }
    }

    pub fn cache(&self) -> &BrushCache {
        self.cache.get_or_init(|| {
            let mut c = BrushCache::default();
            let mut solid = vec![false; (self.w * self.h).max(0) as usize];
            for y in 0..self.h {
                for x in 0..self.w {
                    let i = (y * self.w + x) as usize;
                    let p: Color = self.px[i * 4..i * 4 + 4].try_into().unwrap();
                    if p[3] != 0 {
                        solid[i] = true;
                        c.pixels.push((x, y, p));
                    }
                }
            }
            c.uniform = c.pixels.first().map(|p| p.2).filter(|u| c.pixels.iter().all(|p| p.2 == *u));
            let covered =
                |x: i32, y: i32| x >= 0 && y >= 0 && x < self.w && y < self.h && solid[(y * self.w + x) as usize];
            for dy in -1..=1 {
                for dx in -1..=1 {
                    c.edges[BrushCache::step_index(dx, dy)] = c
                        .pixels
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| !covered(p.0 + dx, p.1 + dy))
                        .map(|(i, _)| i as u32)
                        .collect();
                }
            }
            c
        })
    }

    /// Crops transparent borders. `None` if the image has no visible pixel.
    pub fn from_rgba(w: i32, h: i32, px: &[u8]) -> Option<Brush> {
        if w <= 0 || h <= 0 || px.len() != w as usize * h as usize * 4 {
            return None;
        }
        let at = |x: i32, y: i32| px[(y * w + x) as usize * 4 + 3];
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, -1, -1);
        for y in 0..h {
            for x in 0..w {
                if at(x, y) != 0 {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
        }
        if x1 < 0 {
            return None;
        }
        let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
        let mut out = Vec::with_capacity(bw as usize * bh as usize * 4);
        for y in y0..=y1 {
            let i = (y * w + x0) as usize * 4;
            out.extend_from_slice(&px[i..i + bw as usize * 4]);
        }
        Some(Brush::new(bw, bh, out))
    }
}

#[derive(Clone, Default)]
pub struct Settings {
    pub ink: Ink,
    /// Shading goes one step toward the start of the ramp instead of the end.
    pub reverse: bool,
    pub ramp: Vec<Color>,
    /// Round brush for sizes above one.
    pub circle: bool,
    pub custom: Option<Rc<Brush>>,
}

thread_local! {
    static SETTINGS: RefCell<Rc<Settings>> = RefCell::new(Rc::default());
}

pub fn settings() -> Rc<Settings> {
    SETTINGS.with(|s| s.borrow().clone())
}

pub fn update_settings(f: impl FnOnce(&mut Settings)) {
    SETTINGS.with(|s| f(Rc::make_mut(&mut s.borrow_mut())));
}

/// Applies `src` to `dst` according to the ink. A transparent `src` always erases.
pub fn paint(s: &Settings, dst: &mut [u8], src: Color) {
    if src[3] == 0 {
        dst.fill(0);
        return;
    }
    match s.ink {
        Ink::Simple => dst.copy_from_slice(&src),
        Ink::Alpha => blend_over(dst, &src, 255),
        Ink::LockAlpha => {
            if dst[3] != 0 {
                dst[..3].copy_from_slice(&src[..3]);
            }
        }
        Ink::Behind => {
            let mut out = src;
            blend_over(&mut out, dst, 255);
            dst.copy_from_slice(&out);
        }
        Ink::Shading => {
            if dst[3] == 0 || s.ramp.is_empty() {
                return;
            }
            if let Some(i) = s.ramp.iter().position(|c| c[..3] == dst[..3]) {
                let j = if s.reverse { i.saturating_sub(1) } else { (i + 1).min(s.ramp.len() - 1) };
                dst[..3].copy_from_slice(&s.ramp[j][..3]);
            }
        }
    }
}

/// Ordered-dither matrix value in `0..n*n` for a power-of-two size.
pub fn bayer(n: u32, x: i32, y: i32) -> u32 {
    const M2: [[u32; 2]; 2] = [[0, 2], [3, 1]];
    let (x, y) = (x.rem_euclid(n as i32) as u32, y.rem_euclid(n as i32) as u32);
    if n <= 2 {
        return M2[y as usize][x as usize];
    }
    let h = n / 2;
    4 * bayer(h, (x % h) as i32, (y % h) as i32) + M2[(y / h) as usize][(x / h) as usize]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GradientKind {
    Linear,
    Radial,
}

fn lerp(a: Color, b: Color, t: f32) -> Color {
    let (aa, ba) = (a[3] as f32 * (1.0 - t), b[3] as f32 * t);
    let alpha = aa + ba;
    if alpha <= 0.0 {
        return [0; 4];
    }
    let mut out = [0, 0, 0, alpha.round() as u8];
    for c in 0..3 {
        out[c] = ((a[c] as f32 * aa + b[c] as f32 * ba) / alpha).round() as u8;
    }
    out
}

impl Sprite {
    /// Fills the whole cel with a gradient from `c0` at `a` to `c1` at `b`. `dither` is 0 for a
    /// smooth blend or a Bayer matrix size (2, 4, 8) for a two-color ordered dither.
    #[allow(clippy::too_many_arguments)]
    pub fn gradient(
        &mut self,
        layer: usize,
        frame: usize,
        a: (i32, i32),
        b: (i32, i32),
        c0: Color,
        c1: Color,
        kind: GradientKind,
        dither: u32,
    ) {
        let w = self.width as i32;
        let settings = settings();
        let Some(cel) = self.cel_mut(layer, frame) else { return };
        let (ax, ay) = (a.0 as f32 + 0.5, a.1 as f32 + 0.5);
        let (dx, dy) = (b.0 as f32 - a.0 as f32, b.1 as f32 - a.1 as f32);
        let len2 = dx * dx + dy * dy;
        for (i, px) in cel.pixels.chunks_exact_mut(4).enumerate() {
            let (x, y) = (i as i32 % w, i as i32 / w);
            let (rx, ry) = (x as f32 + 0.5 - ax, y as f32 + 0.5 - ay);
            let t = if len2 == 0.0 {
                0.0
            } else {
                match kind {
                    GradientKind::Linear => (rx * dx + ry * dy) / len2,
                    GradientKind::Radial => ((rx * rx + ry * ry) / len2).sqrt(),
                }
            }
            .clamp(0.0, 1.0);
            let c = if dither >= 2 {
                let n = dither.next_power_of_two().min(8);
                if t > (bayer(n, x, y) as f32 + 0.5) / (n * n) as f32 {
                    c1
                } else {
                    c0
                }
            } else {
                lerp(c0, c1, t)
            };
            // A transparent gradient color replaces under Simple ink but adds nothing under the others.
            if c[3] != 0 || settings.ink == Ink::Simple {
                paint(&settings, px, c);
            }
        }
    }
}

/// Which of the two colors a pattern puts at a pixel. `pattern`: 0 solid, 1 checker (2x2
/// blocks), 2 25%, 3 50%, 4 75% of the first color (ordered 4x4 dither).
pub fn pattern_color(pattern: u32, x: i32, y: i32, a: Color, b: Color) -> Color {
    let first = match pattern {
        0 => true,
        1 => (x.div_euclid(2) + y.div_euclid(2)) % 2 == 0,
        n => bayer(4, x, y) < (n.clamp(2, 4) - 1) * 4,
    };
    if first {
        a
    } else {
        b
    }
}

/// Fills the area under `(x, y)` with a pattern of two colors through the current ink, inside the
/// selection if any. `sym` (bit 0 mirror left/right, bit 1 top/bottom) also fills the areas under
/// the mirrored seed points.
#[allow(clippy::too_many_arguments)]
pub fn fill_pattern(
    e: &mut Editor,
    layer: usize,
    frame: usize,
    x: i32,
    y: i32,
    a: Color,
    b: Color,
    pattern: u32,
    contiguous: bool,
    sym: u32,
) {
    let (w, h) = (e.sprite.width as i32, e.sprite.height as i32);
    let sel = e.sel.active(w as u32, h as u32);
    let Some(cel) = e.sprite.cel_mut(layer, frame) else { return };
    let get = |px: &[u8], x: i32, y: i32| -> Color { px[(y * w + x) as usize * 4..][..4].try_into().unwrap() };
    let same = |p: Color, q: Color| p == q || (p[3] == 0 && q[3] == 0);
    let allowed = |x: i32, y: i32| sel.is_none_or(|s| s.get(x, y));
    let mut seeds = crate::paint::shapes::mirrors((x, y), (w, h), sym);
    seeds.sort();
    seeds.dedup();
    let mut region = vec![false; (w * h) as usize];
    for (sx, sy) in seeds {
        if sx < 0 || sy < 0 || sx >= w || sy >= h || !allowed(sx, sy) {
            continue;
        }
        let target = get(&cel.pixels, sx, sy);
        if contiguous {
            let mut seen = vec![false; (w * h) as usize];
            let mut stack = vec![(sx, sy)];
            while let Some((x, y)) = stack.pop() {
                if x < 0 || y < 0 || x >= w || y >= h || seen[(y * w + x) as usize] {
                    continue;
                }
                seen[(y * w + x) as usize] = true;
                if !allowed(x, y) || !same(get(&cel.pixels, x, y), target) {
                    continue;
                }
                region[(y * w + x) as usize] = true;
                stack.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
            }
        } else {
            for yy in 0..h {
                for xx in 0..w {
                    if allowed(xx, yy) && same(get(&cel.pixels, xx, yy), target) {
                        region[(yy * w + xx) as usize] = true;
                    }
                }
            }
        }
    }
    let settings = settings();
    for (i, _) in region.iter().enumerate().filter(|(_, &r)| r) {
        let (x, y) = (i as i32 % w, i as i32 / w);
        let c = pattern_color(pattern, x, y, a, b);
        if c[3] != 0 || settings.ink == Ink::Simple {
            paint(&settings, &mut cel.pixels[i * 4..i * 4 + 4], c);
        }
    }
    crate::document::color_mode::conform_cel(&mut e.sprite, layer, frame);
}

fn rgba(c: u32) -> Color {
    c.to_be_bytes()
}

/// Sets the ink mode (0 simple, 1 alpha compositing, 2 lock alpha, 3 behind, 4 shading) and the
/// shading ramp of 0xRRGGBBAA colors.
#[no_mangle]
pub unsafe extern "C" fn pixmith_ink_set(mode: u32, reverse: bool, ramp: *const u32, n: usize) {
    crate::ffi::guarded("pixmith_ink_set", std::ptr::null_mut(), || {
        let ramp: Vec<Color> = if ramp.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(ramp, n).iter().map(|&c| rgba(c)).collect()
        };
        update_settings(|s| {
            s.ink = Ink::from_u32(mode);
            s.reverse = reverse;
            s.ramp = ramp;
        });
    })
}

/// Round instead of square brush for sizes above one.
#[no_mangle]
pub extern "C" fn pixmith_brush_circle(circle: bool) {
    crate::ffi::guarded("pixmith_brush_circle", std::ptr::null_mut(), || {
        update_settings(|s| s.circle = circle);
    })
}

/// Uses the selected pixels of a layer as the brush. False if nothing visible is selected.
#[no_mangle]
pub unsafe extern "C" fn pixmith_brush_from_selection(doc: *const Editor, layer: u32, frame: u32) -> bool {
    crate::ffi::guarded("pixmith_brush_from_selection", std::ptr::null_mut(), || {
        let Some(e) = doc.as_ref() else { return false };
        let (_, _, w, h) = crate::paint::selection::copy_bounds(e);
        let Some(px) = crate::paint::selection::copy(e, layer as usize, frame as usize) else { return false };
        let Some(brush) = Brush::from_rgba(w, h, &px) else { return false };
        update_settings(|s| s.custom = Some(Rc::new(brush)));
        true
    })
}

#[no_mangle]
pub extern "C" fn pixmith_brush_reset() {
    crate::ffi::guarded("pixmith_brush_reset", std::ptr::null_mut(), || {
        update_settings(|s| s.custom = None);
    })
}

/// Writes `[has_custom, width, height]` of the custom brush.
#[no_mangle]
pub unsafe extern "C" fn pixmith_brush_info(out: *mut i32) {
    crate::ffi::guarded("pixmith_brush_info", std::ptr::null_mut(), || {
        if out.is_null() {
            return;
        }
        let (has, w, h) = settings().custom.as_ref().map_or((0, 0, 0), |b| (1, b.w, b.h));
        std::slice::from_raw_parts_mut(out, 3).copy_from_slice(&[has, w, h]);
    })
}

/// Gradient over the whole cel (inside the selection, if any). Kind 0 linear, 1 radial; dither 0
/// smooth or a Bayer size. With `restore`, rewinds to the `pixmith_begin_stroke` checkpoint first.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_gradient(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    c0: u32,
    c1: u32,
    kind: u32,
    dither: u32,
    restore: bool,
) {
    crate::ffi::guarded("pixmith_gradient", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return };
        if restore {
            crate::paint::shapes::reset_to_checkpoint(e);
        }
        let kind = if kind == 1 { GradientKind::Radial } else { GradientKind::Linear };
        e.sprite.gradient(layer as usize, frame as usize, (x0, y0), (x1, y1), rgba(c0), rgba(c1), kind, dither);
        crate::paint::selection::clip_to_selection(e, layer as usize, frame as usize);
    })
}

/// Fill with a two-color dither pattern (see `pattern_color`) through the current ink. `flags`:
/// bit 0 mirror left/right, bit 1 top/bottom (the mirrored seed points are filled too).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_fill_pattern(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    a: u32,
    b: u32,
    pattern: u32,
    contiguous: bool,
    flags: u32,
) {
    crate::ffi::guarded("pixmith_fill_pattern", doc, || {
        if let Some(e) = crate::ffi::editor(doc) {
            fill_pattern(e, layer as usize, frame as usize, x, y, rgba(a), rgba(b), pattern, contiguous, flags);
            crate::paint::selection::sync_tiles(e, layer as usize, frame as usize);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = [255, 0, 0, 255];
    const BLUE: Color = [0, 0, 255, 255];

    fn ink(i: Ink) -> Settings {
        Settings { ink: i, ..Default::default() }
    }

    #[test]
    fn simple_replaces_and_transparent_erases() {
        let mut d = [1, 2, 3, 4];
        paint(&ink(Ink::Simple), &mut d, RED);
        assert_eq!(d, RED);
        paint(&ink(Ink::Alpha), &mut d, [0, 0, 0, 0]);
        assert_eq!(d, [0; 4]);
    }

    #[test]
    fn alpha_compositing_blends() {
        let mut d = BLUE;
        paint(&ink(Ink::Alpha), &mut d, [255, 0, 0, 128]);
        assert_eq!(d[3], 255);
        assert!(d[0] > 100 && d[2] > 100);
        let mut empty = [0; 4];
        paint(&ink(Ink::Alpha), &mut empty, [255, 0, 0, 128]);
        assert_eq!(empty, [255, 0, 0, 128]);
    }

    #[test]
    fn lock_alpha_only_touches_visible_pixels_and_keeps_alpha() {
        let mut empty = [0; 4];
        paint(&ink(Ink::LockAlpha), &mut empty, RED);
        assert_eq!(empty, [0; 4]);
        let mut half = [0, 0, 255, 100];
        paint(&ink(Ink::LockAlpha), &mut half, RED);
        assert_eq!(half, [255, 0, 0, 100]);
    }

    #[test]
    fn behind_paints_under() {
        let mut empty = [0; 4];
        paint(&ink(Ink::Behind), &mut empty, RED);
        assert_eq!(empty, RED);
        let mut solid = BLUE;
        paint(&ink(Ink::Behind), &mut solid, RED);
        assert_eq!(solid, BLUE);
        let mut half = [0, 0, 255, 128];
        paint(&ink(Ink::Behind), &mut half, RED);
        assert_eq!(half[3], 255);
        assert!(half[0] > 100 && half[2] > 50);
    }

    #[test]
    fn shading_walks_the_ramp_both_ways() {
        let ramp = vec![[10, 0, 0, 255], [20, 0, 0, 255], [30, 0, 0, 255]];
        let mut s = Settings { ink: Ink::Shading, ramp, ..Default::default() };
        let mut d = [20, 0, 0, 255];
        paint(&s, &mut d, RED);
        assert_eq!(d, [30, 0, 0, 255]);
        paint(&s, &mut d, RED);
        assert_eq!(d, [30, 0, 0, 255]);
        s.reverse = true;
        paint(&s, &mut d, RED);
        assert_eq!(d, [20, 0, 0, 255]);
        let mut other = [99, 0, 0, 255];
        paint(&s, &mut other, RED);
        assert_eq!(other, [99, 0, 0, 255]);
        let mut half = [20, 0, 0, 77];
        paint(&s, &mut half, RED);
        assert_eq!(half, [10, 0, 0, 77]);
    }

    #[test]
    fn bayer_matrices_are_permutations() {
        for n in [2u32, 4, 8] {
            let mut seen: Vec<u32> = (0..n * n).map(|i| bayer(n, (i % n) as i32, (i / n) as i32)).collect();
            seen.sort();
            assert_eq!(seen, (0..n * n).collect::<Vec<_>>());
        }
        assert_eq!(bayer(4, 0, 0), 0);
        assert_eq!((bayer(4, 1, 1), bayer(4, 2, 0), bayer(4, 0, 2)), (4, 2, 3));
    }

    #[test]
    fn linear_gradient_is_monotonic() {
        let mut s = Sprite::new(9, 1);
        s.gradient(0, 0, (0, 0), (8, 0), [0, 0, 0, 255], [255, 255, 255, 255], GradientKind::Linear, 0);
        let px = s.render(0);
        let reds: Vec<u8> = px.chunks(4).map(|c| c[0]).collect();
        assert!(reds.windows(2).all(|w| w[0] <= w[1]));
        assert!(reds[0] < 20);
        assert_eq!(reds[8], 255);
    }

    #[test]
    fn radial_gradient_grows_from_center() {
        let mut s = Sprite::new(11, 11);
        s.gradient(0, 0, (5, 5), (10, 5), [0, 0, 0, 255], [200, 0, 0, 255], GradientKind::Radial, 0);
        let px = s.render(0);
        let r = |x: usize, y: usize| px[(y * 11 + x) * 4];
        assert!(r(5, 5) < r(7, 5) && r(7, 5) < r(10, 5));
        assert_eq!(r(5, 8), r(8, 5));
        assert_eq!(r(0, 0), 200);
    }

    #[test]
    fn dithered_gradient_uses_only_two_colors() {
        for n in [2, 4, 8] {
            let mut s = Sprite::new(16, 16);
            s.gradient(0, 0, (0, 0), (15, 0), RED, BLUE, GradientKind::Linear, n);
            let px = s.render(0);
            assert!(px.chunks(4).all(|c| c == RED || c == BLUE));
            assert_eq!(&px[..4], &RED);
            assert_eq!(&px[15 * 4..16 * 4], &BLUE);
        }
    }

    #[test]
    fn gradient_blends_alpha_without_dark_fringes() {
        let c = lerp([255, 0, 0, 0], [0, 255, 0, 255], 0.5);
        assert_eq!(c, [0, 255, 0, 128]);
    }

    #[test]
    fn brush_from_rgba_crops() {
        let mut px = vec![0u8; 4 * 4 * 4];
        px[(1 * 4 + 2) * 4..][..4].copy_from_slice(&RED);
        px[(2 * 4 + 1) * 4..][..4].copy_from_slice(&BLUE);
        let b = Brush::from_rgba(4, 4, &px).unwrap();
        assert_eq!((b.w, b.h), (2, 2));
        assert_eq!(&b.px[4..8], &RED);
        assert_eq!(&b.px[8..12], &BLUE);
        assert!(Brush::from_rgba(2, 2, &[0; 16]).is_none());
        assert!(Brush::from_rgba(2, 2, &[0; 3]).is_none());
    }

    #[test]
    fn pattern_densities() {
        let count = |p: u32| (0..16).filter(|&i| pattern_color(p, i % 4, i / 4, RED, BLUE) == RED).count();
        assert_eq!((count(0), count(2), count(3), count(4)), (16, 4, 8, 12));
        assert_eq!(count(1), 8);
        assert_eq!(pattern_color(1, 0, 0, RED, BLUE), RED);
        assert_eq!(pattern_color(1, 2, 0, RED, BLUE), BLUE);
        assert_eq!(pattern_color(1, 1, 1, RED, BLUE), RED);
    }

    fn editor_with_wall() -> Editor {
        let mut e = Editor::new(Sprite::new(6, 2));
        e.sprite.draw_line(0, 0, 3, 0, 3, 1, [0, 255, 0, 255], 1);
        e
    }

    #[test]
    fn pattern_fill_respects_contiguity() {
        let mut e = editor_with_wall();
        fill_pattern(&mut e, 0, 0, 0, 0, RED, BLUE, 3, true, 0);
        let px = e.sprite.render(0);
        let at = |x: usize, y: usize| &px[(y * 6 + x) * 4..][..4];
        assert!(at(0, 0) == RED || at(0, 0) == BLUE);
        assert_eq!(at(3, 0), &[0, 255, 0, 255]);
        assert_eq!(at(4, 0), &[0; 4]);

        let mut e = editor_with_wall();
        fill_pattern(&mut e, 0, 0, 0, 0, RED, BLUE, 0, false, 0);
        let px = e.sprite.render(0);
        assert_eq!(&px[4 * 4..5 * 4], &RED);
        assert_eq!(&px[3 * 4..4 * 4], &[0, 255, 0, 255]);
    }

    #[test]
    fn pattern_fill_checker_alternates() {
        let mut e = Editor::new(Sprite::new(4, 4));
        fill_pattern(&mut e, 0, 0, 0, 0, RED, BLUE, 1, true, 0);
        let px = e.sprite.render(0);
        assert_eq!(&px[0..4], &RED);
        assert_eq!(&px[2 * 4..3 * 4], &BLUE);
        assert_eq!(&px[(2 * 4) * 4..][..4], &BLUE);
        assert_eq!(&px[(2 * 4 + 2) * 4..][..4], &RED);
    }

    fn wall_editor() -> Editor {
        let mut e = Editor::new(Sprite::new(8, 2));
        e.sprite.draw_line(0, 0, 3, 0, 3, 1, [0, 255, 0, 255], 1);
        e.sprite.draw_line(0, 0, 4, 0, 4, 1, [0, 255, 0, 255], 1);
        e
    }

    #[test]
    fn pattern_fill_mirrors_the_seed() {
        let mut e = wall_editor();
        fill_pattern(&mut e, 0, 0, 0, 0, RED, BLUE, 0, true, 0);
        let px = e.sprite.render(0);
        assert_eq!(&px[0..4], &RED);
        assert_eq!(&px[7 * 4..8 * 4], &[0; 4]);
        let mut e = wall_editor();
        fill_pattern(&mut e, 0, 0, 1, 1, RED, BLUE, 0, true, 1);
        let px = e.sprite.render(0);
        assert_eq!(&px[(8 + 1) * 4..][..4], &RED);
        assert_eq!(&px[(8 + 6) * 4..][..4], &RED);
        assert_eq!(&px[3 * 4..][..4], &[0, 255, 0, 255]);
        // Seeds on the axis are not applied twice.
        let mut e = Editor::new(Sprite::new(5, 1));
        update_settings(|s| s.ink = Ink::Alpha);
        fill_pattern(&mut e, 0, 0, 2, 0, [255, 0, 0, 100], BLUE, 0, true, 1);
        assert_eq!(e.sprite.render(0)[3], 100);
        update_settings(|s| s.ink = Ink::Simple);
    }

    #[test]
    fn fill_honors_ink_modes() {
        let mut e = Editor::new(Sprite::new(4, 1));
        update_settings(|s| s.ink = Ink::LockAlpha);
        fill_pattern(&mut e, 0, 0, 0, 0, RED, BLUE, 0, true, 0);
        assert!(e.sprite.render(0).iter().all(|&b| b == 0));
        update_settings(|s| s.ink = Ink::Alpha);
        fill_pattern(&mut e, 0, 0, 0, 0, [255, 0, 0, 128], BLUE, 0, true, 0);
        assert_eq!(e.sprite.render(0)[3], 128);
        update_settings(|s| s.ink = Ink::Behind);
        fill_pattern(&mut e, 0, 0, 0, 0, BLUE, BLUE, 0, false, 0);
        let px = e.sprite.render(0);
        assert_eq!(px[3], 255);
        assert!(px[0] > 100 && px[2] > 50, "{px:?}");
        update_settings(|s| s.ink = Ink::Simple);
    }

    #[test]
    fn gradient_honors_ink_modes() {
        update_settings(|s| s.ink = Ink::LockAlpha);
        let mut s = Sprite::new(4, 1);
        s.gradient(0, 0, (0, 0), (3, 0), RED, BLUE, GradientKind::Linear, 0);
        assert!(s.render(0).iter().all(|&b| b == 0));
        update_settings(|s| s.ink = Ink::Alpha);
        let mut s = Sprite::new(4, 1);
        s.draw_line(0, 0, 0, 0, 3, 0, [0, 255, 0, 255], 1);
        s.gradient(0, 0, (0, 0), (3, 0), [255, 0, 0, 0], [255, 0, 0, 255], GradientKind::Linear, 0);
        let px = s.render(0);
        assert_eq!(px[3], 255);
        assert!(px[0] < px[3 * 4] && px[3 * 4] > 200, "{px:?}");
        update_settings(|s| s.ink = Ink::Simple);
    }

    #[test]
    fn settings_round_trip_through_ffi() {
        unsafe {
            let ramp = [0xFF0000FFu32, 0x00FF00FF];
            pixmith_ink_set(4, true, ramp.as_ptr(), 2);
        }
        let s = settings();
        assert_eq!((s.ink, s.reverse, s.ramp.len()), (Ink::Shading, true, 2));
        assert_eq!(s.ramp[1], [0, 255, 0, 255]);
        unsafe { pixmith_ink_set(0, false, std::ptr::null(), 0) };
        pixmith_brush_circle(true);
        assert!(settings().circle);
        let mut e = Editor::new(Sprite::new(4, 4));
        e.sprite.draw_line(0, 0, 1, 1, 2, 1, RED, 1);
        crate::paint::selection::select_rect(&mut e, (0, 0), (3, 3));
        unsafe {
            assert!(pixmith_brush_from_selection(&e, 0, 0));
            let mut info = [0; 3];
            pixmith_brush_info(info.as_mut_ptr());
            assert_eq!(info, [1, 2, 1]);
        }
        pixmith_brush_reset();
        pixmith_brush_circle(false);
        assert!(settings().custom.is_none());
    }
}
