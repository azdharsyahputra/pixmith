//! The Extensions tab of Preferences and the color theme row of the Appearance tab.

use super::{ExtensionsState, apply_theme, theme_choices};
use crate::app::PixmithApp;
use crate::theme::{Colors, font, radius, space, ui_font};
use crate::ui::menus::prefs::{note, row};
use crate::ui::widgets::{self, strong};
use eframe::egui::{self, Align, CornerRadius, Layout, Margin, Ui, vec2};
use pixmith_core::extensions::{self as core, Extension};

/// Gives a painted button its accessible name, so tests and screen readers can find it.
fn labeled(response: egui::Response, text: &str) -> egui::Response {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    response
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// "2 scripts, 1 palette, 1 theme", or nothing for an extension that contributes nothing.
fn summary(e: &Extension) -> String {
    let mut parts = Vec::new();
    if !e.manifest.scripts.is_empty() {
        parts.push(plural(e.manifest.scripts.len(), "script"));
    }
    if !e.manifest.palettes.is_empty() {
        parts.push(plural(e.manifest.palettes.len(), "palette"));
    }
    if !e.manifest.themes.is_empty() {
        parts.push(plural(e.manifest.themes.len(), "theme"));
    }
    parts.join(", ")
}

/// The Extensions tab (`ExtensionsPrefsView`): the installed extensions with Enabled and Uninstall, then
/// Add Extension... and Open Folder.
pub fn tab(app: &mut PixmithApp, ui: &mut Ui) {
    let c = Colors::of(ui.ctx());
    let list = core::installed();
    let height = ui.available_height() - 68.0;
    let mut toggle: Option<(String, bool)> = None;
    let mut uninstall: Option<String> = None;
    egui::ScrollArea::vertical().max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        if list.is_empty() {
            ui.add_space(space::XL);
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new("No extensions installed")
                        .font(strong(ui.ctx(), font::BODY_LARGE))
                        .color(c.text),
                );
                ui.add_space(space::XS);
                note(ui, "Extensions add scripts, palettes and themes.");
                note(ui, "Add a .pixmith-extension file, or drop one on the window.");
            });
        }
        for e in list.iter() {
            card(ui, e, &mut app.extensions, &mut toggle, &mut uninstall);
        }
    });
    ui.add_space(space::SM);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        if labeled(widgets::primary_button(ui, "Add Extension\u{2026}", true), "Add Extension").clicked() {
            app.add_extension_dialog();
        }
        if labeled(widgets::secondary_button(ui, "Open Folder", false), "Open Folder").clicked() {
            app.open_extensions_folder();
        }
    });
    ui.add_space(space::XS);
    match app.extensions.message.clone() {
        Some((text, true)) => {
            ui.label(egui::RichText::new(text).font(ui_font(font::CAPTION)).color(c.destructive));
        }
        Some((text, false)) => note(ui, &text),
        None => note(ui, "Extensions can run scripts. Install only ones you trust."),
    }
    if let Some((name, enabled)) = toggle {
        app.extensions.message = core::set_enabled(&name, enabled).err().map(|e| (e, true));
    }
    if let Some(name) = uninstall {
        app.extensions.confirm = None;
        app.extensions.message = Some(match core::uninstall(&name) {
            Ok(()) => (format!("Uninstalled {name}"), false),
            Err(e) => (e, true),
        });
    }
}

fn card(
    ui: &mut Ui,
    e: &Extension,
    state: &mut ExtensionsState,
    toggle: &mut Option<(String, bool)>,
    uninstall: &mut Option<String>,
) {
    let c = Colors::of(ui.ctx());
    let name = e.manifest.name.clone();
    egui::Frame::new()
        .fill(c.panel2)
        .corner_radius(CornerRadius::same(radius::PANEL))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(space::SM, space::XS);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&e.manifest.display_name)
                        .font(strong(ui.ctx(), font::BODY_LARGE))
                        .color(c.text),
                );
                ui.label(egui::RichText::new(&e.manifest.version).font(ui_font(font::CAPTION)).color(c.text_tertiary));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut on = e.enabled;
                    if ui.checkbox(&mut on, "Enabled").changed() {
                        *toggle = Some((name.clone(), on));
                    }
                });
            });
            let mut by = String::new();
            if !e.manifest.author.is_empty() {
                by = format!("by {}", e.manifest.author);
            }
            let kinds = summary(e);
            let meta = [by, kinds].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("  \u{00B7}  ");
            if !meta.is_empty() {
                ui.label(egui::RichText::new(meta).font(ui_font(font::CAPTION)).color(c.text_secondary));
            }
            if !e.manifest.description.is_empty() {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&e.manifest.description).font(ui_font(font::BODY)).color(c.text_secondary),
                    )
                    .wrap(),
                );
            }
            if let Some(err) = &e.error {
                ui.add(
                    egui::Label::new(egui::RichText::new(err).font(ui_font(font::CAPTION)).color(c.destructive)).wrap(),
                );
            }
            ui.add_space(space::XS);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::SM;
                if state.confirm.as_deref() == Some(name.as_str()) {
                    ui.label(
                        egui::RichText::new(format!("Remove {}?", e.manifest.display_name))
                            .font(ui_font(font::BODY))
                            .color(c.text),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = space::SM;
                        if labeled(widgets::secondary_button(ui, "Uninstall", true), "Uninstall").clicked() {
                            *uninstall = Some(name.clone());
                        }
                        if labeled(widgets::secondary_button(ui, "Cancel", false), "Cancel").clicked() {
                            state.confirm = None;
                        }
                    });
                } else {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if labeled(widgets::secondary_button(ui, "Uninstall\u{2026}", true), "Uninstall\u{2026}")
                            .clicked()
                        {
                            state.confirm = Some(name.clone());
                        }
                    });
                }
            });
        });
    ui.add_space(space::SM);
}

/// The "Color theme" row of the Appearance tab: Default and the themes of the enabled extensions. Picking one
/// switches the look at once. A theme that only styles one appearance also moves Light/Dark to it, so the
/// choice is visible. Returns true when the choice changed; `appearance` may have changed with it.
pub fn theme_row(ui: &mut Ui, theme_id: &mut String, appearance: &mut u8, system_dark: bool) -> bool {
    let choices = theme_choices();
    let mut labels: Vec<String> = vec!["Default".into()];
    labels.extend(choices.iter().map(|t| t.name.clone()));
    let mut index = choices.iter().position(|t| &t.id == theme_id).map_or(0, |i| i + 1);
    let before = index;
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    row(ui, "Color theme", true, |ui| {
        widgets::popup(ui, &refs, &mut index, Some(190.0));
    });
    if index == before {
        return false;
    }
    *theme_id = if index == 0 { String::new() } else { choices[index - 1].id.clone() };
    if let Some(t) = choices.get(index.wrapping_sub(1)) {
        let dark_now = match *appearance {
            1 => false,
            2 => true,
            _ => system_dark,
        };
        if dark_now && !t.dark && t.light {
            *appearance = 1;
        } else if !dark_now && !t.light && t.dark {
            *appearance = 2;
        }
    }
    apply_theme(theme_id);
    true
}
