//! Animated GIF export and numbered PNG sequences.

use crate::document::{Editor, Sprite};
use crate::ffi::{path_arg, set_error};
use crate::formats::export::{encode_png, upscale};
use gif::{DisposalMethod, Encoder, Frame, Repeat};
use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::c_char;
use std::hash::{BuildHasherDefault, Hasher};

pub use crate::formats::durable::write_file;

pub(crate) fn report(result: Result<(), String>) -> bool {
    result.map_err(set_error).is_ok()
}

/// Encoded GIFs larger than this get a warning (they are slow to open and share).
pub const WARN_BYTES: usize = 50 << 20;

fn size_warning(len: usize) -> Option<String> {
    (len > WARN_BYTES).then(|| {
        format!(
            "the GIF is {} MB, which is slow to open and share; use a smaller scale, fewer frames or an animated WebP",
            len >> 20
        )
    })
}

/// A GIF and the warnings the export produced; the data is complete and valid even with warnings.
#[derive(Debug, Default)]
pub struct Encoded {
    pub data: Vec<u8>,
    pub warnings: Vec<String>,
}

/// Half-open pixel rectangle `x0..x1` by `y0..y1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

impl Rect {
    fn union(self, o: Rect) -> Rect {
        Rect { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }

    fn contains(&self, x: usize, y: usize) -> bool {
        (self.x0..self.x1).contains(&x) && (self.y0..self.y1).contains(&y)
    }
}

fn grow(r: &mut Option<Rect>, x: usize, y: usize) {
    let p = Rect { x0: x, y0: y, x1: x + 1, y1: y + 1 };
    *r = Some(r.map_or(p, |r| r.union(p)));
}

fn merge(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (a, b) => a.or(b),
    }
}

/// Hashes packed `0xRRGGBB` keys; the default SipHash is the slowest part of indexing large frames.
#[derive(Default)]
struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    fn write_u32(&mut self, v: u32) {
        let x = (v as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.0 = x ^ (x >> 32);
    }
}

type KeyMap<V> = HashMap<u32, V, BuildHasherDefault<KeyHasher>>;

