//! The Pixels / Tiles mode of tilemap layers driven through the UI: getting out of Tile Stamp, the Tab shortcut,
//! drawing over stamped tiles, and the hint after stamps that change nothing.

use eframe::egui;
use egui_kittest::Harness;
use pixmith_desktop::PixmithApp;
use pixmith_desktop::app::{DocTab, Tool};
use pixmith_desktop::doc::Doc;
use pixmith_desktop::theme::{self, metric};
use pixmith_desktop::ui::canvas::sprite_rect;

const RED: u32 = 0xFF0000FF;
const SIZE: [f32; 2] = [1280.0, 800.0];

/// A 64 x 48 sprite whose top layer is a 16 px tilemap with one green tile, selected for the stamp.
fn harness() -> Harness<'static, PixmithApp> {
    let mut doc = Doc::new(64, 48);
    let layer = doc.new_tilemap_layer(0, 16, 16);
    let ts = doc.tileset_of_layer(layer).unwrap();
    let tile = doc.add_tile(ts).unwrap();
    doc.begin_edit("Tile");
    doc.stamp_tile(layer, 0, 0, 0, tile);
    doc.stroke_begin(layer, 0, 2, 2, 0x00AA00FF, 4, 0);
    doc.stroke_end();
    doc.stamp_tile(layer, 0, 0, 0, 0);
    let mut app = PixmithApp { primary: RED, ..Default::default() };
    app.tabs[0] = DocTab::new(doc);
    app.tabs[0].layer = layer;
    app.tabs[0].zoom = 8.0;
    let tab = &app.tabs[0];
    app.canvas.keep_view(tab);
    app.tool = Tool::Pencil;
    app.tilemap.selected = tile;
    app.tilemap.set_auto_tiles(true);
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
    h.run_ok();
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

fn click(h: &mut Harness<PixmithApp>, x: i32, y: i32) {
    let pos = at(h, x, y);
    h.event(egui::Event::PointerMoved(pos));
    h.run_ok();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
        h.run_ok();
    }
}

fn pixel(h: &Harness<PixmithApp>, x: i32, y: i32) -> u32 {
    let doc = &h.state().tab().doc;
    let mut buf = Vec::new();
    doc.render(0, &mut buf);
    let i = (y as usize * doc.width() as usize + x as usize) * 4;
    u32::from_be_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]])
}

fn key(h: &mut Harness<PixmithApp>, k: egui::Key) {
    h.key_press(k);
    h.run_ok();
}

#[test]
fn stamp_then_draw_over_the_tiles() {
    let mut h = harness();
    pixmith_desktop::ui::tilemap::set_tile_mode(h.state_mut(), true);
    for (x, y) in [(5, 5), (21, 5), (37, 5)] {
        click(&mut h, x, y);
    }
    let layer = h.state().tab().layer;
    assert!((0..3).all(|k| h.state().tab().doc.tile_value_at(layer, 0, 5 + 16 * k, 5) != 0), "three tiles stamped");
    // The Pencil's key, with the Pencil already selected, goes back to Pixels.
    key(&mut h, egui::Key::B);
    assert!(!h.state().tilemap.stamp);
    click(&mut h, 12, 12);
    assert_eq!(pixel(&h, 12, 12), RED, "drawing over a stamped tile");
    assert_ne!(pixel(&h, 28, 12), RED, "Auto Tiles: the other copies keep their pixels");
}

#[test]
fn tab_switches_pixels_and_tiles_on_tilemap_layers_only() {
    let mut h = harness();
    key(&mut h, egui::Key::Tab);
    assert!(h.state().tilemap.stamp);
    key(&mut h, egui::Key::Tab);
    assert!(!h.state().tilemap.stamp);
    h.state_mut().tabs[0].layer = 0;
    key(&mut h, egui::Key::Tab);
    assert!(!h.state().tilemap.stamp, "a normal layer has no tile mode");
}

#[test]
fn tiles_picks_the_pencil_and_a_color_goes_back_to_pixels() {
    let mut h = harness();
    h.state_mut().tool = Tool::Fill;
    h.run_ok();
    pixmith_desktop::ui::tilemap::set_tile_mode(h.state_mut(), true);
    h.run_ok();
    assert_eq!(h.state().tool, Tool::Pencil);
    assert!(h.state().tilemap.stamp, "the tool change does not undo the switch");
    h.state_mut().primary = 0x2244FFFF;
    h.run_ok();
    assert!(!h.state().tilemap.stamp, "choosing a color means drawing");
}

#[test]
fn stamps_that_change_nothing_show_the_hint() {
    let mut h = harness();
    pixmith_desktop::ui::tilemap::set_tile_mode(h.state_mut(), true);
    click(&mut h, 5, 5);
    assert!(h.state().tilemap.hint_until.is_none(), "the first stamp changed the cell");
    click(&mut h, 6, 6);
    click(&mut h, 7, 7);
    assert!(h.state().tilemap.hint_until.is_some(), "two stamps in a row that changed nothing");
}

#[test]
fn picking_a_tile_turns_tiles_on_with_the_pencil() {
    let mut h = harness();
    h.state_mut().tool = Tool::Fill;
    h.run_ok();
    h.state_mut().tilemap.pick_tile(1);
    h.run_ok();
    assert!(h.state().tilemap.stamp);
    assert_eq!(h.state().tool, Tool::Pencil);
}
