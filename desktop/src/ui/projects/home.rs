//! The Home screen: a sidebar (Recent, Projects, New Project) and the main area with a strip of recent
//! canvases and a grid of projects with cover thumbnails, a search box and a sort menu.

use super::cards::{self, Grid};
use super::jobs::{ThumbKey, Thumbs};
use super::{Nav, ProjectsState, Screen, Sort};
use crate::app::PixmithApp;
use crate::theme::{Colors, font, metric, radius, space, ui_font};
use crate::ui::widgets;
use eframe::egui::{
    self, Align, Align2, Color32, Layout, Margin, Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use egui_phosphor::regular as icon;
use std::path::{Path, PathBuf};

pub const SIDEBAR_WIDTH: f32 = 232.0;
/// Content inset of the main area.
const INSET_X: f32 = 32.0;
const INSET_TOP: f32 = 24.0;
/// The recent-canvases strip uses smaller cards than the grid.
const STRIP_CARD_W: f32 = 176.0;
const STRIP_THUMB_H: f32 = 124.0;
const SIDEBAR_PROJECTS: usize = 8;

// --- Shared controls ---------------------------------------------------------------------------------------

/// A full-width button: accent-filled when `primary`, else in the field style. `icon` sits left of the label.
pub fn wide_button(ui: &mut Ui, label: &str, glyph: &str, primary: bool) -> Response {
    let c = Colors::of(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let fill = match (primary, response.is_pointer_button_down_on(), response.hovered()) {
        (true, true, _) => c.accent.gamma_multiply(0.75),
        (true, false, true) => c.accent.gamma_multiply(0.92),
        (true, false, false) => c.accent,
        (false, true, _) => c.pressed,
        (false, false, true) => c.hover,
        (false, false, false) => c.field,
    };
    ui.painter().rect_filled(rect, radius::CONTROL, fill);
    let fg = if primary { Color32::WHITE } else { c.text };
    let font = if primary { widgets::strong(ui.ctx(), font::BODY_LARGE) } else { ui_font(font::BODY_LARGE) };
    let galley = ui.painter().layout_no_wrap(label.into(), font, fg);
    let group = 14.0 + 8.0 + galley.size().x;
    let left = rect.center().x - group / 2.0;
    ui.painter().text(
        pos2(left + 7.0, rect.center().y),
        Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(14.0),
        fg,
    );
    ui.painter().galley(pos2(left + 22.0, rect.center().y - galley.size().y / 2.0), galley, fg);
    response
}

/// The fill of a control that sits on the workspace color: the field color in dark, white in light (the field
/// color is nearly the workspace color there). Light controls also get a hairline.
pub fn workspace_field(ui: &Ui, rect: Rect, r: u8) {
    let c = Colors::of(ui.ctx());
    if ui.visuals().dark_mode {
        ui.painter().rect_filled(rect, r, c.field);
    } else {
        ui.painter().rect_filled(rect, r, c.panel2);
        ui.painter().rect_stroke(rect, r, Stroke::new(1.0, c.separator), StrokeKind::Inside);
    }
}

/// A secondary button for the workspace area (`DSSecondaryButtonStyle` with [`workspace_field`]).
pub fn workspace_button(ui: &mut Ui, text: &str) -> Response {
    let c = Colors::of(ui.ctx());
    let font = ui_font(font::BODY_LARGE);
    let width = ui.painter().layout_no_wrap(text.into(), font.clone(), c.text).size().x;
    let (rect, response) = ui.allocate_exact_size(vec2((width + 2.0 * space::MD).max(64.0), 28.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    workspace_field(ui, rect, radius::CONTROL);
    if response.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, radius::CONTROL, c.pressed.gamma_multiply(0.5));
    } else if response.hovered() {
        ui.painter().rect_filled(rect, radius::CONTROL, c.hover.gamma_multiply(0.5));
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, font, c.text);
    response
}

/// The sort menu (a dropdown on the workspace). Returns the chosen index.
pub fn sort_popup(ui: &mut Ui, selected: usize, options: &[&str], width: f32) -> Option<usize> {
    let c = Colors::of(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(width, metric::CONTROL), Sense::click());
    workspace_field(ui, rect, radius::FIELD);
    if response.hovered() {
        ui.painter().rect_filled(rect, radius::FIELD, c.hover.gamma_multiply(0.4));
    }
    ui.painter().text(
        pos2(rect.left() + space::SM, rect.center().y),
        Align2::LEFT_CENTER,
        options[selected],
        ui_font(font::BODY),
        c.text,
    );
    ui.painter().text(
        pos2(rect.right() - space::SM - 4.0, rect.center().y),
        Align2::CENTER_CENTER,
        icon::CARET_DOWN,
        egui::FontId::proportional(9.0),
        c.text_tertiary,
    );
    let response = response.on_hover_text("Sort projects");
    let mut chosen = None;
    egui::Popup::menu(&response).width(width).show(|ui| {
        for (i, o) in options.iter().enumerate() {
            if ui.selectable_label(i == selected, *o).clicked() {
                chosen = Some(i);
            }
        }
    });
    chosen
}

/// A search field with a magnifier (the field style of `dsTextField`). Returns whether the text changed.
pub fn search_field(ui: &mut Ui, id: &str, text: &mut String, width: f32, hint: &str) -> bool {
    let c = Colors::of(ui.ctx());
    let id = ui.id().with(id);
    let (rect, _) = ui.allocate_exact_size(vec2(width, metric::CONTROL), Sense::hover());
    let focused = ui.memory(|m| m.has_focus(id));
    workspace_field(ui, rect, radius::FIELD);
    if focused {
        ui.painter().rect_stroke(rect, radius::FIELD, Stroke::new(1.5, c.accent_line), StrokeKind::Middle);
    }
    ui.painter().text(
        pos2(rect.left() + space::SM + 6.0, rect.center().y),
        Align2::CENTER_CENTER,
        icon::MAGNIFYING_GLASS,
        egui::FontId::proportional(12.0),
        c.text_tertiary,
    );
    let clear = !text.is_empty();
    let edit_rect = Rect::from_min_max(
        pos2(rect.left() + space::SM + 18.0, rect.top()),
        pos2(rect.right() - space::SM - if clear { 16.0 } else { 0.0 }, rect.bottom()),
    );
    let before = text.clone();
    let edit = egui::TextEdit::singleline(text)
        .id(id)
        .hint_text(egui::RichText::new(hint).color(c.text_tertiary))
        .font(ui_font(font::BODY))
        .frame(egui::Frame::NONE)
        .margin(Margin::ZERO)
        .vertical_align(Align::Center)
        .desired_width(edit_rect.width());
    widgets::put_in(ui, edit_rect, edit);
    if clear {
        let x_rect = Rect::from_center_size(pos2(rect.right() - space::SM - 6.0, rect.center().y), vec2(16.0, 16.0));
        let r = ui.interact(x_rect, id.with("clear"), Sense::click());
        ui.painter().text(
            x_rect.center(),
            Align2::CENTER_CENTER,
            icon::X_CIRCLE,
            egui::FontId::proportional(12.0),
            if r.hovered() { c.text } else { c.text_tertiary },
        );
        if r.clicked() {
            text.clear();
        }
    }
    *text != before
}

/// A section heading above a strip or grid: the title and a count.
pub fn section_title(ui: &mut Ui, title: &str, count: Option<usize>) {
    let c = Colors::of(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        ui.label(egui::RichText::new(title).font(widgets::strong(ui.ctx(), font::BODY_LARGE)).color(c.text));
        if let Some(n) = count {
            ui.label(egui::RichText::new(n.to_string()).font(ui_font(font::BODY)).color(c.text_tertiary));
        }
    });
    ui.add_space(space::MD);
}

/// What a card draws and how it behaves.
pub struct CardSpec<'a> {
    pub id: egui::Id,
    pub card: Rect,
    pub thumb: Rect,
    pub key: Option<&'a ThumbKey>,
    pub name: &'a str,
    /// What assistive technology and tests call the card (the name, also while it is being renamed).
    pub label: &'a str,
    pub details: &'a str,
    pub selected: bool,
    pub dim: bool,
    pub sense: Sense,
    /// Shown in the box when there is no thumbnail yet (a project without canvases).
    pub placeholder: &'a str,
}

