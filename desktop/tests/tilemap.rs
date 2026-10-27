//! Tilemaps, auto-tiling and slices: the `Doc` calls the panels use, and the canvas tools driven through the UI.

use pixmith_desktop::doc::Doc;

const RED: u32 = 0xFF0000FF;

/// A 64 x 64 sprite whose only layer is a 16 x 16 tilemap with a blob terrain and its reduced template.
fn terrain_doc() -> (Doc, u32) {
    let mut doc = Doc::new(64, 64);
    let layer = doc.new_tilemap_layer(0, 16, 16);
    assert!(doc.layer_is_tilemap(layer));
    assert_eq!(doc.terrain_mode(layer), -1, "no terrain data yet");
    assert!(doc.create_terrain(layer, "Grass", 1));
    assert_eq!(doc.terrain_mode(layer), 1);
    assert!(doc.generate_template(layer, true));
    (doc, layer)
}

#[test]
fn convert_to_tilemap_and_back() {
    let mut doc = Doc::new(32, 32);
    doc.begin_edit("Draw");
    doc.draw_shape(pixmith_desktop::ui::canvas::ShapeKind::Rect, 0, 0, (0, 0), (15, 15), RED, 1, 8, false);
    doc.stroke_end();
    assert!(!doc.layer_is_tilemap(0));
    assert!(doc.convert_layer_to_tilemap(0, 16, 16, true));
    let ts = doc.tileset_of_layer(0).expect("a tileset");
    assert_eq!(doc.tile_size(ts), (16, 16));
    assert!(doc.tile_count(ts) >= 2, "the red tile plus the empty tile 0");
    assert!(!doc.convert_layer_to_tilemap(0, 16, 16, true), "only normal layers convert");
    assert!(doc.convert_layer_to_normal(0));
    assert_eq!(doc.tileset_of_layer(0), None);
    assert!(doc.undo(), "converting back is one undo step");
    assert!(doc.layer_is_tilemap(0));
}

#[test]
fn tiles_can_be_added_duplicated_deleted_and_renamed() {
    let mut doc = Doc::new(32, 32);
    let layer = doc.new_tilemap_layer(0, 8, 8);
    let ts = doc.tileset_of_layer(layer).unwrap();
    let n = doc.tile_count(ts);
    let added = doc.add_tile(ts).expect("tile added");
    assert_eq!(doc.tile_count(ts), n + 1);
    let copy = doc.duplicate_tile(ts, added).expect("duplicate");
    assert_eq!(doc.tile_count(ts), n + 2);
    assert_eq!(doc.tile_rgba(ts, copy).map(|p| p.len()), Some(8 * 8 * 4));
    assert!(doc.delete_tile(ts, copy));
    assert_eq!(doc.tile_count(ts), n + 1);
    assert!(doc.rename_tileset(ts, "Ground"));
    assert_eq!(doc.tileset_title(ts), "Ground");
    assert!(doc.compatible_tilesets(layer).is_empty());
}

#[test]
fn stamping_places_flips_and_erases_a_tile() {
    let mut doc = Doc::new(32, 32);
    let layer = doc.new_tilemap_layer(0, 16, 16);
    let ts = doc.tileset_of_layer(layer).unwrap();
    let tile = doc.add_tile(ts).unwrap();
    doc.begin_edit("Stamp Tile");
    assert!(doc.stamp_tile(layer, 0, 17, 3, tile | pixmith_desktop::ui::tilemap::FLIP_X));
    assert_eq!(doc.tile_value_at(layer, 0, 20, 5), tile | pixmith_desktop::ui::tilemap::FLIP_X, "same cell");
    assert_eq!(doc.tile_value_at(layer, 0, 3, 3), 0, "another cell stays empty");
    assert!(doc.stamp_tile(layer, 0, 17, 3, 0));
    assert_eq!(doc.tile_value_at(layer, 0, 17, 3), 0);
    assert_eq!(doc.tile_grid_origin(layer, 0), (0, 0));
}

