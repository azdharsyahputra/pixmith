//! Preferences (`App/Preferences.swift`, `App/Shortcuts.swift` Keyboard tab): the values, their storage text and
//! the Preferences window with its General, Appearance and Keyboard tabs.

use super::controls::color_well;
use super::shortcuts::{Action, Shortcuts};
use crate::app::PixmithApp;
use crate::theme::{self, Colors, font, metric, radius, space, ui_font};
use crate::ui::widgets::{self, SegGlyph};
use eframe::egui::{self, Align, Color32, CornerRadius, Layout, Sense, Stroke, Ui, vec2};

/// The preferred zoom values of the General tab.
pub const ZOOMS: [u32; 12] = [1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64];

/// User preferences. Colors are 0xRRGGBBAA; `None` colors follow the appearance like `Prefs.Default`.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub new_width: u32,
    pub new_height: u32,
    pub checker_light: Option<u32>,
    pub checker_dark: Option<u32>,
    pub grid_color: u32,
    pub default_zoom: u32,
    /// 0 follows the system, 1 light, 2 dark.
    pub appearance: u8,
    pub canvas_bg_on: bool,
    pub canvas_bg: u32,
    /// Id of the chosen color theme from an extension (`<extension>/<path>`); empty is the default look.
    pub theme: String,
    /// Allow AI agents to control Pixmith (Live mode, `docs/LIVE_PROTOCOL.md`).
    pub live_enabled: bool,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            new_width: 32,
            new_height: 32,
            checker_light: None,
            checker_dark: None,
            grid_color: 0x0000001F,
            default_zoom: 16,
            appearance: 0,
            canvas_bg_on: false,
            canvas_bg: 0x3C3C3CFF,
            theme: String::new(),
            live_enabled: true,
        }
    }
}

/// The checkerboard pair of an appearance (`Prefs.Default.checkerLight` / `checkerDark`).
pub fn default_checker(dark: bool) -> (u32, u32) {
    if dark { (0x3A3A3DFF, 0x2F2F32FF) } else { (0xF3F3F5FF, 0xDADADFFF) }
}

fn color32(v: u32) -> Color32 {
    widgets::rgba_color(v)
}

/// The pixel grid's color.
pub fn grid_color32(p: &Prefs) -> Color32 {
    color32(p.grid_color)
}

impl Prefs {
    pub fn checker(&self, dark: bool) -> (Color32, Color32) {
        let (l, d) = default_checker(dark);
        (color32(self.checker_light.unwrap_or(l)), color32(self.checker_dark.unwrap_or(d)))
    }

    /// The area around the sprite: the theme's color unless a custom one is chosen.
    pub fn canvas_background(&self) -> Option<Color32> {
        self.canvas_bg_on.then(|| color32(self.canvas_bg))
    }

    /// The zoom a new sprite opens at, reduced so that large sprites still fit (`Prefs.initialZoom`).
    pub fn default_zoom_for(&self, fit: f32) -> f32 {
        (self.default_zoom.clamp(1, 64) as f32).min(fit)
    }

    /// Whether the dark theme is wanted, given what the system uses.
    pub fn wants_dark(&self, system_dark: bool) -> bool {
        match self.appearance {
            1 => false,
            2 => true,
            _ => system_dark,
        }
    }

    pub fn encode(&self) -> String {
        let mut out = format!(
            "newWidth={}\nnewHeight={}\ngridColor={}\ndefaultZoom={}\nappearance={}\ncanvasBgOn={}\ncanvasBg={}\nliveEnabled={}\n",
            self.new_width,
            self.new_height,
            self.grid_color,
            self.default_zoom,
            self.appearance,
            u8::from(self.canvas_bg_on),
            self.canvas_bg,
            u8::from(self.live_enabled)
        );
        if let Some(v) = self.checker_light {
            out.push_str(&format!("checkerLight={v}\n"));
        }
        if let Some(v) = self.checker_dark {
            out.push_str(&format!("checkerDark={v}\n"));
        }
        if !self.theme.is_empty() {
            out.push_str(&format!("theme={}\n", self.theme.replace('\n', "")));
        }
        out
    }

