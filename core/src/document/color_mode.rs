//! Color modes (RGBA / Grayscale / Indexed).
//!
//! Cel buffers always stay full-canvas straight-alpha RGBA so every tool keeps working. The
//! color mode only constrains the *values* in those buffers (gray pixels, palette colors) and
//! decides how `ase::write` encodes them: 32-bit RGBA, 16-bit gray+alpha, or 8-bit palette
//! indices. Indexed pixels are mapped back to indices by exact palette match, falling back to
//! the nearest allowed palette entry.

use crate::document::{default_palette, Cel, Editor, Sprite};
use crate::ffi::{editor, set_error, sprite};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorMode {
    #[default]
    Rgba,
    Grayscale,
    Indexed,
}

impl ColorMode {
    /// Maps an .aseprite header color depth (bits per pixel) to a mode.
    pub fn from_depth(depth: u16) -> Self {
        match depth {
            16 => ColorMode::Grayscale,
            8 => ColorMode::Indexed,
            _ => ColorMode::Rgba,
        }
    }

    /// The .aseprite header color depth this mode is written with.
    pub fn depth(self) -> u16 {
        match self {
            ColorMode::Rgba => 32,
            ColorMode::Grayscale => 16,
            ColorMode::Indexed => 8,
        }
    }

    fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(ColorMode::Rgba),
            1 => Some(ColorMode::Grayscale),
            2 => Some(ColorMode::Indexed),
            _ => None,
        }
    }
}

/// Integer Rec. 601 luma; exact for gray input.
pub fn luma(c: [u8; 4]) -> u8 {
    ((c[0] as u32 * 299 + c[1] as u32 * 587 + c[2] as u32 * 114 + 500) / 1000) as u8
}

/// Perceptually weighted ("redmean") squared distance, plus an alpha term.
fn dist(a: [u8; 4], b: [u8; 4]) -> u32 {
    let rm = (a[0] as i32 + b[0] as i32) / 2;
    let (dr, dg, db) = (a[0] as i32 - b[0] as i32, a[1] as i32 - b[1] as i32, a[2] as i32 - b[2] as i32);
    let da = a[3] as i32 - b[3] as i32;
    (((512 + rm) * dr * dr) >> 8) as u32
        + 4 * (dg * dg) as u32
        + (((767 - rm) * db * db) >> 8) as u32
        + 4 * (da * da) as u32
}

/// Palette entries (first 256) that visible pixels may use: not the transparent index and not
/// fully transparent. Falls back to progressively looser sets for degenerate palettes.
fn allowed_entries(palette: &[[u8; 4]], ti: u8) -> Vec<(u8, [u8; 4])> {
    let pal = &palette[..palette.len().min(256)];
    let pick = |f: &dyn Fn(usize, &[u8; 4]) -> bool| -> Vec<(u8, [u8; 4])> {
        pal.iter().enumerate().filter(|(i, c)| f(*i, c)).map(|(i, c)| (i as u8, *c)).collect()
    };
    let mut v = pick(&|i, c| i != ti as usize && c[3] != 0);
    if v.is_empty() {
        v = pick(&|i, _| i != ti as usize);
    }
    if v.is_empty() {
        v = pick(&|_, _| true);
    }
    v
}

fn nearest_in(entries: &[(u8, [u8; 4])], c: [u8; 4]) -> Option<(u8, [u8; 4])> {
    entries.iter().copied().min_by_key(|&(i, e)| (dist(c, e), i))
}

/// Looks up palette indices for pixels, with caching.
struct Matcher {
    ti: u8,
    allowed: Vec<(u8, [u8; 4])>,
    exact: HashMap<[u8; 4], u8>,
    exact_any: HashMap<[u8; 4], u8>,
    cache: HashMap<[u8; 4], Option<(u8, [u8; 4])>>,
    palette: Vec<[u8; 4]>,
}

impl Matcher {
    fn new(palette: &[[u8; 4]], ti: u8) -> Self {
        let allowed = allowed_entries(palette, ti);
        let mut exact = HashMap::new();
        for &(i, c) in &allowed {
            exact.entry(c).or_insert(i);
        }
        let mut exact_any = HashMap::new();
        for (i, &c) in palette.iter().take(256).enumerate() {
            exact_any.entry(c).or_insert(i as u8);
        }
        Matcher {
            ti,
            allowed,
            exact,
            exact_any,
            cache: HashMap::new(),
            palette: palette.iter().take(256).copied().collect(),
        }
    }

    fn nearest(&mut self, c: [u8; 4]) -> Option<(u8, [u8; 4])> {
        let allowed = &self.allowed;
        *self.cache.entry(c).or_insert_with(|| nearest_in(allowed, c))
    }

    /// The index a pixel is written as. Background layers may use the transparent index's color.
    fn index_for(&mut self, c: [u8; 4], background: bool) -> u8 {
        if c[3] == 0 {
            return self.ti;
        }
        let map = if background { &self.exact_any } else { &self.exact };
        if let Some(&i) = map.get(&c) {
            return i;
        }
        self.nearest(c).map_or(self.ti, |(i, _)| i)
    }

    /// Like `index_for`, but keeps `hint` when it still describes the pixel (see `hint_valid`).
    fn index_with_hint(&mut self, c: [u8; 4], background: bool, hint: Option<u8>) -> u8 {
        match hint {
            Some(h) if hint_valid(&self.palette, self.ti, h, c, background) => h,
            _ => self.index_for(c, background),
        }
    }
}

/// Whether palette index `h` reproduces pixel `p` when read back from an indexed file: the
/// transparent index for transparent pixels, otherwise an entry of exactly the pixel's color
/// (the transparent index only counts on background layers).
pub(crate) fn hint_valid(palette: &[[u8; 4]], ti: u8, h: u8, p: [u8; 4], background: bool) -> bool {
    if p[3] == 0 {
        return h == ti;
    }
    palette.get(h as usize) == Some(&p) && (background || h != ti)
}

/// Full-canvas palette indices of an indexed cel read from a file; everything outside the cel
/// is the transparent index.
#[allow(clippy::too_many_arguments)]
pub fn index_canvas(data: &[u8], x: i32, y: i32, cw: usize, ch: usize, w: usize, h: usize, ti: u8) -> Vec<u8> {
    let mut out = vec![ti; w * h];
    for row in 0..ch {
        let ty = y + row as i32;
        if ty < 0 || ty >= h as i32 {
            continue;
        }
        for col in 0..cw {
            let tx = x + col as i32;
            if tx >= 0 && tx < w as i32 {
                if let Some(&v) = data.get(row * cw + col) {
                    out[ty as usize * w + tx as usize] = v;
                }
            }
        }
    }
    out
}

/// Encodes RGBA cel rows for the sprite's color mode; used by `ase::write`.
pub struct Encoder {
    mode: ColorMode,
    matcher: Matcher,
}

impl Encoder {
    pub fn new(s: &Sprite) -> Self {
        Self::with_mode(s, s.color_mode)
    }

    pub fn with_mode(s: &Sprite, mode: ColorMode) -> Self {
        Encoder { mode, matcher: Matcher::new(&s.palette, s.transparent_index) }
    }

    /// Header color depth in bits per pixel.
    pub fn depth(&self) -> u16 {
        self.mode.depth()
    }

