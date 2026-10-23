//! Screenshots of the game-development features next to the macOS ones in `docs/design/screens/`
//! (`tween-*@2x.png`, `godot-export-*@2x.png`, `godot-tileset-*@2x.png`). Needs a GPU (wgpu), so they are
//! ignored by default:
//!
//! ```sh
//! cargo test --test gamedev_screens -- --ignored    # writes target/screens/<name>-<theme>.png
//! ```

use eframe::egui;
use egui_kittest::Harness;
use pixmith_desktop::app::DocTab;
use pixmith_desktop::doc::Doc;
use pixmith_desktop::ui::canvas::{ShapeKind, flag};
use pixmith_desktop::ui::gamedev::{TweenEase, TweenProp};
use pixmith_desktop::ui::timeline::TagInfo;
use pixmith_desktop::{PixmithApp, theme};

/// Renders `app` at the macOS screenshot size (1280 x 800 points, 2x) in both themes, waiting for the fonts to be
/// installed before the first real pass.
fn render(name: &str, mut setup: impl FnMut(&mut PixmithApp), frames: usize) {
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
                let mut root = ui.new_child(egui::UiBuilder::new().max_rect(full));
                root.set_clip_rect(full);
                app.ui(&mut root);
            },
            app,
        );
        for _ in 0..frames {
            harness.step();
        }
        let image = harness.render().expect("render");
        let path = dir.join(format!("{name}-{}.png", if dark { "dark" } else { "light" }));
        image.save(&path).unwrap();
        println!("wrote {}", path.display());
    }
}

/// An orange face as a one-layer sprite called "Hero" with eight frames.
fn hero_doc(width: u32, height: u32) -> Doc {
    let mut doc = Doc::new(width, height);
    let mut shape = |kind, a, b, color, flags| {
        doc.begin_edit("Shape");
        doc.draw_shape(kind, 0, 0, a, b, color, 1, flags, false);
        doc.stroke_end();
    };
    let (cx, cy) = (width as i32 / 2, height as i32 / 2);
    shape(ShapeKind::Ellipse, (cx - 10, cy - 12), (cx + 10, cy + 12), 0x1A1A33FF, flag::FILLED);
    shape(ShapeKind::Ellipse, (cx - 9, cy - 11), (cx + 9, cy + 11), 0xE8A862FF, flag::FILLED);
    shape(ShapeKind::Rect, (cx - 4, cy - 4), (cx - 1, cy), 0x1A1A33FF, flag::FILLED);
    doc.rename_layer(0, "Hero");
    for _ in 0..7 {
        let last = doc.frame_count() - 1;
        doc.add_frame(last, true);
    }
    doc
}

fn scratch_project(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pixmith-screens-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("project.godot"), "config_version=5\n").unwrap();
    dir
}

/// The Tween panel over a canvas with a motion path and the transform handles.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn tween() {
    render(
        "tween",
        |app| {
            let mut tab = DocTab::new(hero_doc(96, 64));
            tab.zoom = 8.0;
            tab.layer = 0;
            for (prop, frame, value) in [
                (TweenProp::PosX, 0, -26.0),
                (TweenProp::PosX, 7, 40.0),
                (TweenProp::PosY, 0, 24.0),
                (TweenProp::PosY, 3, -10.0),
                (TweenProp::PosY, 7, 24.0),
                (TweenProp::Rotation, 0, 0.0),
                (TweenProp::Rotation, 7, 40.0),
            ] {
                tab.doc.tween_set_key(0, prop, frame, value, TweenEase::Linear, [0.0; 4]);
            }
            tab.frame = 3;
            app.tabs[0] = tab;
            app.primary = 0xF2A65AFF;
            let tab = &app.tabs[0];
            app.canvas.keep_view(tab);
            let tween = &mut app.gamedev.tween;
            tween.open = true;
            tween.edit_on_canvas = true;
            tween.sel_prop = TweenProp::Rotation;
            tween.sel_frame = Some(0);
        },
        4,
    );
}

/// The Godot SpriteFrames sheet with the atlas preview.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn godot_export() {
    render(
        "godot-export",
        |app| {
            let project = scratch_project("godot");
            let mut doc = hero_doc(48, 48);
            doc.add_tag(&TagInfo { name: "idle".into(), from: 0, to: 2, direction: 0, repeat: 0, color: 0x3B82F6 });
            doc.add_tag(&TagInfo { name: "blink".into(), from: 3, to: 5, direction: 0, repeat: 0, color: 0xEF4444 });
            app.tabs[0] = DocTab::new(doc);
            app.tabs[0].zoom = 8.0;
            let tab = &app.tabs[0];
            app.canvas.keep_view(tab);
            app.primary = 0xF2A65AFF;
            app.gamedev.prefs.set_godot_root(&project.to_string_lossy());
            app.open_godot_export();
        },
        6,
    );
}

/// The TileSet sheet.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn godot_tileset() {
    render(
        "godot-tileset",
        |app| {
            let doc = Doc::new(128, 96);
            // SAFETY: the document is live and the arguments are in range.
            unsafe {
                pixmith_core::document::tilemap::pixmith_add_tilemap_layer(doc.raw(), 0, 16, 16);
                for _ in 0..4 {
                    pixmith_core::document::tilemap::pixmith_tileset_add_tile(doc.raw(), 0);
                }
            }
            app.tabs[0] = DocTab::new(doc);
            app.tabs[0].layer = 1;
            app.tabs[0].zoom = 4.0;
            let tab = &app.tabs[0];
            app.canvas.keep_view(tab);
            app.open_tileset_export();
            if let Some(sheet) = app.gamedev.tileset.as_mut() {
                sheet.s.project_root = "/Users/dev/games/platformer".into();
                sheet.s.output_path = "/Users/dev/games/platformer/art/ground.tres".into();
                sheet.s.terrain_name = "Grass".into();
                sheet.s.collision = 1;
            }
        },
        3,
    );
}