fn key(px: &[u8]) -> u32 {
    (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32
}

fn rgb(key: u32) -> [u8; 3] {
    [(key >> 16) as u8, (key >> 8) as u8, key as u8]
}

/// Calls `f(pixel index, pixel)` for the pixels of `cur` that differ from `prev` (all of them without `prev`).
/// Rows that are equal are skipped with one slice comparison.
fn for_changed(prev: Option<&[u8]>, cur: &[u8], w: usize, mut f: impl FnMut(usize, &[u8])) {
    let row = w * 4;
    for (y, line) in cur.chunks_exact(row).enumerate() {
        let before = prev.map(|p| &p[y * row..(y + 1) * row]);
        if before == Some(line) {
            continue;
        }
        for (x, px) in line.chunks_exact(4).enumerate() {
            if before.is_none_or(|b| &b[x * 4..x * 4 + 4] != px) {
                f(y * w + x, px);
            }
        }
    }
}

/// Splits `colors` (`(rgb key, pixel count)`) into at most `n` boxes by median cut.
fn median_cut(colors: Vec<(u32, u32)>, n: usize) -> Vec<Vec<(u32, u32)>> {
    fn measure(c: &[(u32, u32)]) -> (usize, u8) {
        let (mut lo, mut hi) = ([255u8; 3], [0u8; 3]);
        for &(k, _) in c {
            for (i, v) in rgb(k).into_iter().enumerate() {
                lo[i] = lo[i].min(v);
                hi[i] = hi[i].max(v);
            }
        }
        (0..3).map(|i| (i, hi[i] - lo[i])).max_by_key(|&(_, r)| r).unwrap_or((0, 0))
    }
    let mut boxes: Vec<(Vec<(u32, u32)>, usize, u8)> = Vec::new();
    let (ch, range) = measure(&colors);
    boxes.push((colors, ch, range));
    while boxes.len() < n {
        let Some(at) = (0..boxes.len())
            .filter(|&i| boxes[i].0.len() > 1)
            .max_by_key(|&i| (boxes[i].2 as u64 + 1) * boxes[i].0.len() as u64)
        else {
            break;
        };
        let (mut colors, ch, _) = boxes.swap_remove(at);
        let shift = 16 - 8 * ch;
        colors.sort_unstable_by_key(|&(k, _)| (k >> shift) & 0xff);
        let total: u64 = colors.iter().map(|c| c.1 as u64).sum();
        let (mut acc, mut cut) = (0u64, 1);
        for (i, c) in colors.iter().enumerate() {
            acc += c.1 as u64;
            if acc * 2 >= total {
                cut = (i + 1).clamp(1, colors.len() - 1);
                break;
            }
        }
        let upper = colors.split_off(cut);
        for half in [colors, upper] {
            let (ch, range) = measure(&half);
            boxes.push((half, ch, range));
        }
    }
    boxes.into_iter().map(|b| b.0).collect()
}

/// The palette of a whole animation: exact when the colors fit, quantized once otherwise.
struct Palette {
    /// `[r, g, b, ...]`, entry 0 is the transparent slot when `slot` is set.
    bytes: Vec<u8>,
    index: KeyMap<u8>,
    /// Index 0 is reserved for transparency.
    slot: bool,
}

fn build_palette(counts: KeyMap<u32>, any_alpha: bool) -> Palette {
    let mut colors: Vec<(u32, u32)> = counts.into_iter().collect();
    colors.sort_unstable();
    // Exactly 256 opaque colors fill the table; anything with transparency or more colors leaves slot 0 free.
    let exact = if any_alpha { colors.len() <= 255 } else { colors.len() <= 256 };
    let slot = any_alpha || !exact || colors.len() < 256;
    let first = slot as usize;
    let mut bytes = if slot { vec![0, 0, 0] } else { Vec::new() };
    let mut index = KeyMap::default();
    if exact {
        for (i, &(k, _)) in colors.iter().enumerate() {
            bytes.extend_from_slice(&rgb(k));
            index.insert(k, (first + i) as u8);
        }
    } else {
        for (i, group) in median_cut(colors, 255).into_iter().enumerate() {
            let total: u64 = group.iter().map(|c| c.1 as u64).sum::<u64>().max(1);
            let mut sum = [0u64; 3];
            for &(k, n) in &group {
                for (s, v) in sum.iter_mut().zip(rgb(k)) {
                    *s += v as u64 * n as u64;
                }
                index.insert(k, (1 + i) as u8);
            }
            bytes.extend(sum.iter().map(|&s| ((s + total / 2) / total) as u8));
        }
    }
    // A table needs at least two entries.
    while bytes.len() < 6 {
        bytes.push(0);
    }
    Palette { bytes, index, slot }
}

/// Palette indices of one frame; only the pixels that differ from `prev_rgba` are looked up again.
fn index_frame(
    rgba: &[u8],
    prev: Option<(&[u8], &[u8])>,
    w: usize,
    h: usize,
    pal: &Palette,
) -> Vec<u8> {
    let mut idx = prev.map_or_else(|| vec![0u8; w * h], |(_, i)| i.to_vec());
    let mut last = (u32::MAX, 0u8);
    for_changed(prev.map(|p| p.0), rgba, w, |i, px| {
        idx[i] = if px[3] < 128 {
            0
        } else {
            let k = key(px);
            if last.0 != k {
                last = (k, pal.index.get(&k).copied().unwrap_or(0));
            }
            last.1
        };
    });
    idx
}

/// Bounding rectangles (diff, clear) of how `cur` differs from `prev`: `diff` for every changed pixel, `clear`
/// for the pixels that go from a color to transparent (only those need a disposal to remove).
fn compare(prev: &[u8], cur: &[u8], w: usize, slot: bool) -> (Option<Rect>, Option<Rect>) {
    let (mut diff, mut clear) = (None, None);
    for (y, (a, b)) in prev.chunks_exact(w).zip(cur.chunks_exact(w)).enumerate() {
        if a == b {
            continue;
        }
        for (x, (&p, &c)) in a.iter().zip(b).enumerate() {
            if p != c {
                grow(&mut diff, x, y);
                if slot && c == 0 {
                    grow(&mut clear, x, y);
                }
            }
        }
    }
    (diff, clear)
}

/// Bounding rectangle of the colored (not transparent) pixels of `idx` inside `within`.
fn opaque_bounds(idx: &[u8], w: usize, within: Rect, slot: bool) -> Option<Rect> {
    let mut r = None;
    for y in within.y0..within.y1 {
        for x in within.x0..within.x1 {
            if !(slot && idx[y * w + x] == 0) {
                grow(&mut r, x, y);
            }
        }
    }
    r
}

/// A distinct frame of the animation: identical neighbours are merged into one with the summed delay.
struct Distinct {
    idx: Vec<u8>,
    delay_ms: u64,
    /// Changed pixels versus the previous frame.
    diff: Option<Rect>,
    /// Pixels that turn from a color into transparency versus the previous frame.
    clear: Option<Rect>,
}

/// Longest delay a single GIF frame can hold, in milliseconds.
const MAX_DELAY_MS: u64 = u16::MAX as u64 * 10;

/// Encodes `n` frames of `w` x `h` RGBA (`rgba(i)` yields frame `i`, and is called twice per frame) as a GIF scaled
/// by `scale`.
///
/// How it stays small: one palette for the whole animation (exact up to 255 colors, quantized once beyond that);
/// identical consecutive frames become one with the delays added; every other frame is written as the rectangle
/// that differs from the previous picture, with the pixels inside it that did not change set to the transparent
/// index, and disposal "do not dispose" so the previous picture stays underneath. A pixel that turns transparent
/// cannot be drawn over what is below it, so the frame before it uses disposal "restore to background", which
/// clears its own rectangle (grown to cover those pixels); the frame after it then redraws every colored pixel
/// of that rectangle. The first frame is the only one that is cut to its content instead of its changes.
fn encode<'a>(
    (w, h): (usize, usize),
    scale: usize,
    repeat: Option<Repeat>,
    delays: &[u16],
    rgba: impl Fn(usize) -> Cow<'a, [u8]>,
) -> Result<Encoded, String> {
    let n = delays.len();
    if w == 0 || h == 0 || n == 0 {
        return Err("there are no frames to export".into());
    }
    if w * scale > u16::MAX as usize || h * scale > u16::MAX as usize {
        return Err("image is too large for a GIF".into());
    }
    let check = |px: &[u8]| (px.len() == w * h * 4).then_some(()).ok_or("a frame does not match the image size");

    // Pass 1: every color of the animation, looking only at the pixels that changed from frame to frame.
    let mut counts: KeyMap<u32> = KeyMap::default();
    let mut any_alpha = false;
    let mut prev: Option<Cow<[u8]>> = None;
    for i in 0..n {
        let cur = rgba(i);
        check(&cur)?;
        for_changed(prev.as_deref(), &cur, w, |_, px| {
            if px[3] < 128 {
                any_alpha = true;
            } else {
                *counts.entry(key(px)).or_insert(0) += 1;
            }
        });
        prev = Some(cur);
    }
    let pal = build_palette(counts, any_alpha);
    let slot = pal.slot;

    // Pass 2: palette indices, merging identical frames.
    let mut frames: Vec<Distinct> = Vec::new();
    let mut prev: Option<Cow<[u8]>> = None;
    for (i, &delay) in delays.iter().enumerate() {
        let cur = rgba(i);
        let idx = {
            let before = prev.as_deref().zip(frames.last().map(|f| f.idx.as_slice()));
            index_frame(&cur, before, w, h, &pal)
        };
        prev = Some(cur);
        let delay_ms = delay as u64;
        let (mut diff, mut clear) = (None, None);
        if let Some(last) = frames.last_mut() {
            (diff, clear) = compare(&last.idx, &idx, w, slot);
            if diff.is_none() && last.delay_ms + delay_ms <= MAX_DELAY_MS {
                last.delay_ms += delay_ms;
                continue;
            }
        }
        frames.push(Distinct { idx, delay_ms, diff, clear });
    }

    // Plan: the rectangle of each frame, then write.
    let mut out = Vec::new();
    let mut enc = Encoder::new(&mut out, (w * scale) as u16, (h * scale) as u16, &pal.bytes).map_err(|e| e.to_string())?;
    if let Some(r) = repeat {
        enc.set_repeat(r).map_err(|e| e.to_string())?;
    }
    let canvas = Rect { x0: 0, y0: 0, x1: w, y1: h };
    let mut rects: Vec<Rect> = Vec::with_capacity(frames.len());
    for (i, f) in frames.iter().enumerate() {
        // Pixels cleared by the previous frame's disposal that this frame must paint again.
        let cleared = if i > 0 && f.clear.is_some() { Some(rects[i - 1]) } else { None };
        let mut need = if i == 0 { opaque_bounds(&f.idx, w, canvas, slot) } else { f.diff };
        if let Some(c) = cleared {
            need = merge(need, opaque_bounds(&f.idx, w, c, slot));
        }
        // This frame is disposed to background when the next one removes pixels, which its rectangle must cover.
        let next_clear = frames.get(i + 1).and_then(|n| n.clear);
        let r = merge(need, next_clear).unwrap_or(Rect { x0: 0, y0: 0, x1: 1, y1: 1 });
        rects.push(r);

        let (rw, rh) = (r.x1 - r.x0, r.y1 - r.y0);
        let prev_idx = (i > 0).then(|| frames[i - 1].idx.as_slice());
        let mut buffer = Vec::with_capacity(rw * rh * scale * scale);
        let mut row = Vec::with_capacity(rw * scale);
        for y in r.y0..r.y1 {
            row.clear();
            for x in r.x0..r.x1 {
                let v = f.idx[y * w + x];
                // Unchanged pixels let the picture underneath show through, except where it was just cleared.
                let same = slot && prev_idx.is_some_and(|p| p[y * w + x] == v) && !cleared.is_some_and(|c| c.contains(x, y));
                let v = if same { 0 } else { v };
                row.extend(std::iter::repeat_n(v, scale));
            }
            for _ in 0..scale {
                buffer.extend_from_slice(&row);
            }
        }
        let frame = Frame {
            delay: f.delay_ms.div_ceil(10).clamp(1, u16::MAX as u64) as u16,
            dispose: if next_clear.is_some() { DisposalMethod::Background } else { DisposalMethod::Keep },
            transparent: slot.then_some(0),
            left: (r.x0 * scale) as u16,
            top: (r.y0 * scale) as u16,
            width: (rw * scale) as u16,
            height: (rh * scale) as u16,
            buffer: buffer.into(),
            ..Frame::default()
        };
        enc.write_frame(&frame).map_err(|e| e.to_string())?;
    }
    enc.into_inner().map_err(|e| e.to_string())?;
    let warnings = size_warning(out.len()).into_iter().collect();
    Ok(Encoded { data: out, warnings })
}

