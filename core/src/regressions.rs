//! Regression tests for bugs the pixmith-qa scenarios found where independently built features meet.

use crate::document::{Editor, Sprite};
use crate::paint::filters::{self, Filter, Target};
use std::sync::Arc;

const RED: [u8; 4] = [255, 0, 0, 255];

/// Renders through the C ABI, which is what the app does.
unsafe fn render(doc: *const Editor, frame: usize) -> Vec<u8> {
    let (w, h) = ((*doc).sprite.width as usize, (*doc).sprite.height as usize);
    let mut out = vec![0u8; w * h * 4];
    assert!(crate::ffi::document::pixmith_render(doc, frame as u32, out.as_mut_ptr(), out.len()));
    out
}

#[test]
fn stroke_on_a_linked_cel_shows_in_the_other_frames_at_once() {
    let doc = Box::into_raw(Box::new(Editor::new(Sprite::new(8, 8))));
    unsafe {
        crate::ffi::document::pixmith_add_frame(doc, 0, false);
        (*doc).sprite.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        assert!((*doc).sprite.link_cels(0, 0, 1));
        crate::paint::shapes::pixmith_stroke_begin(doc, 0, 1, 3, 3, u32::from_be_bytes([0, 0, 255, 255]), 1, 0);
        crate::paint::shapes::pixmith_stroke_to(doc, 5, 3);
        crate::paint::shapes::pixmith_stroke_end();
        // No further edit happens: renders and saves must already see the change in every linked frame.
        let (a, b) = (render(doc, 0), render(doc, 1));
        assert_eq!(a, b);
        assert_eq!(&a[(3 * 8 + 4) * 4..][..4], &[0, 0, 255, 255]);
        let saved = crate::formats::ase::write(&(*doc).sprite);
        let back = crate::formats::ase::read(&saved).unwrap();
        assert_eq!(back.render(0), a);
        drop(Box::from_raw(doc));
    }
}

#[test]
fn drawing_tools_update_tilemap_tiles() {
    let doc = Box::into_raw(Box::new(Editor::new(Sprite::new(16, 16))));
    unsafe {
        let layer = crate::document::tilemap::pixmith_add_tilemap_layer(doc, 0, 8, 8);
        crate::paint::shapes::pixmith_stroke_begin(doc, layer, 0, 2, 2, u32::from_be_bytes(RED), 1, 0);
        crate::paint::shapes::pixmith_stroke_to(doc, 4, 2);
        crate::paint::shapes::pixmith_stroke_end();
        assert_ne!(crate::document::tilemap::pixmith_tile_at(doc, layer, 0, 2, 2), 0, "the cell refers to a tile");
        let saved = crate::formats::ase::write(&(*doc).sprite);
        let back = crate::formats::ase::read(&saved).unwrap();
        assert_eq!(back.render(0), render(doc, 0), "the stroke survives a save");
        drop(Box::from_raw(doc));
    }
}

#[test]
fn filters_respect_the_selection_mask_and_linked_cels() {
    let mut e = Editor::new(Sprite::new(4, 4));
    for y in 0..4 {
        e.sprite.draw_line(0, 0, 0, y, 3, y, RED, 1);
    }
    e.sprite.add_frame(0, false);
    assert!(e.sprite.link_cels(0, 0, 1));
    let doc = &mut e as *mut Editor;
    unsafe {
        crate::paint::selection::pixmith_sel_rect(doc, 0, 0, 1, 3);
        assert!(filters::pixmith_filter_apply(doc, 1, 0, 0, 1, 0, 0, 0, 0, std::ptr::null(), 0));
    }
    for frame in 0..2 {
        let px = e.sprite.render(frame);
        assert_eq!(&px[..4], &[0, 255, 255, 255], "inside the mask, frame {frame}");
        assert_eq!(&px[3 * 4..][..4], &RED, "outside the mask, frame {frame}");
    }
    assert!(e.sprite.is_cel_linked(0, 0) && e.sprite.is_cel_linked(0, 1), "a filter keeps linked cels linked");

    let mut s = Sprite::new(2, 1);
    s.draw_line(0, 0, 0, 0, 1, 0, RED, 1);
    s.add_frame(0, false);
    s.link_cels(0, 0, 1);
    assert!(filters::apply(&mut s, &Filter::from_params(1, &[]).unwrap(), Target::AllFrames, 0, 0, None));
    assert!(Arc::ptr_eq(s.cels[0][0].as_ref().unwrap(), s.cels[1][0].as_ref().unwrap()));
    assert_eq!(&s.cels[1][0].as_ref().unwrap().pixels[..4], &[0, 255, 255, 255], "linked cels are filtered once");
}

