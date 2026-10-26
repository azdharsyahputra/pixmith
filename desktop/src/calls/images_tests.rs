use super::*;
use crate::app::PixmithApp;
use crate::ui::dialogs::ExportKind;
use crate::ui::image_formats::{EXPORT_KINDS, ImageSettings};
use std::path::PathBuf;

const RED: u32 = 0xFF0000FF;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pixmith-desktop-images-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn put(doc: &mut Doc, frame: u32, x: i32, y: i32) {
    doc.begin_edit("Draw");
    doc.stroke_begin(0, frame, x, y, RED, 1, 0);
    doc.stroke_end();
}

/// 8x6, three frames of 50/100/150 ms with a red pixel on frame k at (k, k).
fn animation() -> Doc {
    let mut doc = Doc::new(8, 6);
    put(&mut doc, 0, 0, 0);
    doc.add_frame(0, true);
    put(&mut doc, 1, 1, 1);
    doc.add_frame(1, true);
    put(&mut doc, 2, 2, 2);
    for (f, ms) in [50, 100, 150].into_iter().enumerate() {
        doc.set_frame_duration(f as u32, ms);
    }
    doc
}

fn pixels(doc: &Doc, frame: u32) -> Vec<u8> {
    let mut out = Vec::new();
    assert!(doc.render(frame, &mut out));
    out
}

