//! Trace "pseudo-pixel" art (AI generated or upscaled with soft edges, slight grid drift and noise) back onto a real
//! pixel grid.
//!
//! 1. **Grid.** The pitch (cell size in source pixels, fractional allowed) and phase (offset of the first cell edge)
//!    are found per axis from the edge profile: the gradient energy of every column / row is folded modulo a
//!    candidate pitch, and the pitch whose folded profile is sharpest wins. Folding is O(edges) per candidate, so the
//!    scan can use a step small enough that the drift over the whole image stays under half a pixel. Sub-multiples
//!    of the true pitch fold just as sharply, so among the near-ties the largest pitch is taken. The phase is the
//!    peak of the finest fold.
//! 2. **Cells.** Each cell becomes the per-channel median of the pixels in its central 60 % (transparent pixels are
//!    ignored), which drops soft edges and noise.
//! 3. **Background.** Optionally `paint::background` runs on the small cell image.
//! 4. **Palette.** k-means with a fixed seed (k given, or the smallest k within a range that fits), or an existing
//!    palette; every cell snaps to its nearest palette colour.
//! 5. **Outline.** Optionally one-pixel breaks in the dark outer outline are closed.
//! 6. **Crop** to the opaque cells, and a quality score (1 is a perfect grid and palette fit).
//!
//! Entry points: [`trace_grid`] (an RGBA buffer), [`trace_sprite`] (a document: the active cel defines the grid and
//! palette, every cel is resampled), `pixmith_trace_grid` (C ABI) and the `trace_grid` import mode.

use crate::document::{Cel, Editor, Sprite};
use crate::formats::json::{self, Value};
use crate::paint::background::{options_arg, parse_hex, remove_background, BgOptions, BgStats};
use std::collections::{BTreeMap, HashMap};
use std::ffi::{c_char, CString};
use std::ptr::null_mut;
use std::sync::Arc;

/// Largest source image (pixels) that is traced.
const MAX_PIXELS: usize = 1 << 26;
/// Share of a cell (each side) left out of the median: soft edges live there.
const CELL_MARGIN: f64 = 0.2;
/// A fold must concentrate the edge energy at least this many times better than a flat profile to count as a grid.
const MIN_CONCENTRATION: f64 = 1.6;
/// Soft AI sheets drift and jitter, so no single fold of the whole image lines up. Their grid is searched in
/// overlapping tiles of this many source pixels (stride half of it), each with its own phase.
const TILE: usize = 96;
const TILE_STEP: usize = 48;
/// Largest pitch the tile search covers (a tile must hold at least six cells).
const TILE_MAX_PITCH: f64 = 16.0;
/// At most this many tiles are scored (more are subsampled evenly).
const MAX_TILES: usize = 2500;
/// Tile search: the mean normalised power at the best pitch against the median over all pitches. At or above
/// `STRONG_RATIO` the grid is trusted, between `WEAK_RATIO` and it the result carries a warning, below it the
/// pitch is noise.
const STRONG_RATIO: f64 = 3.0;
const WEAK_RATIO: f64 = 1.25;
/// ... plus this over the square root of the number of tile profiles (few tiles make noise peaks taller).
const WEAK_RATIO_SPREAD: f64 = 4.0;
/// Whole-image concentration (1 is flat) at which a fold is reported as a weak grid rather than as nothing.
const WEAK_CONCENTRATION: f64 = 1.3;
/// Pitches within this share of the best score are ties; the largest of them wins (sub-multiples tie with the truth).
const TIE: f64 = 0.9;
/// Work allowed for the Lloyd iterations of one k-means run (distance evaluations).
const KMEANS_BUDGET: usize = 40_000_000;
/// Largest palette k-means is asked for.
const MAX_COLORS: usize = 256;
/// Auto palette: the smallest k whose root-mean-square colour error is below this ...
const AUTO_RMS: f64 = 6.0;
/// ... and where at most `AUTO_OUTLIERS` of the cells are farther than `AUTO_FAR` from their palette colour, so a
/// small detail (an eye highlight) is not merged into its neighbour colour.
const AUTO_FAR: f64 = 36.0;
const AUTO_OUTLIERS: f64 = 0.003;
/// Auto palette: stop when one more colour removes less than this share of the one-colour error.
const AUTO_GAIN: f64 = 0.001;

// MARK: Options and results

