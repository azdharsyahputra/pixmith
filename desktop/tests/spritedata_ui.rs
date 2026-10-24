//! Color profile, cel z-index and reference layers in the egui app: the core calls (`Doc`), the Sprite Properties,
//! Cel Properties and Layer Properties sheets, the Layer and View menus, and the timeline row. The screenshot
//! tests need a GPU and are ignored by default:
//!
//! ```sh
//! cargo test --test spritedata_ui -- --ignored   # writes target/screens/spritedata-<name>-<theme>.png
//! ```

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pixmith_desktop::doc::Doc;
use pixmith_desktop::{PixmithApp, theme};
use std::path::{Path, PathBuf};

const RED: u32 = 0xFF0000FF;
const GREEN: u32 = 0x00FF00FF;
const BLUE: u32 = 0x0000FFFF;

fn harness_with(app: PixmithApp, size: [f32; 2], ppp: Option<f32>) -> Harness<'static, PixmithApp> {
    let mut installed = false;
    let mut builder = Harness::builder().with_size(size);
    if let Some(p) = ppp {
        builder = builder.with_pixels_per_point(p);
    }
    let mut h = builder.build_ui_state(
        move |ui, app: &mut PixmithApp| {
            if !installed {
                theme::install(ui.ctx());
                theme::apply(ui.ctx(), app.dark);
                installed = true;
                ui.ctx().request_discard("fonts installed");
                return;
            }
            let full = ui.ctx().content_rect();
            let mut root = ui.new_child(egui::UiBuilder::new().max_rect(full));
            root.set_clip_rect(full);
            app.ui(&mut root);
        },
        app,
    );
    h.run();
    h
}

fn harness(setup: impl FnOnce(&mut PixmithApp)) -> Harness<'static, PixmithApp> {
    let mut app = PixmithApp::default();
    setup(&mut app);
    harness_with(app, [1280.0, 800.0], None)
}

fn pixel(doc: &Doc, x: i32, y: i32) -> u32 {
    let mut buf = Vec::new();
    assert!(doc.render(0, &mut buf));
    Doc::pixel_of(&buf, doc.width(), x, y).expect("inside the sprite")
}

