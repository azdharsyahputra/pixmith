//! Regression tests for inputs that used to abort the process (the release profile aborts on
//! panic and on failed allocations) or to allocate absurd amounts of memory.

use pixmith_core::document::tilemap;
use pixmith_core::document::{Editor, Sprite};
use pixmith_core::formats::{animation, export, import, sheet_import};
use pixmith_core::paint::{filters, font, selection, transform};
use pixmith_core::palette::edit as palette_edit;
use pixmith_core::scripting as script;
use std::sync::Arc;

const RED: [u8; 4] = [255, 0, 0, 255];

#[test]
fn removing_a_palette_entry_works_on_palettes_longer_than_256() {
    // Files may carry up to 65536 palette entries; only the first 256 can be indexed.
    let mut s = Sprite::new(2, 1);
    s.palette = (0..300u32).map(|i| [(i % 256) as u8, (i / 256) as u8, 7, 255]).collect();
    assert!(palette_edit::remove_entries(&mut s, &[3]));
    assert_eq!(s.palette.len(), 299);
    assert_eq!(s.palette[3], [4, 0, 7, 255]);
    assert!(palette_edit::remove_entries(&mut s, &[290, 298]));
    assert_eq!(s.palette.len(), 297);
}

#[test]
fn non_finite_transform_inputs_are_ignored() {
    let mut e = Editor::new(Sprite::new(8, 8));
    e.sprite.draw_line(0, 0, 1, 1, 6, 6, RED, 1);
    assert!(transform::begin(&mut e, 0, 0, true));
    let before = e.sprite.render(0);
    unsafe {
        for field in 0..7 {
            transform::pixmith_xf_set_field(&mut e, field, f64::NAN);
            transform::pixmith_xf_set_field(&mut e, field, f64::INFINITY);
        }
        transform::pixmith_xf_drag_begin(&mut e);
        for kind in 0..5 {
            transform::pixmith_xf_drag(&mut e, kind, 1, 1, false, f64::NAN, 0.0, 3.0, f64::INFINITY);
        }
    }
    assert_eq!(e.sprite.render(0), before, "the preview must not be wiped by NaN input");
    transform::commit(&mut e);
}

#[test]
fn huge_text_renders_nothing_instead_of_allocating_gigabytes() {
    let text = "W".repeat(3_000_000);
    let m = font::render(&text, 32);
    assert_eq!((m.w, m.h, m.data.len()), (0, 0, 0));
    let ok = font::render("Hi", 2);
    assert!(ok.w > 0 && ok.data.contains(&255));
}

/// A GIF with a `w` x `h` logical screen holding one 1x1 frame.
fn gif_with_screen(w: u16, h: u16) -> Vec<u8> {
    let mut b = b"GIF89a".to_vec();
    b.extend(w.to_le_bytes());
    b.extend(h.to_le_bytes());
    b.extend([0x80, 0, 0]); // global color table of two entries
    b.extend([0, 0, 0, 255, 255, 255]);
    b.extend([0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0]); // image descriptor: 1x1 at the origin
    b.extend([2, 2, 0x44, 0x01, 0]); // LZW: one pixel of index 0
    b.push(0x3B);
    b
}

#[test]
fn gif_import_rejects_huge_screens() {
    let ok = animation::sprite_from_gif(&gif_with_screen(4, 4)).unwrap();
    assert_eq!((ok.width, ok.frame_count()), (4, 1));
    let err = animation::sprite_from_gif(&gif_with_screen(60000, 60000)).unwrap_err();
    assert!(err.contains("too large"), "{err}");
}

