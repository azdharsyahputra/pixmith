//! The menu bar and the app-level dialogs and panels: FX filters, Color Curves, canvas size, cel properties, the
//! History panel, Preferences with rebindable shortcuts, the shortcuts list and About.
//!
//! `bar.rs` and `commands.rs` mirror the macOS menus (`PixmithApp.swift` and every `CommandMenu` in `app/`). Entries
//! owned by features that are built elsewhere call the stubs in `hooks.rs`.

pub mod bar;
pub mod commands;
mod controls;
pub mod filters;
mod help;
mod history;
pub mod hooks;
pub mod prefs;
mod sheets;
pub mod shortcuts;
pub mod spritedata;

use crate::app::{DocTab, PixmithApp};
use eframe::Storage;
use eframe::egui;
use std::collections::HashSet;
use std::path::PathBuf;

pub use filters::FilterKind;
pub use prefs::{Prefs, PrefsTab};
pub use shortcuts::{Action, KeyBinding, Shortcuts};

/// How many files Open Recent remembers.
const MAX_RECENTS: usize = 10;

/// State of the menus and their dialogs. One lives in the app; it is saved with eframe's storage.
pub struct MenusState {
    pub prefs: Prefs,
    pub prefs_open: bool,
    pub prefs_tab: PrefsTab,
    pub shortcuts: Shortcuts,
    pub history_open: bool,
    pub(super) history: history::HistoryCache,
    pub help_open: bool,
    pub(super) help_filter: String,
    pub about_open: bool,
    /// Help > Command Line Tool…
    pub cli_open: bool,
    pub filter: Option<filters::FilterSession>,
    pub scale: Option<filters::ScaleSession>,
    pub canvas_size: Option<sheets::CanvasSizeSession>,
    pub cel: Option<sheets::CelSession>,
    /// Sprite > Sprite Properties… is open.
    pub sprite_props: bool,
    /// Files opened or saved, newest first (`RecentFiles`).
    pub recents: Vec<PathBuf>,
    seen: HashSet<PathBuf>,
    /// The new-sprite size last copied between the preferences and `app.files.new_size`.
    pub synced_new_size: (u32, u32),
}

impl Default for MenusState {
    fn default() -> MenusState {
        let prefs = Prefs::default();
        MenusState {
            synced_new_size: (prefs.new_width, prefs.new_height),
            prefs,
            prefs_open: false,
            prefs_tab: PrefsTab::General,
            shortcuts: Shortcuts::default(),
            history_open: false,
            history: Default::default(),
            help_open: false,
            help_filter: String::new(),
            about_open: false,
            cli_open: false,
            filter: None,
            scale: None,
            canvas_size: None,
            cel: None,
            sprite_props: false,
            recents: Vec::new(),
            seen: HashSet::new(),
        }
    }
}

const PREFS_KEY: &str = "pixmith.prefs";
const SHORTCUTS_KEY: &str = "pixmith.shortcuts";
const RECENTS_KEY: &str = "pixmith.recents";

impl MenusState {
    /// Loads what an earlier run saved.
    pub fn load(storage: Option<&dyn Storage>) -> MenusState {
        let mut s = MenusState::default();
        let Some(storage) = storage else { return s };
        if let Some(text) = storage.get_string(PREFS_KEY) {
            s.prefs = Prefs::decode(&text);
            s.synced_new_size = (s.prefs.new_width, s.prefs.new_height);
        }
        if let Some(text) = storage.get_string(SHORTCUTS_KEY) {
            s.shortcuts = Shortcuts::decode(&text);
        }
        if let Some(text) = storage.get_string(RECENTS_KEY) {
            s.recents = text.lines().filter(|l| !l.is_empty()).map(PathBuf::from).take(MAX_RECENTS).collect();
            s.seen = s.recents.iter().cloned().collect();
        }
        s
    }

    /// Saves preferences, shortcuts and recent files.
    pub fn save(&self, storage: &mut dyn Storage) {
        storage.set_string(PREFS_KEY, self.prefs.encode());
        storage.set_string(SHORTCUTS_KEY, self.shortcuts.encode());
        let recents: Vec<String> = self.recents.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        storage.set_string(RECENTS_KEY, recents.join("\n"));
    }

    /// The recent files that still exist.
    pub fn recents_existing(&self) -> Vec<PathBuf> {
        self.recents.iter().filter(|p| p.exists()).cloned().collect()
    }

    pub fn clear_recents(&mut self) {
        self.recents.clear();
    }

