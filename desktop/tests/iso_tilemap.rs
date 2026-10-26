//! Isometric tilemaps in the egui app: the New Tilemap Layer sheet, stamping and terrain painting in diamond
//! cells, the layer offset, tile origins, the grid overlay following the active layer and the Godot TileSet sheet.

mod common;

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pixmith_desktop::PixmithApp;
use pixmith_desktop::app::{DocTab, Tool};
use pixmith_desktop::doc::Doc;
use pixmith_desktop::theme::{self, metric};
use pixmith_desktop::ui::canvas::sprite_rect;
use pixmith_desktop::ui::iso::Iso;
use pixmith_desktop::ui::tilemap::{self as tm, FLIP_X, TerrainTool};

const SIZE: [f32; 2] = [1280.0, 800.0];
const ZOOM: f32 = 2.0;

/// A 320 x 240 sprite with an isometric tilemap layer of 32 x 16 diamonds whose first cell sits at (160, 0), and
/// one tile (index 1) to place.
fn iso_doc() -> (Doc, u32, u32) {
    let mut doc = Doc::new(320, 240);
    let layer = doc.new_iso_tilemap_layer(0, (32, 16), (32, 16), (160, 0));
    let ts = doc.tileset_of_layer(layer).unwrap();
    let tile = doc.add_tile(ts).unwrap();
    (doc, layer, tile)
}

fn grid() -> Iso {
    Iso::new(32, 16, 160, 0)
}

/// The pixel at the center of cell `(i, j)` of [`grid`].
fn center(cell: (i64, i64)) -> (i32, i32) {
    let (x, y) = grid().cell_origin(cell);
    (x as i32 + 16, y as i32 + 8)
}

fn app_with(doc: Doc, layer: u32) -> PixmithApp {
    let mut app = PixmithApp::default();
    app.tabs[0] = DocTab::new(doc);
    app.tabs[0].layer = layer;
    app.tabs[0].zoom = ZOOM;
    let tab = &app.tabs[0];
    app.canvas.keep_view(tab);
    app
}

fn harness(app: PixmithApp) -> Harness<'static, PixmithApp> {
    let mut installed = false;
    let mut h = Harness::builder().with_size(SIZE).build_ui_state(
        move |ui, app: &mut PixmithApp| {
            if !installed {
                theme::install(ui.ctx());
                installed = true;
            }
            app.ui(ui);
        },
        app,
    );
    h.run();
    h
}

fn at(h: &Harness<'static, PixmithApp>, p: (i32, i32)) -> egui::Pos2 {
    let app = h.state();
    let timeline = pixmith_desktop::ui::timeline::panel_height(app);
    let area = egui::Rect::from_min_max(
        egui::pos2(metric::TOOL_RAIL, 2.0 * metric::BAR_HEIGHT),
        egui::pos2(SIZE[0] - metric::INSPECTOR_WIDTH, SIZE[1] - timeline),
    );
    sprite_rect(area, app.tab(), 1.0).min + egui::vec2(p.0 as f32 + 0.5, p.1 as f32 + 0.5) * app.tab().zoom
}

fn press(h: &mut Harness<'static, PixmithApp>, pos: egui::Pos2, button: egui::PointerButton, pressed: bool) {
    h.event(egui::Event::PointerButton { pos, button, pressed, modifiers: Default::default() });
    h.run();
}

fn hover(h: &mut Harness<'static, PixmithApp>, pos: egui::Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.run();
}

fn click(h: &mut Harness<'static, PixmithApp>, p: (i32, i32)) {
    let pos = at(h, p);
    hover(h, pos);
    press(h, pos, egui::PointerButton::Primary, true);
    press(h, pos, egui::PointerButton::Primary, false);
}

fn doc<'a>(h: &'a Harness<'static, PixmithApp>) -> &'a Doc {
    &h.state().tab().doc
}

// MARK: The sheet