fn png_header_only(w: u32, h: u32) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
        out.extend((body.len() as u32).to_be_bytes());
        out.extend(kind);
        out.extend(body);
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(body);
        out.extend(crc.sum().to_be_bytes());
    }
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]); // 8-bit RGBA
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &[0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[test]
fn png_import_checks_the_size_before_allocating() {
    // The decoder may refuse these itself; either way nothing of 14 GB is ever requested.
    assert!(import::decode_png(&png_header_only(40000, 40000)).is_err());
    assert!(import::decode_png(&png_header_only(20000, 20000)).is_err());
    assert!(import::decode_png(&png_header_only(9000, 9000)).is_err(), "81 million pixels");
}

#[test]
fn sheet_imports_limit_the_number_of_frames() {
    let mut s = Sprite::new(300, 300);
    s.draw_line(0, 0, 0, 0, 299, 299, RED, 1);
    let png = export::frame_png(&s, 0, 1).unwrap();
    let err = import::sprite_from_sheet(&png, 1, 1).err().unwrap();
    assert!(err.contains("frames"), "{err}");
    assert!(import::sprite_from_sheet(&png, 100, 100).is_ok());

    // 70000 frames in the JSON are refused before any frame buffer is allocated.
    let frames: Vec<String> = (0..70000)
        .map(|i| format!("{{\"frame\":{{\"x\":0,\"y\":0,\"w\":1,\"h\":1}},\"duration\":{}}}", 100 + i % 3))
        .collect();
    let json = format!("{{\"frames\":[{}],\"meta\":{{}}}}", frames.join(","));
    let err = sheet_import::sprite_from_sheet_json(&png, &json).err().unwrap();
    assert!(err.contains("too many frames"), "{err}");
}

#[test]
fn exports_refuse_images_that_would_not_fit_in_memory() {
    let big = Sprite::new(4096, 4096);
    assert!(export::frame_png(&big, 0, 64).is_err());
    assert!(export::sheet_png(&big, 64).is_err());
    assert!(pixmith_core::formats::gif::png_sequence(&big, 64, "x").is_err());
    assert!(animation::apng_bytes(&big, 64).is_err());
    assert!(export::frame_png(&Sprite::new(8, 8), 0, 8).is_ok());
}

/// A sprite whose single frame holds `layers` non-empty cels.
fn sprite_with_cels(w: u32, h: u32, frames: usize) -> Sprite {
    let mut s = Sprite::new(w, h);
    s.draw_line(0, 0, 0, 0, 1, 1, RED, 1);
    for _ in 1..frames {
        s.add_frame(0, true);
    }
    s
}

#[test]
fn scaling_and_canvas_resizing_stay_within_a_memory_budget() {
    // 20 distinct 8192 x 8192 cels would be more than 5 GB.
    let mut s = sprite_with_cels(100, 100, 20);
    assert!(!filters::scale_sprite(&mut s, 8192, 8192));
    assert_eq!((s.width, s.height), (100, 100));
    assert!(filters::scale_sprite(&mut s, 200, 200));

    let mut e = Editor::new(sprite_with_cels(16, 16, 20));
    assert!(!selection::resize_canvas(&mut e, 16384, 16384, 0, 0));
    assert_eq!((e.sprite.width, e.sprite.height), (16, 16));
    assert!(selection::resize_canvas(&mut e, 32, 32, 0, 0));
}

#[test]
fn scripts_cannot_grow_a_sprite_without_bound() {
    let mut e = Editor::new(Sprite::new(2, 2));
    let out = script::run(&mut e, "for i = 1, 100000 do sprite:newFrame(true) end", (0, 0), script::Limits::default());
    assert!(out.error.as_deref().is_some_and(|m| m.contains("too many frames")), "{:?}", out.error);
    assert_eq!(e.sprite.frame_count(), 1, "a failed script leaves the document alone");
    let out = script::run(&mut e, "for i = 1, 100000 do sprite:newLayer('x') end", (0, 0), script::Limits::default());
    assert!(out.error.as_deref().is_some_and(|m| m.contains("too many layers")), "{:?}", out.error);
    assert_eq!(e.sprite.layers.len(), 1);
}

#[test]
fn converting_to_a_tilemap_with_absurdly_many_cells_is_refused() {
    let mut s = Sprite::new(4100, 4100);
    s.draw_line(0, 0, 0, 0, 1, 1, RED, 1);
    assert!(!tilemap::convert_to_tilemap(&mut s, 0, 1, 1), "16.8 million 1x1 tiles");
    assert_eq!(s.layers[0].kind, 0);
}

#[test]
fn a_move_started_before_the_sprite_was_replaced_does_not_corrupt_the_new_one() {
    let mut e = Editor::new(Sprite::new(8, 8));
    e.sprite.draw_line(0, 0, 1, 1, 3, 3, RED, 1);
    assert!(selection::move_begin(&mut e, 0, 0));
    e.sprite = Sprite::new(4, 4);
    e.sprite.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
    selection::move_to(&mut e, 1, 1);
    let cel = e.sprite.cels[0][0].as_ref().unwrap();
    assert_eq!(cel.pixels.len(), 4 * 4 * 4, "the cel keeps the canvas size");
}

#[test]
fn a_float_over_a_replaced_sprite_is_dropped_safely() {
    let mut e = Editor::new(Sprite::new(8, 8));
    assert!(selection::paste(&mut e, 0, 0, 2, 2, &[255; 16], 1, 1));
    e.sprite = Sprite::new(3, 3);
    selection::float_commit(&mut e);
    assert!(selection::paste(&mut e, 0, 0, 2, 2, &[255; 16], 0, 0));
    assert_eq!(e.sprite.cels[0][0].as_ref().map(|c| Arc::strong_count(c) > 0), Some(true));
}

#[test]
fn polygon_calls_with_absurd_point_counts_return_early() {
    let mut e = Editor::new(Sprite::new(4, 4));
    let pts = [0i32, 0, 3, 3];
    unsafe {
        pixmith_core::paint::curves::pixmith_polygon(
            &mut e,
            0,
            0,
            pts.as_ptr(),
            usize::MAX / 2 + 1,
            0xFF0000FF,
            1,
            0,
            false,
        );
        selection::pixmith_sel_poly(&mut e, pts.as_ptr(), usize::MAX / 2 + 1);
    }
    assert!(e.sprite.cels[0][0].is_none());
}
