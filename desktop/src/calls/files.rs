//! Core calls used by the files code (`Doc` methods, the only `unsafe` here) and the file actions of the
//! window: new, open, save, close with the unsaved-changes question, quit, export and dropped files.
//! Mirrors `Documents.swift` and `FileActions.swift`.

use crate::app::{DocTab, PixmithApp};
use crate::doc::{Doc, last_error};
use crate::ui::dialogs::{After, Answer, ExportKind, ExportSheet, Prompt, SCALES};
use eframe::egui;
use pixmith_core::ffi::document as core;
use pixmith_core::formats::{animation, gif, import, sheet};
use pixmith_core::history;
use std::ffi::CString;
use std::path::{Path, PathBuf};

fn c_path(path: &Path) -> Result<CString, String> {
    CString::new(path.to_string_lossy().as_bytes()).map_err(|_| "path contains a NUL byte".to_string())
}

fn check(ok: bool) -> Result<(), String> {
    if ok { Ok(()) } else { Err(last_error()) }
}

impl Doc {
    /// Opens a PNG as a new one-layer sprite (`pixmith_import_png`).
    pub fn import_png(path: &Path) -> Result<Doc, String> {
        let c = c_path(path)?;
        // SAFETY: `c` is a valid C string for the duration of the call.
        let ptr = unsafe { import::pixmith_import_png(c.as_ptr()) };
        if ptr.is_null() {
            return Err(last_error());
        }
        // SAFETY: the core just allocated `ptr` and nothing else owns it.
        Ok(unsafe { Doc::from_raw(ptr, None) })
    }

    /// Records the current state as the saved one, so undoing back to it is clean again.
    pub fn mark_saved(&mut self) {
        // SAFETY: `self.raw()` is a live document.
        unsafe { history::pixmith_mark_saved(self.raw()) }
    }

    /// Applied undo steps and undone redo steps.
    pub fn history_counts(&self) -> (u32, u32) {
        let (mut undo, mut redo) = (0, 0);
        // SAFETY: `self.raw()` is live and both out pointers are valid.
        unsafe { history::pixmith_history_counts(self.raw(), &mut undo, &mut redo) };
        (undo, redo)
    }

    /// Saves (`.pxmt` unless the path says `.aseprite`) and records that state as saved.
    pub fn save_marked(&mut self, path: &Path) -> Result<(), String> {
        self.save(path)?;
        self.mark_saved();
        Ok(())
    }

    /// One frame as a PNG at `scale`.
    pub fn export_png(&self, frame: u32, scale: u32, path: &Path) -> Result<(), String> {
        let c = c_path(path)?;
        // SAFETY: `self.raw()` is live; `c` outlives the call.
        check(unsafe { core::pixmith_export_png(self.raw(), frame, scale, c.as_ptr()) })
    }

    /// A sprite sheet PNG. `layout`: 0 horizontal, 1 vertical, 2 grid with `columns` columns.
    pub fn export_sheet(
        &self,
        layout: u32,
        columns: u32,
        padding: u32,
        scale: u32,
        json: bool,
        path: &Path,
    ) -> Result<(), String> {
        let c = c_path(path)?;
        // SAFETY: as above.
        check(unsafe { sheet::pixmith_export_sheet_ex(self.raw(), layout, columns, padding, scale, json, c.as_ptr()) })
    }

    pub fn export_gif(&self, scale: u32, path: &Path) -> Result<(), String> {
        let c = c_path(path)?;
        // SAFETY: as above.
        check(unsafe { gif::pixmith_export_gif(self.raw(), scale, c.as_ptr()) })
    }

    pub fn export_apng(&self, scale: u32, path: &Path) -> Result<(), String> {
        let c = c_path(path)?;
        // SAFETY: as above.
        check(unsafe { animation::pixmith_export_apng(self.raw(), scale, c.as_ptr()) })
    }

    /// Frames as `<base>-001.png`, `<base>-002.png`, ...
    pub fn export_png_sequence(&self, scale: u32, base: &Path) -> Result<(), String> {
        let c = c_path(base)?;
        // SAFETY: as above.
        check(unsafe { gif::pixmith_export_png_sequence(self.raw(), scale, c.as_ptr()) })
    }
}