#[test]
fn the_new_layer_sheet_starts_on_the_view_settings_grid_and_creates_an_isometric_layer() {
    let mut app = PixmithApp::default();
    let s = &mut app.canvas.tools.settings;
    (s.iso_grid_w, s.iso_grid_h, s.iso_grid_x, s.iso_grid_y) = (96, 48, 592, 360);
    app.tabs[0] = DocTab::new(Doc::new(1280, 720));
    let mut h = harness(app);
    assert_eq!(h.state().tilemap.iso_default, Iso::new(96, 48, 592, 360), "follows the view settings");

    tm::new_tilemap_layer_sheet(h.state_mut());
    h.run();
    let f = &h.state().tilemap.form;
    assert!(!f.iso, "square unless asked for");
    assert_eq!((f.tile_w, f.tile_h), (16, 16));

    // Choosing Isometric takes the cell size, origin and a tile image of the cell's size from the grid.
    h.state_mut().tilemap.form.set_iso(true);
    h.run();
    let f = h.state().tilemap.form.clone();
    assert_eq!((f.cell_w, f.cell_h, f.origin), (96, 48, (592, 360)));
    assert_eq!((f.tile_w, f.tile_h), (96, 48));
    // A taller image for cliffs.
    h.state_mut().tilemap.form.tile_h = 96;
    h.run();

    let before = doc(&h).layer_count();
    h.key_press(egui::Key::Enter);
    h.run();
    assert!(h.state().tilemap.sheet.is_none());
    assert_eq!(doc(&h).layer_count(), before + 1);
    let (d, layer) = (doc(&h), h.state().tab().layer);
    assert!(d.layer_is_iso(layer), "the new layer is the active one");
    let ts = d.tileset_of_layer(layer).unwrap();
    assert_eq!(d.tileset_iso_cell(ts), Some((96, 48)));
    assert_eq!(d.tile_size(ts), (96, 96));
    assert_eq!(d.layer_grid_offset(layer), Some((592, 360)));
    assert_eq!(tm::active_iso(h.state().tab()), Some(Iso::new(96, 48, 592, 360)));

    // The next sheet opened on an isometric layer starts on its grid and is isometric.
    tm::new_tilemap_layer_sheet(h.state_mut());
    let f = &h.state().tilemap.form;
    assert!(f.iso);
    assert_eq!((f.cell_w, f.cell_h, f.origin), (96, 48, (592, 360)));
}

#[test]
fn the_convert_sheet_stays_square() {
    let mut app = PixmithApp::default();
    app.tilemap.form.iso = true;
    let mut h = harness(app);
    tm::convert_layer_sheet(h.state_mut());
    assert!(!h.state().tilemap.form.iso, "converting pixels to tiles is square only");
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    let layer = h.state().tab().layer;
    assert!(doc(&h).layer_is_tilemap(layer) && !doc(&h).layer_is_iso(layer));
}

#[test]
fn form_values_are_clamped_to_what_the_core_takes() {
    let mut f = tm::SizeForm { iso: true, cell_w: 31, cell_h: 1, tile_w: 5000, tile_h: 0, ..Default::default() };
    assert_eq!(f.cell(), (32, 4), "cells are even and at least 4");
    assert_eq!(f.tile(), (1024, 1));
    f.iso = false;
    assert_eq!(f.tile(), (256, 1));
    // Tile images that match the diamond follow it, others keep their size.
    let mut f = tm::SizeForm::default();
    f.use_grid(&Iso::new(96, 48, 3, 4));
    f.tile_h = 96;
    f.set_cell(64, 32);
    assert_eq!((f.tile_w, f.tile_h, f.cell_w, f.cell_h, f.origin), (64, 96, 64, 32, (3, 4)));
}

// MARK: Stamping

