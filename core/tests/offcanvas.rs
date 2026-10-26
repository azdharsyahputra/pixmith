//! Cels keep their content beyond the canvas edges (Aseprite semantics): moving a cel off the canvas
//! and back loses nothing, canvas resizes reveal and store content, and `.aseprite` / `.pxmt` files
//! round-trip cels that stick out.

#![allow(clippy::chunks_exact_to_as_chunks)]

use pixmith_core::document::color_mode::ColorMode;
use pixmith_core::document::offcanvas::OffCanvas;
use pixmith_core::document::{Cel, Editor, Sprite};
use pixmith_core::formats::{ase, pxmt};
use pixmith_core::paint::filters::scale_sprite;
use pixmith_core::paint::selection::{self, nudge, resize_canvas};
use pixmith_core::paint::transform;
use pixmith_core::scripting::{run, Limits};
use std::sync::Arc;

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// A `w` x `h` image whose pixels are all distinct (and opaque), so any misplacement shows.
fn pattern(w: usize, h: usize, seed: u8) -> Vec<u8> {
    (0..w * h).flat_map(|i| [(i % 251) as u8, (i / 7 % 251) as u8, seed, 255]).collect()
}

/// A sprite whose layer 0 / frame 0 cel holds `w` x `h` pixels with the top-left at (x, y).
fn sprite_with(cw: u32, ch: u32, (x, y, w, h): (i32, i32, usize, usize)) -> Sprite {
    let mut s = Sprite::new(cw, ch);
    let mut cel = Cel::default();
    cel.set_full(cw as usize, ch as usize, (x, y, w, h), &pattern(w, h, 9)).unwrap();
    s.cels[0][0] = Some(Arc::new(cel));
    s
}

fn cel(s: &Sprite, l: usize, f: usize) -> &Cel {
    s.cels[f][l].as_ref().expect("cel")
}

fn full(s: &Sprite, l: usize, f: usize) -> (i32, i32, usize, usize, Vec<u8>) {
    let fi = cel(s, l, f).full_image(s.width as usize, s.height as usize).expect("content");
    (fi.x, fi.y, fi.w, fi.h, fi.rgba)
}

#[test]
fn set_full_splits_content_and_full_image_joins_it_again() {
    let s = sprite_with(8, 6, (-3, -2, 14, 9));
    s.validate().unwrap();
    let c = cel(&s, 0, 0);
    assert!(c.outside.is_some());
    assert_eq!(c.full_bounds(8, 6), Some((-3, -2, 14, 9)));
    assert_eq!(full(&s, 0, 0), (-3, -2, 14, 9, pattern(14, 9, 9)));
    // What is inside the canvas is in `pixels`, as for any other cel.
    let all = pattern(14, 9, 9);
    let at = |x: usize, y: usize| &all[((y + 2) * 14 + x + 3) * 4..][..4];
    assert_eq!(&c.pixels[(2 * 8 + 5) * 4..][..4], at(5, 2));
    assert_eq!(c.pixel_at(8, 6, -3, -2), &all[..4]);
    assert_eq!(c.pixel_at(8, 6, 10, 6), &all[(8 * 14 + 13) * 4..][..4]);
    assert_eq!(c.pixel_at(8, 6, 11, 6), [0; 4]);
    // The stored part is the tightest box around what lies outside, with the canvas area zeroed.
    let o = c.outside.as_ref().unwrap();
    assert_eq!((o.x, o.y, o.w, o.h), (-3, -2, 14, 9));
    let canvas_corner = ((0 - o.y) as usize * o.w as usize + (0 - o.x) as usize) * 4;
    assert_eq!(&o.rgba[canvas_corner..canvas_corner + 4], &[0; 4], "the canvas area of the stored part is zero");
}