/// Opened by `Prefs.initialZoom`: as large as fits in 512 points, at most the default 800%.
fn initial_zoom(width: u32, height: u32) -> f32 {
    let fit = (512 / width.max(height).max(1)) as f32;
    fit.clamp(1.0, 8.0)
}

/// Two paths name the same file (symlinks resolved when the file exists).
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

impl PixmithApp {
    // --- Names ---------------------------------------------------------------------------------

    /// Gives every file-less tab its "Untitled N" number; call once per frame.
    pub fn sync_untitled(&mut self) {
        let keys: Vec<(usize, bool)> = self.tabs.iter().map(|t| (t.doc.raw() as usize, t.doc.path.is_none())).collect();
        self.files.untitled.retain(|(k, _)| keys.iter().any(|(key, untitled)| key == k && *untitled));
        for (key, untitled) in keys {
            if untitled && !self.files.untitled.iter().any(|(k, _)| *k == key) {
                let n = (1..).find(|n| !self.files.untitled.iter().any(|(_, used)| used == n)).unwrap_or(1);
                self.files.untitled.push((key, n));
            }
        }
    }

    /// File name, or "Untitled" for the only tab and "Untitled N" otherwise (`name(of:)`).
    pub fn tab_name(&self, i: usize) -> String {
        let Some(tab) = self.tabs.get(i) else { return "Untitled".into() };
        if let Some(name) = tab.doc.path.as_ref().and_then(|p| p.file_name()) {
            return name.to_string_lossy().into_owned();
        }
        if let Some(name) = &tab.doc.name {
            return name.clone();
        }
        let key = tab.doc.raw() as usize;
        let n = self.files.untitled.iter().find(|(k, _)| *k == key).map_or(1, |(_, n)| *n);
        if self.tabs.len() == 1 && n == 1 { "Untitled".into() } else { format!("Untitled {n}") }
    }