    pub fn decode(text: &str) -> Prefs {
        let mut p = Prefs::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            if key == "theme" {
                p.theme = value.trim().to_string();
                continue;
            }
            let Ok(v) = value.trim().parse::<u32>() else { continue };
            match key {
                "newWidth" => p.new_width = v.clamp(1, 4096),
                "newHeight" => p.new_height = v.clamp(1, 4096),
                "gridColor" => p.grid_color = v,
                "defaultZoom" => p.default_zoom = v.clamp(1, 64),
                "appearance" => p.appearance = v.min(2) as u8,
                "canvasBgOn" => p.canvas_bg_on = v != 0,
                "canvasBg" => p.canvas_bg = v,
                "liveEnabled" => p.live_enabled = v != 0,
                "checkerLight" => p.checker_light = Some(v),
                "checkerDark" => p.checker_dark = Some(v),
                _ => {}
            }
        }
        p
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum PrefsTab {
    #[default]
    General,
    Appearance,
    Keyboard,
    Extensions,
}

// MARK: Window

/// Draws the Preferences window when it is open; call every frame.
pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    if !app.menus.prefs_open {
        return;
    }
    let c = Colors::of(ctx);
    let frame = egui::Frame::new()
        .fill(c.panel)
        .stroke(Stroke::new(1.0, c.separator))
        .corner_radius(CornerRadius::same(radius::PANEL + 2))
        .inner_margin(egui::Margin::same(16))
        .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(90) });
    let mut open = true;
    let mut dark_changed = None;
    egui::Window::new("Preferences")
        .id(egui::Id::new("preferences_window"))
        .collapsible(false)
        .resizable(false)
        .fixed_size(vec2(488.0, 640.0))
        .default_pos(ctx.content_rect().center() - vec2(244.0, 320.0))
        .frame(frame)
        .open(&mut open)
        .show(ctx, |ui| {
            let tab = app.menus.prefs_tab;
            ui.vertical_centered(|ui| {
                let options = [
                    (SegGlyph::Text("General"), "General"),
                    (SegGlyph::Text("Appearance"), "Appearance"),
                    (SegGlyph::Text("Keyboard"), "Keyboard"),
                    (SegGlyph::Text("Extensions"), "Extensions"),
                ];
                let selected = tab as usize;
                if let Some(i) = widgets::bar_segmented(ui, selected, &options) {
                    app.menus.prefs_tab =
                        [PrefsTab::General, PrefsTab::Appearance, PrefsTab::Keyboard, PrefsTab::Extensions][i];
                    if app.menus.prefs_tab == PrefsTab::Extensions {
                        pixmith_core::extensions::refresh();
                    }
                    app.menus.shortcuts.recording = None;
                    app.menus.shortcuts.conflict = None;
                }
            });
            ui.add_space(space::MD);
            match app.menus.prefs_tab {
                PrefsTab::General => general(app, ui),
                PrefsTab::Appearance => dark_changed = appearance(app, ui),
                PrefsTab::Keyboard => keyboard(&mut app.menus.shortcuts, ui),
                PrefsTab::Extensions => crate::ui::extensions::tab(app, ui),
            }
        });
    if let Some(dark) = dark_changed {
        app.dark = dark;
        theme::apply(ctx, dark);
    }
    if !open {
        app.menus.prefs_open = false;
        app.menus.shortcuts.recording = None;
        app.menus.shortcuts.conflict = None;
    }
}

/// A group of rows with a title above it (a grouped `Form` section).
pub(crate) fn group(ui: &mut Ui, title: &str, rows: impl FnOnce(&mut Ui)) {
    let c = Colors::of(ui.ctx());
    ui.label(egui::RichText::new(title).font(widgets::strong(ui.ctx(), font::CAPTION)).color(c.text_secondary));
    ui.add_space(-space::XS);
    egui::Frame::new()
        .fill(c.panel2)
        .corner_radius(CornerRadius::same(radius::PANEL))
        .inner_margin(egui::Margin::symmetric(12, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(space::SM, 0.0);
            rows(ui);
        });
    ui.add_space(space::SM);
}

/// A row: the label at the left, the control at the right.
pub(crate) fn row(ui: &mut Ui, label: &str, last: bool, control: impl FnOnce(&mut Ui)) {
    let c = Colors::of(ui.ctx());
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 32.0), Layout::left_to_right(Align::Center), |ui| {
        ui.label(egui::RichText::new(label).font(ui_font(font::BODY_LARGE)).color(c.text));
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
    if !last {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, c.separator_soft);
    }
}

pub(crate) fn note(ui: &mut Ui, text: &str) {
    let c = Colors::of(ui.ctx());
    ui.label(egui::RichText::new(text).font(ui_font(font::CAPTION)).color(c.text_secondary));
}