#[test]
fn moving_a_cel_out_and_back_loses_nothing() {
    let original = sprite_with(8, 6, (1, 1, 5, 4));
    let mut c = (*original.cels[0][0].clone().unwrap()).clone();
    assert!(c.outside.is_none());
    c.translate(8, 6, -20, 3).unwrap();
    assert!(c.outside.is_some());
    assert!(c.pixels.iter().all(|&b| b == 0), "everything left the canvas");
    c.translate(8, 6, 7, -3).unwrap();
    assert!(c.outside.is_some(), "still partly outside");
    c.translate(8, 6, 13, 0).unwrap();
    assert_eq!(c, **original.cels[0][0].as_ref().unwrap());
    assert!(c.outside.is_none(), "back inside: nothing stored beyond the canvas");
}

#[test]
fn nudging_a_whole_cel_out_and_back_is_exact_and_undoable() {
    let original = sprite_with(8, 6, (1, 1, 5, 4));
    let mut e = Editor::new(original.clone());
    assert!(nudge(&mut e, 0, 0, -9, 0));
    assert!(e.sprite.cels[0][0].as_ref().unwrap().outside.is_some());
    assert!(e.sprite.render(0).chunks_exact(4).all(|p| p[3] == 0), "nothing visible any more");
    assert!(nudge(&mut e, 0, 0, 9, 0));
    assert_eq!(e.sprite.cels[0][0], original.cels[0][0]);
    e.sprite.validate().unwrap();

    // Undo the move back: the content is beyond the edge again, undo once more: the original.
    assert!(e.undo());
    assert!(e.sprite.cels[0][0].as_ref().unwrap().outside.is_some());
    e.wait_for_history();
    assert!(e.undo());
    assert_eq!(e.sprite.cels[0][0], original.cels[0][0]);
    // Redo restores the beyond-the-edge state, then the final one.
    assert!(e.redo());
    assert!(e.sprite.cels[0][0].as_ref().unwrap().outside.is_some());
    assert!(e.redo());
    assert_eq!(e.sprite.cels[0][0], original.cels[0][0]);
    assert!(e.sprite.validate().is_ok());
}

#[test]
fn a_move_in_several_steps_keeps_everything() {
    let original = sprite_with(10, 10, (2, 2, 6, 6));
    let mut e = Editor::new(original.clone());
    for (dx, dy) in [(-5, 0), (-6, -7), (3, 9), (8, 5)] {
        assert!(nudge(&mut e, 0, 0, dx, dy));
        e.sprite.validate().unwrap();
    }
    // Net movement (0, 7): content moved down, partly beyond the bottom edge.
    let (x, y, w, h, rgba) = full(&e.sprite, 0, 0);
    assert_eq!((x, y, w, h), (2, 9, 6, 6));
    assert_eq!(rgba, pattern(6, 6, 9));
    assert!(nudge(&mut e, 0, 0, 0, -7));
    assert_eq!(e.sprite.cels[0][0], original.cels[0][0]);
}

#[test]
fn growing_the_canvas_reveals_stored_content_and_shrinking_stores_it() {
    // The content starts 4 pixels left of the canvas.
    let s = sprite_with(8, 6, (-4, 1, 9, 3));
    let expected = pattern(9, 3, 9);
    let mut e = Editor::new(s);
    // Grow by 4 on the left: everything is on the canvas.
    assert!(resize_canvas(&mut e, 12, 6, 4, 0));
    assert!(e.sprite.cels[0][0].as_ref().unwrap().outside.is_none());
    assert_eq!(full(&e.sprite, 0, 0), (0, 1, 9, 3, expected.clone()));
    e.sprite.validate().unwrap();
    // Shrink so that the right part falls off, then grow again: it is back.
    assert!(resize_canvas(&mut e, 5, 6, 0, 0));
    assert!(e.sprite.cels[0][0].as_ref().unwrap().outside.is_some());
    assert_eq!(full(&e.sprite, 0, 0), (0, 1, 9, 3, expected.clone()));
    assert!(resize_canvas(&mut e, 12, 6, 0, 0));
    assert!(e.sprite.cels[0][0].as_ref().unwrap().outside.is_none());
    assert_eq!(full(&e.sprite, 0, 0), (0, 1, 9, 3, expected));
    // And the same through undo.
    assert!(e.undo());
    assert_eq!((e.sprite.width, e.sprite.height), (5, 6));
    assert!(e.sprite.cels[0][0].as_ref().unwrap().outside.is_some());
    e.sprite.validate().unwrap();
}

