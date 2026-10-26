//! Background and drop-shadow removal for scanned or AI-generated art: the background colour is read from the image
//! border, removed from the edges inwards (so equal colours enclosed by the figure stay), and optionally a soft
//! shadow connected to the removed background goes with it.
//!
//! Works on straight-alpha RGBA buffers. It is a filter (`Filter::RemoveBackground`, FFI kind
//! `KIND_REMOVE_BACKGROUND`), `app.command.RemoveBackground{...}` in Lua, and the first step of `paint::trace`.

use crate::document::Editor;
use crate::formats::json::{self, Value};
use std::ffi::{c_char, CString};
use std::ptr::null_mut;

/// Largest per-channel difference that is accepted as noise around a flat colour by default.
pub const DEFAULT_TOLERANCE: u8 = 28;
/// A shadow may be at most this many percent darker than the background by default.
pub const DEFAULT_SHADOW_DEPTH: u8 = 55;
/// The shadow step backs off when it would take more than this share of what the flood left behind: that is a
/// figure of the same grey as the shadow, not a shadow.
const SHADOW_MAX_SHARE: f64 = 0.6;
/// The dominant border colour must cover at least this share of the opaque border pixels.
const MIN_BORDER_SHARE: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BgOptions {
    /// Largest per-channel difference to the background colour that still counts as background.
    pub tolerance: u8,
    /// Also remove every pixel near the background colour, enclosed by the figure or not.
    pub global: bool,
    /// Also remove a soft shadow: darker than the background but of its hue, connected to the removed background.
    pub shadow: bool,
    /// How much darker than the background (percent of its brightness) a shadow pixel may be.
    pub shadow_depth: u8,
    /// Use this colour as the background instead of reading it from the border.
    pub color: Option<[u8; 3]>,
}

impl Default for BgOptions {
    fn default() -> Self {
        BgOptions {
            tolerance: DEFAULT_TOLERANCE,
            global: false,
            shadow: false,
            shadow_depth: DEFAULT_SHADOW_DEPTH,
            color: None,
        }
    }
}

impl BgOptions {
    /// `{"tolerance": 28, "global": false, "shadow": true, "shadow_depth": 55, "color": "#ffffff"}`; every key is
    /// optional and unknown keys are ignored.
    pub fn from_json(v: &Value) -> Result<BgOptions, String> {
        let mut o = BgOptions::default();
        if let Some(n) = v.get("tolerance").and_then(Value::as_f64) {
            o.tolerance = n.clamp(0.0, 255.0) as u8;
        }
        if let Some(Value::Bool(b)) = v.get("global") {
            o.global = *b;
        }
        if let Some(Value::Bool(b)) = v.get("shadow") {
            o.shadow = *b;
        }
        if let Some(n) = v.get("shadow_depth").and_then(Value::as_f64) {
            o.shadow_depth = n.clamp(1.0, 95.0) as u8;
        }
        match v.get("color") {
            Some(Value::Str(s)) => o.color = Some(parse_hex(s).ok_or_else(|| format!("bad color \"{s}\""))?),
            Some(Value::Arr(a)) if a.len() >= 3 => {
                let c = |i: usize| a[i].as_f64().map(|n| n.clamp(0.0, 255.0) as u8);
                o.color = Some([c(0).ok_or("bad color")?, c(1).ok_or("bad color")?, c(2).ok_or("bad color")?]);
            }
            _ => {}
        }
        Ok(o)
    }
}

impl BgOptions {
    /// FFI parameters of `Filter::RemoveBackground`: tolerance (0-255), flags (1 shadow, 2 global), then optionally
    /// the shadow depth in percent and a background color as 0xRRGGBBAA (alpha 0 or absent: detect it).
    pub fn from_params(p: &[i32]) -> Option<BgOptions> {
        let flags = *p.get(1)?;
        let color = p.get(3).map(|&v| (v as u32).to_be_bytes()).filter(|c| c[3] != 0).map(|c| [c[0], c[1], c[2]]);
        Some(BgOptions {
            tolerance: (*p.first()?).clamp(0, 255) as u8,
            shadow: flags & 1 != 0,
            global: flags & 2 != 0,
            shadow_depth: p.get(2).map_or(DEFAULT_SHADOW_DEPTH, |&d| d.clamp(1, 95) as u8),
            color,
        })
    }
}