#[derive(Clone, Debug, PartialEq)]
pub enum Pitch {
    /// Find it between `min` and `max` source pixels per cell.
    Auto { min: f64, max: f64 },
    /// Known cell size (x, y).
    Fixed(f64, f64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Colors {
    /// Keep the cell colours as they are.
    Keep,
    /// Exactly (at most) this many colours.
    Count(usize),
    /// The smallest count within the range that fits.
    Auto { min: usize, max: usize },
    /// Snap to these colours.
    Palette(Vec<[u8; 4]>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TraceOptions {
    pub pitch: Pitch,
    /// Known offset of the first cell edge (x, y); `None` finds it.
    pub phase: Option<(f64, f64)>,
    /// Look for the pitch of each axis on its own (otherwise one pitch for both, which is steadier).
    pub separate_axes: bool,
    pub colors: Colors,
    /// Remove the background (and shadow) from the traced cells.
    pub remove_background: Option<BgOptions>,
    /// Close one-pixel breaks in the dark outer outline.
    pub close_outline: bool,
    /// Crop to the opaque cells.
    pub crop: bool,
    /// Seed of the k-means initialisation; the same seed gives the same palette.
    pub seed: u64,
}

impl Default for TraceOptions {
    fn default() -> Self {
        TraceOptions {
            pitch: Pitch::Auto { min: 3.0, max: 32.0 },
            phase: None,
            separate_axes: false,
            colors: Colors::Auto { min: 2, max: 32 },
            remove_background: None,
            close_outline: false,
            crop: true,
            seed: 1,
        }
    }
}

/// A detected grid: cell size and the offset of the first cell edge, per axis, in source pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub pitch: (f64, f64),
    pub phase: (f64, f64),
}

#[derive(Clone, Debug)]
pub struct TraceResult {
    pub width: usize,
    pub height: usize,
    /// Straight-alpha RGBA, `width * height` cells.
    pub rgba: Vec<u8>,
    pub pitch: (f64, f64),
    /// Offset of the first cell edge in the source, `0 <= phase < pitch`.
    pub phase: (f64, f64),
    pub palette: Vec<[u8; 4]>,
    /// 0 to 1: how well the cells and palette explain the source pixels.
    pub quality: f32,
    pub background: BgStats,
    /// Cell column and row of the result's top-left corner inside the full grid.
    pub origin: (usize, usize),
    /// Size of the full grid in cells before cropping.
    pub grid_cells: (usize, usize),
    pub closed_outline: usize,
    /// Set when the grid was found but is weak (soft or drifting edges): check the result or pass `pitch`.
    pub warning: Option<String>,
}

impl TraceResult {
    pub fn to_json(&self) -> Value {
        let n = |v: f64| Value::Num((v * 1000.0).round() / 1000.0);
        let hex = |c: &[u8; 4]| Value::Str(format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]));
        let mut fields = vec![
            ("width".into(), Value::Num(self.width as f64)),
            ("height".into(), Value::Num(self.height as f64)),
            ("pitch".into(), Value::Arr(vec![n(self.pitch.0), n(self.pitch.1)])),
            ("phase".into(), Value::Arr(vec![n(self.phase.0), n(self.phase.1)])),
            ("palette".into(), Value::Arr(self.palette.iter().map(hex).collect())),
            ("quality".into(), n(self.quality as f64)),
            ("origin".into(), Value::Arr(vec![Value::Num(self.origin.0 as f64), Value::Num(self.origin.1 as f64)])),
            (
                "grid_cells".into(),
                Value::Arr(vec![Value::Num(self.grid_cells.0 as f64), Value::Num(self.grid_cells.1 as f64)]),
            ),
            ("closed_outline".into(), Value::Num(self.closed_outline as f64)),
            ("background".into(), self.background.to_json()),
        ];
        if let Some(w) = &self.warning {
            fields.push(("warning".into(), Value::Str(w.clone())));
        }
        Value::Obj(fields)
    }
}

impl TraceOptions {
    /// Reads the JSON options of the C ABI and the import mode. Every key is optional:
    /// `pitch` ("auto", a number, or [x, y]), `min_pitch`, `max_pitch`, `phase` ([x, y]), `separate_axes`,
    /// `colors` (a count, "auto", "keep", or an array of "#rrggbb"), `min_colors`, `max_colors`,
    /// `remove_background` (bool), the keys of [`BgOptions::from_json`] (`tolerance`, `shadow`, `global`,
    /// `shadow_depth`, `color`), `close_outline`, `crop`, `seed`.
    pub fn from_json(v: &Value) -> Result<TraceOptions, String> {
        let mut o = TraceOptions::default();
        let num = |k: &str| v.get(k).and_then(Value::as_f64);
        let flag = |k: &str| match v.get(k) {
            Some(Value::Bool(b)) => Some(*b),
            _ => None,
        };
        let (min, max) = (num("min_pitch").unwrap_or(3.0), num("max_pitch").unwrap_or(32.0));
        o.pitch = match v.get("pitch") {
            None | Some(Value::Null) => Pitch::Auto { min, max },
            Some(Value::Str(s)) if s == "auto" => Pitch::Auto { min, max },
            Some(Value::Num(n)) => Pitch::Fixed(*n, *n),
            Some(Value::Arr(a)) if a.len() == 2 => match (a[0].as_f64(), a[1].as_f64()) {
                (Some(x), Some(y)) => Pitch::Fixed(x, y),
                _ => return Err("pitch must be \"auto\", a number or [x, y]".into()),
            },
            _ => return Err("pitch must be \"auto\", a number or [x, y]".into()),
        };
        if let Some(Value::Arr(a)) = v.get("phase") {
            match (a.first().and_then(Value::as_f64), a.get(1).and_then(Value::as_f64)) {
                (Some(x), Some(y)) => o.phase = Some((x, y)),
                _ => return Err("phase must be [x, y]".into()),
            }
        }
        o.separate_axes = flag("separate_axes").unwrap_or(false);
        let (cmin, cmax) = (num("min_colors").unwrap_or(2.0) as usize, num("max_colors").unwrap_or(32.0) as usize);
        o.colors = match v.get("colors") {
            None | Some(Value::Null) => Colors::Auto { min: cmin, max: cmax },
            Some(Value::Str(s)) if s == "auto" => Colors::Auto { min: cmin, max: cmax },
            Some(Value::Str(s)) if s == "keep" => Colors::Keep,
            Some(Value::Num(n)) if *n >= 1.0 => Colors::Count(*n as usize),
            Some(Value::Arr(a)) => {
                let mut pal = Vec::new();
                for c in a {
                    let hex = c.as_str().and_then(parse_hex).ok_or("colors must be \"#rrggbb\" strings")?;
                    pal.push([hex[0], hex[1], hex[2], 255]);
                }
                Colors::Palette(pal)
            }
            _ => return Err("colors must be a count, \"auto\", \"keep\" or an array of \"#rrggbb\"".into()),
        };
        if flag("remove_background").unwrap_or(false) {
            o.remove_background = Some(BgOptions::from_json(v)?);
        }
        o.close_outline = flag("close_outline").unwrap_or(false);
        o.crop = flag("crop").unwrap_or(true);
        if let Some(s) = num("seed") {
            o.seed = s as u64;
        }
        o.validate()?;
        Ok(o)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.phase.is_some_and(|(x, y)| !x.is_finite() || !y.is_finite()) {
            return Err("the phase must be finite numbers".into());
        }
        match self.pitch {
            Pitch::Auto { min, max } if !(min >= 2.0 && max >= min && max <= 512.0) => {
                return Err("the pitch range must satisfy 2 <= min <= max <= 512".into())
            }
            Pitch::Fixed(x, y) if !(x >= 1.0 && y >= 1.0 && x <= 4096.0 && y <= 4096.0) => {
                return Err("the pitch must be between 1 and 4096".into())
            }
            _ => {}
        }
        match &self.colors {
            Colors::Count(k) if *k == 0 || *k > MAX_COLORS => Err(format!("colors must be between 1 and {MAX_COLORS}")),
            Colors::Auto { min, max } if *min == 0 || max < min || *max > MAX_COLORS => {
                Err(format!("the color range must satisfy 1 <= min <= max <= {MAX_COLORS}"))
            }
            Colors::Palette(p) if p.is_empty() || p.len() > MAX_COLORS => {
                Err(format!("the palette needs 1 to {MAX_COLORS} colors"))
            }
            _ => Ok(()),
        }
    }
}

// MARK: Grid detection

/// Premultiplied copy: transparent pixels do not contribute their (arbitrary) colour.
fn premultiplied(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for p in rgba.chunks_exact(4) {
        let a = p[3] as u32;
        out.extend_from_slice(&[
            ((p[0] as u32 * a + 127) / 255) as u8,
            ((p[1] as u32 * a + 127) / 255) as u8,
            ((p[2] as u32 * a + 127) / 255) as u8,
            p[3],
        ]);
    }
    out
}

/// Sparse edge profile of one axis: `(edge position, energy)` where edge `e` lies between pixel `e - 1` and `e`.
struct Profile {
    edges: Vec<(u32, f32)>,
    total: f64,
    len: usize,
}

/// Edge profiles of overlapping tiles: `x` holds the column profile of every tile, `y` the row profile (both with the
/// noise floor subtracted). Tiles with almost no edges are left out.
#[derive(Default)]
struct Tiles {
    x: Vec<Vec<f32>>,
    y: Vec<Vec<f32>>,
}

fn profiles(rgba: &[u8], w: usize, h: usize) -> (Profile, Profile, Tiles) {
    let q = premultiplied(rgba);
    let mut dx = vec![0f32; w + 1];
    let mut dy = vec![0f32; h + 1];
    // Half-tile bands: column sums per band of rows, row sums per band of columns.
    let (bh, bw) = (h / TILE_STEP + 1, w / TILE_STEP + 1);
    let mut cs = vec![0f32; bh * w];
    let mut rs = vec![0f32; bw * h];
    let diff =
        |a: usize, b: usize| -> f32 { (0..4).map(|c| q[a * 4 + c].abs_diff(q[b * 4 + c])).max().unwrap_or(0) as f32 };
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if x > 0 {
                let d = diff(i, i - 1);
                dx[x] += d;
                cs[(y / TILE_STEP) * w + x] += d;
            }
            if y > 0 {
                let d = diff(i, i - w);
                dy[y] += d;
                rs[(x / TILE_STEP) * h + y] += d;
            }
        }
    }
    let mut tiles = Tiles::default();
    if w >= TILE && h >= TILE {
        let (nx, ny) = ((w - TILE) / TILE_STEP + 1, (h - TILE) / TILE_STEP + 1);
        let skip = (nx * ny).div_ceil(MAX_TILES).max(1);
        let floor_off = |v: &mut Vec<f32>| -> bool {
            let raw: f32 = v.iter().sum();
            if raw < (TILE * TILE) as f32 {
                return false;
            }
            let mut sorted = v[1..].to_vec();
            sorted.sort_by(f32::total_cmp);
            let floor = sorted[sorted.len() / 2];
            v.iter_mut().for_each(|e| *e = (*e - floor).max(0.0));
            v.iter().any(|&e| e > 0.0)
        };
        for ty in (0..ny).step_by(skip) {
            for tx in (0..nx).step_by(skip) {
                let (x0, y0) = (tx * TILE_STEP, ty * TILE_STEP);
                let mut col: Vec<f32> = (0..TILE)
                    .map(|i| cs[(y0 / TILE_STEP) * w + x0 + i] + cs[(y0 / TILE_STEP + 1).min(bh - 1) * w + x0 + i])
                    .collect();
                let mut row: Vec<f32> = (0..TILE)
                    .map(|i| rs[(x0 / TILE_STEP) * h + y0 + i] + rs[(x0 / TILE_STEP + 1).min(bw - 1) * h + y0 + i])
                    .collect();
                if floor_off(&mut col) {
                    tiles.x.push(col);
                }
                if floor_off(&mut row) {
                    tiles.y.push(row);
                }
            }
        }
    }
    (sparse(dx, w), sparse(dy, h), tiles)
}

