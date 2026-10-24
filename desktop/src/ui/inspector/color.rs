//! The Color section and the rail's color chips (`ColorPanel.swift`).

use super::InspectorState;
use super::selectors::{self, Selector};
use crate::app::PixmithApp;
use crate::doc::Doc;
use crate::theme::{Colors, font, metric, space, ui_font};
use crate::ui::widgets::{self, NumberOpts, ds_icon_button, ds_number_field, inspector_section, rgba_color};
use eframe::egui::{
    self, Color32, Id, Painter, Rect, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, WidgetInfo, pos2, vec2,
};
use pixmith_core::colorpick;

/// Hue (degrees), saturation, value and alpha, all but hue in 0...1 (`HSVA`). The math is `pixmith_core::colorpick`.
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
        Hsva::from_rgba_hint(rgba, hue.map(|h| (h, 0.0)))
    }

    /// Like [`Hsva::from_rgba`], with a `(hue, saturation)` hint for what black and gray leave undefined.
    pub fn from_rgba_hint(rgba: u32, hint: Option<(f64, f64)>) -> Hsva {
        let (rgb, a) = colorpick::unpack(rgba);
        let [h, s, v] = colorpick::rgb_to_hsv(rgb, hint);
        Hsva { h, s, v, a }
    }

    pub fn rgba(&self) -> u32 {
        colorpick::pack(colorpick::hsv_to_rgb([self.h, self.s, self.v]), self.a)
    }
}

/// The edited color in every space the selectors use. `hsva` always matches the `0xRRGGBBAA` value; `hsl` is kept
/// beside it so an HSL slider holds its value (an RGB round trip would nudge it) and gray keeps its hue.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spaces {
    pub hsva: Hsva,
    /// Hue (degrees), saturation and lightness (0...1).
    pub hsl: [f64; 3],
}

impl Spaces {
    pub fn rgba(&self) -> u32 {
        self.hsva.rgba()
    }

    /// Follows a color that was changed elsewhere (palette, eyedropper, typed hex), keeping hue and saturation where
    /// the new color leaves them undefined.
    pub fn set_rgba(&mut self, rgba: u32) {
        let (rgb, a) = colorpick::unpack(rgba);
        self.hsva = Hsva::from_rgba_hint(rgba, Some((self.hsva.h, self.hsva.s)));
        self.hsl = colorpick::rgb_to_hsl(rgb, Some((self.hsl[0], self.hsl[1])));
        self.hsva.a = a;
    }

    pub fn set_hsv(&mut self, h: f64, s: f64, v: f64) {
        self.hsva = Hsva { h, s, v, a: self.hsva.a };
        let rgb = colorpick::unpack(self.hsva.rgba()).0;
        self.hsl = colorpick::rgb_to_hsl(rgb, Some((h, self.hsl[1])));
    }

    pub fn set_hsl(&mut self, h: f64, s: f64, l: f64) {
        self.hsl = [h, s, l];
        let a = self.hsva.a;
        let rgba = colorpick::pack(colorpick::hsl_to_rgb([h, s, l]), a);
        self.hsva = Hsva::from_rgba_hint(rgba, Some((h, self.hsva.s)));
    }

    pub fn set_alpha(&mut self, a: f64) {
        self.hsva.a = a;
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
fn sv_plane(ui: &mut Ui, sp: &mut Spaces) -> bool {
    let hsva = sp.hsva;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 96.0), Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::labeled(egui::WidgetType::Button, true, "Saturation and value"));
    let base = rgba_color(Hsva { s: 1.0, v: 1.0, a: 1.0, ..hsva }.rgba());
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
        if changed {
            sp.set_hsv(hsva.h, s, v);
        }
    }
    changed
}

/// A horizontal track with a draggable thumb; `paint` draws the track (`TrackSlider`).
pub(super) fn track_slider(
    ui: &mut Ui,
    name: &str,
    value: &mut f64,
    range: (f64, f64),
    paint: impl FnOnce(&Painter, Rect),
) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::slider(true, *value, name));
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

