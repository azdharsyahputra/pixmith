//! The project switcher in the editor's title row, before the tabs: the name of the project the active
//! document lives in with a chevron. Its menu lists the project's canvases (click to open one in a tab) and
//! "Back to Home". A document outside any project gets a plain Home button.

use super::cards;
use crate::app::PixmithApp;
use crate::theme::{Colors, font, metric, radius, space, ui_font};
use crate::ui::widgets;
use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, Ui, pos2, vec2};
use egui_phosphor::regular as icon;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// More canvases than this get a search field in the menu.
const SEARCH_FROM: usize = 8;
const MENU_WIDTH: f32 = 280.0;
const MAX_NAME_WIDTH: f32 = 180.0;
const RECHECK: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct SwitcherState {
    /// Which project the document at `.0` belongs to (checked again every few seconds).
    cache: Option<(PathBuf, Option<PathBuf>, Instant)>,
    filter: String,
    was_open: bool,
}

/// The project folder of the active tab's document, if it has one.
pub fn current_project(app: &mut PixmithApp) -> Option<PathBuf> {
    let path = app.tabs.get(app.active)?.doc.path.clone()?;
    if let Some((p, project, at)) = &app.projects.switcher.cache
        && *p == path
        && at.elapsed() < RECHECK
    {
        return project.clone();
    }
    let project = super::project_folder_of(&path);
    app.projects.switcher.cache = Some((path, project.clone(), Instant::now()));
    project
}

/// What the chip shows: the glyph, the label and whether it opens a menu.
fn chip(app: &mut PixmithApp) -> (String, &'static str, bool) {
    match current_project(app) {
        Some(folder) => (super::home::project_name(&app.projects, &folder), icon::FOLDER_SIMPLE, true),
        None => ("Home".into(), icon::HOUSE, false),
    }
}

/// The width the switcher takes in the title row, divider included.
pub fn width(app: &mut PixmithApp, ui: &Ui) -> f32 {
    let (label, _, menu) = chip(app);
    let c = Colors::of(ui.ctx());
    let text = ui
        .painter()
        .layout_no_wrap(label, widgets::strong(ui.ctx(), font::BODY_LARGE), c.text)
        .size()
        .x
        .min(MAX_NAME_WIDTH);
    space::SM + 14.0 + 6.0 + text + if menu { 6.0 + 9.0 } else { 0.0 } + space::SM + space::SM + 1.0 + space::XS
}