#[test]
fn growing_with_an_offset_shifts_content_beyond_the_old_canvas() {
    let s = sprite_with(6, 6, (-2, -2, 4, 4));
    let mut e = Editor::new(s);
    // Content at (-2, -2); the canvas grows by 5 on the right and the old content moves right by 1.
    assert!(resize_canvas(&mut e, 11, 6, 1, 0));
    assert_eq!(full(&e.sprite, 0, 0), (-1, -2, 4, 4, pattern(4, 4, 9)));
    // Cropping with a negative offset (like "crop to selection") stores what falls outside.
    let mut e = Editor::new(sprite_with(8, 8, (0, 0, 8, 8)));
    assert!(resize_canvas(&mut e, 4, 4, -2, -2));
    let (x, y, w, h, rgba) = full(&e.sprite, 0, 0);
    assert_eq!((x, y, w, h), (-2, -2, 8, 8));
    assert_eq!(rgba, pattern(8, 8, 9));
    assert!(resize_canvas(&mut e, 8, 8, 2, 2));
    assert_eq!(full(&e.sprite, 0, 0), (0, 0, 8, 8, pattern(8, 8, 9)));
}

#[test]
fn aseprite_files_keep_cels_that_stick_out_of_the_canvas() {
    let mut s = Sprite::new(12, 8);
    let top = s.add_layer(0);
    let (cw, ch) = (12usize, 8usize);
    let mut a = Cel { opacity: 200, ..Default::default() };
    a.set_full(cw, ch, (-10, -5, 15, 9), &pattern(15, 9, 1)).unwrap(); // reaches left and above the canvas
    let mut b = Cel::default();
    b.set_full(cw, ch, (6, 2, 20, 4), &pattern(20, 4, 2)).unwrap(); // reaches past the right edge
    s.cels[0][0] = Some(Arc::new(a));
    s.cels[0][top] = Some(Arc::new(b));
    s.add_frame(0, true);
    s.validate().unwrap();

    let bytes = ase::write(&s);
    let back = ase::read(&bytes).unwrap();
    back.validate().unwrap();
    for (l, f) in [(0, 0), (top, 0), (0, 1), (top, 1)] {
        assert_eq!(full(&back, l, f), full(&s, l, f), "layer {l} frame {f}");
    }
    assert_eq!(full(&back, 0, 0).0, -10);
    assert_eq!(full(&back, top, 0).2, 20);
    assert_eq!(back, s, "the whole document survives");
    assert_eq!(ase::write(&back), bytes, "writing again gives the same bytes");

    // `.pxmt` wraps the same bytes.
    let wrapped = pxmt::write(&s).unwrap();
    let from_pxmt = pxmt::read(&wrapped).unwrap();
    assert_eq!(full(&from_pxmt, 0, 0), full(&s, 0, 0));
    assert_eq!(from_pxmt, s);

    // Only what is inside the canvas is visible to renderers and exporters.
    assert_eq!(back.render(0), s.render(0));
}

#[test]
fn indexed_files_keep_off_canvas_pixels() {
    let mut s = Sprite::new(4, 4);
    let red = s.palette.iter().position(|c| *c == [255, 0, 77, 255]).unwrap_or(8);
    let color = s.palette[red];
    let mut img = Vec::new();
    for _ in 0..6 * 3 {
        img.extend(color);
    }
    let mut c = Cel::default();
    c.set_full(4, 4, (-2, 1, 6, 3), &img).unwrap();
    s.cels[0][0] = Some(Arc::new(c));
    pixmith_core::document::color_mode::convert(&mut s, ColorMode::Indexed, 0);
    let back = ase::read(&ase::write(&s)).unwrap();
    assert_eq!(full(&back, 0, 0), full(&s, 0, 0));
    assert_eq!(back.color_mode, ColorMode::Indexed);
}

