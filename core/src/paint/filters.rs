//! Pixel filters (color adjustments, outline, shadow, convolution) and sprite scaling.
//!
//! Every filter reads a straight-alpha RGBA cel buffer and produces a new one. `apply` runs a
//! filter over a target (one cel, a layer across frames, or a frame across layers), optionally
//! limited to a rectangle. The FFI wrappers at the bottom make each call a single undo step.

use crate::document::{Cel, Editor, Sprite};
use std::sync::Arc;

/// Which cels a filter touches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The cel at (layer, frame).
    Cel,
    /// Every frame of the layer.
    AllFrames,
    /// Every layer of the frame.
    AllLayers,
}

impl Target {
    pub fn from_u32(v: u32) -> Option<Target> {
        match v {
            0 => Some(Target::Cel),
            1 => Some(Target::AllFrames),
            2 => Some(Target::AllLayers),
            _ => None,
        }
    }
}

/// A canvas rectangle limiting where a filter writes. `w == 0` means the whole canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kernel {
    Blur,
    Sharpen,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Filter {
    /// Pixels within `tolerance` (max per-channel RGBA difference) of `from` become `to`.
    ReplaceColor {
        from: [u8; 4],
        to: [u8; 4],
        tolerance: u8,
    },
    Invert,
    Desaturate,
    /// Hue shift in degrees; saturation and lightness in -100..=100 percent.
    Hsl {
        hue: i32,
        saturation: i32,
        lightness: i32,
    },
    /// Both in -100..=100.
    BrightnessContrast {
        brightness: i32,
        contrast: i32,
    },
    /// Per-channel offsets, saturating.
    ChannelOffset {
        r: i32,
        g: i32,
        b: i32,
    },
    Outline {
        color: [u8; 4],
        inside: bool,
        eight: bool,
    },
    DropShadow {
        dx: i32,
        dy: i32,
        color: [u8; 4],
    },
    Despeckle {
        eight: bool,
    },
    Convolve(Kernel),
    /// Lookup tables for R, G, B and A, already combined with the master (RGB) curve.
    Curves(Box<[[u8; 256]; 4]>),
}

pub const KIND_REPLACE_COLOR: u32 = 0;
pub const KIND_INVERT: u32 = 1;
pub const KIND_DESATURATE: u32 = 2;
pub const KIND_HSL: u32 = 3;
pub const KIND_BRIGHTNESS_CONTRAST: u32 = 4;
pub const KIND_CHANNEL_OFFSET: u32 = 5;
pub const KIND_OUTLINE: u32 = 6;
pub const KIND_DROP_SHADOW: u32 = 7;
pub const KIND_DESPECKLE: u32 = 8;
pub const KIND_BLUR: u32 = 9;
pub const KIND_SHARPEN: u32 = 10;
pub const KIND_CURVES: u32 = 11;

/// Most control points accepted per curve.
const MAX_CURVE_POINTS: usize = 64;

/// Builds a 256-entry lookup table from control points `(x, y)`. Between points the curve is a
/// monotone cubic (it never overshoots the neighboring points); outside them it is flat.
/// Fewer than two distinct x positions give the identity.
pub fn curve_lut(points: &[(u8, u8)]) -> [u8; 256] {
    let mut pts: Vec<(f32, f32)> = Vec::new();
    let mut sorted = points.to_vec();
    sorted.sort_by_key(|p| p.0); // stable: for a repeated x the later point wins
    for (x, y) in sorted {
        match pts.last_mut() {
            Some(last) if last.0 == x as f32 => last.1 = y as f32,
            _ => pts.push((x as f32, y as f32)),
        }
    }
    let mut lut = [0u8; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        *v = i as u8;
    }
    let n = pts.len();
    if n < 2 {
        return lut;
    }
    let h: Vec<f32> = (0..n - 1).map(|k| pts[k + 1].0 - pts[k].0).collect();
    let d: Vec<f32> = (0..n - 1).map(|k| (pts[k + 1].1 - pts[k].1) / h[k]).collect();
    let mut m = vec![0.0f32; n];
    m[0] = d[0];
    m[n - 1] = d[n - 2];
    for k in 1..n - 1 {
        if d[k - 1] * d[k] > 0.0 {
            let (w1, w2) = (2.0 * h[k] + h[k - 1], h[k] + 2.0 * h[k - 1]);
            m[k] = (w1 + w2) / (w1 / d[k - 1] + w2 / d[k]);
        }
    }
    let mut seg = 0;
    for (i, v) in lut.iter_mut().enumerate() {
        let x = i as f32;
        *v = if x <= pts[0].0 {
            pts[0].1
        } else if x >= pts[n - 1].0 {
            pts[n - 1].1
        } else {
            while pts[seg + 1].0 < x {
                seg += 1;
            }
            let t = (x - pts[seg].0) / h[seg];
            let (t2, t3) = (t * t, t * t * t);
            (2.0 * t3 - 3.0 * t2 + 1.0) * pts[seg].1
                + (t3 - 2.0 * t2 + t) * h[seg] * m[seg]
                + (-2.0 * t3 + 3.0 * t2) * pts[seg + 1].1
                + (t3 - t2) * h[seg] * m[seg + 1]
        }
        .round()
        .clamp(0.0, 255.0) as u8;
    }
    lut
}

/// Reads one curve from `p` starting at `*i`: a point count, then x, y pairs. Advances `*i`.
fn read_curve(p: &[i32], i: &mut usize) -> Option<[u8; 256]> {
    let n = *p.get(*i)?;
    let n = usize::try_from(n).ok().filter(|&n| n <= MAX_CURVE_POINTS)?;
    let body = p.get(*i + 1..*i + 1 + 2 * n)?;
    *i += 1 + 2 * n;
    let pts: Vec<(u8, u8)> =
        body.chunks_exact(2).map(|c| (c[0].clamp(0, 255) as u8, c[1].clamp(0, 255) as u8)).collect();
    Some(curve_lut(&pts))
}

/// Parameters: five curves (RGB master, R, G, B, A), each as a point count followed by x, y pairs.
fn curves_from_params(p: &[i32]) -> Option<Filter> {
    let mut i = 0;
    let master = read_curve(p, &mut i)?;
    let mut luts = [[0u8; 256]; 4];
    for (c, lut) in luts.iter_mut().enumerate() {
        let own = read_curve(p, &mut i)?;
        *lut = if c < 3 { std::array::from_fn(|v| own[master[v] as usize]) } else { own };
    }
    Some(Filter::Curves(Box::new(luts)))
}

impl Filter {
    /// Builds a filter from an FFI kind id and integer parameters (colors are 0xRRGGBBAA bit
    /// patterns). Returns `None` for an unknown kind or too few parameters.
    pub fn from_params(kind: u32, p: &[i32]) -> Option<Filter> {
        let color = |i: usize| p.get(i).map(|&v| (v as u32).to_be_bytes());
        let flag = |i: usize| p.get(i).map(|&v| v != 0);
        let int = |i: usize| p.get(i).copied();
        Some(match kind {
            KIND_REPLACE_COLOR => {
                Filter::ReplaceColor { from: color(0)?, to: color(1)?, tolerance: int(2)?.clamp(0, 255) as u8 }
            }
            KIND_INVERT => Filter::Invert,
            KIND_DESATURATE => Filter::Desaturate,
            KIND_HSL => Filter::Hsl { hue: int(0)?, saturation: int(1)?, lightness: int(2)? },
            KIND_BRIGHTNESS_CONTRAST => Filter::BrightnessContrast { brightness: int(0)?, contrast: int(1)? },
            KIND_CHANNEL_OFFSET => Filter::ChannelOffset { r: int(0)?, g: int(1)?, b: int(2)? },
            KIND_OUTLINE => Filter::Outline { color: color(0)?, inside: flag(1)?, eight: flag(2)? },
            KIND_DROP_SHADOW => Filter::DropShadow { dx: int(0)?, dy: int(1)?, color: color(2)? },
            KIND_DESPECKLE => Filter::Despeckle { eight: flag(0)? },
            KIND_BLUR => Filter::Convolve(Kernel::Blur),
            KIND_SHARPEN => Filter::Convolve(Kernel::Sharpen),
            KIND_CURVES => return curves_from_params(p),
            _ => return None,
        })
    }