/// One card: thumbnail box on a checkerboard, name and details under it. Draws only what is visible.
pub fn card(ui: &mut Ui, thumbs: &mut Thumbs, c: &Colors, dark: bool, spec: &CardSpec) -> Response {
    let response = ui.interact(spec.card, spec.id, spec.sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, spec.label));
    let hovered = response.hovered();
    cards::ring(ui, spec.thumb, c, spec.selected, hovered && !spec.selected);
    match spec.key {
        Some(key) => cards::thumb_for(ui, thumbs, key, spec.thumb, c, dark),
        None => {
            cards::paint_thumb(ui, spec.thumb, None, c, dark);
            ui.painter().text(
                spec.thumb.center(),
                Align2::CENTER_CENTER,
                spec.placeholder,
                egui::FontId::proportional(28.0),
                c.text_tertiary,
            );
        }
    }
    cards::caption(ui, c, spec.card, spec.thumb.bottom(), spec.name, spec.details, spec.dim);
    response
}

// --- Sidebar -----------------------------------------------------------------------------------------------

fn nav_row(ui: &mut Ui, glyph: &str, label: &str, selected: bool) -> Response {
    let c = Colors::of(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let fill = if selected {
        c.selected
    } else if response.is_pointer_button_down_on() {
        c.pressed
    } else if response.hovered() {
        c.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, radius::CONTROL, fill);
    let tint = if selected { c.accent } else { c.text_secondary };
    ui.painter().text(
        pos2(rect.left() + 18.0, rect.center().y),
        Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(15.0),
        tint,
    );
    let font = if selected { widgets::strong(ui.ctx(), font::BODY_LARGE) } else { ui_font(font::BODY_LARGE) };
    ui.painter().text(pos2(rect.left() + 36.0, rect.center().y), Align2::LEFT_CENTER, label, font, c.text);
    response
}

fn project_row(ui: &mut Ui, name: &str, selected: bool) -> Response {
    let c = Colors::of(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
    let fill = if selected {
        c.selected
    } else if response.hovered() {
        c.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, radius::CONTROL, fill);
    ui.painter().text(
        pos2(rect.left() + 18.0, rect.center().y),
        Align2::CENTER_CENTER,
        icon::FOLDER_SIMPLE,
        egui::FontId::proportional(14.0),
        c.text_tertiary,
    );
    let galley = cards::fit_text(ui, name, ui_font(font::BODY), c.text_secondary, rect.width() - 36.0 - 8.0);
    ui.painter().galley(pos2(rect.left() + 36.0, rect.center().y - galley.size().y / 2.0), galley, c.text_secondary);
    response
}

/// The left column of Home and of the project view: New Project, the Recent and Projects lists and Open Project.
pub fn sidebar(app: &mut PixmithApp, ui: &mut Ui) {
    let c = app.colors();
    let mut open: Option<PathBuf> = None;
    let mut nav: Option<Nav> = None;
    let mut new_project = false;
    let mut open_project = false;
    let on_home = app.projects.screen == Screen::Home;
    let current = match &app.projects.screen {
        Screen::Project(f) => Some(f.clone()),
        _ => None,
    };
    let inner = ui.available_rect_before_wrap().shrink2(vec2(space::LG, space::LG));
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, space::XS);
        if wide_button(ui, "New Project", icon::PLUS, true).clicked() {
            new_project = true;
        }
        ui.add_space(space::MD);
        if nav_row(ui, icon::CLOCK, "Recent", on_home && app.projects.nav == Nav::Recent).clicked() {
            nav = Some(Nav::Recent);
        }
        if nav_row(ui, icon::SQUARES_FOUR, "Projects", on_home && app.projects.nav == Nav::Projects).clicked() {
            nav = Some(Nav::Projects);
        }
        let list: Vec<(PathBuf, String)> = app
            .projects
            .recent
            .iter()
            .take(SIDEBAR_PROJECTS)
            .map(|r| (r.path.clone(), project_name(&app.projects, &r.path)))
            .collect();
        if !list.is_empty() {
            ui.add_space(space::LG);
            ui.label(egui::RichText::new("RECENT PROJECTS").font(ui_font(font::CAPTION)).color(c.text_tertiary));
            ui.add_space(space::XS);
            for (path, name) in &list {
                if project_row(ui, name, current.as_deref() == Some(path.as_path())).clicked() {
                    open = Some(path.clone());
                }
            }
        }
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, space::XS);
            if wide_button(ui, "Open Project\u{2026}", icon::FOLDER_OPEN, false).clicked() {
                open_project = true;
            }
        });
    });
    if let Some(n) = nav {
        app.projects.nav = n;
        app.go_home();
    }
    if let Some(p) = open {
        app.open_project(&p);
    }
    if new_project {
        app.projects.new_project = Some(super::sheets::NewProject::new(&app.projects));
    }
    if open_project {
        app.open_project_dialog();
    }
}