#[test]
fn merge_down_composites_in_full_coordinates() {
    let mut s = Sprite::new(6, 4);
    let top = s.add_layer(0);
    let mut lower = Cel::default();
    lower.set_full(6, 4, (-3, 0, 5, 2), &[RED; 10].concat()).unwrap();
    let mut upper = Cel::default();
    upper.set_full(6, 4, (3, 1, 6, 2), &[BLUE; 12].concat()).unwrap();
    s.cels[0][0] = Some(Arc::new(lower));
    s.cels[0][top] = Some(Arc::new(upper));
    let visible = s.render(0);
    let merged = s.merge_down(top).unwrap();
    s.validate().unwrap();
    assert_eq!(s.render(0), visible);
    let (x, y, w, h, rgba) = full(&s, merged, 0);
    assert_eq!((x, y, w, h), (-3, 0, 12, 3));
    // Lower content (red) at x -3..2 y 0..2; upper (blue) at x 3..9 y 1..3.
    let px = |px: i32, py: i32| -> [u8; 4] {
        rgba[((py - y) as usize * w + (px - x) as usize) * 4..][..4].try_into().unwrap()
    };
    assert_eq!(px(-3, 0), RED);
    assert_eq!(px(8, 2), BLUE);
    assert_eq!(px(3, 1), BLUE);
    assert_eq!(px(2, 0), [0; 4]);
}

#[test]
fn scaling_the_sprite_scales_the_content_beyond_the_canvas() {
    let mut s = sprite_with(4, 4, (-2, 0, 8, 2));
    assert!(scale_sprite(&mut s, 8, 8));
    s.validate().unwrap();
    let (x, y, w, h, rgba) = full(&s, 0, 0);
    assert_eq!((x, y, w, h), (-4, 0, 16, 4));
    // Every source pixel became a 2x2 block.
    let src = pattern(8, 2, 9);
    for sy in 0..2 {
        for sx in 0..8 {
            let want = &src[(sy * 8 + sx) * 4..][..4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let i = ((sy * 2 + dy) * 16 + sx * 2 + dx) * 4;
                assert_eq!(&rgba[i..i + 4], want);
            }
        }
    }
    // Scaling down samples the same pixels as before.
    let mut s2 = sprite_with(8, 8, (-4, 0, 16, 4));
    assert!(scale_sprite(&mut s2, 4, 4));
    s2.validate().unwrap();
    assert_eq!(full(&s2, 0, 0).0, -2);
    assert_eq!(full(&s2, 0, 0).2, 8);
}

#[test]
fn flipping_a_whole_cel_flips_what_sticks_out_too() {
    let mut e = Editor::new(sprite_with(6, 4, (-2, 1, 5, 2)));
    let before = full(&e.sprite, 0, 0);
    assert!(selection::transform(&mut e, 0, 0, selection::Op::FlipH));
    let (x, y, w, h, rgba) = full(&e.sprite, 0, 0);
    assert_eq!((x, y, w, h), (6 - 1 - (before.0 + before.2 as i32 - 1), 1, 5, 2));
    for r in 0..2 {
        for c in 0..5 {
            assert_eq!(rgba[(r * 5 + c) * 4..][..4], before.4[(r * 5 + (4 - c)) * 4..][..4]);
        }
    }
    assert!(selection::transform(&mut e, 0, 0, selection::Op::FlipH));
    assert_eq!(full(&e.sprite, 0, 0), before);
}

#[test]
fn clearing_a_whole_cel_clears_what_is_beyond_the_canvas() {
    let mut e = Editor::new(sprite_with(4, 4, (-2, 0, 3, 3)));
    assert!(selection::delete(&mut e, 0, 0));
    let c = e.sprite.cels[0][0].as_ref().unwrap();
    assert!(c.outside.is_none() && c.pixels.iter().all(|&b| b == 0));
}

