//! Pixmith desktop entry point.

#![cfg_attr(windows, windows_subsystem = "windows")]

use eframe::egui;

/// The app icon shown in the title bar, task bar and dock (`assets/logo.png`).
const LOGO: &[u8] = include_bytes!("../../assets/logo.png");

fn main() {
    use pixmith_desktop::startup;
    startup::install_panic_hook();
    // `Pixmith --diagnose`: report the renderer choice (to the log, and a dialog) and exit.
    if std::env::args().any(|a| a == "--diagnose") {
        let report = startup::diagnose();
        if std::env::var_os("PIXMITH_DIAGNOSE_QUIET").is_none() {
            startup::show_error("Pixmith diagnostics", &report);
        }
        return;
    }
    let choice = startup::choose_renderer();
    startup::log(&format!("start {} renderer={:?}: {}", env!("CARGO_PKG_VERSION"), choice.renderer, choice.reason));

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
    let options = eframe::NativeOptions { viewport, renderer: choice.renderer, ..Default::default() };
    let result = eframe::run_native(
        "Pixmith",
        options,
        Box::new(|cc| {
            let mut app = pixmith_desktop::PixmithApp::new(cc);
            // `Pixmith.exe file.pxmt` (Windows "Open with"): documents named on the command line open straight
            // away and skip Home; a folder opens as a project.
            app.open_args(
                std::env::args_os().skip(1).filter(|a| !a.to_string_lossy().starts_with('-')).map(Into::into),
            );
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = result {
        startup::log(&format!("failed to start with {:?}: {e}", choice.renderer));
        if choice.renderer == eframe::Renderer::Wgpu && !choice.is_fallback && startup::relaunch_with_opengl() {
            startup::log("relaunched with OpenGL");
            return;
        }
        startup::show_error(
            "Pixmith could not start",
            &format!(
                "{e}\n\nYour graphics driver may be too old. Updating it usually helps.\nDetails: {}",
                startup::log_path().display()
            ),
        );
        std::process::exit(1);
    }
}