/// The GIF of a sprite at `scale`, with the export's warnings.
pub fn gif_encode(s: &Sprite, scale: usize) -> Result<Encoded, String> {
    let scale = scale.clamp(1, 64);
    let (w, h) = (s.width as usize, s.height as usize);
    if w * scale > u16::MAX as usize || h * scale > u16::MAX as usize {
        return Err("image is too large for a GIF".into());
    }
    crate::formats::export::check_export_size(w * scale, h * scale)?;
    encode((w, h), scale, Some(Repeat::Infinite), &s.durations[..s.frame_count()], |f| Cow::Owned(s.render(f)))
}

pub fn gif_bytes(s: &Sprite, scale: usize) -> Result<Vec<u8>, String> {
    gif_encode(s, scale).map(|e| e.data)
}

/// `(file name, PNG data)` for each frame, numbered from 1 as `<base>-001.png`.
pub fn png_sequence(s: &Sprite, scale: usize, base: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let scale = scale.clamp(1, 64);
    let (w, h) = (s.width as usize, s.height as usize);
    crate::formats::export::check_export_size(w * scale, h * scale)?;
    let digits = s.frame_count().to_string().len().max(3);
    (0..s.frame_count())
        .map(|f| {
            let png = encode_png(&upscale(&s.render(f), w, h, scale), w * scale, h * scale)?;
            Ok((format!("{base}-{:0digits$}.png", f + 1), png))
        })
        .collect()
}