#[test]
fn terrain_list_edits() {
    let (mut doc, layer) = terrain_doc();
    let grass = doc.terrain_ids(layer)[0];
    assert_eq!(doc.terrain_name(layer, grass), "Grass");
    let water = doc.add_terrain(layer).expect("second terrain");
    assert_eq!(doc.terrain_name(layer, water), "Terrain 2");
    assert_eq!(doc.terrain_ids(layer), vec![grass, water]);
    assert!(doc.move_terrain(layer, water, 0));
    assert_eq!(doc.terrain_ids(layer), vec![water, grass], "priority order follows the list");
    assert!(doc.rename_terrain(layer, water, "Water"));
    assert_eq!(doc.terrain_name(layer, water), "Water");
    assert!(doc.remove_terrain(layer, water));
    assert_eq!(doc.terrain_ids(layer), vec![grass]);
    assert!(doc.set_terrain_mode(layer, 2));
    assert_eq!(doc.terrain_mode(layer), 2);
    assert!(doc.set_terrain_edge_same(layer, true));
    assert!(doc.terrain_edge_same(layer));
    assert!(doc.set_terrain_seed(layer, 7));
    assert_eq!(doc.terrain_seed(layer), 7);
}

#[test]
fn terrain_brush_paints_and_one_undo_removes_the_stroke() {
    let (mut doc, layer) = terrain_doc();
    let grass = doc.terrain_ids(layer)[0];
    assert_eq!(doc.terrain_at(layer, 0, 20, 20), 0);
    doc.begin_edit("Terrain Brush");
    assert!(doc.paint_terrain_brush(layer, 0, 20, 20, grass, 1, false, 0));
    assert!(doc.paint_terrain_brush(layer, 0, 36, 20, grass, 1, false, 0));
    assert_eq!(doc.terrain_at(layer, 0, 20, 20), grass);
    assert_eq!(doc.terrain_at(layer, 0, 36, 20), grass);
    assert!(doc.undo());
    assert_eq!(doc.terrain_at(layer, 0, 20, 20), 0, "the whole stroke is one step");
}

#[test]
fn terrain_rect_flood_and_mirror() {
    let (mut doc, layer) = terrain_doc();
    let grass = doc.terrain_ids(layer)[0];
    doc.begin_edit("Terrain Rectangle");
    assert!(doc.paint_terrain_rect(layer, 0, (0, 0), (15, 15), grass, 0));
    assert_eq!(doc.terrain_at(layer, 0, 5, 5), grass);
    doc.begin_edit("Terrain Fill");
    assert!(doc.flood_terrain(layer, 0, 40, 40, grass, 0));
    assert_eq!(doc.terrain_at(layer, 0, 63, 63), grass);
}

#[test]
fn terrain_mirror_paints_both_halves() {
    let (mut doc, layer) = terrain_doc();
    let grass = doc.terrain_ids(layer)[0];
    // Mirror bit 0 paints the horizontally mirrored cell as well.
    doc.begin_edit("Terrain Brush");
    assert!(doc.paint_terrain_brush(layer, 0, 8, 40, grass, 1, false, 1));
    assert_eq!(doc.terrain_at(layer, 0, 8, 40), grass);
    assert_eq!(doc.terrain_at(layer, 0, 63 - 8, 40), grass);
    assert_eq!(doc.terrain_at(layer, 0, 8, 63 - 40), 0, "no vertical mirror");
}

#[test]
fn tile_masks_peers_weights_and_templates() {
    let (mut doc, layer) = terrain_doc();
    let ts = doc.tileset_of_layer(layer).unwrap();
    assert_eq!(doc.tile_count(ts), 15, "the reduced blob template is 14 tiles plus the empty tile");
    assert_eq!(doc.tile_mask(layer, 0), -1);
    let mask = doc.tile_mask(layer, 6);
    assert_eq!(mask, 21);
    assert!(doc.set_tile_weight(layer, 6, 2.5));
    assert_eq!(doc.tile_weight(layer, 6), 2.5);
    assert!(doc.set_tile_sym(layer, 6, 3));
    assert_eq!(doc.tile_sym(layer, 6), 3);
    let grass = doc.terrain_ids(layer)[0];
    assert!(doc.set_tile_peer(layer, 6, 0, grass));
    assert_eq!(doc.tile_peer(layer, 6, 0), grass as i32);
    assert!(doc.set_tile_terrain(layer, 6, grass));
    assert_eq!(doc.tile_terrain(layer, 6), grass as i32);
    assert!(doc.set_tile_mask(layer, 6, -1));
    assert_eq!(doc.tile_mask(layer, 6), -1, "Clear data removes the tile's mask");
    assert!(doc.assign_template_masks(layer));
}

