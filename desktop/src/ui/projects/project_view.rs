//! The project view: the project's name and canvas count, New Canvas and Import, and a grid of canvas cards
//! (thumbnail, name, size, modified). Double-click or Enter opens a canvas in a tab; the context menu renames,
//! duplicates, deletes (into the project's trash, with an Undo toast and a Recently deleted list), reveals and
//! exports; cards can be dragged to reorder, which saves the order.

use super::cards::{self, GAP, Grid, TEXT_GAP};
use super::home::{self, CardSpec};
use super::jobs::ThumbKey;
use crate::app::PixmithApp;
use crate::calls::projects::{self as calls, Canvas, Project};
use crate::theme::{Colors, font, metric, radius, space, ui_font};
use crate::ui::widgets;
use eframe::egui::{
    self, Align, Align2, Color32, Key, Layout, Margin, Modifiers, Order, Rect, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, pos2, vec2,
};
use egui_phosphor::regular as icon;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const INSET_X: f32 = 32.0;
const INSET_TOP: f32 = 24.0;
/// How long the Undo toast stays.
const TOAST_SECONDS: u64 = 8;

/// A card being dragged to a new place.
#[derive(Clone, Debug)]
pub struct Drag {
    pub path: PathBuf,
    pub index: usize,
}

/// "Moved to Recently deleted" with an Undo.
#[derive(Clone, Debug)]
pub struct Toast {
    pub message: String,
    pub trashed: PathBuf,
    /// Where the canvas stood, so Undo puts it back there.
    pub index: usize,
    pub until: Instant,
}

#[derive(Default)]
pub struct ViewState {
    /// A folder whose scan the next frame starts (set when a screen opens or something changed on disk).
    pub needs_scan: Option<PathBuf>,
    pub needs_scan_all: bool,
    pub selected: Option<PathBuf>,
    /// The canvas being renamed in place and the text so far.
    pub renaming: Option<(PathBuf, String)>,
    /// The first frame of a rename: focus the field and select all.
    pub rename_fresh: bool,
    /// The project name while it is being edited.
    pub name_edit: Option<String>,
    pub name_fresh: bool,
    pub drag: Option<Drag>,
    pub toast: Option<Toast>,
    /// Canvases in the project's trash with their file times, newest first.
    pub trash: Vec<(PathBuf, u64)>,
    pub trash_open: bool,
    /// Scroll the selected card into view on the next frame (keyboard moves).
    pub scroll_to_selected: bool,
}

impl ViewState {
    /// Forgets the state of a previous project (selection, rename, drag, toast).
    pub fn reset(&mut self) {
        *self = ViewState { trash_open: false, ..ViewState::default() };
    }
}

/// Scans arrived: refresh what is derived from the folder (the trash list).
pub fn scans_arrived(app: &mut PixmithApp) {
    if let super::Screen::Project(folder) = app.projects.screen.clone() {
        app.projects.view.trash = calls::trashed_canvases(&folder);
        // A selection that no longer exists goes away.
        if let (Some(sel), Some(p)) = (app.projects.view.selected.clone(), app.projects.scans.project(&folder))
            && !p.canvases.iter().any(|c| c.path == sel)
        {
            app.projects.view.selected = None;
        }
    }
}

// --- Operations --------------------------------------------------------------------------------------------

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

impl PixmithApp {
    fn tab_with(&self, path: &Path) -> Option<usize> {
        self.tabs.iter().position(|t| t.doc.path.as_deref() == Some(path))
    }

    fn rescan(&mut self, folder: &Path) {
        self.projects.view.needs_scan = Some(folder.to_path_buf());
        self.projects.view.trash = calls::trashed_canvases(folder);
    }

    /// The canvas names in their shown order.
    fn shown_names(&self, folder: &Path) -> Vec<String> {
        self.projects
            .scans
            .project(folder)
            .map(|p| p.canvases.iter().map(Canvas::file_name).collect())
            .unwrap_or_default()
    }

    /// Shows `names` as the new order, saves it and keeps the grid steady until the next scan.
    fn apply_order(&mut self, folder: &Path, names: Vec<String>) {
        if let Err(e) = calls::set_order(folder, &names) {
            self.error = Some(e);
            return;
        }
        self.projects.scans.update(folder, |p| {
            p.canvases.sort_by_key(|c| names.iter().position(|n| *n == c.file_name()).unwrap_or(usize::MAX));
        });
        self.rescan(folder);
    }