/// The name a project is listed under: the one in its project file, else its folder's.
pub fn project_name(state: &ProjectsState, folder: &Path) -> String {
    state
        .scans
        .project(folder)
        .map(|p| p.name.clone())
        .filter(|n| !n.is_empty())
        .or_else(|| folder.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| folder.display().to_string())
}

// --- Main area ---------------------------------------------------------------------------------------------

/// One project in the grid.
struct Row {
    path: PathBuf,
    name: String,
    details: String,
    modified: u64,
    cover: Option<ThumbKey>,
    missing: bool,
}

fn rows(state: &ProjectsState) -> Vec<Row> {
    let needle = state.search.trim().to_lowercase();
    let mut rows: Vec<Row> = state
        .recent
        .iter()
        .map(|r| {
            let entry = state.scans.get(&r.path);
            let project = entry.and_then(|e| e.project.as_ref());
            let missing = entry.is_some_and(|e| e.project.is_none() && e.error.is_some() && !e.loading);
            let modified = project.map(|p| p.modified()).filter(|m| *m > 0).unwrap_or(r.opened);
            let details = match (project, missing) {
                (_, true) => "Folder not found".to_string(),
                (Some(p), _) => format!("{} \u{00B7} {}", cards::canvas_count(p.canvases.len()), cards::ago(modified)),
                (None, _) => String::new(),
            };
            let cover = project.and_then(|p| {
                p.canvases.iter().find(|c| c.error.is_none()).map(|c| ThumbKey {
                    path: c.path.clone(),
                    modified: c.modified,
                    size: c.size,
                })
            });
            Row { path: r.path.clone(), name: project_name(state, &r.path), details, modified, cover, missing }
        })
        .filter(|r| needle.is_empty() || r.name.to_lowercase().contains(&needle))
        .collect();
    match state.sort {
        Sort::Name => rows.sort_by_key(|r| r.name.to_lowercase()),
        Sort::Modified => rows.sort_by_key(|r| std::cmp::Reverse(r.modified)),
    }
    rows
}