pub(crate) fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// What a removal did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BgStats {
    /// The background colour that was used (none when the image has no detectable background).
    pub color: Option<[u8; 3]>,
    /// Pixels removed by the flood from the edges.
    pub background: usize,
    /// Pixels removed by the global option, beyond the flood.
    pub global: usize,
    /// Pixels removed as shadow.
    pub shadow: usize,
    /// The shadow step found a candidate that is most of the figure and left it alone.
    pub shadow_skipped: bool,
}

impl BgStats {
    pub fn removed(&self) -> usize {
        self.background + self.global + self.shadow
    }

    pub fn to_json(&self) -> Value {
        let color = match self.color {
            Some(c) => Value::Str(format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])),
            None => Value::Null,
        };
        let n = |v: usize| Value::Num(v as f64);
        Value::Obj(vec![
            ("color".into(), color),
            ("background".into(), n(self.background)),
            ("global".into(), n(self.global)),
            ("shadow".into(), n(self.shadow)),
            ("shadow_skipped".into(), Value::Bool(self.shadow_skipped)),
            ("removed".into(), n(self.removed())),
        ])
    }
}

fn lum(r: i32, g: i32, b: i32) -> i32 {
    (299 * r + 587 * g + 114 * b) / 1000
}

fn near(p: &[u8], bg: [u8; 3], tol: u8) -> bool {
    (0..3).all(|c| p[c].abs_diff(bg[c]) <= tol)
}

fn median(v: &mut [u8]) -> u8 {
    v.sort_unstable();
    v[v.len() / 2]
}

/// The dominant colour of the image border, or `None` when the border is mostly transparent or has no colour that
/// covers half of it. Noise-tolerant: colours are grouped before the winner's pixels are averaged by median.
pub fn detect_background(rgba: &[u8], w: usize, h: usize) -> Option<[u8; 3]> {
    if w == 0 || h == 0 || rgba.len() != w * h * 4 {
        return None;
    }
    let mut ring: Vec<usize> = Vec::new();
    for x in 0..w {
        ring.push(x);
        if h > 1 {
            ring.push((h - 1) * w + x);
        }
    }
    for y in 1..h.saturating_sub(1) {
        ring.push(y * w);
        if w > 1 {
            ring.push(y * w + w - 1);
        }
    }
    let total = ring.len();
    let opaque: Vec<&[u8]> = ring.iter().map(|&i| &rgba[i * 4..i * 4 + 4]).filter(|p| p[3] >= 128).collect();
    if opaque.len() * 2 < total || opaque.is_empty() {
        return None;
    }
    // 32 levels per channel; a bin's weight includes its 26 neighbours so noise across a bin edge does not split it.
    let key = |p: &[u8]| ((p[0] >> 3) as usize) << 10 | ((p[1] >> 3) as usize) << 5 | (p[2] >> 3) as usize;
    let mut hist = vec![0u32; 1 << 15];
    for p in &opaque {
        hist[key(p)] += 1;
    }
    let weight = |k: usize| -> u32 {
        let (r, g, b) = ((k >> 10) as i32, ((k >> 5) & 31) as i32, (k & 31) as i32);
        let mut s = 0;
        for dr in -1..=1 {
            for dg in -1..=1 {
                for db in -1..=1 {
                    let (rr, gg, bb) = (r + dr, g + dg, b + db);
                    if (0..32).contains(&rr) && (0..32).contains(&gg) && (0..32).contains(&bb) {
                        s += hist[((rr as usize) << 10) | ((gg as usize) << 5) | bb as usize];
                    }
                }
            }
        }
        s
    };
    let best = (0..hist.len()).filter(|&k| hist[k] > 0).max_by_key(|&k| (weight(k), std::cmp::Reverse(k)))?;
    let centre = [((best >> 10) << 3) as u8 + 4, (((best >> 5) & 31) << 3) as u8 + 4, ((best & 31) << 3) as u8 + 4];
    let mut chans: [Vec<u8>; 3] = Default::default();
    for p in opaque.iter().filter(|p| near(p, centre, 16)) {
        for c in 0..3 {
            chans[c].push(p[c]);
        }
    }
    if (chans[0].len() as f64) < MIN_BORDER_SHARE * opaque.len() as f64 {
        return None;
    }
    Some([median(&mut chans[0]), median(&mut chans[1]), median(&mut chans[2])])
}