    /// New Canvas: creates the file and opens it in a tab.
    pub fn create_canvas(&mut self, folder: &Path, name: &str, width: u32, height: u32) {
        match calls::new_canvas(folder, name, width, height) {
            Ok(path) => {
                self.files.new_size = (width, height);
                self.rescan(folder);
                self.open_canvas(&path);
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub fn duplicate_canvas(&mut self, path: &Path) {
        let Some(folder) = path.parent().map(Path::to_path_buf) else { return };
        match calls::duplicate_canvas(path) {
            Ok(copy) => {
                // The copy sits right after the original.
                let mut names = self.shown_names(&folder);
                if let Some(i) = names.iter().position(|n| *n == file_name(path)) {
                    names.insert(i + 1, file_name(&copy));
                    if let Err(e) = calls::set_order(&folder, &names) {
                        self.error = Some(e);
                    }
                }
                self.projects.view.selected = Some(copy);
                self.projects.view.scroll_to_selected = true;
                self.rescan(&folder);
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub fn rename_canvas(&mut self, path: &Path, new_name: &str) {
        let Some(folder) = path.parent().map(Path::to_path_buf) else { return };
        match calls::rename_canvas(path, new_name) {
            Ok(target) => {
                if let Some(i) = self.tab_with(path) {
                    self.tabs[i].doc.path = Some(target.clone());
                }
                let mut names = self.shown_names(&folder);
                if let Some(n) = names.iter_mut().find(|n| **n == file_name(path)) {
                    *n = file_name(&target);
                    if let Err(e) = calls::set_order(&folder, &names) {
                        self.error = Some(e);
                    }
                }
                if self.projects.view.selected.as_deref() == Some(path) {
                    self.projects.view.selected = Some(target);
                }
                self.rescan(&folder);
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Delete: moves the canvas to the project's trash and offers Undo.
    pub fn trash_canvas(&mut self, path: &Path) {
        let Some(folder) = path.parent().map(Path::to_path_buf) else { return };
        if self.tab_with(path).is_some() {
            self.error = Some(format!(
                "\u{201C}{}\u{201D} is open in a tab. Close that tab first.",
                path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
            ));
            return;
        }
        let names = self.shown_names(&folder);
        let index = names.iter().position(|n| *n == file_name(path)).unwrap_or(names.len());
        match calls::trash_canvas(path) {
            Ok(trashed) => {
                self.projects.scans.update(&folder, |p| p.canvases.retain(|c| c.path != path));
                let view = &mut self.projects.view;
                // The next card takes the selection, like Finder.
                if view.selected.as_deref() == Some(path) {
                    view.selected = names
                        .get(index + 1)
                        .or_else(|| index.checked_sub(1).and_then(|i| names.get(i)))
                        .map(|n| folder.join(n));
                }
                view.toast = Some(Toast {
                    message: format!(
                        "Moved \u{201C}{}\u{201D} to Recently deleted",
                        path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
                    ),
                    trashed,
                    index,
                    until: Instant::now() + Duration::from_secs(TOAST_SECONDS),
                });
                self.rescan(&folder);
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Restores a trashed canvas, to `index` in the order when it is given.
    pub fn restore_canvas(&mut self, trashed: &Path, index: Option<usize>) {
        let Some(folder) = calls::project_of_trashed(trashed) else { return };
        match calls::restore_canvas(trashed) {
            Ok(path) => {
                if let Some(index) = index {
                    let mut names = self.shown_names(&folder);
                    names.retain(|n| *n != file_name(&path));
                    names.insert(index.min(names.len()), file_name(&path));
                    if let Err(e) = calls::set_order(&folder, &names) {
                        self.error = Some(e);
                    }
                }
                self.projects.view.selected = Some(path);
                self.projects.view.scroll_to_selected = true;
                self.rescan(&folder);
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Import…: picks files and copies them in as `.pxmt` canvases.
    pub fn import_into_project(&mut self, folder: &Path) {
        let picked = rfd::FileDialog::new()
            .set_title("Import")
            .add_filter("Aseprite or PNG", &["aseprite", "ase", "png"])
            .pick_files();
        let Some(files) = picked else { return };
        let mut failures = Vec::new();
        let mut last = None;
        for file in files {
            match calls::import_into_project(folder, &file) {
                Ok(path) => last = Some(path),
                Err(e) => failures.push(e),
            }
        }
        if let Some(path) = last {
            self.projects.view.selected = Some(path);
            self.projects.view.scroll_to_selected = true;
        }
        if !failures.is_empty() {
            self.error = Some(failures.join("\n"));
        }
        self.rescan(folder);
    }
}

// --- The view ----------------------------------------------------------------------------------------------

enum Act {
    Open(PathBuf),
    Select(PathBuf),
    StartRename(PathBuf),
    CommitRename(PathBuf, String),
    Duplicate(PathBuf),
    Trash(PathBuf),
    Reveal(PathBuf),
    ExportAseprite(PathBuf),
    Restore(PathBuf),
    Move { from: usize, slot: usize },
    NewCanvas,
    Import,
    Back,
    SetName(String),
}

pub fn show(app: &mut PixmithApp, ui: &mut Ui, folder: &Path) {
    let c = app.colors();
    let entry = app.projects.scans.get(folder).cloned();
    let project = entry.as_ref().and_then(|e| e.project.clone());
    let mut acts: Vec<Act> = Vec::new();
    let frame = egui::Frame::new().inner_margin(Margin {
        left: INSET_X as i8,
        right: INSET_X as i8,
        top: INSET_TOP as i8,
        bottom: 0,
    });
    frame.show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        header(app, ui, &c, folder, project.as_deref(), &mut acts);
        ui.add_space(space::LG);
        match (&project, &entry) {
            (Some(p), _) if p.canvases.is_empty() && app.projects.view.trash.is_empty() => empty(ui, &c, &mut acts),
            (Some(p), _) => body(app, ui, &c, folder, p, &mut acts),
            (None, Some(e)) if e.error.is_some() && !e.loading => {
                message(ui, &c, icon::WARNING, "This project could not be opened", e.error.as_deref().unwrap_or(""));
            }
            _ => message(ui, &c, icon::CIRCLE_NOTCH, "Loading\u{2026}", ""),
        }
    });
    toast(app, ui.ctx(), &c);
    run(app, folder, acts);
}

fn message(ui: &mut Ui, c: &Colors, glyph: &str, title: &str, body: &str) {
    let rect = ui.available_rect_before_wrap();
    let mid = rect.center();
    ui.painter().text(
        pos2(mid.x, mid.y - 50.0),
        Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(36.0),
        c.text_tertiary,
    );
    ui.painter().text(pos2(mid.x, mid.y - 14.0), Align2::CENTER_CENTER, title, widgets::strong(ui.ctx(), 16.0), c.text);
    if !body.is_empty() {
        let galley = ui.painter().layout(body.into(), ui_font(font::BODY_LARGE), c.text_secondary, 420.0);
        ui.painter().galley(pos2(mid.x - galley.size().x / 2.0, mid.y + 6.0), galley, c.text_secondary);
    }
}

fn empty(ui: &mut Ui, c: &Colors, acts: &mut Vec<Act>) {
    let mid = ui.available_rect_before_wrap().center();
    ui.painter().text(
        pos2(mid.x, mid.y - 80.0),
        Align2::CENTER_CENTER,
        icon::SQUARES_FOUR,
        egui::FontId::proportional(40.0),
        c.text_tertiary,
    );
    ui.painter().text(
        pos2(mid.x, mid.y - 40.0),
        Align2::CENTER_CENTER,
        "No canvases yet",
        widgets::strong(ui.ctx(), 17.0),
        c.text,
    );
    let galley = ui.painter().layout(
        "Create a blank canvas, or import an Aseprite or PNG file into this project.".into(),
        ui_font(font::BODY_LARGE),
        c.text_secondary,
        380.0,
    );
    let h = galley.size().y;
    ui.painter().galley(pos2(mid.x - galley.size().x / 2.0, mid.y - 18.0), galley, c.text_secondary);
    let area = Rect::from_center_size(pos2(mid.x, mid.y + h + 16.0), vec2(300.0, 32.0));
    ui.scope_builder(UiBuilder::new().max_rect(area).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        ui.allocate_ui(vec2(144.0, 32.0), |ui| {
            if home::wide_button(ui, "New Canvas", icon::PLUS, true).clicked() {
                acts.push(Act::NewCanvas);
            }
        });
        ui.allocate_ui(vec2(120.0, 32.0), |ui| {
            if home::wide_button(ui, "Import\u{2026}", icon::DOWNLOAD_SIMPLE, false).clicked() {
                acts.push(Act::Import);
            }
        });
    });
}

fn header(
    app: &mut PixmithApp,
    ui: &mut Ui,
    c: &Colors,
    folder: &Path,
    project: Option<&Project>,
    acts: &mut Vec<Act>,
) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    ui.scope_builder(UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        if widgets::icon_button_sized(ui, icon::CARET_LEFT, metric::ICON_BUTTON, 14.0)
            .on_hover_text("Back to Home")
            .clicked()
        {
            acts.push(Act::Back);
        }
        let name = project
            .map(|p| p.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        project_name(app, ui, c, &name, acts);
        if let Some(p) = project {
            ui.label(
                egui::RichText::new(cards::canvas_count(p.canvases.len()))
                    .font(ui_font(font::BODY))
                    .color(c.text_secondary),
            );
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            let new_canvas = widgets::primary_button(ui, "New Canvas", true);
            new_canvas.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "New Canvas"));
            if new_canvas.clicked() {
                acts.push(Act::NewCanvas);
            }
            if home::workspace_button(ui, "Import\u{2026}").clicked() {
                acts.push(Act::Import);
            }
        });
    });
}

/// The project name: click to edit, Return keeps the change, Esc cancels.
fn project_name(app: &mut PixmithApp, ui: &mut Ui, c: &Colors, name: &str, acts: &mut Vec<Act>) {
    let font = widgets::strong(ui.ctx(), 20.0);
    if let Some(draft) = app.projects.view.name_edit.as_mut() {
        let id = ui.id().with("project_name_edit");
        let (rect, _) = ui.allocate_exact_size(vec2(320.0, 32.0), Sense::hover());
        ui.painter().rect_filled(rect, radius::FIELD, c.field);
        ui.painter().rect_stroke(rect, radius::FIELD, Stroke::new(1.5, c.accent_line), StrokeKind::Middle);
        let out = egui::TextEdit::singleline(draft)
            .id(id)
            .font(font)
            .frame(egui::Frame::NONE)
            .margin(Margin::ZERO)
            .vertical_align(Align::Center)
            .desired_width(rect.width() - 16.0);
        let out = widgets::put_in(ui, rect.shrink2(vec2(8.0, 0.0)), out);
        if app.projects.view.name_fresh {
            app.projects.view.name_fresh = false;
            select_all(ui.ctx(), id, out.clone(), draft.chars().count());
        }
        let escape = ui.input(|i| i.key_pressed(Key::Escape));
        if escape {
            app.projects.view.name_edit = None;
        } else if out.lost_focus() {
            let text = std::mem::take(draft);
            app.projects.view.name_edit = None;
            if !text.trim().is_empty() && text.trim() != name {
                acts.push(Act::SetName(text.trim().to_string()));
            }
        }
    } else {
        let galley = ui.painter().layout_no_wrap(name.to_owned(), font, c.text);
        let width = (galley.size().x + 8.0).min(ui.available_width() * 0.6);
        let (rect, response) = ui.allocate_exact_size(vec2(width + 28.0, 32.0), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(rect, radius::CONTROL, c.hover.gamma_multiply(0.6));
        }
        let shown = cards::fit_text(ui, name, widgets::strong(ui.ctx(), 20.0), c.text, width);
        ui.painter().galley(pos2(rect.left() + 8.0, rect.center().y - shown.size().y / 2.0), shown.clone(), c.text);
        if response.hovered() {
            ui.painter().text(
                pos2(rect.left() + 8.0 + shown.size().x + 12.0, rect.center().y),
                Align2::CENTER_CENTER,
                icon::PENCIL_SIMPLE,
                egui::FontId::proportional(13.0),
                c.text_tertiary,
            );
        }
        if response.on_hover_text("Rename project").clicked() {
            app.projects.view.name_edit = Some(name.to_string());
            app.projects.view.name_fresh = true;
        }
    }
}

/// Focuses the text field `id` and selects its whole text.
fn select_all(ctx: &egui::Context, id: egui::Id, response: egui::Response, len: usize) {
    response.request_focus();
    if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(len),
        )));
        state.store(ctx, id);
    }
}

fn body(app: &mut PixmithApp, ui: &mut Ui, c: &Colors, folder: &Path, project: &Arc<Project>, acts: &mut Vec<Act>) {
    let dark = app.dark;
    let n = project.canvases.len();
    egui::ScrollArea::vertical().id_salt("project_canvases").auto_shrink([false, false]).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let grid = Grid::new(ui.available_width());
        let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), grid.height(n) + 8.0), Sense::hover());
        let origin = area.min + vec2(0.0, 4.0);
        let ctx = ui.ctx().clone();
        keyboard(app, &ctx, project, grid, origin, ui, acts);

        let dragging = app.projects.view.drag.clone();
        let visible = grid.visible(origin, ui.clip_rect().expand(grid.row_h), n);
        for i in visible {
            let canvas = &project.canvases[i];
            let thumb = grid.thumb_rect(origin, i);
            let card_rect = grid.card_rect(origin, i);
            let selected = app.projects.view.selected.as_deref() == Some(canvas.path.as_path());
            let renaming = app.projects.view.renaming.as_ref().is_some_and(|(p, _)| *p == canvas.path);
            let key = (canvas.error.is_none()).then(|| ThumbKey {
                path: canvas.path.clone(),
                modified: canvas.modified,
                size: canvas.size,
            });
            let details = match &canvas.error {
                Some(_) => "Can\u{2019}t be read".to_string(),
                None => format!("{} \u{00D7} {} \u{00B7} {}", canvas.width, canvas.height, cards::ago(canvas.modified)),
            };
            let being_dragged = dragging.as_ref().is_some_and(|d| d.path == canvas.path);
            let spec = CardSpec {
                id: ui.id().with(("canvas", &canvas.path)),
                card: card_rect,
                thumb,
                key: key.as_ref(),
                name: &canvas.name,
                label: &canvas.name,
                details: &details,
                selected,
                dim: being_dragged || canvas.error.is_some(),
                sense: Sense::click_and_drag(),
                placeholder: icon::WARNING,
            };
            // The name line is replaced by a field while renaming.
            let shown_spec = if renaming { CardSpec { name: "", ..spec } } else { spec };
            let r = home::card(ui, &mut app.projects.thumbs, c, dark, &shown_spec);
            if being_dragged {
                ui.painter().rect_filled(thumb, radius::PANEL as f32, c.workspace.gamma_multiply(0.45));
            }
            if renaming {
                rename_field(app, ui, c, card_rect, thumb.bottom(), acts);
            }
            if r.clicked() || r.drag_started() {
                acts.push(Act::Select(canvas.path.clone()));
            }
            if r.double_clicked() && !renaming {
                acts.push(Act::Open(canvas.path.clone()));
            }
            if r.drag_started() && app.projects.view.renaming.is_none() {
                app.projects.view.drag = Some(Drag { path: canvas.path.clone(), index: i });
            }
            let path = canvas.path.clone();
            r.context_menu(|ui| {
                if ui.button("Open").clicked() {
                    acts.push(Act::Open(path.clone()));
                    ui.close();
                }
                ui.separator();
                if ui.button("Rename").clicked() {
                    acts.push(Act::StartRename(path.clone()));
                    ui.close();
                }
                if ui.button("Duplicate").clicked() {
                    acts.push(Act::Duplicate(path.clone()));
                    ui.close();
                }
                if ui.button("Delete").clicked() {
                    acts.push(Act::Trash(path.clone()));
                    ui.close();
                }
                ui.separator();
                if ui.button(home::reveal_label()).clicked() {
                    acts.push(Act::Reveal(path.clone()));
                    ui.close();
                }
                if ui.button("Export As Aseprite\u{2026}").clicked() {
                    acts.push(Act::ExportAseprite(path.clone()));
                    ui.close();
                }
            });
        }
        drag_to_reorder(app, ui, c, grid, origin, project, acts);
        recently_deleted(app, ui, c, folder, acts);
    });
}