enum Act {
    OpenProject(PathBuf),
    OpenCanvas(PathBuf),
    Reveal(PathBuf),
    ForgetProject(PathBuf),
    ForgetCanvas(PathBuf),
    NewProject,
    OpenProjectDialog,
}

pub fn show(app: &mut PixmithApp, ui: &mut Ui) {
    let c = app.colors();
    let dark = app.dark;
    let mut acts: Vec<Act> = Vec::new();
    let title = if app.projects.nav == Nav::Recent { "Recent" } else { "Projects" };
    let frame = egui::Frame::new().inner_margin(Margin {
        left: INSET_X as i8,
        right: INSET_X as i8,
        top: INSET_TOP as i8,
        bottom: 0,
    });
    frame.show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        // Header: title on the left, sort and search on the right.
        let height = 32.0;
        let (header, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        ui.scope_builder(UiBuilder::new().max_rect(header).layout(Layout::left_to_right(Align::Center)), |ui| {
            ui.label(egui::RichText::new(title).font(widgets::strong(ui.ctx(), 20.0)).color(c.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = space::SM;
                if let Some(i) = sort_popup(ui, app.projects.sort.index(), &Sort::LABELS, 138.0) {
                    app.projects.sort = Sort::from_index(i);
                }
                search_field(ui, "home_search", &mut app.projects.search, 220.0, "Search");
            });
        });
        ui.add_space(space::LG);

        let projects = rows(&app.projects);
        let needle = app.projects.search.trim().to_lowercase();
        let strip: Vec<usize> = if app.projects.nav == Nav::Recent {
            (0..app.projects.recent_canvases.len())
                .filter(|i| {
                    needle.is_empty()
                        || app.projects.recent_canvases[*i]
                            .path
                            .file_stem()
                            .is_some_and(|s| s.to_string_lossy().to_lowercase().contains(&needle))
                })
                .collect()
        } else {
            Vec::new()
        };

        if projects.is_empty() && strip.is_empty() {
            empty_state(ui, &c, !app.projects.recent.is_empty() || !needle.is_empty(), &mut acts);
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !strip.is_empty() {
                section_title(ui, "Recent canvases", Some(strip.len()));
                recent_strip(app, ui, &c, dark, &strip, &mut acts);
                ui.add_space(space::XL);
            }
            if !projects.is_empty() {
                section_title(ui, "Projects", Some(projects.len()));
                project_grid(app, ui, &c, dark, &projects, &mut acts);
            }
            ui.add_space(space::XL);
        });
    });
    for act in acts {
        match act {
            Act::OpenProject(p) => app.open_project(&p),
            Act::OpenCanvas(p) => app.open_canvas(&p),
            Act::Reveal(p) => super::reveal(&p),
            Act::ForgetProject(p) => app.projects.forget_project(&p),
            Act::ForgetCanvas(p) => {
                app.menus.recents.retain(|r| *r != p);
                app.projects.recent_canvases.retain(|r| r.path != p);
            }
            Act::NewProject => app.projects.new_project = Some(super::sheets::NewProject::new(&app.projects)),
            Act::OpenProjectDialog => app.open_project_dialog(),
        }
    }
}

