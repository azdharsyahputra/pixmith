//! Controls the menu dialogs share: the top-anchored sheet, `DSSlider`, `DSBarButtonStyle`, a color row.

use crate::theme::{Colors, font, metric, radius, space, ui_font};
use crate::ui::widgets::{self, strong};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, Margin, PopupCloseBehavior, Rect, Response, Sense, Stroke, Ui, pos2, vec2,
};
use std::ops::RangeInclusive;

/// A modal sheet that drops from the top of the window like a SwiftUI `.sheet` (`DSDialog`), so the canvas
/// stays visible below it for live previews. `dim` darkens what is behind it.
pub fn sheet<R>(
    ctx: &egui::Context,
    id: &str,
    width: f32,
    title: &str,
    subtitle: Option<&str>,
    dim: bool,
    body: impl FnOnce(&mut Ui) -> R,
) -> R {
    let c = Colors::of(ctx);
    let frame = egui::Frame::new()
        .fill(c.panel)
        .stroke(Stroke::new(1.0, c.separator))
        .corner_radius(CornerRadius::same(radius::PANEL + 4))
        .inner_margin(Margin::same(20))
        .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(90) });
    let key = egui::Id::new(("menu_sheet", id));
    let area = egui::Modal::default_area(key).anchor(Align2::CENTER_TOP, vec2(0.0, 27.0));
    egui::Modal::new(key)
        .area(area)
        .frame(frame)
        .backdrop_color(Color32::from_black_alpha(if dim { 80 } else { 0 }))
        .show(ctx, |ui| {
            ui.set_width(width - 40.0);
            ui.spacing_mut().item_spacing = vec2(space::SM, space::MD);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = space::XS;
                ui.label(egui::RichText::new(title).font(strong(ui.ctx(), 15.0)));
                if let Some(s) = subtitle {
                    let c = Colors::of(ui.ctx());
                    ui.label(egui::RichText::new(s).font(ui_font(font::BODY)).color(c.text_secondary));
                }
            });
            ui.add_space(space::XS);
            body(ui)
        })
        .inner
}

/// A one-pixel rule across the available width (`DSDivider`).
pub fn divider(ui: &mut Ui) {
    let c = Colors::of(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, c.separator);
}

/// A slider drawn as a field with a fill from the zero point (`DSSlider`): the label at the left, the value at
/// the right; click or drag anywhere to set it. Returns true when `value` changed.
pub fn value_slider(ui: &mut Ui, id: &str, label: &str, value: &mut i32, range: RangeInclusive<i32>) -> bool {
    let signed = *range.start() < 0;
    value_slider_fmt(ui, id, label, value, range, |v| if v > 0 && signed { format!("+{v}") } else { v.to_string() })
}

/// [`value_slider`] with its own text for the value.
pub fn value_slider_fmt(
    ui: &mut Ui,
    id: &str,
    label: &str,
    value: &mut i32,
    range: RangeInclusive<i32>,
    format: impl Fn(i32) -> String,
) -> bool {
    let c = Colors::of(ui.ctx());
    let (lo, hi) = (*range.start() as f32, *range.end() as f32);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), metric::CONTROL), Sense::click_and_drag());
    let _ = id;
    let mut changed = false;
    if (response.dragged() || response.is_pointer_button_down_on())
        && let Some(p) = response.interact_pointer_pos()
    {
        let f = ((p.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
        let v = (lo + f * (hi - lo)).round() as i32;
        if v != *value {
            *value = v;
            changed = true;
        }
    }
    let painter = ui.painter();
    painter.rect_filled(rect, radius::FIELD, c.field);
    let frac = ((*value as f32).clamp(lo, hi) - lo) / (hi - lo);
    let zero = if lo < 0.0 && hi > 0.0 { -lo / (hi - lo) } else { 0.0 };
    let alpha = if response.dragged() { 0.34 } else { 0.22 };
    let (a, b) = (frac.min(zero), frac.max(zero));
    let fill = Rect::from_min_max(
        pos2(rect.left() + rect.width() * a, rect.top()),
        pos2(rect.left() + rect.width() * b, rect.bottom()),
    );
    painter.with_clip_rect(rect).rect_filled(fill, CornerRadius::ZERO, c.accent.gamma_multiply(alpha));
    painter.text(
        pos2(rect.left() + space::SM, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        strong(ui.ctx(), font::CAPTION),
        c.text_tertiary,
    );
    painter.text(
        pos2(rect.right() - space::SM, rect.center().y),
        Align2::RIGHT_CENTER,
        format(*value),
        ui_font(font::BODY),
        c.text,
    );
    changed
}

/// A text button in the bar style (`DSBarButtonStyle`): field fill, 11 pt medium label, 24 pt high.
pub fn bar_button(ui: &mut Ui, text: &str, enabled: bool, prominent: bool) -> Response {
    let c = Colors::of(ui.ctx());
    let font = strong(ui.ctx(), font::CAPTION);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, c.text);
    let size = vec2(galley.size().x + 2.0 * (space::SM + 2.0), metric::CONTROL);
    let (rect, response) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, text));
    let (hover, down) = (response.hovered() && enabled, response.is_pointer_button_down_on() && enabled);
    let fill = if prominent {
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
    let fg = if prominent { Color32::WHITE } else { c.text };
    let (fill, fg) = if enabled { (fill, fg) } else { (fill.gamma_multiply(0.4), fg.gamma_multiply(0.4)) };
    ui.painter().rect_filled(rect, radius::CONTROL, fill);
    let g = ui.painter().layout_no_wrap(text.to_owned(), strong(ui.ctx(), font::CAPTION), fg);
    ui.painter().galley(rect.center() - g.size() / 2.0, g, fg);
    response
}

/// A row with a color well: a swatch that opens a color picker (`ColorRow`). `value` is 0xRRGGBBAA.
pub fn color_row(ui: &mut Ui, label: &str, value: &mut u32) -> bool {
    widgets::form_row(ui, label, |ui| color_well(ui, label, value))
}

/// The color well alone.
pub fn color_well(ui: &mut Ui, id: &str, value: &mut u32) -> bool {
    let response = widgets::swatch(ui, *value, 24.0, false, radius::FIELD as f32);
    let mut changed = false;
    egui::Popup::from_toggle_button_response(&response)
        .id(ui.id().with(("color_well", id)))
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let mut color = widgets::rgba_color(*value);
            if egui::color_picker::color_picker_color32(ui, &mut color, egui::color_picker::Alpha::OnlyBlend) {
                let [r, g, b, a] = color.to_srgba_unmultiplied();
                *value = (r as u32) << 24 | (g as u32) << 16 | (b as u32) << 8 | a as u32;
                changed = true;
            }
        });
    changed
}
