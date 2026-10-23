//! Renders the window headlessly to PNGs for side-by-side comparison with the macOS app's screenshots in
//! `docs/design/screens/`. Needs a GPU (wgpu), so it is ignored by default:
//!
//! ```sh
//! cargo test --test screens -- --ignored        # writes target/screens/<name>-<theme>.png
//! ```

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
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

/// The sprite of the `canvas` scenario with a rectangular selection (marching ants); with `transform`, a free
/// transform of it as in `transform-*@2x.png` (the strip under the context bar, handles and pivot).
fn selection_scene(app: &mut PixmithApp, tool: pixmith_desktop::app::Tool, transform: bool) {
    use pixmith_desktop::doc::Doc;
    use pixmith_desktop::ui::canvas::{ShapeKind, flag};
    use pixmith_desktop::ui::selection::SelMode;
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
    doc.sel_begin(SelMode::Replace);
    doc.sel_rect((14, 16), (33, 30));
    doc.sel_end();
    if transform {
        let layer = doc.layer_count() - 1;
        assert!(doc.xf_begin(layer, 0, false));
        doc.xf_set_field(4, 12.0);
    }
    app.tabs[0] = pixmith_desktop::app::DocTab::new(doc);
    app.tabs[0].zoom = 8.0;
    app.primary = 0xF2A65AFF;
    app.tool = tool;
    let tab = &app.tabs[0];
    app.canvas.keep_view(tab);
    app.selection.system_clipboard = false;
}

/// Like `render_installed`, but tolerates the repaints the marching ants ask for.
fn render_animated(name: &str, mut setup: impl FnMut(&mut PixmithApp)) {
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
                let full = ui.ctx().content_rect();
                let mut root = ui.new_child(eframe::egui::UiBuilder::new().max_rect(full));
                root.set_clip_rect(full);
                app.ui(&mut root);
            },
            app,
        );
        harness.run_ok();
        let image = harness.render().expect("render");
        let path = dir.join(format!("{name}-{}.png", if dark { "dark" } else { "light" }));
        image.save(&path).unwrap();
        println!("wrote {}", path.display());
    }
}

#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn selection() {
    render_animated("selection", |app| selection_scene(app, pixmith_desktop::app::Tool::Marquee, false));
}

#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn selection_wand() {
    render_animated("selection-wand", |app| {
        selection_scene(app, pixmith_desktop::app::Tool::Wand, false);
        app.selection.tolerance = 32;
    });
}

#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn transform() {
    render_animated("transform", |app| selection_scene(app, pixmith_desktop::app::Tool::Marquee, true));
}

#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn selection_move_handles() {
    render_animated("selection-move", |app| selection_scene(app, pixmith_desktop::app::Tool::Move, false));
}

/// A 128 x 96 sprite with a "Ground" tilemap layer, a blob terrain with its reduced template, and some painted
/// ground (compare `tilemap-autotile-*.png`).
fn autotile_doc(app: &mut PixmithApp) {
    use pixmith_desktop::app::{DocTab, Tool};
    use pixmith_desktop::doc::Doc;
    let mut doc = Doc::new(128, 96);
    doc.rename_layer(0, "Layer 1");
    let layer = doc.new_tilemap_layer(0, 16, 16);
    doc.rename_layer(layer, "Ground");
    assert!(doc.create_terrain(layer, "Grass", 1));
    doc.add_terrain(layer);
    doc.add_terrain(layer);
    assert!(doc.generate_template(layer, true));
    doc.set_tile_mask(layer, 3, -1);
    doc.begin_edit("Terrain Rectangle");
    doc.paint_terrain_rect(layer, 0, (16, 16), (63, 47), 1, 0);
    doc.paint_terrain_rect(layer, 0, (32, 48), (47, 63), 1, 0);
    doc.paint_terrain_rect(layer, 0, (96, 64), (111, 79), 1, 0);
    app.tabs[0] = DocTab::new(doc);
    app.tabs[0].layer = layer;
    app.tabs[0].zoom = 6.0;
    app.tool = Tool::Pencil;
    app.tilemap.brush = true;
    app.tilemap.show_masks = true;
    app.tilemap.selected = 6;
    let tab = &app.tabs[0];
    app.canvas.keep_view(tab);
}

