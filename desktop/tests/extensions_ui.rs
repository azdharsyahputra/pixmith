//! Preferences > Extensions, installing a package and the color theme picker, driven headlessly with synthetic
//! input. Each test points `PIXMITH_SUPPORT_DIR` at its own folder, so nothing touches the real app data.

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pixmith_core::extensions::{self, zip};
use pixmith_desktop::ui::extensions::{apply_theme, theme_choices};
use pixmith_desktop::ui::menus::{Prefs, PrefsTab};
use pixmith_desktop::{PixmithApp, theme};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

/// The support folder is process-wide, so the tests here take turns.
static LOCK: Mutex<()> = Mutex::new(());

struct Support {
    dir: PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Support {
    fn new(name: &str) -> Support {
        let guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("pixmith-extui-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: the tests of this file are serialized by LOCK and no other thread reads the variable meanwhile.
        unsafe { std::env::set_var("PIXMITH_SUPPORT_DIR", &dir) };
        extensions::refresh();
        Support { dir, _guard: guard }
    }
}

impl Drop for Support {
    fn drop(&mut self) {
        theme::clear_theme_override();
        unsafe { std::env::remove_var("PIXMITH_SUPPORT_DIR") };
        extensions::refresh();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const MANIFEST: &str = r#"{"name":"sample","displayName":"Sample Pack","version":"1.2.0","author":"Ada",
  "description":"A palette and a theme.",
  "contributes":{"palettes":[{"name":"Sunset","path":"p.gpl"}],"themes":[{"name":"Midnight","path":"t.json"}]}}"#;
const THEME: &str = r##"{"name":"Midnight","accent":"#FF6AC1","dark":{"panel":"#14141F","text":"#EDEDF7"},"light":{"panel":"#F4F1FA"}}"##;
const GPL: &str = "GIMP Palette\nName: Sunset\n#\n255 0 0\n255 128 0\n255 255 0\n";

fn write_package(dir: &std::path::Path) -> PathBuf {
    let bytes =
        zip::build(&[("package.json", MANIFEST.as_bytes()), ("p.gpl", GPL.as_bytes()), ("t.json", THEME.as_bytes())]);
    let path = dir.join("sample.pixmith-extension");
    std::fs::write(&path, bytes).unwrap();
    path
}

fn harness(app: PixmithApp) -> Harness<'static, PixmithApp> {
    let mut installed = false;
    let mut h = Harness::builder().with_size([1280.0, 800.0]).build_ui_state(
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
    let _ = h.try_run();
    h
}

#[test]
fn an_empty_extensions_tab_explains_itself() {
    let _s = Support::new("empty");
    let mut app = PixmithApp::default();
    app.menus.prefs_open = true;
    app.menus.prefs_tab = PrefsTab::Extensions;
    let h = harness(app);
    assert!(h.query_by_label("No extensions installed").is_some());
    assert!(h.query_by_label("Add Extension").is_some());
    assert!(h.query_by_label("Open Folder").is_some());
}

#[test]
fn installing_a_package_lists_it_and_opens_the_tab() {
    let s = Support::new("install");
    let pkg = write_package(&s.dir);
    let mut app = PixmithApp::default();
    // Opening a package (File > Open, drop, command line) goes through open_path and installs it.
    app.open_path(&pkg);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(app.menus.prefs_open && app.menus.prefs_tab == PrefsTab::Extensions);
    assert!(
        s.dir
            .join(if cfg!(any(target_os = "macos", windows)) { "Pixmith" } else { "pixmith" })
            .join("Extensions/sample/package.json")
            .is_file()
    );
    let h = harness(app);
    assert!(h.query_by_label("Sample Pack").is_some(), "the extension is listed");
    assert!(h.query_by_label("Enabled").is_some());
    assert!(h.query_by_label("Uninstall\u{2026}").is_some());
}

#[test]
fn a_bad_package_shows_an_error_and_installs_nothing() {
    let s = Support::new("bad");
    let path = s.dir.join("evil.pixmith-extension");
    std::fs::write(
        &path,
        zip::build(&[("package.json", br#"{"name":"x","version":"1"}"#), ("../../escape.lua", b"x")]),
    )
    .unwrap();
    let mut app = PixmithApp::default();
    app.open_path(&path);
    let message = app.error.clone().expect("an error is shown");
    assert!(message.starts_with("Could not install the extension"), "{message}");
    assert!(extensions::installed().is_empty());
    assert!(!s.dir.join("escape.lua").exists());
}

#[test]
fn enable_disable_and_uninstall_from_the_tab() {
    let s = Support::new("toggle");
    let pkg = write_package(&s.dir);
    let mut app = PixmithApp::default();
    app.install_extension(&pkg);
    let mut h = harness(app);

    h.get_by_label("Enabled").click();
    h.run();
    assert!(!extensions::installed()[0].enabled, "unchecking Enabled disables it");
    assert!(extensions::themes().is_empty() && extensions::palettes().is_empty());
    h.get_by_label("Enabled").click();
    h.run();
    assert!(extensions::installed()[0].enabled);
    assert_eq!(extensions::themes().len(), 1);

    h.get_by_label("Uninstall\u{2026}").click();
    h.run();
    assert!(h.query_by_label("Remove Sample Pack?").is_some(), "asks first");
    h.get_by_label("Cancel").click();
    h.run();
    assert_eq!(extensions::installed().len(), 1);

    h.get_by_label("Uninstall\u{2026}").click();
    h.run();
    h.get_by_label("Uninstall").click();
    h.run();
    assert!(extensions::installed().is_empty());
    assert!(h.query_by_label("No extensions installed").is_some());
}

#[test]
fn contributed_palettes_join_the_presets() {
    let s = Support::new("palettes");
    let pkg = write_package(&s.dir);
    let before = pixmith_desktop::doc::Doc::palette_preset_names();
    let mut app = PixmithApp::default();
    app.install_extension(&pkg);
    let names = pixmith_desktop::doc::Doc::palette_preset_names();
    assert_eq!(names.len(), before.len() + 1);
    assert_eq!(names.last().unwrap(), "Sunset (Sample Pack)");
    let last = (names.len() - 1) as u32;
    app.tab_mut().doc.load_palette_preset(last).unwrap();
    assert_eq!(app.tab().doc.palette().len(), 3, "the contributed palette replaced the document's");
}

#[test]
fn a_theme_overrides_the_design_tokens_and_goes_away_again() {
    let s = Support::new("theme");
    let mut app = PixmithApp::default();
    app.install_extension(&write_package(&s.dir));
    let default_panel = theme::Colors::builtin_dark().panel;
    let choices = theme_choices();
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].name, "Midnight");
    assert!(choices[0].dark && choices[0].light);

    assert!(apply_theme(&choices[0].id));
    let c = theme::Colors::dark();
    assert_eq!(c.panel, egui::Color32::from_rgb(0x14, 0x14, 0x1F));
    assert_eq!(c.accent, egui::Color32::from_rgb(0xFF, 0x6A, 0xC1));
    assert_eq!(c.text, egui::Color32::from_rgb(0xED, 0xED, 0xF7));
    assert_eq!(c.hover, theme::Colors::builtin_dark().hover, "tokens the theme leaves out keep their value");
    assert_ne!(c.selected, theme::Colors::builtin_dark().selected, "selected fills follow the accent");
    assert_eq!(theme::Colors::light().panel, egui::Color32::from_rgb(0xF4, 0xF1, 0xFA));

    // The theme's extension is disabled: back to the default look; the choice itself is kept.
    extensions::set_enabled("sample", false).unwrap();
    assert!(!apply_theme(&choices[0].id));
    assert_eq!(theme::Colors::dark().panel, default_panel);
    assert!(theme_choices().is_empty());
    assert!(!apply_theme(""));
}

#[test]
fn the_whole_window_follows_the_theme() {
    let s = Support::new("window");
    let mut app = PixmithApp::default();
    app.install_extension(&write_package(&s.dir));
    app.menus.prefs_open = false;
    app.menus.prefs.theme = theme_choices()[0].id.clone();
    let mut h = harness(app);
    let ctx = h.ctx.clone();
    theme::apply(&ctx, true);
    assert!(apply_theme(&h.state().menus.prefs.theme.clone()));
    theme::apply(&ctx, true);
    h.run();
    assert_eq!(ctx.style_of(egui::Theme::Dark).visuals.panel_fill, egui::Color32::from_rgb(0x14, 0x14, 0x1F));
}

#[test]
fn the_chosen_theme_is_saved_with_the_preferences() {
    let p = Prefs { theme: "sample/t.json".into(), appearance: 2, ..Default::default() };
    assert_eq!(Prefs::decode(&p.encode()), p);
    assert!(!Prefs::default().encode().contains("theme="));
    // A theme id never breaks the numeric keys after it.
    let back = Prefs::decode("theme=a/b.json\nnewWidth=64\n");
    assert_eq!((back.theme.as_str(), back.new_width), ("a/b.json", 64));
}

#[test]
fn a_package_dropped_on_the_window_installs() {
    let s = Support::new("drop");
    let pkg = write_package(&s.dir);
    let mut h = harness(PixmithApp::default());
    h.input_mut().dropped_files.push(std::sync::Arc::new(Dropped(pkg)));
    h.run();
    h.run();
    assert_eq!(extensions::installed().len(), 1, "dropping a package installs it");
}

/// The scripts of an enabled extension load as plugins and their commands reach File > Scripts; disabling the
/// extension unloads them.
#[test]
fn an_extension_plugin_registers_its_command() {
    let s = Support::new("plugin");
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/extension-template");
    let names = ["package.json", "scripts/sample.lua", "palettes/sunset.gpl", "themes/midnight.json"];
    let files: Vec<(&str, Vec<u8>)> = names.iter().map(|n| (*n, std::fs::read(base.join(n)).unwrap())).collect();
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, d)| (*n, d.as_slice())).collect();
    let pkg = s.dir.join("t.pixmith-extension");
    std::fs::write(&pkg, zip::build(&refs)).unwrap();
    let mut app = PixmithApp::default();
    app.extensions.plugins.enabled = true;
    app.install_extension(&pkg);
    let mut h = harness(app);
    let wait = |h: &mut Harness<'static, PixmithApp>, what: &str, done: &dyn Fn(&PixmithApp) -> bool| {
        for _ in 0..300 {
            h.step();
            if done(h.state()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let a = h.state();
        panic!(
            "timed out waiting for {what}: error {:?}, sessions {}, plugins {}",
            a.error,
            a.gamedev.lua.sessions().count(),
            a.extensions.plugins.session_count()
        );
    };
    wait(&mut h, "the plugin command", &|a| a.lua_command(0).is_some());
    assert_eq!(h.state().lua_command(0).unwrap().title, "Sample: Diagonal Dots...");
    extensions::set_enabled("sample-extension", false).unwrap();
    wait(&mut h, "the command to go", &|a| a.lua_command(0).is_none());
}