#[test]
fn missing_tiles_are_reported_and_marked() {
    let (mut doc, layer) = terrain_doc();
    let grass = doc.terrain_ids(layer)[0];
    doc.begin_edit("Terrain Rectangle");
    doc.paint_terrain_rect(layer, 0, (16, 16), (47, 47), grass, 0);
    assert!(doc.missing_cells(layer, 0).is_empty(), "the template covers every shape");
    // Take away the data of the tile every middle cell uses.
    let used = doc.tile_value_at(layer, 0, 24, 24) & 0x1FFF_FFFF;
    assert!(used > 0);
    assert!(doc.set_tile_mask(layer, used, -1));
    let cells = doc.missing_cells(layer, 0);
    assert!(!cells.is_empty());
    assert!(cells.iter().all(|(x, y)| x % 16 == 0 && y % 16 == 0), "cells are reported by their top-left pixel");
    assert!(!doc.missing_report(layer, 0).is_empty());
    assert!(doc.rebuild_terrain(layer));
    assert!(doc.clear_overrides(layer, 0));
}

#[test]
fn manual_tiles_override_and_clear() {
    let (mut doc, layer) = terrain_doc();
    let ts = doc.tileset_of_layer(layer).unwrap();
    let tile = doc.add_tile(ts).unwrap();
    doc.begin_edit("Manual Tile");
    assert!(doc.set_tile_override(layer, 0, 5, 5, tile));
    assert_eq!(doc.tile_value_at(layer, 0, 5, 5) & 0x1FFF_FFFF, tile);
    doc.begin_edit("Auto Tile");
    assert!(doc.set_tile_override(layer, 0, 5, 5, 0));
    assert_ne!(doc.tile_value_at(layer, 0, 5, 5) & 0x1FFF_FFFF, tile, "the automatic result returns");
}

#[test]
fn slices_create_move_resize_and_edit() {
    let mut doc = Doc::new(32, 32);
    assert_eq!(doc.slice_count(), 0);
    let i = doc.add_slice(2, 3, 10, 8).expect("slice");
    assert_eq!(doc.slice_name(i), "Slice 1");
    let info = doc.slice_info(i, 0).unwrap();
    assert_eq!((info.x, info.y, info.w, info.h), (2, 3, 10, 8));
    assert_eq!(info.center, None);

    doc.begin_edit("Move Slice");
    assert!(doc.set_slice_bounds(i, 0, 5, 6, 10, 8));
    assert_eq!(doc.slice_info(i, 0).map(|s| (s.x, s.y)), Some((5, 6)));
    assert!(doc.undo(), "a drag is one undo step");
    assert_eq!(doc.slice_info(i, 0).map(|s| (s.x, s.y)), Some((2, 3)));

    doc.in_transaction(|d| {
        assert!(d.set_slice_props(i, 0, "button", Some([1, 1, 6, 4]), Some([3, 2])));
        assert!(d.set_user_data(4, i, 0, "hello", Some(0x112233FF)));
        d.label_last_step("Slice Properties");
    });
    let info = doc.slice_info(i, 0).unwrap();
    assert_eq!(doc.slice_name(i), "button");
    assert_eq!(info.center, Some((1, 1, 6, 4)));
    assert_eq!(info.pivot, Some((3, 2)));
    assert_eq!(doc.user_data_text(4, i, 0), "hello");
    assert_eq!(doc.user_data_color(4, i, 0), Some(0x112233FF));

    assert!(doc.remove_slice(i));
    assert_eq!(doc.slice_count(), 0);
    assert!(doc.undo());
    assert_eq!(doc.slice_count(), 1);
}

