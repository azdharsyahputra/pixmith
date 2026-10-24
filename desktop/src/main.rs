//! Pixmith desktop entry point.

#![cfg_attr(windows, windows_subsystem = "windows")]

use eframe::egui;

/// The app icon shown in the title bar, task bar and dock (`assets/logo.png`).
const LOGO: &[u8] = include_bytes!("../../assets/logo.png");

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Pixmith")
        .with_app_id("pixmith")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([1000.0, 640.0])
        // Open filling the screen: the smaller default window cut off the inspector and timeline.
        .with_maximized(true)
        .with_drag_and_drop(true);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(LOGO) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("Pixmith", options, Box::new(|cc| Ok(Box::new(pixmith_desktop::PixmithApp::new(cc)))))
}
