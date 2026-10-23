//! Renders the window headlessly to PNGs for side-by-side comparison with the macOS app's screenshots in
//! `docs/design/screens/`. Needs a GPU (wgpu), so it is ignored by default:
//!
//! ```sh
//! cargo test --test screens -- --ignored        # writes target/screens/<name>-<theme>.png
//! ```

use eframe::egui;
use egui_kittest::Harness;
use pixmith_desktop::{PixmithApp, theme};

/// Renders `app` at the macOS screenshot size (1280 x 800 points, 2x) in both themes.
pub fn render(name: &str, mut setup: impl FnMut(&mut PixmithApp)) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/screens");
    std::fs::create_dir_all(&dir).unwrap();
    for dark in [true, false] {
        let mut app = PixmithApp { dark, ..Default::default() };
        setup(&mut app);
        let mut installed = false;
        let mut harness = Harness::builder().with_size([1280.0, 800.0]).with_pixels_per_point(2.0).build_ui_state(
            |ui, app: &mut PixmithApp| {
                if !installed {
                    // New fonts take effect on the next pass; drawing text in the "strong" family before
                    // that panics, so the first pass draws nothing.
                    theme::install(ui.ctx());
                    theme::apply(ui.ctx(), app.dark);
                    installed = true;
                    ui.ctx().request_repaint();
                    return;
                }
                // The harness insets its root ui by 8 points; eframe gives the app the whole window.
                let screen = ui.ctx().content_rect();
                let mut window = ui.new_child(egui::UiBuilder::new().max_rect(screen));
                window.set_clip_rect(screen);
                app.ui(&mut window);
            },
            app,
        );
        harness.run();
        let image = harness.render().expect("render");
        let path = dir.join(format!("{name}-{}.png", if dark { "dark" } else { "light" }));
        image.save(&path).unwrap();
        println!("wrote {}", path.display());
    }
}

#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn main_window() {
    render("main", |_| {});
}

/// Title row with several tabs, one with unsaved changes (compare `tabs-*.png`).
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn tabs() {
    render("tabs", |app| {
        use pixmith_desktop::app::DocTab;
        use pixmith_desktop::doc::Doc;
        app.tabs[0].doc.touch();
        app.tabs.push(DocTab::new(Doc::new(48, 48)));
        app.tabs.push(DocTab::new(Doc::new(16, 16)));
        app.active = 1;
    });
}

/// The New Sprite sheet over the main window.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn new_sprite_sheet() {
    render("new-sprite", |app| {
        app.files.new_sheet = Some(pixmith_desktop::ui::dialogs::NewSheet { width: 32, height: 32 });
    });
}

/// The unsaved-changes question.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn unsaved_changes() {
    render("unsaved", |app| {
        app.tabs[0].doc.touch();
        app.request_close(0);
    });
}

/// The sprite sheet export options.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn export_sheet() {
    render("export-sheet", |app| {
        app.request_export(pixmith_desktop::ui::dialogs::ExportKind::Sheet);
    });
}

/// The canvas with a sprite drawn through the same calls the tools use, at 800% like `main-dark@2x.png`.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn canvas() {
    use pixmith_desktop::doc::Doc;
    use pixmith_desktop::ui::canvas::{ShapeKind, flag};
    render("canvas", |app| {
        let mut doc = Doc::new(48, 48);
        let mut shape = |kind, a, b, color, flags| {
            doc.begin_edit("Shape");
            doc.draw_shape(kind, 0, 0, a, b, color, 1, flags, false);
            doc.stroke_end();
        };
        shape(ShapeKind::Rect, (4, 4), (43, 43), 0x2B3566FF, flag::FILLED);
        shape(ShapeKind::Ellipse, (11, 7), (36, 41), 0x1A1A33FF, flag::FILLED);
        shape(ShapeKind::Ellipse, (12, 8), (35, 40), 0xE8A862FF, flag::FILLED);
        shape(ShapeKind::Rect, (17, 21), (20, 25), 0x1A1A33FF, flag::FILLED);
        shape(ShapeKind::Rect, (27, 21), (30, 25), 0x1A1A33FF, flag::FILLED);
        shape(ShapeKind::Rect, (14, 12), (17, 14), 0xFBE3A0FF, flag::FILLED);
        app.tabs[0] = pixmith_desktop::app::DocTab::new(doc);
        app.tabs[0].zoom = 8.0;
        app.primary = 0xF2A65AFF;
        app.tool = pixmith_desktop::app::Tool::Pencil;
        let tab = &app.tabs[0];
        app.canvas.keep_view(tab);
    });
}

