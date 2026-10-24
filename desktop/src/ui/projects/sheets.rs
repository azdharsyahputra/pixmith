//! The sheets of the Home screen and project view: New Project (name and location) and New Canvas (name and
//! size). They use the dialog controls of the other sheets (`DSDialog`).

use super::ProjectsState;
use crate::app::PixmithApp;
use crate::theme::{Colors, font, metric, radius, space, ui_font};
use crate::ui::widgets::{self, DialogAction};
use eframe::egui::{self, Sense, Ui, pos2, vec2};
use std::path::{Path, PathBuf};

/// The New Project sheet.
#[derive(Clone, Debug)]
pub struct NewProject {
    pub name: String,
    pub location: PathBuf,
    fresh: bool,
}

/// The folder New Project starts in: the last one used, else Documents, else the home folder.
fn default_location(state: &ProjectsState) -> PathBuf {
    if let Some(dir) = state.location.as_ref().filter(|d| d.is_dir()) {
        return dir.clone();
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    match home {
        Some(h) if h.join("Documents").is_dir() => h.join("Documents"),
        Some(h) => h,
        None => PathBuf::from("."),
    }
}

impl NewProject {
    pub fn new(state: &ProjectsState) -> NewProject {
        NewProject { name: "Untitled Project".into(), location: default_location(state), fresh: true }
    }

    /// Why Create is unavailable, if it is.
    pub fn problem(&self) -> Option<String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Some("Enter a name for the project.".into());
        }
        if name.contains(['/', '\\', ':']) || name.starts_with('.') {
            return Some("The name cannot contain slashes or colons, or start with a dot.".into());
        }
        if self.location.join(name).exists() {
            return Some(format!("A folder named \u{201C}{name}\u{201D} already exists in that location."));
        }
        None
    }
}

/// The New Canvas sheet.
#[derive(Clone, Debug)]
pub struct NewCanvas {
    pub folder: PathBuf,
    pub name: String,
    pub width: u32,
    pub height: u32,
    fresh: bool,
}

impl NewCanvas {
    pub fn new(folder: &Path, size: (u32, u32)) -> NewCanvas {
        NewCanvas { folder: folder.to_path_buf(), name: "Canvas".into(), width: size.0, height: size.1, fresh: true }
    }

    pub fn problem(&self) -> Option<String> {
        let name = self.name.trim();
        (name.contains(['/', '\\', ':']) || name.starts_with('.'))
            .then(|| "The name cannot contain slashes or colons, or start with a dot.".to_string())
    }
}

pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    if app.error.is_some() {
        return;
    }
    new_project(app, ctx);
    new_canvas(app, ctx);
}

fn name_field(ui: &mut Ui, id: &str, text: &mut String, fresh: &mut bool) {
    let field_id = ui.id().with(id);
    let r = widgets::ds_text_field(ui, id, text, None);
    if std::mem::take(fresh) {
        r.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), field_id) {
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(text.chars().count()),
            )));
            state.store(ui.ctx(), field_id);
        }
    }
}

fn problem_note(ui: &mut Ui, problem: &Option<String>) {
    if let Some(p) = problem {
        let c = Colors::of(ui.ctx());
        ui.label(egui::RichText::new(p).font(ui_font(font::CAPTION)).color(c.destructive));
    }
}

fn new_project(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(mut sheet) = app.projects.new_project.take() else { return };
    let mut choose = false;
    let problem = sheet.problem();
    let action = widgets::dialog(
        ctx,
        "new_project",
        420.0,
        "New Project",
        Some("A project is a folder that holds your canvases."),
        |ui| {
            widgets::form_row(ui, "Name", |ui| {
                let mut fresh = sheet.fresh;
                name_field(ui, "project_name", &mut sheet.name, &mut fresh);
                sheet.fresh = fresh;
            });
            widgets::form_row(ui, "Location", |ui| {
                let c = Colors::of(ui.ctx());
                let button_w = 84.0;
                let (rect, _) = ui.allocate_exact_size(
                    vec2(ui.available_width() - button_w - space::SM, metric::CONTROL),
                    Sense::hover(),
                );
                ui.painter().rect_filled(rect, radius::FIELD, c.field);
                let shown = super::cards::fit_text(
                    ui,
                    &display_path(&sheet.location),
                    ui_font(font::BODY),
                    c.text_secondary,
                    rect.width() - 2.0 * space::SM,
                );
                ui.painter().galley(
                    pos2(rect.left() + space::SM, rect.center().y - shown.size().y / 2.0),
                    shown,
                    c.text_secondary,
                );
                if widgets::secondary_button(ui, "Choose\u{2026}", false).clicked() {
                    choose = true;
                }
            });
            problem_note(ui, &problem);
            widgets::dialog_buttons(ui, None, Some("Cancel"), "Create", problem.is_none())
        },
    );
    if choose
        && let Some(dir) =
            rfd::FileDialog::new().set_title("Choose a Location").set_directory(&sheet.location).pick_folder()
    {
        sheet.location = dir;
    }
    match action {
        DialogAction::Primary if problem.is_none() => {
            let (parent, name) = (sheet.location.clone(), sheet.name.clone());
            app.create_project(&parent, &name);
        }
        DialogAction::Cancel => {}
        _ => app.projects.new_project = Some(sheet),
    }
}

/// The path with the home folder shortened to `~`.
fn display_path(path: &Path) -> String {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    match home.and_then(|h| path.strip_prefix(&h).ok().map(|rest| (h, rest.to_path_buf()))) {
        Some((_, rest)) if rest.as_os_str().is_empty() => "~".into(),
        Some((_, rest)) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => path.display().to_string(),
    }
}

fn new_canvas(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(mut sheet) = app.projects.new_canvas.take() else { return };
    let problem = sheet.problem();
    let action = widgets::dialog(ctx, "new_canvas", 320.0, "New Canvas", None, |ui| {
        widgets::form_row(ui, "Name", |ui| {
            let mut fresh = sheet.fresh;
            name_field(ui, "canvas_name", &mut sheet.name, &mut fresh);
            sheet.fresh = fresh;
        });
        widgets::form_row(ui, "Width", |ui| widgets::number_field(ui, "W", &mut sheet.width, 1..=4096, "px", 120.0));
        widgets::form_row(ui, "Height", |ui| widgets::number_field(ui, "H", &mut sheet.height, 1..=4096, "px", 120.0));
        problem_note(ui, &problem);
        widgets::dialog_buttons(ui, None, Some("Cancel"), "Create", problem.is_none())
    });
    match action {
        DialogAction::Primary if problem.is_none() => {
            let folder = sheet.folder.clone();
            app.create_canvas(&folder, &sheet.name, sheet.width, sheet.height);
        }
        DialogAction::Cancel => {}
        _ => app.projects.new_canvas = Some(sheet),
    }
}

impl ProjectsState {
    /// A sheet of this module is open, so shortcuts wait.
    pub fn has_sheet(&self) -> bool {
        self.new_project.is_some() || self.new_canvas.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_names_are_checked() {
        let dir = std::env::temp_dir().join(format!("pixmith-sheets-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Taken")).unwrap();
        let mut s = NewProject { name: "Fresh".into(), location: dir.clone(), fresh: false };
        assert!(s.problem().is_none());
        s.name = "  ".into();
        assert!(s.problem().is_some());
        s.name = "a/b".into();
        assert!(s.problem().is_some());
        s.name = "Taken".into();
        assert!(s.problem().unwrap().contains("already exists"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