    /// Renames a document from its tab. A saved document's file is renamed on disk (keeping its extension);
    /// one without a file keeps the name for its tab and as the Save As suggestion (`renameDocument`).
    pub fn rename_document(&mut self, i: usize, new_name: &str) {
        let mut base = new_name.trim().to_string();
        for ext in [".pxmt", ".aseprite", ".ase"] {
            if base.to_lowercase().ends_with(ext) {
                base.truncate(base.len() - ext.len());
            }
        }
        if base.is_empty() || base.contains(['/', '\\', ':']) {
            return;
        }
        let Some(tab) = self.tabs.get_mut(i) else { return };
        let Some(path) = tab.doc.path.clone() else {
            tab.doc.name = Some(base);
            return;
        };
        let ext = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "pxmt".into());
        let target = path.with_file_name(format!("{base}.{ext}"));
        if target == path {
            return;
        }
        let same_file_new_case = target.to_string_lossy().to_lowercase() == path.to_string_lossy().to_lowercase();
        if target.exists() && !same_file_new_case {
            let name = target.file_name().unwrap_or_default().to_string_lossy();
            self.error = Some(format!("\u{201c}{name}\u{201d} already exists in that folder."));
            return;
        }
        match std::fs::rename(&path, &target) {
            Ok(()) => tab.doc.path = Some(target),
            Err(e) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.error = Some(format!("Could not rename \u{201c}{name}\u{201d}: {e}"));
            }
        }
    }

    // --- Opening -------------------------------------------------------------------------------

    /// Untitled, unedited and never changed: opening something replaces it, like a browser's blank tab.
    fn is_pristine(&self, i: usize) -> bool {
        self.tabs.get(i).is_some_and(|t| t.doc.path.is_none() && !t.doc.is_dirty() && t.doc.history_counts() == (0, 0))
    }

    /// A tab for `doc`, with a revision no other tab or earlier document of this slot has used.
    fn new_tab(&mut self, doc: Doc) -> DocTab {
        let mut tab = DocTab::new(doc);
        tab.zoom = initial_zoom(tab.doc.width(), tab.doc.height());
        self.files.next_revision += 1_000_000;
        tab.revision = self.files.next_revision;
        tab
    }

    /// Shows a document: replaces the active tab if it is a pristine untitled one, else opens a new tab.
    pub fn present(&mut self, doc: Doc) {
        let tab = self.new_tab(doc);
        if self.is_pristine(self.active) {
            self.tabs[self.active] = tab;
        } else {
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
        }
        self.sync_untitled();
        self.show_editor();
    }

    pub fn request_new_sprite(&mut self) {
        let (width, height) = self.files.new_size;
        self.files.new_sheet = Some(crate::ui::dialogs::NewSheet { width, height });
    }

    pub fn new_sprite(&mut self, width: u32, height: u32) {
        self.present(Doc::new(width.clamp(1, 4096), height.clamp(1, 4096)));
    }

    /// The Open panel: Pixmith documents, Aseprite files and PNGs.
    pub fn open_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("Pixmith, Aseprite or PNG", &["pxmt", "aseprite", "ase", "png"])
            .add_filter("Pixmith", &["pxmt"])
            .add_filter("Aseprite", &["aseprite", "ase"])
            .add_filter("PNG", &["png"])
            .pick_file();
        if let Some(path) = picked {
            self.open_path(&path);
        }
    }

    /// Opens `path` in a tab (or switches to the tab that already has it). Errors go to `self.error`.
    pub fn open_path(&mut self, path: &Path) {
        // A folder opens as a project (dropped on the window, or given on the command line).
        if path.is_dir() {
            self.open_project(path);
            return;
        }
        let is_png = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"));
        if is_png {
            match Doc::import_png(path) {
                Ok(mut doc) => {
                    self.add_recent(path);
                    doc.touch();
                    self.present(doc);
                }
                Err(e) => self.error = Some(e),
            }
            return;
        }
        if let Some(i) = self.tabs.iter().position(|t| t.doc.path.as_deref().is_some_and(|p| same_file(p, path))) {
            self.active = i;
            self.show_editor();
            return;
        }
        match Doc::open(path) {
            Ok(doc) => self.present(doc),
            Err(e) => self.error = Some(e),
        }
    }

    // --- Saving --------------------------------------------------------------------------------

    /// Save: straight to the file when there is one, else Save As.
    pub fn save(&mut self) -> bool {
        self.save_tab(self.active)
    }

    pub fn save_as(&mut self) -> bool {
        self.save_as_tab(self.active)
    }

    fn save_tab(&mut self, i: usize) -> bool {
        match self.tabs.get(i).and_then(|t| t.doc.path.clone()) {
            Some(path) => self.write_to(i, &path),
            None => self.save_as_tab(i),
        }
    }

    fn save_as_tab(&mut self, i: usize) -> bool {
        let Some(tab) = self.tabs.get(i) else { return false };
        // New documents are saved as .pxmt; a document opened from an .aseprite file stays one.
        let name = tab.doc.path.as_ref().and_then(|p| p.file_name()).map_or_else(
            || format!("{}.pxmt", tab.doc.name.as_deref().unwrap_or("Untitled")),
            |n| n.to_string_lossy().into_owned(),
        );
        let is_aseprite = Path::new(&name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("aseprite") || e.eq_ignore_ascii_case("ase"));
        let pixmith = ("Pixmith", &["pxmt"][..]);
        let aseprite = ("Aseprite", &["aseprite", "ase"][..]);
        let (first, second) = if is_aseprite { (aseprite, pixmith) } else { (pixmith, aseprite) };
        let mut dialog =
            rfd::FileDialog::new().add_filter(first.0, first.1).add_filter(second.0, second.1).set_file_name(name);
        if let Some(dir) = tab.doc.path.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(mut path) = dialog.save_file() else { return false };
        if path.extension().is_none() {
            path.set_extension(if is_aseprite { "aseprite" } else { "pxmt" });
        }
        self.write_to(i, &path)
    }

    fn write_to(&mut self, i: usize, path: &Path) -> bool {
        if let Some(other) = (0..self.tabs.len())
            .find(|&j| j != i && self.tabs[j].doc.path.as_deref().is_some_and(|p| same_file(p, path)))
        {
            self.error = Some(format!(
                "\u{201C}{}\u{201D} is already open in another tab. Close that tab first.",
                self.tab_name(other)
            ));
            return false;
        }
        match self.tabs[i].doc.save_marked(path) {
            Ok(()) => {
                self.sync_untitled();
                true
            }
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }

    // --- Closing and quitting ------------------------------------------------------------------

    /// Closes tab `i`, asking about unsaved changes first.
    pub fn request_close(&mut self, i: usize) {
        let Some(tab) = self.tabs.get(i) else { return };
        if tab.doc.is_dirty() {
            let previous = self.active;
            self.active = i; // show the tab the question is about
            self.files.prompt = Some(Prompt { tab: i, after: After::CloseTab, previous });
        } else {
            self.close_tab(i, None);
        }
    }

    /// Removes tab `i` without asking. The last tab becomes a fresh untitled document.
    fn close_tab(&mut self, i: usize, return_to: Option<usize>) {
        if self.tabs.len() <= 1 {
            if !self.is_pristine(0) {
                let (w, h) = self.files.new_size;
                let tab = self.new_tab(Doc::new(w, h));
                self.tabs[0] = tab;
                self.active = 0;
            }
            self.sync_untitled();
            return;
        }
        self.tabs.remove(i);
        self.active = match return_to {
            Some(p) if p != i => {
                if p > i {
                    p - 1
                } else {
                    p
                }
            }
            _ if self.active > i => self.active - 1,
            _ => self.active.min(i).min(self.tabs.len() - 1),
        };
        self.sync_untitled();
    }

    /// Settles the unsaved-changes question.
    pub fn answer_prompt(&mut self, answer: Answer) {
        let Some(prompt) = self.files.prompt.take() else { return };
        let proceed = match answer {
            Answer::Save => self.save_tab(prompt.tab) && !self.tabs[prompt.tab].doc.is_dirty(),
            Answer::Discard => true,
            Answer::Cancel => false,
        };
        if !proceed {
            self.files.quit_skip.clear();
            self.files.quitting = false;
            return;
        }
        match prompt.after {
            After::CloseTab => self.close_tab(prompt.tab, Some(prompt.previous)),
            After::Quit => {
                if answer == Answer::Discard {
                    self.files.quit_skip.push(prompt.tab);
                }
                self.continue_quit();
            }
        }
    }

    /// Asks about each dirty tab in turn, then lets the window close.
    pub fn continue_quit(&mut self) {
        let dirty = (0..self.tabs.len()).find(|i| self.tabs[*i].doc.is_dirty() && !self.files.quit_skip.contains(i));
        match dirty {
            Some(i) => {
                let previous = self.active;
                self.active = i;
                self.files.prompt = Some(Prompt { tab: i, after: After::Quit, previous });
            }
            None => self.files.quit_approved = true,
        }
    }

    /// Window-level events: dropped files and the close button.
    pub fn handle_window_events(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for path in dropped {
            self.open_path(&path);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.files.quit_approved {
            if self.tabs.iter().any(|t| t.doc.is_dirty()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if !self.files.quitting {
                    self.files.quitting = true;
                    self.files.quit_skip.clear();
                    self.continue_quit();
                }
            } else {
                self.files.quit_approved = true;
            }
        }
        if self.files.quit_approved && self.files.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    // --- Export --------------------------------------------------------------------------------

    pub fn request_export(&mut self, kind: ExportKind) {
        self.files.export = Some(ExportSheet::new(kind, self.tab().doc.frame_count()));
    }

    /// Opens the save panel for `opts` and runs the export.
    pub fn run_export(&mut self, opts: ExportSheet) {
        let tab = &self.tabs[self.active];
        let base = tab.doc.path.as_ref().and_then(|p| p.file_stem()).map_or_else(
            || tab.doc.name.clone().unwrap_or_else(|| "Untitled".into()),
            |s| s.to_string_lossy().into_owned(),
        );
        let (name, filter) = match opts.kind {
            ExportKind::Png | ExportKind::Apng | ExportKind::Sequence => (format!("{base}.png"), ("PNG", "png")),
            ExportKind::Sheet => (format!("{base}-sheet.png"), ("PNG", "png")),
            ExportKind::Gif => (format!("{base}.gif"), ("GIF", "gif")),
        };
        let mut dialog = rfd::FileDialog::new().add_filter(filter.0, &[filter.1]).set_file_name(name);
        if let Some(dir) = tab.doc.path.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else { return };
        let scale = SCALES[opts.scale.min(SCALES.len() - 1)];
        let doc = &tab.doc;
        let result = match opts.kind {
            ExportKind::Png => doc.export_png(tab.frame, scale, &path),
            ExportKind::Sheet => {
                doc.export_sheet(opts.layout as u32, opts.columns.max(1), opts.padding, scale, opts.json, &path)
            }
            ExportKind::Gif => doc.export_gif(scale, &path),
            ExportKind::Apng => doc.export_apng(scale, &path),
            ExportKind::Sequence => {
                let base = path.with_extension("");
                let first = PathBuf::from(format!("{}-001.png", base.display()));
                if first != path && first.exists() && !confirm_replace(&first) {
                    return;
                }
                doc.export_png_sequence(scale, &base)
            }
        };
        if let Err(e) = result {
            self.error = Some(e);
        }
    }
}

fn confirm_replace(first: &Path) -> bool {
    let folder = first.parent().and_then(|p| p.file_name()).map_or(String::new(), |n| n.to_string_lossy().into_owned());
    rfd::MessageDialog::new()
        .set_title("Pixmith")
        .set_description(format!(
            "Files with these names already exist in \u{201C}{folder}\u{201D}. Do you want to replace them?"
        ))
        .set_buttons(rfd::MessageButtons::OkCancelCustom("Replace".into(), "Cancel".into()))
        .show()
        == rfd::MessageDialogResult::Custom("Replace".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pixmith-desktop-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn app() -> PixmithApp {
        PixmithApp::default()
    }

    #[test]
    fn save_then_open_round_trips() {
        let dir = scratch("roundtrip");
        let path = dir.join("art.aseprite");
        let mut a = app();
        a.new_sprite(20, 12);
        a.tab_mut().doc.touch();
        assert!(a.tab().doc.is_dirty());
        assert!(a.write_to(a.active, &path));
        assert!(!a.tab().doc.is_dirty());
        assert_eq!(a.tab_name(a.active), "art.aseprite");

        let mut b = app();
        b.open_path(&path);
        assert!(b.error.is_none(), "{:?}", b.error);
        assert_eq!(b.tabs.len(), 1, "the untouched Untitled tab is replaced");
        assert_eq!((b.tab().doc.width(), b.tab().doc.height()), (20, 12));
        assert!(!b.tab().doc.is_dirty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn opening_the_same_file_switches_to_its_tab() {
        let dir = scratch("same-file");
        let path = dir.join("a.aseprite");
        let mut a = app();
        assert!(a.write_to(0, &path));
        a.new_sprite(8, 8);
        assert_eq!(a.tabs.len(), 2);
        assert_eq!(a.active, 1);
        a.open_path(&path);
        assert_eq!((a.tabs.len(), a.active), (2, 0));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saving_over_a_file_open_in_another_tab_is_refused() {
        let dir = scratch("conflict");
        let path = dir.join("a.aseprite");
        let mut a = app();
        assert!(a.write_to(0, &path));
        a.new_sprite(8, 8);
        assert!(!a.write_to(1, &path));
        assert!(a.error.as_deref().is_some_and(|e| e.contains("already open")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_file_is_reported() {
        let mut a = app();
        a.open_path(Path::new("/definitely/not/here.aseprite"));
        assert!(a.error.is_some());
        assert_eq!(a.tabs.len(), 1);
    }

    #[test]
    fn png_opens_as_an_unsaved_sprite() {
        let dir = scratch("png");
        let path = dir.join("pic.png");
        Doc::new(5, 7).export_png(0, 1, &path).unwrap();
        let mut a = app();
        a.open_path(&path);
        assert!(a.error.is_none(), "{:?}", a.error);
        assert_eq!((a.tab().doc.width(), a.tab().doc.height()), (5, 7));
        assert!(a.tab().doc.is_dirty());
        assert!(a.tab().doc.path.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn untitled_tabs_are_numbered() {
        let mut a = app();
        a.sync_untitled();
        assert_eq!(a.tab_name(0), "Untitled");
        a.tab_mut().doc.touch();
        a.new_sprite(4, 4);
        assert_eq!((a.tab_name(0).as_str(), a.tab_name(1).as_str()), ("Untitled 1", "Untitled 2"));
    }

    #[test]
    fn closing_a_clean_tab_needs_no_question() {
        let mut a = app();
        a.tab_mut().doc.touch();
        a.new_sprite(4, 4);
        a.tab_mut().doc.touch();
        for tab in &mut a.tabs {
            tab.doc.mark_saved();
        }
        a.request_close(0);
        assert!(a.files.prompt.is_none());
        assert_eq!(a.tabs.len(), 1);
        assert_eq!(a.active, 0);
    }

    #[test]
    fn closing_a_dirty_tab_asks_and_discard_closes_it() {
        let mut a = app();
        a.new_sprite(4, 4); // replaces the pristine tab
        a.tab_mut().doc.touch();
        a.new_sprite(6, 6);
        assert_eq!(a.tabs.len(), 2);
        a.active = 1;
        a.request_close(0);
        let prompt = a.files.prompt.expect("asks first");
        assert_eq!((prompt.tab, prompt.after), (0, After::CloseTab));
        assert_eq!(a.active, 0, "shows the tab in question");
        a.answer_prompt(Answer::Cancel);
        assert_eq!(a.tabs.len(), 2);
        a.request_close(0);
        a.answer_prompt(Answer::Discard);
        assert_eq!(a.tabs.len(), 1);
        assert_eq!(a.tab().doc.width(), 6, "back on the tab that was showing");
    }

    #[test]
    fn closing_the_last_dirty_tab_leaves_a_fresh_one() {
        let mut a = app();
        a.tab_mut().doc.touch();
        a.request_close(0);
        a.answer_prompt(Answer::Discard);
        assert_eq!(a.tabs.len(), 1);
        assert!(!a.tab().doc.is_dirty());
    }

    #[test]
    fn save_in_the_question_writes_and_closes() {
        let dir = scratch("save-answer");
        let path = dir.join("a.aseprite");
        let mut a = app();
        assert!(a.write_to(0, &path));
        a.tab_mut().doc.touch();
        a.new_sprite(4, 4);
        a.request_close(0);
        a.answer_prompt(Answer::Save);
        assert_eq!(a.tabs.len(), 1);
        assert!(a.error.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn quitting_asks_about_each_dirty_tab() {
        let mut a = app();
        a.tab_mut().doc.touch();
        a.new_sprite(4, 4);
        a.tab_mut().doc.touch();
        a.files.quitting = true;
        a.continue_quit();
        assert_eq!(a.files.prompt.map(|p| p.tab), Some(0));
        a.answer_prompt(Answer::Discard);
        assert_eq!(a.files.prompt.map(|p| p.tab), Some(1));
        assert!(!a.files.quit_approved);
        a.answer_prompt(Answer::Discard);
        assert!(a.files.quit_approved);
        assert_eq!(a.tabs.len(), 2, "quitting closes no tabs");
    }

    #[test]
    fn cancelling_while_quitting_stops_the_quit() {
        let mut a = app();
        a.tab_mut().doc.touch();
        a.files.quitting = true;
        a.continue_quit();
        a.answer_prompt(Answer::Cancel);
        assert!(!a.files.quit_approved);
        assert!(!a.files.quitting);
        assert!(a.files.quit_skip.is_empty());
    }

    #[test]
    fn exports_write_files() {
        let dir = scratch("export");
        let doc = Doc::new(6, 6);
        let png = dir.join("one.png");
        doc.export_png(0, 2, &png).unwrap();
        doc.export_sheet(0, 1, 0, 1, true, &dir.join("sheet.png")).unwrap();
        doc.export_gif(1, &dir.join("anim.gif")).unwrap();
        doc.export_apng(1, &dir.join("anim-p.png")).unwrap();
        doc.export_png_sequence(1, &dir.join("seq")).unwrap();
        for name in ["one.png", "sheet.png", "sheet.json", "anim.gif", "anim-p.png", "seq-001.png"] {
            assert!(dir.join(name).exists(), "{name} missing");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn zoom_label_matches_swift_g_format() {
        // Lives in the title row; checked here with the other file-independent helpers.
        assert_eq!(crate::ui::title_row::zoom_label(8.0), "800%");
        assert_eq!(crate::ui::title_row::zoom_label(0.375), "37.5%");
        assert_eq!(crate::ui::title_row::zoom_label(1.0), "100%");
    }
}