pub fn show(app: &mut PixmithApp, ui: &mut Ui) {
    let c = Colors::of(ui.ctx());
    let (label, glyph, menu) = chip(app);
    let folder = current_project(app);
    if let Some(f) = &folder {
        app.projects.scans.ensure(ui.ctx(), f);
    }
    let font = widgets::strong(ui.ctx(), font::BODY_LARGE);
    let galley = cards::fit_text(ui, &label, font, c.text, MAX_NAME_WIDTH);
    let chip_w = space::SM + 14.0 + 6.0 + galley.size().x + if menu { 6.0 + 9.0 } else { 0.0 } + space::SM;
    let (rect, response) = ui.allocate_exact_size(vec2(chip_w, metric::ICON_BUTTON), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let fill = if response.is_pointer_button_down_on() || open {
        c.pressed
    } else if response.hovered() {
        c.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, radius::CONTROL, fill);
    ui.painter().text(
        pos2(rect.left() + space::SM + 7.0, rect.center().y),
        Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(14.0),
        c.text_secondary,
    );
    let text_left = rect.left() + space::SM + 14.0 + 6.0;
    ui.painter().galley(pos2(text_left, rect.center().y - galley.size().y / 2.0), galley.clone(), c.text);
    if menu {
        ui.painter().text(
            pos2(text_left + galley.size().x + 6.0 + 4.5, rect.center().y),
            Align2::CENTER_CENTER,
            icon::CARET_DOWN,
            egui::FontId::proportional(9.0),
            c.text_tertiary,
        );
    }
    // A hairline between the switcher and the tabs.
    let x = rect.right() + space::SM;
    ui.painter().vline(x, (rect.center().y - 8.0)..=(rect.center().y + 8.0), Stroke::new(1.0, c.separator));
    ui.add_space(space::SM + 1.0 + space::XS);

    let Some(folder) = folder.filter(|_| menu) else {
        if response.on_hover_text("Back to Home (Ctrl+Shift+H)").clicked() {
            app.go_home();
        }
        return;
    };
    let response = response.on_hover_text(folder.display().to_string());
    if open && !app.projects.switcher.was_open {
        app.projects.scans.request(ui.ctx(), &folder);
        app.projects.switcher.filter.clear();
    }
    app.projects.switcher.was_open = open;
    let mut chosen: Option<PathBuf> = None;
    let mut home = false;
    egui::Popup::menu(&response).width(MENU_WIDTH).show(|ui| {
        menu_contents(app, ui, &folder, &mut chosen, &mut home);
    });
    if let Some(path) = chosen {
        app.open_canvas(&path);
    }
    if home {
        app.go_home();
    }
}

fn menu_contents(app: &mut PixmithApp, ui: &mut Ui, folder: &Path, chosen: &mut Option<PathBuf>, home: &mut bool) {
    let c = Colors::of(ui.ctx());
    ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
    ui.set_min_width(MENU_WIDTH - 16.0);
    let project = app.projects.scans.project(folder);
    let current = app.tabs.get(app.active).and_then(|t| t.doc.path.clone());
    match project {
        None => {
            ui.label(egui::RichText::new("Loading\u{2026}").font(ui_font(font::BODY)).color(c.text_secondary));
        }
        Some(p) if p.canvases.is_empty() => {
            ui.label(egui::RichText::new("No canvases").font(ui_font(font::BODY)).color(c.text_secondary));
        }
        Some(p) => {
            if p.canvases.len() > SEARCH_FROM {
                let mut filter = std::mem::take(&mut app.projects.switcher.filter);
                super::home::search_field(ui, "switcher_search", &mut filter, MENU_WIDTH - 16.0, "Search canvases");
                app.projects.switcher.filter = filter;
                ui.add_space(space::XS);
            }
            let needle = app.projects.switcher.filter.trim().to_lowercase();
            egui::ScrollArea::vertical().max_height(320.0).auto_shrink([true, true]).show(ui, |ui| {
                let mut shown = 0;
                for canvas in
                    p.canvases.iter().filter(|cv| needle.is_empty() || cv.name.to_lowercase().contains(&needle))
                {
                    shown += 1;
                    let is_current = current.as_deref() == Some(canvas.path.as_path());
                    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &canvas.name));
                    if is_current {
                        ui.painter().rect_filled(rect, radius::CONTROL, c.selected);
                    } else if response.hovered() {
                        ui.painter().rect_filled(rect, radius::CONTROL, c.hover);
                    }
                    if is_current {
                        ui.painter().text(
                            pos2(rect.left() + 14.0, rect.center().y),
                            Align2::CENTER_CENTER,
                            icon::CHECK,
                            egui::FontId::proportional(12.0),
                            c.accent,
                        );
                    }
                    let size = if canvas.error.is_some() {
                        String::new()
                    } else {
                        format!("{} \u{00D7} {}", canvas.width, canvas.height)
                    };
                    let size_w =
                        ui.painter().layout_no_wrap(size.clone(), ui_font(font::CAPTION), c.text_secondary).size().x;
                    ui.painter().text(
                        pos2(rect.right() - 10.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        size,
                        ui_font(font::CAPTION),
                        c.text_secondary,
                    );
                    let name = cards::fit_text(
                        ui,
                        &canvas.name,
                        ui_font(font::BODY),
                        c.text,
                        rect.width() - 28.0 - size_w - 18.0,
                    );
                    ui.painter().galley(pos2(rect.left() + 28.0, rect.center().y - name.size().y / 2.0), name, c.text);
                    if response.clicked() {
                        if !is_current {
                            *chosen = Some(canvas.path.clone());
                        }
                        ui.close();
                    }
                }
                if shown == 0 {
                    ui.label(egui::RichText::new("No matches").font(ui_font(font::BODY)).color(c.text_secondary));
                }
            });
        }
    }
    ui.add_space(space::XS);
    ui.separator();
    ui.add_space(space::XS);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Back to Home"));
    if response.hovered() {
        ui.painter().rect_filled(rect, radius::CONTROL, c.hover);
    }
    ui.painter().text(
        pos2(rect.left() + 14.0, rect.center().y),
        Align2::CENTER_CENTER,
        icon::HOUSE,
        egui::FontId::proportional(13.0),
        c.text_secondary,
    );
    ui.painter().text(
        pos2(rect.left() + 28.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Back to Home",
        ui_font(font::BODY),
        c.text,
    );
    ui.painter().text(
        pos2(rect.right() - 10.0, rect.center().y),
        Align2::RIGHT_CENTER,
        ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::H,
        )),
        ui_font(font::CAPTION),
        c.text_tertiary,
    );
    if response.clicked() {
        *home = true;
        ui.close();
    }
    let _ = Rect::NOTHING;
}