#[test]
fn stamp_mode_places_erases_and_picks_tiles_in_diamond_cells() {
    let (d, layer, tile) = iso_doc();
    let mut h = harness(app_with(d, layer));
    h.state_mut().tool = Tool::Pencil;
    h.state_mut().tilemap.set_stamp(true);
    h.state_mut().tilemap.selected = tile;

    // A click anywhere in a diamond fills that cell: the corner pixel of cell (2, 1) is not the center.
    let c = center((2, 1));
    click(&mut h, (c.0 + 10, c.1));
    assert_eq!(doc(&h).tile_value_in_cell(layer, 0, 2, 1), tile);
    assert_eq!(doc(&h).tile_value_at(layer, 0, c.0, c.1), tile);
    assert_eq!(doc(&h).tile_value_in_cell(layer, 0, 1, 2), 0, "the mirrored cell is empty");
    assert_eq!(doc(&h).tile_value_in_cell(layer, 0, 0, 0), 0);
    assert_eq!(doc(&h).layer_cell_origin(layer, 2, 1), Some((160 + 16, 24)));
    assert!(doc(&h).is_dirty());

    // One undo removes the stamp.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run();
    assert_eq!(doc(&h).tile_value_in_cell(layer, 0, 2, 1), 0);

    // A flipped tile in the cell to the right (3, 1), then Shift erases it and Option picks.
    h.state_mut().tilemap.flip_x = true;
    let c2 = center((3, 1));
    click(&mut h, c2);
    assert_eq!(doc(&h).tile_value_in_cell(layer, 0, 3, 1), tile | FLIP_X);
    h.state_mut().tilemap.select_value(0);
    h.state_mut().tilemap.selected = 1;
    let pos = at(&h, c2);
    hover(&mut h, pos);
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    for pressed in [true, false] {
        let button = egui::PointerButton::Primary;
        h.event_modifiers(egui::Event::PointerButton { pos, button, pressed, modifiers: alt }, alt);
        h.run();
    }
    assert_eq!(h.state().tilemap.value(), tile | FLIP_X, "Option-click picked the tile with its flip");
    let shift = egui::Modifiers { shift: true, ..Default::default() };
    for pressed in [true, false] {
        let button = egui::PointerButton::Primary;
        h.event_modifiers(egui::Event::PointerButton { pos, button, pressed, modifiers: shift }, shift);
        h.run();
    }
    assert_eq!(doc(&h).tile_value_in_cell(layer, 0, 3, 1), 0, "Shift-click erased it");
}

#[test]
fn stamping_follows_the_layer_offset() {
    let (mut d, layer, tile) = iso_doc();
    // A second layer one elevation level (half a cell) higher.
    assert!(d.set_layer_grid_offset(layer, (160, -8)));
    let mut h = harness(app_with(d, layer));
    h.state_mut().tilemap.set_stamp(true);
    h.state_mut().tilemap.selected = tile;
    let raised = Iso::new(32, 16, 160, -8);
    let (x, y) = raised.cell_origin((1, 1));
    click(&mut h, (x as i32 + 16, y as i32 + 8));
    assert_eq!(doc(&h).tile_value_in_cell(layer, 0, 1, 1), tile);
    assert_eq!(doc(&h).layer_cell_at(layer, x as i32 + 16, y as i32 + 8), Some((1, 1)));
}

// MARK: Terrain

fn terrain_iso_doc() -> (Doc, u32) {
    let (mut d, layer, _) = iso_doc();
    assert!(!d.create_terrain(layer, "Grass", 2), "the dual grid does not fit isometric layers");
    assert!(d.create_terrain(layer, "Grass", 1));
    assert!(!d.set_terrain_mode(layer, 2));
    assert!(d.generate_template(layer, false));
    (d, layer)
}

#[test]
fn the_terrain_brush_paints_diamond_cells() {
    let (d, layer) = terrain_iso_doc();
    let grass = d.terrain_ids(layer)[0];
    let mut h = harness(app_with(d, layer));
    h.state_mut().tilemap.brush = true;
    h.state_mut().tilemap.terrain = grass;

    // Drag across three cells of one row (i grows down-right).
    let (a, b) = (center((2, 2)), center((4, 2)));
    let (pa, pb) = (at(&h, a), at(&h, b));
    hover(&mut h, pa);
    press(&mut h, pa, egui::PointerButton::Primary, true);
    hover(&mut h, pb);
    press(&mut h, pb, egui::PointerButton::Primary, false);
    for cell in [(2, 2), (3, 2), (4, 2)] {
        let c = center(cell);
        assert_eq!(doc(&h).terrain_at(layer, 0, c.0, c.1), grass, "cell {cell:?}");
        assert_ne!(doc(&h).tile_value_in_cell(layer, 0, cell.0 as i32, cell.1 as i32), 0, "an auto-tile was placed");
    }
    // The cell above the stroke stays empty, and any pixel of a painted diamond counts as that cell.
    let c = center((2, 1));
    assert_eq!(doc(&h).terrain_at(layer, 0, c.0, c.1), 0);
    let (ox, oy) = grid().cell_origin((3, 2));
    assert_eq!(doc(&h).terrain_at(layer, 0, ox as i32 + 20, oy as i32 + 8), grass);
    // The whole stroke is one undo step.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run();
    assert_eq!(doc(&h).terrain_at(layer, 0, a.0, a.1), 0);
}