    /// Returns a filtered copy of a `w` x `h` straight-alpha RGBA buffer.
    pub fn run(&self, src: &[u8], w: usize, h: usize) -> Vec<u8> {
        match *self {
            Filter::ReplaceColor { from, to, tolerance } => map_pixels(src, |p| {
                let hit =
                    if from[3] == 0 && p[3] == 0 { true } else { (0..4).all(|c| p[c].abs_diff(from[c]) <= tolerance) };
                if hit {
                    to
                } else {
                    p
                }
            }),
            Filter::Invert => map_pixels(src, |p| [255 - p[0], 255 - p[1], 255 - p[2], p[3]]),
            Filter::Desaturate => map_pixels(src, |p| {
                let g = ((299 * p[0] as u32 + 587 * p[1] as u32 + 114 * p[2] as u32 + 500) / 1000) as u8;
                [g, g, g, p[3]]
            }),
            Filter::Hsl { hue, saturation, lightness } => map_pixels(src, |p| {
                let (hh, s, l) = rgb_to_hsl(p[0], p[1], p[2]);
                let hh = (hh + hue as f32).rem_euclid(360.0);
                let s = shift_unit(s, saturation);
                let l = shift_unit(l, lightness);
                let [r, g, b] = hsl_to_rgb(hh, s, l);
                [r, g, b, p[3]]
            }),
            Filter::BrightnessContrast { brightness, contrast } => {
                let add = brightness.clamp(-100, 100) as f32 * 255.0 / 100.0;
                let cc = contrast.clamp(-100, 100) as f32 * 255.0 / 100.0;
                let factor = 259.0 * (cc + 255.0) / (255.0 * (259.0 - cc));
                map_pixels(src, |p| {
                    let f = |v: u8| ((v as f32 - 128.0) * factor + 128.0 + add).round().clamp(0.0, 255.0) as u8;
                    [f(p[0]), f(p[1]), f(p[2]), p[3]]
                })
            }
            Filter::ChannelOffset { r, g, b } => map_pixels(src, |p| {
                let f = |v: u8, o: i32| (v as i32 + o).clamp(0, 255) as u8;
                [f(p[0], r), f(p[1], g), f(p[2], b), p[3]]
            }),
            Filter::Outline { color, inside, eight } => outline(src, w, h, color, inside, eight),
            Filter::DropShadow { dx, dy, color } => drop_shadow(src, w, h, dx, dy, color),
            Filter::Despeckle { eight } => despeckle(src, w, h, eight),
            Filter::Convolve(k) => convolve(src, w, h, k),
            Filter::Curves(ref l) => map_pixels(src, |p| {
                [l[0][p[0] as usize], l[1][p[1] as usize], l[2][p[2] as usize], l[3][p[3] as usize]]
            }),
        }
    }
}

fn map_pixels(src: &[u8], f: impl Fn([u8; 4]) -> [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    for p in src.chunks_exact(4) {
        out.extend_from_slice(&f([p[0], p[1], p[2], p[3]]));
    }
    out
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s.clamp(0.0, 1.0), l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r, g, b].map(|v| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// Moves `v` in 0..=1 toward 1 (positive percent) or 0 (negative percent).
fn shift_unit(v: f32, percent: i32) -> f32 {
    let t = percent.clamp(-100, 100) as f32 / 100.0;
    if t >= 0.0 {
        v + (1.0 - v) * t
    } else {
        v * (1.0 + t)
    }
}

const N4: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
const N8: [(i32, i32); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)];

fn neighbors(eight: bool) -> &'static [(i32, i32)] {
    if eight {
        &N8
    } else {
        &N4
    }
}

/// Alpha at (x, y); outside the canvas counts as transparent.
fn alpha_at(src: &[u8], w: usize, h: usize, x: i32, y: i32) -> u8 {
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        0
    } else {
        src[(y as usize * w + x as usize) * 4 + 3]
    }
}

fn outline(src: &[u8], w: usize, h: usize, color: [u8; 4], inside: bool, eight: bool) -> Vec<u8> {
    let mut out = src.to_vec();
    let ns = neighbors(eight);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let i = (y as usize * w + x as usize) * 4;
            let opaque = src[i + 3] > 0;
            let edge = if inside {
                opaque && ns.iter().any(|&(dx, dy)| alpha_at(src, w, h, x + dx, y + dy) == 0)
            } else {
                !opaque && ns.iter().any(|&(dx, dy)| alpha_at(src, w, h, x + dx, y + dy) > 0)
            };
            if edge {
                out[i..i + 4].copy_from_slice(&color);
            }
        }
    }
    out
}

/// Source-over with straight alpha: `s` on top of `d`.
fn over(d: [u8; 4], s: [u8; 4]) -> [u8; 4] {
    let (sa, da) = (s[3] as u32, d[3] as u32);
    if sa == 0 {
        return d;
    }
    if sa == 255 || da == 0 {
        return s;
    }
    let da_part = da * (255 - sa) / 255;
    let oa = sa + da_part;
    let mut out = [0u8; 4];
    for c in 0..3 {
        out[c] = ((s[c] as u32 * sa + d[c] as u32 * da_part + oa / 2) / oa) as u8;
    }
    out[3] = oa as u8;
    out
}

fn drop_shadow(src: &[u8], w: usize, h: usize, dx: i32, dy: i32, color: [u8; 4]) -> Vec<u8> {
    let mut out = src.to_vec();
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let sa = alpha_at(src, w, h, x - dx, y - dy) as u32;
            if sa == 0 {
                continue;
            }
            let shadow = [color[0], color[1], color[2], (sa * color[3] as u32 / 255) as u8];
            let i = (y as usize * w + x as usize) * 4;
            let d = [src[i], src[i + 1], src[i + 2], src[i + 3]];
            out[i..i + 4].copy_from_slice(&over(shadow, d));
        }
    }
    out
}

fn despeckle(src: &[u8], w: usize, h: usize, eight: bool) -> Vec<u8> {
    let mut out = src.to_vec();
    let ns = neighbors(eight);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let i = (y as usize * w + x as usize) * 4;
            if src[i + 3] > 0 && ns.iter().all(|&(dx, dy)| alpha_at(src, w, h, x + dx, y + dy) == 0) {
                out[i..i + 4].fill(0);
            }
        }
    }
    out
}

