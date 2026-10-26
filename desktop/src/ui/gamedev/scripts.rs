//! Lua scripts (`App/Scripts.swift`): the script folder and its list, Run Script..., Install Example Scripts, and
//! running a script as a background job on a snapshot, with one undo step for the whole run.
//!
//! The script runs on a snapshot off the UI thread behind a progress sheet with Cancel, so the window stays
//! responsive and the document is not touched while it runs; a successful result is applied as one undo step.

use super::paths;
use crate::app::PixmithApp;
use crate::calls::gamedev::example_scripts;
use crate::theme::{Colors, font, ui_font};
use crate::ui::widgets::{self, DialogAction};
use eframe::egui;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// The user's script folder and the list of scripts found in it (`ScriptLibrary`).
#[derive(Default)]
pub struct ScriptLibrary {
    /// Overrides the platform folder (tests).
    pub folder: Option<PathBuf>,
    list: Vec<PathBuf>,
    scanned: bool,
}

impl ScriptLibrary {
    pub fn folder(&self) -> PathBuf {
        self.folder.clone().unwrap_or_else(paths::scripts_dir)
    }

    /// Looks at the folder again: `.lua` files, natural order (`localizedStandardCompare`).
    pub fn rescan(&mut self) {
        let mut files: Vec<PathBuf> = std::fs::read_dir(self.folder())
            .map(|d| {
                d.filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| {
                        p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lua"))
                            && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort_by(|a, b| natural_cmp(&name_of(a), &name_of(b)));
        self.list = files;
        self.scanned = true;
    }

    /// The scripts found by the last scan, without touching the disk (for menus built every frame).
    pub fn cached(&self) -> &[PathBuf] {
        &self.list
    }

    /// The scripts found, scanning the folder the first time.
    pub fn scripts(&mut self) -> &[PathBuf] {
        if !self.scanned {
            self.rescan();
        }
        &self.list
    }

    /// Copies the example scripts that ship with the app into the folder (existing files are kept).
    pub fn install_examples(&mut self) {
        let folder = self.folder();
        let _ = std::fs::create_dir_all(&folder);
        for (name, source) in example_scripts() {
            let file = folder.join(&name);
            if !file.exists() {
                let _ = std::fs::write(file, source);
            }
        }
        self.rescan();
    }

    pub fn open_folder(&self) {
        let folder = self.folder();
        let _ = std::fs::create_dir_all(&folder);
        reveal(&folder);
    }
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Case-insensitive, with runs of digits compared as numbers: "b2" before "b10".
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut n = String::new();
                    while let Some(d) = it.peek().copied().filter(char::is_ascii_digit) {
                        n.push(d);
                        it.next();
                    }
                    n.trim_start_matches('0').to_string()
                };
                let (na, nb) = (take(&mut a), take(&mut b));
                let ord = na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Shows a folder in the system file manager.
fn reveal(folder: &Path) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(folder).spawn();
}

impl PixmithApp {
    /// The scripts of the script folder, for File > Scripts.
    pub fn scripts(&mut self) -> Vec<PathBuf> {
        self.gamedev.scripts.scripts().to_vec()
    }

    /// File > Scripts > Rescan.
    pub fn rescan_scripts(&mut self) {
        self.gamedev.scripts.rescan();
    }

    /// File > Scripts > Install Example Scripts.
    pub fn install_example_scripts(&mut self) {
        self.gamedev.scripts.install_examples();
    }

    /// File > Scripts > Open Scripts Folder.
    pub fn open_scripts_folder(&self) {
        self.gamedev.scripts.open_folder();
    }

    pub fn scripts_folder(&self) -> PathBuf {
        self.gamedev.scripts.folder()
    }

    /// File > Scripts > Run Script...: picks a Lua file and runs it.
    pub fn run_script_dialog(&mut self) {
        if self.job_running() {
            return;
        }
        let picked = rfd::FileDialog::new()
            .add_filter("Lua script", &["lua"])
            .set_title("Choose a Lua script to run")
            .pick_file();
        if let Some(path) = picked {
            self.run_script_at(&path);
        }
    }

    /// Runs the script at `path` on a snapshot of the active document (`runScript(at:)`) as a Lua session
    /// (`ui::lua_dialog`): dialogs it opens are drawn by the app, and a successful run is one undo step; a failing
    /// one leaves the document untouched.
    pub fn run_script_at(&mut self, path: &Path) {
        if self.job_running() {
            return;
        }
        let name = name_of(path);
        let Ok(source) = std::fs::read_to_string(path) else {
            self.error = Some(format!("Could not read {name}."));
            return;
        };
        // The session is named by the path, so two scripts called alike keep their "always allow" answers apart.
        self.lua_start_script(&path.to_string_lossy(), &source);
    }
}

/// The last line may be "\u{1}PIXMITH-SELECT layer=<n> frame=<n>" (1-based): the script moved the selection.
pub fn split_selection(raw: &str) -> (Option<(u32, u32)>, String) {
    let mut lines: Vec<&str> = raw.split('\n').collect();
    let mut selection = None;
    if let Some(last) = lines.last()
        && last.starts_with("\u{1}PIXMITH-SELECT")
    {
        let (mut layer, mut frame) = (None, None);
        for part in last.split(' ').skip(1) {
            if let Some((k, v)) = part.split_once('=') {
                match (k, v.parse::<u32>()) {
                    ("layer", Ok(n)) => layer = Some(n),
                    ("frame", Ok(n)) => frame = Some(n),
                    _ => {}
                }
            }
        }
        selection = layer.zip(frame);
        lines.pop();
    }
    (selection, lines.join("\n"))
}

/// The result alert of a script, or any notice with an OK button (`NSAlert`).
pub fn show_notice(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(notice) = app.gamedev.notice.clone() else { return };
    let c = Colors::of(ctx);
    let action = widgets::dialog(ctx, "notice", 400.0, &notice.title, None, |ui| {
        let color = if notice.warning { c.text } else { c.text_secondary };
        ui.add(egui::Label::new(egui::RichText::new(&notice.text).font(ui_font(font::BODY)).color(color)).wrap());
        widgets::dialog_buttons(ui, None, None, "OK", true)
    });
    if action != DialogAction::None {
        app.gamedev.notice = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pixmith-scripts-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Runs the frames of the Lua side until the script's session is over.
    fn finish_job(app: &mut PixmithApp) {
        let ctx = egui::Context::default();
        for _ in 0..2000 {
            // Inside a frame, so a progress window that shows up on a slow machine can lay out its text.
            ctx.begin_pass(egui::RawInput::default());
            crate::ui::lua_dialog::show(app, &ctx);
            // No painter here: drop the font atlas uploads the frame asked for.
            ctx.end_pass().textures_delta.clear();
            if !app.job_running() && app.gamedev.lua.sessions().next().is_none() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("script did not finish");
    }

    #[test]
    fn natural_order_and_case() {
        let mut v = vec!["b10.lua", "B2.lua", "a.lua", "b1.lua"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a.lua", "b1.lua", "B2.lua", "b10.lua"]);
    }

    #[test]
    fn the_scripts_folder_lists_lua_files_and_installs_examples() {
        let dir = scratch("list");
        let mut lib = ScriptLibrary { folder: Some(dir.join("Scripts")), ..Default::default() };
        assert!(lib.scripts().is_empty());
        lib.install_examples();
        let names: Vec<String> = lib.scripts().iter().map(|p| name_of(p)).collect();
        assert!(names.len() >= 5, "{names:?}");
        assert!(names.iter().any(|n| n == "flip-all-frames.lua"));
        // Existing files are kept.
        let file = dir.join("Scripts/flip-all-frames.lua");
        std::fs::write(&file, "-- mine").unwrap();
        lib.install_examples();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "-- mine");
        std::fs::write(dir.join("Scripts/notes.txt"), "x").unwrap();
        lib.rescan();
        assert!(lib.scripts().iter().all(|p| p.extension().unwrap() == "lua"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_script_runs_as_one_undo_step() {
        let dir = scratch("run");
        let file = dir.join("dot.lua");
        std::fs::write(&file, "sprite:setPixel(1, 1, 0xFF0000FF) app.undoLabel('Red dot') print('hello')").unwrap();
        let mut app = PixmithApp::default();
        app.run_script_at(&file);
        assert!(app.job_running());
        finish_job(&mut app);
        assert!(app.error.is_none());
        assert!(app.tab().doc.is_dirty(), "the result was applied");
        assert_eq!(app.tab().doc.history_counts(), (1, 0));
        let n = app.gamedev.notice.clone().expect("print output shows an alert");
        assert_eq!((n.title.as_str(), n.text.as_str(), n.warning), ("dot.lua", "hello", false));
        let mut px = Vec::new();
        assert!(app.tab().doc.render(0, &mut px));
        assert_eq!(&px[(32 + 1) * 4..][..4], &[255, 0, 0, 255]);
        assert!(app.tab_mut().doc.undo());
        assert!(app.tab().doc.render(0, &mut px));
        assert_eq!(&px[(32 + 1) * 4..][..4], &[0, 0, 0, 0]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failing_script_leaves_the_document_alone_and_alerts() {
        let dir = scratch("fail");
        let file = dir.join("bad.lua");
        std::fs::write(&file, "sprite:setPixel(0, 0, 0xFF0000FF) error('boom')").unwrap();
        let mut app = PixmithApp::default();
        app.run_script_at(&file);
        finish_job(&mut app);
        assert!(!app.tab().doc.is_dirty());
        assert_eq!(app.tab().doc.history_counts(), (0, 0));
        let n = app.gamedev.notice.clone().expect("error alert");
        assert_eq!(n.title, "Script error in bad.lua");
        assert!(n.text.contains("boom") && n.text.contains("The document was not changed."), "{}", n.text);
        assert!(n.warning);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_script_can_move_the_selection() {
        let dir = scratch("select");
        let file = dir.join("sel.lua");
        std::fs::write(&file, "sprite:newFrame() app.activeFrame = 2").unwrap();
        let mut app = PixmithApp::default();
        app.run_script_at(&file);
        finish_job(&mut app);
        assert_eq!(app.tab().doc.frame_count(), 2);
        assert_eq!(app.tab().frame, 1);
        assert!(app.gamedev.notice.is_none(), "the selection line is not shown");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_script_slower_than_the_progress_delay_still_finishes() {
        // Runs longer than PROGRESS_AFTER on any machine, so the progress sheet is laid out inside `finish_job`.
        let dir = scratch("slow");
        let file = dir.join("slow.lua");
        std::fs::write(
            &file,
            "local t = os.clock() while os.clock() - t < 0.8 do end sprite:setPixel(1, 1, 0xFF0000FF)",
        )
        .unwrap();
        let mut app = PixmithApp::default();
        app.run_script_at(&file);
        finish_job(&mut app);
        assert!(app.error.is_none());
        assert!(app.tab().doc.is_dirty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn selection_line_is_split_off() {
        let (sel, rest) = split_selection("hi\nthere\n\u{1}PIXMITH-SELECT layer=2 frame=3");
        assert_eq!((sel, rest.as_str()), (Some((2, 3)), "hi\nthere"));
        let (sel, rest) = split_selection("");
        assert_eq!((sel, rest.as_str()), (None, ""));
    }

    #[test]
    fn a_missing_script_is_reported() {
        let mut app = PixmithApp::default();
        app.run_script_at(Path::new("/definitely/not/here.lua"));
        assert!(app.error.as_deref().is_some_and(|e| e.contains("Could not read here.lua")));
        assert!(!app.job_running());
    }
}