/// The hex field with a swatch of the color, and optionally an alpha field. Returns whether alpha changed and the
/// color typed into the hex field on commit.
fn hex_row(ui: &mut Ui, st_hex: &mut String, current: u32, sp: &mut Spaces, with_alpha: bool) -> (bool, Option<u32>) {
    let c = Colors::of(ui.ctx());
    let hex_id = Id::new("inspector_hex");
    let mut changed = false;
    let mut typed = None;
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.horizontal(|ui| {
        let w = if with_alpha { ui.available_width() - 6.0 - 70.0 } else { ui.available_width() };
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
        let chip = Rect::from_center_size(pos2(rect.left() + space::SM + 6.0, rect.center().y), vec2(12.0, 12.0));
        widgets::paint_swatch(ui.painter(), chip, current, 3.0, 2.4);
        let text_rect =
            Rect::from_min_max(pos2(chip.right() + 6.0, rect.top()), pos2(rect.right() - space::SM, rect.bottom()));
        let edit = TextEdit::singleline(st_hex)
            .id(hex_id)
            .font(ui_font(font::BODY))
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .vertical_align(egui::Align::Center)
            .desired_width(text_rect.width());
        let r = widgets::put_in(ui, text_rect, edit).on_hover_text("Hex color: RRGGBB");
        if r.lost_focus() {
            match parse_hex(st_hex, sp.hsva.a) {
                Some(rgba) => {
                    sp.set_rgba(rgba);
                    typed = Some(sp.rgba());
                }
                None => *st_hex = hex_string(current),
            }
        }
        if with_alpha {
            let opts = NumberOpts {
                step: 0.5,
                unit: "%",
                width: Some(70.0),
                help: "Alpha. Drag the label to scrub.",
                ..Default::default()
            };
            let mut a = sp.hsva.a * 100.0;
            if ds_number_field(ui, "color_a", "A", &mut a, &opts).changed {
                sp.set_alpha(a / 100.0);
                changed = true;
            }
        }
    });
    (changed, typed)
}

/// Paints the alpha track: a checkerboard under the color from transparent to opaque.
pub(super) fn paint_alpha_track(p: &Painter, rect: Rect, rgb: [f64; 3]) {
    widgets::draw_checker(p, rect, 4.0, 7.0);
    widgets::draw_gradient(p, rect, 7.0, 36, 1, |pos| {
        let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64;
        rgba_color(colorpick::pack(rgb, t))
    });
}

/// The alpha slider of the selectors that have no alpha row.
fn alpha_slider(ui: &mut Ui, sp: &mut Spaces) -> bool {
    let rgb = colorpick::unpack(sp.rgba()).0;
    let mut a = sp.hsva.a;
    let changed = track_slider(ui, "Alpha", &mut a, (0.0, 1.0), |p, rect| paint_alpha_track(p, rect, rgb));
    if changed {
        sp.set_alpha(a);
    }
    changed
}

/// The Spectrum selector's H, S and V fields.
fn hsv_fields(ui: &mut Ui, sp: &mut Spaces) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 12.0) / 3.0;
        let Hsva { h, s, v, .. } = sp.hsva;
        let mut hh = h;
        let hue = NumberOpts { range: (0.0, 359.0), unit: "°", width: Some(w), help: "Hue", ..Default::default() };
        if ds_number_field(ui, "color_h", "H", &mut hh, &hue).changed {
            sp.set_hsv(hh, s, v);
            changed = true;
        }
        let pct = |help| NumberOpts { step: 0.5, unit: "%", width: Some(w), help, ..Default::default() };
        let mut ss = s * 100.0;
        if ds_number_field(ui, "color_s", "S", &mut ss, &pct("Saturation")).changed {
            sp.set_hsv(h, ss / 100.0, v);
            changed = true;
        }
        let mut vv = v * 100.0;
        if ds_number_field(ui, "color_v", "V", &mut vv, &pct("Value")).changed {
            sp.set_hsv(h, s, vv / 100.0);
            changed = true;
        }
    });
    changed
}

