//! Auto-tiling on isometric tilemaps: the terrain engine works on (i, j) cells, so the four sides and
//! four corners of a diamond are the ring directions turned by 45 degrees on screen.

use super::*;
use crate::document::tilemap::{add_iso_tilemap_layer, stamp_cell, tile_in_cell, FLIP_X, FLIP_Y};
use std::collections::HashSet;

/// 128x64 canvas, 16x24 pictures on 16x8 diamonds, cell (0, 0) at (60, 4): a full template tileset in `mode`.
fn setup(mode: Mode) -> (Sprite, usize) {
    let mut s = Sprite::new(128, 64);
    let l = add_iso_tilemap_layer(&mut s, 0, 16, 24, 16, 8, (60, 4));
    assert!(set_terrain(&mut s, l, "Grass", [1, 2, 3, 255], mode));
    generate_template(&mut s, l, false);
    (s, l)
}

fn mask_of(s: &Sprite, l: usize, i: i64, j: i64) -> Option<u8> {
    let v = tile_in_cell(s, l, 0, i, j);
    let at = load(s, l).unwrap();
    let t = ((v & FLIP_X != 0) as u8) | ((v & FLIP_Y != 0) as u8) << 1;
    at.tiles.get(&(v & TILE_ID_MASK)).map(|m| {
        let mut out = 0;
        for &d in at.mode.dirs() {
            if m.mask(at.mode) & at.mode.bit(d).unwrap() != 0 {
                out |= at.mode.bit(transform_dir_iso(d, t)).unwrap();
            }
        }
        out
    })
}

#[test]
fn flips_move_neighbours_as_the_picture_moves_on_screen() {
    // N, NE, E, SE, S, SW, W, NW = 0..8. Flipping X mirrors left-right: the upper right edge (N) becomes the
    // upper left edge (W), the right corner (NE) the left corner (SW), the lower right edge (E) the lower left (S).
    let flip_x: Vec<usize> = (0..8).map(|d| transform_dir_iso(d, 1)).collect();
    assert_eq!(flip_x, [6, 5, 4, 3, 2, 1, 0, 7]);
    // Flipping Y mirrors top-bottom: N (upper right) becomes E (lower right), NE and SW stay, W becomes S.
    let flip_y: Vec<usize> = (0..8).map(|d| transform_dir_iso(d, 2)).collect();
    assert_eq!(flip_y, [2, 1, 0, 7, 6, 5, 4, 3]);
    // Both is a half turn.
    let both: Vec<usize> = (0..8).map(|d| transform_dir_iso(d, 3)).collect();
    assert_eq!(both, [4, 5, 6, 7, 0, 1, 2, 3]);
    // The square engine flips differently, so the two must not be confused.
    assert_ne!(flip_x, (0..8).map(|d| transform_dir(d, 1)).collect::<Vec<_>>());
}

/// Screen position (relative to the diamond's center) of the mark for each ring direction.
fn spot(d: usize, a: f64, b: f64) -> (f64, f64) {
    [
        (a / 2.0, -b / 2.0),
        (a, 0.0),
        (a / 2.0, b / 2.0),
        (0.0, b),
        (-a / 2.0, b / 2.0),
        (-a, 0.0),
        (-a / 2.0, -b / 2.0),
        (0.0, -b),
    ][d]
}

