//! Small controls shared by the Godot sheets and the Tween panel that the common widgets do not cover:
//! the macOS checkbox, the compact bar button (`DSBarButtonStyle`), a text field with a placeholder
//! (`TextField(prompt, text:).dsTextField()`), form rows with a custom label width (`DSFormRow`), form groups
//! (`DSFormGroup`) and notices (`DSNotice`).

use crate::theme::{Colors, font, metric, radius, space, ui_font};
use crate::ui::widgets::strong;
use eframe::egui::{self, Align, Layout, Rect, Response, Sense, Stroke, StrokeKind, TextEdit, Ui, pos2, vec2};
use egui_phosphor::regular as icon;

/// Paints `galley` heavier than regular by drawing it twice, `weight` points apart (the system font has one
/// weight, which stands in for the medium and semibold weights of the Swift app).
pub fn paint_heavy(
    painter: &egui::Painter,
    pos: egui::Pos2,
    galley: std::sync::Arc<egui::Galley>,
    color: egui::Color32,
    weight: f32,
) {
    painter.galley(pos, galley.clone(), color);
    painter.galley(pos + vec2(weight, 0.0), galley, color);
}

/// Text in a medium or semibold weight, laid out as one label of its natural size.
pub fn heavy_label(ui: &mut Ui, text: &str, size: f32, color: egui::Color32, weight: f32) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), ui_font(size), color);
    let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(weight, 0.0), Sense::hover());
    paint_heavy(ui.painter(), rect.min, galley, color, weight);
}

/// The tone of a notice row.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Warning,
    Error,
}

/// The orange of `Color.orange`.
pub const ORANGE: egui::Color32 = egui::Color32::from_rgb(0xFF, 0x9F, 0x0A);