#[test]
fn free_transform_move_keeps_content_beyond_the_canvas() {
    let original = sprite_with(8, 6, (1, 1, 4, 3));
    let mut e = Editor::new(original.clone());
    assert!(transform::begin(&mut e, 0, 0, false));
    transform::drag_begin(&mut e);
    transform::drag(&mut e, transform::Drag::Move, (0.0, 0.0), (-4.0, 0.0));
    let (x, y, w, h, _) = full(&e.sprite, 0, 0);
    assert_eq!((x, y, w, h), (-3, 1, 4, 3));
    // Dragging back before committing restores the cel exactly.
    transform::drag(&mut e, transform::Drag::Move, (0.0, 0.0), (0.0, 0.0));
    transform::commit(&mut e);
    assert_eq!(e.sprite.cels[0][0], original.cels[0][0]);

    // Commit while beyond the edge, then move it back with another session.
    assert!(transform::begin(&mut e, 0, 0, false));
    transform::drag_begin(&mut e);
    transform::drag(&mut e, transform::Drag::Move, (0.0, 0.0), (-6.0, 0.0));
    transform::commit(&mut e);
    assert!(e.sprite.cels[0][0].as_ref().unwrap().outside.is_some());
    assert!(transform::begin(&mut e, 0, 0, false));
    transform::drag_begin(&mut e);
    transform::drag(&mut e, transform::Drag::Move, (0.0, 0.0), (6.0, 0.0));
    transform::commit(&mut e);
    assert_eq!(e.sprite.cels[0][0], original.cels[0][0]);
    e.sprite.validate().unwrap();
}

#[test]
fn linked_cels_and_copies_keep_what_is_beyond_the_canvas() {
    let mut s = sprite_with(6, 6, (-1, -1, 4, 4));
    s.add_frame(0, true);
    s.link_cels(0, 0, 1);
    assert!(Arc::ptr_eq(s.cels[0][0].as_ref().unwrap(), s.cels[1][0].as_ref().unwrap()));
    let dup = s.duplicate_layer(0).unwrap();
    assert_eq!(full(&s, dup, 0), full(&s, 0, 0));
    let f = s.add_frame(1, true);
    assert_eq!(full(&s, 0, f), full(&s, 0, 0));
    assert!(s.move_cels((0, 0), (0, 0), 0, 2, true));
    assert_eq!(full(&s, 0, 2), full(&s, 0, 0));
}

#[test]
fn validate_checks_the_off_canvas_part() {
    let good = sprite_with(4, 4, (-2, 0, 4, 2));
    good.validate().unwrap();
    let broken = |f: &dyn Fn(&mut OffCanvas)| {
        let mut s = good.clone();
        let mut c = (**s.cels[0][0].as_ref().unwrap()).clone();
        let mut o = (**c.outside.as_ref().unwrap()).clone();
        f(&mut o);
        c.outside = Some(Arc::new(o));
        s.cels[0][0] = Some(Arc::new(c));
        s.validate()
    };
    assert!(broken(&|o| o.rgba.pop().map(|_| ()).unwrap_or(())).is_err());
    assert!(broken(&|o| o.x = 40_000).is_err());
    assert!(broken(&|o| o.y = -40_000).is_err());
    assert!(broken(&|o| (o.w, o.h) = (0, 0)).is_err());
    assert!(broken(&|o| (o.w, o.h) = (65_536, 1)).is_err());
    // Content that would reach too far is refused, and the cel is left as it was.
    let mut c = Cel::default();
    assert!(c.set_full(4, 4, (40_000, 0, 2, 2), &[255; 16]).is_err());
    assert!(c.set_full(4, 4, (0, 0, 2, 2), &[255; 15]).is_err());
    assert_eq!(c, Cel::default());
    let mut m = (**good.cels[0][0].as_ref().unwrap()).clone();
    assert!(m.translate(4, 4, 40_000, 0).is_err());
    assert_eq!(&m, &**good.cels[0][0].as_ref().unwrap());
}

