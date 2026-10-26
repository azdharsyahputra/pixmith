//! Helpers shared by the project tests and screenshot scenarios: scratch folders, sample sprites and projects,
//! and a headless harness that waits for the background thumbnail and scan threads.

#![allow(dead_code)]

use eframe::egui;
use egui_kittest::Harness;
use pixmith_desktop::app::PixmithApp;
use pixmith_desktop::doc::Doc;
use pixmith_desktop::projects;
use pixmith_desktop::theme;
use pixmith_desktop::ui::canvas::{ShapeKind, flag};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A fresh scratch folder for one test.
pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pixmith-projects-ui-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A small sprite drawn from `seed`: a tinted background, a body and two eyes.
pub fn art(seed: u32, w: u32, h: u32) -> Doc {
    let mut doc = Doc::new(w, h);
    let palette = [
        (0x2B3566FF_u32, 0xE8A862FF_u32),
        (0x1F4D3AFF, 0x9ED06CFF),
        (0x4A2340FF, 0xF08AB0FF),
        (0x2A4A66FF, 0x7FD6E8FF),
        (0x553B1FFF, 0xF2D16BFF),
        (0x3A2A66FF, 0xB59CFFFF),
    ];
    let (bg, fg) = palette[seed as usize % palette.len()];
    let (w, h) = (w as i32, h as i32);
    let mut shape = |kind, a, b, color, flags| {
        doc.begin_edit("Shape");
        doc.draw_shape(kind, 0, 0, a, b, color, 1, flags, false);
        doc.stroke_end();
    };
    shape(ShapeKind::Rect, (1, 1), (w - 2, h - 2), bg, flag::FILLED);
    let inset = (seed as i32 % 3) + w / 6;
    shape(ShapeKind::Ellipse, (inset, inset), (w - 1 - inset, h - 1 - inset), fg, flag::FILLED);
    let eye = (w / 8).max(1);
    shape(ShapeKind::Rect, (w / 3 - eye / 2, h / 3), (w / 3 + eye / 2, h / 3 + eye), 0x1A1A33FF, flag::FILLED);
    shape(ShapeKind::Rect, (2 * w / 3 - eye / 2, h / 3), (2 * w / 3 + eye / 2, h / 3 + eye), 0x1A1A33FF, flag::FILLED);
    doc.touch();
    doc
}

/// A project folder `dir/name` with the given canvases (name, width, height), each drawn.
pub fn make_project(dir: &Path, name: &str, canvases: &[(&str, u32, u32)]) -> PathBuf {
    let folder = dir.join(name);
    projects::create_project(&folder, name).unwrap();
    for (i, (canvas, w, h)) in canvases.iter().enumerate() {
        let path = projects::new_canvas(&folder, canvas, *w, *h).unwrap();
        art(i as u32 + name.len() as u32, *w, *h).save(&path).unwrap();
    }
    folder
}

pub fn harness_sized(app: PixmithApp, size: [f32; 2], ppp: f32) -> Harness<'static, PixmithApp> {
    let mut installed = false;
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(ppp).build_ui_state(
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
    // Background jobs (thumbnails, scans) may keep asking for repaints on a slow machine; that is not a failure.
    let _ = h.run_ok();
    h
}

pub fn harness(app: PixmithApp) -> Harness<'static, PixmithApp> {
    harness_sized(app, [1280.0, 800.0], 1.0)
}

/// Runs frames until `done` holds for the app (the threads need real time), up to a few seconds.
pub fn settle(h: &mut Harness<'static, PixmithApp>, done: impl Fn(&PixmithApp) -> bool) {
    let start = Instant::now();
    loop {
        h.run_ok();
        if done(h.state()) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for the background work");
        std::thread::sleep(Duration::from_millis(15));
        h.step();
    }
}

/// All scans and thumbnails the screen asked for have arrived.
pub fn loaded(app: &PixmithApp) -> bool {
    app.projects.thumbs.loading() == 0 && !app.projects.scans_loading()
}