#[test]
fn the_terrain_rect_paints_a_block_of_cells_and_pick_reads_one() {
    let (d, layer) = terrain_iso_doc();
    let grass = d.terrain_ids(layer)[0];
    let mut h = harness(app_with(d, layer));
    h.state_mut().tilemap.brush = true;
    h.state_mut().tilemap.terrain_tool = TerrainTool::Rect;
    h.state_mut().tilemap.terrain = grass;
    let (pa, pb) = (at(&h, center((1, 1))), at(&h, center((3, 2))));
    hover(&mut h, pa);
    press(&mut h, pa, egui::PointerButton::Primary, true);
    hover(&mut h, pb);
    press(&mut h, pb, egui::PointerButton::Primary, false);
    // The rectangle is a rectangle of cells, a diamond on screen: (1..=3) x (1..=2).
    for i in 1..=3 {
        for j in 1..=2 {
            let c = center((i, j));
            assert_eq!(doc(&h).terrain_at(layer, 0, c.0, c.1), grass, "cell {i},{j}");
        }
    }
    let outside = center((4, 1));
    assert_eq!(doc(&h).terrain_at(layer, 0, outside.0, outside.1), 0);
    let outside = center((0, 2));
    assert_eq!(doc(&h).terrain_at(layer, 0, outside.0, outside.1), 0);

    h.state_mut().tilemap.terrain_tool = TerrainTool::Pick;
    h.state_mut().tilemap.terrain = 0;
    click(&mut h, center((2, 2)));
    assert_eq!(h.state().tilemap.terrain, grass);
}

// MARK: Layer offset and tile origin