fn put(doc: &mut Doc, layer: u32, x: i32, y: i32, color: u32) {
    doc.begin_edit("Pencil");
    doc.stroke_begin(layer, 0, x, y, color, 1, 0);
    doc.stroke_end();
    doc.touch();
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pixmith-spritedata-ui-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes a solid PNG with the `image` crate.
fn write_png(path: &Path, w: u32, h: u32, rgba: [u8; 4]) {
    image::RgbaImage::from_pixel(w, h, image::Rgba(rgba)).save(path).unwrap();
}

/// A tab whose first layer is red and second layer green at (0, 0).
fn two_layers(app: &mut PixmithApp) {
    let tab = app.tab_mut();
    put(&mut tab.doc, 0, 0, 0, RED);
    let top = tab.doc.add_layer(0);
    put(&mut tab.doc, top, 0, 0, GREEN);
    tab.layer = 0;
    tab.changed();
}

fn open_file(name: &str) -> Doc {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/fixtures/aseprite").join(name);
    Doc::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn doc_calls_show_set_and_undo_the_color_profile() {
    let mut doc = open_file("ase-go/prop.ase");
    assert_eq!(doc.color_profile_kind(), 2);
    assert_eq!(doc.color_profile_label(), "Embedded ICC (Display P3)");
    assert!(doc.set_color_profile(1));
    assert_eq!(doc.color_profile_label(), "sRGB");
    assert!(!doc.set_color_profile(1));
    assert!(doc.set_color_profile(0));
    assert_eq!(doc.color_profile_label(), "None");
    assert!(doc.undo() && doc.undo());
    assert_eq!(doc.color_profile_kind(), 2);
}

#[test]
fn doc_calls_set_the_z_index_and_reorder_cels() {
    let mut app = PixmithApp::default();
    two_layers(&mut app);
    let doc = &mut app.tab_mut().doc;
    assert_eq!(pixel(doc, 0, 0), GREEN);
    assert_eq!(doc.cel_z_index(0, 0), Some(0));
    assert!(doc.set_cel_z_index(0, 0, 2));
    assert_eq!(pixel(doc, 0, 0), RED);
    assert!(doc.undo());
    assert_eq!(pixel(doc, 0, 0), GREEN);
    assert_eq!(doc.cel_z_index(5, 0), None);
}

#[test]
fn a_reference_layer_from_a_file_is_scaled_locked_and_not_exported() {
    let dir = temp_dir("newref");
    let png = dir.join("wide.png");
    write_png(&png, 32, 8, [0, 0, 255, 255]);
    let mut doc = Doc::new(16, 16);
    let layer = doc.new_reference_layer(0, &png).unwrap();
    assert_eq!(layer, 1);
    assert!(doc.layer_is_reference(1) && doc.layer_is_locked(1));
    assert_eq!(doc.layer_name(1), "wide");
    // 32x8 fits the 16x16 canvas as 16x4, centered: rows 6..10.
    assert_eq!(pixel(&doc, 0, 6), BLUE);
    assert_eq!(pixel(&doc, 15, 9), BLUE);
    assert_eq!(pixel(&doc, 0, 5) & 0xFF, 0);
    // Dimmed, hidden, and the exported PNG never shows it.
    doc.set_reference_display(50);
    assert_eq!(pixel(&doc, 0, 6) & 0xFF, 127);
    doc.set_reference_display(0);
    assert_eq!(pixel(&doc, 0, 6) & 0xFF, 0);
    doc.set_reference_display(100);
    let out = dir.join("out.png");
    doc.export_png(0, 1, &out).unwrap();
    let exported = image::open(&out).unwrap().into_rgba8();
    assert!(exported.pixels().all(|p| p.0[3] == 0), "the reference image is not exported");
    // Garbage is refused and changes nothing.
    let bad = dir.join("bad.png");
    std::fs::write(&bad, b"not an image").unwrap();
    assert!(doc.new_reference_layer(0, &bad).is_err());
    assert_eq!(doc.layer_count(), 2);
}

#[test]
fn the_sprite_properties_sheet_assigns_and_removes_the_profile() {
    let mut h = harness(|app| {
        app.tabs[0].doc = open_file("ase-go/prop.ase");
    });
    h.get_by_label("Sprite").click();
    h.run();
    h.get_by_label_contains("Sprite Properties").click();
    h.run();
    assert!(h.state().menus.sprite_props);
    assert!(h.query_by_label("Embedded ICC (Display P3)").is_some());
    h.get_by_label("Assign sRGB").click();
    h.run();
    assert_eq!(h.state().tab().doc.color_profile_label(), "sRGB");
    assert!(h.query_by_label("sRGB").is_some());
    h.get_by_label("Remove Profile").click();
    h.run();
    assert_eq!(h.state().tab().doc.color_profile_label(), "None");
}

#[test]
fn the_cel_sheet_applies_a_z_index() {
    let mut h = harness(two_layers);
    h.get_by_label("Frame").click();
    h.run();
    h.get_by_label_contains("Cel Properties").click();
    h.run();
    assert!(h.state().menus.cel.is_some());
    h.state_mut().menus.cel.as_mut().unwrap().z_index = 3;
    h.run();
    h.get_by_label("Apply").click();
    h.run();
    assert_eq!(h.state().tab().doc.cel_z_index(0, 0), Some(3));
    assert_eq!(pixel(&h.state().tab().doc, 0, 0), RED);
}

#[test]
fn the_layer_menu_turns_a_layer_into_a_reference_layer() {
    let mut h = harness(two_layers);
    assert!(!h.state().tab().doc.layer_is_reference(0));
    h.get_by_label("Layer").click();
    h.run();
    // "New Reference Layer from File…" comes first; the toggle is the last match.
    let entry = h.get_all_by_label_contains("Reference Layer").last().expect("the Reference Layer entry");
    entry.click();
    h.run();
    assert!(h.state().tab().doc.layer_is_reference(0));
    assert!(h.state().tab().doc.layer_is_locked(0));
    // Not part of the normal composite any more... but the editor draws it (red under green here).
    assert_eq!(pixel(&h.state().tab().doc, 0, 0), GREEN);
    // Undo restores it.
    h.get_by_label("Edit").click();
    h.run();
    h.get_by_label_contains("Undo").click();
    h.run();
    assert!(!h.state().tab().doc.layer_is_reference(0));
}

#[test]
fn the_view_menu_sets_the_reference_display_without_dirtying_the_document() {
    let mut h = harness(|app| {
        let dir = temp_dir("viewmenu");
        let png = dir.join("r.png");
        write_png(&png, 16, 16, [0, 0, 255, 255]);
        let tab = app.tab_mut();
        tab.doc.new_reference_layer(0, &png).unwrap();
        tab.changed();
    });
    let dirty = h.state().tab().doc.is_dirty();
    h.get_by_label("View").click();
    h.run();
    h.get_by_label_contains("Reference Layers").click();
    h.run();
    h.get_by_label_contains("Semi-transparent").click();
    h.run();
    assert_eq!(h.state().tab().doc.reference_display(), 50);
    assert_eq!(h.state().tab().doc.is_dirty(), dirty, "a view setting is not an edit");
    assert_eq!(pixel(&h.state().tab().doc, 3, 3) & 0xFF, 127);
}

#[test]
fn the_layer_sheet_has_reference_and_lock_options() {
    let mut h = harness(|app| {
        let PixmithApp { tabs, active, timeline, .. } = app;
        timeline.open_layer_sheet(&tabs[*active], 0);
    });
    h.state_mut().timeline.layer_sheet_mut().unwrap().reference = true;
    h.state_mut().timeline.layer_sheet_mut().unwrap().locked = true;
    h.run();
    {
        let PixmithApp { tabs, active, timeline, .. } = h.state_mut();
        timeline.apply_layer_sheet(&mut tabs[*active]);
    }
    h.run();
    assert!(h.state().tab().doc.layer_is_reference(0));
    assert!(h.state().tab().doc.layer_is_locked(0));
}

// MARK: Screenshots

fn render(name: &str, mut setup: impl FnMut(&mut PixmithApp)) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/screens");
    std::fs::create_dir_all(&dir).unwrap();
    for dark in [true, false] {
        let mut app = PixmithApp { dark, ..Default::default() };
        setup(&mut app);
        let mut h = harness_with(app, [1280.0, 800.0], Some(2.0));
        let image = h.render().expect("render");
        let path = dir.join(format!("spritedata-{name}-{}.png", if dark { "dark" } else { "light" }));
        image.save(&path).unwrap();
        println!("wrote {}", path.display());
    }
}

#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn screenshot_sprite_properties() {
    render("sprite-properties", |app| {
        app.tabs[0].doc = open_file("ase-go/prop.ase");
        app.menus.sprite_props = true;
    });
}