/// A checkbox with the label to its right (`Toggle(...).toggleStyle(.checkbox)` at 12 pt). Returns whether the
/// value changed. `id` must be unique within the ui.
pub fn check(ui: &mut Ui, id: &str, on: &mut bool, title: &str) -> bool {
    let c = Colors::of(ui.ctx());
    let enabled = ui.is_enabled();
    let galley = ui.painter().layout_no_wrap(title.to_owned(), ui_font(font::BODY), c.text);
    let size = vec2(14.0 + 6.0 + galley.size().x, 16.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let response =
        ui.interact(rect, ui.id().with(("check", id)), if enabled { Sense::click() } else { Sense::hover() });
    let mut changed = false;
    if enabled && response.clicked() {
        *on = !*on;
        changed = true;
    }
    let dim = |col: egui::Color32| if enabled { col } else { col.gamma_multiply(0.4) };
    let box_rect = Rect::from_min_size(pos2(rect.left(), rect.center().y - 7.0), vec2(14.0, 14.0));
    let hot = response.is_pointer_button_down_on();
    if *on {
        ui.painter().rect_filled(box_rect, 3.5, dim(if hot { c.accent.gamma_multiply(0.8) } else { c.accent }));
        let p = [
            pos2(box_rect.left() + 3.4, box_rect.center().y + 0.2),
            pos2(box_rect.left() + 5.9, box_rect.center().y + 2.9),
            pos2(box_rect.left() + 10.6, box_rect.center().y - 2.9),
        ];
        let stroke = Stroke::new(1.7, dim(egui::Color32::WHITE));
        ui.painter().line_segment([p[0], p[1]], stroke);
        ui.painter().line_segment([p[1], p[2]], stroke);
    } else {
        ui.painter().rect_filled(box_rect, 3.5, dim(if hot { c.pressed } else { c.field }));
        ui.painter().rect_stroke(box_rect, 3.5, Stroke::new(1.0, dim(c.text_tertiary)), StrokeKind::Inside);
    }
    ui.painter().galley(pos2(rect.left() + 20.0, rect.center().y - galley.size().y / 2.0), galley, dim(c.text));
    changed
}

/// The width of a [`bar_button`] titled `title`.
pub fn bar_button_width(ui: &Ui, ctx_font_title: &str) -> f32 {
    let g =
        ui.painter().layout_no_wrap(ctx_font_title.to_owned(), strong(ui.ctx(), font::CAPTION), egui::Color32::WHITE);
    g.size().x + 2.0 * (space::SM + 2.0)
}

/// A compact action button (`DSBarButtonStyle`): 24 pt high, caption text.
pub fn bar_button(ui: &mut Ui, title: &str) -> Response {
    let c = Colors::of(ui.ctx());
    let enabled = ui.is_enabled();
    let w = bar_button_width(ui, title);
    let (rect, response) =
        ui.allocate_exact_size(vec2(w, metric::CONTROL), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if response.is_pointer_button_down_on() && enabled {
        c.pressed
    } else if response.hovered() && enabled {
        c.hover
    } else {
        c.field
    };
    let dim = |col: egui::Color32| if enabled { col } else { col.gamma_multiply(0.4) };
    ui.painter().rect_filled(rect, radius::CONTROL, dim(fill));
    let galley = ui.painter().layout_no_wrap(title.to_owned(), ui_font(font::CAPTION), dim(c.text));
    paint_heavy(ui.painter(), rect.center() - galley.size() / 2.0, galley, dim(c.text), 0.3);
    response
}

/// A single-line field with a placeholder shown while it is empty. `width` `None` fills the row.
pub fn text_field(ui: &mut Ui, id: &str, text: &mut String, hint: &str, width: Option<f32>) -> Response {
    let c = Colors::of(ui.ctx());
    let id = ui.id().with(("text_field", id));
    let w = width.unwrap_or_else(|| ui.available_width());
    let (rect, _) = ui.allocate_exact_size(vec2(w, metric::CONTROL), Sense::hover());
    let focused = ui.memory(|m| m.has_focus(id));
    ui.painter().rect_filled(rect, radius::FIELD, c.field);
    if focused {
        ui.painter().rect_stroke(rect, radius::FIELD, Stroke::new(1.5, c.accent_line), StrokeKind::Middle);
    }
    let edit = TextEdit::singleline(text)
        .id(id)
        .font(ui_font(font::BODY))
        .hint_text(egui::RichText::new(hint).color(c.text_tertiary))
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::ZERO)
        .vertical_align(Align::Center)
        .desired_width(rect.width() - 2.0 * space::SM);
    crate::ui::widgets::put_in(ui, rect.shrink2(vec2(space::SM, 0.0)), edit)
}

/// A label-left, control-right row (`DSFormRow`) with a label column of `label_width`.
pub fn form_row<R>(ui: &mut Ui, label_width: f32, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let c = Colors::of(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        let (rect, _) = ui.allocate_exact_size(vec2(label_width, metric::CONTROL), Sense::hover());
        ui.painter().text(
            pos2(rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            ui_font(font::BODY),
            c.text_secondary,
        );
        add(ui)
    })
    .inner
}

/// A row of content indented under the label column: `Color.clear` of `label_width`, then the content.
pub fn indented<R>(ui: &mut Ui, label_width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        ui.add_space(label_width);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = space::SM - 2.0;
            add(ui)
        })
        .inner
    })
    .inner
}

/// A titled block of an export form: a quiet heading, then rows, with a hairline above all but the first.
pub fn form_group(ui: &mut Ui, title: &str, first: bool, add: impl FnOnce(&mut Ui)) {
    let c = Colors::of(ui.ctx());
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = space::SM;
        if !first {
            let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().rect_filled(line, 0.0, c.separator_soft);
            ui.add_space(space::XS - space::SM);
        }
        heavy_label(ui, title, font::CAPTION, c.text, 0.5);
        add(ui);
    });
}

/// A thin hairline across the available width (`Rectangle().fill(DSColor.separatorSoft).frame(height: 1)`).
pub fn hairline(ui: &mut Ui) {
    let c = Colors::of(ui.ctx());
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(line, 0.0, c.separator_soft);
}

/// A small notice row: a symbol and wrapped text in a quiet or warning tone (`DSNotice`).
pub fn notice(ui: &mut Ui, text: &str, tone: Tone) {
    let c = Colors::of(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = space::XS + 2.0;
        if tone != Tone::Info {
            let (glyph, color) =
                if tone == Tone::Error { (icon::WARNING_OCTAGON, c.destructive) } else { (icon::WARNING, ORANGE) };
            ui.add_space(0.0);
            ui.label(egui::RichText::new(glyph).size(11.0).color(color));
        }
        let color = if tone == Tone::Info { c.text_secondary } else { c.text };
        ui.add(egui::Label::new(egui::RichText::new(text).font(ui_font(font::CAPTION)).color(color)).wrap());
    });
}

/// Lays `add` out right to left, so the first widget added is the rightmost.
pub fn right_to_left<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.with_layout(Layout::right_to_left(Align::Center), add).inner
}