fn empty_state(ui: &mut Ui, c: &Colors, searching: bool, acts: &mut Vec<Act>) {
    let rect = ui.available_rect_before_wrap();
    let mid = rect.center();
    let (title, body) = if searching {
        ("Nothing matches", "Try another name, or clear the search.")
    } else {
        ("No projects yet", "A project is a folder of canvases. Create one, or open a folder you already have.")
    };
    let icon_pos = pos2(mid.x, mid.y - 70.0);
    ui.painter().text(
        icon_pos,
        Align2::CENTER_CENTER,
        if searching { icon::MAGNIFYING_GLASS } else { icon::FOLDERS },
        egui::FontId::proportional(40.0),
        c.text_tertiary,
    );
    ui.painter().text(pos2(mid.x, mid.y - 30.0), Align2::CENTER_CENTER, title, widgets::strong(ui.ctx(), 17.0), c.text);
    let galley = ui.painter().layout(body.into(), ui_font(font::BODY_LARGE), c.text_secondary, 380.0);
    ui.painter().galley(pos2(mid.x - galley.size().x / 2.0, mid.y - 8.0), galley.clone(), c.text_secondary);
    if !searching {
        let y = mid.y - 8.0 + galley.size().y + 18.0;
        let area = Rect::from_center_size(pos2(mid.x, y + 14.0), vec2(300.0, 32.0));
        ui.scope_builder(UiBuilder::new().max_rect(area).layout(Layout::left_to_right(Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            ui.allocate_ui(vec2(144.0, 32.0), |ui| {
                if wide_button(ui, "New Project", icon::PLUS, true).clicked() {
                    acts.push(Act::NewProject);
                }
            });
            ui.allocate_ui(vec2(148.0, 32.0), |ui| {
                if wide_button(ui, "Open Project\u{2026}", icon::FOLDER_OPEN, false).clicked() {
                    acts.push(Act::OpenProjectDialog);
                }
            });
        });
    }
}

fn recent_strip(app: &mut PixmithApp, ui: &mut Ui, c: &Colors, dark: bool, shown: &[usize], acts: &mut Vec<Act>) {
    let card_h = STRIP_THUMB_H + cards::TEXT_GAP + cards::TEXT_HEIGHT;
    let gap = 16.0;
    egui::ScrollArea::horizontal()
        .id_salt("recent_strip")
        .auto_shrink([false, true])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show(ui, |ui| {
            let total = shown.len() as f32 * (STRIP_CARD_W + gap) - gap;
            let (area, _) = ui.allocate_exact_size(vec2(total.max(ui.available_width()), card_h + 6.0), Sense::hover());
            for (n, &i) in shown.iter().enumerate() {
                let x = area.left() + n as f32 * (STRIP_CARD_W + gap);
                let thumb = Rect::from_min_size(pos2(x, area.top() + 3.0), vec2(STRIP_CARD_W, STRIP_THUMB_H));
                let card_rect = Rect::from_min_size(thumb.min, vec2(STRIP_CARD_W, card_h));
                if !ui.is_rect_visible(card_rect.expand(8.0)) {
                    continue;
                }
                let rc = app.projects.recent_canvases[i].clone();
                let name = rc.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let folder = rc
                    .path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let details = format!("{folder} \u{00B7} {}", cards::ago(rc.modified));
                let spec = CardSpec {
                    id: ui.id().with(("recent_canvas", &rc.path)),
                    card: card_rect,
                    thumb,
                    key: Some(&rc.key),
                    name: &name,
                    label: &name,
                    details: &details,
                    selected: false,
                    dim: false,
                    sense: Sense::click(),
                    placeholder: icon::IMAGE,
                };
                let r = card(ui, &mut app.projects.thumbs, c, dark, &spec).on_hover_text(rc.path.display().to_string());
                if r.clicked() {
                    acts.push(Act::OpenCanvas(rc.path.clone()));
                }
                r.context_menu(|ui| {
                    if ui.button("Open").clicked() {
                        acts.push(Act::OpenCanvas(rc.path.clone()));
                        ui.close();
                    }
                    if ui.button(reveal_label()).clicked() {
                        acts.push(Act::Reveal(rc.path.clone()));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Remove from Recents").clicked() {
                        acts.push(Act::ForgetCanvas(rc.path.clone()));
                        ui.close();
                    }
                });
            }
        });
}

/// "Show in Finder" on macOS, "Show in Explorer" on Windows, "Show in Folder" elsewhere.
pub fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Show in Finder"
    } else if cfg!(windows) {
        "Show in Explorer"
    } else {
        "Show in Folder"
    }
}

