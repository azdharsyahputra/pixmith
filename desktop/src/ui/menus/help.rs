//! The Keyboard Shortcuts window (`ShortcutsHelpView` in `App/Shortcuts.swift`): the single keys, the keys that
//! cannot be changed and every menu item with a shortcut, filtered as you type.

use super::commands::{self, MENUS};
use super::shortcuts::Action;
use crate::app::PixmithApp;
use crate::theme::{Colors, font, radius, space, ui_font};
use crate::ui::widgets;
use eframe::egui::{self, Align, CornerRadius, Layout, Stroke, Ui, vec2};

/// Keys handled elsewhere that cannot be rebound.
fn fixed_shortcuts() -> Vec<(&'static str, &'static str)> {
    let mac = cfg!(target_os = "macos");
    vec![
        ("\u{2190} \u{2192} \u{2191} \u{2193}", "Nudge the selection or move tool (\u{21e7} moves 10 px)"),
        ("\u{232b}", "Delete the selected pixels"),
        ("\u{21a9}", "Commit a floating paste or open polygon"),
        ("\u{238b}", "Cancel polygon or paste, then deselect"),
        (if mac { "\u{2325} + click" } else { "Alt + click" }, "Pick a color from the canvas"),
        ("Right-click", "Draw with the secondary color"),
        ("\u{21e7} + drag", "Constrain lines, squares and circles"),
        (if mac { "\u{2318} + scroll, pinch" } else { "Ctrl + scroll, pinch" }, "Zoom the canvas"),
    ]
}

fn matches(filter: &str, s: &str) -> bool {
    filter.is_empty() || s.to_lowercase().contains(&filter.to_lowercase())
}

/// Draws the window when it is open; call every frame.
pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    if !app.menus.help_open {
        return;
    }
    let c = Colors::of(ctx);
    let frame = egui::Frame::new()
        .fill(c.panel)
        .stroke(Stroke::new(1.0, c.separator))
        .corner_radius(CornerRadius::same(radius::PANEL))
        .shadow(egui::Shadow { offset: [0, 6], blur: 20, spread: 0, color: egui::Color32::from_black_alpha(80) });
    let mut open = true;
    egui::Window::new("Keyboard Shortcuts")
        .id(egui::Id::new("shortcuts_window"))
        .collapsible(false)
        .default_pos([120.0, 72.0])
        .default_size([420.0, 520.0])
        .min_size([420.0, 480.0])
        .frame(frame)
        .open(&mut open)
        .show(ctx, |ui| {
            let mut filter = std::mem::take(&mut app.menus.help_filter);
            ui.add_space(space::MD);
            ui.horizontal(|ui| {
                ui.add_space(space::MD);
                widgets::ds_text_field(ui, "shortcut_filter", &mut filter, Some(ui.available_width() - space::MD));
            });
            ui.add_space(space::SM);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(ui.available_width());
                sections(app, ui, &filter);
            });
            app.menus.help_filter = filter;
        });
    if !open {
        app.menus.help_open = false;
    }
}

fn sections(app: &PixmithApp, ui: &mut Ui, filter: &str) {
    let s = &app.menus.shortcuts;
    for (title, tools) in [("Tools", true), ("Actions", false)] {
        let rows: Vec<(String, String)> = Action::all()
            .into_iter()
            .filter(|a| a.is_tool() == tools)
            .map(|a| (a.title(), s.label(a)))
            .filter(|(t, k)| matches(filter, t) || matches(filter, k))
            .collect();
        section(ui, title, &rows);
    }
    let fixed: Vec<(String, String)> = fixed_shortcuts()
        .into_iter()
        .filter(|(k, w)| matches(filter, w) || matches(filter, k))
        .map(|(k, w)| (w.to_string(), k.to_string()))
        .collect();
    section(ui, "Other keys", &fixed);
    for menu in MENUS {
        let mut cmds = Vec::new();
        commands::walk(&commands::entries(menu, app), &mut cmds);
        let rows: Vec<(String, String)> = cmds
            .into_iter()
            .filter_map(|c| c.shortcut().map(|sc| (c.label(app), ui.ctx().format_shortcut(&sc))))
            .filter(|(t, k)| matches(filter, t) || matches(filter, k) || matches(filter, menu))
            .collect();
        section(ui, &format!("{menu} menu"), &rows);
    }
}

fn section(ui: &mut Ui, title: &str, rows: &[(String, String)]) {
    if rows.is_empty() {
        return;
    }
    let c = Colors::of(ui.ctx());
    ui.add_space(space::SM);
    ui.horizontal(|ui| {
        ui.add_space(space::MD);
        ui.label(egui::RichText::new(title).font(widgets::strong(ui.ctx(), font::CAPTION)).color(c.text_secondary));
    });
    for (what, keys) in rows {
        ui.allocate_ui_with_layout(vec2(ui.available_width(), 28.0), Layout::left_to_right(Align::Center), |ui| {
            ui.add_space(space::MD);
            ui.label(egui::RichText::new(what).font(ui_font(font::BODY_LARGE)).color(c.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(space::MD);
                widgets::key_hint(ui, keys);
            });
        });
    }
}