    /// Encodes whole pixels of a cel row. `background` is the layer's background flag.
    pub fn encode_row<'a>(&mut self, background: bool, row: &'a [u8]) -> Cow<'a, [u8]> {
        self.encode_cel_row(background, row, None)
    }

    /// Like `encode_row`; in Indexed mode `hints` (the cel's remembered indices for this row)
    /// are written wherever they still match the pixels.
    pub fn encode_cel_row<'a>(&mut self, background: bool, row: &'a [u8], hints: Option<&[u8]>) -> Cow<'a, [u8]> {
        match self.mode {
            ColorMode::Rgba => Cow::Borrowed(row),
            ColorMode::Grayscale => {
                Cow::Owned(row.chunks_exact(4).flat_map(|p| [luma([p[0], p[1], p[2], 255]), p[3]]).collect())
            }
            ColorMode::Indexed => Cow::Owned(
                row.chunks_exact(4)
                    .enumerate()
                    .map(|(i, p)| {
                        let hint = hints.and_then(|h| h.get(i).copied());
                        self.matcher.index_with_hint([p[0], p[1], p[2], p[3]], background, hint)
                    })
                    .collect(),
            ),
        }
    }
}

/// Runs `f(layer, cel)` on a copy of every cel on layers accepted by `layer_ok`. Cels shared
/// between frames of a layer stay shared.
pub(crate) fn rewrite_cels(s: &mut Sprite, layer_ok: impl Fn(usize) -> bool, mut f: impl FnMut(usize, &mut Cel)) {
    let mut done: HashMap<(usize, *const Cel), Arc<Cel>> = HashMap::new();
    let mut keep_alive = Vec::new();
    for row in &mut s.cels {
        for (li, slot) in row.iter_mut().enumerate() {
            if !layer_ok(li) {
                continue;
            }
            let Some(cel) = slot.as_ref() else { continue };
            let new = done
                .entry((li, Arc::as_ptr(cel)))
                .or_insert_with(|| {
                    let mut c = (**cel).clone();
                    f(li, &mut c);
                    Arc::new(c)
                })
                .clone();
            keep_alive.push(cel.clone());
            *slot = Some(new);
        }
    }
}

/// Applies `f(x, y, pixel)` to every pixel of every cel on layers accepted by `layer_ok`.
/// Remembered palette indices are kept; they are re-checked against pixels when saving.
fn map_cels(s: &mut Sprite, layer_ok: impl Fn(usize) -> bool, mut f: impl FnMut(usize, usize, [u8; 4]) -> [u8; 4]) {
    let w = (s.width as usize).max(1);
    rewrite_cels(s, &layer_ok, |_, c| {
        for (i, px) in c.pixels.chunks_exact_mut(4).enumerate() {
            let out = f(i % w, i / w, [px[0], px[1], px[2], px[3]]);
            px.copy_from_slice(&out);
        }
        // The part beyond the canvas follows. Coordinates only matter modulo a dither matrix (at most 8).
        if let Some(o) = c.outside.as_mut() {
            let o = Arc::make_mut(o);
            let ow = o.w as usize;
            for (i, px) in o.rgba.chunks_exact_mut(4).enumerate() {
                let (x, y) =
                    ((o.x as i64 + (i % ow) as i64).rem_euclid(8), (o.y as i64 + (i / ow) as i64).rem_euclid(8));
                let out = f(x as usize, y as usize, [px[0], px[1], px[2], px[3]]);
                px.copy_from_slice(&out);
            }
        }
    });
    // Tile images follow (positions are tile-local); this also re-renders tilemap cels.
    if !s.tilesets.is_empty() {
        crate::document::tilemap::map_tiles(s, &layer_ok, &mut f);
    }
}

/// Open-addressing set of packed non-transparent colors, for fast "is on the palette" checks.
struct ColorSet {
    slots: Vec<u32>,
}

impl ColorSet {
    const SIZE: usize = 1024;

    fn new(colors: impl Iterator<Item = [u8; 4]>) -> Self {
        let mut set = ColorSet { slots: vec![0; Self::SIZE] };
        for c in colors.filter(|c| c[3] != 0) {
            let v = u32::from_be_bytes(c);
            let mut i = Self::slot(v);
            while set.slots[i] != 0 && set.slots[i] != v {
                i = (i + 1) % Self::SIZE;
            }
            set.slots[i] = v;
        }
        set
    }

    fn slot(v: u32) -> usize {
        (v.wrapping_mul(0x9E37_79B1) >> 22) as usize % Self::SIZE
    }

    fn contains(&self, v: u32) -> bool {
        let mut i = Self::slot(v);
        loop {
            match self.slots[i] {
                0 => return false,
                s if s == v => return true,
                _ => i = (i + 1) % Self::SIZE,
            }
        }
    }
}

/// Snaps pixels to the color mode: nearest palette color when Indexed, gray when Grayscale.
/// Transparent pixels and RGBA sprites are left alone.
pub struct Snapper {
    mode: ColorMode,
    normal: ColorSet,
    background: ColorSet,
    matcher: Matcher,
}

impl Snapper {
    pub fn new(s: &Sprite) -> Self {
        let matcher = Matcher::new(&s.palette, s.transparent_index);
        Snapper {
            mode: s.color_mode,
            normal: ColorSet::new(matcher.allowed.iter().map(|&(_, c)| c)),
            background: ColorSet::new(s.palette.iter().take(256).copied()),
            matcher,
        }
    }

    /// Whether snapping can change anything (not for RGBA sprites).
    pub fn active(&self) -> bool {
        self.mode != ColorMode::Rgba
    }

    fn snap_packed(&mut self, v: u32, background: bool) -> u32 {
        let p = v.to_be_bytes();
        if p[3] == 0 {
            return v;
        }
        match self.mode {
            ColorMode::Rgba => v,
            ColorMode::Grayscale => {
                if p[0] == p[1] && p[1] == p[2] {
                    v
                } else {
                    let l = luma(p);
                    u32::from_be_bytes([l, l, l, p[3]])
                }
            }
            ColorMode::Indexed => {
                let set = if background { &self.background } else { &self.normal };
                if set.contains(v) {
                    v
                } else {
                    self.matcher.nearest(p).map_or(v, |(_, c)| u32::from_be_bytes(c))
                }
            }
        }
    }

    /// Snaps one pixel.
    pub fn snap_pixel(&mut self, p: [u8; 4], background: bool) -> [u8; 4] {
        self.snap_packed(u32::from_be_bytes(p), background).to_be_bytes()
    }

    /// Snaps a whole RGBA buffer in place. Returns whether anything changed.
    pub fn snap_rgba(&mut self, px: &mut [u8], background: bool) -> bool {
        let mut changed = false;
        let mut memo: Option<(u32, u32)> = None;
        for p in px.chunks_exact_mut(4) {
            let v = u32::from_be_bytes([p[0], p[1], p[2], p[3]]);
            let out = match memo {
                Some((i, o)) if i == v => o,
                _ => {
                    let o = self.snap_packed(v, background);
                    memo = Some((v, o));
                    o
                }
            };
            if out != v {
                p.copy_from_slice(&out.to_be_bytes());
                changed = true;
            }
        }
        changed
    }

    /// Whether snapping would leave the buffer unchanged.
    pub fn is_clean(&mut self, px: &[u8], background: bool) -> bool {
        let mut memo = None;
        px.chunks_exact(4).all(|p| {
            let v = u32::from_be_bytes([p[0], p[1], p[2], p[3]]);
            memo == Some(v) || {
                let ok = self.snap_packed(v, background) == v;
                if ok {
                    memo = Some(v);
                }
                ok
            }
        })
    }
}

/// Snaps one cel to the sprite's color mode (no-op for RGBA sprites, tilemap cels and cels that
/// are already clean). Call when an operation that can create arbitrary colors commits.
pub fn conform_cel(s: &mut Sprite, layer: usize, frame: usize) -> bool {
    if s.color_mode == ColorMode::Rgba {
        return false;
    }
    let mut sn = Snapper::new(s);
    if !sn.active() {
        return false;
    }
    let bg = is_background(s, layer);
    let Some(Some(cel)) = s.cels.get(frame).and_then(|r| r.get(layer)) else { return false };
    if cel.tiles.is_some() || sn.is_clean(&cel.pixels, bg) {
        return false;
    }
    let Some(cel) = s.cel_mut(layer, frame) else { return false };
    sn.snap_rgba(&mut cel.pixels, bg)
}