fn rename_field(app: &mut PixmithApp, ui: &mut Ui, c: &Colors, card: Rect, thumb_bottom: f32, acts: &mut Vec<Act>) {
    let rect = Rect::from_min_size(pos2(card.left(), thumb_bottom + TEXT_GAP - 3.0), vec2(card.width(), 24.0));
    let fresh = std::mem::take(&mut app.projects.view.rename_fresh);
    let Some((path, draft)) = app.projects.view.renaming.as_mut() else { return };
    let id = ui.id().with(("rename", &*path));
    ui.painter().rect_filled(rect, radius::FIELD, c.field);
    ui.painter().rect_stroke(rect, radius::FIELD, Stroke::new(1.5, c.accent_line), StrokeKind::Middle);
    let out = egui::TextEdit::singleline(draft)
        .id(id)
        .font(widgets::strong(ui.ctx(), font::BODY_LARGE))
        .frame(egui::Frame::NONE)
        .margin(Margin::ZERO)
        .vertical_align(Align::Center)
        .desired_width(rect.width() - 12.0);
    let out = widgets::put_in(ui, rect.shrink2(vec2(6.0, 0.0)), out);
    if fresh {
        select_all(ui.ctx(), id, out.clone(), draft.chars().count());
        return;
    }
    let escape = ui.input(|i| i.key_pressed(Key::Escape));
    if escape {
        app.projects.view.renaming = None;
    } else if out.lost_focus() {
        let (path, text) = app.projects.view.renaming.take().expect("renaming");
        acts.push(Act::CommitRename(path, text));
    }
}

