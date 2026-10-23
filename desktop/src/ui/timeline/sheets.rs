//! The modal editors opened from the timeline: tag sheet (`TagSheet`) and layer properties
//! (`LayerPropertiesSheet`), drawn as centered egui windows in the dialog style.

use super::tags::{DIRECTIONS, TagInfo};
use crate::app::DocTab;
use crate::theme::{Colors, font, radius, ui_font};
use eframe::egui::{self, Align, Color32, CornerRadius, Layout, RichText, Stroke, vec2};

pub const BLEND_MODES: [&str; 19] = [
    "Normal",
    "Multiply",
    "Screen",
    "Overlay",
    "Darken",
    "Lighten",
    "Color Dodge",
    "Color Burn",
    "Hard Light",
    "Soft Light",
    "Difference",
    "Exclusion",
    "Hue",
    "Saturation",
    "Color",
    "Luminosity",
    "Addition",
    "Subtract",
    "Divide",
];

/// Creates a tag (`index` None) or edits an existing one.
pub struct TagSheet {
    pub index: Option<u32>,
    pub tag: TagInfo,
}

pub struct LayerSheet {
    pub layer: u32,
    pub name: String,
    pub opacity: u8,
    pub blend: u16,
    pub visible: bool,
    pub group: bool,
}

impl LayerSheet {
    pub fn new(tab: &DocTab, layer: u32) -> LayerSheet {
        let d = &tab.doc;
        LayerSheet {
            layer,
            name: d.layer_name(layer),
            opacity: d.layer_opacity(layer),
            blend: d.layer_blend_mode(layer),
            visible: d.layer_visible(layer),
            group: d.layer_is_group(layer),
        }
    }
}

enum Outcome {
    Open,
    Cancel,
    Primary,
    Destructive,
}

fn window(ctx: &egui::Context, c: &Colors, title: &str, width: f32, body: impl FnOnce(&mut egui::Ui)) -> bool {
    let frame = egui::Frame::new()
        .fill(c.elevated)
        .stroke(Stroke::new(1.0, c.separator))
        .corner_radius(CornerRadius::same(radius::PANEL + 4))
        .inner_margin(egui::Margin::same(20))
        .shadow(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(90) });
    let mut open = true;
    egui::Window::new(title)
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .fixed_size(vec2(width, 0.0))
        .frame(frame)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 12.0);
            ui.label(RichText::new(title).font(crate::ui::widgets::strong(ui.ctx(), font::TITLE)).color(c.text));
            body(ui);
        });
    open
}

fn row(ui: &mut egui::Ui, c: &Colors, label: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (rect, _) = ui.allocate_exact_size(vec2(72.0, 24.0), egui::Sense::hover());
        ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, label, ui_font(font::BODY), c.text_secondary);
        content(ui);
    });
}

fn button(ui: &mut egui::Ui, c: &Colors, label: &str, primary: bool, destructive: bool) -> egui::Response {
    let text_w = ui.painter().layout_no_wrap(label.to_owned(), ui_font(font::BODY), c.text).size().x;
    let (rect, resp) = ui.allocate_exact_size(vec2(text_w + 28.0, 28.0), egui::Sense::click());
    let base = if primary { c.accent } else { c.field };
    let fill = if resp.is_pointer_button_down_on() {
        if primary { c.accent.gamma_multiply(0.8) } else { c.pressed }
    } else if resp.hovered() && !primary {
        c.hover
    } else {
        base
    };
    ui.painter().rect_filled(rect, CornerRadius::same(radius::CONTROL), fill);
    let fg = if primary {
        Color32::WHITE
    } else if destructive {
        c.destructive
    } else {
        c.text
    };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, ui_font(font::BODY), fg);
    resp
}

fn footer(ui: &mut egui::Ui, c: &Colors, primary: &str, destructive: Option<&str>) -> Outcome {
    let mut out = Outcome::Open;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if button(ui, c, primary, true, false).clicked() {
            out = Outcome::Primary;
        }
        if button(ui, c, "Cancel", false, false).clicked() {
            out = Outcome::Cancel;
        }
        if let Some(d) = destructive {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                if button(ui, c, d, false, true).clicked() {
                    out = Outcome::Destructive;
                }
            });
        }
    });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        out = Outcome::Cancel;
    }
    out
}

fn text_field(ui: &mut egui::Ui, c: &Colors, text: &mut String, width: f32) {
    egui::Frame::new()
        .fill(c.field)
        .corner_radius(CornerRadius::same(radius::FIELD))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(text)
                    .frame(egui::Frame::NONE)
                    .desired_width(width - 16.0)
                    .font(ui_font(font::BODY))
                    .margin(egui::Margin::ZERO),
            );
        });
}

