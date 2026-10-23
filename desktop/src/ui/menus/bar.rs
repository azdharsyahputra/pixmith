//! The menu bar: a compact row of menus at the left of the title row. Windows and Linux have no global menu bar,
//! and on macOS the same in-window menu keeps the two front ends alike. When the window is too narrow for twelve
//! titles they fold into one menu button.

use super::commands::{Cmd, Entry, MENUS, entries};
use crate::app::PixmithApp;
use crate::theme::{Colors, font, ui_font};
use eframe::egui::{self, Button, RichText, Style, Ui, vec2};
use egui_phosphor::regular as icon;

/// Horizontal padding inside a menu title.
const PAD: f32 = 8.0;
/// The room the tabs keep before the menus fold into one button.
const MIN_TABS: f32 = 260.0;
/// Width of the folded menu button.
const FOLDED: f32 = 32.0;

/// The width the bar takes in a row `available` points wide.
pub fn width(ui: &Ui, available: f32) -> f32 {
    let natural = natural_width(ui);
    if available - natural >= MIN_TABS { natural } else { FOLDED }
}

fn natural_width(ui: &Ui) -> f32 {
    let c = Colors::of(ui.ctx());
    MENUS
        .iter()
        .map(|name| {
            ui.painter().layout_no_wrap((*name).to_owned(), ui_font(font::BODY), c.text).size().x + 2.0 * PAD + 2.0
        })
        .sum()
}

/// Draws the bar into `ui` (a row of the width [`width`] returned) and runs the command that was picked.
pub fn show(app: &mut PixmithApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let folded = ui.max_rect().width() < natural_width(ui);
    let mut chosen = None;
    {
        let app: &PixmithApp = app;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().button_padding = vec2(PAD, 4.0);
            let style = |s: &mut Style| {
                egui::containers::menu::menu_style(s);
                s.spacing.button_padding = vec2(PAD, 4.0);
                s.spacing.item_spacing.x = 2.0;
            };
            egui::MenuBar::new().style(style).ui(ui, |ui| {
                if folded {
                    ui.menu_button(RichText::new(icon::LIST).size(15.0), |ui| {
                        for name in MENUS {
                            ui.menu_button(name, |ui| body(ui, app, &entries(name, app), &mut chosen));
                        }
                    });
                } else {
                    for name in MENUS {
                        ui.menu_button(name, |ui| body(ui, app, &entries(name, app), &mut chosen));
                    }
                }
            });
        });
    }
    if let Some(cmd) = chosen {
        cmd.run(app, &ctx);
    }
}

/// The items of one menu or submenu.
fn body(ui: &mut Ui, app: &PixmithApp, items: &[Entry], chosen: &mut Option<Cmd>) {
    ui.set_min_width(230.0);
    ui.spacing_mut().button_padding = vec2(PAD, 3.0);
    ui.spacing_mut().item_spacing.y = 1.0;
    let has_state = items.iter().any(|e| matches!(e, Entry::Item(c) if c.checked(app).is_some()));
    for entry in items {
        match entry {
            Entry::Item(cmd) => {
                let label = cmd.label(app);
                let text = match (cmd.checked(app), has_state) {
                    (Some(true), _) => format!("{}  {label}", icon::CHECK),
                    (_, true) => format!("\u{2003}  {label}"),
                    _ => label,
                };
                let mut button = Button::new(text);
                if let Some(shortcut) = cmd.shortcut() {
                    button = button.shortcut_text(ui.ctx().format_shortcut(&shortcut));
                }
                if ui.add_enabled(cmd.enabled(app), button).clicked() {
                    *chosen = Some(*cmd);
                    ui.close();
                }
            }
            Entry::Sep => {
                ui.add_space(2.0);
                ui.separator();
                ui.add_space(2.0);
            }
            Entry::Sub(title, inner) => {
                ui.menu_button(title.as_str(), |ui| body(ui, app, inner, chosen));
            }
            Entry::Note(text) => {
                ui.add_enabled(false, Button::new(*text));
            }
        }
    }
}