#[test]
fn the_layer_offset_round_trips_through_undo_and_both_file_formats() {
    let dir = common::scratch("iso-offset");
    let (mut d, layer, _) = iso_doc();
    assert_eq!(d.layer_grid_offset(layer), Some((160, 0)));
    assert!(d.set_layer_grid_offset(layer, (592, 360)));
    assert_eq!(d.layer_grid_offset(layer), Some((592, 360)));
    assert!(d.undo(), "one undo step");
    assert_eq!(d.layer_grid_offset(layer), Some((160, 0)));
    assert!(d.redo());
    assert_eq!(d.layer_cell_origin(layer, 1, 0), Some((592 + 16, 360 + 8)));
    // Square layers have no offset.
    let sq = d.new_tilemap_layer(layer, 16, 16);
    assert_eq!(d.layer_grid_offset(sq), None);
    assert!(!d.set_layer_grid_offset(sq, (1, 1)));
    assert!(!d.layer_is_iso(sq));
    for name in ["offset.pxmt", "offset.aseprite"] {
        let path = dir.join(name);
        d.save_marked(&path).unwrap();
        let back = Doc::open(&path).unwrap();
        assert!(back.layer_is_iso(layer), "{name}");
        assert_eq!(back.layer_grid_offset(layer), Some((592, 360)), "{name}");
        let ts = back.tileset_of_layer(layer).unwrap();
        assert_eq!(back.tileset_iso_cell(ts), Some((32, 16)), "{name}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn tile_origins_can_be_set_reset_and_saved() {
    let dir = common::scratch("iso-origin");
    let mut d = Doc::new(128, 128);
    let layer = d.new_iso_tilemap_layer(0, (32, 40), (32, 16), (0, 0));
    let ts = d.tileset_of_layer(layer).unwrap();
    let tile = d.add_tile(ts).unwrap();
    // A tall tile is bottom-aligned by default: the diamond is the bottom 16 rows (pivot y = 40 - 8).
    assert_eq!(d.tile_pivot(ts, tile), Some((16, 32)));
    assert_eq!(d.tile_texture_origin(ts, tile), Some((0, 12)));
    assert!(!d.tile_pivot_is_set(ts, tile));
    // The origin is the pivot minus the image center.
    assert!(d.set_tile_pivot(ts, tile, (20, 30)));
    assert!(d.tile_pivot_is_set(ts, tile));
    assert_eq!(d.tile_texture_origin(ts, tile), Some((4, 10)));
    let path = dir.join("origin.pxmt");
    d.save_marked(&path).unwrap();
    let back = Doc::open(&path).unwrap();
    let ts2 = back.tileset_of_layer(layer).unwrap();
    assert_eq!(back.tile_pivot(ts2, tile), Some((20, 30)));
    assert!(d.clear_tile_pivot(ts, tile));
    assert_eq!(d.tile_pivot(ts, tile), Some((16, 32)));
    assert!(d.undo());
    assert_eq!(d.tile_pivot(ts, tile), Some((20, 30)));
    // Tiles of square tilesets have no origin.
    let sq = d.new_tilemap_layer(0, 16, 16);
    let sts = d.tileset_of_layer(sq).unwrap();
    assert_eq!(d.tile_pivot(sts, 1), None);
    let _ = std::fs::remove_dir_all(dir);
}

// MARK: Overlay and selection follow the layer

#[test]
fn the_iso_grid_and_select_iso_tile_follow_the_active_iso_layer() {
    use pixmith_desktop::ui::selection;
    let (d, layer, _) = iso_doc();
    let mut app = app_with(d, layer);
    // The view settings say something else entirely.
    let s = &mut app.canvas.tools.settings;
    (s.iso_grid_w, s.iso_grid_h, s.iso_grid_x, s.iso_grid_y) = (96, 48, 592, 360);
    assert_eq!(tm::active_iso(app.tab()), Some(Iso::new(32, 16, 160, 0)));

    // Select > Isometric Tile at a pixel selects the layer's diamond (256 pixels for 32 x 16).
    let c = center((2, 2));
    app.canvas.cursor = Some(c);
    selection::select_iso_tile(&mut app);
    let d = &app.tab().doc;
    assert!(d.sel_contains(c.0, c.1));
    let n = (0..240).flat_map(|y| (0..320).map(move |x| (x, y))).filter(|&(x, y)| d.sel_contains(x, y)).count();
    assert_eq!(n, 32 * 16 / 2);

    // On a layer that is not an isometric tilemap the view settings apply again.
    app.tabs[0].layer = 0;
    assert_eq!(tm::active_iso(app.tab()), None);
    app.tabs[0].doc.sel_clear();
    app.canvas.cursor = Some((640, 384));
    selection::select_iso_tile(&mut app);
    let d = &app.tab().doc;
    assert!(!d.sel_contains(c.0, c.1));
}

// MARK: Godot

#[test]
fn the_godot_tileset_sheet_shows_the_isometric_shape_and_exports_it() {
    use pixmith_desktop::ui::gamedev::TilesetSettings;
    let dir = common::scratch("iso-godot");
    let project = dir.join("game");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("project.godot"), "config_version=5\n").unwrap();
    let (d, layer, _) = iso_doc();
    let mut app = app_with(d, layer);
    pixmith_desktop::ui::gamedev::tileset::open(&mut app);
    let sheet = app.gamedev.tileset.as_ref().expect("sheet");
    assert_eq!(sheet.iso, vec![Some((32, 16))]);
    let mut h = harness(app);
    h.run();
    h.get_by_label_contains("Isometric (diamond 32\u{d7}16)");
    h.get_by_label("Diamond Down");

    let s = TilesetSettings {
        project_root: project.to_string_lossy().into_owned(),
        output_path: project.join("iso.tres").to_string_lossy().into_owned(),
        ..Default::default()
    };
    doc(&h).tileset_export(&s).unwrap();
    let text = std::fs::read_to_string(project.join("iso.tres")).unwrap();
    assert!(text.contains("tile_shape = 1\ntile_layout = 5\ntile_size = Vector2i(32, 16)"), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}
