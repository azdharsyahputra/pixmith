//! Screenshots of Preferences > Extensions, Appearance > Color theme and a themed window, from the real template
//! extension. Needs a GPU (wgpu), so it is ignored by default:
//!
//! ```sh
//! cargo test --test extensions_screens -- --ignored      # writes target/screens/ext-*.png
//! ```

use eframe::egui;
use egui_kittest::Harness;
use pixmith_core::extensions::{self, zip};
use pixmith_desktop::ui::menus::PrefsTab;
use pixmith_desktop::{PixmithApp, theme};
use std::path::{Path, PathBuf};

fn collect(dir: &Path, base: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, base, out);
        } else if path.file_name().is_some_and(|n| n != "pack.sh") {
            let name = path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            out.push((name, std::fs::read(&path).unwrap()));
        }
    }
}

/// The sample extension of `scripts/extension-template/` as a package.
fn template_package(dir: &Path) -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/extension-template");
    let mut files = Vec::new();
    collect(&base, &base, &mut files);
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    let path = dir.join("sample.pixmith-extension");
    std::fs::write(&path, zip::build(&refs)).unwrap();
    path
}

fn render(name: &str, themed: bool, setup: impl Fn(&mut PixmithApp)) {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/screens");
    std::fs::create_dir_all(&out).unwrap();
    let support = std::env::temp_dir().join(format!("pixmith-extshot-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&support);
    std::fs::create_dir_all(&support).unwrap();
    // SAFETY: this file runs its tests one after another (a single test fn below).
    unsafe { std::env::set_var("PIXMITH_SUPPORT_DIR", &support) };
    extensions::refresh();
    for dark in [true, false] {
        let mut app = PixmithApp { dark, ..Default::default() };
        app.install_extension(&template_package(&support));
        app.menus.prefs_open = false;
        if themed {
            app.menus.prefs.theme = "sample-extension/themes/midnight.json".into();
            pixmith_desktop::ui::extensions::apply_theme(&app.menus.prefs.theme);
        }
        setup(&mut app);
        let mut installed = false;
        let mut harness = Harness::builder().with_size([1280.0, 800.0]).with_pixels_per_point(2.0).build_ui_state(
            |ui, app: &mut PixmithApp| {
                if !installed {
                    theme::install(ui.ctx());
                    theme::apply(ui.ctx(), app.dark);
                    installed = true;
                    ui.ctx().request_repaint();
                    return;
                }
                let screen = ui.ctx().content_rect();
                let mut window = ui.new_child(egui::UiBuilder::new().max_rect(screen));
                window.set_clip_rect(screen);
                app.ui(&mut window);
            },
            app,
        );
        harness.run();
        let image = harness.render().expect("render");
        image.save(out.join(format!("{name}-{}.png", if dark { "dark" } else { "light" }))).unwrap();
    }
    theme::clear_theme_override();
    unsafe { std::env::remove_var("PIXMITH_SUPPORT_DIR") };
    extensions::refresh();
    let _ = std::fs::remove_dir_all(support);
}

#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn extension_screens() {
    render("ext-prefs-extensions", false, |app| {
        app.menus.prefs_open = true;
        app.menus.prefs_tab = PrefsTab::Extensions;
    });
    render("ext-prefs-appearance", true, |app| {
        app.menus.prefs_open = true;
        app.menus.prefs_tab = PrefsTab::Appearance;
    });
    render("ext-theme-window", true, |_| {});
}
