//! Frame difference: which pixels a frame changed compared with the one before it.
//!
//! Onion skin shows where the previous frame was; it does not show a part that "stuck" (half of a body moved a
//! pixel, the other half did not). The difference does: every pixel is classified by comparing frame N with its
//! reference frame.
//!
//! * `appeared`: transparent in the reference frame, opaque or translucent now.
//! * `disappeared`: not transparent in the reference frame, fully transparent now.
//! * `changed`: not transparent in either, but the color or the alpha differs.
//! * Unchanged: the same RGBA, or transparent in both (the color of a fully transparent pixel never counts).
//!
//! The reference frame is the previous one. The first frame has no previous one; what it is compared with is
//! the [`DiffWrap`] mode (the animation loops, which is the usual case for a walk cycle):
//!
//! * `Tag` (the default): a frame inside a tag compares with the previous frame of the same tag, and the first
//!   frame of a tag with that tag's last frame, so an 8-direction walk sheet (one tag per direction) has no fake
//!   spike where one direction ends and the next begins. When tags overlap or nest, the innermost tag (the one
//!   spanning the fewest frames; the first listed on a tie) decides. Frames outside every tag behave as
//!   `Document`; a tag of one frame has no reference for its frame.
//! * `Document`: frame 0 compares with the last frame of the document, every other frame with the one before it
//!   (tags are ignored). This is what the old `wrap = true` did.
//! * `None`: frame 0 has no reference and the result is empty (`reference == None`); the old `wrap = false`.
//!
//! A one-frame sprite never has a reference.
//!
//! The difference is taken on the composite of the visible layers ([`DiffSource::Composite`], reference layers
//! left out) or on the pixels of one layer's cel ([`DiffSource::Layer`], visibility ignored; a missing cel is
//! transparent). Linked cels and frames made of the same cels are recognized by identity without comparing pixels.

use crate::document::{Sprite, Tag};
use std::sync::Arc;

pub const KIND_UNCHANGED: u8 = 0;
pub const KIND_CHANGED: u8 = 1;
pub const KIND_APPEARED: u8 = 2;
pub const KIND_DISAPPEARED: u8 = 3;

/// How the first frame of a loop is compared (see the module docs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffWrap {
    /// Wrap inside the frame's tag (innermost on overlap); outside tags like `Document`.
    #[default]
    Tag,
    /// Frame 0 compares with the document's last frame.
    Document,
    /// Frame 0 has no reference.
    None,
}

impl DiffWrap {
    /// The ABI value: 0 tag, 1 document, 2 none.
    pub fn from_i32(v: i32) -> Option<DiffWrap> {
        Some(match v {
            0 => DiffWrap::Tag,
            1 => DiffWrap::Document,
            2 => DiffWrap::None,
            _ => return None,
        })
    }
    pub fn to_i32(self) -> i32 {
        self as i32
    }
}