/// An animated GIF of a planned job (trimmed or reordered frames), with the export's warnings. `job.repeat`
/// plays: 0 loops forever.
pub fn gif_encode_job(job: &crate::formats::export_plan::Job, scale: usize) -> Result<Encoded, String> {
    let first = job.frames.first().ok_or("there are no frames to export")?;
    let (fw, fh) = (first.w, first.h);
    if job.frames.iter().any(|f| (f.w, f.h) != (fw, fh)) {
        return Err("the frames differ in size; use a uniform trim for animations".into());
    }
    let repeat = match job.repeat {
        0 => Some(Repeat::Infinite),
        1 => None, // no loop extension: the animation plays once
        n => Some(Repeat::Finite(n - 1)),
    };
    let delays: Vec<u16> = job.frames.iter().map(|f| f.duration).collect();
    encode((fw, fh), scale.clamp(1, 64), repeat, &delays, |i| Cow::Borrowed(job.frames[i].rgba.as_slice()))
}

pub fn gif_bytes_job(job: &crate::formats::export_plan::Job, scale: usize) -> Result<Vec<u8>, String> {
    gif_encode_job(job, scale).map(|e| e.data)
}

/// `(file name, PNG data)` for each frame of a job, `<base>-001.png`, ... Frames keep their (trimmed) size.
pub fn png_sequence_job(
    job: &crate::formats::export_plan::Job,
    scale: usize,
    base: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let scale = scale.clamp(1, 64);
    let digits = job.frames.len().to_string().len().max(3);
    job.frames
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let png = encode_png(&upscale(&f.rgba, f.w, f.h, scale), f.w * scale, f.h * scale)?;
            Ok((format!("{base}-{:0digits$}.png", i + 1), png))
        })
        .collect()
}