#[test]
fn user_data_of_sprite_layer_and_cel() {
    let mut doc = Doc::new(8, 8);
    for (kind, a, b) in [(0, 0, 0), (1, 0, 0), (2, 0, 0)] {
        assert!(doc.set_user_data(kind, a, b, &format!("note {kind}"), None));
        assert_eq!(doc.user_data_text(kind, a, b), format!("note {kind}"));
        assert_eq!(doc.user_data_color(kind, a, b), None);
    }
}

/// Drives the real UI headlessly.
mod ui {
    use super::*;
    use eframe::egui;
    use egui_kittest::Harness;
    use pixmith_desktop::PixmithApp;
    use pixmith_desktop::app::{DocTab, Tool};
    use pixmith_desktop::theme::{self, metric};
    use pixmith_desktop::ui::canvas::sprite_rect;
    use pixmith_desktop::ui::tilemap::TerrainTool;

    const SIZE: [f32; 2] = [1280.0, 800.0];

    fn harness(doc: Doc, layer: u32) -> Harness<'static, PixmithApp> {
        let mut app = PixmithApp::default();
        app.tabs[0] = DocTab::new(doc);
        app.tabs[0].layer = layer;
        app.tabs[0].zoom = 8.0;
        let tab = &app.tabs[0];
        app.canvas.keep_view(tab);
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

    fn at(h: &Harness<PixmithApp>, x: i32, y: i32) -> egui::Pos2 {
        let app = h.state();
        let timeline = pixmith_desktop::ui::timeline::panel_height(app);
        let area = egui::Rect::from_min_max(
            egui::pos2(metric::TOOL_RAIL, 2.0 * metric::BAR_HEIGHT),
            egui::pos2(SIZE[0] - metric::INSPECTOR_WIDTH, SIZE[1] - timeline),
        );
        sprite_rect(area, app.tab(), 1.0).min + egui::vec2(x as f32 + 0.5, y as f32 + 0.5) * app.tab().zoom
    }

    fn press(h: &mut Harness<PixmithApp>, pos: egui::Pos2, button: egui::PointerButton, pressed: bool) {
        h.event(egui::Event::PointerButton { pos, button, pressed, modifiers: Default::default() });
        h.run();
    }

    fn hover(h: &mut Harness<PixmithApp>, pos: egui::Pos2) {
        h.event(egui::Event::PointerMoved(pos));
        h.run();
    }

    fn drag(h: &mut Harness<PixmithApp>, from: (i32, i32), to: (i32, i32)) {
        let (a, b) = (at(h, from.0, from.1), at(h, to.0, to.1));
        hover(h, a);
        press(h, a, egui::PointerButton::Primary, true);
        hover(h, b);
        press(h, b, egui::PointerButton::Primary, false);
    }

