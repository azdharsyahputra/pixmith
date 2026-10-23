//! The Color section and the rail's color chips (`ColorPanel.swift`).

use super::InspectorState;
use crate::app::PixmithApp;
use crate::doc::Doc;
use crate::theme::{Colors, font, metric, space, ui_font};
use crate::ui::widgets::{self, NumberOpts, ds_icon_button, ds_number_field, inspector_section, rgba_color};
use eframe::egui::{self, Color32, Id, Painter, Rect, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, pos2, vec2};

/// Hue (degrees), saturation, value and alpha, all but hue in 0...1 (`HSVA`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hsva {
    pub h: f64,
    pub s: f64,
    pub v: f64,
    pub a: f64,
}

impl Hsva {
    /// From `0xRRGGBBAA`. `hue` is kept when the color is gray (hue is undefined there) so dragging stays stable.
    pub fn from_rgba(rgba: u32, hue: Option<f64>) -> Hsva {
        let r = ((rgba >> 24) & 0xFF) as f64 / 255.0;
        let g = ((rgba >> 16) & 0xFF) as f64 / 255.0;
        let b = ((rgba >> 8) & 0xFF) as f64 / 255.0;
        let a = (rgba & 0xFF) as f64 / 255.0;
        let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
        let d = mx - mn;
        let v = mx;
        let s = if mx == 0.0 { 0.0 } else { d / mx };
        let mut h = if d == 0.0 {
            hue.unwrap_or(0.0)
        } else if mx == r {
            60.0 * (((g - b) / d) % 6.0)
        } else if mx == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        if h < 0.0 {
            h += 360.0;
        }
        if (s == 0.0 || v == 0.0)
            && let Some(hue) = hue
        {
            h = hue;
        }
        Hsva { h, s, v, a }
    }

    pub fn rgba(&self) -> u32 {
        let c = self.v * self.s;
        let hp = (self.h % 360.0) / 60.0;
        let x = c * (1.0 - ((hp % 2.0) - 1.0).abs());
        let (r1, g1, b1) = match hp as i32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = self.v - c;
        let byte = |f: f64| (f * 255.0).round().clamp(0.0, 255.0) as u32;
        byte(r1 + m) << 24 | byte(g1 + m) << 16 | byte(b1 + m) << 8 | byte(self.a)
    }
}

fn hex_string(c: u32) -> String {
    format!("{:06X}", c >> 8)
}

/// Parses what the hex field holds: `RGB`, `RRGGBB` or `RRGGBBAA`, with an optional `#`.
fn parse_hex(text: &str, alpha: f64) -> Option<u32> {
    let mut t = text.trim_matches(|c| c == '#' || c == ' ').to_uppercase();
    if t.len() == 3 {
        t = t.chars().flat_map(|c| [c, c]).collect();
    }
    if (t.len() != 6 && t.len() != 8) || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(&t, 16).ok()?;
    Some(if t.len() == 6 { v << 8 | (alpha * 255.0).round() as u32 } else { v })
}