#[test]
fn canvas_resize_and_scale_keep_links_and_move_slices() {
    let mut e = Editor::new(Sprite::new(8, 8));
    e.sprite.draw_line(0, 0, 1, 1, 1, 1, RED, 1);
    e.sprite.add_frame(0, false);
    e.sprite.link_cels(0, 0, 1);
    let doc = &mut e as *mut Editor;
    unsafe {
        crate::document::slices::pixmith_slice_add(doc, 2, 2, 3, 3);
    }
    assert!(crate::paint::selection::resize_canvas(&mut e, 12, 12, 2, 2));
    assert!(e.sprite.is_cel_linked(0, 0), "resizing keeps linked cels linked");
    let key = &e.sprite.extra.slices[0].keys[0];
    assert_eq!((key.bounds.x, key.bounds.y), (4, 4), "slices move with the content");
    assert!(filters::scale_sprite(&mut e.sprite, 24, 24));
    assert!(e.sprite.is_cel_linked(0, 0), "scaling keeps linked cels linked");
    let key = &e.sprite.extra.slices[0].keys[0];
    assert_eq!((key.bounds.x, key.bounds.y, key.bounds.w, key.bounds.h), (8, 8, 6, 6), "slices scale with the sprite");
}

#[test]
fn drawing_in_indexed_mode_stays_on_the_palette() {
    let doc = Box::into_raw(Box::new(Editor::new(Sprite::new(8, 8))));
    unsafe {
        let e = &mut *doc;
        e.sprite.draw_line(0, 0, 0, 0, 1, 1, RED, 1);
        crate::document::color_mode::convert(&mut e.sprite, crate::document::color_mode::ColorMode::Indexed, 0);
        crate::paint::ink::pixmith_gradient(
            doc,
            0,
            0,
            0,
            0,
            7,
            0,
            u32::from_be_bytes(RED),
            u32::from_be_bytes([0, 0, 255, 255]),
            0,
            0,
            false,
        );
        let palette = e.sprite.palette.clone();
        for p in e.sprite.render(0).chunks_exact(4) {
            assert!(p[3] == 0 || palette.contains(&[p[0], p[1], p[2], p[3]]), "{p:?} is not a palette color");
        }
        drop(Box::from_raw(doc));
    }
}

#[test]
fn filters_reach_tilemap_layers_and_strokes_do_not_pile_up_tiles() {
    let doc = Box::into_raw(Box::new(Editor::new(Sprite::new(32, 8))));
    unsafe {
        let layer = crate::document::tilemap::pixmith_add_tilemap_layer(doc, 0, 8, 8);
        let tiles = |d: *mut Editor| {
            let sprite = &(*d).sprite;
            sprite.tilesets[0].tiles.len()
        };
        let start = tiles(doc);
        // A long stroke across four cells allocates one tile per cell, however many segments it has.
        crate::paint::shapes::pixmith_stroke_begin(doc, layer, 0, 1, 3, u32::from_be_bytes(RED), 1, 0);
        for x in 2..30 {
            crate::paint::shapes::pixmith_stroke_to(doc, x, 3);
        }
        crate::paint::shapes::pixmith_stroke_end();
        assert_eq!(tiles(doc), start + 4);
        // A gesture that paints and erases the same cell leaves no tile behind.
        crate::paint::shapes::pixmith_stroke_begin(doc, layer, 0, 4, 6, u32::from_be_bytes(RED), 1, 0);
        crate::paint::shapes::pixmith_stroke_to(doc, 5, 6);
        crate::paint::shapes::pixmith_stroke_style(doc, 0, 1);
        crate::paint::shapes::pixmith_stroke_to(doc, 4, 6);
        crate::paint::shapes::pixmith_stroke_to(doc, 5, 6);
        crate::paint::shapes::pixmith_stroke_end();
        let tile_after = tiles(doc);
        assert!(tile_after <= start + 4, "tiles: {tile_after}");
        // Filters change the tiles that the cel uses.
        assert!(filters::pixmith_filter_apply(doc, 1, 0, layer, 0, 0, 0, 0, 0, std::ptr::null(), 0));
        let px = (*doc).sprite.render(0);
        assert_eq!(&px[(3 * 32 + 1) * 4..][..4], &[0, 255, 255, 255], "invert reached the tile pixels");
        let saved = crate::formats::ase::write(&(*doc).sprite);
        assert_eq!(crate::formats::ase::read(&saved).unwrap().render(0), px, "and survives a save");
        assert!(crate::ffi::document::pixmith_undo(doc));
        assert_eq!(&(*doc).sprite.render(0)[(3 * 32 + 1) * 4..][..4], &RED);
        drop(Box::from_raw(doc));
    }
}