    fn doc<'a>(h: &'a Harness<PixmithApp>) -> &'a Doc {
        &h.state().tab().doc
    }

    #[test]
    fn stamp_mode_places_tiles_and_leaves_with_a_tool_change() {
        let mut d = Doc::new(64, 64);
        let layer = d.new_tilemap_layer(0, 16, 16);
        let ts = d.tileset_of_layer(layer).unwrap();
        let tile = d.add_tile(ts).unwrap();
        let mut h = harness(d, layer);
        h.state_mut().tool = Tool::Pencil;
        h.state_mut().tilemap.set_stamp(true);
        h.state_mut().tilemap.selected = tile;
        let p = at(&h, 20, 20);
        hover(&mut h, p);
        press(&mut h, p, egui::PointerButton::Primary, true);
        press(&mut h, p, egui::PointerButton::Primary, false);
        assert_eq!(doc(&h).tile_value_at(layer, 0, 20, 20), tile);
        assert!(doc(&h).is_dirty());
        // The tile took the place of drawing: the canvas pixels are the tile's, not the primary color.
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run();
        assert_eq!(doc(&h).tile_value_at(layer, 0, 20, 20), 0, "one undo removes the stamp");
        // Stamping again works after an undo.
        press(&mut h, p, egui::PointerButton::Primary, true);
        press(&mut h, p, egui::PointerButton::Primary, false);
        assert_eq!(doc(&h).tile_value_at(layer, 0, 20, 20), tile);
        // Choosing a tool leaves stamp mode.
        h.key_press(egui::Key::E);
        h.run();
        assert!(!h.state().tilemap.stamp);
    }

    #[test]
    fn terrain_brush_drag_paints_a_connected_stroke() {
        let (d, layer) = terrain_doc();
        let mut h = harness(d, layer);
        h.state_mut().tilemap.brush = true;
        h.state_mut().tilemap.terrain = doc(&h).terrain_ids(layer)[0];
        drag(&mut h, (4, 4), (52, 4));
        let grass = doc(&h).terrain_ids(layer)[0];
        for x in [4, 20, 36, 52] {
            assert_eq!(doc(&h).terrain_at(layer, 0, x, 4), grass, "cell at x={x}");
        }
        assert!(doc(&h).is_dirty());
        // Right button erases.
        let p = at(&h, 20, 4);
        hover(&mut h, p);
        press(&mut h, p, egui::PointerButton::Secondary, true);
        press(&mut h, p, egui::PointerButton::Secondary, false);
        assert_eq!(doc(&h).terrain_at(layer, 0, 20, 4), 0);
        assert_eq!(doc(&h).terrain_at(layer, 0, 4, 4), grass);
    }

    #[test]
    fn terrain_rect_tool_commits_on_release() {
        let (d, layer) = terrain_doc();
        let mut h = harness(d, layer);
        h.state_mut().tilemap.brush = true;
        h.state_mut().tilemap.terrain_tool = TerrainTool::Rect;
        h.state_mut().tilemap.terrain = doc(&h).terrain_ids(layer)[0];
        let (a, b) = (at(&h, 2, 2), at(&h, 40, 20));
        hover(&mut h, a);
        press(&mut h, a, egui::PointerButton::Primary, true);
        hover(&mut h, b);
        assert_eq!(doc(&h).terrain_at(layer, 0, 40, 20), 0, "nothing is painted before the release");
        press(&mut h, b, egui::PointerButton::Primary, false);
        let grass = doc(&h).terrain_ids(layer)[0];
        assert_eq!(doc(&h).terrain_at(layer, 0, 30, 10), grass);
    }

    #[test]
    fn terrain_pick_takes_the_terrain_under_the_cursor() {
        let (mut d, layer) = terrain_doc();
        let grass = d.terrain_ids(layer)[0];
        let water = d.add_terrain(layer).unwrap();
        d.begin_edit("Terrain Brush");
        d.paint_terrain_brush(layer, 0, 20, 20, water, 1, false, 0);
        let mut h = harness(d, layer);
        h.state_mut().tilemap.brush = true;
        h.state_mut().tilemap.terrain_tool = TerrainTool::Pick;
        h.state_mut().tilemap.terrain = grass;
        let p = at(&h, 20, 20);
        hover(&mut h, p);
        press(&mut h, p, egui::PointerButton::Primary, true);
        press(&mut h, p, egui::PointerButton::Primary, false);
        assert_eq!(h.state().tilemap.terrain, water);
    }

    #[test]
    fn slice_tool_creates_moves_and_resizes() {
        let mut h = harness(Doc::new(64, 64), 0);
        h.state_mut().tool = Tool::Slice;
        drag(&mut h, (4, 4), (13, 9));
        assert_eq!(doc(&h).slice_count(), 1);
        let s = doc(&h).slice_info(0, 0).unwrap();
        assert_eq!((s.x, s.y, s.w, s.h), (4, 4, 10, 6));
        assert_eq!(h.state().tilemap.slice_selected, Some(0));

        // Dragging inside the slice moves it.
        drag(&mut h, (8, 6), (18, 16));
        let s = doc(&h).slice_info(0, 0).unwrap();
        assert_eq!((s.x, s.y, s.w, s.h), (14, 14, 10, 6));

        // The bottom-right handle resizes it.
        let (a, b) = (at(&h, 23, 19), at(&h, 29, 24));
        let corner = a + egui::vec2(4.0, 4.0);
        hover(&mut h, corner);
        press(&mut h, corner, egui::PointerButton::Primary, true);
        hover(&mut h, b);
        press(&mut h, b, egui::PointerButton::Primary, false);
        let s = doc(&h).slice_info(0, 0).unwrap();
        assert_eq!((s.x, s.y), (14, 14));
        assert!(s.w > 10 && s.h > 6, "resized: {s:?}");

        // Each gesture is one undo step with its own label; the creation is the first.
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run();
        let s = doc(&h).slice_info(0, 0).unwrap();
        assert_eq!((s.x, s.y, s.w, s.h), (4, 4, 10, 6));
    }

    #[test]
    fn tool_change_leaves_the_brush_and_slice_properties_open_a_sheet() {
        let (d, layer) = terrain_doc();
        let mut h = harness(d, layer);
        h.state_mut().tilemap.brush = true;
        h.key_press(egui::Key::M);
        h.run();
        assert!(!h.state().tilemap.brush, "choosing any tool switches the brush off");

        let mut h = harness(Doc::new(64, 64), 0);
        h.state_mut().tool = Tool::Slice;
        drag(&mut h, (4, 4), (13, 9));
        // Slice Properties… (menu, inspector button, double-click) opens the sheet of the selected slice.
        assert!(pixmith_desktop::ui::tilemap::has_selected_slice(h.state()));
        pixmith_desktop::ui::tilemap::slice_properties(h.state_mut());
        assert_eq!(h.state().tilemap.slice_sheet.as_ref().map(|s| s.name.as_str()), Some("Slice 1"));
        h.run();
    }

    #[test]
    fn menu_sheets_create_and_convert_layers() {
        use pixmith_desktop::ui::tilemap as tm;
        let mut h = harness(Doc::new(32, 32), 0);
        assert!(tm::can_convert_to_tilemap(h.state()));
        assert!(!tm::is_tilemap_layer(h.state()));

        tm::convert_layer_sheet(h.state_mut());
        h.run();
        h.key_press(egui::Key::Enter);
        h.run();
        assert!(tm::is_tilemap_layer(h.state()), "Return confirms the Convert sheet");
        assert!(!tm::can_convert_to_tilemap(h.state()));
        assert!(h.state().tilemap.sheet.is_none());

        let before = doc(&h).layer_count();
        tm::new_tilemap_layer_sheet(h.state_mut());
        h.run();
        h.key_press(egui::Key::Enter);
        h.run();
        assert_eq!(doc(&h).layer_count(), before + 1);
        assert!(tm::is_tilemap_layer(h.state()), "the new layer is selected");

        tm::toggle_stamp(h.state_mut());
        assert!(h.state().tilemap.stamp);
        tm::convert_to_normal(h.state_mut());
        assert!(!tm::is_tilemap_layer(h.state()));
        assert!(!h.state().tilemap.stamp, "stamp mode ends with the tilemap");
    }
}