thread_local! {
    static WARNINGS: std::cell::RefCell<std::ffi::CString> = std::cell::RefCell::new(std::ffi::CString::default());
}

/// Records the warnings of the export that just ran on this thread (replacing those of the one before), for
/// `pixmith_export_warnings`.
pub(crate) fn set_export_warnings(warnings: &[String]) {
    let text = warnings.join("\n").replace('\0', "");
    WARNINGS.with(|w| *w.borrow_mut() = std::ffi::CString::new(text).unwrap_or_default());
}

/// Warnings of the last `pixmith_export_run`, `pixmith_export_run_ex` or `pixmith_export_gif` on this thread,
/// separated by newlines; an empty string when there were none. A warning does not fail the export: the files are
/// written. The pointer stays valid until the next export on this thread.
#[no_mangle]
pub extern "C" fn pixmith_export_warnings() -> *const c_char {
    crate::ffi::guarded("pixmith_export_warnings", std::ptr::null_mut(), || WARNINGS.with(|w| w.borrow().as_ptr()))
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_export_gif(doc: *const Editor, scale: u32, path: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_export_gif", std::ptr::null_mut(), || {
        let (Some(e), Some(path)) = (crate::ffi::synced(doc), path_arg(path)) else { return false };
        set_export_warnings(&[]);
        report(gif_encode(&e.sprite, scale as usize).and_then(|g| {
            set_export_warnings(&g.warnings);
            write_file(&path, &g.data)
        }))
    })
}