    /// Remembers the files of the open tabs that were not seen before.
    fn track_recents(&mut self, tabs: &[DocTab]) {
        for path in tabs.iter().filter_map(|t| t.doc.path.as_ref()) {
            if self.seen.insert(path.clone()) {
                self.recents.retain(|p| p != path);
                self.recents.insert(0, path.clone());
                self.recents.truncate(MAX_RECENTS);
            }
        }
    }

    pub fn open_preferences(&mut self) {
        self.prefs_open = true;
    }

    pub fn open_canvas_size(&mut self, width: u32, height: u32) {
        self.canvas_size = Some(sheets::CanvasSizeSession::new(width, height));
    }

    pub fn open_cel_sheet(&mut self, tab: &DocTab) {
        self.cel = Some(sheets::CelSession::new(tab));
    }

    pub fn open_filter(&mut self, tab: &mut DocTab, kind: FilterKind, primary: u32) {
        filters::open(self, tab, primary, kind);
    }

    /// One of this module's modal sheets is open.
    fn sheet_open(&self) -> bool {
        self.filter.is_some()
            || self.scale.is_some()
            || self.canvas_size.is_some()
            || self.cel.is_some()
            || self.sprite_props
            || self.about_open
            || self.cli_open
    }
}

/// A modal dialog is open, so menu and key shortcuts wait (`SheetState.isOpen`).
pub fn blocked(app: &PixmithApp) -> bool {
    app.error.is_some()
        || app.files.prompt.is_some()
        || app.files.new_sheet.is_some()
        || app.files.export.is_some()
        || app.projects.has_sheet()
        || app.gamedev.lua.modal_open()
        || app.menus.sheet_open()
}

/// Copies the preferences that the canvas draws with into its state.
pub fn apply_prefs(app: &mut PixmithApp) {
    let p = &app.menus.prefs;
    let custom = p.checker_light.is_some() || p.checker_dark.is_some();
    app.canvas.checker = custom.then(|| p.checker(app.dark));
    app.canvas.grid = Some(prefs::grid_color32(p));
    app.canvas.default_zoom = Some(p.default_zoom);
}

/// Applies the saved appearance to a freshly created app.
pub fn apply_appearance(app: &mut PixmithApp, ctx: &egui::Context) {
    let system_dark = ctx.input(|i| i.raw.system_theme) != Some(egui::Theme::Light);
    app.dark = app.menus.prefs.wants_dark(system_dark);
    crate::ui::extensions::apply_theme(&app.menus.prefs.theme);
    crate::theme::apply(ctx, app.dark);
    app.files.new_size = (app.menus.prefs.new_width, app.menus.prefs.new_height);
    app.menus.synced_new_size = app.files.new_size;
    apply_prefs(app);
}

/// Runs the menu shortcuts. Called before anything else reads keys, so that Cmd+Shift+N is New Layer and not
/// the New Sprite shortcut that `consume_key` would also match.
pub fn shortcuts_first(app: &mut PixmithApp, ctx: &egui::Context) {
    if !blocked(app) {
        commands::handle_shortcuts(app, ctx);
    }
}

/// Per-frame work of the menus: dialogs and panels. Called after the window is laid out.
pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    app.menus.track_recents(&app.tabs);
    // A sprite created from the New Sprite sheet becomes the new default size, like `Prefs.newWidth`.
    if app.files.new_size != app.menus.synced_new_size {
        app.menus.synced_new_size = app.files.new_size;
        (app.menus.prefs.new_width, app.menus.prefs.new_height) = app.files.new_size;
    }
    apply_prefs(app);
    hooks::sheet_import_dialog(app, ctx);
    filters::show(app, ctx);
    if let Some(mut s) = app.menus.canvas_size.take()
        && !sheets::canvas_size(app, ctx, &mut s)
    {
        app.menus.canvas_size = Some(s);
    }
    if let Some(mut s) = app.menus.cel.take()
        && !sheets::cel(app, ctx, &mut s)
    {
        app.menus.cel = Some(s);
    }
    if app.menus.sprite_props && spritedata::sprite_properties(app, ctx) {
        app.menus.sprite_props = false;
    }
    if app.menus.about_open && sheets::about(ctx) {
        app.menus.about_open = false;
    }
    if app.menus.cli_open && sheets::command_line_tool(ctx) {
        app.menus.cli_open = false;
    }
    history::show(app, ctx);
    prefs::show(app, ctx);
    help::show(app, ctx);
}
