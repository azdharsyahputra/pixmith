//! Support for viewers that keep a rendered copy of the document: render just a region, and learn which
//! region an edit changed.

use crate::document::Editor;
use crate::render::dirty::DirtyTake;

/// Composites the rectangle `(x, y, w, h)` of `frame` into `out` (`w * h * 4` bytes, tightly packed,
/// straight-alpha RGBA). The result equals the same rectangle of `pixmith_render`'s output, including blend
/// modes and group isolation. Returns false if the rectangle is empty or not inside the canvas, or `out`
/// is too small.
#[no_mangle]
pub unsafe extern "C" fn pixmith_render_rect(
    doc: *const Editor,
    frame: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    out: *mut u8,
    len: usize,
) -> bool {
    crate::ffi::guarded("pixmith_render_rect", std::ptr::null_mut(), || {
        let Some(e) = crate::ffi::synced(doc) else { return false };
        let s = &e.sprite;
        let (x, y, w, h) = (x as u64, y as u64, w as u64, h as u64);
        if w == 0
            || h == 0
            || x + w > s.width as u64
            || y + h > s.height as u64
            || out.is_null()
            || len < (w * h * 4) as usize
        {
            return false;
        }
        let px = crate::document::layers::render_frame_rect_with(
            s,
            frame as usize,
            (x as usize, y as usize, w as usize, h as usize),
            e.reference_display,
        );
        std::ptr::copy_nonoverlapping(px.as_ptr(), out, px.len());
        true
    })
}

