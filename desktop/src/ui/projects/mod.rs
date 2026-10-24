//! Projects: the Home screen (recent canvases and projects), the project view (a grid of its canvases) and the
//! project switcher in the editor's title row. Mirrors the Swift app's Home and project views.
//!
//! The window shows one [`Screen`]: the editor (tabs, canvas, panels) or Home or one project. Opening a canvas
//! (double-click, Enter, a file dropped on the window, File > Open, a path on the command line) switches to the
//! editor. File > Home (Ctrl+Shift+H) and the switcher's "Back to Home" switch back; the tabs stay open.
//!
//! Folder work goes through `calls::projects`. Thumbnails and scans load on background threads (`jobs.rs`) and
//! only the visible rows of a grid are laid out, so a project with hundreds of canvases stays smooth.

pub mod cards;
pub mod home;
pub mod jobs;
pub mod project_view;
pub mod sheets;
pub mod switcher;

use crate::app::PixmithApp;
use crate::calls::projects as calls;
use eframe::egui;
use jobs::{Scans, ThumbKey, Thumbs};
use std::path::{Path, PathBuf};

/// What the window shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    Editor,
    Home,
    Project(PathBuf),
}

/// The Home sidebar's two lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Recent,
    Projects,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Modified,
    Name,
}

impl Sort {
    pub const LABELS: [&'static str; 2] = ["Sort: Modified", "Sort: Name"];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Sort {
        if i == 1 { Sort::Name } else { Sort::Modified }
    }
}

/// A project the user has created or opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentProject {
    pub path: PathBuf,
    /// Seconds since the Unix epoch.
    pub opened: u64,
}

/// A recently opened canvas file, with the facts the strip needs.
#[derive(Clone, Debug)]
pub struct RecentCanvas {
    pub path: PathBuf,
    pub key: ThumbKey,
    pub modified: u64,
}

/// How many projects Home remembers.
pub const MAX_RECENT_PROJECTS: usize = 100;
/// How many recent canvases the strip shows.
pub const MAX_RECENT_CANVASES: usize = 12;

const STORAGE_KEY: &str = "pixmith.projects";

pub struct ProjectsState {
    pub screen: Screen,
    pub nav: Nav,
    pub sort: Sort,
    pub search: String,
    /// Projects, newest opened first.
    pub recent: Vec<RecentProject>,
    /// Where New Project puts its folder: the last location used.
    pub location: Option<PathBuf>,
    pub scans: Scans,
    pub thumbs: Thumbs,
    /// The strip on Home, refreshed whenever Home opens.
    pub recent_canvases: Vec<RecentCanvas>,
    pub view: project_view::ViewState,
    pub new_project: Option<sheets::NewProject>,
    pub new_canvas: Option<sheets::NewCanvas>,
    pub switcher: switcher::SwitcherState,
    pub(crate) focused: bool,
}

impl Default for ProjectsState {
    fn default() -> ProjectsState {
        ProjectsState {
            screen: Screen::Editor,
            nav: Nav::Recent,
            sort: Sort::Modified,
            search: String::new(),
            recent: Vec::new(),
            location: None,
            scans: Scans::default(),
            thumbs: Thumbs::default(),
            recent_canvases: Vec::new(),
            view: Default::default(),
            new_project: None,
            new_canvas: None,
            switcher: Default::default(),
            focused: true,
        }
    }
}

impl ProjectsState {
    /// Loads what an earlier run saved (recent projects, the New Project location, the list settings).
    pub fn load(storage: Option<&dyn eframe::Storage>) -> ProjectsState {
        let mut s = ProjectsState::default();
        if let Some(text) = storage.and_then(|st| st.get_string(STORAGE_KEY)) {
            s.decode(&text);
        }
        s
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        storage.set_string(STORAGE_KEY, self.encode());
    }

    pub fn encode(&self) -> String {
        let mut out = String::new();
        for r in &self.recent {
            out.push_str(&format!("recent\t{}\t{}\n", r.opened, r.path.display()));
        }
        if let Some(l) = &self.location {
            out.push_str(&format!("location\t{}\n", l.display()));
        }
        out.push_str(&format!("sort\t{}\n", if self.sort == Sort::Name { "name" } else { "modified" }));
        out.push_str(&format!("nav\t{}\n", if self.nav == Nav::Projects { "projects" } else { "recent" }));
        out
    }

    pub fn decode(&mut self, text: &str) {
        for line in text.lines() {
            let mut parts = line.splitn(3, '\t');
            match (parts.next(), parts.next(), parts.next()) {
                (Some("recent"), Some(opened), Some(path)) if !path.is_empty() => {
                    if self.recent.len() < MAX_RECENT_PROJECTS {
                        self.recent
                            .push(RecentProject { path: PathBuf::from(path), opened: opened.parse().unwrap_or(0) });
                    }
                }
                (Some("location"), Some(path), None) if !path.is_empty() => self.location = Some(PathBuf::from(path)),
                (Some("sort"), Some(v), None) => self.sort = if v == "name" { Sort::Name } else { Sort::Modified },
                (Some("nav"), Some(v), None) => self.nav = if v == "projects" { Nav::Projects } else { Nav::Recent },
                _ => {}
            }
        }
    }