/// Removes the background (and, with the options, enclosed background colours and a soft shadow) in place by making
/// pixels fully transparent. Pixels that are already transparent count as removed background.
pub fn remove_background(rgba: &mut [u8], w: usize, h: usize, o: &BgOptions) -> BgStats {
    let mut stats = BgStats::default();
    if w == 0 || h == 0 || rgba.len() != w * h * 4 {
        return stats;
    }
    let Some(bg) = o.color.or_else(|| detect_background(rgba, w, h)) else { return stats };
    stats.color = Some(bg);
    let n = w * h;
    let tol = o.tolerance;
    // `gone[i]`: transparent already, or removed by this call.
    let mut gone: Vec<bool> = (0..n).map(|i| rgba[i * 4 + 3] < 128).collect();
    let is_bg = |rgba: &[u8], i: usize| rgba[i * 4 + 3] >= 128 && near(&rgba[i * 4..i * 4 + 4], bg, tol);
    let mut stack: Vec<usize> = Vec::new();

    // 1. Flood from the border through background-coloured (or transparent) pixels, 4-connected so a diagonal
    //    break in an outline does not let it in.
    let seed = |i: usize, rgba: &[u8], gone: &mut Vec<bool>, stack: &mut Vec<usize>, count: &mut usize| {
        if gone[i] {
            stack.push(i);
        } else if is_bg(rgba, i) {
            gone[i] = true;
            *count += 1;
            stack.push(i);
        }
    };
    for x in 0..w {
        seed(x, rgba, &mut gone, &mut stack, &mut stats.background);
        seed((h - 1) * w + x, rgba, &mut gone, &mut stack, &mut stats.background);
    }
    for y in 0..h {
        seed(y * w, rgba, &mut gone, &mut stack, &mut stats.background);
        seed(y * w + w - 1, rgba, &mut gone, &mut stack, &mut stats.background);
    }
    let neighbours = |i: usize, f: &mut dyn FnMut(usize)| {
        let (x, y) = (i % w, i / w);
        if x > 0 {
            f(i - 1);
        }
        if x + 1 < w {
            f(i + 1);
        }
        if y > 0 {
            f(i - w);
        }
        if y + 1 < h {
            f(i + w);
        }
    };
    while let Some(i) = stack.pop() {
        let mut next = Vec::new();
        neighbours(i, &mut |j| {
            if !gone[j] && is_bg(rgba, j) {
                next.push(j);
            }
        });
        for j in next {
            if !gone[j] {
                gone[j] = true;
                stats.background += 1;
                stack.push(j);
            }
        }
    }

    // 2. Soft shadow: pixels that are the background colour darkened by a common factor, reached from the removed
    //    background through other such pixels.
    if o.shadow {
        let bl = lum(bg[0] as i32, bg[1] as i32, bg[2] as i32);
        if bl >= 60 {
            let min_f = 1.0 - o.shadow_depth as f64 / 100.0;
            let dev_tol = (tol.max(12) as f64) * 0.75;
            let is_shadow = |i: usize| -> bool {
                let p = &rgba[i * 4..i * 4 + 4];
                if p[3] < 128 {
                    return false;
                }
                let f = lum(p[0] as i32, p[1] as i32, p[2] as i32) as f64 / bl as f64;
                f >= min_f && f <= 1.03 && (0..3).all(|c| (p[c] as f64 - bg[c] as f64 * f).abs() <= dev_tol)
            };
            let remaining = (0..n).filter(|&i| !gone[i]).count();
            let mut found: Vec<usize> = Vec::new();
            let mut mark = vec![false; n];
            let mut front: Vec<usize> = (0..n).filter(|&i| gone[i]).collect();
            while let Some(i) = front.pop() {
                let mut next = Vec::new();
                neighbours(i, &mut |j| {
                    if !gone[j] && !mark[j] && is_shadow(j) {
                        next.push(j);
                    }
                });
                for j in next {
                    if !mark[j] {
                        mark[j] = true;
                        found.push(j);
                        front.push(j);
                    }
                }
            }
            if found.len() as f64 > SHADOW_MAX_SHARE * remaining as f64 {
                stats.shadow_skipped = !found.is_empty();
            } else {
                for &j in &found {
                    gone[j] = true;
                }
                stats.shadow = found.len();
            }
        }
    }

    // 3. Optionally every pixel of the background colour anywhere.
    if o.global {
        for i in 0..n {
            if !gone[i] && is_bg(rgba, i) {
                gone[i] = true;
                stats.global += 1;
            }
        }
    }

    for (i, g) in gone.iter().enumerate() {
        if *g {
            rgba[i * 4..i * 4 + 4].fill(0);
        }
    }
    stats
}