/// Like `render_installed` at a custom window size, with some inspector sections collapsed first.
fn render_sized(name: &str, size: [f32; 2], collapse: &[&str], mut setup: impl FnMut(&mut PixmithApp)) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/screens");
    std::fs::create_dir_all(&dir).unwrap();
    for dark in [true, false] {
        let mut app = PixmithApp { dark, ..Default::default() };
        setup(&mut app);
        let mut installed = false;
        let mut harness = Harness::builder().with_size(size).with_pixels_per_point(2.0).build_ui_state(
            |ui, app: &mut PixmithApp| {
                if !installed {
                    theme::install(ui.ctx());
                    theme::apply(ui.ctx(), app.dark);
                    for id in collapse {
                        pixmith_desktop::ui::widgets::set_section_collapsed(ui.ctx(), id, true);
                    }
                    installed = true;
                    ui.ctx().request_discard("fonts installed");
                    return;
                }
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
fn autotile() {
    render_sized("tilemap-autotile", [1280.0, 800.0], &["color", "palette", "layer", "cel"], autotile_doc);
}

/// The whole inspector of the autotile scene, tall enough to show the Terrain controls.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn autotile_inspector() {
    render_sized("tilemap-autotile-inspector", [1280.0, 1500.0], &["color", "palette", "layer", "cel"], autotile_doc);
}

/// The face sprite on a layer converted to a 16 x 16 tilemap, stamp mode on (compare `tilemap-*.png`).
fn tilemap_doc(app: &mut PixmithApp) {
    use pixmith_desktop::app::{DocTab, Tool};
    use pixmith_desktop::doc::Doc;
    use pixmith_desktop::ui::canvas::{ShapeKind, flag};
    let mut doc = Doc::new(48, 48);
    doc.rename_layer(0, "Tiles");
    let mut shape = |kind, a, b, color, flags| {
        doc.begin_edit("Shape");
        doc.draw_shape(kind, 0, 0, a, b, color, 1, flags, false);
        doc.stroke_end();
    };
    shape(ShapeKind::Rect, (4, 4), (43, 43), 0x2B3566FF, flag::FILLED);
    shape(ShapeKind::Ellipse, (12, 8), (35, 40), 0xE8A862FF, flag::FILLED);
    assert!(doc.convert_layer_to_tilemap(0, 16, 16, true));
    app.tabs[0] = DocTab::new(doc);
    app.tabs[0].layer = 0;
    app.tabs[0].zoom = 8.0;
    app.tool = Tool::Pencil;
    app.tilemap.set_stamp(true);
    app.tilemap.selected = 2;
    let tab = &app.tabs[0];
    app.canvas.keep_view(tab);
}

#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn tilemap() {
    render_sized("tilemap", [1280.0, 800.0], &["color", "palette", "layer", "cel"], tilemap_doc);
}

/// Slices with a 9-patch center and a pivot, the slice tool active.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn slices() {
    use pixmith_desktop::app::{DocTab, Tool};
    use pixmith_desktop::doc::Doc;
    render_sized("slices", [1280.0, 800.0], &["color", "palette", "layer", "cel"], |app| {
        let mut doc = Doc::new(48, 48);
        let a = doc.add_slice(4, 4, 20, 16).unwrap();
        doc.set_slice_props(a, 0, "button", Some([4, 4, 12, 8]), Some([10, 8]));
        doc.add_slice(26, 10, 16, 30).unwrap();
        doc.set_slice_props(1, 0, "panel", None, None);
        app.tabs[0] = DocTab::new(doc);
        app.tabs[0].zoom = 8.0;
        app.tool = Tool::Slice;
        app.tilemap.slice_selected = Some(0);
        let tab = &app.tabs[0];
        app.canvas.keep_view(tab);
    });
}

/// The Slice Properties sheet over the slices scene.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn slice_sheet() {
    use pixmith_desktop::app::{DocTab, Tool};
    use pixmith_desktop::doc::Doc;
    render_sized("slice-sheet", [1280.0, 800.0], &["color", "palette", "layer", "cel"], |app| {
        let mut doc = Doc::new(48, 48);
        let a = doc.add_slice(4, 4, 20, 16).unwrap();
        doc.set_slice_props(a, 0, "button", Some([4, 4, 12, 8]), Some([10, 8]));
        doc.set_user_data(4, a, 0, "9-patch button", Some(0xE8A862FF));
        app.tabs[0] = DocTab::new(doc);
        app.tabs[0].zoom = 8.0;
        app.tool = Tool::Slice;
        app.tilemap.slice_selected = Some(0);
        pixmith_desktop::ui::tilemap::slice_properties(app);
        let tab = &app.tabs[0];
        app.canvas.keep_view(tab);
    });
}