/// Subtracts the noise floor (the median) and keeps the non-zero entries.
fn sparse(mut d: Vec<f32>, len: usize) -> Profile {
    let mut sorted: Vec<f32> = d[1..len.max(1)].to_vec();
    sorted.sort_by(f32::total_cmp);
    let floor = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    let mut edges = Vec::new();
    let mut total = 0.0f64;
    for (e, v) in d.iter_mut().enumerate() {
        let v = (*v - floor).max(0.0);
        if v > 0.0 && e > 0 && e < len {
            edges.push((e as u32, v));
            total += v as f64;
        }
    }
    Profile { edges, total, len }
}

/// Folds the profile modulo `p` into `bins` equal bins of the period. Each edge spreads its energy evenly over the
/// pixel it stands for (one source pixel wide), so edges on whole pixels do not pile up in a few bins.
fn fold(pr: &Profile, p: f64, bins: usize, out: &mut Vec<f64>) {
    out.clear();
    out.resize(bins, 0.0);
    let per_px = bins as f64 / p; // bins per source pixel
    for &(e, v) in &pr.edges {
        let start = ((e as f64 - 0.5) / p).rem_euclid(1.0) * bins as f64;
        let end = start + per_px;
        let share = v as f64 / per_px;
        let mut i = start.floor();
        while i < end {
            let overlap = (end.min(i + 1.0) - start.max(i)).max(0.0);
            out[(i as usize) % bins] += overlap * share;
            i += 1.0;
        }
    }
}

/// Concentration of the folded profile: sum of squared bin shares per pixel of the period (1 / p for a flat fold).
fn fold_score(pr: &Profile, p: f64, bin_width: f64, scratch: &mut Vec<f64>) -> f64 {
    if pr.total <= 0.0 {
        return 0.0;
    }
    let bins = ((p / bin_width).ceil() as usize).clamp(4, 4096);
    fold(pr, p, bins, scratch);
    let sq: f64 = scratch.iter().map(|m| m * m).sum();
    sq / (pr.total * pr.total) * (bins as f64 / p)
}

/// Samples `(pitch, score)` between `min` and `max`, with a relative step small enough that adjacent pitches drift
/// less than half a pixel apart over the whole axis.
fn scan(profiles: &[&Profile], min: f64, max: f64, bin_width: f64) -> Vec<(f64, f64)> {
    let extent = profiles.iter().map(|p| p.len).max().unwrap_or(1) as f64;
    let rel = (0.4 / extent).clamp(2e-4, 0.01);
    let mut scratch = Vec::new();
    let mut out = Vec::new();
    let mut p = min;
    while p <= max * (1.0 + 1e-9) {
        let s: f64 = profiles.iter().map(|pr| fold_score(pr, p, bin_width, &mut scratch)).sum();
        out.push((p, s));
        p *= 1.0 + rel;
    }
    out
}

/// The pitch of the best-scoring run; among runs scoring within [`TIE`] of the best, the largest pitch.
fn pick_pitch(samples: &[(f64, f64)]) -> Option<(f64, f64)> {
    let best = samples.iter().map(|s| s.1).fold(0.0, f64::max);
    if best <= 0.0 {
        return None;
    }
    let mut chosen: Option<(f64, f64)> = None;
    let mut run: Option<(f64, f64)> = None;
    for &(p, s) in samples.iter().chain(std::iter::once(&(f64::INFINITY, 0.0))) {
        if s >= TIE * best {
            if run.is_none_or(|r| s > r.1) {
                run = Some((p, s));
            }
        } else if let Some(r) = run.take() {
            if chosen.is_none_or(|c| r.0 > c.0) {
                chosen = Some(r);
            }
        }
    }
    chosen
}