fn general(app: &mut PixmithApp, ui: &mut Ui) {
    let dark = app.dark;
    let mut p = app.menus.prefs.clone();
    group(ui, "New sprite", |ui| {
        row(ui, "Width", false, |ui| {
            widgets::number_field(ui, "W", &mut p.new_width, 1..=4096, "px", 110.0);
        });
        row(ui, "Height", true, |ui| {
            widgets::number_field(ui, "H", &mut p.new_height, 1..=4096, "px", 110.0);
        });
    });
    group(ui, "Canvas", |ui| {
        let (light, darker) = default_checker(dark);
        row(ui, "Checkerboard light", false, |ui| {
            let mut v = p.checker_light.unwrap_or(light);
            if color_well(ui, "checker_light", &mut v) {
                p.checker_light = Some(v | 0xFF);
            }
        });
        row(ui, "Checkerboard dark", false, |ui| {
            let mut v = p.checker_dark.unwrap_or(darker);
            if color_well(ui, "checker_dark", &mut v) {
                p.checker_dark = Some(v | 0xFF);
            }
        });
        row(ui, "Grid", false, |ui| {
            color_well(ui, "grid", &mut p.grid_color);
        });
        row(ui, "Default zoom", true, |ui| {
            let zooms: Vec<u32> = if ZOOMS.contains(&p.default_zoom) {
                ZOOMS.to_vec()
            } else {
                let mut z = ZOOMS.to_vec();
                z.push(p.default_zoom);
                z
            };
            let labels: Vec<String> = zooms.iter().map(|z| format!("{z}x")).collect();
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let mut index = zooms.iter().position(|&z| z == p.default_zoom).unwrap_or(0);
            widgets::popup(ui, &refs, &mut index, Some(90.0));
            p.default_zoom = zooms[index.min(zooms.len() - 1)];
        });
    });
    note(ui, "Large sprites open at a smaller zoom so they fit the window.");
    ui.add_space(space::SM);
    group(ui, "Live mode", |ui| {
        row(ui, "Allow AI agents to control Pixmith", true, |ui| {
            ui.checkbox(&mut p.live_enabled, "");
        });
    });
    note(
        ui,
        "Lets tools on this computer, such as the Pixmith MCP server, edit the open documents. Each edit is an undo step.",
    );
    ui.add_space(space::SM);
    group(ui, "Scripts: file access", |ui| {
        row(ui, "Forget allowed scripts", true, |ui| {
            if super::controls::bar_button(ui, "Forget", true, false).clicked() {
                crate::calls::script_access::forget_grants();
                app.gamedev.notice = Some(crate::ui::gamedev::Notice {
                    title: "Scripts".into(),
                    text: "Scripts will ask again before they use files or run commands.".into(),
                    warning: false,
                });
            }
        });
    });
    note(
        ui,
        "Scripts ask before they read or write files or run commands. \u{201c}Always Allow\u{201d} lasts until Pixmith quits.",
    );
    ui.add_space(space::SM);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        if super::controls::bar_button(ui, "Restore Defaults", true, false).clicked() {
            p = Prefs {
                appearance: p.appearance,
                canvas_bg_on: p.canvas_bg_on,
                canvas_bg: p.canvas_bg,
                theme: p.theme.clone(),
                live_enabled: p.live_enabled,
                ..Prefs::default()
            };
        }
    });
    if p != app.menus.prefs {
        if (p.new_width, p.new_height) != (app.menus.prefs.new_width, app.menus.prefs.new_height) {
            app.files.new_size = (p.new_width, p.new_height);
            app.menus.synced_new_size = app.files.new_size;
        }
        app.menus.prefs = p;
    }
}

/// The Appearance tab. Returns the new theme when the picker changed it.
fn appearance(app: &mut PixmithApp, ui: &mut Ui) -> Option<bool> {
    let mut p = app.menus.prefs.clone();
    let mut theme_change = None;
    group(ui, "Theme", |ui| {
        row(ui, "Appearance", false, |ui| {
            let options = [
                (SegGlyph::Text("System"), "System"),
                (SegGlyph::Text("Light"), "Light"),
                (SegGlyph::Text("Dark"), "Dark"),
            ];
            if let Some(i) = widgets::bar_segmented(ui, p.appearance as usize, &options) {
                p.appearance = i as u8;
                let system = ui.ctx().input(|i| i.raw.system_theme) != Some(egui::Theme::Light);
                theme_change = Some(p.wants_dark(system));
            }
        });
        let system = ui.ctx().input(|i| i.raw.system_theme) != Some(egui::Theme::Light);
        let mut id = p.theme.clone();
        if crate::ui::extensions::theme_row(ui, &mut id, &mut p.appearance, system) {
            p.theme = id;
            theme_change = Some(p.wants_dark(system));
        }
    });
    group(ui, "Canvas", |ui| {
        row(ui, "Custom canvas background", false, |ui| {
            ui.checkbox(&mut p.canvas_bg_on, "");
        });
        row(ui, "Background color", true, |ui| {
            ui.add_enabled_ui(p.canvas_bg_on, |ui| {
                let mut v = p.canvas_bg;
                if color_well(ui, "canvas_bg", &mut v) {
                    p.canvas_bg = v | 0xFF;
                }
            });
        });
    });
    note(ui, "The area around the sprite. Checkerboard and grid colors are under General.");
    app.menus.prefs = p;
    theme_change
}