#[test]
fn transform_dirs_match_rendering_on_screen() {
    // A marker at the screen spot of direction N (the upper right edge) must land at the spot of the direction
    // `transform_dir_iso` names when the tile is drawn flipped.
    let (tw, th) = (16usize, 8usize);
    let mut ts = crate::document::tilemap::Tileset::new_iso("t", tw as u32, th as u32, 16, 8);
    let id = ts.add_blank() as usize;
    let (cx, cy) = (tw as f64 / 2.0, th as f64 / 2.0);
    let (mx, my) = spot(0, 8.0, 4.0);
    let (px, py) = ((cx + mx - 0.5) as usize, (cy + my - 0.5) as usize);
    std::sync::Arc::make_mut(&mut ts.tiles[id])[(py * tw + px) * 4..][..4].copy_from_slice(&[255, 0, 0, 255]);
    for t in 0..4u8 {
        let g = TileGrid { x: 0, y: 0, w: 1, h: 1, tiles: vec![id as u32 | transform_flags(t)] };
        let out = crate::document::tilemap::render_iso(&g, &ts, (0, 0), tw, th);
        let mut found = None;
        for y in 0..th {
            for x in 0..tw {
                if out[(y * tw + x) * 4 + 3] != 0 {
                    found = Some((x as f64 + 0.5 - cx, y as f64 + 0.5 - cy));
                }
            }
        }
        let (fx, fy) = found.expect("the marker is drawn");
        let want = spot(transform_dir_iso(0, t), 8.0, 4.0);
        assert!((fx - want.0).abs() <= 1.0 && (fy - want.1).abs() <= 1.0, "transform {t}: {:?} vs {want:?}", (fx, fy));
    }
}

#[test]
fn iso_sides_follow_the_cell_axes() {
    let (mut s, l) = setup(Mode::Sides);
    for (i, j) in [(4, 4), (4, 3), (5, 4), (4, 5), (3, 4)] {
        paint_terrain(&mut s, l, 0, i, j, Some(1));
    }
    // Sides: N = 1 (the cell at j - 1), E = 2 (i + 1), S = 4 (j + 1), W = 8 (i - 1).
    assert_eq!(mask_of(&s, l, 4, 4), Some(15));
    assert_eq!(mask_of(&s, l, 4, 3), Some(4), "its only neighbour is below in cell space (S)");
    assert_eq!(mask_of(&s, l, 5, 4), Some(8));
    assert_eq!(mask_of(&s, l, 4, 5), Some(1));
    assert_eq!(mask_of(&s, l, 3, 4), Some(2));
    assert!(missing_cells(&s, l, 0).is_empty());
    // On screen, E (i + 1) is down and to the right.
    let a = cell_origin_of(&s, l, 4, 4);
    let b = cell_origin_of(&s, l, 5, 4);
    assert_eq!((b.0 - a.0, b.1 - a.1), (8, 4));
}

fn cell_origin_of(s: &Sprite, l: usize, i: i64, j: i64) -> (i64, i64) {
    crate::document::tilemap::cell_origin(s, l, i, j).unwrap()
}

#[test]
fn iso_blob_resolves_every_neighbourhood() {
    let (s, l) = setup(Mode::Blob);
    for m in 0..=255u8 {
        let mut s2 = s.clone();
        paint_terrain(&mut s2, l, 0, 5, 5, Some(1));
        for d in 0..8 {
            if m & (1 << d) != 0 {
                paint_terrain(&mut s2, l, 0, 5 + DIRS[d].0, 5 + DIRS[d].1, Some(1));
            }
        }
        assert_eq!(mask_of(&s2, l, 5, 5), Some(reduce(m)), "mask {m}");
        assert!(!missing_cells(&s2, l, 0).contains(&(5, 5)), "mask {m}");
    }
}

#[test]
fn a_small_iso_map_gets_the_matching_edge_and_corner_tiles() {
    // A 3x3 block of cells: corners (0, 0), (2, 0), (0, 2), (2, 2) show two sides and one corner each.
    let (mut s, l) = setup(Mode::Blob);
    let cells: Vec<(i32, i32)> = (3..6).flat_map(|j| (3..6).map(move |i| (i, j))).collect();
    paint_cells(&mut s, l, 0, &cells, Some(1));
    let n = N;
    let (e, so, w) = (E, S, W);
    assert_eq!(mask_of(&s, l, 4, 4), Some(255), "the middle has all eight neighbours");
    assert_eq!(mask_of(&s, l, 3, 3), Some(so | e | SE), "top corner cell: S, E and SE");
    assert_eq!(mask_of(&s, l, 5, 3), Some(so | w | SW));
    assert_eq!(mask_of(&s, l, 3, 5), Some(n | e | NE));
    assert_eq!(mask_of(&s, l, 5, 5), Some(n | w | NW));
    assert_eq!(mask_of(&s, l, 4, 3), Some(so | e | w | SE | SW), "edge cell");
    assert!(missing_cells(&s, l, 0).is_empty());
    // The tiles are where the cells are on screen: the (3, 3) corner is the top of the diamond-shaped patch.
    let top = cell_origin_of(&s, l, 3, 3);
    for (i, j) in [(4, 3), (3, 4), (4, 4), (5, 5)] {
        assert!(cell_origin_of(&s, l, i, j).1 > top.1);
    }
}

