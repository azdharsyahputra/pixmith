//! Draws the rows of a Lua dialog with the design-system controls and reports what the user did as
//! [`Event`]s. Nothing in here talks to the core: the caller sends the events.
//!
//! The look is `DSDialog`: form rows with an aligned label column (`DSFormRow`), fields 24 pt high (`dsTextField`,
//! `DSNumberField`, `DSPopup`, `DSSlider`), 28 pt buttons (`DSPrimaryButtonStyle`, `DSSecondaryButtonStyle`).

use super::model::{Dialog, Event, Kind, Widget, strip_mnemonic};
use crate::theme::{Colors, font, metric, radius, space, ui_font};
use crate::ui::widgets::{self, NumberOpts};
use eframe::egui::{
    self, Align, Align2, Color32, Context, FontId, Id, Key, Layout, Rect, RichText, Sense, Stroke, StrokeKind, Ui,
    WidgetInfo, WidgetType, vec2,
};
use pixmith_core::formats::json::Value;

/// Narrowest and widest a dialog gets before the content wraps or scrolls.
const MIN_WIDTH: f32 = 320.0;
const MAX_WIDTH: f32 = 560.0;
/// The most room a label column takes.
const MAX_LABEL_COLUMN: f32 = 150.0;
const BOX: f32 = 16.0;

/// What the renderer remembers about one dialog between frames (the dialog data itself is the model).
#[derive(Default)]
pub struct View {
    /// The initial keyboard focus was given.
    pub focus_done: bool,
    /// Widgets a `dialog.modify` gave the focus.
    pub pending_focus: Vec<String>,
}

fn text_width(ctx: &Context, text: &str, font: FontId) -> f32 {
    ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font, Color32::WHITE).size().x)
}

fn display_text(w: &Widget) -> String {
    strip_mnemonic(&w.text).0
}

/// The accessible name of a widget: what the user reads next to it.
fn name_of(w: &Widget) -> String {
    let text = display_text(w);
    match (&w.label, text.is_empty()) {
        (Some(l), _) if !l.is_empty() => l.clone(),
        (_, false) => text,
        _ => w.id.clone(),
    }
}

fn label_cell_width(ctx: &Context, w: &Widget) -> f32 {
    w.label.as_deref().map_or(0.0, |l| text_width(ctx, l, ui_font(font::BODY)).min(MAX_LABEL_COLUMN))
}

/// The width of the label column: the widest first label of any row.
fn label_column(ctx: &Context, d: &Dialog) -> f32 {
    d.rows
        .iter()
        .filter_map(|r| r.widgets.iter().find(|w| w.visible))
        .filter(|w| w.label.is_some())
        .map(|w| label_cell_width(ctx, w))
        .fold(0.0, f32::max)
}

/// The least width a widget needs.
fn min_body_width(ctx: &Context, w: &Widget) -> f32 {
    let body = ui_font(font::BODY);
    match w.kind {
        Kind::Label => text_width(ctx, &display_text(w), body).min(MAX_WIDTH - 2.0 * space::LG),
        Kind::Button => {
            let font = if w.focus { widgets::strong(ctx, font::BODY_LARGE) } else { ui_font(font::BODY_LARGE) };
            (text_width(ctx, &display_text(w), font) + 2.0 * space::MD).max(64.0)
        }
        Kind::Check | Kind::Radio => BOX + 6.0 + text_width(ctx, &display_text(w), body),
        Kind::Entry => 160.0,
        Kind::Number => 90.0,
        Kind::Slider => 160.0,
        Kind::Color => 132.0,
        Kind::Combobox => {
            let widest = w.options.iter().map(|o| text_width(ctx, o, body.clone())).fold(0.0, f32::max);
            (widest + 2.0 * space::SM + 18.0).max(120.0)
        }
        Kind::File => 220.0,
        Kind::Shades => (w.colors.len() as f32 * 22.0).clamp(22.0, 300.0),
        Kind::Separator => 0.0,
    }
}