/// The old boolean: `true` wraps over the whole document, `false` does not wrap.
impl From<bool> for DiffWrap {
    fn from(wrap: bool) -> DiffWrap {
        if wrap {
            DiffWrap::Document
        } else {
            DiffWrap::None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffSource {
    /// The visible layers, composited.
    Composite,
    /// One layer's cel (0-based index of a non-group layer).
    Layer(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameDiff {
    pub width: usize,
    pub height: usize,
    /// One byte per pixel, row-major: `KIND_*`.
    pub mask: Vec<u8>,
    pub changed: u32,
    pub appeared: u32,
    pub disappeared: u32,
    /// `(x, y, w, h)` of everything that differs, `None` when nothing does.
    pub bounds: Option<(u32, u32, u32, u32)>,
    /// The frame compared against, `None` when the frame has no reference.
    pub reference: Option<usize>,
}

impl FrameDiff {
    fn empty(width: usize, height: usize, reference: Option<usize>) -> FrameDiff {
        FrameDiff {
            width,
            height,
            mask: vec![0; width * height],
            changed: 0,
            appeared: 0,
            disappeared: 0,
            bounds: None,
            reference,
        }
    }

    /// Every pixel that differs.
    pub fn total(&self) -> u32 {
        self.changed + self.appeared + self.disappeared
    }

    /// The mask as RGBA for display or export: one color per kind, `alpha` for the pixels that differ and
    /// fully transparent elsewhere. `colors` = changed, appeared, disappeared as `[r, g, b]`.
    pub fn to_rgba(&self, colors: [[u8; 3]; 3], alpha: u8) -> Vec<u8> {
        let mut out = vec![0u8; self.mask.len() * 4];
        for (p, &k) in out.chunks_exact_mut(4).zip(&self.mask) {
            if k != KIND_UNCHANGED {
                let c = colors[(k - 1).min(2) as usize];
                p.copy_from_slice(&[c[0], c[1], c[2], alpha]);
            }
        }
        out
    }
}

/// Default tint colors: magenta changed, green appeared, red disappeared.
pub const DEFAULT_COLORS: [[u8; 3]; 3] = [[255, 0, 255], [0, 255, 0], [255, 0, 0]];

/// Compares two straight-alpha RGBA images of `w` x `h` pixels. `prev` is the reference, `cur` the frame.
/// The reference frame number is left `None`; the callers fill it in.
pub fn compare_rgba(prev: &[u8], cur: &[u8], w: usize, h: usize) -> FrameDiff {
    let mut d = FrameDiff::empty(w, h, None);
    let n = w * h * 4;
    if w == 0 || h == 0 || prev.len() < n || cur.len() < n || prev[..n] == cur[..n] {
        return d;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..h {
        let row = y * w;
        let (pr, cr) = (&prev[row * 4..(row + w) * 4], &cur[row * 4..(row + w) * 4]);
        if pr == cr {
            continue;
        }
        let (mut first, mut last) = (usize::MAX, 0usize);
        for x in 0..w {
            let (p, c) = (&pr[x * 4..x * 4 + 4], &cr[x * 4..x * 4 + 4]);
            if p == c {
                continue;
            }
            let kind = match (p[3] == 0, c[3] == 0) {
                (true, true) => continue,
                (true, false) => {
                    d.appeared += 1;
                    KIND_APPEARED
                }
                (false, true) => {
                    d.disappeared += 1;
                    KIND_DISAPPEARED
                }
                (false, false) => {
                    d.changed += 1;
                    KIND_CHANGED
                }
            };
            d.mask[row + x] = kind;
            first = first.min(x);
            last = x;
        }
        if first != usize::MAX {
            x0 = x0.min(first);
            x1 = x1.max(last);
            y0 = y0.min(y);
            y1 = y;
        }
    }
    if x0 != usize::MAX {
        d.bounds = Some((x0 as u32, y0 as u32, (x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32));
    }
    d
}

/// The frame `frame` is compared with: the one before it, or (first frame, `wrap`) the last one.
pub fn reference_frame(frames: usize, frame: usize, wrap: bool) -> Option<usize> {
    reference_frame_in(&[], frames, frame, wrap.into())
}

/// The innermost tag containing `frame` (fewest frames; the first listed on a tie), as `(from, to)` clamped to
/// the document.
fn innermost_tag(tags: &[Tag], frames: usize, frame: usize) -> Option<(usize, usize)> {
    tags.iter()
        .map(|t| (t.from as usize, (t.to as usize).min(frames - 1)))
        .filter(|&(from, to)| (from..=to).contains(&frame))
        .min_by_key(|&(from, to)| to - from)
}

/// The frame `frame` is compared with under `wrap` (see the module docs).
pub fn reference_frame_in(tags: &[Tag], frames: usize, frame: usize, wrap: DiffWrap) -> Option<usize> {
    if frame >= frames || frames < 2 {
        return None;
    }
    if wrap == DiffWrap::Tag {
        if let Some((from, to)) = innermost_tag(tags, frames, frame) {
            return match frame {
                f if f > from => Some(f - 1),
                _ => (to > from).then_some(to),
            };
        }
    }
    match frame {
        0 => (wrap != DiffWrap::None).then_some(frames - 1),
        n => Some(n - 1),
    }
}

/// The pixels of a layer's cel in a frame, canvas-sized; `None` for an empty cel.
fn cel_pixels(s: &Sprite, frame: usize, layer: usize) -> Option<&[u8]> {
    let c = s.cels.get(frame)?.get(layer)?.as_ref()?;
    (c.pixels.len() == s.width as usize * s.height as usize * 4).then_some(&c.pixels[..])
}

/// Whether two frames hold the very same cels in every layer, so their composites cannot differ.
fn same_cels(s: &Sprite, a: usize, b: usize) -> bool {
    s.cels[a].len() == s.cels[b].len()
        && s.cels[a].iter().zip(&s.cels[b]).all(|(x, y)| match (x, y) {
            (None, None) => true,
            (Some(x), Some(y)) => Arc::ptr_eq(x, y),
            _ => false,
        })
}

/// The difference between `frame` and its reference frame (see the module docs). `None` when the frame does
/// not exist or the layer is out of range or a group. Call `Sprite::flush_links` first if linked cels may have
/// a pending edit.
pub fn frame_diff(s: &Sprite, frame: usize, source: DiffSource, wrap: impl Into<DiffWrap>) -> Option<FrameDiff> {
    let frames = s.cels.len();
    if frame >= frames {
        return None;
    }
    let (w, h) = (s.width as usize, s.height as usize);
    if let DiffSource::Layer(l) = source {
        if s.layers.get(l)?.is_group() {
            return None;
        }
    }
    let Some(prev) = reference_frame_in(&s.tags, frames, frame, wrap.into()) else {
        return Some(FrameDiff::empty(w, h, None));
    };
    let mut d = match source {
        DiffSource::Composite => {
            if same_cels(s, frame, prev) {
                FrameDiff::empty(w, h, None)
            } else {
                let rect = (0, 0, w, h);
                let (a, b) = (
                    crate::document::layers::render_frame_rect(s, prev, rect),
                    crate::document::layers::render_frame_rect(s, frame, rect),
                );
                compare_rgba(&a, &b, w, h)
            }
        }
        DiffSource::Layer(l) => {
            let same = match (s.cels[prev].get(l), s.cels[frame].get(l)) {
                (Some(Some(a)), Some(Some(b))) => Arc::ptr_eq(a, b),
                _ => false,
            };
            let (a, b) = (cel_pixels(s, prev, l), cel_pixels(s, frame, l));
            if same || (a.is_none() && b.is_none()) {
                FrameDiff::empty(w, h, None)
            } else {
                // An empty cel compares as a transparent canvas.
                let zeros = vec![0u8; if a.is_none() || b.is_none() { w * h * 4 } else { 0 }];
                compare_rgba(a.unwrap_or(&zeros), b.unwrap_or(&zeros), w, h)
            }
        }
    };
    d.reference = Some(prev);
    Some(d)
}

// MARK: C ABI

/// What `pixmith_frame_diff` reports (see `pixmith_view.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct PixmithFrameDiff {
    pub changed: u32,
    pub appeared: u32,
    pub disappeared: u32,
    /// The frame compared against, -1 if the frame has none.
    pub reference_frame: i32,
    /// Bounding box of every differing pixel; `w` and `h` are 0 when nothing differs.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Computes the difference of `frame` from its reference frame (`wrap` true: the document's last frame for
/// frame 0, false: none; see `pixmith_frame_diff_mode` for the tag mode) of the composite of the visible layers (`layer` < 0) or of one layer's cel (`layer` >= 0, a
/// non-group layer). Writes the counts and bounds to `out` and, when `mask` is not null, one byte per pixel
/// (`width * height`, row-major: 0 same, 1 changed, 2 appeared, 3 disappeared). Returns false for a missing
/// document, frame or layer, or a `mask` that is too small.
#[no_mangle]
pub unsafe extern "C" fn pixmith_frame_diff(
    doc: *const crate::document::Editor,
    frame: u32,
    layer: i32,
    wrap: bool,
    mask: *mut u8,
    len: usize,
    out: *mut PixmithFrameDiff,
) -> bool {
    crate::ffi::guarded("pixmith_frame_diff", std::ptr::null_mut(), || {
        let Some(s) = crate::ffi::sprite(doc) else { return false };
        let source = if layer < 0 { DiffSource::Composite } else { DiffSource::Layer(layer as usize) };
        let Some(d) = frame_diff(s, frame as usize, source, wrap) else { return false };
        write_diff(&d, mask, len, out)
    })
}

/// Like `pixmith_frame_diff`, with the wrap mode: 0 tag (default), 1 document, 2 none (see [`DiffWrap`]). Any
/// other mode fails.
#[no_mangle]
pub unsafe extern "C" fn pixmith_frame_diff_mode(
    doc: *const crate::document::Editor,
    frame: u32,
    layer: i32,
    mode: i32,
    mask: *mut u8,
    len: usize,
    out: *mut PixmithFrameDiff,
) -> bool {
    crate::ffi::guarded("pixmith_frame_diff_mode", std::ptr::null_mut(), || {
        let Some(s) = crate::ffi::sprite(doc) else { return false };
        let Some(wrap) = DiffWrap::from_i32(mode) else { return false };
        let source = if layer < 0 { DiffSource::Composite } else { DiffSource::Layer(layer as usize) };
        let Some(d) = frame_diff(s, frame as usize, source, wrap) else { return false };
        write_diff(&d, mask, len, out)
    })
}

unsafe fn write_diff(d: &FrameDiff, mask: *mut u8, len: usize, out: *mut PixmithFrameDiff) -> bool {
    if !mask.is_null() {
        if len < d.mask.len() {
            return false;
        }
        std::ptr::copy_nonoverlapping(d.mask.as_ptr(), mask, d.mask.len());
    }
    if !out.is_null() {
        let (x, y, w, h) = d.bounds.unwrap_or((0, 0, 0, 0));
        *out = PixmithFrameDiff {
            changed: d.changed,
            appeared: d.appeared,
            disappeared: d.disappeared,
            reference_frame: d.reference.map_or(-1, |f| f as i32),
            x,
            y,
            w,
            h,
        };
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Cel;

    fn cel(w: usize, h: usize, set: &[(usize, usize, [u8; 4])]) -> Arc<Cel> {
        let mut pixels = vec![0u8; w * h * 4];
        for &(x, y, c) in set {
            pixels[(y * w + x) * 4..][..4].copy_from_slice(&c);
        }
        Arc::new(Cel { opacity: 255, pixels, ..Default::default() })
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    /// A 6x4 sprite with `n` frames whose single layer holds `cels[i]`.
    fn sprite(cels: Vec<Arc<Cel>>) -> Sprite {
        let mut s = Sprite::new(6, 4);
        while s.frame_count() < cels.len() {
            let n = s.frame_count();
            s.add_frame(n - 1, false);
        }
        for (i, c) in cels.into_iter().enumerate() {
            s.cels[i][0] = Some(c);
        }
        s
    }

    fn kinds(d: &FrameDiff) -> Vec<(usize, usize, u8)> {
        let mut v = Vec::new();
        for y in 0..d.height {
            for x in 0..d.width {
                let k = d.mask[y * d.width + x];
                if k != KIND_UNCHANGED {
                    v.push((x, y, k));
                }
            }
        }
        v
    }

    #[test]
    fn classifies_changed_appeared_and_disappeared() {
        let a = cel(6, 4, &[(0, 0, RED), (1, 0, RED), (2, 0, RED)]);
        let b = cel(6, 4, &[(0, 0, RED), (1, 0, BLUE), (3, 2, BLUE)]);
        let s = sprite(vec![a, b]);
        let d = frame_diff(&s, 1, DiffSource::Composite, false).unwrap();
        assert_eq!(d.reference, Some(0));
        assert_eq!(kinds(&d), vec![(1, 0, KIND_CHANGED), (2, 0, KIND_DISAPPEARED), (3, 2, KIND_APPEARED)]);
        assert_eq!((d.changed, d.appeared, d.disappeared, d.total()), (1, 1, 1, 3));
        assert_eq!(d.bounds, Some((1, 0, 3, 3)));
    }

    #[test]
    fn identical_frames_are_empty() {
        let s = sprite(vec![cel(6, 4, &[(2, 2, RED)]), cel(6, 4, &[(2, 2, RED)])]);
        let d = frame_diff(&s, 1, DiffSource::Composite, false).unwrap();
        assert_eq!((d.total(), d.bounds, d.reference), (0, None, Some(0)));
    }

    #[test]
    fn transparent_pixels_never_count_whatever_their_color() {
        let a = cel(6, 4, &[(1, 1, [9, 9, 9, 0])]);
        let b = cel(6, 4, &[(1, 1, [200, 1, 1, 0])]);
        let s = sprite(vec![a, b]);
        for source in [DiffSource::Composite, DiffSource::Layer(0)] {
            assert_eq!(frame_diff(&s, 1, source, false).unwrap().total(), 0);
        }
    }

    #[test]
    fn an_alpha_change_counts_as_changed() {
        let s = sprite(vec![cel(6, 4, &[(1, 1, [10, 20, 30, 255])]), cel(6, 4, &[(1, 1, [10, 20, 30, 128])])]);
        let d = frame_diff(&s, 1, DiffSource::Layer(0), false).unwrap();
        assert_eq!((d.changed, d.total()), (1, 1));
    }

    #[test]
    fn half_body_shifted_one_pixel_shows_exactly_the_moved_pixels() {
        // A 4-wide, 4-tall body at x=1..5. The top half (rows 0-1) moves one pixel right; the bottom half
        // (rows 2-3) stays: a torn character.
        let (w, h) = (8, 4);
        let body = |shift_top: usize| {
            let mut px = Vec::new();
            for y in 0..h {
                let dx = if y < 2 { shift_top } else { 0 };
                for x in 1..5 {
                    px.push((x + dx, y, RED));
                }
            }
            cel(w, h, &px)
        };
        let mut s = Sprite::new(w as u32, h as u32);
        s.add_frame(0, false);
        s.cels[0][0] = Some(body(0));
        s.cels[1][0] = Some(body(1));
        for source in [DiffSource::Composite, DiffSource::Layer(0)] {
            let d = frame_diff(&s, 1, source, false).unwrap();
            // Each moved row loses x=1 and gains x=5; the interior stays opaque in both frames.
            let mut want = Vec::new();
            for y in 0..2 {
                want.push((1, y, KIND_DISAPPEARED));
                want.push((5, y, KIND_APPEARED));
            }
            assert_eq!(kinds(&d), want, "{source:?}");
            assert_eq!((d.changed, d.appeared, d.disappeared), (0, 2, 2));
            assert_eq!(d.bounds, Some((1, 0, 5, 2)), "the bottom half did not move, so rows 2-3 stay out");
        }
        // The same shift with a different color per pixel (so the overlap changes too) marks all four columns.
        let striped = |shift: usize| {
            let px: Vec<_> = (0..2)
                .flat_map(|y| (1..5).map(move |x| (x + shift, y, [(x * 40) as u8, 0, 0, 255])))
                .chain((2..4).flat_map(|y| (1..5).map(move |x| (x, y, [(x * 40) as u8, 0, 0, 255]))))
                .collect::<Vec<_>>();
            cel(w, h, &px)
        };
        s.cels[0][0] = Some(striped(0));
        s.cels[1][0] = Some(striped(1));
        let d = frame_diff(&s, 1, DiffSource::Composite, false).unwrap();
        assert_eq!((d.changed, d.appeared, d.disappeared), (6, 2, 2));
        assert!(kinds(&d).iter().all(|&(_, y, _)| y < 2));
    }

    #[test]
    fn first_frame_wraps_to_the_last_only_when_asked() {
        let s = sprite(vec![cel(6, 4, &[(0, 0, RED)]), cel(6, 4, &[(1, 0, RED)]), cel(6, 4, &[(2, 0, RED)])]);
        let d = frame_diff(&s, 0, DiffSource::Composite, true).unwrap();
        assert_eq!(d.reference, Some(2));
        assert_eq!(kinds(&d), vec![(0, 0, KIND_APPEARED), (2, 0, KIND_DISAPPEARED)]);
        let d = frame_diff(&s, 0, DiffSource::Composite, false).unwrap();
        assert_eq!((d.reference, d.total(), d.bounds), (None, 0, None));
        let d = frame_diff(&s, 2, DiffSource::Composite, false).unwrap();
        assert_eq!(d.reference, Some(1));
    }

    #[test]
    fn a_single_frame_has_no_reference() {
        let s = sprite(vec![cel(6, 4, &[(0, 0, RED)])]);
        let d = frame_diff(&s, 0, DiffSource::Composite, true).unwrap();
        assert_eq!((d.reference, d.total()), (None, 0));
    }

    #[test]
    fn bad_frame_layer_or_group_is_none() {
        let mut s = sprite(vec![cel(6, 4, &[]), cel(6, 4, &[])]);
        assert!(frame_diff(&s, 2, DiffSource::Composite, false).is_none());
        assert!(frame_diff(&s, 1, DiffSource::Layer(5), false).is_none());
        let g = s.add_group(0);
        assert!(frame_diff(&s, 1, DiffSource::Layer(g), false).is_none());
    }

    #[test]
    fn composite_follows_visibility_and_a_layer_ignores_it() {
        let mut s = sprite(vec![cel(6, 4, &[(0, 0, RED)]), cel(6, 4, &[(1, 0, RED)])]);
        let top = s.add_layer(0);
        s.cels[0][top] = Some(cel(6, 4, &[(5, 3, BLUE)]));
        s.cels[1][top] = Some(cel(6, 4, &[(4, 3, BLUE)]));
        s.layers[top].visible = false;
        let comp = frame_diff(&s, 1, DiffSource::Composite, false).unwrap();
        assert_eq!(comp.bounds, Some((0, 0, 2, 1)), "the hidden layer is left out");
        let only = frame_diff(&s, 1, DiffSource::Layer(top), false).unwrap();
        assert_eq!(only.bounds, Some((4, 3, 2, 1)), "a single layer counts even when hidden");
        let base = frame_diff(&s, 1, DiffSource::Layer(0), false).unwrap();
        assert_eq!(base.total(), 2);
    }

    #[test]
    fn missing_cel_is_transparent_and_linked_cels_are_unchanged() {
        let shared = cel(6, 4, &[(2, 2, RED)]);
        let mut s = sprite(vec![shared.clone(), shared.clone()]);
        assert_eq!(frame_diff(&s, 1, DiffSource::Composite, false).unwrap().total(), 0);
        assert_eq!(frame_diff(&s, 1, DiffSource::Layer(0), false).unwrap().total(), 0);
        s.cels[1][0] = None;
        let d = frame_diff(&s, 1, DiffSource::Layer(0), false).unwrap();
        assert_eq!((d.disappeared, d.total()), (1, 1));
        let d = frame_diff(&s, 0, DiffSource::Layer(0), true).unwrap();
        assert_eq!((d.appeared, d.reference), (1, Some(1)));
    }

    #[test]
    fn compare_rgba_and_tint() {
        let prev = [0, 0, 0, 0, 1, 2, 3, 255, 9, 9, 9, 255, 0, 0, 0, 0];
        let cur = [5, 5, 5, 255, 1, 2, 3, 255, 0, 0, 0, 0, 7, 7, 7, 0];
        let d = compare_rgba(&prev, &cur, 4, 1);
        assert_eq!(d.mask, vec![KIND_APPEARED, 0, KIND_DISAPPEARED, 0]);
        let rgba = d.to_rgba(DEFAULT_COLORS, 100);
        assert_eq!(&rgba[0..4], &[0, 255, 0, 100]);
        assert_eq!(&rgba[4..8], &[0, 0, 0, 0]);
        assert_eq!(&rgba[8..12], &[255, 0, 0, 100]);
        // Short buffers give an empty result instead of panicking.
        assert_eq!(compare_rgba(&prev[..4], &cur, 4, 1).total(), 0);
    }

    #[test]
    fn reference_frame_rules() {
        assert_eq!(reference_frame(4, 0, true), Some(3));
        assert_eq!(reference_frame(4, 0, false), None);
        assert_eq!(reference_frame(4, 3, false), Some(2));
        assert_eq!(reference_frame(1, 0, true), None);
        assert_eq!(reference_frame(4, 4, true), None);
    }

    /// An 8-direction walk: 8 tags of 8 frames on a 16x4 canvas. In tag `t`, frame `k` has a pixel at (k, 0) and a
    /// static "body" of `2 * t` pixels on row 1, so the cycle moves 2 pixels per step but jumps by 4 where a
    /// tag follows another.
    fn walk_sheet() -> Sprite {
        let mut s = Sprite::new(16, 4);
        for _ in 1..64 {
            let n = s.frame_count();
            s.add_frame(n - 1, false);
        }
        for t in 0..8usize {
            for k in 0..8usize {
                let mut px = vec![(k, 0, RED)];
                px.extend((0..2 * t).map(|x| (x, 1, BLUE)));
                s.cels[t * 8 + k][0] = Some(cel(16, 4, &px));
            }
            s.add_tag(t * 8, t * 8 + 7);
        }
        s
    }

    #[test]
    fn tag_wrap_has_no_spike_where_a_tag_starts() {
        let s = walk_sheet();
        for t in 0..8usize {
            for k in 0..8usize {
                let f = t * 8 + k;
                let tag = frame_diff(&s, f, DiffSource::Composite, DiffWrap::Tag).unwrap();
                let want = if k == 0 { t * 8 + 7 } else { f - 1 };
                assert_eq!(tag.reference, Some(want), "frame {f}");
                assert_eq!(tag.total(), 2, "frame {f}: one pixel leaves, one arrives, whatever the tag");
            }
        }
        // Tag is the default, and a document wrap shows the fake spike at the start of tag 1.
        assert_eq!(DiffWrap::default(), DiffWrap::Tag);
        let doc = frame_diff(&s, 8, DiffSource::Composite, DiffWrap::Document).unwrap();
        assert_eq!((doc.reference, doc.total()), (Some(7), 4));
        let none = frame_diff(&s, 8, DiffSource::Composite, DiffWrap::None).unwrap();
        assert_eq!((none.reference, none.total()), (Some(7), 4), "frame 8 is not the first frame");
        // The layer source takes the same references.
        let l = frame_diff(&s, 8, DiffSource::Layer(0), DiffWrap::Tag).unwrap();
        assert_eq!((l.reference, l.total()), (Some(15), 2));
        // Frame 0: the last frame of the first tag (tag), of the document (document) or nothing (none).
        let first = |m| frame_diff(&s, 0, DiffSource::Composite, m).unwrap().reference;
        assert_eq!((first(DiffWrap::Tag), first(DiffWrap::Document), first(DiffWrap::None)), (Some(7), Some(63), None));
        // The old booleans: true = document, false = none.
        assert_eq!(frame_diff(&s, 0, DiffSource::Composite, true).unwrap().reference, Some(63));
        assert_eq!(frame_diff(&s, 0, DiffSource::Composite, false).unwrap().reference, None);
    }

    #[test]
    fn untagged_frames_fall_back_to_document_wrap() {
        let mut s = sprite((0..6).map(|i| cel(6, 4, &[(i, 0, RED)])).collect());
        s.add_tag(1, 3);
        let r = |f| reference_frame_in(&s.tags, 6, f, DiffWrap::Tag);
        // Inside the tag: previous frame, and the tag start wraps to the tag end.
        assert_eq!((r(1), r(2), r(3)), (Some(3), Some(1), Some(2)));
        // Outside: the previous frame, and frame 0 the document's last frame.
        assert_eq!((r(0), r(4), r(5)), (Some(5), Some(3), Some(4)));
        assert_eq!(reference_frame_in(&s.tags, 6, 0, DiffWrap::None), None);
        // A one-frame tag has no reference, and a tag reaching past the end is clamped to the document.
        s.add_tag(5, 5);
        assert_eq!(reference_frame_in(&s.tags, 6, 5, DiffWrap::Tag), None);
        s.tags[1].from = 4;
        s.tags[1].to = 40;
        assert_eq!(reference_frame_in(&s.tags, 6, 4, DiffWrap::Tag), Some(5));
        assert_eq!(reference_frame_in(&s.tags, 6, 5, DiffWrap::Tag), Some(4));
    }

    #[test]
    fn nested_or_overlapping_tags_use_the_innermost() {
        let mut s = sprite((0..12).map(|i| cel(6, 4, &[(i % 6, 0, RED)])).collect());
        s.add_tag(0, 11); // the whole walk
        s.add_tag(4, 7); // a sub-cycle inside it
        s.add_tag(6, 9); // overlaps the sub-cycle on frames 6-7 (same length: the first listed wins)
        let r = |f| reference_frame_in(&s.tags, 12, f, DiffWrap::Tag);
        assert_eq!(r(0), Some(11), "only the outer tag");
        assert_eq!(r(3), Some(2));
        assert_eq!(r(4), Some(7), "start of the inner tag wraps inside it, not to frame 3");
        assert_eq!(r(5), Some(4));
        assert_eq!(r(6), Some(5), "tie between the two 4-frame tags: the first listed (4..7)");
        assert_eq!(r(7), Some(6), "still the first listed tag");
        assert_eq!(r(8), Some(7), "only the later tag (6..9) holds frame 8, and it is not its first frame");
        assert_eq!(r(9), Some(8));
        assert_eq!(r(10), Some(9), "back in the outer tag");
        assert_eq!(r(11), Some(10));
    }

    #[test]
    fn frame_diff_mode_abi() {
        let doc = crate::ffi::document::pixmith_new(6, 4);
        unsafe {
            let e = &mut *doc;
            for _ in 0..3 {
                let n = e.sprite.frame_count();
                e.sprite.add_frame(n - 1, false);
            }
            for f in 0..4 {
                e.sprite.cels[f][0] = Some(cel(6, 4, &[(f, 0, RED)]));
            }
            e.sprite.add_tag(0, 1);
            e.sprite.add_tag(2, 3);
            let mut out = PixmithFrameDiff::default();
            let refs: Vec<i32> = [0, 1, 2]
                .iter()
                .map(|&m| {
                    assert!(pixmith_frame_diff_mode(doc, 2, -1, m, std::ptr::null_mut(), 0, &mut out));
                    out.reference_frame
                })
                .collect();
            assert_eq!(refs, vec![3, 1, 1], "tag, document, none: frame 2 starts the second tag");
            assert!(pixmith_frame_diff_mode(doc, 0, -1, 2, std::ptr::null_mut(), 0, &mut out));
            assert_eq!(out.reference_frame, -1);
            assert!(!pixmith_frame_diff_mode(doc, 0, -1, 3, std::ptr::null_mut(), 0, &mut out));
            assert!(!pixmith_frame_diff_mode(doc, 9, -1, 0, std::ptr::null_mut(), 0, &mut out));
            // The old function keeps its meaning.
            assert!(pixmith_frame_diff(doc, 2, -1, true, std::ptr::null_mut(), 0, &mut out));
            assert_eq!(out.reference_frame, 1);
            assert!(pixmith_frame_diff(doc, 0, -1, true, std::ptr::null_mut(), 0, &mut out));
            assert_eq!(out.reference_frame, 3);
            crate::ffi::document::pixmith_free(doc);
        }
    }

    #[test]
    fn ffi_reports_counts_bounds_and_mask() {
        let doc = crate::ffi::document::pixmith_new(6, 4);
        unsafe {
            let e = &mut *doc;
            e.sprite.add_frame(0, false);
            e.sprite.cels[0][0] = Some(cel(6, 4, &[(1, 1, RED)]));
            e.sprite.cels[1][0] = Some(cel(6, 4, &[(2, 1, RED)]));
            let mut out = PixmithFrameDiff::default();
            let mut mask = vec![0u8; 24];
            assert!(pixmith_frame_diff(doc, 1, -1, false, mask.as_mut_ptr(), mask.len(), &mut out));
            assert_eq!((out.appeared, out.disappeared, out.changed, out.reference_frame), (1, 1, 0, 0));
            assert_eq!((out.x, out.y, out.w, out.h), (1, 1, 2, 1));
            assert_eq!(mask[6 + 1], KIND_DISAPPEARED);
            assert_eq!(mask[6 + 2], KIND_APPEARED);
            // Stats only, frame 0 without wrap has no reference, bad arguments fail cleanly.
            assert!(pixmith_frame_diff(doc, 0, 0, false, std::ptr::null_mut(), 0, &mut out));
            assert_eq!((out.reference_frame, out.w), (-1, 0));
            assert!(!pixmith_frame_diff(doc, 9, -1, false, std::ptr::null_mut(), 0, &mut out));
            assert!(!pixmith_frame_diff(doc, 1, -1, false, mask.as_mut_ptr(), 3, &mut out));
            assert!(!pixmith_frame_diff(std::ptr::null(), 1, -1, false, std::ptr::null_mut(), 0, &mut out));
            crate::ffi::document::pixmith_free(doc);
        }
    }
}