/// Arrow keys move the selection, Return opens, Delete deletes, F2 renames.
fn keyboard(
    app: &mut PixmithApp,
    ctx: &egui::Context,
    project: &Project,
    grid: Grid,
    origin: egui::Pos2,
    ui: &mut Ui,
    acts: &mut Vec<Act>,
) {
    let view = &mut app.projects.view;
    let blocked = ctx.egui_wants_keyboard_input()
        || view.renaming.is_some()
        || app.error.is_some()
        || app.files.new_sheet.is_some()
        || app.files.prompt.is_some()
        || app.projects.new_canvas.is_some()
        || app.projects.new_project.is_some();
    if blocked || project.canvases.is_empty() {
        return;
    }
    let n = project.canvases.len();
    let current = view.selected.as_ref().and_then(|s| project.canvases.iter().position(|c| c.path == *s));
    let step = |i: usize, dx: i32, dy: i32| -> usize {
        let cols = grid.cols as i32;
        let target = i as i32 + dx + dy * cols;
        if (0..n as i32).contains(&target) { target as usize } else { i }
    };
    let (left, right, up, down, enter, delete, rename) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowLeft),
            i.consume_key(Modifiers::NONE, Key::ArrowRight),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Delete) | i.consume_key(Modifiers::NONE, Key::Backspace),
            i.consume_key(Modifiers::NONE, Key::F2),
        )
    });
    let (dx, dy) = (i32::from(right) - i32::from(left), i32::from(down) - i32::from(up));
    if dx != 0 || dy != 0 {
        let next = match current {
            Some(i) => step(i, dx, dy),
            None => 0,
        };
        view.selected = Some(project.canvases[next].path.clone());
        view.scroll_to_selected = true;
    }
    if view.scroll_to_selected {
        view.scroll_to_selected = false;
        if let Some(i) = view.selected.as_ref().and_then(|s| project.canvases.iter().position(|c| c.path == *s)) {
            ui.scroll_to_rect(grid.card_rect(origin, i).expand(GAP / 2.0), None);
        }
    }
    if let Some(i) = current {
        let path = project.canvases[i].path.clone();
        if enter {
            acts.push(Act::Open(path));
        } else if delete {
            acts.push(Act::Trash(path));
        } else if rename {
            acts.push(Act::StartRename(path));
        }
    }
}