/// The width the dialog wants for its content (without the frame).
pub fn natural_width(ctx: &Context, d: &Dialog) -> f32 {
    let column = label_column(ctx, d);
    let widest = d
        .rows
        .iter()
        .map(|r| {
            let vis: Vec<&Widget> = r.widgets.iter().filter(|w| w.visible).collect();
            let mut total = 0.0;
            for (i, w) in vis.iter().enumerate() {
                if w.label.is_some() {
                    total += if i == 0 { column } else { label_cell_width(ctx, w) } + space::SM;
                }
                total += min_body_width(ctx, w);
            }
            total + space::SM * vis.len().saturating_sub(1) as f32
        })
        .fold(0.0, f32::max);
    let title = text_width(ctx, &d.title, widgets::strong(ctx, 15.0));
    widest.max(title).clamp(MIN_WIDTH, MAX_WIDTH)
}

/// Draws all rows. `view` carries the focus requests; events are appended to `out`.
pub fn body(ui: &mut Ui, d: &mut Dialog, view: &mut View, out: &mut Vec<Event>) {
    let column = label_column(ui.ctx(), d);
    ui.spacing_mut().item_spacing = vec2(space::SM, space::SM);
    let did = d.id.clone();
    let mut radio: Option<String> = None;
    for row in &mut d.rows {
        let vis: Vec<usize> = (0..row.widgets.len()).filter(|&i| row.widgets[i].visible).collect();
        if vis.is_empty() {
            continue;
        }
        if vis.len() == 1 && row.widgets[vis[0]].kind == Kind::Separator {
            separator(ui, &row.widgets[vis[0]].text);
            continue;
        }
        // Widths: fixed parts first, then the free width shared by the widgets that expand.
        let total = ui.available_width();
        let mut fixed = space::SM * (vis.len() - 1) as f32;
        let mut expanders = 0;
        let mut cells = Vec::new();
        for (n, &i) in vis.iter().enumerate() {
            let w = &row.widgets[i];
            let label = if w.label.is_some() {
                (if n == 0 { column } else { label_cell_width(ui.ctx(), w) }) + 0.0
            } else {
                0.0
            };
            let body = min_body_width(ui.ctx(), w);
            fixed += label + body + if w.label.is_some() { space::SM } else { 0.0 };
            if w.hexpand && w.kind != Kind::Separator && w.kind != Kind::Label {
                expanders += 1;
            }
            cells.push((label, body));
        }
        let extra = if expanders > 0 { ((total - fixed) / expanders as f32).max(0.0) } else { 0.0 };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            for (n, &i) in vis.iter().enumerate() {
                let w = &mut row.widgets[i];
                let (label_w, body_w) = cells[n];
                if let Some(label) = &w.label {
                    form_label(ui, label, label_w);
                }
                let width = if w.hexpand && w.kind != Kind::Separator && w.kind != Kind::Label {
                    body_w + extra
                } else {
                    body_w
                };
                let enabled = w.enabled;
                let key = (did.clone(), w.id.clone());
                ui.push_id(key, |ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        if let Some(id) = widget(ui, &did, w, width, view, out) {
                            radio = Some(id);
                        }
                    });
                });
            }
        });
    }
    if let Some(id) = radio {
        d.select_radio(&id);
    }
    view.focus_done = true;
    view.pending_focus.clear();
}

fn form_label(ui: &mut Ui, label: &str, width: f32) {
    let c = Colors::of(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, metric::CONTROL), Sense::hover());
    ui.painter().with_clip_rect(rect).text(
        egui::pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        label,
        ui_font(font::BODY),
        c.text_secondary,
    );
}