// ---- Drawing tools and view features (`ui::tools`) ----

/// A small sprite with a few shapes on `frames` frames; the body moves two pixels right per frame.
fn tools_sprite(frames: u32) -> pixmith_desktop::doc::Doc {
    use pixmith_desktop::doc::Doc;
    use pixmith_desktop::ui::canvas::{ShapeKind, flag};
    let mut doc = Doc::new(24, 24);
    for _ in 1..frames {
        let last = doc.frame_count() - 1;
        doc.add_frame(last, false);
    }
    let layer = doc.layer_count() - 1;
    for f in 0..frames {
        let dx = f as i32 * 2;
        doc.begin_edit("Shape");
        doc.draw_shape(ShapeKind::Ellipse, layer, f, (4 + dx, 6), (13 + dx, 17), 0xE8A862FF, 1, flag::FILLED, false);
        doc.draw_shape(ShapeKind::Rect, layer, f, (7 + dx, 9), (8 + dx, 11), 0x1A1A33FF, 1, flag::FILLED, false);
        doc.stroke_end();
    }
    doc
}

/// View > View Options (floating window over the editor).
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn view_options() {
    render_installed("view-options", |app| {
        app.tabs[0] = pixmith_desktop::app::DocTab::new(tools_sprite(3));
        app.tabs[0].frame = 1;
        app.timeline.onion_skin = true;
        app.canvas.tools.settings.onion_future = 1;
        app.canvas.tools.settings.onion_tint = true;
        pixmith_desktop::ui::tools::open_view_options(app);
    });
}

/// The canvas with onion skin (tinted), tiled in both axes, the pixel grid and a custom grid.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn onion_tiled_grid() {
    render_installed("onion-tiled", |app| {
        use pixmith_desktop::ui::tools;
        app.tabs[0] = pixmith_desktop::app::DocTab::new(tools_sprite(3));
        app.tabs[0].frame = 1;
        app.tabs[0].zoom = 12.0;
        let tab = &app.tabs[0];
        app.canvas.keep_view(tab);
        app.timeline.onion_skin = true;
        app.canvas.tools.settings.onion_future = 1;
        app.canvas.tools.settings.onion_tint = true;
        app.canvas.tools.settings.onion_opacity = 0.5;
        app.canvas.tools.settings.custom_grid = true;
        app.canvas.tools.settings.custom_grid_w = 8;
        app.canvas.tools.settings.custom_grid_h = 8;
        tools::set_tiled_mode(app, tools::view::TiledMode::Both);
    });
}

/// View > Preview.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn preview_window() {
    render_installed("preview", |app| {
        app.tabs[0] = pixmith_desktop::app::DocTab::new(tools_sprite(3));
        pixmith_desktop::ui::tools::open_preview(app);
        app.canvas.tools.preview.playing = false;
    });
}

/// The Text sheet with a live preview on the canvas.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn text_sheet() {
    render_installed("text-sheet", |app| {
        use pixmith_desktop::app::Tool;
        use pixmith_desktop::ui::tools::text::TextRequest;
        app.tool = Tool::Text;
        let tab = app.tab_mut();
        tab.doc.begin_edit("Text");
        let depth = tab.doc.history_counts().0;
        let doc = tab.doc.raw() as usize;
        app.canvas.tools.text.string = "Pixmith".into();
        app.canvas.tools.text.begin(TextRequest { at: (3, 4), color: 0xF2A65AFF, started: false, depth, doc });
    });
}

/// Context bars of the new tools: spray with its density slider, gradient, curve.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn tool_bars() {
    use pixmith_desktop::app::Tool;
    for (name, tool) in [("spray", Tool::Spray), ("gradient", Tool::Gradient), ("curve", Tool::Curve)] {
        render_installed(&format!("bar-{name}"), |app| app.tool = tool);
    }
}

/// Like `render_installed`, then runs `interact` on the harness (clicks, key presses) before the picture.
fn render_interactive(
    name: &str,
    mut setup: impl FnMut(&mut PixmithApp),
    mut interact: impl FnMut(&mut Harness<'_, PixmithApp>),
) {
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
                let full = ui.ctx().content_rect();
                let mut root = ui.new_child(eframe::egui::UiBuilder::new().max_rect(full));
                root.set_clip_rect(full);
                app.ui(&mut root);
            },
            app,
        );
        harness.run();
        interact(&mut harness);
        harness.run();
        let image = harness.render().expect("render");
        let path = dir.join(format!("{name}-{}.png", if dark { "dark" } else { "light" }));
        image.save(&path).unwrap();
        println!("wrote {}", path.display());
    }
}