/// The drop marker and the dragged card's ghost; moves the card when the button is released.
fn drag_to_reorder(
    app: &mut PixmithApp,
    ui: &mut Ui,
    c: &Colors,
    grid: Grid,
    origin: egui::Pos2,
    project: &Project,
    acts: &mut Vec<Act>,
) {
    let Some(drag) = app.projects.view.drag.clone() else { return };
    let ctx = ui.ctx().clone();
    let n = project.canvases.len();
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        app.projects.view.drag = None;
        return;
    }
    let Some(pointer) = ctx.pointer_latest_pos() else { return };
    // Scroll while the pointer is near the top or bottom edge.
    let clip = ui.clip_rect();
    if pointer.y < clip.top() + 48.0 {
        ui.scroll_with_delta(vec2(0.0, 14.0));
        ctx.request_repaint();
    } else if pointer.y > clip.bottom() - 48.0 {
        ui.scroll_with_delta(vec2(0.0, -14.0));
        ctx.request_repaint();
    }
    // The slot between cards the pointer is nearest to: 0..=n.
    let rel = pointer - origin;
    let row = ((rel.y / grid.row_h).floor().max(0.0) as usize).min(grid.rows(n).saturating_sub(1));
    let col_f = (rel.x + GAP / 2.0) / (grid.card_w + GAP);
    let col = (col_f.floor().max(0.0) as usize).min(grid.cols - 1);
    let hovered = (row * grid.cols + col).min(n - 1);
    let hovered_rect = grid.thumb_rect(origin, hovered);
    let slot = if pointer.x < hovered_rect.center().x { hovered } else { hovered + 1 };
    let marker_x =
        if slot < n && slot == hovered { hovered_rect.left() - GAP / 2.0 } else { hovered_rect.right() + GAP / 2.0 };
    let marker =
        Rect::from_min_max(pos2(marker_x - 1.5, hovered_rect.top()), pos2(marker_x + 1.5, hovered_rect.bottom()));
    let noop = slot == drag.index || slot == drag.index + 1;
    if !noop {
        ui.painter().rect_filled(marker, 1.5, c.accent);
    }
    // The ghost follows the pointer.
    let layer = egui::LayerId::new(Order::Tooltip, egui::Id::new("canvas_drag_ghost"));
    let painter = ctx.layer_painter(layer);
    let ghost = Rect::from_center_size(pointer + vec2(0.0, 6.0), vec2(120.0, 28.0));
    painter.rect_filled(ghost, radius::CONTROL, c.elevated);
    painter.rect_stroke(ghost, radius::CONTROL, Stroke::new(1.0, c.accent_line), StrokeKind::Inside);
    let name = project.canvases.get(drag.index).map(|cv| cv.name.clone()).unwrap_or_default();
    let galley = ui.painter().layout(name, ui_font(font::BODY), c.text, 104.0);
    painter.galley(pos2(ghost.left() + 8.0, ghost.center().y - galley.size().y / 2.0), galley, c.text);
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    if ctx.input(|i| i.pointer.any_released()) {
        app.projects.view.drag = None;
        if !noop {
            acts.push(Act::Move { from: drag.index, slot });
        }
    }
}