fn separator(ui: &mut Ui, text: &str) {
    let c = Colors::of(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    let y = rect.center().y;
    let mut left = rect.left();
    if !text.is_empty() {
        let galley =
            ui.painter().layout_no_wrap(text.to_owned(), widgets::strong(ui.ctx(), font::CAPTION), c.text_tertiary);
        let w = galley.size().x;
        ui.painter().galley(egui::pos2(rect.left(), y - galley.size().y / 2.0), galley, c.text_tertiary);
        left += w + space::SM;
    }
    ui.painter().hline(left..=rect.right(), y, Stroke::new(1.0, c.separator_soft));
}

/// Draws one widget `width` points wide. Returns the id of a radio that was just selected.
fn widget(ui: &mut Ui, did: &str, w: &mut Widget, width: f32, view: &mut View, out: &mut Vec<Event>) -> Option<String> {
    let want_focus = w.focus && !view.focus_done || view.pending_focus.contains(&w.id);
    let key = (did.to_owned(), w.id.clone());
    match w.kind {
        Kind::Label => label(ui, w, width),
        Kind::Button => button(ui, w, out),
        Kind::Check => check(ui, w, width, out),
        Kind::Radio => return radio(ui, w, width, out),
        Kind::Entry => {
            let r = widgets::ds_text_field(ui, "entry", &mut w.text, Some(width));
            if want_focus {
                r.request_focus();
            }
            if r.changed() {
                out.push(Event::Change { id: w.id.clone(), value: Value::Str(w.text.clone()) });
            }
        }
        Kind::Number => number(ui, w, width, want_focus, out),
        Kind::Slider => slider(ui, w, width, out),
        Kind::Color => color(ui, w, width, out),
        Kind::Combobox => combobox(ui, w, width, out),
        Kind::File => file(ui, w, width, out),
        Kind::Shades => shades(ui, w, width, out),
        Kind::Separator => separator(ui, &w.text),
    }
    let _ = key;
    None
}

fn label(ui: &mut Ui, w: &Widget, width: f32) {
    let c = Colors::of(ui.ctx());
    ui.allocate_ui(vec2(width.max(24.0), 0.0), |ui| {
        ui.set_max_width(width.max(24.0));
        ui.add(egui::Label::new(RichText::new(display_text(w)).font(ui_font(font::BODY)).color(c.text)).wrap());
    });
}

fn button(ui: &mut Ui, w: &Widget, out: &mut Vec<Event>) {
    let text = display_text(w);
    let enabled = w.enabled;
    let response =
        if w.focus { widgets::primary_button(ui, &text, enabled) } else { widgets::secondary_button(ui, &text, false) };
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, text.clone()));
    if response.clicked() && enabled {
        out.push(Event::Click { id: w.id.clone(), value: None, color: None });
    }
}

fn check(ui: &mut Ui, w: &mut Widget, width: f32, out: &mut Vec<Event>) {
    let c = Colors::of(ui.ctx());
    let text = display_text(w);
    let (rect, response) = ui.allocate_exact_size(vec2(width.max(BOX), metric::CONTROL), Sense::click());
    let boxr = Rect::from_min_size(egui::pos2(rect.left(), rect.center().y - BOX / 2.0), vec2(BOX, BOX));
    let painter = ui.painter();
    if w.selected {
        painter.rect_filled(boxr, radius::CHIP, c.accent);
        painter.text(
            boxr.center(),
            Align2::CENTER_CENTER,
            egui_phosphor::regular::CHECK,
            FontId::proportional(11.0),
            Color32::WHITE,
        );
    } else {
        let fill = if response.hovered() { c.hover } else { c.field };
        painter.rect_filled(boxr, radius::CHIP, fill);
        painter.rect_stroke(boxr, radius::CHIP, Stroke::new(1.0, c.separator), StrokeKind::Inside);
    }
    painter.text(
        egui::pos2(boxr.right() + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        &text,
        ui_font(font::BODY),
        c.text,
    );
    let (enabled, selected) = (w.enabled, w.selected);
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, selected, text.clone()));
    if response.clicked() && enabled {
        w.selected = !w.selected;
        out.push(Event::Click { id: w.id.clone(), value: Some(w.selected), color: None });
    }
}

fn radio(ui: &mut Ui, w: &mut Widget, width: f32, out: &mut Vec<Event>) -> Option<String> {
    let c = Colors::of(ui.ctx());
    let text = display_text(w);
    let (rect, response) = ui.allocate_exact_size(vec2(width.max(BOX), metric::CONTROL), Sense::click());
    let center = egui::pos2(rect.left() + BOX / 2.0, rect.center().y);
    let painter = ui.painter();
    if w.selected {
        painter.circle_filled(center, BOX / 2.0, c.accent);
        painter.circle_filled(center, 3.0, Color32::WHITE);
    } else {
        let fill = if response.hovered() { c.hover } else { c.field };
        painter.circle_filled(center, BOX / 2.0, fill);
        painter.circle_stroke(center, BOX / 2.0 - 0.5, Stroke::new(1.0, c.separator));
    }
    painter.text(
        egui::pos2(rect.left() + BOX + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        &text,
        ui_font(font::BODY),
        c.text,
    );
    let (enabled, selected) = (w.enabled, w.selected);
    response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, enabled, selected, text.clone()));
    if response.clicked() && enabled && !w.selected {
        out.push(Event::Click { id: w.id.clone(), value: Some(true), color: None });
        return Some(w.id.clone());
    }
    None
}