/// `base` is a path without extension; frames are written as `<base>-001.png`, `<base>-002.png`, ...
#[no_mangle]
pub unsafe extern "C" fn pixmith_export_png_sequence(doc: *const Editor, scale: u32, base: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_export_png_sequence", std::ptr::null_mut(), || {
        let (Some(e), Some(base)) = (crate::ffi::synced(doc), path_arg(base)) else { return false };
        report(
            png_sequence(&e.sprite, scale as usize, &base)
                .and_then(|files| files.iter().try_for_each(|(name, data)| write_file(name, data))),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Cel;
    use std::sync::Arc;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn animated() -> Sprite {
        let mut s = Sprite::new(2, 2);
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        s.add_frame(0, true);
        s.draw_line(0, 1, 1, 1, 1, 1, BLUE, 1);
        s.durations = vec![100, 250];
        s
    }

    /// One composed picture per GIF frame (disposal methods applied like a viewer does), with the frame's
    /// delay and its rectangle `(left, top, width, height)`.
    struct Decoded {
        w: usize,
        h: usize,
        shown: Vec<Vec<u8>>,
        delays: Vec<u16>,
        rects: Vec<(usize, usize, usize, usize)>,
    }

    fn decode(data: &[u8]) -> Decoded {
        let mut opts = gif::DecodeOptions::new();
        opts.set_color_output(gif::ColorOutput::RGBA);
        let mut dec = opts.read_info(data).unwrap();
        let (w, h) = (dec.width() as usize, dec.height() as usize);
        let mut canvas = vec![0u8; w * h * 4];
        let mut d = Decoded { w, h, shown: Vec::new(), delays: Vec::new(), rects: Vec::new() };
        while let Some(f) = dec.read_next_frame().unwrap() {
            let (l, t, fw, fh) = (f.left as usize, f.top as usize, f.width as usize, f.height as usize);
            assert!(l + fw <= w && t + fh <= h, "frame leaves the screen");
            let before = canvas.clone();
            for (row, line) in f.buffer.chunks_exact(fw * 4).enumerate() {
                for (col, px) in line.chunks_exact(4).enumerate() {
                    if px[3] != 0 {
                        canvas[((t + row) * w + l + col) * 4..][..4].copy_from_slice(px);
                    }
                }
            }
            d.shown.push(canvas.clone());
            d.delays.push(f.delay);
            d.rects.push((l, t, fw, fh));
            match f.dispose {
                gif::DisposalMethod::Background => {
                    for y in t..t + fh {
                        canvas[(y * w + l) * 4..(y * w + l + fw) * 4].fill(0);
                    }
                }
                gif::DisposalMethod::Previous => canvas = before,
                _ => {}
            }
        }
        d
    }

    /// Alpha below 128 is transparent (all zero); everything else is opaque.
    fn flat(rgba: &[u8]) -> Vec<u8> {
        rgba.chunks_exact(4).flat_map(|p| if p[3] < 128 { [0; 4] } else { [p[0], p[1], p[2], 255] }).collect()
    }

    fn encode_frames(frames: &[Vec<u8>], delays: &[u16], (w, h): (usize, usize), scale: usize) -> Decoded {
        let g = encode((w, h), scale, Some(Repeat::Infinite), delays, |i| Cow::Borrowed(frames[i].as_slice())).unwrap();
        decode(&g.data)
    }

    /// Fails unless every shown picture equals the expected frame, comparing every pixel.
    fn assert_same(d: &Decoded, frames: &[Vec<u8>], (w, h): (usize, usize), scale: usize, what: &str) {
        assert_eq!((d.w, d.h), (w * scale, h * scale), "{what}");
        for (i, (shown, want)) in d.shown.iter().zip(frames).enumerate() {
            let want = flat(&upscale(want, w, h, scale));
            if *shown != want {
                let at = shown.iter().zip(&want).position(|(a, b)| a != b).unwrap() / 4;
                panic!("{what}: picture {i} differs first at pixel ({}, {})", at % (w * scale), at / (w * scale));
            }
        }
    }

    struct Rng(u64);

    impl Rng {
        fn next(&mut self, n: u64) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }

    /// Moving, appearing and vanishing sprites over a transparent or opaque background, with repeated frames.
    fn scene(seed: u64, (w, h): (usize, usize), n: usize, background: Option<[u8; 4]>) -> Vec<Vec<u8>> {
        let mut rng = Rng(seed);
        let colors = [RED, BLUE, [0, 255, 0, 255], [255, 255, 0, 255], [10, 10, 10, 255], [255, 0, 255, 200]];
        let mut pos: Vec<(usize, usize, usize)> = (0..3)
            .map(|_| (rng.next(w as u64 - 6) as usize, rng.next(h as u64 - 6) as usize, 2 + rng.next(4) as usize))
            .collect();
        let mut frames: Vec<Vec<u8>> = Vec::new();
        for _ in 0..n {
            if !frames.is_empty() && rng.next(100) < 15 {
                frames.push(frames.last().unwrap().clone());
                continue;
            }
            let mut f: Vec<u8> = (0..w * h).flat_map(|_| background.unwrap_or([0; 4])).collect();
            for (k, p) in pos.iter_mut().enumerate() {
                p.0 = (p.0 + rng.next(7) as usize + w - 3) % (w - 6);
                p.1 = (p.1 + rng.next(7) as usize + h - 3) % (h - 6);
                if rng.next(10) < 3 {
                    continue; // this sprite is not there in this frame
                }
                let c = colors[(k + rng.next(2) as usize) % colors.len()];
                for y in p.1..p.1 + p.2 {
                    for x in p.0..p.0 + p.2 {
                        f[(y * w + x) * 4..][..4].copy_from_slice(&c);
                    }
                }
            }
            frames.push(f);
        }
        frames
    }

    #[test]
    fn gif_has_frames_delays_and_transparency() {
        let s = animated();
        let d = decode(&gif_bytes(&s, 1).unwrap());
        assert_eq!((d.w, d.h, d.shown.len()), (2, 2, 2));
        assert_eq!(d.delays, vec![10, 25]);
        assert_eq!(&d.shown[0][0..4], &RED);
        assert_eq!(d.shown[0][4..8][3], 0);
        assert_eq!(&d.shown[1][12..16], &BLUE);
        assert_eq!(&d.shown[1][0..4], &RED);
        // Frame 2 only adds one pixel.
        assert_eq!(d.rects[1], (1, 1, 1, 1));
    }

    #[test]
    fn gif_scales() {
        let s = animated();
        let d = decode(&gif_bytes(&s, 3).unwrap());
        assert_eq!((d.w, d.h, d.shown.len()), (6, 6, 2));
        assert_eq!(&d.shown[0][(2 * 6 + 2) * 4..(2 * 6 + 2) * 4 + 4], &RED);
        assert_eq!(d.rects[1], (3, 3, 3, 3));
        let frames: Vec<_> = (0..2).map(|f| s.render(f)).collect();
        assert_same(&d, &frames, (2, 2), 3, "scale 3");
    }

    #[test]
    fn gif_with_many_colors_still_encodes() {
        let mut s = Sprite::new(32, 32);
        for i in 0..32u8 {
            for j in 0..32u8 {
                s.draw_line(0, 0, i as i32, j as i32, i as i32, j as i32, [i * 8, j * 8, i ^ j, 255], 1);
            }
        }
        let d = decode(&gif_bytes(&s, 1).unwrap());
        assert_eq!(d.shown.len(), 1);
        assert!(d.shown[0].chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn quantizing_keeps_one_palette_and_the_transparency() {
        // More than 255 colors, one pixel changing per frame, a hole that opens and closes.
        let (w, h) = (32, 32);
        let mut frames = Vec::new();
        for f in 0..4usize {
            let mut px: Vec<u8> = (0..w * h)
                .flat_map(|i| [(i % 32 * 8) as u8, (i / 32 * 8) as u8, ((i * 7) % 251) as u8, 255])
                .collect();
            if f % 2 == 1 {
                px[(5 * w + 5) * 4 + 3] = 0;
            }
            px[(10 * w + f) * 4..][..4].copy_from_slice(&[1, 2, 3, 255]);
            frames.push(px);
        }
        let g = encode((w, h), 1, None, &[10; 4], |i| Cow::Borrowed(frames[i].as_slice())).unwrap();
        let d = decode(&g.data);
        assert_eq!(d.shown.len(), 4);
        for (shown, want) in d.shown.iter().zip(&frames) {
            let alpha = |p: &[u8]| p.chunks_exact(4).map(|px| px[3] >= 128).collect::<Vec<_>>();
            assert_eq!(alpha(shown), alpha(want));
        }
        // The palette is global, written once.
        let mut opts = gif::DecodeOptions::new();
        opts.set_color_output(gif::ColorOutput::Indexed);
        let mut dec = opts.read_info(&g.data[..]).unwrap();
        assert!(dec.global_palette().is_some());
        while let Some(f) = dec.read_next_frame().unwrap() {
            assert!(f.palette.is_none());
        }
    }

    #[test]
    fn exactly_256_colors_are_kept_exactly() {
        let (w, h) = (16, 16);
        let base: Vec<u8> = (0..256usize).flat_map(|i| [i as u8, (255 - i) as u8, (i * 3) as u8, 255]).collect();
        let mut second = base.clone();
        second[40 * 4..][..4].copy_from_slice(&base[7 * 4..][..4]);
        let frames = vec![base, second];
        let d = encode_frames(&frames, &[10, 10], (w, h), 1);
        assert_same(&d, &frames, (w, h), 1, "256 colors");
    }

    #[test]
    fn decoded_frames_match_the_source_pixel_for_pixel() {
        let size = (24, 18);
        for seed in 0..40 {
            for (name, background) in [("clear", None), ("opaque", Some([30, 60, 90, 255]))] {
                for scale in [1, 3] {
                    let frames = scene(seed, size, 14, background);
                    let delays = vec![30; frames.len()];
                    let d = encode_frames(&frames, &delays, size, scale);
                    let mut distinct = vec![&frames[0]];
                    for f in &frames[1..] {
                        if flat(f) != flat(distinct.last().unwrap()) {
                            distinct.push(f);
                        }
                    }
                    assert_eq!(d.shown.len(), distinct.len(), "seed {seed} {name} x{scale}");
                    let distinct: Vec<Vec<u8>> = distinct.into_iter().cloned().collect();
                    assert_same(&d, &distinct, size, scale, &format!("seed {seed} {name} x{scale}"));
                }
            }
        }
    }

    #[test]
    fn opaque_pixels_that_turn_transparent_are_cleared() {
        // A 6x1 strip: a block moves right, then vanishes, then reappears elsewhere.
        let px = |v: &[u8]| -> Vec<u8> { v.iter().flat_map(|&c| if c == 0 { [0; 4] } else { [c, 0, 0, 255] }).collect() };
        let frames = vec![
            px(&[1, 1, 0, 0, 0, 0]),
            px(&[0, 1, 1, 0, 0, 0]),
            px(&[0, 0, 0, 0, 0, 0]),
            px(&[0, 0, 0, 0, 2, 2]),
            px(&[3, 0, 0, 0, 2, 2]),
            px(&[0, 0, 0, 0, 0, 0]),
        ];
        let d = encode_frames(&frames, &[10; 6], (6, 1), 1);
        assert_same(&d, &frames, (6, 1), 1, "strip");
    }

    #[test]
    fn identical_frames_are_merged_and_delays_summed() {
        let (a, b) = (animated().render(0), animated().render(1));
        let frames = vec![a.clone(), a.clone(), b.clone(), b.clone(), b.clone(), a.clone()];
        let d = encode_frames(&frames, &[100, 250, 70, 5, 5, 30], (2, 2), 1);
        assert_eq!(d.shown.len(), 3);
        assert_eq!(d.delays, vec![35, 8, 3]);
        assert_same(&d, &[a.clone(), b, a], (2, 2), 1, "merged");
    }

    #[test]
    fn merged_delays_that_overflow_a_gif_frame_are_split() {
        let a = animated().render(0);
        let frames = vec![a; 11];
        let d = encode_frames(&frames, &[u16::MAX; 11], (2, 2), 1);
        assert_eq!(d.delays, vec![u16::MAX, 6554]);
        assert_same(&d, &vec![frames[0].clone(); 2], (2, 2), 1, "overflow");
    }

    #[test]
    fn a_still_sprite_with_several_frames_is_one_gif_frame() {
        let mut s = Sprite::new(4, 4);
        s.draw_line(0, 0, 0, 0, 3, 3, RED, 1);
        s.add_frame(0, true);
        s.add_frame(1, true);
        s.durations = vec![40, 50, 60];
        let d = decode(&gif_bytes(&s, 1).unwrap());
        assert_eq!((d.shown.len(), d.delays[0]), (1, 15));
    }

    /// The 1280x768 sheet of the bug report: only a 16x16 sprite changes between frames.
    fn big_sheet(frames: usize) -> Sprite {
        let (w, h) = (1280usize, 768usize);
        let mut s = Sprite::new(w as u32, h as u32);
        for f in 1..frames {
            s.add_frame(f - 1, true);
        }
        for f in 0..frames {
            let mut px = vec![0u8; w * h * 4];
            for y in 0..h {
                for x in 0..w {
                    // A tiled background of 100 colors.
                    let c = (x / 16 + y / 16 * 3) % 100;
                    px[(y * w + x) * 4..][..4].copy_from_slice(&[(c * 2) as u8, (c * 5 % 256) as u8, 40, 255]);
                }
            }
            let (sx, sy) = (100 + f * 40, 300 + f % 5 * 7);
            for y in sy..sy + 16 {
                for x in sx..sx + 16 {
                    px[(y * w + x) * 4..][..4].copy_from_slice(&[255, 255, (f * 10) as u8, 255]);
                }
            }
            s.cels[f][0] = Some(Arc::new(Cel { opacity: 255, pixels: px, ..Default::default() }));
        }
        s.durations = vec![100; frames];
        s
    }

    #[test]
    fn a_small_moving_sprite_on_a_big_sheet_stays_small() {
        let s = big_sheet(20);
        let t = std::time::Instant::now();
        let data = gif_bytes(&s, 1).unwrap();
        eprintln!("1280x768 x 20 frames: {} bytes in {:?}", data.len(), t.elapsed());
        assert!(data.len() < 1 << 20, "{} bytes", data.len());
        let d = decode(&data);
        assert_eq!(d.shown.len(), 20);
        // After the first frame, only the two squares (where the sprite was and where it is) are written.
        for r in &d.rects[1..] {
            assert!(r.2 <= 64 && r.3 <= 48, "{r:?}");
        }
        let frames: Vec<_> = (0..20).map(|f| s.render(f)).collect();
        assert_same(&d, &frames, (1280, 768), 1, "big sheet");
    }

    #[test]
    fn size_warning_starts_above_the_threshold() {
        assert!(size_warning(WARN_BYTES).is_none());
        let w = size_warning(WARN_BYTES + (1 << 20)).unwrap();
        assert!(w.contains("51 MB"), "{w}");
        assert!(gif_encode(&animated(), 1).unwrap().warnings.is_empty());
    }

    #[test]
    fn warnings_are_read_through_the_c_abi() {
        set_export_warnings(&["a".into(), "b".into()]);
        let text = unsafe { std::ffi::CStr::from_ptr(pixmith_export_warnings()) }.to_str().unwrap().to_string();
        assert_eq!(text, "a\nb");
        set_export_warnings(&[]);
        assert_eq!(unsafe { std::ffi::CStr::from_ptr(pixmith_export_warnings()) }.to_bytes(), b"");
    }

    #[test]
    fn png_sequence_is_numbered() {
        let files = png_sequence(&animated(), 2, "out/walk").unwrap();
        let names: Vec<_> = files.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(names, ["out/walk-001.png", "out/walk-002.png"]);
        let (w, h, _) = crate::formats::import::decode_png(&files[1].1).unwrap();
        assert_eq!((w, h), (4, 4));
    }
}