/// Alpha-aware 3x3 convolution with edge replication: colors are weighted by alpha so
/// transparent pixels do not bleed their (meaningless) color into neighbors.
fn convolve(src: &[u8], w: usize, h: usize, kernel: Kernel) -> Vec<u8> {
    let (k, div): ([f32; 9], f32) = match kernel {
        Kernel::Blur => ([1.0, 2.0, 1.0, 2.0, 4.0, 2.0, 1.0, 2.0, 1.0], 16.0),
        Kernel::Sharpen => ([0.0, -1.0, 0.0, -1.0, 5.0, -1.0, 0.0, -1.0, 0.0], 1.0),
    };
    let mut out = src.to_vec();
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let (mut sa, mut sr, mut sg, mut sb) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for ky in 0..3 {
                for kx in 0..3 {
                    let px = (x + kx - 1).clamp(0, w as i32 - 1) as usize;
                    let py = (y + ky - 1).clamp(0, h as i32 - 1) as usize;
                    let i = (py * w + px) * 4;
                    let wt = k[(ky * 3 + kx) as usize] / div;
                    let a = src[i + 3] as f32 / 255.0;
                    sa += wt * a;
                    sr += wt * a * src[i] as f32;
                    sg += wt * a * src[i + 1] as f32;
                    sb += wt * a * src[i + 2] as f32;
                }
            }
            let o = (y as usize * w + x as usize) * 4;
            if sa <= 0.0 {
                out[o..o + 4].fill(0);
                continue;
            }
            // Premultiplied sums divided by the alpha weight give the straight color.
            out[o] = (sr / sa).round().clamp(0.0, 255.0) as u8;
            out[o + 1] = (sg / sa).round().clamp(0.0, 255.0) as u8;
            out[o + 2] = (sb / sa).round().clamp(0.0, 255.0) as u8;
            out[o + 3] = (sa.min(1.0) * 255.0).round() as u8;
        }
    }
    out
}

/// Applies `filter` to the cels chosen by `target`. Cels that don't exist, group layers and
/// non-image layers are skipped. Returns whether any pixel changed.
pub fn apply(
    sprite: &mut Sprite,
    filter: &Filter,
    target: Target,
    layer: usize,
    frame: usize,
    rect: Option<Rect>,
) -> bool {
    let (w, h) = (sprite.width as usize, sprite.height as usize);
    let (x0, y0, x1, y1) = match rect.filter(|r| r.w > 0) {
        Some(r) => {
            let x0 = (r.x.max(0) as usize).min(w);
            let y0 = (r.y.max(0) as usize).min(h);
            let x1 = (r.x as i64 + r.w as i64).clamp(x0 as i64, w as i64) as usize;
            let y1 = (r.y as i64 + r.h as i64).clamp(y0 as i64, h as i64) as usize;
            (x0, y0, x1, y1)
        }
        None => (0, 0, w, h),
    };
    let mut cells = Vec::new();
    match target {
        Target::Cel => cells.push((layer, frame)),
        Target::AllFrames => cells.extend((0..sprite.frame_count()).map(|f| (layer, f))),
        Target::AllLayers => cells.extend((0..sprite.layers.len()).map(|l| (l, frame))),
    }
    let mut changed = false;
    let mut snapper = crate::document::color_mode::Snapper::new(sprite);
    let backgrounds: Vec<bool> = sprite.layers.iter().map(|l| l.flags & 8 != 0).collect();
    // Linked cels share one image: filter each distinct cel once and keep the link (like Aseprite).
    sprite.flush_links();
    let mut handled: std::collections::HashSet<*const Cel> = std::collections::HashSet::new();
    for (l, f) in cells {
        // Tilemap layers are filtered through their pixels and written back to the tiles by the caller.
        // Groups have no pixels; a locked reference layer is protected from filters like it is from drawing.
        if sprite.layers.get(l).is_none_or(|ly| ly.is_group() || (ly.is_reference() && ly.is_locked())) {
            continue;
        }
        let Some(Some(cel)) = sprite.cels.get_mut(f).and_then(|row| row.get_mut(l)) else { continue };
        if cel.pixels.len() != w * h * 4 || !handled.insert(Arc::as_ptr(cel)) {
            continue;
        }
        let original = cel.clone();
        let filtered = filter.run(&cel.pixels, w, h);
        let mut result = cel.pixels.clone();
        for y in y0..y1 {
            let a = (y * w + x0) * 4;
            let b = (y * w + x1) * 4;
            result[a..b].copy_from_slice(&filtered[a..b]);
        }
        // Indexed and grayscale sprites keep their palette: results are snapped before comparing.
        if snapper.active() {
            snapper.snap_rgba(&mut result, backgrounds[l]);
        }
        if result != cel.pixels {
            let new = Arc::new(Cel {
                opacity: cel.opacity,
                pixels: result,
                tiles: cel.tiles.clone(),
                user_data: cel.user_data.clone(),
                z_index: cel.z_index,
                // Filters work on the canvas; what lies beyond it stays as it is.
                outside: cel.outside.clone(),
                ..Default::default()
            });
            handled.insert(Arc::as_ptr(&new));
            for row in sprite.cels.iter_mut() {
                if let Some(slot) = row.get_mut(l) {
                    if slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, &original)) {
                        *slot = Some(new.clone());
                    }
                }
            }
            changed = true;
        }
    }
    changed
}

/// Undoes `apply`'s changes outside `mask`, so a non-rectangular selection limits a filter the way the rectangle does.
/// Returns whether any pixel still differs from `old`.
fn restrict_to_mask(next: &mut Sprite, old: &Sprite, mask: &crate::paint::selection::Selection) -> bool {
    let w = next.width as usize;
    let mut changed = false;
    // (filtered cel, its masked result): cels that share a buffer keep sharing.
    let mut done: Vec<(Arc<Cel>, Arc<Cel>)> = Vec::new();
    for (f, row) in next.cels.iter_mut().enumerate() {
        for (l, slot) in row.iter_mut().enumerate() {
            let (Some(new), Some(Some(before))) = (slot.as_ref(), old.cels.get(f).and_then(|r| r.get(l))) else {
                continue;
            };
            if Arc::ptr_eq(new, before) {
                continue;
            }
            if let Some((_, result)) = done.iter().find(|(n, _)| Arc::ptr_eq(n, new)) {
                *slot = Some(result.clone());
                continue;
            }
            let mut pixels = new.pixels.clone();
            for (i, px) in pixels.chunks_exact_mut(4).enumerate() {
                if !mask.get((i % w) as i32, (i / w) as i32) {
                    px.copy_from_slice(&before.pixels[i * 4..i * 4 + 4]);
                }
            }
            let result = if pixels == before.pixels {
                before.clone()
            } else {
                changed = true;
                Arc::new(Cel { pixels, ..(**new).clone() })
            };
            done.push((new.clone(), result.clone()));
            *slot = Some(result);
        }
    }
    changed
}