/// The Keyboard tab (`KeyboardPrefsView`).
fn keyboard(s: &mut Shortcuts, ui: &mut Ui) {
    let c = Colors::of(ui.ctx());
    let height = ui.available_height() - 64.0;
    egui::ScrollArea::vertical().max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        let tools: Vec<Action> = Action::all().into_iter().filter(|a| a.is_tool()).collect();
        let actions: Vec<Action> = Action::all().into_iter().filter(|a| !a.is_tool()).collect();
        for (title, list) in [("Tools", tools), ("Actions", actions)] {
            group(ui, title, |ui| {
                let n = list.len();
                for (i, a) in list.into_iter().enumerate() {
                    row(ui, &a.title(), i + 1 == n, |ui| key_button(ui, s, a));
                }
            });
        }
    });
    ui.add_space(space::SM);
    if let Some(conflict) = s.conflict.clone() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(
                egui::RichText::new(format!(
                    "{} is already used by {}.",
                    conflict.binding.label(),
                    conflict.other.title()
                ))
                .font(ui_font(font::BODY))
                .color(Color32::from_rgb(0xFF, 0x9F, 0x0A)),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if super::controls::bar_button(ui, "Cancel", true, false).clicked() {
                    s.conflict = None;
                }
                if super::controls::bar_button(ui, "Reassign", true, false).clicked() {
                    s.resolve_conflict();
                }
            });
        });
    } else {
        ui.label(
            egui::RichText::new("Click a key, then press the new one. Delete unassigns, Esc cancels.")
                .font(ui_font(font::CAPTION))
                .color(c.text_secondary),
        );
    }
    ui.add_space(space::XS);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        if super::controls::bar_button(ui, "Restore Default Shortcuts", s.is_customized(), false).clicked() {
            s.reset();
        }
    });
}

/// The 120 pt key button of a row: the key, or "Press a key…" while recording.
fn key_button(ui: &mut Ui, s: &mut Shortcuts, a: Action) {
    let c = Colors::of(ui.ctx());
    let recording = s.recording == Some(a);
    let (rect, response) = ui.allocate_exact_size(vec2(120.0, metric::CONTROL), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, s.label(a)));
    let (hover, down) = (response.hovered(), response.is_pointer_button_down_on());
    let fill = if recording {
        c.accent.gamma_multiply(if down {
            0.75
        } else if hover {
            0.92
        } else {
            1.0
        })
    } else if down {
        c.pressed
    } else if hover {
        c.hover
    } else {
        c.field
    };
    ui.painter().rect_filled(rect, radius::CONTROL, fill);
    let text = if recording { "Press a key\u{2026}".to_string() } else { s.label(a) };
    let fg = if recording { Color32::WHITE } else { c.text };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, widgets::strong(ui.ctx(), font::CAPTION), fg);
    if response.clicked() {
        s.conflict = None;
        s.recording = if recording { None } else { Some(a) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip() {
        let p = Prefs {
            new_width: 64,
            checker_light: Some(0x112233FF),
            appearance: 2,
            canvas_bg_on: true,
            default_zoom: 8,
            ..Default::default()
        };
        assert_eq!(Prefs::decode(&p.encode()), p);
        assert_eq!(Prefs::decode("garbage\nnewWidth=99999\n").new_width, 4096);
    }

    #[test]
    fn appearance_follows_the_choice() {
        let mut p = Prefs::default();
        assert!(p.wants_dark(true) && !p.wants_dark(false));
        p.appearance = 1;
        assert!(!p.wants_dark(true));
        p.appearance = 2;
        assert!(p.wants_dark(false));
    }

    #[test]
    fn default_zoom_shrinks_to_fit() {
        let p = Prefs::default();
        assert_eq!(p.default_zoom_for(40.0), 16.0);
        assert_eq!(p.default_zoom_for(5.0), 5.0);
    }
}