#[test]
fn converting_to_grayscale_converts_the_part_beyond_the_canvas() {
    let mut s = Sprite::new(2, 2);
    let mut c = Cel::default();
    c.set_full(2, 2, (-1, 0, 2, 1), &[RED, RED].concat()).unwrap();
    s.cels[0][0] = Some(Arc::new(c));
    pixmith_core::document::color_mode::convert(&mut s, ColorMode::Grayscale, 0);
    let o = s.cels[0][0].as_ref().unwrap().outside.clone().unwrap();
    assert_eq!(o.rgba[0], o.rgba[1]);
    assert_eq!(o.rgba[1], o.rgba[2]);
    assert_ne!(o.rgba[0], 255);
}

// MARK: Lua

fn ok(e: &mut Editor, src: &str) -> Vec<String> {
    let o = run(e, src, (0, 0), Limits::default());
    assert!(o.error.is_none(), "{:?}", o.error);
    o.messages
}

#[test]
fn lua_position_moves_the_whole_cel_and_back_restores_it() {
    let original = sprite_with(8, 6, (2, 1, 4, 3));
    let mut e = Editor::new(original.clone());
    let msgs = ok(
        &mut e,
        r#"
        local cel = sprite.cels[1]
        assert(cel.position == Point(2, 1) and cel.bounds == Rectangle(2, 1, 4, 3))
        local before = cel.image:clone()
        cel.position = Point(-20, 0)
        assert(cel.position == Point(-20, 0) and cel.bounds == Rectangle(-20, 0, 4, 3))
        assert(cel.image.width == 4 and cel.image.height == 3)
        assert(sprite:getPixel(2, 1) == 0 and not cel.isEmpty)
        cel.position = Point(2, 1)
        assert(cel.bounds == Rectangle(2, 1, 4, 3))
        assert(cel.image.bytes == before.bytes)
        -- partly beyond the right edge
        cel.position = Point(6, 1)
        assert(cel.bounds == Rectangle(6, 1, 4, 3))
        local px = cel.image:getPixel(3, 2)
        cel.position = Point(2, 1)
        assert(cel.image.bytes == before.bytes)
        print("done")
        "#,
    );
    assert_eq!(msgs, vec!["done"]);
    assert_eq!(e.sprite.cels[0][0], original.cels[0][0]);
}

#[test]
fn lua_new_cel_and_cel_image_accept_positions_beyond_the_canvas() {
    let mut e = Editor::new(Sprite::new(8, 6));
    ok(
        &mut e,
        r#"
        local img = Image(3, 2)
        img:clear(0xFF0000FF)
        local layer = sprite:newLayer("far")
        local cel = sprite:newCel(layer, 1, img, Point(-1, -1))
        assert(cel.position == Point(-1, -1) and cel.bounds == Rectangle(-1, -1, 3, 2))
        assert(sprite:getPixel(0, 0, layer, 1) == 0xFF0000FF and sprite:getPixel(2, 0, layer, 1) == 0)
        local back = cel.image
        assert(back.width == 3 and back.height == 2)
        assert(back:getPixel(0, 0) == 0xFF0000FF)
        -- drawing on the view reaches the part beyond the canvas too
        back:putPixel(0, 0, 0x00FF00FF)
        assert(cel.image:getPixel(0, 0) == 0x00FF00FF)
        local far = sprite:newCel(layer, 1, img, Point(100, 100))
        assert(far.bounds == Rectangle(100, 100, 3, 2) and not far.isEmpty)
        far.position = Point(0, 0)
        assert(far.bounds == Rectangle(0, 0, 3, 2) and sprite:getPixel(2, 1, layer, 1) == 0xFF0000FF)
        -- setting the image keeps the cel's position
        cel = sprite:newCel(layer, 1, img, Point(-2, 3))
        local bigger = Image(5, 1) bigger:clear(Color(0, 0, 255))
        cel.image = bigger
        assert(cel.bounds == Rectangle(-2, 3, 5, 1))
        assert(not pcall(function() cel.position = Point(100000, 0) end))
        assert(cel.bounds == Rectangle(-2, 3, 5, 1))
        "#,
    );
    e.sprite.validate().unwrap();
}
