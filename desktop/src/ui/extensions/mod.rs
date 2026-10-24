//! Extensions (`App/Extensions.swift`, `App/Themes.swift`): the Extensions tab of Preferences, installing a
//! `.pixmith-extension` package (Add Extension..., File > Open or a file dropped on the window), and the color
//! theme picker of the Appearance tab.
//!
//! Everything about packages (validation, unpacking, enable state) lives in `pixmith_core::extensions`; this
//! module is the window around it. Themes override the design tokens of `theme.rs` through
//! `theme::set_theme_override`.

mod plugins;
mod tab;

pub use tab::{tab, theme_row};

use crate::app::PixmithApp;
use crate::theme;
use eframe::egui;
use pixmith_core::extensions as core;
use std::path::Path;

/// UI state of the Extensions tab.
#[derive(Default)]
pub struct ExtensionsState {
    /// Extension whose Uninstall is waiting for confirmation.
    pub confirm: Option<String>,
    /// A line under the list: the result of the last install, uninstall or switch (text, is an error).
    pub message: Option<(String, bool)>,
    /// `core::generation()` last seen, to notice installs and removals made anywhere.
    seen: u64,
    /// Sessions of the scripts that enabled extensions contribute.
    pub plugins: plugins::Plugins,
}

/// Name and availability of one entry of the theme picker.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeChoice {
    pub id: String,
    pub name: String,
    pub extension: String,
    pub dark: bool,
    pub light: bool,
}

/// The themes of the enabled extensions.
pub fn theme_choices() -> Vec<ThemeChoice> {
    core::installed()
        .iter()
        .filter(|e| e.active())
        .flat_map(|e| {
            e.themes.iter().map(|t| ThemeChoice {
                id: t.id.clone(),
                name: t.theme.name.clone(),
                extension: e.manifest.display_name.clone(),
                dark: t.theme.styles_dark(),
                light: t.theme.styles_light(),
            })
        })
        .collect()
}

/// Makes theme `id` (empty: the default look) the active one. A theme that is not installed or not enabled
/// falls back to the default without forgetting the choice. Returns whether a theme is in effect.
pub fn apply_theme(id: &str) -> bool {
    match (!id.is_empty()).then(|| core::theme(id)).flatten() {
        Some(t) => {
            theme::set_theme_override(&t.theme.dark, &t.theme.light);
            true
        }
        None => {
            theme::clear_theme_override();
            false
        }
    }
}

impl PixmithApp {
    /// Installs the package at `path` (Add Extension..., opening or dropping a `.pixmith-extension`): shows
    /// Preferences > Extensions with the result, or the error.
    pub fn install_extension(&mut self, path: &Path) {
        match core::install(path) {
            Ok(ext) => {
                let verb = if ext.error.is_some() { "Installed with problems" } else { "Installed" };
                self.extensions.message =
                    Some((format!("{verb}: {} {}", ext.manifest.display_name, ext.manifest.version), false));
                self.menus.prefs_open = true;
                self.menus.prefs_tab = crate::ui::menus::PrefsTab::Extensions;
            }
            Err(e) => {
                self.error = Some(format!("Could not install the extension: {e}"));
            }
        }
    }

    /// Add Extension...: picks a package.
    pub fn add_extension_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("Pixmith extension", &[core::PACKAGE_EXTENSION])
            .set_title("Add Extension")
            .pick_file();
        if let Some(path) = picked {
            self.install_extension(&path);
        }
    }

    /// Opens the folder that holds the installed extensions in the system file manager.
    pub fn open_extensions_folder(&self) {
        let folder = core::extensions_dir();
        let _ = std::fs::create_dir_all(&folder);
        let program = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(windows) {
            "explorer"
        } else {
            "xdg-open"
        };
        let _ = std::process::Command::new(program).arg(folder).spawn();
    }
}

/// Per-frame work: when the set of extensions changed, the active theme is looked up again (a theme whose
/// extension was disabled or uninstalled drops back to the default look; one that came back returns).
pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    plugins::sync(app);
    let generation = core::generation();
    if app.extensions.seen != generation {
        app.extensions.seen = generation;
        apply_theme(&app.menus.prefs.theme);
        theme::apply(ctx, app.dark);
    }
}