/// Nearest-neighbor resize of the whole sprite (every frame and layer). Returns false if the
/// size is unchanged or out of range.
pub fn scale_sprite(sprite: &mut Sprite, new_w: u32, new_h: u32) -> bool {
    const MAX: u32 = 8192;
    if new_w == 0 || new_h == 0 || new_w > MAX || new_h > MAX {
        return false;
    }
    if new_w == sprite.width && new_h == sprite.height {
        return false;
    }
    if !sprite.cels_fit_budget(new_w as u64, new_h as u64) {
        return false;
    }
    // Tilemap layers are enlarged tile by tile when the canvas grows by whole factors and
    // are otherwise converted to normal layers (see `tilemap::prepare_scale`).
    crate::document::tilemap::prepare_scale(sprite, new_w, new_h);
    let (w, h) = (sprite.width as usize, sprite.height as usize);
    let (nw, nh) = (new_w as usize, new_h as usize);
    if nw * nh > crate::document::MAX_CANVAS_PIXELS
        || sprite.distinct_cel_count().saturating_mul(nw * nh * 4) > crate::document::MAX_CEL_MEMORY
    {
        return false;
    }
    // Source column/row sampled at each destination pixel's center.
    let xs: Vec<usize> = (0..nw).map(|x| ((2 * x + 1) * w / (2 * nw)).min(w - 1)).collect();
    let ys: Vec<usize> = (0..nh).map(|y| ((2 * y + 1) * h / (2 * nh)).min(h - 1)).collect();
    let mut done: std::collections::HashMap<*const Cel, Arc<Cel>> = std::collections::HashMap::new();
    for cel in sprite.cels.iter_mut().flatten().flatten() {
        if cel.pixels.len() != w * h * 4 {
            continue;
        }
        // Linked cels share one buffer; scale it once so they stay linked.
        if let Some(scaled) = done.get(&Arc::as_ptr(cel)) {
            *cel = scaled.clone();
            continue;
        }
        let key = Arc::as_ptr(cel);
        let mut px = vec![0u8; nw * nh * 4];
        for (y, &sy) in ys.iter().enumerate() {
            for (x, &sx) in xs.iter().enumerate() {
                let s = (sy * w + sx) * 4;
                let d = (y * nw + x) * 4;
                px[d..d + 4].copy_from_slice(&cel.pixels[s..s + 4]);
            }
        }
        let mut scaled = Cel {
            opacity: cel.opacity,
            pixels: px,
            tiles: cel.tiles.clone(),
            user_data: cel.user_data.clone(),
            z_index: cel.z_index,
            ..Default::default()
        };
        if let Some(full) = cel.outside.as_ref().and_then(|_| cel.full_image(w, h)) {
            scale_beyond_canvas(&mut scaled, &full, (w, h), (nw, nh));
        }
        let scaled = Arc::new(scaled);
        done.insert(key, scaled.clone());
        *cel = scaled;
    }
    sprite.extra.scaled(new_w as f64 / w as f64, new_h as f64 / h as f64);
    sprite.width = new_w;
    sprite.height = new_h;
    true
}

/// Scales the full content `full` of a cel (which has pixels beyond the canvas) from a canvas of
/// `from` to one of `to`, with the same nearest-neighbor sampling as the canvas itself, and stores the
/// result in `dst`. Content that would end up too far away for the file format is dropped.
fn scale_beyond_canvas(
    dst: &mut Cel,
    full: &crate::document::offcanvas::FullImage,
    (w, h): (usize, usize),
    (nw, nh): (usize, usize),
) {
    // Destination pixel d samples source floor((2d + 1) * w / (2 nw)); find the destination range
    // whose samples fall inside the source box.
    let range = |lo: i64, len: i64, from: i64, to: i64| -> (i64, i64) {
        let first = (lo * to).div_euclid(from) - 1;
        let last = ((lo + len) * to).div_euclid(from) + 2;
        let src = |d: i64| ((2 * d + 1) * from).div_euclid(2 * to);
        let a = (first..=last).find(|&d| src(d) >= lo).unwrap_or(last);
        let b = (first..=last).rev().find(|&d| src(d) < lo + len).unwrap_or(first - 1);
        (a, b + 1)
    };
    let (x0, x1) = range(full.x as i64, full.w as i64, w as i64, nw as i64);
    let (y0, y1) = range(full.y as i64, full.h as i64, h as i64, nh as i64);
    if x1 <= x0 || y1 <= y0 || x0 < i32::MIN as i64 / 2 || y0 < i32::MIN as i64 / 2 {
        return;
    }
    let (rw, rh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if rw.saturating_mul(rh) > crate::document::offcanvas::MAX_OFFCANVAS_PIXELS {
        return;
    }
    let sample = |d: i64, from: i64, to: i64| ((2 * d + 1) * from).div_euclid(2 * to);
    let mut out = vec![0u8; rw * rh * 4];
    for dy in 0..rh {
        let sy = sample(y0 + dy as i64, h as i64, nh as i64) - full.y as i64;
        if sy < 0 || sy >= full.h as i64 {
            continue;
        }
        for dx in 0..rw {
            let sx = sample(x0 + dx as i64, w as i64, nw as i64) - full.x as i64;
            if sx < 0 || sx >= full.w as i64 {
                continue;
            }
            let s = (sy as usize * full.w + sx as usize) * 4;
            out[(dy * rw + dx) * 4..][..4].copy_from_slice(&full.rgba[s..s + 4]);
        }
    }
    let _ = dst.set_full(nw, nh, (x0 as i32, y0 as i32, rw, rh), &out);
}

// ---- C ABI -------------------------------------------------------------------------------

unsafe fn params<'a>(p: *const i32, n: usize) -> &'a [i32] {
    if p.is_null() || n == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(p, n)
    }
}

fn rect_arg(x: i32, y: i32, w: u32, h: u32) -> Option<Rect> {
    (w > 0).then_some(Rect { x, y, w, h })
}

/// Applies a filter as one undo step. `kind` and `p` are described in `pixmith_filters.h`;
/// `target` is 0 cel, 1 all frames of `layer`, 2 all layers of `frame`; `w == 0` means the
/// whole canvas. Returns true if pixels changed (otherwise nothing is recorded in the history).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_filter_apply(
    doc: *mut Editor,
    kind: u32,
    target: u32,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    p: *const i32,
    n: usize,
) -> bool {
    crate::ffi::guarded("pixmith_filter_apply", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return false };
        let (Some(filter), Some(target)) = (Filter::from_params(kind, params(p, n)), Target::from_u32(target)) else {
            return false;
        };
        let mut next = e.sprite.clone();
        if !apply(&mut next, &filter, target, layer as usize, frame as usize, rect_arg(x, y, w, h)) {
            return false;
        }
        if let Some(mask) = e.sel.active(next.width, next.height) {
            if !restrict_to_mask(&mut next, &e.sprite, mask) {
                return false;
            }
        }
        crate::document::tilemap::reconcile(&mut next);
        e.checkpoint();
        e.sprite = next;
        true
    })
}

/// Renders `frame` as it would look after the filter, without touching the document or history.
/// `out` must hold width * height * 4 bytes.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_filter_preview(
    doc: *const Editor,
    kind: u32,
    target: u32,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    p: *const i32,
    n: usize,
    out: *mut u8,
    len: usize,
) -> bool {
    crate::ffi::guarded("pixmith_filter_preview", std::ptr::null_mut(), || {
        let Some(e) = doc.as_ref() else { return false };
        let (Some(filter), Some(target)) = (Filter::from_params(kind, params(p, n)), Target::from_u32(target)) else {
            return false;
        };
        // Only the displayed frame matters, so "all frames" previews just that frame.
        let target = if target == Target::AllFrames { Target::Cel } else { target };
        let mut s = e.sprite.clone();
        apply(&mut s, &filter, target, layer as usize, frame as usize, rect_arg(x, y, w, h));
        if let Some(mask) = e.sel.active(s.width, s.height) {
            restrict_to_mask(&mut s, &e.sprite, mask);
        }
        let px = s.render(frame as usize);
        if out.is_null() || len < px.len() {
            return false;
        }
        std::ptr::copy_nonoverlapping(px.as_ptr(), out, px.len());
        true
    })
}