fn recently_deleted(app: &mut PixmithApp, ui: &mut Ui, c: &Colors, folder: &Path, acts: &mut Vec<Act>) {
    let trash = app.projects.view.trash.clone();
    if trash.is_empty() {
        return;
    }
    ui.add_space(space::XL);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, radius::CONTROL, c.hover.gamma_multiply(0.5));
    }
    let open = app.projects.view.trash_open;
    ui.painter().text(
        pos2(rect.left() + 12.0, rect.center().y),
        Align2::CENTER_CENTER,
        if open { icon::CARET_DOWN } else { icon::CARET_RIGHT },
        egui::FontId::proportional(11.0),
        c.text_secondary,
    );
    let title = "Recently deleted".to_string();
    ui.painter().text(
        pos2(rect.left() + 28.0, rect.center().y),
        Align2::LEFT_CENTER,
        &title,
        widgets::strong(ui.ctx(), font::BODY_LARGE),
        c.text,
    );
    let w = ui.painter().layout_no_wrap(title, widgets::strong(ui.ctx(), font::BODY_LARGE), c.text).size().x;
    ui.painter().text(
        pos2(rect.left() + 28.0 + w + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        trash.len().to_string(),
        ui_font(font::BODY),
        c.text_tertiary,
    );
    if response.clicked() {
        app.projects.view.trash_open = !open;
    }
    if !open {
        return;
    }
    let _ = folder;
    for (path, modified) in trash {
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
        ui.painter().text(
            pos2(row.left() + 12.0, row.center().y),
            Align2::CENTER_CENTER,
            icon::FILE,
            egui::FontId::proportional(15.0),
            c.text_tertiary,
        );
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        ui.painter().text(
            pos2(row.left() + 32.0, row.center().y - 8.0),
            Align2::LEFT_CENTER,
            name,
            ui_font(font::BODY_LARGE),
            c.text,
        );
        ui.painter().text(
            pos2(row.left() + 32.0, row.center().y + 9.0),
            Align2::LEFT_CENTER,
            format!("Edited {}", cards::ago(modified)),
            ui_font(font::CAPTION),
            c.text_secondary,
        );
        let button = Rect::from_center_size(pos2(row.right() - 48.0, row.center().y), vec2(80.0, 28.0));
        let mut child = ui.new_child(UiBuilder::new().max_rect(button));
        if home::workspace_button(&mut child, "Restore").clicked() {
            acts.push(Act::Restore(path.clone()));
        }
        ui.painter().hline(row.x_range(), row.bottom(), Stroke::new(1.0, c.separator_soft));
    }
}