#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn screenshot_cel_properties() {
    render("cel-properties", |app| {
        two_layers(app);
        let PixmithApp { menus, tabs, active, .. } = app;
        menus.open_cel_sheet(&tabs[*active]);
    });
}

#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn screenshot_layer_properties() {
    render("layer-properties", |app| {
        let dir = temp_dir("shot-layer");
        let png = dir.join("sketch.png");
        write_png(&png, 32, 32, [90, 160, 255, 255]);
        let tab = app.tab_mut();
        let l = tab.doc.new_reference_layer(0, &png).unwrap();
        tab.layer = l;
        tab.changed();
        let PixmithApp { tabs, active, timeline, .. } = app;
        timeline.open_layer_sheet(&tabs[*active], l);
    });
}

#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn screenshot_timeline_with_a_reference_layer() {
    render("timeline", |app| {
        let dir = temp_dir("shot-timeline");
        let png = dir.join("sketch.png");
        let mut img = image::RgbaImage::new(48, 48);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([(x * 5) as u8, (y * 5) as u8, 180, 255]);
        }
        img.save(&png).unwrap();
        let tab = app.tab_mut();
        put(&mut tab.doc, 0, 10, 10, RED);
        let l = tab.doc.new_reference_layer(0, &png).unwrap();
        tab.doc.set_reference_display(50);
        tab.layer = l;
        tab.changed();
    });
}