/// The Color section: a selector menu in the header (Spectrum, Wheel, RGB, HSV and HSL sliders, Gray, Normal
/// map), the picked selector, and the hex field. Edits the primary or the secondary color, whichever is selected
/// in the header (or on the tool rail).
pub fn section(ui: &mut Ui, st: &mut InspectorState, primary: &mut u32, secondary: &mut u32, doc: &Doc) {
    let hex_id = Id::new("inspector_hex");
    let hex_focused = ui.memory(|m| m.has_focus(hex_id));
    let current = if st.secondary { *secondary } else { *primary };
    // `pull`: follow changes made elsewhere (palette, eyedropper, swap, rail).
    if current != st.spaces.rgba() {
        st.spaces.set_rgba(current);
    }
    if !hex_focused {
        st.hex = hex_string(current);
    }

    let (secondary_target, prim, sec) = (st.secondary, *primary, *secondary);
    let selector = st.picker.selector;
    let mut new_target = None;
    let mut new_selector = None;
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
            new_selector = selectors::selector_menu(ui, selector);
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = space::SM;
            let sp = &mut st.spaces;
            let mut changed = false;
            let mut alpha_field = true;
            match selector {
                Selector::Spectrum => {
                    changed |= sv_plane(ui, sp);
                    let hsva = sp.hsva;
                    let hue_color = |h: f64| rgba_color(colorpick::pack(colorpick::hsv_to_rgb([h, 1.0, 1.0]), 1.0));
                    let mut h = hsva.h;
                    if track_slider(ui, "Hue", &mut h, (0.0, 360.0), |p, rect| {
                        widgets::draw_gradient(p, rect, 7.0, 36, 1, |pos| {
                            hue_color(((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64 * 360.0)
                        });
                    }) {
                        sp.set_hsv(h, hsva.s, hsva.v);
                        changed = true;
                    }
                    changed |= alpha_slider(ui, sp);
                }
                Selector::Wheel => {
                    changed |= selectors::wheel(ui, &mut st.picker, sp);
                    changed |= alpha_slider(ui, sp);
                }
                Selector::Normal => {
                    changed |= selectors::normal_disc(ui, &mut st.picker, sp);
                    changed |= alpha_slider(ui, sp);
                }
                Selector::Rgb => {
                    changed |= selectors::rgb_sliders(ui, sp);
                    alpha_field = false;
                }
                Selector::Hsv => {
                    changed |= selectors::hsv_sliders(ui, sp);
                    alpha_field = false;
                }
                Selector::Hsl => {
                    changed |= selectors::hsl_sliders(ui, sp);
                    alpha_field = false;
                }
                Selector::Gray => {
                    changed |= selectors::gray_slider(ui, sp);
                    alpha_field = false;
                }
            }
            let (alpha_changed, typed) = hex_row(ui, &mut st.hex, current, sp, alpha_field);
            changed |= alpha_changed;
            if selector == Selector::Spectrum {
                changed |= hsv_fields(ui, sp);
            }
            if changed {
                new_color = Some(sp.rgba());
            }
            if typed.is_some() {
                new_color = typed;
            }
        },
    );
    if let Some(secondary_target) = new_target {
        st.secondary = secondary_target;
    }
    if let Some(s) = new_selector {
        st.picker.selector = s;
    }
    if let Some(rgba) = new_color {
        // Only a real change repaints: a held pointer on a snapping or quantizing control re-sends the same color.
        let target = if st.secondary { &mut *secondary } else { &mut *primary };
        let rgba = if st.secondary { rgba } else { doc.snap_color(rgba) };
        if *target != rgba {
            *target = rgba;
            ui.ctx().request_repaint();
        }
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