/// The Undo toast at the bottom of the window.
fn toast(app: &mut PixmithApp, ctx: &egui::Context, c: &Colors) {
    let Some(t) = app.projects.view.toast.clone() else { return };
    if Instant::now() >= t.until {
        app.projects.view.toast = None;
        return;
    }
    ctx.request_repaint_after(t.until.saturating_duration_since(Instant::now()).min(Duration::from_millis(250)));
    let mut undo = false;
    let mut dismiss = false;
    egui::Area::new(egui::Id::new("canvas_toast"))
        .order(Order::Foreground)
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(c.elevated)
                .stroke(Stroke::new(1.0, c.separator))
                .corner_radius(radius::PANEL)
                .inner_margin(Margin::symmetric(14, 8))
                .shadow(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(80) })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = space::MD;
                        ui.label(egui::RichText::new(&t.message).font(ui_font(font::BODY_LARGE)).color(c.text));
                        let r = ui.add(
                            egui::Label::new(
                                egui::RichText::new("Undo")
                                    .font(widgets::strong(ctx, font::BODY_LARGE))
                                    .color(c.accent),
                            )
                            .sense(Sense::click()),
                        );
                        if r.clicked() {
                            undo = true;
                        }
                        if widgets::icon_button_sized(ui, icon::X, 22.0, 10.0).clicked() {
                            dismiss = true;
                        }
                    });
                });
        });
    if undo {
        app.projects.view.toast = None;
        app.restore_canvas(&t.trashed, Some(t.index));
    } else if dismiss {
        app.projects.view.toast = None;
    }
}