// ---- C ABI -------------------------------------------------------------------------------

fn json_out(v: &Value) -> *mut c_char {
    CString::new(json::to_string(v)).map_or(null_mut(), CString::into_raw)
}

pub(crate) unsafe fn options_arg(options_json: *const c_char) -> Result<Value, String> {
    match crate::ffi::path_arg(options_json) {
        Some(t) if !t.trim().is_empty() => {
            let v = json::parse(&t)?;
            if v.as_object().is_none() {
                return Err("the options must be a JSON object".into());
            }
            Ok(v)
        }
        _ => Ok(Value::Obj(Vec::new())),
    }
}

/// Removes the background of the cel at (`layer`, `frame`) as one undo step, limited to the selection. `options_json`
/// is `{"tolerance": 28, "global": false, "shadow": false, "shadow_depth": 55, "color": "#rrggbb"}` (all optional).
/// Returns the statistics as JSON (`color`, `background`, `global`, `shadow`, `shadow_skipped`, `removed`; free with
/// `pixmith_string_free`), or null on failure (see `pixmith_last_error`). Nothing is recorded when nothing changed.
#[no_mangle]
pub unsafe extern "C" fn pixmith_remove_background(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    options_json: *const c_char,
) -> *mut c_char {
    crate::ffi::guarded("pixmith_remove_background", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return null_mut() };
        let opts = match options_arg(options_json).and_then(|v| BgOptions::from_json(&v)) {
            Ok(o) => o,
            Err(m) => {
                crate::ffi::set_error(m);
                return null_mut();
            }
        };
        let (w, h) = (e.sprite.width as usize, e.sprite.height as usize);
        let Some(Some(cel)) = e.sprite.cels.get(frame as usize).and_then(|r| r.get(layer as usize)) else {
            crate::ffi::set_error("there is no cel there");
            return null_mut();
        };
        if cel.pixels.len() != w * h * 4 {
            crate::ffi::set_error("that layer has no pixels to clean");
            return null_mut();
        }
        let mut probe = cel.pixels.clone();
        let stats = remove_background(&mut probe, w, h, &opts);
        let filter = crate::paint::filters::Filter::RemoveBackground(opts);
        let mut next = e.sprite.clone();
        let target = crate::paint::filters::Target::Cel;
        if crate::paint::filters::apply(&mut next, &filter, target, layer as usize, frame as usize, None) {
            if let Some(mask) = e.sel.active(next.width, next.height) {
                crate::paint::filters::restrict_to_mask(&mut next, &e.sprite, mask);
            }
            e.checkpoint();
            e.sprite = next;
        }
        json_out(&stats.to_json())
    })
}