/// Saturation (x) and value (y) of the current hue (`SVPlane`). Returns true if the user changed them.
fn sv_plane(ui: &mut Ui, hsva: &mut Hsva) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 96.0), Sense::click_and_drag());
    let base = rgba_color(Hsva { s: 1.0, v: 1.0, a: 1.0, ..*hsva }.rgba());
    let radius = crate::theme::radius::CONTROL as f32;
    let painter = ui.painter();
    widgets::draw_gradient(painter, rect, radius, 48, 24, |p| {
        let x = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let y = ((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        let mix = |w: u8, b: u8| ((w as f32 * (1.0 - x) + b as f32 * x) * (1.0 - y)).round() as u8;
        Color32::from_rgb(mix(255, base.r()), mix(255, base.g()), mix(255, base.b()))
    });
    painter.rect_stroke(rect, radius, Stroke::new(1.0, Color32::from_black_alpha(51)), StrokeKind::Middle);
    let center = pos2(rect.left() + hsva.s as f32 * rect.width(), rect.top() + (1.0 - hsva.v as f32) * rect.height());
    let clipped = painter.with_clip_rect(rect);
    clipped.circle_stroke(center, 7.0, Stroke::new(1.0, Color32::from_black_alpha(102)));
    clipped.circle_stroke(center, 6.0, Stroke::new(2.0, Color32::WHITE));
    let mut changed = false;
    if response.is_pointer_button_down_on()
        && let Some(p) = response.interact_pointer_pos()
    {
        let s = (((p.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0)) as f64;
        let v = 1.0 - (((p.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0)) as f64;
        changed = s != hsva.s || v != hsva.v;
        hsva.s = s;
        hsva.v = v;
    }
    changed
}

/// A horizontal track with a draggable thumb; `paint` draws the track (`TrackSlider`).
fn track_slider(ui: &mut Ui, value: &mut f64, range: (f64, f64), paint: impl FnOnce(&Painter, Rect)) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::click_and_drag());
    let painter = ui.painter();
    paint(painter, rect);
    let f = ((*value - range.0) / (range.1 - range.0)).clamp(0.0, 1.0) as f32;
    let center = pos2(rect.left() + 7.0 + (rect.width() - 14.0) * f, rect.center().y);
    painter.circle_filled(center + vec2(0.0, 0.5), 8.0, Color32::from_black_alpha(22));
    painter.circle_filled(center, 7.0, Color32::WHITE);
    painter.circle_stroke(center, 6.5, Stroke::new(1.0, Color32::from_black_alpha(89)));
    let mut changed = false;
    if response.is_pointer_button_down_on()
        && let Some(p) = response.interact_pointer_pos()
    {
        let t = ((p.x - rect.left() - 7.0) / (rect.width() - 14.0).max(1.0)).clamp(0.0, 1.0) as f64;
        let v = range.0 + t * (range.1 - range.0);
        changed = v != *value;
        *value = v;
    }
    changed
}

/// One of the two target buttons in the header: a 14 pt swatch in a 24 pt hit area.
fn target_button(ui: &mut Ui, name: &str, rgba: u32, selected: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(metric::CONTROL, metric::CONTROL), Sense::click());
    let c = Colors::of(ui.ctx());
    let r = Rect::from_center_size(rect.center(), vec2(14.0, 14.0));
    widgets::paint_swatch(ui.painter(), r, rgba, 4.0, 2.8);
    if selected {
        ui.painter().rect_stroke(r.expand(2.5), 5.5, Stroke::new(2.0, c.accent), StrokeKind::Middle);
    }
    response.on_hover_text(format!("Edit the {name} color")).clicked()
}