/// Snaps every cel and tile image to the sprite's color mode. Returns whether anything changed.
pub fn conform_all(s: &mut Sprite) -> bool {
    let mut sn = Snapper::new(s);
    if !sn.active() {
        return false;
    }
    let backgrounds: Vec<bool> = (0..s.layers.len()).map(|l| is_background(s, l)).collect();
    let mut done: HashMap<(usize, *const Cel), Option<Arc<Cel>>> = HashMap::new();
    let mut keep_alive = Vec::new();
    let mut changed = false;
    for f in 0..s.cels.len() {
        for li in 0..s.cels[f].len() {
            let Some(cel) = s.cels[f][li].clone() else { continue };
            if cel.tiles.is_some() {
                continue;
            }
            let bg = backgrounds.get(li).copied().unwrap_or(false);
            let new = done
                .entry((li, Arc::as_ptr(&cel)))
                .or_insert_with(|| {
                    if sn.is_clean(&cel.pixels, bg) {
                        return None;
                    }
                    let mut c = (*cel).clone();
                    sn.snap_rgba(&mut c.pixels, bg);
                    Some(Arc::new(c))
                })
                .clone();
            if let Some(n) = new {
                s.cels[f][li] = Some(n);
                changed = true;
            }
            keep_alive.push(cel);
        }
    }
    let mut tiles_changed = false;
    for ts in &mut s.tilesets {
        for tile in &mut ts.tiles {
            if !sn.is_clean(tile.as_slice(), false) {
                sn.snap_rgba(Arc::make_mut(tile).as_mut_slice(), false);
                tiles_changed = true;
            }
        }
    }
    if tiles_changed {
        crate::document::tilemap::refresh_all(s);
    }
    changed || tiles_changed
}

/// Gives every cel a palette index for every pixel (Indexed mode). Remembered indices that
/// still match their pixel are kept, so duplicate palette colors stay distinguishable.
pub fn ensure_hints(s: &mut Sprite) {
    let (palette, ti) = (s.palette.clone(), s.transparent_index);
    let backgrounds: Vec<bool> = (0..s.layers.len()).map(|l| is_background(s, l)).collect();
    let mut m = Matcher::new(&palette, ti);
    rewrite_cels(
        s,
        |_| true,
        |li, cel| {
            if cel.tiles.is_some() {
                return;
            }
            let bg = backgrounds.get(li).copied().unwrap_or(false);
            let old = cel.indices.take();
            let out: Vec<u8> = cel
                .pixels
                .chunks_exact(4)
                .enumerate()
                .map(|(i, p)| {
                    m.index_with_hint([p[0], p[1], p[2], p[3]], bg, old.as_ref().and_then(|o| o.get(i).copied()))
                })
                .collect();
            cel.indices = Some(Arc::new(out));
        },
    );
}

/// Square Bayer threshold matrix (`n` must be a power of two), values 0..n*n.
fn bayer(n: usize) -> Vec<u32> {
    let mut m = vec![0u32];
    let mut size = 1;
    while size < n {
        let ns = size * 2;
        let mut nm = vec![0u32; ns * ns];
        for y in 0..size {
            for x in 0..size {
                let v = 4 * m[y * size + x];
                nm[y * ns + x] = v;
                nm[y * ns + x + size] = v + 2;
                nm[(y + size) * ns + x] = v + 3;
                nm[(y + size) * ns + x + size] = v + 1;
            }
        }
        m = nm;
        size = ns;
    }
    m
}

/// Dither strength: mean nearest-neighbor distance between palette colors, per channel.
fn dither_amplitude(entries: &[(u8, [u8; 4])]) -> f32 {
    if entries.len() < 2 {
        return 0.0;
    }
    let total: f32 = entries
        .iter()
        .enumerate()
        .map(|(i, (_, a))| {
            entries
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, (_, b))| {
                    let d = |k: usize| (a[k] as f32 - b[k] as f32).powi(2);
                    (d(0) + d(1) + d(2)).sqrt()
                })
                .fold(f32::MAX, f32::min)
        })
        .sum();
    (total / entries.len() as f32 / 3f32.sqrt()).clamp(8.0, 255.0)
}

/// Makes sure an alpha-0 palette entry exists and is the transparent index, so that all
/// opaque palette colors stay usable by visible pixels.
pub fn prepare_transparent(s: &mut Sprite) {
    if s.palette.get(s.transparent_index as usize).is_some_and(|c| c[3] == 0) {
        return;
    }
    if let Some(i) = s.palette.iter().take(256).position(|c| c[3] == 0) {
        s.transparent_index = i as u8;
    } else if s.palette.len() < 256 {
        s.palette.push([0; 4]);
        s.transparent_index = (s.palette.len() - 1) as u8;
    }
}

/// Converts pixel values to `target`. `dither` is a Bayer matrix size (2, 4 or 8; 0 = none) and
/// only matters when converting to Indexed. Converting Indexed to Indexed re-snaps pixels to
/// the palette.
pub fn convert(s: &mut Sprite, target: ColorMode, dither: u32) {
    match target {
        ColorMode::Rgba => {}
        ColorMode::Grayscale => map_cels(
            s,
            |_| true,
            |_, _, p| {
                if p[3] == 0 {
                    p
                } else {
                    let l = luma(p);
                    [l, l, l, p[3]]
                }
            },
        ),
        ColorMode::Indexed => {
            if s.palette.is_empty() {
                s.palette = default_palette();
            }
            s.palette.truncate(256);
            if s.color_mode != ColorMode::Indexed {
                prepare_transparent(s);
            }
            let mut m = Matcher::new(&s.palette, s.transparent_index);
            let n = match dither {
                0 => 0,
                2 | 4 | 8 => dither as usize,
                _ => 4,
            };
            let matrix = bayer(n.max(1));
            let amp = dither_amplitude(&m.allowed);
            map_cels(
                s,
                |_| true,
                |x, y, p| {
                    if p[3] == 0 || m.exact.contains_key(&p) {
                        return p;
                    }
                    let mut q = p;
                    if n > 0 {
                        let t = (matrix[(y % n) * n + x % n] as f32 + 0.5) / (n * n) as f32 - 0.5;
                        let off = (t * amp).round() as i32;
                        for c in &mut q[..3] {
                            *c = (*c as i32 + off).clamp(0, 255) as u8;
                        }
                    }
                    // Alpha is compared against the original pixel, not the dithered one.
                    q[3] = p[3];
                    m.nearest(q).map_or(p, |(_, c)| c)
                },
            );
            ensure_hints(s);
        }
    }
    s.color_mode = target;
}

/// Constrains a drawing color to the sprite's color mode.
pub fn snap_color(s: &Sprite, c: [u8; 4]) -> [u8; 4] {
    if c[3] == 0 {
        return c;
    }
    match s.color_mode {
        ColorMode::Rgba => c,
        ColorMode::Grayscale => {
            let l = luma(c);
            [l, l, l, c[3]]
        }
        ColorMode::Indexed => nearest_in(&allowed_entries(&s.palette, s.transparent_index), c).map_or(c, |(_, e)| e),
    }
}

fn is_background(s: &Sprite, layer: usize) -> bool {
    s.layers.get(layer).is_some_and(|l| l.flags & 8 != 0)
}