fn project_grid(app: &mut PixmithApp, ui: &mut Ui, c: &Colors, dark: bool, rows: &[Row], acts: &mut Vec<Act>) {
    let grid = Grid::new(ui.available_width());
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), grid.height(rows.len()) + 6.0), Sense::hover());
    let origin = area.min + vec2(0.0, 3.0);
    let visible = grid.visible(origin, ui.clip_rect().expand(grid.row_h), rows.len());
    for i in visible {
        let row = &rows[i];
        let spec = CardSpec {
            id: ui.id().with(("project", &row.path)),
            card: grid.card_rect(origin, i),
            thumb: grid.thumb_rect(origin, i),
            key: row.cover.as_ref(),
            name: &row.name,
            label: &row.name,
            details: &row.details,
            selected: false,
            dim: row.missing,
            sense: Sense::click(),
            placeholder: if row.missing { icon::WARNING } else { icon::SQUARES_FOUR },
        };
        let r = card(ui, &mut app.projects.thumbs, c, dark, &spec).on_hover_text(row.path.display().to_string());
        if r.clicked() {
            acts.push(Act::OpenProject(row.path.clone()));
        }
        r.context_menu(|ui| {
            if ui.add_enabled(!row.missing, egui::Button::new("Open")).clicked() {
                acts.push(Act::OpenProject(row.path.clone()));
                ui.close();
            }
            if ui.add_enabled(!row.missing, egui::Button::new(reveal_label())).clicked() {
                acts.push(Act::Reveal(row.path.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button("Remove from Recents").clicked() {
                acts.push(Act::ForgetProject(row.path.clone()));
                ui.close();
            }
        });
    }
}