    /// A scan is running or about to start (for tests and progress).
    pub fn scans_loading(&self) -> bool {
        self.scans.any_loading() || self.view.needs_scan.is_some() || self.view.needs_scan_all
    }

    /// The folder of the project view, if it is showing.
    pub fn folder(&self) -> Option<&Path> {
        match &self.screen {
            Screen::Project(f) => Some(f),
            _ => None,
        }
    }

    /// The editor is showing.
    pub fn is_editor(&self) -> bool {
        self.screen == Screen::Editor
    }

    /// Puts `folder` first in the recent projects.
    pub fn remember_project(&mut self, folder: &Path) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        self.recent.retain(|r| r.path != folder);
        self.recent.insert(0, RecentProject { path: folder.to_path_buf(), opened: now });
        self.recent.truncate(MAX_RECENT_PROJECTS);
    }

    pub fn forget_project(&mut self, folder: &Path) {
        self.recent.retain(|r| r.path != folder);
    }
}

/// The folder of the project a document lives in, if it does.
pub fn project_folder_of(file: &Path) -> Option<PathBuf> {
    let parent = file.parent()?;
    calls::is_project(parent).then(|| parent.to_path_buf())
}

impl PixmithApp {
    // --- Switching screens ---------------------------------------------------------------------

    /// File > Home: the Home screen (the open tabs stay).
    pub fn go_home(&mut self) {
        self.projects.screen = Screen::Home;
        self.projects.view.reset();
        self.projects.focused = true;
        self.refresh_screen_data();
    }

    /// Back to the editor (a tab was clicked, or a document was opened).
    pub fn show_editor(&mut self) {
        self.projects.screen = Screen::Editor;
    }

    /// Reloads what the current screen lists. The scans run in the background.
    pub fn refresh_screen_data(&mut self) {
        match self.projects.screen.clone() {
            Screen::Home => {
                self.projects.recent_canvases = self.recent_canvases();
                self.projects.view.needs_scan_all = true;
            }
            Screen::Project(folder) => {
                self.projects.view.needs_scan = Some(folder);
            }
            Screen::Editor => {}
        }
    }

    /// The canvas files among File > Open Recent, newest first, that still exist.
    fn recent_canvases(&self) -> Vec<RecentCanvas> {
        self.menus
            .recents_existing()
            .into_iter()
            .filter(|p| calls::is_canvas_file(p))
            .take(MAX_RECENT_CANVASES)
            .map(|path| RecentCanvas { key: ThumbKey::of(&path), modified: calls::modified_secs(&path), path })
            .collect()
    }

    // --- Projects ------------------------------------------------------------------------------

    /// Shows project `folder`. A folder that is not a project yet becomes one, named after the folder.
    pub fn open_project(&mut self, folder: &Path) {
        if !folder.is_dir() {
            self.error = Some(format!("\u{201C}{}\u{201D} could not be found.", folder.display()));
            self.projects.forget_project(folder);
            return;
        }
        if !calls::is_project(folder) {
            let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if let Err(e) = calls::create_project(folder, &name) {
                self.error = Some(e);
                return;
            }
        }
        self.projects.remember_project(folder);
        self.projects.screen = Screen::Project(folder.to_path_buf());
        self.projects.view.reset();
        self.projects.focused = true;
        self.refresh_screen_data();
    }

    /// Creates the folder `parent/name` as a project and shows it.
    pub fn create_project(&mut self, parent: &Path, name: &str) -> bool {
        let name = name.trim();
        let folder = parent.join(name);
        match calls::create_project(&folder, name) {
            Ok(()) => {
                self.projects.location = Some(parent.to_path_buf());
                self.open_project(&folder);
                true
            }
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }

    /// Open Project: pick a folder.
    pub fn open_project_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Open Project");
        if let Some(dir) = self.projects.location.as_ref().filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(folder) = dialog.pick_folder() {
            self.open_project(&folder);
        }
    }

    /// Opens a canvas file in a tab and shows the editor.
    pub fn open_canvas(&mut self, path: &Path) {
        self.open_path(path);
    }

    /// Paths from the command line: folders open as projects, files as documents. Skips Home when it opens a
    /// document.
    pub fn open_args<I: IntoIterator<Item = PathBuf>>(&mut self, args: I) {
        for path in args {
            self.open_path(&path);
        }
    }