/// Replaces a palette entry. In Indexed mode, pixels using that entry take the new color.
/// Pixels whose remembered index is still valid follow only their own entry; pixels without
/// one follow by color, so they cannot be told apart from an identical duplicate entry.
pub fn set_palette_entry(s: &mut Sprite, index: usize, c: [u8; 4]) -> bool {
    let Some(old) = s.palette.get(index).copied() else { return false };
    let old_palette = s.palette.clone();
    s.palette[index] = c;
    if s.color_mode == ColorMode::Indexed && old != c && old[3] != 0 {
        let ti = s.transparent_index;
        // The transparent index only shows up as a color on background layers.
        let only_bg = index == ti as usize;
        let backgrounds: Vec<bool> = (0..s.layers.len()).map(|l| is_background(s, l)).collect();
        let bg = |l: usize| backgrounds.get(l).copied().unwrap_or(false);
        rewrite_cels(
            s,
            |l| !only_bg || bg(l),
            |li, cel| {
                let hints = cel.indices.clone();
                for (i, px) in cel.pixels.chunks_exact_mut(4).enumerate() {
                    let p = [px[0], px[1], px[2], px[3]];
                    let hit = match hints.as_ref().and_then(|h| h.get(i).copied()) {
                        Some(h) if hint_valid(&old_palette, ti, h, p, bg(li)) => h as usize == index,
                        _ => p == old,
                    };
                    if hit {
                        px.copy_from_slice(&c);
                    }
                }
                // Pixels beyond the canvas carry no remembered indices and follow by color.
                if let Some(o) = cel.outside.as_mut() {
                    for px in Arc::make_mut(o).rgba.chunks_exact_mut(4) {
                        if px[..4] == old {
                            px.copy_from_slice(&c);
                        }
                    }
                }
            },
        );
        // Tile images carry no remembered indices, so they follow by color.
        if !s.tilesets.is_empty() {
            crate::document::tilemap::map_tiles(s, &|l| !only_bg || bg(l), &mut |_, _, p| if p == old { c } else { p });
        }
    }
    true
}

/// Colors used by the sprite: exact (most frequent first) if they fit in `max` (0 = 256),
/// otherwise median-cut quantized to `max` opaque colors sorted by brightness.
pub fn palette_from_sprite(s: &Sprite, max: usize) -> Vec<[u8; 4]> {
    let n = if max == 0 { 256 } else { max.clamp(1, 256) };
    let mut counts: HashMap<[u8; 4], u32> = HashMap::new();
    let mut seen = HashSet::new();
    for cel in s.cels.iter().flatten().flatten() {
        if !seen.insert(Arc::as_ptr(cel)) {
            continue;
        }
        for p in cel.pixels.chunks_exact(4).chain(cel.outside.iter().flat_map(|o| o.rgba.chunks_exact(4))) {
            if p[3] != 0 {
                *counts.entry([p[0], p[1], p[2], p[3]]).or_insert(0) += 1;
            }
        }
    }
    if counts.is_empty() {
        return Vec::new();
    }
    let mut colors: Vec<_> = counts.into_iter().collect();
    colors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    if colors.len() <= n {
        return colors.into_iter().map(|(c, _)| c).collect();
    }
    let mut rgb: HashMap<[u8; 3], u32> = HashMap::new();
    for (c, k) in colors {
        *rgb.entry([c[0], c[1], c[2]]).or_insert(0) += k;
    }
    let mut rgb: Vec<_> = rgb.into_iter().collect();
    rgb.sort();
    median_cut(rgb, n)
}

fn median_cut(colors: Vec<([u8; 3], u32)>, n: usize) -> Vec<[u8; 4]> {
    fn widest(b: &[([u8; 3], u32)]) -> (usize, u32) {
        (0..3)
            .map(|k| {
                let lo = b.iter().map(|c| c.0[k]).min().unwrap_or(0);
                let hi = b.iter().map(|c| c.0[k]).max().unwrap_or(0);
                (k, (hi - lo) as u32)
            })
            .max_by_key(|&(k, r)| (r, std::cmp::Reverse(k)))
            .unwrap()
    }
    let mut boxes = vec![colors];
    while boxes.len() < n {
        // Split the box with the most visible spread: widest channel range times population.
        let pick = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .max_by_key(|(_, b)| widest(b).1 as u64 * b.iter().map(|c| c.1 as u64).sum::<u64>())
            .map(|(i, _)| i);
        let Some(i) = pick else { break };
        let mut b = boxes.swap_remove(i);
        let (k, _) = widest(&b);
        b.sort_by_key(|c| (c.0[k], c.0));
        let half = b.iter().map(|c| c.1 as u64).sum::<u64>() / 2;
        let mut acc = 0;
        let mut at = 1;
        for (j, c) in b.iter().enumerate() {
            acc += c.1 as u64;
            if acc >= half {
                at = j + 1;
                break;
            }
        }
        let at = at.clamp(1, b.len() - 1);
        let hi = b.split_off(at);
        boxes.push(b);
        boxes.push(hi);
    }
    let mut out: Vec<[u8; 4]> = boxes
        .iter()
        .map(|b| {
            let total: u64 = b.iter().map(|c| c.1 as u64).sum::<u64>().max(1);
            let avg =
                |k: usize| ((b.iter().map(|c| c.0[k] as u64 * c.1 as u64).sum::<u64>() + total / 2) / total) as u8;
            [avg(0), avg(1), avg(2), 255]
        })
        .collect();
    out.sort_by_key(|&c| (luma(c), c));
    out.dedup();
    out
}

// ---- C ABI ----

/// 0 = RGBA, 1 = grayscale, 2 = indexed.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cm_get_mode(doc: *const Editor) -> u32 {
    crate::ffi::guarded("pixmith_cm_get_mode", std::ptr::null_mut(), || sprite(doc).map_or(0, |s| s.color_mode as u32))
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_cm_transparent_index(doc: *const Editor) -> u32 {
    crate::ffi::guarded("pixmith_cm_transparent_index", std::ptr::null_mut(), || {
        sprite(doc).map_or(0, |s| s.transparent_index as u32)
    })
}

/// Converts the whole sprite to `mode` (0 RGBA, 1 grayscale, 2 indexed) as one undo step.
/// `dither` is a Bayer matrix size (0 = none, 2, 4, 8). Returns false if nothing changed.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cm_convert(doc: *mut Editor, mode: u32, dither: u32) -> bool {
    crate::ffi::guarded("pixmith_cm_convert", doc, || {
        let Some(e) = editor(doc) else { return false };
        let Some(target) = ColorMode::from_u32(mode) else {
            set_error("unknown color mode");
            return false;
        };
        if target == e.sprite.color_mode && target != ColorMode::Indexed {
            set_error("the sprite is already in that color mode");
            return false;
        }
        e.checkpoint();
        e.label_last(match target {
            ColorMode::Rgba => "Convert to RGB Color",
            ColorMode::Grayscale => "Convert to Grayscale",
            ColorMode::Indexed => "Convert to Indexed",
        });
        convert(&mut e.sprite, target, dither);
        true
    })
}

/// Snaps every pixel of every cel (and tile image) to the palette or to gray, as one undo step.
/// Returns false, recording nothing, if the sprite is RGBA or already conforms.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cm_conform_all(doc: *mut Editor) -> bool {
    crate::ffi::guarded("pixmith_cm_conform_all", doc, || {
        let Some(e) = editor(doc) else { return false };
        let mut next = e.sprite.clone();
        if !conform_all(&mut next) {
            return false;
        }
        e.checkpoint();
        e.label_last("Snap Pixels to Palette");
        e.sprite = next;
        true
    })
}

/// Snaps a 0xRRGGBBAA drawing color to the color mode (nearest palette color when indexed).
#[no_mangle]
pub unsafe extern "C" fn pixmith_cm_snap_color(doc: *const Editor, rgba: u32) -> u32 {
    crate::ffi::guarded("pixmith_cm_snap_color", std::ptr::null_mut(), || {
        sprite(doc).map_or(rgba, |s| u32::from_be_bytes(snap_color(s, rgba.to_be_bytes())))
    })
}