/// Returns what changed in the document since the previous call and starts over:
/// 0 nothing, 1 a rectangle of one frame (`frame` and `rect` = x, y, w, h are written), 2 anything.
/// A new document reports 2 first. Edits made through operations that do not report their extent count
/// as 2, so a viewer may trust a rectangle.
#[no_mangle]
pub unsafe extern "C" fn pixmith_take_dirty_rect(doc: *mut Editor, frame: *mut u32, rect: *mut i32) -> i32 {
    crate::ffi::guarded("pixmith_take_dirty_rect", std::ptr::null_mut(), || {
        let Some(e) = doc.as_mut() else { return 2 };
        match e.dirty.take() {
            DirtyTake::Nothing => 0,
            DirtyTake::All => 2,
            DirtyTake::Rect { frame: f, x, y, w, h } => {
                if !frame.is_null() {
                    *frame = f as u32;
                }
                if !rect.is_null() {
                    std::ptr::copy_nonoverlapping([x, y, w, h].as_ptr(), rect, 4);
                }
                1
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Cel, Layer, Sprite};
    use crate::paint::shapes::{PIXEL_PERFECT, SYM_X, SYM_Y};
    use std::sync::Arc;

    fn crop(full: &[u8], w: usize, r: (usize, usize, usize, usize)) -> Vec<u8> {
        (r.1..r.1 + r.3).flat_map(|y| full[(y * w + r.0) * 4..(y * w + r.0 + r.2) * 4].to_vec()).collect()
    }

    /// A deterministic pseudo-random cel with a mix of opaque, translucent and empty pixels.
    fn noise(w: usize, h: usize, seed: u32) -> Cel {
        let mut v = seed.wrapping_mul(2654435761).max(1);
        let mut next = || {
            v ^= v << 13;
            v ^= v >> 17;
            v ^= v << 5;
            v
        };
        let mut pixels = vec![0u8; w * h * 4];
        for p in pixels.chunks_exact_mut(4) {
            let r = next();
            let alpha = match r % 4 {
                0 => 0,
                1 => 255,
                _ => (r >> 8) as u8,
            };
            p.copy_from_slice(&[(r >> 16) as u8, (r >> 24) as u8, (r >> 4) as u8, alpha]);
        }
        Cel { opacity: 255, pixels, ..Default::default() }
    }

    fn rects(w: usize, h: usize) -> Vec<(usize, usize, usize, usize)> {
        vec![
            (0, 0, w, h),
            (0, 0, 1, 1),
            (w - 1, h - 1, 1, 1),
            (3, 2, 7, 5),
            (0, 4, w, 3),
            (5, 0, 2, h),
            (w / 2, h / 2, w - w / 2, h - h / 2),
        ]
    }

    fn check_all_rects(s: &Sprite) {
        let (w, h) = (s.width as usize, s.height as usize);
        let full = s.render(0);
        for r in rects(w, h) {
            assert_eq!(crop(&full, w, r), crate::document::layers::render_frame_rect(s, 0, r), "rect {r:?}");
        }
    }

    #[test]
    fn rect_matches_full_render_for_every_blend_mode() {
        for mode in 0..=18u16 {
            let (w, h) = (16, 12);
            let mut s = Sprite::new(w as u32, h as u32);
            s.cels[0][0] = Some(Arc::new(noise(w, h, 1)));
            for k in 1..3 {
                let i = s.add_layer(k - 1);
                s.cels[0][i] = Some(Arc::new(noise(w, h, 10 + k as u32)));
                s.layers[i].opacity = 200;
                s.layers[i].blend_mode = if k == 2 { mode } else { 0 };
            }
            let top = s.layers.len() - 1;
            if let Some(c) = s.cels[0][top].as_mut() {
                Arc::make_mut(c).opacity = 180;
            }
            check_all_rects(&s);
        }
    }

    #[test]
    fn rect_matches_full_render_with_groups() {
        let (w, h) = (16, 12);
        let mut s = Sprite::new(w as u32, h as u32);
        s.cels[0][0] = Some(Arc::new(noise(w, h, 3)));
        // base | group A (opacity 128, multiply) { a1, group B (screen) { b1, b2 }, a2 } | top
        let mut layers = vec![s.layers[0].clone()];
        let group = |name: &str, level: u16, opacity: u8, mode: u16| {
            let mut g = Layer::new(name);
            g.kind = 1;
            (g.child_level, g.opacity, g.blend_mode) = (level, opacity, mode);
            g
        };
        let image = |name: &str, level: u16, mode: u16| {
            let mut l = Layer::new(name);
            (l.child_level, l.blend_mode) = (level, mode);
            l
        };
        layers.push(group("A", 0, 128, 1));
        layers.push(image("a1", 1, 0));
        layers.push(group("B", 1, 255, 2));
        layers.push(image("b1", 2, 3));
        layers.push(image("b2", 2, 0));
        layers.push(image("a2", 1, 5));
        layers.push(image("top", 0, 0));
        s.layers = layers;
        s.cels = vec![(0..s.layers.len())
            .map(|i| (!s.layers[i].is_group()).then(|| Arc::new(noise(w, h, 20 + i as u32))))
            .collect()];
        check_all_rects(&s);
        // A hidden child and a transparent group change what is isolated.
        s.layers[4].visible = false;
        check_all_rects(&s);
        s.layers[2].visible = false;
        check_all_rects(&s);
        s.layers[1].opacity = 255;
        s.layers[1].blend_mode = 0;
        check_all_rects(&s);
    }

    #[test]
    fn rect_matches_full_render_with_tilemaps_and_missing_cels() {
        let mut s = Sprite::new(16, 16);
        s.draw_line(0, 0, 1, 1, 14, 9, [200, 30, 30, 255], 3);
        let t = crate::document::tilemap::add_tilemap_layer(&mut s, 0, 4, 4);
        assert!(s.cels[0][t].is_some() || s.layers[t].kind == 2);
        s.draw_line(0, 0, 0, 15, 15, 0, [10, 10, 220, 180], 2);
        let extra = s.add_layer(t);
        s.cels[0][extra] = None;
        check_all_rects(&s);
        // Frames without a cel render the same everywhere.
        s.add_frame(0, false);
        let full = s.render(1);
        assert_eq!(crop(&full, 16, (2, 3, 9, 6)), crate::document::layers::render_frame_rect(&s, 1, (2, 3, 9, 6)));
        // Requests outside the canvas are clipped, never panic.
        assert_eq!(crate::document::layers::render_frame_rect(&s, 0, (10, 10, 100, 100)).len(), 6 * 6 * 4);
        assert!(crate::document::layers::render_frame_rect(&s, 0, (99, 99, 4, 4)).is_empty());
    }

    #[test]
    fn ffi_render_rect_validates_its_arguments() {
        let mut e = Editor::new(Sprite::new(8, 8));
        e.sprite.draw_line(0, 0, 2, 2, 5, 5, [255, 0, 0, 255], 1);
        let mut out = vec![0u8; 4 * 4 * 4];
        unsafe {
            assert!(pixmith_render_rect(&e, 0, 2, 2, 4, 4, out.as_mut_ptr(), out.len()));
            assert_eq!(out, crop(&e.sprite.render(0), 8, (2, 2, 4, 4)));
            assert!(!pixmith_render_rect(&e, 0, 6, 6, 4, 4, out.as_mut_ptr(), out.len())); // outside
            assert!(!pixmith_render_rect(&e, 0, 0, 0, 0, 4, out.as_mut_ptr(), out.len())); // empty
            assert!(!pixmith_render_rect(&e, 0, 0, 0, 4, 4, out.as_mut_ptr(), 10)); // buffer too small
            assert!(!pixmith_render_rect(&e, 0, 0, 0, 4, 4, std::ptr::null_mut(), 64));
            assert!(!pixmith_render_rect(std::ptr::null(), 0, 0, 0, 4, 4, out.as_mut_ptr(), 64));
        }
    }

    // ---- dirty tracking ----------------------------------------------------------------------

    fn take(e: &mut Editor) -> (i32, u32, [i32; 4]) {
        let (mut f, mut r) = (u32::MAX, [0; 4]);
        let kind = unsafe { pixmith_take_dirty_rect(e, &mut f, r.as_mut_ptr()) };
        (kind, f, r)
    }

    #[test]
    fn a_new_document_is_dirty_then_clean() {
        let mut e = Editor::new(Sprite::new(8, 8));
        assert_eq!(take(&mut e).0, 2);
        assert_eq!(take(&mut e).0, 0);
    }

    #[test]
    fn draw_line_reports_its_bounds() {
        let mut e = Editor::new(Sprite::new(32, 32));
        take(&mut e);
        unsafe {
            crate::ffi::document::pixmith_begin_stroke(&mut e);
            assert_eq!(take(&mut e).0, 0, "a checkpoint changes no pixels");
            crate::ffi::document::pixmith_draw_line(&mut e, 0, 0, 4, 5, 10, 8, 0xFF0000FF, 3);
        }
        let (kind, frame, r) = take(&mut e);
        assert_eq!((kind, frame), (1, 0));
        // Inside the reported rectangle, with room for the brush.
        assert!(r[0] <= 3 && r[1] <= 4 && r[0] + r[2] >= 12 && r[1] + r[3] >= 10, "{r:?}");
        assert!(r[2] < 32 && r[3] < 32, "{r:?}");
        // Two segments accumulate.
        unsafe {
            crate::ffi::document::pixmith_draw_line(&mut e, 0, 0, 20, 20, 21, 21, 0xFF0000FF, 1);
            crate::ffi::document::pixmith_draw_line(&mut e, 0, 0, 2, 2, 3, 3, 0xFF0000FF, 1);
        }
        let (_, _, r) = take(&mut e);
        assert!(r[0] <= 2 && r[1] <= 2 && r[0] + r[2] >= 22 && r[1] + r[3] >= 22, "{r:?}");
    }

    #[test]
    fn anything_else_dirties_everything() {
        let mut e = Editor::new(Sprite::new(8, 8));
        take(&mut e);
        unsafe { crate::ffi::document::pixmith_set_layer_visible(&mut e, 0, false) };
        assert_eq!(take(&mut e).0, 2);
        unsafe { crate::ffi::document::pixmith_draw_line(&mut e, 0, 0, 1, 1, 2, 2, 0xFF0000FF, 1) };
        assert_eq!(take(&mut e).0, 1);
        unsafe { crate::ffi::document::pixmith_fill(&mut e, 0, 0, 0, 0, 0x00FF00FF) };
        assert_eq!(take(&mut e).0, 2);
        unsafe {
            crate::ffi::document::pixmith_begin_stroke(&mut e);
            crate::ffi::document::pixmith_draw_line(&mut e, 0, 0, 1, 1, 2, 2, 0xFF0000FF, 1);
        }
        take(&mut e);
        assert!(unsafe { crate::ffi::document::pixmith_undo(&mut e) });
        assert_eq!(take(&mut e).0, 2);
        assert!(unsafe { crate::ffi::document::pixmith_redo(&mut e) });
        assert_eq!(take(&mut e).0, 2);
        unsafe { crate::ffi::document::pixmith_undo(&mut e) };
        take(&mut e);
        assert!(unsafe { crate::history::pixmith_history_goto(&mut e, 1) });
        assert_eq!(take(&mut e).0, 2);
        // A rectangle after a full change is part of the full change.
        unsafe {
            crate::ffi::document::pixmith_set_layer_visible(&mut e, 0, true);
            crate::ffi::document::pixmith_draw_line(&mut e, 0, 0, 1, 1, 2, 2, 0xFF0000FF, 1);
        }
        assert_eq!(take(&mut e).0, 2);
    }

    #[test]
    fn edits_that_show_elsewhere_are_not_narrowed() {
        // Another frame: two rectangles of different frames make a full change.
        let mut e = Editor::new(Sprite::new(8, 8));
        e.sprite.add_frame(0, false);
        take(&mut e);
        unsafe {
            crate::ffi::document::pixmith_draw_line(&mut e, 0, 0, 1, 1, 2, 2, 0xFF0000FF, 1);
            crate::ffi::document::pixmith_draw_line(&mut e, 0, 1, 1, 1, 2, 2, 0xFF0000FF, 1);
        }
        assert_eq!(take(&mut e).0, 2);
        // A linked cel shows its edit in the sibling frames.
        let mut e = Editor::new(Sprite::new(8, 8));
        e.sprite.draw_line(0, 0, 0, 0, 3, 3, [255, 0, 0, 255], 1);
        e.sprite.add_frame(0, true);
        assert!(e.sprite.link_cels(0, 0, 1));
        take(&mut e);
        unsafe { crate::ffi::document::pixmith_draw_line(&mut e, 0, 0, 1, 1, 2, 2, 0xFF0000FF, 1) };
        assert_eq!(take(&mut e).0, 2);
        // Tilemap layers share tiles between cells.
        let mut e = Editor::new(Sprite::new(8, 8));
        let t = crate::document::tilemap::add_tilemap_layer(&mut e.sprite, 0, 4, 4);
        take(&mut e);
        unsafe { crate::ffi::document::pixmith_draw_line(&mut e, t as u32, 0, 1, 1, 2, 2, 0xFF0000FF, 1) };
        assert_eq!(take(&mut e).0, 2);
    }

    /// The contract viewers rely on: patching a stale copy with the reported rectangle gives the full render.
    fn assert_patching_matches(e: &mut Editor, mut ops: impl FnMut(&mut Editor, u32)) {
        let (w, h) = (e.sprite.width as usize, e.sprite.height as usize);
        take(e);
        let mut copy = e.sprite.render(0);
        for step in 0..40 {
            ops(e, step);
            let (kind, frame, r) = take(e);
            match kind {
                0 => {}
                1 => {
                    assert_eq!(frame, 0);
                    let rect = (r[0] as usize, r[1] as usize, r[2] as usize, r[3] as usize);
                    let patch = crate::document::layers::render_frame_rect(&e.sprite, 0, rect);
                    for row in 0..rect.3 {
                        let d = ((rect.1 + row) * w + rect.0) * 4;
                        copy[d..d + rect.2 * 4].copy_from_slice(&patch[row * rect.2 * 4..(row + 1) * rect.2 * 4]);
                    }
                }
                _ => copy = e.sprite.render(0),
            }
            assert_eq!(copy.len(), w * h * 4);
            assert!(copy == e.sprite.render(0), "step {step}: the patched copy differs from a full render");
        }
    }

    #[test]
    fn patching_with_dirty_rects_matches_full_renders_for_strokes() {
        for flags in [0, SYM_X, SYM_Y, SYM_X | SYM_Y, PIXEL_PERFECT, PIXEL_PERFECT | SYM_X] {
            for size in [1u32, 2, 5, 9] {
                let mut e = Editor::new(Sprite::new(48, 40));
                e.sprite.add_layer(0);
                e.sprite.layers[1].opacity = 200;
                e.sprite.layers[1].blend_mode = 1;
                e.sprite.cels[0][0] = Some(Arc::new(noise(48, 40, 5)));
                let mut x = 7i32;
                assert_patching_matches(&mut e, |e, step| {
                    // A wandering path: the first call starts the stroke, later ones extend it.
                    x = (x * 31 + step as i32 * 7) % 46;
                    let y = (x * 17 + step as i32 * 5) % 38;
                    unsafe {
                        if step == 0 {
                            crate::ffi::document::pixmith_begin_stroke(e);
                            crate::paint::shapes::pixmith_stroke_begin(e, 1, 0, x, y, 0xFF3366CC, size, flags);
                        } else {
                            crate::paint::shapes::pixmith_stroke_to(e, x, y);
                        }
                        // Interleaved single segments, like the plain pencil path.
                        if step % 5 == 4 {
                            crate::ffi::document::pixmith_draw_line(e, 0, 0, y, x, y / 2, x / 2, 0x40FF8080, size);
                        }
                    }
                });
                crate::paint::shapes::pixmith_stroke_end();
            }
        }
    }

    #[test]
    fn patching_with_dirty_rects_matches_with_a_selection() {
        let mut e = Editor::new(Sprite::new(32, 32));
        e.sprite.cels[0][0] = Some(Arc::new(noise(32, 32, 9)));
        crate::paint::selection::select_rect(&mut e, (8, 8), (20, 20));
        assert_patching_matches(&mut e, |e, step| unsafe {
            if step == 0 {
                crate::ffi::document::pixmith_begin_stroke(e);
                crate::paint::shapes::pixmith_stroke_begin(e, 0, 0, 2, 2, 0xFFFF0000, 4, 0);
            } else {
                crate::paint::shapes::pixmith_stroke_to(e, (step as i32 * 3) % 30, (step as i32 * 5) % 30);
            }
        });
        crate::paint::shapes::pixmith_stroke_end();
    }
}