    /// File > Export As Aseprite: the document of the active tab, written as `.aseprite` under a name asked
    /// for. The tab keeps its own file and format.
    pub fn export_as_aseprite(&mut self) {
        let tab = &self.tabs[self.active];
        let stem = tab.doc.path.as_ref().and_then(|p| p.file_stem()).map_or_else(
            || tab.doc.name.clone().unwrap_or_else(|| "Untitled".into()),
            |s| s.to_string_lossy().into_owned(),
        );
        let mut dialog = rfd::FileDialog::new()
            .add_filter("Aseprite", &["aseprite", "ase"])
            .set_file_name(format!("{stem}.aseprite"));
        if let Some(dir) = tab.doc.path.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(mut path) = dialog.save_file() else { return };
        if path.extension().is_none() {
            path.set_extension("aseprite");
        }
        if let Err(e) = self.tabs[self.active].doc.save_aseprite(&path) {
            self.error = Some(e);
        }
    }

    /// Writes a canvas file of a project as `.aseprite` without opening it in a tab.
    pub fn export_canvas_as_aseprite(&mut self, canvas: &Path) {
        let stem = canvas.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut dialog = rfd::FileDialog::new()
            .add_filter("Aseprite", &["aseprite", "ase"])
            .set_file_name(format!("{stem}.aseprite"));
        if let Some(dir) = canvas.parent() {
            dialog = dialog.set_directory(dir);
        }
        let Some(mut target) = dialog.save_file() else { return };
        if target.extension().is_none() {
            target.set_extension("aseprite");
        }
        let result = crate::doc::Doc::open(canvas).and_then(|doc| doc.save_aseprite(&target));
        if let Err(e) = result {
            self.error = Some(e);
        }
    }
}

/// Shows a file or folder in the system file manager.
pub fn reveal(path: &Path) {
    let mut command = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg("-R").arg(path);
        c
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("explorer");
        c.arg(format!("/select,{}", path.display()));
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(if path.is_dir() { path } else { path.parent().unwrap_or(path) });
        c
    };
    let _ = command.spawn();
}

/// Per-frame work for every screen: collects finished thumbnails and scans and reloads a screen when the
/// window regains focus (files may have changed meanwhile).
pub fn tick(app: &mut PixmithApp, ctx: &egui::Context) {
    app.projects.thumbs.poll(ctx);
    if app.projects.scans.poll() {
        project_view::scans_arrived(app);
    }
    if app.projects.is_editor() {
        return;
    }
    let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
    if focused && !app.projects.focused {
        app.refresh_screen_data();
    }
    app.projects.focused = focused;
    if let Some(folder) = app.projects.view.needs_scan.take() {
        app.projects.scans.request(ctx, &folder);
    }
    if std::mem::take(&mut app.projects.view.needs_scan_all) {
        let folders: Vec<PathBuf> = app.projects.recent.iter().map(|r| r.path.clone()).collect();
        for f in folders {
            app.projects.scans.request(ctx, &f);
        }
    }
}

/// Lays out the Home screen or the project view: the sidebar and the content beside it. Called with the root
/// `Ui` in place of the editor's panels.
pub fn show_screen(app: &mut PixmithApp, root: &mut egui::Ui) {
    let c = app.colors();
    egui::Panel::left("projects_sidebar")
        .exact_size(home::SIDEBAR_WIDTH)
        .resizable(false)
        .frame(egui::Frame::new().fill(c.panel))
        .show(root, |ui| home::sidebar(app, ui));
    egui::CentralPanel::default().frame(egui::Frame::new().fill(c.workspace)).show(root, |ui| {
        match app.projects.screen.clone() {
            Screen::Project(folder) => project_view::show(app, ui, &folder),
            _ => home::show(app, ui),
        }
    });
    sheets::show(app, root.ctx());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip() {
        let s = ProjectsState {
            recent: vec![
                RecentProject { path: PathBuf::from("/work/Game One"), opened: 30 },
                RecentProject { path: PathBuf::from("/work/two"), opened: 20 },
            ],
            location: Some(PathBuf::from("/work")),
            sort: Sort::Name,
            nav: Nav::Projects,
            ..ProjectsState::default()
        };
        let mut back = ProjectsState::default();
        back.decode(&s.encode());
        assert_eq!(back.recent, s.recent);
        assert_eq!(back.location, s.location);
        assert_eq!((back.sort, back.nav), (Sort::Name, Nav::Projects));
        back.decode("garbage\nrecent\tx\n");
        assert_eq!(back.recent.len(), 2);
    }

    #[test]
    fn recent_projects_are_unique_newest_first_and_limited() {
        let mut s = ProjectsState::default();
        for i in 0..(MAX_RECENT_PROJECTS + 5) {
            s.remember_project(Path::new(&format!("/p/{i}")));
        }
        s.remember_project(Path::new("/p/7"));
        assert_eq!(s.recent.len(), MAX_RECENT_PROJECTS);
        assert_eq!(s.recent[0].path, Path::new("/p/7"));
        assert_eq!(s.recent.iter().filter(|r| r.path == Path::new("/p/7")).count(), 1);
        s.forget_project(Path::new("/p/7"));
        assert!(s.recent.iter().all(|r| r.path != Path::new("/p/7")));
    }
}