fn number(ui: &mut Ui, w: &mut Widget, width: f32, want_focus: bool, out: &mut Vec<Event>) {
    let step = 10f64.powi(-(w.decimals as i32));
    let opts = NumberOpts {
        range: (-1.0e9, 1.0e9),
        step,
        digits: w.decimals,
        width: Some(width),
        help: "",
        ..NumberOpts::default()
    };
    if want_focus {
        let id = ui.id().with("number").with("text");
        ui.memory_mut(|m| m.request_focus(id));
    }
    let result = widgets::ds_number_field(ui, "number", "", &mut w.value, &opts);
    if result.changed {
        out.push(Event::Change { id: w.id.clone(), value: Value::Num(w.value) });
    }
}

fn slider(ui: &mut Ui, w: &mut Widget, width: f32, out: &mut Vec<Event>) {
    let c = Colors::of(ui.ctx());
    let enabled = w.enabled;
    let (rect, response) = ui.allocate_exact_size(vec2(width, metric::CONTROL), Sense::click_and_drag());
    let (min, max) = (w.min.round(), w.max.round().max(w.min.round()));
    let span = (max - min).max(1.0);
    let mut value = w.value.clamp(min, max).round();
    if enabled {
        if (response.dragged() || response.drag_started() || response.clicked())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let f = f64::from(((pos.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0));
            value = (min + f * span).round();
        }
        if response.has_focus() {
            let step = ui.input(|i| {
                let mut s = 0.0;
                if i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::ArrowDown) {
                    s -= 1.0;
                }
                if i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::ArrowUp) {
                    s += 1.0;
                }
                if i.key_pressed(Key::PageDown) {
                    s -= (span / 10.0).round().max(1.0);
                }
                if i.key_pressed(Key::PageUp) {
                    s += (span / 10.0).round().max(1.0);
                }
                s
            });
            if step != 0.0 {
                value = (value + step).clamp(min, max);
                out.push(Event::Change { id: w.id.clone(), value: Value::Num(value) });
                out.push(Event::Release { id: w.id.clone() });
                w.value = value;
            }
        }
    }
    if value != w.value {
        w.value = value;
        out.push(Event::Change { id: w.id.clone(), value: Value::Num(value) });
    }
    if enabled && (response.drag_stopped() || response.clicked()) {
        out.push(Event::Release { id: w.id.clone() });
    }
    let frac = ((w.value.clamp(min, max) - min) / span) as f32;
    let painter = ui.painter();
    painter.rect_filled(rect, radius::FIELD, c.field);
    let alpha = if response.dragged() { 0.34 } else { 0.22 };
    painter.rect_filled(
        Rect::from_min_size(rect.min, vec2(rect.width() * frac, rect.height())),
        radius::FIELD,
        c.accent.gamma_multiply(alpha),
    );
    if response.has_focus() {
        painter.rect_stroke(rect, radius::FIELD, Stroke::new(1.5, c.accent_line), StrokeKind::Middle);
    }
    painter.text(
        egui::pos2(rect.right() - space::SM, rect.center().y),
        Align2::RIGHT_CENTER,
        format!("{}", w.value as i64),
        ui_font(font::BODY),
        c.text,
    );
    let (name, current) = (name_of(w), w.value);
    response.widget_info(|| {
        let mut info = WidgetInfo::slider(enabled, current, name.clone());
        info.label = Some(name.clone());
        info
    });
}

/// `#RRGGBB`, with `AA` appended when the color is not opaque.
pub fn hex(color: u32) -> String {
    if color & 0xFF == 0xFF { format!("#{:06X}", color >> 8) } else { format!("#{color:08X}") }
}

/// Reads `RRGGBB` or `RRGGBBAA`, with or without `#`.
pub fn parse_hex(text: &str) -> Option<u32> {
    let t = text.trim().trim_start_matches('#');
    match t.len() {
        6 => u32::from_str_radix(t, 16).ok().map(|v| v << 8 | 0xFF),
        8 => u32::from_str_radix(t, 16).ok(),
        _ => None,
    }
}