/// Re-scans around a pitch with a ten times finer step and a narrower bin.
fn refine(profiles: &[&Profile], p0: f64) -> f64 {
    let extent = profiles.iter().map(|p| p.len).max().unwrap_or(1) as f64;
    let rel = (0.4 / extent).clamp(2e-4, 0.01) / 10.0;
    let mut scratch = Vec::new();
    let mut best = (p0, f64::MIN);
    for j in -25..=25 {
        let p = p0 * (1.0 + j as f64 * rel);
        let s: f64 = profiles.iter().map(|pr| fold_score(pr, p, 0.15, &mut scratch)).sum();
        if s > best.1 {
            best = (p, s);
        }
    }
    best.0
}

/// Offset of the first cell edge for a known pitch: the peak of the finely folded, smoothed profile.
fn phase_for(pr: &Profile, p: f64) -> f64 {
    if pr.total <= 0.0 {
        return 0.0;
    }
    let bins = ((p / 0.1).ceil() as usize).clamp(8, 4096);
    let mut m = Vec::new();
    fold(pr, p, bins, &mut m);
    let bw = p / bins as f64;
    let r = ((1.5 / bw).round() as i64).max(1);
    // Triangular smoothing, circular.
    let smooth: Vec<f64> = (0..bins as i64)
        .map(|i| {
            (-r..=r)
                .map(|d| m[(i + d).rem_euclid(bins as i64) as usize] * (1.0 - d.abs() as f64 / (r + 1) as f64))
                .sum()
        })
        .collect();
    let peak = (0..bins).max_by(|&a, &b| smooth[a].total_cmp(&smooth[b]).then(b.cmp(&a))).unwrap_or(0);
    // Centroid of the top lobe.
    let floor = smooth[peak] * 0.5;
    let (mut sw, mut so) = (0.0, 0.0);
    for d in -r..=r {
        let v = (smooth[(peak as i64 + d).rem_euclid(bins as i64) as usize] - floor).max(0.0);
        sw += v;
        so += v * d as f64;
    }
    let at = peak as f64 + 0.5 + if sw > 0.0 { so / sw } else { 0.0 };
    (at * bw).rem_euclid(p)
}

/// Searches the pitch in tiles: for every candidate the Fourier power of each tile's edge profile (normalised by its
/// energy, so a tile with edges at random positions scores 1 on average) is averaged. Every tile may have its own
/// phase and the grid may drift between tiles, which is how soft AI sheets behave. Returns the pitch and the ratio of
/// its score to the median score of the candidates around it.
fn tile_scan(sets: &[&Vec<Vec<f32>>], min: f64, max: f64) -> Option<(f64, f64, f64)> {
    let max = max.min(TILE_MAX_PITCH);
    if max < min {
        return None;
    }
    let n: usize = sets.iter().map(|s| s.len()).sum();
    if n == 0 {
        return None;
    }
    let rel = 0.4 / TILE as f64;
    let mut ps = Vec::new();
    let mut p = min;
    while p <= max * (1.0 + 1e-9) {
        ps.push(p);
        p *= 1.0 + rel;
    }
    let mut score = vec![0.0f64; ps.len()];
    for prof in sets.iter().flat_map(|s| s.iter()) {
        let energy: f64 = prof.iter().map(|&v| (v as f64) * (v as f64)).sum();
        if energy <= 0.0 {
            continue;
        }
        for (k, &p) in ps.iter().enumerate() {
            let th = std::f64::consts::TAU / p;
            let (rc, rs) = (th.cos(), th.sin());
            let (mut c, mut s) = ((-th / 2.0).cos(), (-th / 2.0).sin());
            let (mut re, mut im) = (0.0, 0.0);
            for &v in prof.iter() {
                re += v as f64 * c;
                im += v as f64 * s;
                (c, s) = (c * rc - s * rs, c * rs + s * rc);
            }
            score[k] += (re * re + im * im) / energy;
        }
    }
    // Each candidate is compared with the median score of the candidates within a factor 1.35 around it, so a sharp
    // peak stands out while the broad rise of the score toward large pitches (few cells per tile) does not.
    let half = (0.3 / (1.0 + rel).ln()).round() as usize;
    let mut ratios = Vec::with_capacity(ps.len());
    for k in 0..ps.len() {
        let mut near: Vec<f64> = score[k.saturating_sub(half)..(k + half + 1).min(ps.len())].to_vec();
        near.sort_by(f64::total_cmp);
        let med = near[near.len() / 2] / n as f64;
        ratios.push(if med > 0.0 { score[k] / n as f64 / med } else { 0.0 });
    }
    let samples: Vec<(f64, f64)> = ps.iter().copied().zip(ratios).collect();
    let (p, r) = pick_pitch(&samples)?;
    Some((p, r, WEAK_RATIO + WEAK_RATIO_SPREAD / (n as f64).sqrt()))
}

/// A detected grid and, when it is weak, a warning for the caller.
struct Detected {
    grid: Grid,
    warning: Option<String>,
}

/// Finds the cell size and the phase of the pseudo-pixel grid.
pub fn detect_grid(rgba: &[u8], w: usize, h: usize, pitch: &Pitch, separate_axes: bool) -> Result<Grid, String> {
    check_image(rgba, w, h)?;
    let (px, py, tiles) = profiles(rgba, w, h);
    detect_with(&px, &py, &tiles, pitch, separate_axes).map(|d| d.grid)
}