#[test]
#[allow(clippy::chunks_exact_to_as_chunks)]
fn every_format_exports_and_opens_again() {
    let dir = scratch("roundtrip");
    let doc = animation();
    let want = pixels(&doc, 0);
    for kind in EXPORT_KINDS {
        let ext = crate::ui::image_formats::extension(kind);
        let path = dir.join(format!("hero.{ext}"));
        let s = ImageSettings::default();
        let frame = (!s.all_frames(kind)).then_some(0);
        doc.export_image(frame, 1, &path, &s.json()).unwrap_or_else(|e| panic!("{ext}: {e}"));
        let back = Doc::import_image(&path, None).unwrap_or_else(|e| panic!("{ext}: {e}"));
        assert_eq!((back.width(), back.height()), (8, 6), "{ext}");
        assert_eq!(back.frame_count(), if s.all_frames(kind) { 3 } else { 1 }, "{ext}");
        let got = pixels(&back, 0);
        // Formats without alpha show the white background where the sprite is empty; JPEG is only close.
        let first_red = |px: &[u8]| px.chunks_exact(4).position(|p| p[3] == 255 && p[0] > 200 && p[1] < 60);
        if matches!(ext, "bmp" | "tga" | "webp" | "ico") {
            assert_eq!(got, want, "{ext}");
        } else if ext != "jpg" {
            assert_eq!(first_red(&got), Some(0), "{ext}");
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn webp_and_flc_keep_durations() {
    let dir = scratch("durations");
    let doc = animation();
    for ext in ["webp", "flc"] {
        let path = dir.join(format!("a.{ext}"));
        doc.export_image(None, 2, &path, "{}").unwrap();
        let back = Doc::import_image(&path, None).unwrap();
        assert_eq!(back.frame_count(), 3, "{ext}");
        assert_eq!((back.width(), back.height()), (16, 12), "{ext}");
        assert_eq!((0..3).map(|f| back.frame_duration(f)).collect::<Vec<_>>(), vec![50, 100, 150], "{ext}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn icon_sizes_are_listed_and_one_can_be_opened() {
    let dir = scratch("ico");
    let doc = animation();
    let path = dir.join("hero.ico");
    let settings = ImageSettings { ico_sizes: 0b1_0101, ..Default::default() }; // 16, 32, 64
    doc.export_image(Some(0), 1, &path, &settings.json()).unwrap();
    let info = Doc::image_info(&path).unwrap();
    assert_eq!((info.format.as_str(), info.sizes.clone()), ("ico", vec![16, 32, 64]));
    assert_eq!(Doc::import_image(&path, None).unwrap().width(), 64);
    assert_eq!(Doc::import_image(&path, Some(16)).unwrap().width(), 16);
    assert!(Doc::import_image(&path, Some(48)).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn opening_files_goes_through_the_app() {
    let dir = scratch("open");
    let doc = animation();
    let mut app = PixmithApp::default();
    for ext in ["webp", "jpg", "bmp", "tga", "pcx", "flc"] {
        let path = dir.join(format!("a.{ext}"));
        doc.export_image(None, 1, &path, "{}").unwrap();
        app.open_path(&path);
        assert!(app.error.is_none(), "{ext}: {:?}", app.error);
        assert_eq!((app.tab().doc.width(), app.tab().doc.height()), (8, 6), "{ext}");
        assert!(app.tab().doc.is_dirty() && app.tab().doc.path.is_none(), "{ext} opens as an unsaved sprite");
    }
    // An icon with one size opens straight away; with several it asks first.
    let single = dir.join("one.ico");
    doc.export_image(Some(0), 1, &single, "{}").unwrap();
    app.open_path(&single);
    assert!(app.files.icon_choice.is_none() && app.error.is_none());
    let many = dir.join("many.ico");
    doc.export_image(Some(0), 1, &many, r#"{"ico_sizes":[16,32,48]}"#).unwrap();
    let tabs = app.tabs.len();
    app.open_path(&many);
    let choice = app.files.icon_choice.clone().expect("the icon question");
    assert_eq!((choice.sizes.clone(), choice.selected), (vec![16, 32, 48], 2));
    assert_eq!(app.tabs.len(), tabs, "nothing opens before the answer");
    app.finish_open_image(&choice.path, Some(32));
    app.files.icon_choice = None;
    assert_eq!(app.tab().doc.width(), 32);
    // A damaged file is an error message, not a tab.
    let bad = dir.join("bad.bmp");
    std::fs::write(&bad, b"BM not really a bitmap at all, just text").unwrap();
    let tabs = app.tabs.len();
    app.open_path(&bad);
    assert!(app.error.is_some());
    assert_eq!(app.tabs.len(), tabs);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn export_kinds_map_to_formats_the_core_writes() {
    let dir = scratch("kinds");
    let doc = Doc::new(4, 4);
    for kind in EXPORT_KINDS {
        assert_ne!(kind, ExportKind::Png);
        let path = dir.join(format!("k.{}", crate::ui::image_formats::extension(kind)));
        doc.export_image(Some(0), 1, &path, &ImageSettings::default().json()).unwrap();
        assert!(path.metadata().unwrap().len() > 0);
    }
    assert!(doc.export_image(Some(0), 1, &dir.join("x.unknown"), "{}").is_err());
    assert!(doc.export_image(Some(9), 1, &dir.join("x.bmp"), "{}").is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[allow(clippy::chunks_exact_to_as_chunks)]
fn trace_import_opens_pseudo_pixel_art_at_real_size() {
    use crate::ui::menus::hooks::TraceImport;
    let dir = scratch("trace");
    // A 12 x 12 sprite of four colors, enlarged by 7.3 with nearest-neighbor sampling (no margin, no soft edges).
    let colors = [[200u8, 40, 40], [40, 160, 60], [30, 30, 200], [240, 220, 40]];
    let sprite = |x: usize, y: usize| colors[(x * 7 + y * 13 + x * y) % 4];
    let (w, h) = (88usize, 88usize);
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let c = sprite(((x as f64 + 0.5) / 7.3) as usize, ((y as f64 + 0.5) / 7.3) as usize);
            rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    let path = dir.join("pseudo.png");
    let mut source = pixmith_core::document::Sprite::new(w as u32, h as u32);
    source.cels[0][0] = Some(std::sync::Arc::new(pixmith_core::document::Cel { pixels: rgba, ..Default::default() }));
    std::fs::write(&path, pixmith_core::formats::export::frame_png(&source, 0, 1).unwrap()).unwrap();

    let mut t = TraceImport::new(path.clone());
    t.remove_background = false;
    t.colors = 4;
    let json = t.options_json();
    let parsed = pixmith_core::formats::json::parse(&json).unwrap();
    pixmith_core::paint::trace::TraceOptions::from_json(&parsed).expect("the dialog writes options the core accepts");
    let doc = Doc::import_trace(&path, &json).unwrap();
    assert_eq!((doc.width(), doc.height()), (12, 12));
    let got = pixels(&doc, 0);
    for y in 0..12 {
        for x in 0..12 {
            let want = sprite(x, y);
            assert_eq!(&got[(y * 12 + x) * 4..][..4], &[want[0], want[1], want[2], 255], "cell {x},{y}");
        }
    }
    // Options the core refuses come back as an error, not a panic.
    assert!(Doc::import_trace(&path, "{\"mode\":\"trace_grid\",\"colors\":\"lots\"}").is_err());
}