fn color(ui: &mut Ui, w: &mut Widget, width: f32, out: &mut Vec<Event>) {
    let c = Colors::of(ui.ctx());
    let enabled = w.enabled;
    let (rect, response) = ui.allocate_exact_size(vec2(width, metric::CONTROL), Sense::click());
    let fill = if response.hovered() || response.is_pointer_button_down_on() { c.hover } else { c.field };
    ui.painter().rect_filled(rect, radius::FIELD, fill);
    let chip = Rect::from_min_size(rect.min + vec2(4.0, 4.0), vec2(metric::CONTROL - 8.0, metric::CONTROL - 8.0));
    widgets::paint_swatch(ui.painter(), chip, w.color, radius::CHIP as f32 - 1.0, 4.0);
    ui.painter().text(
        egui::pos2(chip.right() + space::SM, rect.center().y),
        Align2::LEFT_CENTER,
        hex(w.color),
        ui_font(font::BODY),
        c.text,
    );
    let name = name_of(w);
    let current = hex(w.color);
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::Button, enabled, name.clone());
        info.value = None;
        info.label = Some(format!("{name} {current}"));
        info
    });

    // The picker keeps its own hue/saturation state so dragging through black or white does not lose it.
    let state_id = ui.id().with("hsva");
    let mut state: (u32, egui::ecolor::Hsva) = ui.data(|d| d.get_temp(state_id)).unwrap_or_else(|| {
        let [r, g, b, a] = w.color.to_be_bytes();
        (w.color, egui::ecolor::Hsva::from_srgba_unmultiplied([r, g, b, a]))
    });
    if state.0 != w.color {
        let [r, g, b, a] = w.color.to_be_bytes();
        state = (w.color, egui::ecolor::Hsva::from_srgba_unmultiplied([r, g, b, a]));
    }
    let mut picked = None;
    egui::Popup::menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(240.0);
        ui.spacing_mut().slider_width = 220.0;
        if egui::color_picker::color_picker_hsva_2d(ui, &mut state.1, egui::color_picker::Alpha::OnlyBlend) {
            let [r, g, b, a] = state.1.to_srgba_unmultiplied();
            picked = Some(u32::from_be_bytes([r, g, b, a]));
        }
        ui.add_space(space::XS);
        let hex_id = ui.id().with("hex");
        let focused = ui.memory(|m| m.has_focus(hex_id));
        let mut text: String =
            if focused { ui.data(|d| d.get_temp(hex_id)).unwrap_or_else(|| hex(w.color)) } else { hex(w.color) };
        let edit = egui::TextEdit::singleline(&mut text).id(hex_id).font(ui_font(font::BODY)).desired_width(120.0);
        let r = ui.add(edit);
        if r.has_focus() {
            ui.data_mut(|d| d.insert_temp(hex_id, text.clone()));
        }
        if r.lost_focus() {
            ui.data_mut(|d| d.remove_temp::<String>(hex_id));
            if let Some(v) = parse_hex(&text) {
                picked = Some(v);
            }
        }
    });
    if let Some(v) = picked
        && v != w.color
    {
        w.color = v;
        let [r, g, b, a] = v.to_be_bytes();
        if state.0 != v {
            // From the hex field: derive the picker state from the typed color.
            state.1 = egui::ecolor::Hsva::from_srgba_unmultiplied([r, g, b, a]);
        }
        state.0 = v;
        out.push(Event::Change { id: w.id.clone(), value: Value::Num(f64::from(v)) });
    }
    ui.data_mut(|d| d.insert_temp(state_id, state));
}