#[test]
fn auto_tiles_give_a_painted_cell_its_own_copy_and_copies_are_outlined() {
    let mut doc = Doc::new(32, 32);
    let layer = doc.new_tilemap_layer(0, 16, 16);
    let ts = doc.tileset_of_layer(layer).unwrap();
    let tile = doc.add_tile(ts).unwrap();
    doc.begin_edit("Stamp Tile");
    assert!(doc.stamp_tile(layer, 0, 0, 0, tile));
    assert!(doc.stamp_tile(layer, 0, 16, 16, tile));
    assert_eq!(doc.tile_copies_at(layer, 0, 20, 20), vec![[0, 0, 16, 16], [16, 16, 16, 16]]);

    let mut app = pixmith_desktop::PixmithApp::default();
    app.tilemap.set_auto_tiles(true);
    doc.begin_edit("Pencil");
    doc.stroke_begin(layer, 0, 20, 20, RED, 1, 0);
    doc.stroke_end();
    app.tilemap.set_auto_tiles(false);
    assert_ne!(doc.tile_value_at(layer, 0, 20, 20), tile, "the painted cell got its own tile");
    assert_eq!(doc.tile_value_at(layer, 0, 4, 4), tile, "the other cell keeps the original");
    assert_eq!(doc.tile_copies_at(layer, 0, 20, 20).len(), 1);
}