/// The Color section: saturation-value plane, hue and alpha sliders, hex, alpha and HSV fields. Edits the
/// primary or the secondary color, whichever is selected in the header (or on the tool rail).
pub fn section(ui: &mut Ui, st: &mut InspectorState, primary: &mut u32, secondary: &mut u32, doc: &Doc) {
    let hex_id = Id::new("inspector_hex");
    let hex_focused = ui.memory(|m| m.has_focus(hex_id));
    let current = if st.secondary { *secondary } else { *primary };
    // `pull`: follow changes made elsewhere (palette, eyedropper, swap, rail).
    if current != st.hsva.rgba() {
        st.hsva = Hsva::from_rgba(current, Some(st.hsva.h));
    }
    if !hex_focused {
        st.hex = hex_string(current);
    }

    let (secondary_target, prim, sec) = (st.secondary, *primary, *secondary);
    let mut new_target = None;
    let mut new_color: Option<u32> = None;
    inspector_section(
        ui,
        "color",
        "Color",
        Some(if secondary_target { "Secondary" } else { "Primary" }),
        |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if target_button(ui, "secondary", sec, secondary_target) {
                new_target = Some(true);
            }
            if target_button(ui, "primary", prim, !secondary_target) {
                new_target = Some(false);
            }
        },
        |ui| {
            let c = Colors::of(ui.ctx());
            ui.spacing_mut().item_spacing.y = space::SM;
            let hsva = &mut st.hsva;
            let mut changed = sv_plane(ui, hsva);

            let hue_color = |h: f64| rgba_color(Hsva { h, s: 1.0, v: 1.0, a: 1.0 }.rgba());
            changed |= track_slider(ui, &mut hsva.h, (0.0, 360.0), |p, rect| {
                widgets::draw_gradient(p, rect, 7.0, 36, 1, |pos| {
                    hue_color(((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64 * 360.0)
                });
            });
            let (h, s, v) = (hsva.h, hsva.s, hsva.v);
            changed |= track_slider(ui, &mut hsva.a, (0.0, 1.0), |p, rect| {
                widgets::draw_checker(p, rect, 4.0, 7.0);
                widgets::draw_gradient(p, rect, 7.0, 36, 1, |pos| {
                    let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64;
                    rgba_color(Hsva { h, s, v, a: t }.rgba())
                });
            });

            ui.spacing_mut().item_spacing.x = 6.0;
            ui.horizontal(|ui| {
                // Hex field with a swatch of the current color.
                let w = ui.available_width() - 6.0 - 70.0;
                let (rect, _) = ui.allocate_exact_size(vec2(w, metric::CONTROL), Sense::hover());
                let focused = ui.memory(|m| m.has_focus(hex_id));
                ui.painter().rect_filled(rect, crate::theme::radius::FIELD, c.field);
                if focused {
                    ui.painter().rect_stroke(
                        rect,
                        crate::theme::radius::FIELD,
                        Stroke::new(1.5, c.accent_line),
                        StrokeKind::Middle,
                    );
                }
                let chip =
                    Rect::from_center_size(pos2(rect.left() + space::SM + 6.0, rect.center().y), vec2(12.0, 12.0));
                widgets::paint_swatch(ui.painter(), chip, current, 3.0, 2.4);
                let text_rect = Rect::from_min_max(
                    pos2(chip.right() + 6.0, rect.top()),
                    pos2(rect.right() - space::SM, rect.bottom()),
                );
                let edit = TextEdit::singleline(&mut st.hex)
                    .id(hex_id)
                    .font(ui_font(font::BODY))
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::ZERO)
                    .vertical_align(egui::Align::Center)
                    .desired_width(text_rect.width());
                let r = widgets::put_in(ui, text_rect, edit).on_hover_text("Hex color: RRGGBB");
                if r.lost_focus() {
                    match parse_hex(&st.hex, st.hsva.a) {
                        Some(rgba) => {
                            st.hsva = Hsva::from_rgba(rgba, Some(st.hsva.h));
                            new_color = Some(st.hsva.rgba());
                        }
                        None => st.hex = hex_string(current),
                    }
                }
                let hsva = &mut st.hsva;
                let opts = NumberOpts {
                    step: 0.5,
                    unit: "%",
                    width: Some(70.0),
                    help: "Alpha. Drag the label to scrub.",
                    ..Default::default()
                };
                let mut a = hsva.a * 100.0;
                if ds_number_field(ui, "color_a", "A", &mut a, &opts).changed {
                    hsva.a = a / 100.0;
                    changed = true;
                }
            });
            ui.horizontal(|ui| {
                let hsva = &mut st.hsva;
                let w = (ui.available_width() - 12.0) / 3.0;
                let mut h = hsva.h;
                let hue =
                    NumberOpts { range: (0.0, 359.0), unit: "°", width: Some(w), help: "Hue", ..Default::default() };
                if ds_number_field(ui, "color_h", "H", &mut h, &hue).changed {
                    hsva.h = h;
                    changed = true;
                }
                let pct = |help| NumberOpts { step: 0.5, unit: "%", width: Some(w), help, ..Default::default() };
                let mut s = hsva.s * 100.0;
                if ds_number_field(ui, "color_s", "S", &mut s, &pct("Saturation")).changed {
                    hsva.s = s / 100.0;
                    changed = true;
                }
                let mut v = hsva.v * 100.0;
                if ds_number_field(ui, "color_v", "V", &mut v, &pct("Value")).changed {
                    hsva.v = v / 100.0;
                    changed = true;
                }
            });
            if changed {
                new_color = Some(st.hsva.rgba());
            }
        },
    );
    if let Some(secondary_target) = new_target {
        st.secondary = secondary_target;
    }
    if let Some(rgba) = new_color {
        if st.secondary {
            *secondary = rgba;
        } else {
            *primary = doc.snap_color(rgba);
        }
        ui.ctx().request_repaint();
    }
}