fn run(app: &mut PixmithApp, folder: &Path, acts: Vec<Act>) {
    for act in acts {
        match act {
            Act::Open(path) => app.open_canvas(&path),
            Act::Select(path) => app.projects.view.selected = Some(path),
            Act::StartRename(path) => {
                let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                app.projects.view.selected = Some(path.clone());
                app.projects.view.renaming = Some((path, stem));
                app.projects.view.rename_fresh = true;
            }
            Act::CommitRename(path, text) => {
                let old = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if !text.trim().is_empty() && text.trim() != old {
                    app.rename_canvas(&path, &text);
                }
            }
            Act::Duplicate(path) => app.duplicate_canvas(&path),
            Act::Trash(path) => app.trash_canvas(&path),
            Act::Reveal(path) => super::reveal(&path),
            Act::ExportAseprite(path) => app.export_canvas_as_aseprite(&path),
            Act::Restore(trashed) => app.restore_canvas(&trashed, None),
            Act::Move { from, slot } => {
                let mut names = app.shown_names(folder);
                if from < names.len() {
                    let moved = names.remove(from);
                    names.insert(if slot > from { slot - 1 } else { slot }.min(names.len()), moved);
                    app.apply_order(folder, names);
                }
            }
            Act::NewCanvas => app.projects.new_canvas = Some(super::sheets::NewCanvas::new(folder, app.files.new_size)),
            Act::Import => app.import_into_project(folder),
            Act::Back => app.go_home(),
            Act::SetName(name) => match calls::set_project_name(folder, &name) {
                Ok(()) => app.rescan(folder),
                Err(e) => app.error = Some(e),
            },
        }
    }
}