fn detect_with(px: &Profile, py: &Profile, tiles: &Tiles, pitch: &Pitch, separate: bool) -> Result<Detected, String> {
    if px.total <= 0.0 && py.total <= 0.0 {
        return Err("no pixel grid found: the image has no edges to line up".to_string());
    }
    let mut warnings: Vec<String> = Vec::new();
    let (pitch_x, pitch_y) = match *pitch {
        Pitch::Fixed(x, y) => (x, y),
        Pitch::Auto { min, max } => {
            let mut find = |set: &[&Profile], tl: &[&Vec<Vec<f32>>]| -> Result<f64, String> {
                let samples = scan(set, min, max, 0.5);
                let best = samples.iter().map(|s| s.1).fold(0.0, f64::max);
                let global = pick_pitch(&samples).map(|(p, _)| (p, best * p / set.len() as f64));
                if let Some((p, c)) = global {
                    if c >= MIN_CONCENTRATION {
                        return Ok(refine(set, p));
                    }
                }
                let tiled = tile_scan(tl, min, max);
                let weak = |p: f64, c: f64, what: &str| {
                    format!("weak pixel grid: best pitch ≈ {p:.2} ({what} {c:.2}); check the result or pass pitch")
                };
                match (tiled, global) {
                    (Some((p, r, _)), _) if r >= STRONG_RATIO => Ok(p),
                    (_, Some((p, c))) if c >= WEAK_CONCENTRATION => {
                        warnings.push(weak(p, c, "concentration"));
                        Ok(refine(set, p))
                    }
                    (Some((p, r, gate)), _) if r >= gate => {
                        warnings.push(weak(p, r, "tile contrast"));
                        Ok(p)
                    }
                    (t, g) => {
                        let p = t.map(|t| t.0).or(g.map(|g| g.0)).unwrap_or(min);
                        let c = g.map_or(0.0, |g| g.1);
                        Err(format!("grid too weak (best pitch ≈ {p:.1}, concentration {c:.2}); pass pitch"))
                    }
                }
            };
            if separate {
                (find(&[px], &[&tiles.x])?, find(&[py], &[&tiles.y])?)
            } else {
                let p = find(&[px, py], &[&tiles.x, &tiles.y])?;
                (p, p)
            }
        }
    };
    warnings.dedup();
    let grid = Grid { pitch: (pitch_x, pitch_y), phase: (phase_for(px, pitch_x), phase_for(py, pitch_y)) };
    Ok(Detected { grid, warning: (!warnings.is_empty()).then(|| warnings.join("; ")) })
}

fn check_image(rgba: &[u8], w: usize, h: usize) -> Result<(), String> {
    if w == 0 || h == 0 {
        return Err("the image is empty".into());
    }
    if w.checked_mul(h).is_none_or(|n| n > MAX_PIXELS) {
        return Err("the image is too large to trace".into());
    }
    if rgba.len() != w * h * 4 {
        return Err("the pixel buffer does not match the image size".into());
    }
    Ok(())
}

// MARK: Cells

/// The cells of one axis as `[start, end)` intervals in source pixels. Cells that stick out of the image count when
/// at least half of them is inside.
fn axis_cells(len: usize, pitch: f64, phase: f64) -> Vec<(f64, f64)> {
    let phase = phase.rem_euclid(pitch);
    let mut cells = Vec::new();
    let mut k = (-phase / pitch).floor() as i64;
    loop {
        let a = phase + k as f64 * pitch;
        if a >= len as f64 {
            break;
        }
        let b = a + pitch;
        let seen = b.min(len as f64) - a.max(0.0);
        if seen >= 0.5 * pitch - 1e-9 {
            cells.push((a, b));
        }
        k += 1;
    }
    cells
}

/// Source pixels of a cell along one axis: the central part, at least one pixel.
fn inner_range(cell: (f64, f64), len: usize, pitch: f64) -> (usize, usize) {
    let m = CELL_MARGIN * pitch;
    let lo = (cell.0 + m).max(0.0);
    let hi = (cell.1 - m).min(len as f64);
    let (a, b) = ((lo - 0.5).ceil().max(0.0) as usize, (hi - 0.5).ceil().max(0.0) as usize);
    let (a, b) = (a.min(len - 1), b.min(len));
    if b > a {
        (a, b)
    } else {
        let c = (((lo + hi) / 2.0 - 0.5).round().max(0.0) as usize).min(len - 1);
        (c, c + 1)
    }
}

fn median_of(v: &mut [u8]) -> u8 {
    v.sort_unstable();
    v[v.len() / 2]
}

struct Lattice {
    xs: Vec<(f64, f64)>,
    ys: Vec<(f64, f64)>,
    px: Vec<(usize, usize)>,
    py: Vec<(usize, usize)>,
}

impl Lattice {
    fn new(w: usize, h: usize, g: &Grid) -> Result<Lattice, String> {
        let xs = axis_cells(w, g.pitch.0, g.phase.0);
        let ys = axis_cells(h, g.pitch.1, g.phase.1);
        if xs.len() < 2 || ys.len() < 2 {
            return Err("the grid has fewer than two cells: the pitch is too large for this image".into());
        }
        if xs.len() * ys.len() > crate::document::MAX_CANVAS_PIXELS {
            return Err("the traced image would be too large".into());
        }
        let px = xs.iter().map(|&c| inner_range(c, w, g.pitch.0)).collect();
        let py = ys.iter().map(|&c| inner_range(c, h, g.pitch.1)).collect();
        Ok(Lattice { xs, ys, px, py })
    }

    fn size(&self) -> (usize, usize) {
        (self.xs.len(), self.ys.len())
    }

    /// Per-channel median of the opaque pixels of each cell; a cell that is mostly transparent stays transparent.
    fn sample(&self, rgba: &[u8], w: usize) -> Vec<u8> {
        let (nx, ny) = self.size();
        let mut out = vec![0u8; nx * ny * 4];
        let mut ch: [Vec<u8>; 3] = Default::default();
        for (cy, &(y0, y1)) in self.py.iter().enumerate() {
            for (cx, &(x0, x1)) in self.px.iter().enumerate() {
                ch.iter_mut().for_each(Vec::clear);
                let mut total = 0;
                for y in y0..y1 {
                    for x in x0..x1 {
                        total += 1;
                        let p = &rgba[(y * w + x) * 4..][..4];
                        if p[3] >= 128 {
                            for c in 0..3 {
                                ch[c].push(p[c]);
                            }
                        }
                    }
                }
                if ch[0].len() * 2 >= total && !ch[0].is_empty() {
                    let o = (cy * nx + cx) * 4;
                    out[o] = median_of(&mut ch[0]);
                    out[o + 1] = median_of(&mut ch[1]);
                    out[o + 2] = median_of(&mut ch[2]);
                    out[o + 3] = 255;
                }
            }
        }
        out
    }

    /// Mean absolute difference (0 to 255) between the opaque source pixels inside cells and their cell colour.
    fn fit_error(&self, rgba: &[u8], w: usize, cells: &[u8]) -> f64 {
        let nx = self.xs.len();
        let (mut sum, mut n) = (0u64, 0u64);
        for (cy, &(y0, y1)) in self.py.iter().enumerate() {
            for (cx, &(x0, x1)) in self.px.iter().enumerate() {
                let c = &cells[(cy * nx + cx) * 4..][..4];
                if c[3] == 0 {
                    continue;
                }
                for y in y0..y1 {
                    for x in x0..x1 {
                        let p = &rgba[(y * w + x) * 4..][..4];
                        if p[3] >= 128 {
                            sum += (0..3).map(|i| p[i].abs_diff(c[i]) as u64).sum::<u64>();
                            n += 3;
                        }
                    }
                }
            }
        }
        if n == 0 {
            0.0
        } else {
            sum as f64 / n as f64
        }
    }
}