/// The File menu open.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn menu_file() {
    render_interactive(
        "menu-file",
        |_| {},
        |h| {
            h.get_by_label("File").click();
            h.run();
        },
    );
}

/// The Sprite menu with the Color Mode submenu open.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn menu_sprite() {
    render_interactive(
        "menu-sprite",
        |_| {},
        |h| {
            h.get_by_label("Sprite").click();
            h.run();
            h.get_by_label_contains("Color Mode").hover();
            h.run();
        },
    );
}

/// The face sprite of the `canvas` scenario at 800%, so the sheets can be compared with the macOS screenshots.
fn face(app: &mut PixmithApp) {
    use pixmith_desktop::doc::Doc;
    use pixmith_desktop::ui::canvas::{ShapeKind, flag};
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
    let tab = &app.tabs[0];
    app.canvas.keep_view(tab);
}

fn open_filter(app: &mut PixmithApp, kind: pixmith_desktop::ui::menus::FilterKind) {
    let PixmithApp { menus, tabs, active, primary, .. } = app;
    menus.open_filter(&mut tabs[*active], kind, *primary);
}

/// Hue/Saturation/Lightness with a live preview (compare `filter-sheet-*@2x.png`).
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn filter_hsl() {
    render_interactive(
        "filter-hsl",
        |app| {
            face(app);
            open_filter(app, pixmith_desktop::ui::menus::FilterKind::Hsl);
            if let Some(f) = &mut app.menus.filter {
                f.p.hue = 60;
            }
        },
        |h| {
            h.run();
        },
    );
}

/// Replace Color, Outline and Color Curves sheets.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn filter_others() {
    use pixmith_desktop::ui::menus::FilterKind;
    for (name, kind) in [
        ("filter-replace", FilterKind::ReplaceColor),
        ("filter-outline", FilterKind::Outline),
        ("filter-curves", FilterKind::Curves),
    ] {
        render_interactive(
            name,
            |app| {
                face(app);
                open_filter(app, kind);
                if let Some(f) = &mut app.menus.filter {
                    if kind == FilterKind::Curves {
                        f.p.curves.channels[0] = vec![(0, 0), (64, 100), (192, 170), (255, 255)];
                    }
                    if kind == FilterKind::Outline {
                        f.p.outline_color = 0xFF3B30FF;
                    }
                }
            },
            |h| {
                h.run();
            },
        );
    }
}

/// Scale Sprite, Canvas Size, Cel Properties and About.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn small_sheets() {
    use pixmith_desktop::ui::menus::FilterKind;
    render_interactive(
        "sheet-scale",
        |app| open_filter(app, FilterKind::Scale),
        |h| {
            h.run();
        },
    );
    render_interactive(
        "sheet-canvas-size",
        |app| {
            app.menus.open_canvas_size(32, 32);
        },
        |h| {
            h.run();
        },
    );
    render_interactive(
        "sheet-cel",
        |app| {
            let PixmithApp { menus, tabs, active, .. } = app;
            menus.open_cel_sheet(&tabs[*active]);
        },
        |h| {
            h.run();
        },
    );
    render_interactive(
        "sheet-about",
        |app| app.menus.about_open = true,
        |h| {
            h.run();
        },
    );
}

/// The History panel, Preferences (three tabs) and the shortcuts list.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn windows() {
    use pixmith_desktop::ui::menus::PrefsTab;
    render_interactive(
        "history",
        |app| {
            face(app);
            let tab = app.tab_mut();
            for name in ["Pencil", "Rectangle", "Filter: Invert", "Filter: Blur"] {
                tab.doc.begin_edit(name);
                tab.doc.stroke_end();
                tab.doc.label_last_step(name);
            }
            tab.doc.undo();
            tab.changed();
            app.menus.history_open = true;
        },
        |h| {
            h.run();
        },
    );
    for (name, tab) in [
        ("prefs-general", PrefsTab::General),
        ("prefs-appearance", PrefsTab::Appearance),
        ("prefs-keyboard", PrefsTab::Keyboard),
    ] {
        render_interactive(
            name,
            |app| {
                app.menus.prefs_open = true;
                app.menus.prefs_tab = tab;
            },
            |h| {
                h.run();
            },
        );
    }
    render_interactive(
        "shortcuts-help",
        |app| app.menus.help_open = true,
        |h| {
            h.run();
        },
    );
}