#[test]
fn flipped_tiles_stand_in_for_the_mirrored_neighbourhood() {
    let mut at = Autotile { mode: Mode::Sides, iso: true, ..Default::default() };
    // A tile with a neighbour to the E (lower right on screen), usable flipped left-right.
    at.tiles.insert(1, TileMeta { terrain: Some(1), peer: [0, 0, 1, 0, 0, 0, 0, 0], weight: 1.0, sym: 1 });
    let want_s = [0, 0, 0, 0, 1, 0, 0, 0];
    let c = at.choose(Some(1), &want_s, 0, 0, false);
    assert_eq!(c.value, 1 | FLIP_X);
    assert!(c.exact);
    // Flipped top-bottom the E neighbour moves to the N one (upper right).
    at.tiles.get_mut(&1).unwrap().sym = 2;
    let want_n = [1, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(at.choose(Some(1), &want_n, 0, 0, false).value, 1 | FLIP_Y);
    assert!(!at.choose(Some(1), &want_s, 0, 0, false).exact);
    // In a square map the same flip means something else: left-right turns E into W.
    at.iso = false;
    at.tiles.get_mut(&1).unwrap().sym = 1;
    let want_w = [0, 0, 0, 0, 0, 0, 1, 0];
    let c = at.choose(Some(1), &want_w, 0, 0, true);
    assert_eq!(c.value, 1 | FLIP_X);
    assert!(!at.choose(Some(1), &want_s, 0, 0, true).exact);
}

#[test]
fn the_dual_grid_needs_a_half_cell_shift_so_isometric_layers_refuse_it() {
    let (mut s, l) = setup(Mode::Sides);
    assert!(!set_mode(&mut s, l, Mode::Corners));
    assert!(!set_terrain(&mut s, l, "x", [0, 0, 0, 255], Mode::Corners));
    assert_eq!(load(&s, l).unwrap().mode, Mode::Sides);
    // A layer whose stored mode is corners (from another tool) opens as a blob layer.
    let mut at = load(&s, l).unwrap();
    at.mode = Mode::Corners;
    save(&mut s, l, &at);
    assert_eq!(load(&s, l).unwrap().mode, Mode::Blob);
    // A square layer still allows it.
    let mut sq = Sprite::new(32, 32);
    let ql = crate::document::tilemap::add_tilemap_layer(&mut sq, 0, 8, 8);
    assert!(set_terrain(&mut sq, ql, "G", [1, 1, 1, 255], Mode::Corners));
}

#[test]
fn painting_by_pixel_uses_the_layers_offset() {
    let mut e = Editor::new(Sprite::new(1280, 720));
    let doc = &mut e as *mut Editor;
    unsafe {
        let l = crate::document::tilemap::pixmith_add_iso_tilemap_layer(doc, 0, 96, 96, 96, 48, 592, 360);
        assert!(pixmith_autotile_set_terrain(doc, l, std::ptr::null(), 0x336633FF, 1));
        assert_eq!(pixmith_autotile_generate_template(doc, l, false), 47);
        let s = &mut (*doc).sprite;
        // The middle of cell (3, 2): origin (592 + 24, 360 + 120) = (616, 480), middle (664, 504).
        assert_eq!(crate::document::tilemap::cell_at(s, l as usize, 664, 504), Some((3, 2)));
        assert!(pixmith_autotile_paint(doc, l, 0, 664, 504, 1));
        assert_eq!(pixmith_autotile_terrain_at(doc, l, 0, 664, 504), 1);
        assert_eq!(pixmith_autotile_terrain_at(doc, l, 0, 664 + 100, 504), 0);
        let s = &(*doc).sprite;
        assert_ne!(tile_in_cell(s, l as usize, 0, 3, 2) & TILE_ID_MASK, 0);
        // Its picture is on the canvas at the cell: 96x96 picture, pivot (48, 72), center (664, 504) -> top (616, 432).
        let cel = s.cels[0][l as usize].as_ref().unwrap();
        let any = (432..528).any(|y| (616..712).any(|x| cel.pixels[(y * 1280 + x) * 4 + 3] != 0));
        assert!(any, "the tile is drawn at the cell");
        let none_elsewhere = (0..300).all(|y| (0..1280).all(|x| cel.pixels[(y * 1280 + x) * 4 + 3] == 0));
        assert!(none_elsewhere);
        // The missing-cell report gives box corners in canvas pixels.
        let mut out = [0i32; 8];
        let n = pixmith_autotile_missing_cells(doc, l, 0, out.as_mut_ptr(), 8);
        assert!(n == 0 || (out[0], out[1]) == (616, 480));
    }
}

#[test]
fn manual_tiles_win_and_the_map_survives_saving() {
    let (mut s, l) = setup(Mode::Blob);
    paint_cells(&mut s, l, 0, &[(4, 4), (5, 4), (4, 5), (5, 5)], Some(1));
    let before = s.render(0);
    let extra = s.tilesets[0].add_blank();
    // A manual tile in one cell sticks through a rebuild.
    assert!(stamp_cell(&mut s, l, 0, 4, 4, extra));
    crate::gamedev::autotile::note_stamp_cell(&mut s, l, 0, 4, 4, extra);
    rebuild_all(&mut s, l);
    assert_eq!(tile_in_cell(&s, l, 0, 4, 4), extra);
    clear_overrides(&mut s, l, 0);
    assert_eq!(s.render(0), before);
    // Saving and loading keeps intent, tiles and the picture.
    let back = crate::formats::pxmt::read(&crate::formats::pxmt::write(&s).unwrap()).unwrap();
    assert_eq!(back.render(0), s.render(0));
    assert_eq!(terrain_at(&back, l, 0, 5, 5), Some(1));
    assert!(back.tilesets[0].is_iso());
    // A tool for the terrain brush: a block of cells in a region the canvas shows.
    let seen: HashSet<(i32, i32)> = flood_region(&back, l, 0, 4, 4).into_iter().collect();
    assert!(seen.contains(&(5, 5)) && seen.contains(&(4, 5)));
}

#[test]
fn the_template_shows_the_diamond_and_marks_the_set_neighbours() {
    let (s, l) = setup(Mode::Sides);
    let ts = &s.tilesets[s.layers[l].tileset as usize];
    assert_eq!(ts.tiles.len(), 17, "empty tile plus 16 side masks");
    // Mask 15 (all four sides) has all four edge marks.
    let tile = &ts.tiles[16];
    let (tw, th) = (16usize, 24usize);
    let (px, py) = ts.pivot(16);
    let at = |x: i64, y: i64| tile[((y as usize) * tw + x as usize) * 4 + 3];
    assert!(at(px as i64, py as i64 - 4) > 0, "diamond outline at the top");
    assert!(th > 8 && at(px as i64 + 4, py as i64 - 2) > 0, "the upper right edge is marked (N)");
    assert!(at(px as i64 - 4, py as i64 + 2) > 0, "the lower left edge is marked (S)");
    // Mask 0 has the outline only.
    let none = &ts.tiles[1];
    assert!(none[(((py - 2) as usize) * tw + (px + 4) as usize) * 4 + 3] > 0, "outline pixel on the upper right edge");
}