// MARK: Palette

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

type Rgb = [f64; 3];

fn d2(a: &Rgb, b: &Rgb) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

struct Fit {
    centres: Vec<Rgb>,
    sse: f64,
}

/// Weighted k-means in RGB: k-means++ initialisation from a seeded generator, then Lloyd iterations.
fn kmeans(points: &[(Rgb, f64)], k: usize, seed: u64) -> Fit {
    let k = k.min(points.len()).max(1);
    let mut rng = Rng::new(seed.wrapping_add(k as u64));
    let total_w: f64 = points.iter().map(|p| p.1).sum();
    let pick = |rng: &mut Rng, weights: &[f64]| -> usize {
        let sum: f64 = weights.iter().sum();
        if sum <= 0.0 {
            return 0;
        }
        let mut t = rng.unit() * sum;
        for (i, w) in weights.iter().enumerate() {
            if t < *w {
                return i;
            }
            t -= w;
        }
        weights.len() - 1
    };
    let mut centres: Vec<Rgb> = Vec::with_capacity(k);
    let first: Vec<f64> = points.iter().map(|p| p.1).collect();
    centres.push(points[pick(&mut rng, &first)].0);
    let mut near: Vec<f64> = points.iter().map(|p| d2(&p.0, &centres[0])).collect();
    while centres.len() < k {
        let weights: Vec<f64> = points.iter().zip(&near).map(|(p, d)| p.1 * d).collect();
        if weights.iter().sum::<f64>() <= 0.0 {
            break;
        }
        let c = points[pick(&mut rng, &weights)].0;
        for (d, p) in near.iter_mut().zip(points) {
            *d = d.min(d2(&p.0, &c));
        }
        centres.push(c);
    }
    let mut assign = vec![usize::MAX; points.len()];
    // Bound the work for huge images: points x centres x iterations.
    let rounds = (KMEANS_BUDGET / (points.len() * centres.len()).max(1)).clamp(3, 60);
    for _ in 0..rounds {
        let mut changed = false;
        for (i, p) in points.iter().enumerate() {
            let best = (0..centres.len())
                .min_by(|&a, &b| d2(&p.0, &centres[a]).total_cmp(&d2(&p.0, &centres[b])).then(a.cmp(&b)))
                .unwrap_or(0);
            if assign[i] != best {
                assign[i] = best;
                changed = true;
            }
        }
        if !changed {
            break;
        }
        let mut acc = vec![([0.0f64; 3], 0.0f64); centres.len()];
        for (p, &a) in points.iter().zip(&assign) {
            for c in 0..3 {
                acc[a].0[c] += p.0[c] * p.1;
            }
            acc[a].1 += p.1;
        }
        for (c, (sum, w)) in centres.iter_mut().zip(acc) {
            if w > 0.0 {
                *c = [sum[0] / w, sum[1] / w, sum[2] / w];
            }
        }
    }
    let sse = points.iter().zip(&assign).map(|(p, &a)| p.1 * d2(&p.0, &centres[a])).sum::<f64>();
    let _ = total_w;
    Fit { centres, sse }
}

fn luminance(c: [u8; 4]) -> u32 {
    299 * c[0] as u32 + 587 * c[1] as u32 + 114 * c[2] as u32
}

/// Distinct opaque cell colours with their counts, in a fixed order.
fn histogram(cells: &[u8]) -> BTreeMap<[u8; 3], u32> {
    let mut h = BTreeMap::new();
    for p in cells.chunks_exact(4).filter(|p| p[3] != 0) {
        *h.entry([p[0], p[1], p[2]]).or_insert(0) += 1;
    }
    h
}

fn palette_from(fit: &Fit, hist: &[([u8; 3], u32)]) -> Vec<[u8; 4]> {
    let mut pal: Vec<[u8; 4]> =
        fit.centres.iter().map(|c| [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, 255]).collect();
    pal.sort_by_key(|c| (luminance(*c), *c));
    pal.dedup();
    // Drop colours nothing snaps to.
    let mut used = vec![false; pal.len()];
    for (c, _) in hist {
        used[nearest(&pal, [c[0], c[1], c[2], 255])] = true;
    }
    let mut keep = used.into_iter();
    pal.retain(|_| keep.next().unwrap_or(false));
    pal
}

fn nearest(pal: &[[u8; 4]], c: [u8; 4]) -> usize {
    let dist = |p: &[u8; 4]| (0..3).map(|i| (p[i] as i32 - c[i] as i32).pow(2)).sum::<i32>();
    (0..pal.len()).min_by_key(|&i| (dist(&pal[i]), i)).unwrap_or(0)
}

/// Chooses the palette for the opaque cells.
fn choose_palette(cells: &[u8], colors: &Colors, seed: u64) -> Vec<[u8; 4]> {
    let hist = histogram(cells);
    let unique: Vec<([u8; 3], u32)> = hist.into_iter().collect();
    let exact = || unique.iter().map(|(c, _)| [c[0], c[1], c[2], 255]).collect::<Vec<_>>();
    let points: Vec<(Rgb, f64)> =
        unique.iter().map(|(c, n)| ([c[0] as f64, c[1] as f64, c[2] as f64], *n as f64)).collect();
    let total: f64 = points.iter().map(|p| p.1).sum();
    match colors {
        Colors::Palette(p) => p.clone(),
        Colors::Keep => {
            if unique.len() <= MAX_COLORS {
                let mut p = exact();
                p.sort_by_key(|c| (luminance(*c), *c));
                p
            } else {
                Vec::new()
            }
        }
        _ if unique.is_empty() => Vec::new(),
        Colors::Count(k) => {
            if unique.len() <= *k {
                let mut p = exact();
                p.sort_by_key(|c| (luminance(*c), *c));
                return p;
            }
            palette_from(&kmeans(&points, *k, seed), &unique)
        }
        Colors::Auto { min, max } => {
            let kmax = (*max).min(unique.len());
            let kmin = (*min).min(kmax);
            if unique.len() <= kmin {
                let mut p = exact();
                p.sort_by_key(|c| (luminance(*c), *c));
                return p;
            }
            let fits = |fit: &Fit| -> bool {
                let far: f64 = points
                    .iter()
                    .filter(|p| fit.centres.iter().map(|c| d2(&p.0, c)).fold(f64::MAX, f64::min) > AUTO_FAR * AUTO_FAR)
                    .map(|p| p.1)
                    .sum();
                (fit.sse / total).sqrt() <= AUTO_RMS && far <= AUTO_OUTLIERS * total
            };
            let mut best = kmeans(&points, kmin, seed);
            let one = kmeans(&points, 1, seed).sse.max(1e-9);
            let mut prev = best.sse;
            let mut k = kmin;
            while !fits(&best) && k < kmax {
                k += 1;
                let next = kmeans(&points, k, seed);
                let gain = (prev - next.sse) / one;
                prev = next.sse;
                if gain < AUTO_GAIN {
                    break;
                }
                best = next;
            }
            palette_from(&best, &unique)
        }
    }
}