/// Writes the lookup table of one curve (`n` points as x, y pairs) to `out` (256 bytes).
#[no_mangle]
pub unsafe extern "C" fn pixmith_curve_lut(xy: *const i32, n: usize, out: *mut u8) -> bool {
    crate::ffi::guarded("pixmith_curve_lut", std::ptr::null_mut(), || {
        if out.is_null() || n > MAX_CURVE_POINTS || (xy.is_null() && n > 0) {
            return false;
        }
        let raw = if n == 0 { &[][..] } else { std::slice::from_raw_parts(xy, 2 * n) };
        let pts: Vec<(u8, u8)> =
            raw.chunks_exact(2).map(|c| (c[0].clamp(0, 255) as u8, c[1].clamp(0, 255) as u8)).collect();
        std::ptr::copy_nonoverlapping(curve_lut(&pts).as_ptr(), out, 256);
        true
    })
}

/// Resizes the whole sprite with nearest-neighbor sampling as one undo step.
#[no_mangle]
pub unsafe extern "C" fn pixmith_scale_sprite(doc: *mut Editor, width: u32, height: u32) -> bool {
    crate::ffi::guarded("pixmith_scale_sprite", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return false };
        let mut next = e.sprite.clone();
        if !scale_sprite(&mut next, width, height) {
            return false;
        }
        e.checkpoint();
        e.sprite = next;
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const CLEAR: [u8; 4] = [0; 4];

    /// Builds a one-layer, one-frame sprite from pixels in row-major order.
    fn sprite(w: u32, h: u32, px: &[[u8; 4]]) -> Sprite {
        let mut s = Sprite::new(w, h);
        s.cels[0][0] = Some(Arc::new(Cel { opacity: 255, pixels: px.concat(), ..Default::default() }));
        s
    }

    fn pixels(s: &Sprite) -> Vec<[u8; 4]> {
        s.cels[0][0].as_ref().unwrap().pixels.chunks(4).map(|c| c.try_into().unwrap()).collect()
    }

    fn run(s: &mut Sprite, f: Filter) -> bool {
        apply(s, &f, Target::Cel, 0, 0, None)
    }

    #[test]
    fn replace_color_exact_and_tolerance() {
        let mut s = sprite(3, 1, &[RED, [250, 5, 0, 255], BLUE]);
        run(&mut s, Filter::ReplaceColor { from: RED, to: GREEN, tolerance: 0 });
        assert_eq!(pixels(&s), vec![GREEN, [250, 5, 0, 255], BLUE]);
        let mut s = sprite(3, 1, &[RED, [250, 5, 0, 255], BLUE]);
        run(&mut s, Filter::ReplaceColor { from: RED, to: GREEN, tolerance: 5 });
        assert_eq!(pixels(&s), vec![GREEN, GREEN, BLUE]);
    }

    #[test]
    fn replace_transparent_matches_any_clear_pixel() {
        let mut s = sprite(2, 1, &[[9, 9, 9, 0], RED]);
        run(&mut s, Filter::ReplaceColor { from: CLEAR, to: BLUE, tolerance: 0 });
        assert_eq!(pixels(&s), vec![BLUE, RED]);
    }

    #[test]
    fn filter_on_a_linked_cel_changes_every_linked_frame_once() {
        let mut s = sprite(1, 1, &[RED]);
        s.add_frame(0, false);
        s.add_frame(1, false);
        assert!(s.link_cels(0, 0, 2));
        // Filtering the middle frame changes the shared cel, so all three frames change and stay linked.
        assert!(apply(&mut s, &Filter::Invert, Target::Cel, 0, 1, None));
        for f in 0..3 {
            assert_eq!(s.render(f), vec![0, 255, 255, 255], "frame {f}");
        }
        assert!(s.is_cel_linked(0, 0) && s.is_cel_linked(0, 2));
        // All frames: the shared cel is filtered once (twice would give back the original red only if run twice).
        assert!(apply(&mut s, &Filter::Invert, Target::AllFrames, 0, 0, None));
        for f in 0..3 {
            assert_eq!(s.render(f), RED.to_vec(), "frame {f}");
        }
        assert!(s.is_cel_linked(0, 1));
    }

    #[test]
    fn invert_keeps_alpha() {
        let mut s = sprite(2, 1, &[[10, 20, 30, 128], CLEAR]);
        run(&mut s, Filter::Invert);
        assert_eq!(pixels(&s), vec![[245, 235, 225, 128], [255, 255, 255, 0]]);
    }

    #[test]
    fn desaturate_uses_luma() {
        let mut s = sprite(3, 1, &[RED, GREEN, [100, 100, 100, 200]]);
        run(&mut s, Filter::Desaturate);
        assert_eq!(pixels(&s), vec![[76, 76, 76, 255], [150, 150, 150, 255], [100, 100, 100, 200]]);
    }

    #[test]
    fn hsl_hue_saturation_lightness() {
        let mut s = sprite(1, 1, &[RED]);
        run(&mut s, Filter::Hsl { hue: 120, saturation: 0, lightness: 0 });
        assert_eq!(pixels(&s), vec![GREEN]);
        run(&mut s, Filter::Hsl { hue: 120, saturation: 0, lightness: 0 });
        assert_eq!(pixels(&s), vec![BLUE]);
        let mut s = sprite(1, 1, &[RED]);
        run(&mut s, Filter::Hsl { hue: 0, saturation: -100, lightness: 0 });
        assert_eq!(pixels(&s), vec![[128, 128, 128, 255]]);
        let mut s = sprite(1, 1, &[[100, 50, 50, 255]]);
        run(&mut s, Filter::Hsl { hue: 0, saturation: 0, lightness: 100 });
        assert_eq!(pixels(&s), vec![[255, 255, 255, 255]]);
        let mut s = sprite(1, 1, &[[100, 50, 50, 255]]);
        run(&mut s, Filter::Hsl { hue: 0, saturation: 0, lightness: -100 });
        assert_eq!(pixels(&s), vec![[0, 0, 0, 255]]);
    }

    #[test]
    fn hsl_identity_is_lossless() {
        let src = [[12, 200, 77, 255], [255, 255, 0, 90], [3, 3, 3, 255]];
        let mut s = sprite(3, 1, &src);
        assert!(!run(&mut s, Filter::Hsl { hue: 0, saturation: 0, lightness: 0 }));
        assert_eq!(pixels(&s), src.to_vec());
    }

    #[test]
    fn brightness_and_contrast() {
        let mut s = sprite(1, 1, &[[100, 128, 200, 255]]);
        run(&mut s, Filter::BrightnessContrast { brightness: 20, contrast: 0 });
        assert_eq!(pixels(&s), vec![[151, 179, 251, 255]]);
        let mut s = sprite(1, 1, &[[100, 128, 200, 77]]);
        assert!(!run(&mut s, Filter::BrightnessContrast { brightness: 0, contrast: 0 }));
        run(&mut s, Filter::BrightnessContrast { brightness: 0, contrast: 100 });
        // Factor 129.5 pushes everything off 128 to the extremes; 128 itself is fixed.
        assert_eq!(pixels(&s), vec![[0, 128, 255, 77]]);
        run(&mut s, Filter::BrightnessContrast { brightness: 100, contrast: 0 });
        assert_eq!(pixels(&s), vec![[255, 255, 255, 77]]);
    }

    fn ramp(lut: &[u8; 256]) -> Vec<u8> {
        lut.to_vec()
    }

    #[test]
    fn curve_lut_identity_and_degenerate_inputs() {
        let id: Vec<u8> = (0..=255).collect();
        assert_eq!(ramp(&curve_lut(&[(0, 0), (255, 255)])), id);
        assert_eq!(ramp(&curve_lut(&[])), id);
        assert_eq!(ramp(&curve_lut(&[(10, 200)])), id);
        // Collinear interior points do not bend the line.
        assert_eq!(ramp(&curve_lut(&[(0, 0), (64, 64), (128, 128), (255, 255)])), id);
    }

    #[test]
    fn curve_lut_interpolates_and_clamps() {
        let inv = curve_lut(&[(0, 255), (255, 0)]);
        assert_eq!((inv[0], inv[255], inv[128]), (255, 0, 127));
        // Flat outside the points.
        let l = curve_lut(&[(100, 50), (200, 150)]);
        assert!(l[..=100].iter().all(|&v| v == 50));
        assert!(l[200..].iter().all(|&v| v == 150));
        assert_eq!(l[150], 100);
        // Unsorted points and a repeated x (the later point wins).
        let a = curve_lut(&[(255, 255), (0, 0), (128, 20), (128, 200)]);
        assert_eq!(a[128], 200);
    }

    #[test]
    fn curve_lut_is_monotone_and_never_overshoots() {
        let l = curve_lut(&[(0, 0), (60, 10), (70, 240), (255, 255)]);
        assert!(l.windows(2).all(|w| w[0] <= w[1]), "{l:?}");
        assert_eq!((l[0], l[60], l[70], l[255]), (0, 10, 240, 255));
        let flat = curve_lut(&[(0, 100), (100, 100), (255, 200)]);
        assert!(flat[..=100].iter().all(|&v| v == 100));
        assert!(flat.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn curves_filter_uses_master_then_channel() {
        // master: invert; red channel: raise everything to at least 100 after the master.
        let p = [2, 0, 255, 255, 0, 2, 0, 100, 255, 255, 0, 0, 0, 0];
        let f = Filter::from_params(KIND_CURVES, &p).unwrap();
        let mut s = sprite(1, 1, &[[255, 255, 0, 200]]);
        assert!(apply(&mut s, &f, Target::Cel, 0, 0, None));
        // R: master(255)=0, red curve(0)=100. G: master(255)=0. B: master(0)=255. Alpha untouched.
        assert_eq!(pixels(&s), vec![[100, 0, 255, 200]]);
    }

    #[test]
    fn curves_alpha_and_identity_and_bad_params() {
        let id = [2, 0, 0, 255, 255];
        let mut p = id.to_vec();
        for _ in 0..3 {
            p.extend_from_slice(&id);
        }
        p.extend_from_slice(&[2, 0, 0, 255, 128]); // alpha halves
        let f = Filter::from_params(KIND_CURVES, &p).unwrap();
        let mut s = sprite(1, 1, &[[10, 20, 30, 255]]);
        assert!(apply(&mut s, &f, Target::Cel, 0, 0, None));
        assert_eq!(pixels(&s), vec![[10, 20, 30, 128]]);
        // All identity: nothing changes.
        let mut ident = id.to_vec();
        for _ in 0..4 {
            ident.extend_from_slice(&id);
        }
        let f = Filter::from_params(KIND_CURVES, &ident).unwrap();
        let mut s = sprite(1, 1, &[[10, 20, 30, 40]]);
        assert!(!apply(&mut s, &f, Target::Cel, 0, 0, None));
        // Truncated, negative and oversized counts are rejected.
        assert!(Filter::from_params(KIND_CURVES, &ident[..ident.len() - 1]).is_none());
        assert!(Filter::from_params(KIND_CURVES, &[-1]).is_none());
        assert!(Filter::from_params(KIND_CURVES, &[1000]).is_none());
    }

    #[test]
    fn curves_respect_target_and_region() {
        let inv = [2, 0, 255, 255, 0];
        let mut p = inv.to_vec();
        for _ in 0..3 {
            p.extend_from_slice(&[0]);
        }
        p.extend_from_slice(&[0]);
        let f = Filter::from_params(KIND_CURVES, &p).unwrap();
        let mut s = sprite(2, 1, &[[0, 0, 0, 255], [0, 0, 0, 255]]);
        let r = Some(Rect { x: 1, y: 0, w: 1, h: 1 });
        assert!(apply(&mut s, &f, Target::Cel, 0, 0, r));
        assert_eq!(pixels(&s), vec![[0, 0, 0, 255], [255, 255, 255, 255]]);
    }

    #[test]
    fn ffi_curve_lut_and_preview_stay_pure() {
        let mut out = [0u8; 256];
        unsafe {
            assert!(pixmith_curve_lut([0, 255, 255, 0].as_ptr(), 2, out.as_mut_ptr()));
            assert_eq!((out[0], out[255]), (255, 0));
            assert!(pixmith_curve_lut(std::ptr::null(), 0, out.as_mut_ptr()));
            assert_eq!((out[0], out[255]), (0, 255));
            assert!(!pixmith_curve_lut(std::ptr::null(), 3, out.as_mut_ptr()));
            let mut e = Editor::new(sprite(1, 1, &[[0, 0, 0, 255]]));
            let p = [2, 0, 255, 255, 0, 0, 0, 0, 0];
            let mut buf = [0u8; 4];
            assert!(pixmith_filter_preview(
                &e,
                KIND_CURVES,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                p.as_ptr(),
                p.len(),
                buf.as_mut_ptr(),
                4
            ));
            assert_eq!(buf, [255, 255, 255, 255]);
            assert!(!e.undo());
            assert!(pixmith_filter_apply(&mut e, KIND_CURVES, 0, 0, 0, 0, 0, 0, 0, p.as_ptr(), p.len()));
            assert!(e.undo());
            assert!(!e.undo());
        }
    }

    #[test]
    fn channel_offsets_saturate() {
        let mut s = sprite(1, 1, &[[250, 5, 100, 255]]);
        run(&mut s, Filter::ChannelOffset { r: 10, g: -10, b: 20 });
        assert_eq!(pixels(&s), vec![[255, 0, 120, 255]]);
    }

    /// 3x3 canvas with one red pixel in the middle.
    fn dot() -> Sprite {
        let mut px = [CLEAR; 9];
        px[4] = RED;
        sprite(3, 3, &px)
    }

    #[test]
    fn outline_outside_four_and_eight() {
        let mut s = dot();
        run(&mut s, Filter::Outline { color: BLUE, inside: false, eight: false });
        let p = pixels(&s);
        let blue: Vec<usize> = (0..9).filter(|&i| p[i] == BLUE).collect();
        assert_eq!(blue, vec![1, 3, 5, 7]);
        assert_eq!(p[4], RED);

        let mut s = dot();
        run(&mut s, Filter::Outline { color: BLUE, inside: false, eight: true });
        let p = pixels(&s);
        assert_eq!((0..9).filter(|&i| p[i] == BLUE).count(), 8);
        assert_eq!(p[4], RED);
    }

    #[test]
    fn outline_inside_treats_canvas_edge_as_empty() {
        // 3x3 solid red: only the center has all neighbors opaque and inside the canvas.
        let mut s = sprite(3, 3, &[RED; 9]);
        run(&mut s, Filter::Outline { color: BLUE, inside: true, eight: false });
        let p = pixels(&s);
        assert_eq!(p[4], RED);
        assert!((0..9).filter(|&i| i != 4).all(|i| p[i] == BLUE));

        // 5x5 canvas with a 3x3 block: the ring is recolored; with 8-connectivity too.
        let mut px = [CLEAR; 25];
        for y in 1..4 {
            for x in 1..4 {
                px[y * 5 + x] = RED;
            }
        }
        let mut s = sprite(5, 5, &px);
        run(&mut s, Filter::Outline { color: BLUE, inside: true, eight: true });
        let p = pixels(&s);
        assert_eq!(p[12], RED);
        assert_eq!(p.iter().filter(|&&c| c == BLUE).count(), 8);
        assert_eq!(p[0], CLEAR);
    }

    #[test]
    fn outline_four_vs_eight_differ_on_diagonals() {
        // Diagonal neighbor only matters for 8-connectivity.
        let mut px = [CLEAR; 9];
        px[0] = RED;
        let mut s4 = sprite(3, 3, &px);
        let mut s8 = s4.clone();
        run(&mut s4, Filter::Outline { color: BLUE, inside: false, eight: false });
        run(&mut s8, Filter::Outline { color: BLUE, inside: false, eight: true });
        assert_eq!(pixels(&s4)[4], CLEAR);
        assert_eq!(pixels(&s8)[4], BLUE);
    }

    #[test]
    fn drop_shadow_goes_under_the_image() {
        let mut s = dot();
        run(&mut s, Filter::DropShadow { dx: 1, dy: 1, color: [0, 0, 0, 255] });
        let p = pixels(&s);
        assert_eq!(p[4], RED);
        assert_eq!(p[8], [0, 0, 0, 255]);
        assert_eq!(p.iter().filter(|c| c[3] > 0).count(), 2);

        let mut s = dot();
        run(&mut s, Filter::DropShadow { dx: -1, dy: 0, color: [0, 0, 0, 128] });
        assert_eq!(pixels(&s)[3], [0, 0, 0, 128]);
    }

    #[test]
    fn drop_shadow_shows_through_semi_transparent_pixels() {
        let mut s = sprite(2, 1, &[[255, 0, 0, 128], CLEAR]);
        run(&mut s, Filter::DropShadow { dx: 1, dy: 0, color: BLUE });
        let p = pixels(&s);
        assert_eq!(p[0], [255, 0, 0, 128]);
        assert_eq!(p[1], [0, 0, 255, 128]);
    }

    #[test]
    fn despeckle_removes_isolated_pixels_only() {
        let mut px = [CLEAR; 25];
        px[0] = RED; // isolated
        px[2 * 5 + 2] = RED; // pair (2,2)-(3,2)
        px[2 * 5 + 3] = RED;
        px[4 * 5 + 4] = RED; // isolated
        let mut s = sprite(5, 5, &px);
        run(&mut s, Filter::Despeckle { eight: false });
        let p = pixels(&s);
        assert_eq!(p[0], CLEAR);
        assert_eq!(p[24], CLEAR);
        assert_eq!(p[12], RED);
        assert_eq!(p[13], RED);
    }

    #[test]
    fn despeckle_connectivity_modes() {
        let mut px = [CLEAR; 9];
        px[0] = RED;
        px[4] = RED; // diagonal pair
        let mut s4 = sprite(3, 3, &px);
        let mut s8 = s4.clone();
        run(&mut s4, Filter::Despeckle { eight: false });
        run(&mut s8, Filter::Despeckle { eight: true });
        assert!(pixels(&s4).iter().all(|c| *c == CLEAR));
        assert_eq!(pixels(&s8)[0], RED);
        assert_eq!(pixels(&s8)[4], RED);
    }

    #[test]
    fn blur_spreads_a_dot_and_preserves_flat_areas() {
        let mut px = [CLEAR; 9];
        px[4] = [200, 100, 0, 255];
        let mut s = sprite(3, 3, &px);
        run(&mut s, Filter::Convolve(Kernel::Blur));
        let p = pixels(&s);
        // Center keeps 4/16 of the alpha; edge neighbors 2/16; corners 1/16. Color is unchanged
        // because transparent pixels do not contribute.
        assert_eq!(p[4], [200, 100, 0, 64]);
        assert_eq!(p[1], [200, 100, 0, 32]);
        assert_eq!(p[0], [200, 100, 0, 16]);

        let mut s = sprite(3, 3, &[[10, 200, 30, 255]; 9]);
        assert!(!run(&mut s, Filter::Convolve(Kernel::Blur)));
    }

    #[test]
    fn blur_mixes_adjacent_opaque_colors() {
        let mut s = sprite(3, 1, &[[0, 0, 0, 255], [255, 255, 255, 255], [0, 0, 0, 255]]);
        run(&mut s, Filter::Convolve(Kernel::Blur));
        let p = pixels(&s);
        // Vertical edge replication leaves only the horizontal 1/4, 1/2, 1/4 weights.
        assert_eq!(p[1], [128, 128, 128, 255]);
        // Left sees 3/4 black (itself plus replicated edge) and 1/4 white.
        assert_eq!(p[0], [64, 64, 64, 255]);
    }

    #[test]
    fn sharpen_increases_contrast_and_keeps_flat_areas() {
        let mut s = sprite(3, 3, &[[100, 100, 100, 255]; 9]);
        assert!(!run(&mut s, Filter::Convolve(Kernel::Sharpen)));
        let mut px = [[100, 100, 100, 255]; 9];
        px[4] = [120, 120, 120, 255];
        let mut s = sprite(3, 3, &px);
        run(&mut s, Filter::Convolve(Kernel::Sharpen));
        // 5*120 - 4*100 = 200.
        assert_eq!(pixels(&s)[4], [200, 200, 200, 255]);
        // Edge neighbor: 5*100 - 120 - 100 - 100 - 100 (replicated edge) = 80.
        assert_eq!(pixels(&s)[1], [80, 80, 80, 255]);
    }

    #[test]
    fn rect_limits_where_filter_writes() {
        let mut s = sprite(4, 1, &[RED; 4]);
        let r = Rect { x: 1, y: 0, w: 2, h: 1 };
        assert!(apply(&mut s, &Filter::Invert, Target::Cel, 0, 0, Some(r)));
        let inv = [0, 255, 255, 255];
        assert_eq!(pixels(&s), vec![RED, inv, inv, RED]);
        // Rects partly outside the canvas are clipped; w == 0 means everything.
        let mut s = sprite(2, 1, &[RED; 2]);
        let r = Rect { x: -5, y: -5, w: 6, h: 6 };
        apply(&mut s, &Filter::Invert, Target::Cel, 0, 0, Some(r));
        assert_eq!(pixels(&s), vec![[0, 255, 255, 255], RED]);
        let mut s = sprite(2, 1, &[RED; 2]);
        apply(&mut s, &Filter::Invert, Target::Cel, 0, 0, Some(Rect { x: 1, y: 1, w: 0, h: 0 }));
        assert_eq!(pixels(&s), vec![[0, 255, 255, 255]; 2]);
    }

    #[test]
    fn rect_uses_neighbors_outside_it() {
        // Outline restricted to the right half still sees the pixel on the left.
        let mut s = sprite(3, 1, &[RED, CLEAR, CLEAR]);
        let r = Rect { x: 1, y: 0, w: 2, h: 1 };
        apply(&mut s, &Filter::Outline { color: BLUE, inside: false, eight: false }, Target::Cel, 0, 0, Some(r));
        assert_eq!(pixels(&s), vec![RED, BLUE, CLEAR]);
    }

    #[test]
    fn targets_select_cels() {
        let mut s = Sprite::new(1, 1);
        s.add_frame(0, false);
        let top = s.add_layer(0);
        for f in 0..2 {
            for l in 0..2 {
                s.cels[f][l] = Some(Arc::new(Cel { opacity: 255, pixels: RED.to_vec(), ..Default::default() }));
            }
        }
        let px = |s: &Sprite, f: usize, l: usize| s.cels[f][l].as_ref().unwrap().pixels.clone();
        let inv = vec![0, 255, 255, 255];

        let mut a = s.clone();
        assert!(apply(&mut a, &Filter::Invert, Target::Cel, top, 1, None));
        assert_eq!((px(&a, 1, top), px(&a, 0, top), px(&a, 1, 0)), (inv.clone(), RED.to_vec(), RED.to_vec()));

        let mut a = s.clone();
        apply(&mut a, &Filter::Invert, Target::AllFrames, 0, 1, None);
        assert_eq!((px(&a, 0, 0), px(&a, 1, 0), px(&a, 0, top)), (inv.clone(), inv.clone(), RED.to_vec()));

        let mut a = s.clone();
        apply(&mut a, &Filter::Invert, Target::AllLayers, 0, 1, None);
        assert_eq!((px(&a, 1, 0), px(&a, 1, top), px(&a, 0, 0)), (inv.clone(), inv, RED.to_vec()));
    }

    #[test]
    fn missing_cels_and_groups_are_skipped() {
        let mut s = Sprite::new(1, 1);
        assert!(!apply(&mut s, &Filter::Invert, Target::Cel, 0, 0, None));
        assert!(s.cels[0][0].is_none());
        s.layers[0].kind = 1;
        s.cels[0][0] = Some(Arc::new(Cel { opacity: 255, pixels: RED.to_vec(), ..Default::default() }));
        assert!(!apply(&mut s, &Filter::Invert, Target::Cel, 0, 0, None));
        assert!(!apply(&mut s, &Filter::Invert, Target::Cel, 9, 9, None));
    }

    #[test]
    fn scale_up_and_down_nearest() {
        let mut s = sprite(2, 2, &[RED, GREEN, BLUE, CLEAR]);
        assert!(scale_sprite(&mut s, 4, 4));
        assert_eq!((s.width, s.height), (4, 4));
        let p = pixels(&s);
        assert_eq!(&p[0..4], &[RED, RED, GREEN, GREEN]);
        assert_eq!(&p[4..8], &[RED, RED, GREEN, GREEN]);
        assert_eq!(&p[8..12], &[BLUE, BLUE, CLEAR, CLEAR]);

        assert!(scale_sprite(&mut s, 2, 2));
        assert_eq!(pixels(&s), vec![RED, GREEN, BLUE, CLEAR]);

        // Non-uniform: 2x1 -> 3x1 samples centers 1/6, 3/6, 5/6 => src 0, 1, 1.
        let mut s = sprite(2, 1, &[RED, GREEN]);
        scale_sprite(&mut s, 3, 1);
        assert_eq!(pixels(&s), vec![RED, GREEN, GREEN]);
    }

    #[test]
    fn scale_covers_all_frames_and_layers() {
        let mut s = Sprite::new(1, 1);
        s.add_frame(0, false);
        s.add_layer(0);
        for f in 0..2 {
            for l in 0..2 {
                s.cels[f][l] = Some(Arc::new(Cel { opacity: 77, pixels: RED.to_vec(), ..Default::default() }));
            }
        }
        s.cels[1][1] = None;
        assert!(scale_sprite(&mut s, 3, 2));
        for (f, row) in s.cels.iter().enumerate() {
            for (l, cel) in row.iter().enumerate() {
                match cel {
                    Some(c) => assert_eq!((c.pixels.len(), c.opacity), (24, 77), "{f},{l}"),
                    None => assert_eq!((f, l), (1, 1)),
                }
            }
        }
        assert_eq!(s.render(0).len(), 24);
    }

    #[test]
    fn scale_keeps_linked_cels_shared() {
        let mut s = sprite(2, 1, &[RED, GREEN]);
        s.add_frame(0, false);
        assert!(s.link_cels(0, 0, 1));
        assert!(Arc::ptr_eq(s.cels[0][0].as_ref().unwrap(), s.cels[1][0].as_ref().unwrap()));
        assert!(scale_sprite(&mut s, 4, 2));
        assert!(Arc::ptr_eq(s.cels[0][0].as_ref().unwrap(), s.cels[1][0].as_ref().unwrap()));
        assert_eq!(s.distinct_cel_count(), 1);
    }

    #[test]
    fn scale_rejects_bad_sizes() {
        let mut s = sprite(2, 1, &[RED, GREEN]);
        assert!(!scale_sprite(&mut s, 2, 1));
        assert!(!scale_sprite(&mut s, 0, 5));
        assert!(!scale_sprite(&mut s, 9000, 1));
        assert_eq!((s.width, s.height), (2, 1));
    }

    #[test]
    fn from_params_validates() {
        assert!(Filter::from_params(KIND_INVERT, &[]).is_some());
        assert!(Filter::from_params(KIND_HSL, &[1, 2]).is_none());
        assert!(Filter::from_params(99, &[]).is_none());
        assert_eq!(
            Filter::from_params(KIND_OUTLINE, &[0xFF0000FFu32 as i32, 1, 0]),
            Some(Filter::Outline { color: RED, inside: true, eight: false })
        );
    }

    #[test]
    fn ffi_apply_is_one_undo_step_and_preview_is_pure() {
        let mut e = Editor::new(sprite(2, 1, &[RED, GREEN]));
        e.sprite.add_frame(0, true);
        let before = e.sprite.clone();
        unsafe {
            let doc: *mut Editor = &mut e;
            let mut out = vec![0u8; 8];
            assert!(pixmith_filter_preview(
                doc,
                KIND_INVERT,
                1,
                0,
                0,
                0,
                0,
                0,
                0,
                std::ptr::null(),
                0,
                out.as_mut_ptr(),
                8
            ));
            assert_eq!(&out[0..4], &[0, 255, 255, 255]);
            assert_eq!(e.sprite, before);
            assert!(!e.undo());

            assert!(pixmith_filter_apply(doc, KIND_INVERT, 1, 0, 0, 0, 0, 0, 0, std::ptr::null(), 0));
            assert_eq!(e.sprite.cels[1][0].as_ref().unwrap().pixels[0], 0);
            assert_eq!(e.sprite.cels[0][0].as_ref().unwrap().pixels[0], 0);
            assert!(e.undo());
            assert_eq!(e.sprite, before);
            assert!(!e.undo());

            // Unknown kind and no-op filters leave history alone.
            assert!(!pixmith_filter_apply(doc, 99, 0, 0, 0, 0, 0, 0, 0, std::ptr::null(), 0));
            let p = [0, 0, 0];
            assert!(!pixmith_filter_apply(doc, KIND_HSL, 0, 0, 0, 0, 0, 0, 0, p.as_ptr(), 3));
            assert!(!e.undo());

            assert!(pixmith_scale_sprite(doc, 4, 2));
            assert_eq!(e.sprite.width, 4);
            assert!(e.undo());
            assert_eq!(e.sprite, before);
            assert!(!pixmith_filter_apply(std::ptr::null_mut(), 1, 0, 0, 0, 0, 0, 0, 0, std::ptr::null(), 0));
        }
    }
}