/// Selects a color for editing and makes sure the Color section is open (`ColorTarget.edit`).
pub fn edit_color(ctx: &egui::Context, st: &mut InspectorState, secondary: bool) {
    st.secondary = secondary;
    widgets::set_section_collapsed(ctx, "color", false);
}

/// One chip of the rail: a 22 pt swatch with either the accent ring (being edited) or a panel-colored gap.
fn chip(painter: &Painter, c: &Colors, rect: Rect, rgba: u32, ring: bool) {
    widgets::paint_swatch(painter, rect, rgba, 5.0, 22.0 / 5.0);
    if ring {
        painter.rect_stroke(rect.expand(1.0), 5.0, Stroke::new(2.0, c.accent), StrokeKind::Middle);
    } else {
        painter.rect_stroke(rect.expand(1.5), 5.0, Stroke::new(1.0, c.separator), StrokeKind::Middle);
        painter.rect_stroke(rect.expand(1.5), 5.0, Stroke::new(1.5, c.panel), StrokeKind::Middle);
    }
}

/// The primary and secondary color chips, overlapped, with a swap button (`ColorChips`). Click one to edit it
/// in the Color section. Placed at the bottom of the rail's `ui`.
pub fn chips(app: &mut PixmithApp, ui: &mut Ui) {
    let c = Colors::of(ui.ctx());
    let area = ui.max_rect();
    let top = area.bottom() - space::SM - 36.0 - space::XS - metric::CONTROL;
    let origin = pos2(area.center().x - 18.0, top);
    let primary_rect = Rect::from_min_size(origin, vec2(22.0, 22.0));
    let secondary_rect = primary_rect.translate(vec2(14.0, 14.0));
    let editing_secondary = app.inspector.secondary;

    // Secondary first: the primary chip overlaps it, and the later widget wins the click.
    let sec = ui.interact(secondary_rect, Id::new("rail_chip_secondary"), Sense::click());
    let prim = ui.interact(primary_rect, Id::new("rail_chip_primary"), Sense::click());
    chip(ui.painter(), &c, secondary_rect, app.secondary, editing_secondary);
    chip(ui.painter(), &c, primary_rect, app.primary, !editing_secondary);
    if sec.on_hover_text("Secondary color (right click). Click to edit it.").clicked() {
        edit_color(ui.ctx(), &mut app.inspector, true);
    }
    if prim.on_hover_text("Primary color (left click). Click to edit it.").clicked() {
        edit_color(ui.ctx(), &mut app.inspector, false);
    }

    let swap = Rect::from_min_size(pos2(area.center().x - 12.0, top + 36.0 + space::XS), vec2(24.0, 24.0));
    let mut child = ui.new_child(UiBuilder::new().max_rect(swap));
    if ds_icon_button(&mut child, egui_phosphor::regular::ARROWS_LEFT_RIGHT, 24.0, 13.0, false, false)
        .on_hover_text("Swap colors (X)")
        .clicked()
    {
        std::mem::swap(&mut app.primary, &mut app.secondary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsva_roundtrips_the_screenshot_color() {
        let h = Hsva::from_rgba(0xF2A65AFF, None);
        assert_eq!((h.h.round(), (h.s * 100.0).round(), (h.v * 100.0).round()), (30.0, 63.0, 95.0));
        assert_eq!(h.rgba(), 0xF2A65AFF);
    }

    #[test]
    fn gray_keeps_the_hue() {
        assert_eq!(Hsva::from_rgba(0x808080FF, Some(200.0)).h, 200.0);
    }

    #[test]
    fn hex_forms() {
        assert_eq!(parse_hex("#f0a", 1.0), Some(0xFF00AAFF));
        assert_eq!(parse_hex("102030", 0.5), Some(0x10203080));
        assert_eq!(parse_hex("10203040", 1.0), Some(0x10203040));
        assert_eq!(parse_hex("zz", 1.0), None);
    }
}