/// Snaps every opaque cell to its nearest palette colour (alpha becomes 0 or 255).
fn snap(cells: &mut [u8], pal: &[[u8; 4]]) {
    if pal.is_empty() {
        return;
    }
    let mut memo: HashMap<[u8; 3], [u8; 4]> = HashMap::new();
    for p in cells.chunks_exact_mut(4) {
        if p[3] == 0 {
            p.fill(0);
            continue;
        }
        let key = [p[0], p[1], p[2]];
        let c = *memo.entry(key).or_insert_with(|| {
            let c = pal[nearest(pal, [key[0], key[1], key[2], 255])];
            [c[0], c[1], c[2], 255]
        });
        p.copy_from_slice(&c);
    }
}

// MARK: Outline

/// Closes one-pixel breaks in the dark outer outline of an RGBA image whose background is transparent. A transparent
/// pixel is a break when outline-coloured pixels lie on both sides of it along some axis (horizontal, vertical or a
/// diagonal) and, across that axis, one side is the figure's fill and the other is background: a gap between two
/// limbs has background on both. The outline colour is the most common colour on the figure's edge, if it is dark.
/// Returns the number of pixels filled.
pub fn close_outline(rgba: &mut [u8], w: usize, h: usize) -> usize {
    if w == 0 || h == 0 || rgba.len() != w * h * 4 {
        return 0;
    }
    let opaque = |rgba: &[u8], x: i64, y: i64| -> bool {
        x >= 0 && y >= 0 && x < w as i64 && y < h as i64 && rgba[(y as usize * w + x as usize) * 4 + 3] != 0
    };
    let colour = |rgba: &[u8], x: i64, y: i64| -> Option<[u8; 3]> {
        opaque(rgba, x, y).then(|| {
            let o = (y as usize * w + x as usize) * 4;
            [rgba[o], rgba[o + 1], rgba[o + 2]]
        })
    };
    let mut count: BTreeMap<[u8; 3], usize> = BTreeMap::new();
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if opaque(rgba, x, y)
                && [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dy)| !opaque(rgba, x + dx, y + dy))
            {
                *count.entry(colour(rgba, x, y).unwrap_or_default()).or_insert(0) += 1;
            }
        }
    }
    let Some((&line, _)) = count.iter().max_by_key(|(c, n)| (**n, std::cmp::Reverse(**c))) else { return 0 };
    if luminance([line[0], line[1], line[2], 255]) / 1000 > 110 {
        return 0;
    }
    let mut filled = 0;
    for _ in 0..2 {
        let mut fix: Vec<(usize, usize)> = Vec::new();
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                if opaque(rgba, x, y) {
                    continue;
                }
                let is_line = |dx: i64, dy: i64| colour(rgba, x + dx, y + dy) == Some(line);
                let is_fill = |dx: i64, dy: i64| colour(rgba, x + dx, y + dy).is_some_and(|c| c != line);
                let is_back = |dx: i64, dy: i64| !opaque(rgba, x + dx, y + dy);
                let hit = [(1i64, 0i64), (0, 1), (1, 1), (1, -1)].iter().any(|&(dx, dy)| {
                    if !(is_line(-dx, -dy) && is_line(dx, dy)) {
                        return false;
                    }
                    let (ex, ey) = (-dy, dx);
                    (is_fill(ex, ey) && is_back(-ex, -ey)) || (is_fill(-ex, -ey) && is_back(ex, ey))
                });
                if hit {
                    fix.push((x as usize, y as usize));
                }
            }
        }
        if fix.is_empty() {
            break;
        }
        for (x, y) in fix {
            rgba[(y * w + x) * 4..][..4].copy_from_slice(&[line[0], line[1], line[2], 255]);
            filled += 1;
        }
    }
    filled
}

// MARK: Tracing

struct Traced {
    cells: Vec<u8>,
    size: (usize, usize),
    lattice: Lattice,
    grid: Grid,
    palette: Vec<[u8; 4]>,
    background: BgStats,
    closed: usize,
    quality: f32,
    warning: Option<String>,
}