/// The inspector with the Swift screenshot's colors and a renamed layer (compare with `main-<theme>@2x.png`).
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn inspector() {
    render("inspector", |app| {
        app.primary = 0xF2A65AFF;
        app.secondary = 0x2B3A67FF;
        let tab = app.tab_mut();
        let layer = tab.layer;
        tab.doc.rename_layer(layer, "Outline");
        tab.doc.palette_add(0xF2A65AFF);
        tab.doc.palette_add(0x2B3A6780);
        tab.changed();
    });
}

/// The timeline as in `docs/design/screens/main-*.png`: Background, Body, Outline, a group with Eyes and
/// Shine, six frames and the overlapping tags idle / blink.
fn timeline_doc(app: &mut PixmithApp) {
    use pixmith_desktop::ui::timeline::TagInfo;
    let tab = app.tab_mut();
    let doc = &mut tab.doc;
    doc.rename_layer(0, "Background");
    for name in ["Body", "Outline", "Eyes", "Shine"] {
        let top = doc.layer_count() - 1;
        let i = doc.add_layer(top);
        doc.rename_layer(i, name);
    }
    let group = doc.wrap_in_group(3).expect("group");
    doc.move_layer_into(5, group).expect("shine into group");
    for _ in 0..5 {
        let last = doc.frame_count() - 1;
        doc.add_frame(last, false);
    }
    for l in 0..doc.layer_count() {
        if doc.layer_is_group(l) {
            continue;
        }
        for f in 0..doc.frame_count() {
            // SAFETY: the document is live and the arguments are in range.
            unsafe { pixmith_core::ffi::document::pixmith_draw_line(doc.raw(), l, f, 1, 1, 2, 1, 0xFFFFFFFF, 1) };
        }
    }
    doc.add_tag(&TagInfo { name: "idle".into(), from: 0, to: 2, direction: 0, repeat: 0, color: 0x3B82F6 });
    doc.add_tag(&TagInfo { name: "blink".into(), from: 2, to: 5, direction: 0, repeat: 0, color: 0xEF4444 });
    tab.layer = 2;
    tab.frame = 1;
    tab.changed();
}

/// Like `render`, but waits for the fonts to be installed before the first real pass (the "strong" family
/// the timeline uses does not exist until `theme::install` has taken effect).
fn render_installed(name: &str, mut setup: impl FnMut(&mut PixmithApp)) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/screens");
    std::fs::create_dir_all(&dir).unwrap();
    for dark in [true, false] {
        let mut app = PixmithApp { dark, ..Default::default() };
        setup(&mut app);
        let mut installed = false;
        let mut harness = Harness::builder().with_size([1280.0, 800.0]).with_pixels_per_point(2.0).build_ui_state(
            |ui, app: &mut PixmithApp| {
                if !installed {
                    theme::install(ui.ctx());
                    theme::apply(ui.ctx(), app.dark);
                    installed = true;
                    ui.ctx().request_discard("fonts installed");
                    return;
                }
                // The harness insets its root ui; the real window has no inset.
                let full = ui.ctx().content_rect();
                let mut root = ui.new_child(eframe::egui::UiBuilder::new().max_rect(full));
                root.set_clip_rect(full);
                app.ui(&mut root);
            },
            app,
        );
        harness.run();
        let image = harness.render().expect("render");
        let path = dir.join(format!("{name}-{}.png", if dark { "dark" } else { "light" }));
        image.save(&path).unwrap();
        println!("wrote {}", path.display());
    }
}

#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn timeline() {
    render_installed("timeline", timeline_doc);
}
