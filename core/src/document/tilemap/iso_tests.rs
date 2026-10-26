//! Tests for isometric tilemaps: cell math, rendering, draw order, tall tiles, layer offsets, pixel edits,
//! canvas operations and file round trips.

use super::*;
use crate::document::{isoext, Layer};
use crate::formats::{ase, pxmt};

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const YELLOW: [u8; 4] = [255, 255, 0, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

unsafe fn ed_<'a>(d: *mut Editor) -> &'a mut Editor {
    &mut *d
}

fn px(s: &Sprite, x: usize, y: usize) -> [u8; 4] {
    let r = s.render(0);
    r[(y * s.width as usize + x) * 4..][..4].try_into().unwrap()
}

fn layer_px(s: &Sprite, layer: usize, x: usize, y: usize) -> [u8; 4] {
    let cel = s.cels[0][layer].as_ref().expect("cel");
    cel.pixels[(y * s.width as usize + x) * 4..][..4].try_into().unwrap()
}

/// A tile image that is `c` everywhere.
fn solid(ts: &Tileset, c: [u8; 4]) -> Vec<u8> {
    c.repeat(ts.tile_w as usize * ts.tile_h as usize)
}

/// Adds a tile of color `c` to tileset `ti` and returns its index.
fn add_solid(s: &mut Sprite, ti: usize, c: [u8; 4]) -> u32 {
    let img = solid(&s.tilesets[ti], c);
    let n = s.tilesets[ti].add_blank();
    s.tilesets[ti].tiles[n as usize] = Arc::new(img);
    n
}

/// A canvas with one isometric layer: `tile` images on `cell` diamonds, cell (0, 0)'s box at `offset`.
fn iso_sprite(canvas: (u32, u32), tile: (u32, u32), cell: (u32, u32), offset: (i32, i32)) -> (Sprite, usize) {
    let mut s = Sprite::new(canvas.0, canvas.1);
    let l = add_iso_tilemap_layer(&mut s, 0, tile.0, tile.1, cell.0, cell.1, offset);
    (s, l)
}

#[test]
fn square_tilesets_are_unchanged() {
    let mut s = Sprite::new(16, 16);
    let l = add_tilemap_layer(&mut s, 0, 8, 8);
    assert!(!s.tilesets[0].is_iso() && s.tilesets[0].iso.is_none());
    assert_eq!(s.layers[l].offset, (0, 0));
    let red = add_solid(&mut s, 0, RED);
    assert!(stamp(&mut s, l, 0, 9, 1, red));
    assert_eq!(px(&s, 8, 0), RED);
    assert_eq!(px(&s, 15, 7), RED);
    assert_eq!(px(&s, 7, 0), CLEAR);
    // Iso-only operations refuse square layers and tilesets.
    assert!(!set_layer_offset(&mut s, l, (3, 3)));
    assert!(!set_tile_pivot(&mut s, 0, red, Some((1, 1))));
    assert!(!stamp_cell(&mut s, l, 0, 0, 0, red));
    assert!(tile_hit_at(&s, l, 0, 9, 1).is_none());
    // Square pixel-to-cell helpers keep their meaning.
    assert_eq!(cell_at(&s, l, 9, 1), Some((1, 0)));
    assert_eq!(cell_origin(&s, l, 1, 1), Some((8, 8)));
    assert_eq!(
        render_cel(s.cels[0][l].as_ref().unwrap().tiles.as_ref().unwrap(), &s.tilesets[0], (5, 5), 16, 16),
        s.cels[0][l].as_ref().unwrap().pixels
    );
}

#[test]
fn a_96_by_48_tile_lands_where_the_map_formula_puts_it() {
    // The game's map: 96x48 diamonds, cell (0, 0) at (592, 360); cell (i, j) at (x + (i-j)*w/2, y + (i+j)*h/2).
    let (mut s, l) = iso_sprite((1280, 720), (96, 48), (96, 48), (592, 360));
    let red = add_solid(&mut s, 0, RED);
    let green = add_solid(&mut s, 0, GREEN);
    assert!(stamp_cell(&mut s, l, 0, 0, 0, red));
    assert!(stamp_cell(&mut s, l, 0, 2, 1, green));
    // Cell (0, 0): box (592, 360)..(688, 408).
    assert_eq!(layer_px(&s, l, 592, 360), RED);
    assert_eq!(layer_px(&s, l, 687, 407), RED);
    assert_eq!(layer_px(&s, l, 591, 360), CLEAR);
    assert_eq!(layer_px(&s, l, 688, 360), CLEAR);
    assert_eq!(layer_px(&s, l, 592, 359), CLEAR);
    assert_eq!(layer_px(&s, l, 592, 408), CLEAR);
    // Cell (2, 1): box at (592 + 48, 360 + 72) = (640, 432).
    assert_eq!(layer_px(&s, l, 640, 432), GREEN);
    assert_eq!(layer_px(&s, l, 735, 479), GREEN);
    assert_eq!(layer_px(&s, l, 639, 432), CLEAR);
    assert_eq!(cell_origin(&s, l, 2, 1), Some((640, 432)));
    assert_eq!(cell_origin(&s, l, -1, 3), Some((592 - 192, 360 + 48)));
}

#[test]
fn a_32_by_16_grid_at_the_origin() {
    let (mut s, l) = iso_sprite((128, 64), (32, 16), (32, 16), (0, 0));
    let red = add_solid(&mut s, 0, RED);
    assert!(stamp_cell(&mut s, l, 0, 3, 0, red));
    // (3, 0): box at (48, 24).
    assert_eq!(layer_px(&s, l, 48, 24), RED);
    assert_eq!(layer_px(&s, l, 79, 39), RED);
    assert_eq!(layer_px(&s, l, 47, 24), CLEAR);
    assert_eq!(tile_at(&s, l, 0, 64, 32), red, "the pixel in the middle of the diamond finds the cell");
    assert_eq!(tile_at(&s, l, 0, 4, 4), 0);
}

#[test]
fn tall_tiles_are_drawn_back_to_front_by_i_plus_j_then_i() {
    // 8x12 images on 8x4 cells at (16, 16): each image sticks 8 pixels above its diamond.
    let (mut s, l) = iso_sprite((48, 48), (8, 12), (8, 4), (16, 16));
    let (a, b, c, d) = (
        add_solid(&mut s, 0, RED),
        add_solid(&mut s, 0, GREEN),
        add_solid(&mut s, 0, BLUE),
        add_solid(&mut s, 0, YELLOW),
    );
    // Insert in a scrambled order: the result must not depend on it.
    for (i, j, t) in [(1, 1, d), (0, 1, c), (0, 0, a), (1, 0, b)] {
        assert!(stamp_cell(&mut s, l, 0, i, j, t));
    }
    // A at (16,16) draws x 16..24, y 8..20; B (1,0) at (20,18): x 20..28, y 10..22; C (0,1) at (12,18): x 12..20,
    // y 10..22; D (1,1) at (16,20): x 16..24, y 12..24.
    assert_eq!(layer_px(&s, l, 17, 9), RED, "only A");
    assert_eq!(layer_px(&s, l, 22, 11), GREEN, "A and B: B is in front (i + j = 1 beats 0)");
    assert_eq!(layer_px(&s, l, 18, 11), BLUE, "A and C: C is in front");
    assert_eq!(layer_px(&s, l, 20, 14), YELLOW, "D (i + j = 2) is in front of everything");
    assert_eq!(layer_px(&s, l, 26, 11), GREEN, "only B");
    assert_eq!(layer_px(&s, l, 13, 11), BLUE, "only C");
    // C (0, 1) is drawn before B (1, 0): same i + j, smaller i first.
    let mut cells = vec![(1, 0), (0, 1)];
    cells.sort_by(|x, y| iso::draw_order(*x, *y));
    assert_eq!(cells, [(0, 1), (1, 0)]);
    // The same map built in another order renders identically.
    let (mut s2, l2) = iso_sprite((48, 48), (8, 12), (8, 4), (16, 16));
    for c in [RED, GREEN, BLUE, YELLOW] {
        add_solid(&mut s2, 0, c);
    }
    for (i, j, t) in [(0, 0, a), (1, 0, b), (0, 1, c), (1, 1, d)] {
        stamp_cell(&mut s2, l2, 0, i, j, t);
    }
    assert_eq!(s.cels[0][l].as_ref().unwrap().pixels, s2.cels[0][l2].as_ref().unwrap().pixels);
}

#[test]
fn overlapping_translucent_pixels_blend_source_over() {
    let mut d = [0, 0, 255, 255];
    blend_over(&mut d, &[255, 0, 0, 128]);
    assert_eq!(d[3], 255);
    assert!((d[0] as i32 - 128).abs() <= 1 && (d[2] as i32 - 127).abs() <= 1, "{d:?}");
    let mut empty = [9, 9, 9, 0];
    blend_over(&mut empty, &[1, 2, 3, 40]);
    assert_eq!(empty, [1, 2, 3, 40]);
    let mut keep = [5, 6, 7, 99];
    blend_over(&mut keep, &[1, 2, 3, 0]);
    assert_eq!(keep, [5, 6, 7, 99]);
    let mut two = [0, 200, 0, 128];
    blend_over(&mut two, &[200, 0, 0, 128]);
    assert_eq!(two[3], 192, "alpha 128 over alpha 128 is 192");
}

#[test]
fn tiles_are_clipped_to_the_canvas() {
    let (mut s, l) = iso_sprite((20, 20), (8, 4), (8, 4), (0, 0));
    let red = add_solid(&mut s, 0, RED);
    for (i, j) in [(0, 0), (-1, -1), (30, 2), (2, 30), (-5, 9), (3, 3)] {
        assert!(stamp_cell(&mut s, l, 0, i, j, red));
    }
    assert_eq!(layer_px(&s, l, 0, 0), RED);
    let n = s.cels[0][l].as_ref().unwrap().pixels.len();
    assert_eq!(n, 20 * 20 * 4);
    // (3, 3) is at (0, 12): x 0..8, y 12..16.
    assert_eq!(layer_px(&s, l, 7, 15), RED);
    assert_eq!(layer_px(&s, l, 8, 15), CLEAR);
    // Tile partly left of the canvas: cell (-1, -1) at (0, -4): visible rows y 0..0 only.
    assert!(s.validate().is_ok());
}

#[test]
fn tall_tiles_default_to_bottom_aligned_and_take_a_custom_pivot() {
    let (mut s, l) = iso_sprite((300, 300), (96, 96), (96, 48), (100, 100));
    let t = add_solid(&mut s, 0, RED);
    assert_eq!(s.tilesets[0].pivot(t), (48, 72), "the diamond is the bottom 48 rows of the picture");
    assert_eq!(s.tilesets[0].texture_origin(t), (0, 24));
    stamp_cell(&mut s, l, 0, 0, 0, t);
    // Cell center (148, 124): picture top-left (100, 52).
    assert_eq!(layer_px(&s, l, 100, 52), RED);
    assert_eq!(layer_px(&s, l, 100, 51), CLEAR);
    assert_eq!(layer_px(&s, l, 195, 147), RED);
    assert_eq!(layer_px(&s, l, 195, 148), CLEAR);
    // Center the picture on the diamond instead.
    assert!(set_tile_pivot(&mut s, 0, t, Some((48, 48))));
    assert_eq!(s.tilesets[0].texture_origin(t), (0, 0));
    assert_eq!(layer_px(&s, l, 100, 76), RED);
    assert_eq!(layer_px(&s, l, 100, 75), CLEAR);
    // Going back to the default.
    assert!(set_tile_pivot(&mut s, 0, t, None));
    assert_eq!(layer_px(&s, l, 100, 52), RED);
    // A plain 96x48 tile has its pivot in the middle either way.
    let (s2, _) = iso_sprite((10, 10), (96, 48), (96, 48), (0, 0));
    assert_eq!(s2.tilesets[0].pivot(0), (48, 24));
    assert_eq!(s2.tilesets[0].texture_origin(0), (0, 0));
}

#[test]
fn flips_mirror_the_picture_and_its_pivot() {
    let (mut s, l) = iso_sprite((300, 300), (96, 96), (96, 48), (100, 100));
    let t = s.tilesets[0].add_blank();
    // Top row red, everything else blue.
    let mut img = BLUE.repeat(96 * 96);
    for x in 0..96 {
        img[x * 4..x * 4 + 4].copy_from_slice(&RED);
    }
    s.tilesets[0].tiles[t as usize] = Arc::new(img);
    stamp_cell(&mut s, l, 0, 0, 0, t);
    assert_eq!(layer_px(&s, l, 100, 52), RED, "the red row is the top row");
    // Flipped vertically the pivot is 24 from the top: picture top at 124 - 24 = 100, red row at the bottom.
    stamp_cell(&mut s, l, 0, 0, 0, t | FLIP_Y);
    assert_eq!(layer_px(&s, l, 100, 100 + 95), RED);
    assert_eq!(layer_px(&s, l, 100, 100), BLUE);
    assert_eq!(layer_px(&s, l, 100, 52), CLEAR);
}

#[test]
fn layers_at_different_elevations_use_their_own_pixel_offsets() {
    let mut s = Sprite::new(1280, 720);
    let floor = add_iso_tilemap_layer(&mut s, 0, 96, 48, 96, 48, (592, 360));
    let ledge = add_iso_tilemap_layer(&mut s, floor, 96, 48, 96, 48, (592, 168));
    let red = add_solid(&mut s, 0, RED);
    let green = add_solid(&mut s, 1, GREEN);
    stamp_cell(&mut s, floor, 0, 0, 0, red);
    stamp_cell(&mut s, ledge, 0, 0, 0, green);
    assert_eq!(layer_px(&s, floor, 592, 360), RED);
    assert_eq!(layer_px(&s, ledge, 592, 168), GREEN, "the ledge top is at y 168, not on the floor's 48 px steps");
    assert_eq!(layer_px(&s, ledge, 592, 360), CLEAR);
    // An offset half a cell (24 px) off the floor's grid works: same cell, 24 px higher.
    assert!(set_layer_offset(&mut s, ledge, (592, 360 - 24)));
    assert_eq!(layer_px(&s, ledge, 592, 336), GREEN);
    assert_eq!(layer_px(&s, ledge, 592, 168), CLEAR);
    assert_eq!(cell_origin(&s, ledge, 0, 0), Some((592, 336)));
    assert_eq!(cell_origin(&s, floor, 0, 0), Some((592, 360)));
    assert_eq!(cell_at(&s, ledge, 640, 360), Some((0, 0)));
    // The cell under the same pixel differs per layer.
    assert_ne!(cell_at(&s, ledge, 700, 340), cell_at(&s, floor, 700, 340));
    // The pixels render in the composite at the layer's position.
    assert_eq!(px(&s, 592, 336), GREEN);
}

#[test]
fn offsets_can_be_set_undone_and_do_not_apply_to_square_layers() {
    let mut e = Editor::new(Sprite::new(64, 64));
    let l = add_iso_tilemap_layer(&mut e.sprite, 0, 8, 4, 8, 4, (10, 10));
    let t = add_solid(&mut e.sprite, 0, RED);
    stamp_cell(&mut e.sprite, l, 0, 0, 0, t);
    let doc = &mut e as *mut Editor;
    unsafe {
        assert!(pixmith_layer_set_grid_offset(doc, l as u32, 30, 20));
        assert!(ed_(doc).sprite.layers[l].offset == (30, 20));
        assert_eq!(layer_px(&ed_(doc).sprite, l, 30, 20), RED);
        assert_eq!(layer_px(&ed_(doc).sprite, l, 10, 10), CLEAR);
        let mut out = [0i32; 2];
        assert!(pixmith_layer_grid_offset(doc, l as u32, out.as_mut_ptr()));
        assert_eq!(out, [30, 20]);
        assert!(ed_(doc).undo());
        assert_eq!(ed_(doc).sprite.layers[l].offset, (10, 10));
        assert_eq!(layer_px(&ed_(doc).sprite, l, 10, 10), RED);
        assert!(!pixmith_layer_grid_offset(doc, 0, out.as_mut_ptr()), "the plain layer has no grid");
    }
}

#[test]
fn pixels_map_to_cells_for_stamping_and_picking() {
    let (mut s, l) = iso_sprite((1280, 720), (96, 48), (96, 48), (592, 360));
    let t = add_solid(&mut s, 0, RED);
    // The middle of cell (0, 0) is (640, 384).
    assert!(stamp(&mut s, l, 0, 640, 384, t));
    assert_eq!(tile_at(&s, l, 0, 640, 384), t);
    assert_eq!(tile_at(&s, l, 0, 640 + 40, 384), t, "inside the right half of the diamond");
    assert_eq!(tile_at(&s, l, 0, 640 + 50, 384), 0, "past the right corner is cell (1, -1)");
    // Stamping a pixel in the cell down and to the right grows the grid.
    assert!(stamp(&mut s, l, 0, 640 + 24, 384 + 12 + 12, t | FLIP_X));
    assert_eq!(tile_at(&s, l, 0, 640 + 24, 384 + 24), t | FLIP_X);
    assert_eq!(tile_in_cell(&s, l, 0, 1, 0), t | FLIP_X);
    // A cell left of the grid origin (negative cell coordinates) grows the grid the other way.
    assert!(stamp_cell(&mut s, l, 0, -2, 3, t));
    assert_eq!(tile_in_cell(&s, l, 0, -2, 3), t);
    assert_eq!(tile_in_cell(&s, l, 0, 0, 0), t);
    let g = s.cels[0][l].as_ref().unwrap().tiles.as_ref().unwrap();
    assert_eq!((g.x, g.y), (-2, 0));
    // Clearing a cell.
    assert!(stamp_cell(&mut s, l, 0, 0, 0, 0));
    assert_eq!(tile_in_cell(&s, l, 0, 0, 0), 0);
    // Out of range cells are refused.
    assert!(!stamp_cell(&mut s, l, 0, 100_000, 0, t));
    assert!(!stamp_cell(&mut s, l, 0, 0, 0, 99), "no such tile");
}

#[test]
fn picking_finds_the_visible_tall_tile_before_the_diamond_cell() {
    // 8x12 images on 8x4 diamonds: a tile in cell (1, 1) sticks up over cell (0, 0)'s diamond.
    let (mut s, l) = iso_sprite((48, 48), (8, 12), (8, 4), (16, 16));
    let a = add_solid(&mut s, 0, RED);
    let d = add_solid(&mut s, 0, YELLOW);
    stamp_cell(&mut s, l, 0, 0, 0, a);
    stamp_cell(&mut s, l, 0, 1, 1, d);
    // Pixel (20, 14) lies in the diamond of cell (0, 0)'s box? Cell (0,0) is (16,16)..(24,20); (20,14) is above it.
    assert_ne!(cell_at(&s, l, 20, 14), Some((0, 0)), "geometrically the pixel is above cell (0, 0)");
    let (cell, v) = tile_hit_at(&s, l, 0, 20, 14).expect("D's picture is visible there");
    assert_eq!((cell, v), ((1, 1), d));
    // Where only A shows, A is found.
    let (cell, v) = tile_hit_at(&s, l, 0, 17, 9).unwrap();
    assert_eq!((cell, v), ((0, 0), a));
    // Empty ground gives nothing.
    assert!(tile_hit_at(&s, l, 0, 45, 45).is_none());
}

#[test]
fn painted_pixels_are_written_back_to_the_owning_tile() {
    let (mut s, l) = iso_sprite((48, 48), (8, 12), (8, 4), (16, 16));
    let a = add_solid(&mut s, 0, RED);
    stamp_cell(&mut s, l, 0, 0, 0, a);
    // Paint a green pixel onto A's picture: A changes, not a copy.
    s.draw_line(l, 0, 18, 12, 18, 12, GREEN, 1);
    assert_eq!(s.tilesets[0].tiles.len(), 2, "no new tile");
    let tile = &s.tilesets[0].tiles[a as usize];
    // (18, 12) is at (2, 4) inside the picture whose top-left is (16, 8).
    assert_eq!(&tile[(4 * 8 + 2) * 4..][..4], &GREEN);
    assert_eq!(layer_px(&s, l, 18, 12), GREEN);
    // Painting an empty cell gets a fresh tile, placed with the default pivot, and the grid grows to hold it.
    s.draw_line(l, 0, 40, 40, 40, 40, BLUE, 1);
    assert_eq!(s.tilesets[0].tiles.len(), 3);
    assert_eq!(layer_px(&s, l, 40, 40), BLUE);
    let cell = cell_at(&s, l, 40, 40).unwrap();
    let v = tile_in_cell(&s, l, 0, cell.0, cell.1);
    assert_eq!(v & TILE_ID_MASK, 2);
    // Saving keeps what was painted.
    let back = ase::read(&ase::write(&s)).unwrap();
    assert_eq!(back.render(0), s.render(0));
    // Erasing a pixel of a tall tile removes it from its picture.
    s.draw_line(l, 0, 18, 12, 18, 12, CLEAR, 1);
    let tile = &s.tilesets[0].tiles[a as usize];
    assert_eq!(&tile[(4 * 8 + 2) * 4..][..4], &CLEAR);
}

#[test]
fn editing_a_tile_updates_every_cell_that_shows_it() {
    let (mut s, l) = iso_sprite((64, 32), (8, 4), (8, 4), (0, 0));
    let t = add_solid(&mut s, 0, RED);
    stamp_cell(&mut s, l, 0, 0, 0, t);
    stamp_cell(&mut s, l, 0, 3, 1, t);
    s.draw_line(l, 0, 0, 0, 0, 0, GREEN, 1);
    // Cell (3, 1) at (8, 8): its top-left pixel shows the same edited tile pixel.
    assert_eq!(layer_px(&s, l, 8, 8), GREEN);
}

#[test]
fn iso_grid_math_is_available_from_the_tileset() {
    let ts = Tileset::new_iso("t", 96, 96, 96, 48);
    let g = ts.iso_grid((592, 360)).unwrap();
    assert_eq!((g.w, g.h, g.x, g.y), (96, 48, 592, 360));
    assert_eq!(g.cell_at(640, 384), (0, 0));
    assert!(Tileset::new("sq", 8, 8).iso_grid((0, 0)).is_none());
    // Odd cell sizes are rounded to even ones.
    let odd = Tileset::new_iso("o", 33, 20, 33, 17);
    assert_eq!(odd.iso.as_ref().map(|i| (i.cell_w, i.cell_h)), Some((34, 18)));
}

#[test]
fn canvas_resize_moves_the_layer_offset_not_the_cells() {
    let mut e = Editor::new(Sprite::new(64, 64));
    let l = add_iso_tilemap_layer(&mut e.sprite, 0, 8, 4, 8, 4, (10, 10));
    let t = add_solid(&mut e.sprite, 0, RED);
    stamp_cell(&mut e.sprite, l, 0, 2, 1, t);
    let before = e.sprite.render(0);
    assert!(crate::paint::selection::resize_canvas(&mut e, 80, 80, 6, 4));
    assert_eq!(e.sprite.layers[l].offset, (16, 14));
    let g = e.sprite.cels[0][l].as_ref().unwrap().tiles.as_ref().unwrap();
    assert_eq!((g.x, g.y), (2, 1), "the grid still starts at the same cell");
    // The picture moved with the canvas content.
    let after = e.sprite.render(0);
    for y in 0..64usize {
        for x in 0..64usize {
            assert_eq!(&before[(y * 64 + x) * 4..][..4], &after[((y + 4) * 80 + x + 6) * 4..][..4], "{x},{y}");
        }
    }
    assert!(e.sprite.validate().is_ok());
    // Cropping back restores it.
    assert!(crate::paint::selection::resize_canvas(&mut e, 64, 64, -6, -4));
    assert_eq!(e.sprite.layers[l].offset, (10, 10));
    assert_eq!(e.sprite.render(0), before);
}

#[test]
fn scaling_by_a_whole_factor_scales_cells_pivots_and_offsets() {
    let mut s = Sprite::new(32, 32);
    let l = add_iso_tilemap_layer(&mut s, 0, 8, 12, 8, 4, (4, 2));
    let t = add_solid(&mut s, 0, RED);
    assert!(set_tile_pivot(&mut s, 0, t, Some((3, 9))));
    stamp_cell(&mut s, l, 0, 1, 1, t);
    assert!(crate::paint::filters::scale_sprite(&mut s, 64, 64));
    let ts = &s.tilesets[0];
    assert_eq!((ts.tile_w, ts.tile_h), (16, 24));
    let spec = ts.iso.as_ref().unwrap();
    assert_eq!((spec.cell_w, spec.cell_h), (16, 8));
    assert_eq!(spec.pivots.get(&t), Some(&(6, 18)));
    assert_eq!(s.layers[l].offset, (8, 4));
    assert_eq!(tile_in_cell(&s, l, 0, 1, 1), t);
    assert!(s.validate().is_ok());
    // A skewed scale turns the layer into a normal one (the diamond would stop being 2:1).
    let mut s = Sprite::new(32, 32);
    let l = add_iso_tilemap_layer(&mut s, 0, 8, 4, 8, 4, (4, 2));
    let t = add_solid(&mut s, 0, RED);
    stamp_cell(&mut s, l, 0, 1, 1, t);
    assert!(crate::paint::filters::scale_sprite(&mut s, 64, 32));
    assert_eq!(s.layers[l].kind, 0);
    assert_eq!(s.layers[l].offset, (0, 0));
    assert!(s.tilesets.is_empty());
}

#[test]
fn converting_a_layer_back_to_normal_keeps_the_pixels() {
    let (mut s, l) = iso_sprite((32, 32), (8, 4), (8, 4), (4, 2));
    let t = add_solid(&mut s, 0, RED);
    stamp_cell(&mut s, l, 0, 1, 1, t);
    let before = s.render(0);
    assert!(convert_to_normal(&mut s, l));
    assert_eq!(s.layers[l].offset, (0, 0));
    assert_eq!(s.render(0), before);
    assert!(s.tilesets.is_empty());
}

#[test]
fn tile_management_follows_pivots() {
    let mut s = Sprite::new(32, 32);
    add_iso_tilemap_layer(&mut s, 0, 8, 12, 8, 4, (0, 0));
    let (a, b, c) = (add_solid(&mut s, 0, RED), add_solid(&mut s, 0, GREEN), add_solid(&mut s, 0, BLUE));
    set_tile_pivot(&mut s, 0, b, Some((1, 2)));
    set_tile_pivot(&mut s, 0, c, Some((5, 6)));
    let dup = duplicate_tile(&mut s, 0, b).unwrap();
    assert_eq!(s.tilesets[0].pivot(dup), (1, 2));
    assert!(delete_tile(&mut s, 0, a));
    // b is now tile 1, c tile 2.
    assert_eq!(s.tilesets[0].pivot(1), (1, 2));
    assert_eq!(s.tilesets[0].pivot(2), (5, 6));
    assert!(s.tilesets[0].iso.as_ref().unwrap().pivots.keys().all(|&k| (k as usize) < s.tilesets[0].tiles.len()));
}

#[test]
fn a_tileset_changes_shape_only_while_empty() {
    let mut s = Sprite::new(32, 32);
    let l = add_tilemap_layer(&mut s, 0, 8, 4);
    assert!(set_tileset_iso(&mut s, 0, Some((8, 4))));
    assert!(s.tilesets[0].is_iso());
    let t = add_solid(&mut s, 0, RED);
    assert!(stamp_cell(&mut s, l, 0, 0, 0, t));
    assert!(!set_tileset_iso(&mut s, 0, None), "tiles are in use");
    stamp_cell(&mut s, l, 0, 0, 0, 0);
    assert!(set_tileset_iso(&mut s, 0, None));
    assert!(!s.tilesets[0].is_iso());
}

// MARK: Files

fn rich_iso_sprite() -> Sprite {
    let mut s = Sprite::new(200, 120);
    s.extra.sprite.text = Some("my notes".into());
    let floor = add_iso_tilemap_layer(&mut s, 0, 24, 36, 24, 12, (100, 20));
    let ledge = add_iso_tilemap_layer(&mut s, floor, 24, 36, 24, 12, (100, 8));
    assert!(assign_tileset(&mut s, ledge, 0), "both layers show the same tileset");
    let a = add_solid(&mut s, 0, RED);
    let b = add_solid(&mut s, 0, GREEN);
    set_tile_pivot(&mut s, 0, b, Some((12, 30)));
    for (i, j, t) in [(0, 0, a), (1, 0, b), (0, 1, a), (2, 2, b | FLIP_X)] {
        stamp_cell(&mut s, floor, 0, i, j, t);
    }
    stamp_cell(&mut s, ledge, 0, 1, 1, b);
    // A square tilemap next to them.
    let sq = add_tilemap_layer(&mut s, ledge, 4, 4);
    let t = add_solid(&mut s, 1, BLUE);
    stamp(&mut s, sq, 0, 8, 4, t);
    s
}

#[test]
fn iso_data_round_trips_through_aseprite_files() {
    let s = rich_iso_sprite();
    let back = ase::read(&ase::write(&s)).unwrap();
    assert_eq!(back.tilesets.len(), 2);
    assert_eq!(back.tilesets[0].iso, s.tilesets[0].iso);
    assert_eq!(back.tilesets[0].iso.as_ref().unwrap().pivots.get(&2), Some(&(12, 30)));
    assert!(back.tilesets[1].iso.is_none());
    assert_eq!(back.layers[1].offset, (100, 20));
    assert_eq!(back.layers[2].offset, (100, 8));
    assert_eq!(back.layers[3].offset, (0, 0));
    assert_eq!(back.cels[0][1].as_ref().unwrap().tiles, s.cels[0][1].as_ref().unwrap().tiles);
    assert_eq!(back.render(0), s.render(0), "the picture is the same after the trip");
    // The user's own sprite text is untouched and the extension line is not left in it.
    assert_eq!(back.extra.sprite.text.as_deref(), Some("my notes"));
    assert!(back.validate().is_ok());
}

#[test]
fn pxmt_round_trip_and_version() {
    let s = rich_iso_sprite();
    let bytes = pxmt::write(&s).unwrap();
    assert_eq!(&bytes[8..10], &2u16.to_le_bytes(), "files with isometric tilemaps are version 2");
    let back = pxmt::read(&bytes).unwrap();
    assert_eq!(back.tilesets, s.tilesets);
    assert_eq!(
        back.layers.iter().map(|l| l.offset).collect::<Vec<_>>(),
        s.layers.iter().map(|l| l.offset).collect::<Vec<_>>()
    );
    assert_eq!(back.render(0), s.render(0));
    // And saving again does not accumulate lines.
    let again = pxmt::read(&pxmt::write(&back).unwrap()).unwrap();
    assert_eq!(again.extra.sprite.text.as_deref(), Some("my notes"));
    assert_eq!(again.tilesets, s.tilesets);
}

#[test]
fn files_without_iso_data_stay_version_1_and_carry_no_extension() {
    let mut s = Sprite::new(16, 16);
    let l = add_tilemap_layer(&mut s, 0, 8, 8);
    let t = add_solid(&mut s, 0, RED);
    stamp(&mut s, l, 0, 9, 1, t);
    let bytes = pxmt::write(&s).unwrap();
    assert_eq!(&bytes[8..10], &1u16.to_le_bytes());
    let raw = ase::write(&s);
    assert!(!raw.windows(isoext::PREFIX.len()).any(|w| w == isoext::PREFIX.as_bytes()));
    assert!(isoext::line(&s).is_none());
    let back = pxmt::read(&bytes).unwrap();
    assert_eq!(back.tilesets, s.tilesets);
}

#[test]
fn a_reader_that_does_not_know_the_extension_sees_ordinary_square_tilemaps() {
    let s = rich_iso_sprite();
    let mut raw = ase::write(&s);
    // The extension is one line of the sprite's user data text.
    let at = raw
        .windows(isoext::PREFIX.len())
        .position(|w| w == isoext::PREFIX.as_bytes())
        .expect("the line is in the file");
    // Rename it, as if the reader did not recognize it: same length, so every chunk size stays valid.
    raw[at..at + isoext::PREFIX.len()].copy_from_slice(b"pixmith.iso.v0:");
    let square = ase::read(&raw).unwrap();
    assert!(square.validate().is_ok());
    assert!(square.tilesets.iter().all(|t| !t.is_iso()));
    assert!(square.layers.iter().all(|l| l.offset == (0, 0)));
    assert_eq!(square.tilesets.len(), s.tilesets.len());
    for (a, b) in square.tilesets.iter().zip(&s.tilesets) {
        assert_eq!(
            (a.tile_w, a.tile_h, &a.tiles),
            (b.tile_w, b.tile_h, &b.tiles),
            "the tile images are plain Aseprite tiles"
        );
    }
    assert!(square.layers.iter().zip(&s.layers).all(|(a, b)| a.kind == b.kind && a.tileset == b.tileset));
    // The text it shows is the user's note plus the unknown line, which stays harmlessly in the user data.
    let text = square.extra.sprite.text.unwrap();
    assert!(text.starts_with("my notes\n") && text.contains("pixmith.iso.v0:{"), "{text}");
}

#[test]
fn damaged_or_mismatched_extension_data_is_ignored() {
    let s = rich_iso_sprite();
    let good = isoext::line(&s).unwrap();
    let fresh = || {
        let mut layer = Layer::new("a");
        layer.kind = KIND_TILEMAP;
        (vec![Tileset::new("a", 24, 36)], vec![layer])
    };
    for bad in [
        "not json",
        "{\"v\":99,\"tilesets\":[{\"i\":0,\"tw\":24,\"th\":36,\"cw\":8,\"ch\":4}]}",
        "{\"v\":1,\"tilesets\":[{\"i\":0,\"tw\":5,\"th\":5,\"cw\":8,\"ch\":4}],\"layers\":[]}",
        "{\"v\":1,\"tilesets\":[{\"i\":77,\"tw\":24,\"th\":36,\"cw\":8,\"ch\":4}],\"layers\":[{\"i\":99,\"x\":1,\"y\":2}]}",
        "{\"v\":1,\"tilesets\":[{\"i\":0,\"tw\":24,\"th\":36,\"cw\":-4,\"ch\":0}]}",
        "{\"v\":1,\"layers\":[{\"i\":0,\"x\":1,\"y\":2}]}",
        "[]",
        "",
    ] {
        let (mut tilesets, mut layers) = fresh();
        assert!(!isoext::apply(bad, &mut tilesets, &mut layers), "{bad}");
        assert!(!tilesets[0].is_iso(), "{bad}");
        assert_eq!(layers[0].offset, (0, 0), "{bad}");
    }
    // The good line applies.
    let (mut tilesets, mut layers) = fresh();
    tilesets.push(Tileset::new("b", 4, 4));
    // Layer 0 uses tileset 0, which becomes isometric; the layer line only matters for layers listed in it.
    assert!(isoext::apply(good.strip_prefix(isoext::PREFIX).unwrap(), &mut tilesets, &mut layers));
    assert!(tilesets[0].is_iso() && !tilesets[1].is_iso());
}

#[test]
fn undo_restores_iso_tiles_pivots_and_offsets() {
    let mut e = Editor::new(Sprite::new(64, 64));
    e.checkpoint();
    let l = add_iso_tilemap_layer(&mut e.sprite, 0, 8, 12, 8, 4, (10, 10));
    let t = add_solid(&mut e.sprite, 0, RED);
    e.checkpoint();
    stamp_cell(&mut e.sprite, l, 0, 1, 1, t);
    let doc = &mut e as *mut Editor;
    unsafe {
        assert!(pixmith_tile_set_pivot(doc, 0, t, 2, 3));
        assert_eq!(ed_(doc).sprite.tilesets[0].pivot(t), (2, 3));
        assert!(ed_(doc).undo());
        assert_eq!(ed_(doc).sprite.tilesets[0].pivot(t), (4, 10));
        assert!(ed_(doc).undo());
        assert_eq!(tile_in_cell(&ed_(doc).sprite, l, 0, 1, 1), 0);
        assert_eq!(ed_(doc).sprite.render(0), vec![0; 64 * 64 * 4]);
        assert!(ed_(doc).redo());
        assert_eq!(tile_in_cell(&ed_(doc).sprite, l, 0, 1, 1), t);
    }
}

#[test]
fn cels_only_move_between_layers_with_the_same_grid() {
    let mut s = Sprite::new(64, 64);
    let a = add_iso_tilemap_layer(&mut s, 0, 8, 4, 8, 4, (0, 0));
    let b = add_iso_tilemap_layer(&mut s, a, 8, 4, 8, 4, (0, 8));
    assert!(!s.cel_fits(a, b), "different tilesets");
    let ta = s.layers[a].tileset as usize;
    assert!(assign_tileset(&mut s, b, ta));
    assert!(!s.cel_fits(a, b), "the offsets differ, so the picture would land elsewhere");
    assert!(set_layer_offset(&mut s, b, (0, 0)));
    assert!(s.cel_fits(a, b));
}

#[test]
fn c_abi_helpers_agree_with_the_core() {
    let mut e = Editor::new(Sprite::new(1280, 720));
    let doc = &mut e as *mut Editor;
    unsafe {
        let l = pixmith_add_iso_tilemap_layer(doc, 0, 96, 96, 96, 48, 592, 360);
        assert!(pixmith_layer_is_iso(doc, l) && pixmith_tileset_is_iso(doc, 0));
        assert!(!pixmith_layer_is_iso(doc, 0));
        let mut cell = [0u32; 2];
        assert!(pixmith_tileset_iso_cell(doc, 0, cell.as_mut_ptr()));
        assert_eq!(cell, [96, 48]);
        let t = pixmith_tileset_add_tile(doc, 0);
        let mut v = [0i32; 2];
        assert!(pixmith_tile_pivot(doc, 0, t, v.as_mut_ptr()));
        assert_eq!(v, [48, 72]);
        assert!(!pixmith_tile_pivot_is_set(doc, 0, t));
        assert!(pixmith_tile_texture_origin(doc, 0, t, v.as_mut_ptr()));
        assert_eq!(v, [0, 24]);
        assert!(pixmith_tile_set_pivot(doc, 0, t, 40, 60));
        assert!(pixmith_tile_pivot_is_set(doc, 0, t));
        assert!(pixmith_tile_clear_pivot(doc, 0, t));
        assert!(pixmith_layer_cell_at(doc, l, 640, 384, v.as_mut_ptr()));
        assert_eq!(v, [0, 0]);
        assert!(pixmith_layer_cell_origin(doc, l, 2, 1, v.as_mut_ptr()));
        assert_eq!(v, [640, 432]);
        assert!(pixmith_tile_stamp_cell(doc, l, 0, 2, 1, t));
        assert_eq!(pixmith_tile_in_cell(doc, l, 0, 2, 1), t);
        let mut hit = [0i32; 3];
        assert!(!pixmith_tile_hit_at(doc, l, 0, 700, 430, hit.as_mut_ptr()), "the tile image is transparent");
        assert!(pixmith_iso_cell_at(96, 48, 592, 360, 640, 384, v.as_mut_ptr()));
        assert_eq!(v, [0, 0]);
        assert!(pixmith_iso_cell_origin(96, 48, 592, 360, 2, 1, v.as_mut_ptr()));
        assert_eq!(v, [640, 432]);
        assert!(!pixmith_tile_grid_origin(doc, l, 0, v.as_mut_ptr()), "no square grid on an isometric layer");
        assert!(!pixmith_tileset_set_iso(doc, 0, 0, 0), "tiles in use");
        let mut sq = [0u32; 2];
        assert!(!pixmith_tileset_iso_cell(doc, 99, sq.as_mut_ptr()));
        assert!(!pixmith_layer_cell_at(doc, 99, 0, 0, v.as_mut_ptr()));
    }
}