fn bounds(cells: &[u8], nx: usize, ny: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (nx, ny, 0, 0);
    for y in 0..ny {
        for x in 0..nx {
            if cells[(y * nx + x) * 4 + 3] != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
}

fn crop_cells(cells: &[u8], nx: usize, r: (usize, usize, usize, usize)) -> Vec<u8> {
    let mut out = Vec::with_capacity(r.2 * r.3 * 4);
    for y in r.1..r.1 + r.3 {
        out.extend_from_slice(&cells[(y * nx + r.0) * 4..(y * nx + r.0 + r.2) * 4]);
    }
    out
}

fn trace_full(rgba: &[u8], w: usize, h: usize, o: &TraceOptions) -> Result<Traced, String> {
    o.validate()?;
    check_image(rgba, w, h)?;
    let (px, py, tiles) = profiles(rgba, w, h);
    let Detected { mut grid, warning } = detect_with(&px, &py, &tiles, &o.pitch, o.separate_axes)?;
    if let Some((fx, fy)) = o.phase {
        grid.phase = (fx.rem_euclid(grid.pitch.0), fy.rem_euclid(grid.pitch.1));
    }
    let lattice = Lattice::new(w, h, &grid)?;
    let (nx, ny) = lattice.size();
    let mut cells = lattice.sample(rgba, w);
    let background = match &o.remove_background {
        Some(b) => remove_background(&mut cells, nx, ny, b),
        None => BgStats::default(),
    };
    let palette = choose_palette(&cells, &o.colors, o.seed);
    snap(&mut cells, &palette);
    let closed = if o.close_outline { close_outline(&mut cells, nx, ny) } else { 0 };
    let mad = lattice.fit_error(rgba, w, &cells);
    let quality = (1.0 - mad / 30.0).clamp(0.0, 1.0) as f32;
    Ok(Traced { cells, size: (nx, ny), lattice, grid, palette, background, closed, quality, warning })
}

/// Traces an RGBA image onto its pixel grid. See the module documentation for the steps.
pub fn trace_grid(rgba: &[u8], w: usize, h: usize, o: &TraceOptions) -> Result<TraceResult, String> {
    let t = trace_full(rgba, w, h, o)?;
    let (nx, ny) = t.size;
    let rect = if o.crop { bounds(&t.cells, nx, ny) } else { Some((0, 0, nx, ny)) };
    let Some(rect) = rect else { return Err("nothing is left after removing the background".into()) };
    let rgba = if rect == (0, 0, nx, ny) { t.cells.clone() } else { crop_cells(&t.cells, nx, rect) };
    Ok(TraceResult {
        width: rect.2,
        height: rect.3,
        rgba,
        pitch: t.grid.pitch,
        phase: t.grid.phase,
        palette: t.palette,
        quality: t.quality,
        background: t.background,
        origin: (rect.0, rect.1),
        grid_cells: (nx, ny),
        closed_outline: t.closed,
        warning: t.warning,
    })
}

/// Traces a document in place: the cel at (`layer`, `frame`) defines the grid, the palette and the crop, and every
/// image cel is resampled with them, so a whole animation lands on one grid. The canvas becomes the traced size and
/// the palette of an RGB sprite is set to the traced palette. Off-canvas cel content is dropped.
pub fn trace_sprite(s: &mut Sprite, layer: usize, frame: usize, o: &TraceOptions) -> Result<TraceResult, String> {
    if s.color_mode != crate::document::color_mode::ColorMode::Rgba {
        return Err("trace needs an RGB sprite: change the color mode to RGB first".into());
    }
    if s.layers.iter().any(|l| l.kind == crate::document::tilemap::KIND_TILEMAP) {
        return Err("trace is not available on sprites with tilemap layers".into());
    }
    s.flush_links();
    let (w, h) = (s.width as usize, s.height as usize);
    let cel = s
        .cels
        .get(frame)
        .and_then(|r| r.get(layer))
        .and_then(|c| c.as_ref())
        .filter(|c| c.pixels.len() == w * h * 4)
        .ok_or("the active cel has no pixels to trace")?;
    let t = trace_full(&cel.pixels, w, h, o)?;
    let (nx, ny) = t.size;
    let rect = if o.crop { bounds(&t.cells, nx, ny) } else { Some((0, 0, nx, ny)) };
    let Some(rect) = rect else { return Err("nothing is left after removing the background".into()) };
    let (rw, rh) = (rect.2, rect.3);
    if !s.cels_fit_budget(rw as u64, rh as u64) {
        return Err(format!(
            "the traced sprite would need too much memory: {}",
            s.cel_budget_note(rw as u64, rh as u64)
        ));
    }
    let active_ptr = Arc::as_ptr(cel);
    let active_pixels = crop_cells(&t.cells, nx, rect);
    let mut done: HashMap<*const Cel, Arc<Cel>> = HashMap::new();
    let mut work: Vec<(usize, usize)> = Vec::new();
    for (f, row) in s.cels.iter().enumerate() {
        for (l, c) in row.iter().enumerate() {
            if c.is_some() {
                work.push((f, l));
            }
        }
    }
    let palette_mode = Colors::Palette(t.palette.clone());
    for (f, l) in work {
        let Some(old) = s.cels[f][l].clone() else { continue };
        let key = Arc::as_ptr(&old);
        if let Some(n) = done.get(&key) {
            s.cels[f][l] = Some(n.clone());
            continue;
        }
        let pixels = if key == active_ptr {
            active_pixels.clone()
        } else if old.pixels.len() == w * h * 4 {
            let mut cells = t.lattice.sample(&old.pixels, w);
            if let Some(b) = &o.remove_background {
                remove_background(&mut cells, nx, ny, b);
            }
            if let Colors::Palette(p) = &palette_mode {
                snap(&mut cells, p);
            }
            if o.close_outline {
                close_outline(&mut cells, nx, ny);
            }
            crop_cells(&cells, nx, rect)
        } else {
            vec![0u8; rw * rh * 4]
        };
        let new = Arc::new(Cel {
            opacity: old.opacity,
            pixels,
            user_data: old.user_data.clone(),
            z_index: old.z_index,
            ..Default::default()
        });
        done.insert(key, new.clone());
        s.cels[f][l] = Some(new);
    }
    s.extra.scaled(rw as f64 / w as f64, rh as f64 / h as f64);
    s.width = rw as u32;
    s.height = rh as u32;
    if !t.palette.is_empty() && t.palette.len() <= MAX_COLORS {
        s.palette = t.palette.clone();
    }
    Ok(TraceResult {
        width: rw,
        height: rh,
        rgba: active_pixels,
        pitch: t.grid.pitch,
        phase: t.grid.phase,
        palette: t.palette,
        quality: t.quality,
        background: t.background,
        origin: (rect.0, rect.1),
        grid_cells: (nx, ny),
        closed_outline: t.closed,
        warning: t.warning,
    })
}

// MARK: C ABI

/// Traces the document onto its pixel grid as one undo step (see [`trace_sprite`]); the cel at (`layer`, `frame`)
/// defines the grid. `options_json` is described at [`TraceOptions::from_json`]. Returns the result as JSON
/// (`width`, `height`, `pitch`, `phase`, `palette`, `quality`, `origin`, `grid_cells`, `closed_outline`,
/// `background`; free with `pixmith_string_free`), or null on failure (see `pixmith_last_error`); the document is
/// then unchanged.
#[no_mangle]
pub unsafe extern "C" fn pixmith_trace_grid(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    options_json: *const c_char,
) -> *mut c_char {
    crate::ffi::guarded("pixmith_trace_grid", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return null_mut() };
        let opts = match options_arg(options_json).and_then(|v| TraceOptions::from_json(&v)) {
            Ok(o) => o,
            Err(m) => {
                crate::ffi::set_error(m);
                return null_mut();
            }
        };
        let mut next = e.sprite.clone();
        match trace_sprite(&mut next, layer as usize, frame as usize, &opts) {
            Ok(r) => {
                e.checkpoint();
                e.sprite = next;
                e.sel.mask = None;
                CString::new(json::to_string(&r.to_json())).map_or(null_mut(), CString::into_raw)
            }
            Err(m) => {
                crate::ffi::set_error(m);
                null_mut()
            }
        }
    })
}

/// Opens an image's pixels as a new sprite traced onto its grid (the `trace_grid` import mode).
pub fn sprite_from_trace(rgba: &[u8], w: usize, h: usize, o: &TraceOptions) -> Result<Sprite, String> {
    let r = trace_grid(rgba, w, h, o)?;
    let mut s = Sprite::new(r.width as u32, r.height as u32);
    if !r.palette.is_empty() && r.palette.len() <= MAX_COLORS {
        s.palette = r.palette;
    }
    s.cels[0][0] = crate::formats::import::cel(r.rgba);
    Ok(s)
}
