//! Small controls only the tilemap, terrain and slice panels use: `DSBarButtonStyle` buttons, titled `DSChip`s
//! and the terrain swatch.

use crate::theme::{self, Colors, font, metric, radius, space};
use crate::ui::widgets::{rgba_color, strong};
use eframe::egui::{self, Color32, Painter, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

/// `DSBarButtonStyle`: a compact text button (captionMedium, 24 pt high, field fill). `fill` stretches it over
/// the available width; `prominent` uses the accent. Disabled buttons are drawn at 40%.
pub fn bar_button(ui: &mut Ui, label: &str, icon: Option<&str>, fill: bool, prominent: bool) -> Response {
    let c = Colors::of(ui.ctx());
    let enabled = ui.is_enabled();
    let fg0 = if prominent { Color32::WHITE } else { c.text };
    let text = ui.painter().layout_no_wrap(label.to_owned(), strong(ui.ctx(), font::CAPTION), fg0);
    let icon_w = if icon.is_some() { 10.0 + 4.0 } else { 0.0 };
    let natural = text.size().x + icon_w + 2.0 * (space::SM + 2.0);
    let width = if fill { ui.available_width() } else { natural };
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, metric::CONTROL), if enabled { Sense::click() } else { Sense::hover() });
    let (hover, down) = (response.hovered() && enabled, response.is_pointer_button_down_on() && enabled);
    let fill_color = if prominent {
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
    let alpha = if enabled { 1.0 } else { 0.4 };
    ui.painter().rect_filled(rect, radius::CONTROL, fill_color.gamma_multiply(alpha));
    let fg = fg0.gamma_multiply(alpha);
    let total = text.size().x + icon_w;
    let left = rect.center().x - total / 2.0;
    if let Some(glyph) = icon {
        ui.painter().text(
            pos2(left + 5.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(11.0),
            fg,
        );
    }
    ui.painter().galley(pos2(left + icon_w, rect.center().y - text.size().y / 2.0), text, fg);
    response
}

/// `DSChip` with a title and an optional leading symbol: field fill when off, accent-tinted when on.
pub fn title_chip(ui: &mut Ui, title: &str, icon: Option<&str>, on: bool, help: &str) -> Response {
    let c = Colors::of(ui.ctx());
    let enabled = ui.is_enabled();
    let g = ui.painter().layout_no_wrap(title.to_owned(), strong(ui.ctx(), font::CAPTION), c.text);
    let icon_w = if icon.is_some() { 14.0 + 5.0 } else { 0.0 };
    let size = vec2(g.size().x + icon_w + 2.0 * (space::SM + 2.0), metric::CONTROL + 2.0);
    let (rect, response) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let (hover, down) = (response.hovered() && enabled, response.is_pointer_button_down_on() && enabled);
    let fill = if on {
        if hover || down { c.selected_hover } else { c.selected }
    } else if down {
        c.pressed
    } else if hover {
        c.hover
    } else {
        c.field
    };
    let alpha = if enabled { 1.0 } else { 0.35 };
    ui.painter().rect_filled(rect, radius::CONTROL, fill.gamma_multiply(alpha));
    let fg = if on {
        c.accent
    } else if hover {
        c.text
    } else {
        c.text_secondary
    }
    .gamma_multiply(alpha);
    let g = ui.painter().layout_no_wrap(title.to_owned(), strong(ui.ctx(), font::CAPTION), fg);
    let left = rect.center().x - (g.size().x + icon_w) / 2.0;
    if let Some(glyph) = icon {
        ui.painter().text(
            pos2(left + 7.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(14.0),
            fg,
        );
    }
    ui.painter().galley(pos2(left + icon_w, rect.center().y - g.size().y / 2.0), g, fg);
    if help.is_empty() { response } else { response.on_hover_text(help) }
}

/// A terrain's color as a small rounded swatch (`TerrainSwatch`).
pub fn terrain_swatch(painter: &Painter, rect: Rect, rgba: u32) {
    painter.rect_filled(rect, 3.0, rgba_color(rgba | 0xFF));
    painter.rect_stroke(rect, 3.0, Stroke::new(1.0, Color32::from_white_alpha(31)), StrokeKind::Inside);
}

/// A caption-sized label at the cursor.
pub fn caption(ui: &mut Ui, text: &str, color: Color32) {
    ui.label(egui::RichText::new(text).font(theme::ui_font(font::CAPTION)).color(color));
}

/// A caption-medium label.
pub fn caption_medium(ui: &mut Ui, text: &str, color: Color32) {
    ui.label(egui::RichText::new(text).font(strong(ui.ctx(), font::CAPTION)).color(color));
}

/// The system green of the macOS app (`NSColor.systemGreen`).
pub fn system_green(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(0x30, 0xD1, 0x58) } else { Color32::from_rgb(0x34, 0xC7, 0x59) }
}

/// The system orange (`NSColor.systemOrange`).
pub fn system_orange(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(0xFF, 0x9F, 0x0A) } else { Color32::from_rgb(0xFF, 0x95, 0x00) }
}