fn tilemap_pair() -> (*mut Editor, u32) {
    let doc = Box::into_raw(Box::new(Editor::new(Sprite::new(16, 16))));
    unsafe {
        let layer = crate::document::tilemap::pixmith_add_tilemap_layer(doc, 0, 8, 8);
        crate::paint::shapes::pixmith_stroke_begin(doc, layer, 0, 2, 2, u32::from_be_bytes(RED), 1, 0);
        crate::paint::shapes::pixmith_stroke_to(doc, 4, 2);
        crate::paint::shapes::pixmith_stroke_end();
        crate::ffi::document::pixmith_add_frame(doc, 0, false);
        (doc, layer)
    }
}

#[test]
fn linked_tilemap_cels_stay_linked_through_a_save_and_a_script() {
    let (doc, layer) = tilemap_pair();
    let l = layer as usize;
    unsafe {
        assert!(crate::document::layers::pixmith_cel_link(doc, layer, 0, 1));
        assert!(crate::document::layers::pixmith_cel_is_linked(doc, layer, 0));
        assert!(crate::document::layers::pixmith_cel_is_linked(doc, layer, 1));
        let o = crate::scripting::run(
            &mut *doc,
            "local n = 0 for _, c in ipairs(app.activeSprite.cels) do if c.linked then n = n + 1 end end print(n)",
            (l, 0),
            Default::default(),
        );
        assert_eq!((o.error, o.messages), (None, vec!["2".to_string()]));
        let back = crate::formats::ase::read(&crate::formats::ase::write(&(*doc).sprite)).unwrap();
        assert!(back.is_cel_linked(l, 0) && back.is_cel_linked(l, 1), "the link survives a save");
        assert_eq!(back.render(1), back.render(0));
        assert!(back.cels[0][l].as_ref().unwrap().tiles.is_some(), "and it is still a tilemap cel");
        // The same through .pxmt.
        let pxmt = crate::formats::pxmt::write(&(*doc).sprite).unwrap();
        assert!(crate::formats::pxmt::read(&pxmt).unwrap().is_cel_linked(l, 1));
        // Unlinking is also respected by the file.
        assert!(crate::document::layers::pixmith_cel_unlink(doc, layer, 1));
        let back = crate::formats::ase::read(&crate::formats::ase::write(&(*doc).sprite)).unwrap();
        assert!(!back.is_cel_linked(l, 0));
        drop(Box::from_raw(doc));
    }
}

#[test]
fn saving_unsynced_linked_tilemap_cels_keeps_the_link_and_the_edit() {
    let (doc, layer) = tilemap_pair();
    let l = layer as usize;
    unsafe {
        let s = &mut (*doc).sprite;
        assert!(s.link_cels(l, 0, 1));
        // A pixel edit that did not go through the tile-aware tools: saving writes it back into the tiles.
        let cel = Arc::make_mut(s.cels[0][l].as_mut().unwrap());
        cel.pixels[(10 * 16 + 10) * 4..][..4].copy_from_slice(&RED);
        s.cels[1][l] = s.cels[0][l].clone();
        let back = crate::formats::ase::read(&crate::formats::ase::write(s)).unwrap();
        assert!(back.is_cel_linked(l, 0) && back.is_cel_linked(l, 1));
        assert_eq!(&back.render(1)[(10 * 16 + 10) * 4..][..4], &RED);
        drop(Box::from_raw(doc));
    }
}
