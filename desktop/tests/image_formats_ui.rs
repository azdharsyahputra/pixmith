//! The WebP, JPEG, BMP, TGA, ICO, PCX and FLC options in the Export sheet and the "which icon size" question: widget
//! tests that need no GPU, and screenshots (ignored: they need wgpu) written to `target/screens/`:
//!
//! ```sh
//! cargo test --test image_formats_ui -- --ignored
//! ```

mod common;

use egui_kittest::kittest::Queryable;
use pixmith_desktop::PixmithApp;
use pixmith_desktop::ui::dialogs::ExportKind;
use pixmith_desktop::ui::image_formats::IconChoice;

fn app_with(kind: ExportKind, dark: bool) -> PixmithApp {
    let mut app = PixmithApp { dark, ..Default::default() };
    app.request_export(kind);
    app
}

#[test]
fn each_format_shows_its_own_options() {
    // Row labels are painted, not widgets, so the checks look for the checkboxes and the notes under the controls.
    for (kind, text) in [
        (ExportKind::Webp, "Lossless keeps every pixel"),
        (ExportKind::Jpeg, "JPEG has no transparency"),
        (ExportKind::Bmp, "Transparency needs 32 bit"),
        (ExportKind::Tga, "Transparency needs 32 bit"),
        (ExportKind::Ico, "No size chosen"),
        (ExportKind::Pcx, "PCX has no transparency"),
        (ExportKind::Flc, "FLI/FLC has 256 colors"),
    ] {
        let mut h = common::harness_sized(app_with(kind, true), [1280.0, 800.0], 1.0);
        h.run();
        assert!(h.query_by_label_contains(text).is_some(), "{kind:?} should show \"{text}\"");
        assert!(h.query_by_label_contains("Writes one frame").is_none());
    }
    let mut h = common::harness_sized(app_with(ExportKind::Webp, true), [1280.0, 800.0], 1.0);
    h.run();
    assert!(h.query_by_label("Animated (all frames)").is_some());
    let mut h = common::harness_sized(app_with(ExportKind::Tga, true), [1280.0, 800.0], 1.0);
    h.run();
    assert!(h.query_by_label("Run-length compression").is_some());
    // The sprite sheet rows are not there.
    assert!(h.query_by_label_contains("Columns apply").is_none());
}

#[test]
fn the_icon_question_names_the_file() {
    let mut app = PixmithApp::default();
    app.files.icon_choice = Some(IconChoice { path: "hero.ico".into(), sizes: vec![16, 32, 256], selected: 2 });
    let mut h = common::harness_sized(app, [1280.0, 800.0], 1.0);
    h.run();
    assert!(h.query_by_label_contains("Which size of the icon").is_some());
    assert!(h.query_by_label_contains("hero.ico").is_some());
    assert!(h.query_by_label_contains("holds 3 sizes").is_some());
}

fn shoot(name: &str, setup: impl Fn(bool) -> PixmithApp) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/screens");
    std::fs::create_dir_all(&dir).unwrap();
    for dark in [true, false] {
        let mut h = common::harness_sized(setup(dark), [1280.0, 800.0], 2.0);
        h.run();
        let image = h.render().expect("render");
        let path = dir.join(format!("{name}-{}.png", if dark { "dark" } else { "light" }));
        image.save(&path).unwrap();
        println!("wrote {}", path.display());
    }
}

#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn screenshots() {
    for (name, kind) in [
        ("export-webp", ExportKind::Webp),
        ("export-jpeg", ExportKind::Jpeg),
        ("export-tga", ExportKind::Tga),
        ("export-ico", ExportKind::Ico),
        ("export-flc", ExportKind::Flc),
    ] {
        shoot(name, |dark| app_with(kind, dark));
    }
    shoot("icon-size", |dark| {
        let mut app = PixmithApp { dark, ..Default::default() };
        app.files.icon_choice = Some(IconChoice { path: "hero.ico".into(), sizes: vec![16, 32, 48, 256], selected: 3 });
        app
    });
}