/// Index of the first palette entry exactly equal to `rgba`, or -1.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cm_palette_index_of(doc: *const Editor, rgba: u32) -> i32 {
    crate::ffi::guarded("pixmith_cm_palette_index_of", std::ptr::null_mut(), || {
        let c = rgba.to_be_bytes();
        sprite(doc).and_then(|s| s.palette.iter().position(|&p| p == c)).map_or(-1, |i| i as i32)
    })
}

/// Sets a palette entry as one undo step; in Indexed mode pixels using it are recolored.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cm_palette_set(doc: *mut Editor, index: u32, rgba: u32) -> bool {
    crate::ffi::guarded("pixmith_cm_palette_set", doc, || {
        let Some(e) = editor(doc) else { return false };
        let c = rgba.to_be_bytes();
        if e.sprite.palette.get(index as usize).is_none_or(|&old| old == c) {
            return false;
        }
        e.checkpoint();
        e.label_last("Edit Palette Entry");
        set_palette_entry(&mut e.sprite, index as usize, c)
    })
}

/// Replaces the palette with the sprite's colors (`max_colors` 0 = exact, up to 256; otherwise
/// quantized when there are more). One undo step. Returns the new palette size, 0 on failure.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cm_palette_from_sprite(doc: *mut Editor, max_colors: u32) -> u32 {
    crate::ffi::guarded("pixmith_cm_palette_from_sprite", doc, || {
        let Some(e) = editor(doc) else { return 0 };
        let indexed = e.sprite.color_mode == ColorMode::Indexed;
        let mut max = if max_colors == 0 { 256 } else { max_colors as usize };
        if indexed {
            max = max.min(255); // leave room for the transparent entry
        }
        let palette = palette_from_sprite(&e.sprite, max);
        if palette.is_empty() {
            set_error("the sprite has no visible pixels");
            return 0;
        }
        e.checkpoint();
        e.label_last("Palette from Sprite");
        crate::palette::edit::replace_palette(&mut e.sprite, palette);
        e.sprite.palette.len() as u32
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Layer;
    use crate::formats::ase;

    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];

    fn px(s: &Sprite, x: usize, y: usize) -> [u8; 4] {
        let r = s.render(0);
        let i = (y * s.width as usize + x) * 4;
        r[i..i + 4].try_into().unwrap()
    }

    /// 4x1 sprite with an explicit palette: [transparent, black, red, green, white].
    fn indexed_sprite() -> Sprite {
        let mut s = Sprite::new(4, 1);
        s.palette = vec![[0; 4], BLACK, RED, GREEN, WHITE];
        s.color_mode = ColorMode::Indexed;
        s.transparent_index = 0;
        s.draw_line(0, 0, 1, 0, 1, 0, RED, 1);
        s.draw_line(0, 0, 2, 0, 2, 0, BLACK, 1);
        s.draw_line(0, 0, 3, 0, 3, 0, WHITE, 1);
        s
    }

    fn header_depth(bytes: &[u8]) -> u16 {
        u16::from_le_bytes([bytes[12], bytes[13]])
    }

    #[test]
    fn indexed_round_trip_keeps_mode_and_pixels() {
        let s = indexed_sprite();
        let bytes = ase::write(&s);
        assert_eq!(header_depth(&bytes), 8);
        assert_eq!(bytes[28], 0);
        let back = ase::read(&bytes).unwrap();
        assert_eq!(back.color_mode, ColorMode::Indexed);
        assert_eq!(back.transparent_index, 0);
        assert_eq!(back.palette, s.palette);
        assert_eq!(back.render(0), s.render(0));
        assert_eq!(ase::write(&back), bytes, "re-saving is byte-identical");
    }

    #[test]
    fn indexed_files_are_smaller_than_rgba_files() {
        let mut s = Sprite::new(64, 64);
        s.palette = vec![[0; 4], BLACK, RED];
        s.color_mode = ColorMode::Indexed;
        for y in 0..64 {
            s.draw_line(0, 0, 0, y, 63, y, if y % 2 == 0 { RED } else { BLACK }, 1);
        }
        let indexed = ase::write(&s);
        s.color_mode = ColorMode::Rgba;
        let rgba = ase::write(&s);
        assert_eq!(header_depth(&rgba), 32);
        assert_eq!(header_depth(&indexed), 8);
        let back = ase::read(&indexed).unwrap();
        assert_eq!(px(&back, 5, 0), RED);
        assert_eq!(px(&back, 5, 1), BLACK);
    }

    #[test]
    fn indexed_transparent_index_other_than_zero() {
        let mut s = indexed_sprite();
        s.palette = vec![BLACK, RED, [0; 4], WHITE];
        s.transparent_index = 2;
        s.cels = vec![vec![None]];
        s.draw_line(0, 0, 0, 0, 0, 0, BLACK, 1);
        s.draw_line(0, 0, 1, 0, 1, 0, RED, 1);
        let bytes = ase::write(&s);
        assert_eq!(bytes[28], 2);
        let back = ase::read(&bytes).unwrap();
        assert_eq!(back.transparent_index, 2);
        assert_eq!(back.render(0), s.render(0));
        assert_eq!(px(&back, 0, 0), BLACK, "index 0 is a real color here");
    }

    #[test]
    fn visible_pixels_never_use_the_transparent_index() {
        // Black sits at the transparent index 0 (Aseprite's default layout); visible black
        // must go to the other black entry rather than becoming transparent.
        let mut s = Sprite::new(2, 1);
        s.palette = vec![BLACK, RED, BLACK];
        s.color_mode = ColorMode::Indexed;
        s.draw_line(0, 0, 0, 0, 0, 0, BLACK, 1);
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(px(&back, 0, 0), BLACK);
        assert_eq!(px(&back, 1, 0), [0; 4]);
    }

    #[test]
    fn background_layer_may_use_transparent_index_color() {
        let mut s = Sprite::new(2, 1);
        s.palette = vec![BLACK, RED];
        s.color_mode = ColorMode::Indexed;
        s.layers[0].flags |= 8; // background
        s.draw_line(0, 0, 0, 0, 0, 0, BLACK, 1);
        s.draw_line(0, 0, 1, 0, 1, 0, RED, 1);
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back.render(0), s.render(0));
        assert_eq!(px(&back, 0, 0), BLACK);
    }

    #[test]
    fn reads_and_resaves_handmade_indexed_file_unchanged() {
        // Palette index 1 is the only visible color in the existing indexed fixture format.
        let mut s = Sprite::new(3, 1);
        s.palette = vec![BLACK, [10, 20, 30, 255]];
        s.color_mode = ColorMode::Indexed;
        s.draw_line(0, 0, 1, 0, 1, 0, [10, 20, 30, 255], 1);
        let bytes = ase::write(&s);
        let back = ase::read(&bytes).unwrap();
        assert_eq!(back.render(0), s.render(0));
        assert_eq!(ase::write(&back), bytes);
    }

    #[test]
    fn grayscale_round_trip() {
        let mut s = Sprite::new(3, 1);
        s.color_mode = ColorMode::Grayscale;
        s.draw_line(0, 0, 0, 0, 0, 0, [10, 10, 10, 255], 1);
        s.draw_line(0, 0, 1, 0, 1, 0, [200, 200, 200, 128], 1);
        let bytes = ase::write(&s);
        assert_eq!(header_depth(&bytes), 16);
        let back = ase::read(&bytes).unwrap();
        assert_eq!(back.color_mode, ColorMode::Grayscale);
        assert_eq!(back.render(0), s.render(0));
        assert_eq!(ase::write(&back), bytes);
    }

    #[test]
    fn rgba_files_are_unaffected() {
        let mut s = Sprite::new(2, 2);
        s.draw_line(0, 0, 0, 0, 1, 1, [1, 2, 3, 255], 1);
        let bytes = ase::write(&s);
        assert_eq!(header_depth(&bytes), 32);
        assert_eq!(ase::read(&bytes).unwrap(), s);
    }

    #[test]
    fn convert_to_indexed_picks_nearest_and_keeps_transparency() {
        let mut s = Sprite::new(4, 1);
        s.palette = vec![BLACK, WHITE, RED];
        s.draw_line(0, 0, 0, 0, 0, 0, [20, 20, 20, 255], 1);
        s.draw_line(0, 0, 1, 0, 1, 0, [240, 230, 250, 255], 1);
        s.draw_line(0, 0, 2, 0, 2, 0, [200, 30, 10, 255], 1);
        let mut e = Editor::new(s);
        assert!(unsafe { pixmith_cm_convert(&mut e, 2, 0) });
        let s = &e.sprite;
        assert_eq!(s.color_mode, ColorMode::Indexed);
        // A transparent entry was appended and black stays usable.
        assert_eq!(s.palette.len(), 4);
        assert_eq!(s.palette[3], [0; 4]);
        assert_eq!(s.transparent_index, 3);
        assert_eq!(px(s, 0, 0), BLACK);
        assert_eq!(px(s, 1, 0), WHITE);
        assert_eq!(px(s, 2, 0), RED);
        assert_eq!(px(s, 3, 0), [0; 4]);
    }

    #[test]
    fn convert_is_one_undo_step() {
        let mut s = Sprite::new(2, 1);
        s.palette = vec![BLACK, WHITE];
        s.draw_line(0, 0, 0, 0, 1, 0, [100, 100, 100, 255], 1);
        let before = s.clone();
        let mut e = Editor::new(s);
        assert!(unsafe { pixmith_cm_convert(&mut e, 2, 4) });
        assert_ne!(e.sprite, before);
        assert!(e.undo());
        assert_eq!(e.sprite, before);
        assert!(!e.undo(), "only one step was recorded");
        assert!(e.redo());
        assert_eq!(e.sprite.color_mode, ColorMode::Indexed);
    }

    #[test]
    fn converting_to_the_current_mode_is_a_no_op() {
        let mut e = Editor::new(Sprite::new(1, 1));
        assert!(!unsafe { pixmith_cm_convert(&mut e, 0, 0) });
        assert!(!e.undo());
    }

    #[test]
    fn dithering_mixes_palette_colors_and_only_uses_them() {
        let mut s = Sprite::new(8, 8);
        s.palette = vec![BLACK, WHITE];
        s.fill(0, 0, 0, 0, [128, 128, 128, 255]);
        let mut plain = s.clone();
        convert(&mut plain, ColorMode::Indexed, 0);
        convert(&mut s, ColorMode::Indexed, 4);
        let r = s.render(0);
        let whites = r.chunks_exact(4).filter(|p| p == &WHITE).count();
        let blacks = r.chunks_exact(4).filter(|p| p == &BLACK).count();
        assert_eq!(whites + blacks, 64, "only palette colors remain");
        assert!(whites > 16 && blacks > 16, "mid gray dithers to a mix: {whites}/{blacks}");
        let flat = plain.render(0);
        let flat_whites = flat.chunks_exact(4).filter(|p| p == &WHITE).count();
        assert!(flat_whites == 0 || flat_whites == 64, "without dithering the fill is uniform");
    }

    #[test]
    fn convert_to_grayscale_uses_luma_and_back_to_rgba() {
        let mut s = Sprite::new(3, 1);
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        s.draw_line(0, 0, 1, 0, 1, 0, WHITE, 1);
        s.draw_line(0, 0, 2, 0, 2, 0, [0, 0, 255, 100], 1);
        convert(&mut s, ColorMode::Grayscale, 0);
        assert_eq!(s.color_mode, ColorMode::Grayscale);
        assert_eq!(px(&s, 0, 0), [76, 76, 76, 255]);
        assert_eq!(px(&s, 1, 0), WHITE);
        assert_eq!(px(&s, 2, 0)[3], 100);
        let gray = s.render(0);
        convert(&mut s, ColorMode::Rgba, 0);
        assert_eq!(s.color_mode, ColorMode::Rgba);
        assert_eq!(s.render(0), gray);
    }

    #[test]
    fn indexed_to_grayscale_and_grayscale_to_indexed() {
        let mut s = indexed_sprite();
        convert(&mut s, ColorMode::Grayscale, 0);
        assert_eq!(px(&s, 1, 0), [76, 76, 76, 255]);
        convert(&mut s, ColorMode::Indexed, 0);
        // Gray 76 is closest to black among {black, red, green, white}.
        assert!(s.palette.contains(&px(&s, 1, 0)));
    }

    #[test]
    fn conversion_keeps_shared_cels_shared() {
        let mut s = Sprite::new(2, 1);
        s.palette = vec![BLACK, WHITE];
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        s.add_frame(0, false);
        s.cels[1][0] = s.cels[0][0].clone(); // a linked cel
        assert!(Arc::ptr_eq(s.cels[0][0].as_ref().unwrap(), s.cels[1][0].as_ref().unwrap()));
        convert(&mut s, ColorMode::Indexed, 0);
        assert!(Arc::ptr_eq(s.cels[0][0].as_ref().unwrap(), s.cels[1][0].as_ref().unwrap()));
    }

    #[test]
    fn snap_color_by_mode() {
        let mut s = Sprite::new(1, 1);
        s.palette = vec![[0; 4], BLACK, RED, WHITE];
        assert_eq!(snap_color(&s, [250, 10, 10, 255]), [250, 10, 10, 255]);
        s.color_mode = ColorMode::Indexed;
        assert_eq!(snap_color(&s, [250, 10, 10, 255]), RED);
        assert_eq!(snap_color(&s, [10, 10, 10, 255]), BLACK);
        assert_eq!(snap_color(&s, [1, 2, 3, 0]), [1, 2, 3, 0], "erase color is preserved");
        s.color_mode = ColorMode::Grayscale;
        assert_eq!(snap_color(&s, RED), [76, 76, 76, 255]);
        unsafe {
            let mut e = Editor::new(s.clone());
            e.sprite.color_mode = ColorMode::Indexed;
            assert_eq!(pixmith_cm_snap_color(&e, 0xFA0A0AFF), 0xFF0000FF);
            assert_eq!(pixmith_cm_palette_index_of(&e, 0xFF0000FF), 2);
            assert_eq!(pixmith_cm_palette_index_of(&e, 0x123456FF), -1);
        }
    }

    #[test]
    fn editing_a_palette_entry_recolors_pixels_in_indexed_mode_only() {
        let mut s = indexed_sprite();
        let blue = [0, 0, 255, 255];
        assert!(set_palette_entry(&mut s, 2, blue));
        assert_eq!(px(&s, 1, 0), blue, "red pixel follows its palette entry");
        assert_eq!(px(&s, 2, 0), BLACK);
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back.render(0), s.render(0));

        let mut rgba = indexed_sprite();
        rgba.color_mode = ColorMode::Rgba;
        assert!(set_palette_entry(&mut rgba, 2, blue));
        assert_eq!(px(&rgba, 1, 0), RED, "RGBA pixels are independent of the palette");
        assert!(!set_palette_entry(&mut rgba, 99, blue));
    }

    #[test]
    fn palette_edit_ffi_is_undoable() {
        let mut e = Editor::new(indexed_sprite());
        assert!(unsafe { pixmith_cm_palette_set(&mut e, 2, 0x0000FFFF) });
        assert_eq!(px(&e.sprite, 1, 0), [0, 0, 255, 255]);
        assert!(!unsafe { pixmith_cm_palette_set(&mut e, 2, 0x0000FFFF) }, "unchanged entry is ignored");
        assert!(e.undo());
        assert_eq!(px(&e.sprite, 1, 0), RED);
    }

    #[test]
    fn palette_from_sprite_exact_orders_by_frequency() {
        let mut s = Sprite::new(4, 1);
        s.draw_line(0, 0, 0, 0, 2, 0, GREEN, 1);
        s.draw_line(0, 0, 3, 0, 3, 0, RED, 1);
        assert_eq!(palette_from_sprite(&s, 0), vec![GREEN, RED]);
        assert!(palette_from_sprite(&Sprite::new(2, 2), 0).is_empty());
    }

    #[test]
    fn palette_from_sprite_quantizes_to_n_colors() {
        let mut s = Sprite::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                let c = [(x * 16) as u8, (y * 16) as u8, ((x + y) * 8) as u8, 255];
                s.draw_line(0, 0, x, y, x, y, c, 1);
            }
        }
        assert_eq!(palette_from_sprite(&s, 0).len(), 256);
        for n in [2usize, 8, 16, 64] {
            let p = palette_from_sprite(&s, n);
            assert!(!p.is_empty() && p.len() <= n, "n={n} got {}", p.len());
            assert!(p.len() >= n / 2, "n={n} got only {}", p.len());
            assert!(p.iter().all(|c| c[3] == 255));
        }
        // Two clusters quantize to two representative colors.
        let mut two = Sprite::new(4, 1);
        two.draw_line(0, 0, 0, 0, 1, 0, [250, 0, 0, 255], 1);
        two.draw_line(0, 0, 2, 0, 3, 0, [0, 0, 250, 255], 1);
        two.draw_line(0, 0, 1, 0, 1, 0, [240, 0, 0, 255], 1);
        two.draw_line(0, 0, 3, 0, 3, 0, [0, 0, 240, 255], 1);
        let p = palette_from_sprite(&two, 2);
        assert_eq!(p.len(), 2);
        assert!(p.iter().any(|c| c[0] > 200) && p.iter().any(|c| c[2] > 200));
    }

    #[test]
    fn palette_from_sprite_ffi_in_indexed_mode_keeps_pixels_and_transparency() {
        let mut e = Editor::new(indexed_sprite());
        let before = e.sprite.render(0);
        let n = unsafe { pixmith_cm_palette_from_sprite(&mut e, 0) };
        assert_eq!(n as usize, e.sprite.palette.len());
        assert_eq!(e.sprite.palette.len(), 4, "3 used colors + transparent entry");
        assert_eq!(e.sprite.palette[e.sprite.transparent_index as usize], [0; 4]);
        assert_eq!(e.sprite.render(0), before);
        let back = ase::read(&ase::write(&e.sprite)).unwrap();
        assert_eq!(back.render(0), before);
        assert!(e.undo());
        assert_eq!(e.sprite.palette.len(), 5);
    }

    #[test]
    fn palette_from_empty_sprite_fails_without_changes() {
        let mut e = Editor::new(Sprite::new(2, 2));
        assert_eq!(unsafe { pixmith_cm_palette_from_sprite(&mut e, 0) }, 0);
        assert!(!e.undo());
    }

    #[test]
    fn multiple_layers_and_frames_round_trip_indexed() {
        let mut s = indexed_sprite();
        let top = s.add_layer(0);
        s.layers[top] = Layer { opacity: 100, ..Layer::new("top") };
        s.draw_line(top, 0, 0, 0, 0, 0, GREEN, 1);
        s.add_frame(0, true);
        s.draw_line(0, 1, 3, 0, 3, 0, RED, 1);
        let bytes = ase::write(&s);
        let back = ase::read(&bytes).unwrap();
        for f in 0..2 {
            assert_eq!(back.render(f), s.render(f));
        }
        assert_eq!(ase::write(&back), bytes);
    }

    // ---- keeping pixels on the palette ----

    fn on_palette(s: &Sprite, frame: usize) -> bool {
        s.render(frame).chunks_exact(4).all(|p| p[3] == 0 || s.palette.contains(&[p[0], p[1], p[2], p[3]]))
    }

    /// 4x1 Indexed sprite with palette [clear, black, white, red].
    fn bw_sprite() -> Sprite {
        let mut s = Sprite::new(4, 1);
        s.palette = vec![[0; 4], BLACK, WHITE, RED];
        s.color_mode = ColorMode::Indexed;
        s
    }

    #[test]
    fn conform_cel_snaps_indexed_and_grayscale_only() {
        let mut s = bw_sprite();
        s.draw_line(0, 0, 0, 0, 0, 0, [250, 10, 10, 255], 1);
        s.draw_line(0, 0, 1, 0, 1, 0, [128, 128, 128, 255], 1);
        s.draw_line(0, 0, 2, 0, 2, 0, WHITE, 1);
        assert!(conform_cel(&mut s, 0, 0));
        assert_eq!(px(&s, 0, 0), RED);
        assert!(on_palette(&s, 0));
        assert_eq!(px(&s, 3, 0), [0; 4], "transparent pixels stay transparent");
        assert!(!conform_cel(&mut s, 0, 0), "already clean");

        s.color_mode = ColorMode::Grayscale;
        assert!(conform_cel(&mut s, 0, 0));
        assert_eq!(px(&s, 0, 0), [76, 76, 76, 255]);

        s.color_mode = ColorMode::Rgba;
        s.draw_line(0, 0, 0, 0, 0, 0, [1, 2, 3, 255], 1);
        assert!(!conform_cel(&mut s, 0, 0));
        assert_eq!(px(&s, 0, 0), [1, 2, 3, 255]);
    }

    #[test]
    fn conform_respects_the_transparent_index_and_background_layers() {
        // Palette index 0 is opaque black and also the transparent index.
        let mut s = Sprite::new(2, 1);
        s.palette = vec![BLACK, RED, [10, 10, 10, 255]];
        s.color_mode = ColorMode::Indexed;
        s.draw_line(0, 0, 0, 0, 0, 0, BLACK, 1);
        assert!(conform_cel(&mut s, 0, 0));
        assert_eq!(px(&s, 0, 0), [10, 10, 10, 255], "visible black cannot use the transparent entry");
        s.layers[0].flags |= 8;
        s.draw_line(0, 0, 0, 0, 0, 0, BLACK, 1);
        assert!(!conform_cel(&mut s, 0, 0), "a background layer may show it");
    }

    #[test]
    fn conform_all_covers_every_cel_and_keeps_links() {
        let mut s = bw_sprite();
        s.draw_line(0, 0, 0, 0, 3, 0, [100, 100, 100, 255], 1);
        s.add_frame(0, true);
        let extra = s.add_layer(0);
        s.draw_line(extra, 1, 1, 0, 1, 0, [240, 240, 240, 255], 1);
        let linked = Arc::ptr_eq(s.cels[0][0].as_ref().unwrap(), s.cels[1][0].as_ref().unwrap());
        assert!(conform_all(&mut s));
        assert!(on_palette(&s, 0) && on_palette(&s, 1));
        assert_eq!(linked, Arc::ptr_eq(s.cels[0][0].as_ref().unwrap(), s.cels[1][0].as_ref().unwrap()));
        assert!(!conform_all(&mut s));
        s.color_mode = ColorMode::Rgba;
        assert!(!conform_all(&mut s));
    }

    #[test]
    fn conform_all_ffi_is_one_labeled_undo_step() {
        let mut e = Editor::new(bw_sprite());
        e.sprite.draw_line(0, 0, 0, 0, 0, 0, [120, 120, 120, 255], 1);
        let before = e.sprite.clone();
        assert!(unsafe { pixmith_cm_conform_all(&mut e) });
        assert!(on_palette(&e.sprite, 0));
        assert_eq!(e.history_label(0), Some("Snap Pixels to Palette"));
        assert!(!unsafe { pixmith_cm_conform_all(&mut e) }, "nothing left to do records nothing");
        assert!(e.undo());
        assert_eq!(e.sprite, before);
    }

    #[test]
    fn filters_stay_on_the_palette() {
        use crate::paint::filters::{apply, Filter, Target};

        let mut s = bw_sprite();
        s.draw_line(0, 0, 0, 0, 3, 0, BLACK, 1);
        assert!(!apply(&mut s, &Filter::BrightnessContrast { brightness: 30, contrast: 0 }, Target::Cel, 0, 0, None));
        assert!(on_palette(&s, 0));
        assert_eq!(px(&s, 0, 0), BLACK, "a small brightness change snaps back, so nothing changes");
        let mut s = bw_sprite();
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        assert!(apply(&mut s, &Filter::Desaturate, Target::Cel, 0, 0, None));
        assert!(on_palette(&s, 0));
    }

    #[test]
    fn gradients_stay_on_the_palette() {
        let mut e = Editor::new(bw_sprite());
        unsafe { crate::paint::ink::pixmith_gradient(&mut e, 0, 0, 0, 0, 3, 0, 0x000000FF, 0xFFFFFFFF, 0, 0, false) };
        assert!(on_palette(&e.sprite, 0), "{:?}", e.sprite.render(0));
    }

    #[test]
    fn alpha_ink_stays_on_the_palette() {
        let mut e = Editor::new(bw_sprite());
        e.sprite.draw_line(0, 0, 0, 0, 3, 0, WHITE, 1);
        unsafe {
            crate::paint::ink::pixmith_ink_set(1, false, std::ptr::null(), 0);
            e.checkpoint();
            crate::paint::shapes::pixmith_stroke_begin(&mut e, 0, 0, 1, 0, 0xFF000080, 1, 0);
            crate::paint::shapes::pixmith_stroke_end();
            crate::paint::ink::pixmith_ink_set(0, false, std::ptr::null(), 0);
        }
        assert!(on_palette(&e.sprite, 0), "{:?}", e.sprite.render(0));
    }

    #[test]
    fn floating_paste_stays_on_the_palette() {
        let mut e = Editor::new(bw_sprite());
        e.sprite.draw_line(0, 0, 0, 0, 3, 0, WHITE, 1);
        let data = [255u8, 0, 0, 100, 0, 0, 255, 200];
        assert!(crate::paint::selection::paste(&mut e, 0, 0, 2, 1, &data, 1, 0));
        assert!(on_palette(&e.sprite, 0), "{:?}", e.sprite.render(0));
        crate::paint::selection::float_commit(&mut e);
    }

    #[test]
    fn merge_down_stays_on_the_palette() {
        let mut s = bw_sprite();
        s.draw_line(0, 0, 0, 0, 3, 0, WHITE, 1);
        let top = s.add_layer(0);
        s.draw_line(top, 0, 0, 0, 3, 0, RED, 1);
        s.layers[top].opacity = 128;
        assert_eq!(s.merge_down(top), Some(0));
        assert!(on_palette(&s, 0));
    }

    #[test]
    fn imported_layers_are_snapped() {
        let mut e = Editor::new(bw_sprite());
        let mut png = Sprite::new(4, 1);
        png.draw_line(0, 0, 0, 0, 3, 0, [90, 200, 40, 255], 1);
        let bytes = crate::formats::export::frame_png(&png, 0, 1).unwrap();
        let at = crate::formats::import::add_png_layer(&mut e, 0, 0, "x", &bytes).unwrap();
        assert!(on_palette(&e.sprite, 0));
        assert_ne!(e.sprite.cels[0][at].as_ref().unwrap().pixels[0..4], [90, 200, 40, 255]);
    }

    #[test]
    fn free_transform_results_are_snapped() {
        let mut e = Editor::new(bw_sprite());
        e.sprite.draw_line(0, 0, 0, 0, 1, 0, WHITE, 1);
        e.sprite.draw_line(0, 0, 2, 0, 3, 0, BLACK, 1);
        assert!(crate::paint::transform::begin(&mut e, 0, 0, true));
        unsafe {
            crate::paint::transform::pixmith_xf_set_field(&mut e, 2, 30.0);
        }
        assert!(on_palette(&e.sprite, 0), "{:?}", e.sprite.render(0));
        crate::paint::transform::commit(&mut e);
    }

    #[test]
    fn palette_edits_recolor_tile_images_too() {
        use crate::document::tilemap::{add_tilemap_layer, stamp};
        let mut s = Sprite::new(16, 16);
        s.palette = vec![[0; 4], RED, WHITE];
        s.color_mode = ColorMode::Indexed;
        let l = add_tilemap_layer(&mut s, 0, 8, 8);
        let t = s.tilesets[0].add_blank();
        let tile = Arc::make_mut(&mut s.tilesets[0].tiles[t as usize]);
        tile[0..4].copy_from_slice(&RED);
        tile[4..8].copy_from_slice(&WHITE);
        stamp(&mut s, l, 0, 0, 0, t);
        let green = [0, 255, 0, 255];
        assert!(set_palette_entry(&mut s, 1, green));
        assert_eq!(&s.tilesets[0].tiles[t as usize][0..4], &green);
        assert_eq!(&s.tilesets[0].tiles[t as usize][4..8], &WHITE);
        assert_eq!(px(&s, 0, 0), green, "the tilemap cel is re-rendered");
        // Removing a color re-snaps the tile images through the conversion.
        assert!(crate::palette::edit::remove_entries(&mut s, &[1]));
        assert!(s.palette.contains(&[
            s.tilesets[0].tiles[t as usize][0],
            s.tilesets[0].tiles[t as usize][1],
            s.tilesets[0].tiles[t as usize][2],
            255
        ]));
        // Tile images saved in the sprite's mode round trip.
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back.tilesets, s.tilesets);
        // conform_all also reaches tile images.
        Arc::make_mut(&mut s.tilesets[0].tiles[t as usize])[0..4].copy_from_slice(&[7, 7, 7, 255]);
        assert!(conform_all(&mut s));
        assert!(s.palette.contains(&s.tilesets[0].tiles[t as usize][0..4].try_into().unwrap()));
    }

    #[test]
    fn palette_ops_label_their_undo_steps() {
        let mut e = Editor::new(indexed_sprite());
        unsafe {
            assert!(crate::palette::presets::pixmith_pe_preset_load(&mut e, 4));
            assert!(crate::palette::edit::pixmith_pe_insert(&mut e, 0, 0x112233FF));
            assert!(crate::palette::edit::pixmith_pe_ramp(&mut e, 0, 1, 2, false) >= 0);
            assert!(pixmith_cm_convert(&mut e, 1, 0));
            assert!(crate::palette::edit::pixmith_pe_set_live(&mut e, 0, 0x445566FF, true));
        }
        let labels: Vec<_> = (0..5).map(|i| e.history_label(i).unwrap().to_string()).collect();
        assert_eq!(
            labels,
            [
                "Load Palette Preset: Game Boy (4)",
                "Insert Palette Entry",
                "Create Color Ramp",
                "Convert to Grayscale",
                "Edit Palette Entry"
            ]
        );
    }

    #[test]
    fn bayer_matrix_is_a_permutation() {
        for n in [1usize, 2, 4, 8] {
            let mut m = bayer(n);
            m.sort();
            assert_eq!(m, (0..(n * n) as u32).collect::<Vec<_>>());
        }
    }
}