fn combobox(ui: &mut Ui, w: &mut Widget, width: f32, out: &mut Vec<Event>) {
    let c = Colors::of(ui.ctx());
    let enabled = w.enabled;
    let (rect, response) = ui.allocate_exact_size(vec2(width, metric::CONTROL), Sense::click());
    let fill = if response.hovered() || response.is_pointer_button_down_on() { c.hover } else { c.field };
    ui.painter().rect_filled(rect, radius::FIELD, fill);
    let title = w.option.clone().unwrap_or_default();
    ui.painter().with_clip_rect(rect.shrink2(vec2(space::SM, 0.0))).text(
        egui::pos2(rect.left() + space::SM, rect.center().y),
        Align2::LEFT_CENTER,
        &title,
        ui_font(font::BODY),
        c.text,
    );
    let (cx, cy) = (rect.right() - space::SM - 4.0, rect.center().y);
    let stroke = Stroke::new(1.5, c.text_tertiary);
    ui.painter().line_segment([egui::pos2(cx - 3.6, cy - 1.8), egui::pos2(cx, cy + 1.8)], stroke);
    ui.painter().line_segment([egui::pos2(cx, cy + 1.8), egui::pos2(cx + 3.6, cy - 1.8)], stroke);
    let name = name_of(w);
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::ComboBox, enabled, name.clone());
        info.current_text_value = Some(title.clone());
        info
    });
    let selected = w.option_index();
    let mut chosen = None;
    egui::Popup::menu(&response).width(rect.width()).show(|ui| {
        for (i, option) in w.options.iter().enumerate() {
            if ui.selectable_label(Some(i) == selected, option).clicked() {
                chosen = Some(i);
            }
        }
    });
    if let Some(i) = chosen
        && Some(i) != selected
    {
        let option = w.options[i].clone();
        w.option = Some(option.clone());
        out.push(Event::Change { id: w.id.clone(), value: Value::Str(option) });
    }
}

fn file(ui: &mut Ui, w: &mut Widget, width: f32, out: &mut Vec<Event>) {
    let field_w = (width - metric::CONTROL - space::XS).max(60.0);
    let committed_id = ui.id().with("committed");
    let committed: String = ui.data(|d| d.get_temp(committed_id)).unwrap_or_else(|| w.filename.clone());
    let r = widgets::ds_text_field(ui, "path", &mut w.filename, Some(field_w));
    if r.lost_focus() {
        if w.filename != committed {
            out.push(Event::Change { id: w.id.clone(), value: Value::Str(w.filename.clone()) });
        }
        ui.data_mut(|d| d.insert_temp(committed_id, w.filename.clone()));
    } else if !r.has_focus() {
        ui.data_mut(|d| d.insert_temp(committed_id, w.filename.clone()));
    }
    let browse = widgets::ds_icon_button(ui, egui_phosphor::regular::FOLDER_OPEN, metric::CONTROL, 13.0, false, false);
    browse.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Choose file"));
    if browse.on_hover_text("Choose\u{2026}").clicked()
        && let Some(path) = pick_path(w)
    {
        w.filename = path.clone();
        ui.data_mut(|d| d.insert_temp(committed_id, path.clone()));
        out.push(Event::Change { id: w.id.clone(), value: Value::Str(path) });
    }
}

/// The system file picker for a `file` widget.
fn pick_path(w: &Widget) -> Option<String> {
    let mut dialog = rfd::FileDialog::new();
    if !w.title.is_empty() {
        dialog = dialog.set_title(w.title.as_str());
    }
    if !w.filetypes.is_empty() {
        dialog = dialog.add_filter("Files", &w.filetypes);
    }
    let current = std::path::Path::new(&w.filename);
    if let Some(dir) = current.parent().filter(|p| p.is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    if let Some(name) = current.file_name() {
        dialog = dialog.set_file_name(name.to_string_lossy());
    }
    let picked = if w.save && !w.open { dialog.save_file() } else { dialog.pick_file() };
    picked.map(|p| p.to_string_lossy().into_owned())
}

fn shades(ui: &mut Ui, w: &Widget, width: f32, out: &mut Vec<Event>) {
    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::left_to_right(Align::Min).with_main_wrap(true), |ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
        for (i, &color) in w.colors.iter().enumerate() {
            let r = widgets::swatch(ui, color, 20.0, false, f32::from(radius::CHIP) - 1.0);
            let name = format!("{} {}", name_of(w), hex(color));
            r.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, name.clone()));
            if r.on_hover_text(hex(color)).clicked() {
                out.push(Event::Click { id: w.id.clone(), value: None, color: Some(color) });
            }
            let _ = i;
        }
    });
}

/// The key of the egui id a dialog's focus handling and tests can use.
pub fn dialog_id(did: &str) -> Id {
    Id::new(("lua_dialog", did))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        assert_eq!(hex(0x1D2B53FF), "#1D2B53");
        assert_eq!(hex(0x1D2B5380), "#1D2B5380");
        assert_eq!(parse_hex("#1D2B53"), Some(0x1D2B53FF));
        assert_eq!(parse_hex("1d2b5380"), Some(0x1D2B5380));
        assert_eq!(parse_hex("12345"), None);
        assert_eq!(parse_hex("zzzzzz"), None);
    }
}