/// Draws the tag sheet; returns true once it should be closed.
pub fn show_tag(ctx: &egui::Context, sheet: &mut TagSheet, tab: &mut DocTab) -> bool {
    let c = Colors::of(ctx);
    let last = tab.doc.frame_count().max(1);
    let mut outcome = Outcome::Open;
    let editing = sheet.index.is_some();
    let still_open = window(ctx, &c, if editing { "Edit Tag" } else { "New Tag" }, 320.0, |ui| {
        let tag = &mut sheet.tag;
        row(ui, &c, "Name", |ui| text_field(ui, &c, &mut tag.name, 208.0));
        row(ui, &c, "Color", |ui| {
            let mut rgb = [(tag.color >> 16) as u8, (tag.color >> 8) as u8, tag.color as u8];
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                tag.color = (rgb[0] as u32) << 16 | (rgb[1] as u32) << 8 | rgb[2] as u32;
            }
        });
        row(ui, &c, "Frames", |ui| {
            let mut from = tag.from + 1;
            let mut to = tag.to + 1;
            ui.add(egui::DragValue::new(&mut from).range(1..=last));
            ui.add(egui::DragValue::new(&mut to).range(1..=last));
            tag.from = from - 1;
            tag.to = to - 1;
        });
        row(ui, &c, "Direction", |ui| {
            egui::ComboBox::from_id_salt("tag_direction")
                .selected_text(DIRECTIONS[(tag.direction as usize).min(3)])
                .width(160.0)
                .show_ui(ui, |ui| {
                    for (i, name) in DIRECTIONS.iter().enumerate() {
                        ui.selectable_value(&mut tag.direction, i as u8, *name);
                    }
                });
        });
        row(ui, &c, "Repeat", |ui| {
            ui.add(egui::DragValue::new(&mut tag.repeat).range(0..=255));
            ui.label(
                RichText::new(if tag.repeat == 0 { "infinite" } else { "times" })
                    .font(ui_font(font::CAPTION))
                    .color(c.text_tertiary),
            );
        });
        outcome = footer(ui, &c, if editing { "Save" } else { "Create" }, editing.then_some("Delete"));
    });
    match outcome {
        Outcome::Primary => {
            let mut t = sheet.tag.clone();
            t.to = t.from.max(t.to);
            let ok = match sheet.index {
                Some(i) => tab.doc.update_tag(i, &t),
                None => tab.doc.add_tag(&t),
            };
            if ok {
                tab.changed();
            }
            true
        }
        Outcome::Destructive => {
            if let Some(i) = sheet.index
                && tab.doc.remove_tag(i)
            {
                tab.changed();
            }
            true
        }
        Outcome::Cancel => true,
        Outcome::Open => !still_open,
    }
}

/// Draws the layer properties sheet; returns true once it should be closed.
pub fn show_layer(ctx: &egui::Context, sheet: &mut LayerSheet, tab: &mut DocTab) -> bool {
    let c = Colors::of(ctx);
    let mut outcome = Outcome::Open;
    let title = if sheet.group { "Group Properties" } else { "Layer Properties" };
    let group = sheet.group;
    let still_open = window(ctx, &c, title, 300.0, |ui| {
        row(ui, &c, "Name", |ui| text_field(ui, &c, &mut sheet.name, 188.0));
        row(ui, &c, "Visible", |ui| {
            ui.checkbox(&mut sheet.visible, "");
        });
        row(ui, &c, "Opacity", |ui| {
            let mut pct = (sheet.opacity as f32 / 255.0 * 100.0).round();
            ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).suffix("%").integer());
            sheet.opacity = (pct / 100.0 * 255.0).round() as u8;
        });
        ui.add_enabled_ui(!group, |ui| {
            row(ui, &c, "Blend mode", |ui| {
                egui::ComboBox::from_id_salt("layer_blend")
                    .selected_text(BLEND_MODES[(sheet.blend as usize).min(BLEND_MODES.len() - 1)])
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        for (i, name) in BLEND_MODES.iter().enumerate() {
                            ui.selectable_value(&mut sheet.blend, i as u16, *name);
                        }
                    });
            });
        });
        outcome = footer(ui, &c, "OK", None);
    });
    match outcome {
        Outcome::Primary => {
            if tab.doc.set_layer_props(sheet.layer, &sheet.name, sheet.opacity, sheet.blend, sheet.visible) {
                tab.changed();
            }
            true
        }
        Outcome::Cancel | Outcome::Destructive => true,
        Outcome::Open => !still_open,
    }
}
