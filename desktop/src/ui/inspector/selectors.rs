//! The Color section's selectors besides the Spectrum: color wheel, RGB / HSV / HSL sliders, Gray and the
//! normal-map disc (`ColorSelectors.swift`). All of them edit a [`Spaces`], the color in every space at once; the
//! math is `pixmith_core::colorpick`.

use super::color::{Spaces, paint_alpha_track, track_slider};
use crate::theme::{Colors, font, metric, radius, ui_font};
use crate::ui::widgets::{self, NumberOpts, ds_icon_button, ds_number_field, ds_popup, rgba_color};
use eframe::egui::{
    self, Color32, ColorImage, Context, Painter, Pos2, Rect, Sense, Stroke, TextureHandle, TextureId, TextureOptions,
    Ui, WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;
use pixmith_core::colorpick::{self, Harmony, WheelModel};

/// The ways to pick a color (`ColorSelector`), in menu order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Selector {
    /// Saturation/value square and hue bar.
    #[default]
    Spectrum,
    /// Hue/saturation wheel with a value or lightness bar.
    Wheel,
    Rgb,
    Hsv,
    Hsl,
    Gray,
    /// A tangent-space normal from a hemisphere disc.
    Normal,
}

impl Selector {
    pub const ALL: [Selector; 7] = [
        Selector::Spectrum,
        Selector::Wheel,
        Selector::Rgb,
        Selector::Hsv,
        Selector::Hsl,
        Selector::Gray,
        Selector::Normal,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Selector::Spectrum => "Spectrum",
            Selector::Wheel => "Color Wheel",
            Selector::Rgb => "RGB Sliders",
            Selector::Hsv => "HSV Sliders",
            Selector::Hsl => "HSL Sliders",
            Selector::Gray => "Gray",
            Selector::Normal => "Normal Map",
        }
    }

    /// The stored name.
    pub fn key(self) -> &'static str {
        match self {
            Selector::Spectrum => "spectrum",
            Selector::Wheel => "wheel",
            Selector::Rgb => "rgb",
            Selector::Hsv => "hsv",
            Selector::Hsl => "hsl",
            Selector::Gray => "gray",
            Selector::Normal => "normal",
        }
    }

    pub fn from_key(key: &str) -> Option<Selector> {
        Selector::ALL.into_iter().find(|s| s.key() == key)
    }
}

/// Titles of the wheel's two color models and of the harmonies, in `WheelModel` / `Harmony::ALL` order.
pub const WHEEL_MODELS: [&str; 2] = ["HSV", "HSL"];
pub const HARMONIES: [&str; 6] =
    ["No Harmony", "Complementary", "Triadic", "Analogous", "Split Complementary", "Square"];

const STORAGE_KEY: &str = "colorPicker";

/// A rendered disc bitmap and what it was rendered for.
struct DiscTex {
    key: (usize, u8, i32),
    tex: TextureHandle,
}

/// What the selectors remember between frames, and what is stored in the preferences: the picked selector, the
/// wheel's model and its harmony.
pub struct PickerState {
    pub selector: Selector,
    pub wheel_model: WheelModel,
    pub harmony: Harmony,
    wheel_tex: Option<DiscTex>,
    normal_tex: Option<DiscTex>,
}

impl Default for PickerState {
    fn default() -> Self {
        PickerState {
            selector: Selector::Spectrum,
            wheel_model: WheelModel::Hsv,
            harmony: Harmony::None,
            wheel_tex: None,
            normal_tex: None,
        }
    }
}

impl PickerState {
    /// The preferences text (`key=value` lines), kept in the app's storage.
    pub fn encode(&self) -> String {
        let model = if self.wheel_model == WheelModel::Hsl { 1 } else { 0 };
        let harmony = Harmony::ALL.iter().position(|h| *h == self.harmony).unwrap_or(0);
        format!("selector={}\nwheelModel={model}\nharmony={harmony}\n", self.selector.key())
    }

    /// Loads the stored choice (`ColorPickerPrefs` in the app's storage).
    pub fn load(&mut self, storage: Option<&dyn eframe::Storage>) {
        if let Some(text) = storage.and_then(|s| s.get_string(STORAGE_KEY)) {
            self.apply_text(&text);
        }
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        storage.set_string(STORAGE_KEY, self.encode());
    }

    /// Reads [`PickerState::encode`] text; unknown or missing lines keep the current values.
    pub fn apply_text(&mut self, text: &str) {
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let n = v.trim().parse::<u32>().ok();
            match k.trim() {
                "selector" => self.selector = Selector::from_key(v.trim()).unwrap_or(self.selector),
                "wheelModel" => self.wheel_model = n.map_or(self.wheel_model, WheelModel::from_u32),
                "harmony" => self.harmony = n.map_or(self.harmony, Harmony::from_u32),
                _ => {}
            }
        }
    }
}

/// The header's selector menu. Returns the newly chosen selector.
pub fn selector_menu(ui: &mut Ui, current: Selector) -> Option<Selector> {
    let mut chosen = None;
    let response = ds_icon_button(ui, icon::PALETTE, metric::CONTROL, 13.0, false, false)
        .on_hover_text(format!("Color selector: {}", current.title()));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Color selector"));
    egui::Popup::menu(&response).show(|ui| {
        for s in Selector::ALL {
            if ui.selectable_label(s == current, s.title()).clicked() {
                chosen = Some(s);
                ui.close();
            }
        }
    });
    chosen
}

// MARK: Slider rows

/// Width of the number field beside a slider.
const FIELD_W: f32 = 64.0;
const TRACK_H: f32 = 14.0;

fn rgb32(rgb: [f64; 3]) -> Color32 {
    rgba_color(colorpick::pack(rgb, 1.0))
}

/// Samples a gradient across `rect`: `at(t)` is the color at fraction `t` (0...1) of the width.
fn gradient(p: &Painter, rect: Rect, cells: usize, at: impl Fn(f64) -> [f64; 3]) {
    widgets::draw_gradient(p, rect, 7.0, cells, 1, |pos| {
        rgb32(at(((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64))
    });
}

/// One slider row: a gradient track and a number field with the channel's letter. `value` is in display units
/// (what the field shows); returns true when either control changed it.
#[allow(clippy::too_many_arguments)]
fn slider_row(
    ui: &mut Ui,
    id: &str,
    label: &str,
    value: &mut f64,
    range: (f64, f64),
    unit: &str,
    help: &str,
    field_w: f32,
    paint: impl FnOnce(&Painter, Rect),
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let w = (ui.available_width() - field_w - 8.0).max(40.0);
        ui.allocate_ui(vec2(w, TRACK_H), |ui| changed |= track_slider(ui, help, value, range, paint));
        let opts = NumberOpts {
            range,
            step: (range.1 - range.0) / 200.0,
            unit,
            width: Some(field_w),
            help,
            ..Default::default()
        };
        changed |= ds_number_field(ui, id, label, value, &opts).changed;
    });
    changed
}

/// The alpha row of the slider selectors.
fn alpha_row(ui: &mut Ui, sp: &mut Spaces) -> bool {
    let rgb = colorpick::unpack(sp.rgba()).0;
    let mut a = sp.hsva.a * 100.0;
    let changed = slider_row(ui, "sel_a", "A", &mut a, (0.0, 100.0), "%", "Alpha", FIELD_W, |p, rect| {
        paint_alpha_track(p, rect, rgb)
    });
    if changed {
        sp.set_alpha(a / 100.0);
    }
    changed
}

/// Red, green, blue and alpha sliders.
pub fn rgb_sliders(ui: &mut Ui, sp: &mut Spaces) -> bool {
    let rgba = sp.rgba();
    let mut ch = [(rgba >> 24 & 0xFF) as f64, (rgba >> 16 & 0xFF) as f64, (rgba >> 8 & 0xFF) as f64];
    let base = ch;
    let names = [("R", "Red"), ("G", "Green"), ("B", "Blue")];
    let mut changed = false;
    for i in 0..3 {
        let others = base;
        let help = names[i].1;
        changed |= slider_row(ui, names[i].0, names[i].0, &mut ch[i], (0.0, 255.0), "", help, FIELD_W, |p, rect| {
            gradient(p, rect, 4, |t| {
                let mut c = [others[0] / 255.0, others[1] / 255.0, others[2] / 255.0];
                c[i] = t;
                c
            })
        });
    }
    if changed {
        let a = (sp.hsva.a * 255.0).round() as u32;
        sp.set_rgba((ch[0] as u32) << 24 | (ch[1] as u32) << 16 | (ch[2] as u32) << 8 | a);
    }
    changed | alpha_row(ui, sp)
}

/// Hue, saturation, value and alpha sliders.
pub fn hsv_sliders(ui: &mut Ui, sp: &mut Spaces) -> bool {
    let [h0, s0, v0] = [sp.hsva.h, sp.hsva.s, sp.hsva.v];
    let (mut h, mut s, mut v) = (h0, s0 * 100.0, v0 * 100.0);
    let mut changed = false;
    changed |= slider_row(ui, "sel_h", "H", &mut h, (0.0, 360.0), "°", "Hue", FIELD_W, |p, rect| {
        gradient(p, rect, 36, |t| colorpick::hsv_to_rgb([t * 360.0, 1.0, 1.0]))
    });
    changed |= slider_row(ui, "sel_s", "S", &mut s, (0.0, 100.0), "%", "Saturation", FIELD_W, |p, rect| {
        gradient(p, rect, 4, |t| colorpick::hsv_to_rgb([h0, t, v0]))
    });
    changed |= slider_row(ui, "sel_v", "V", &mut v, (0.0, 100.0), "%", "Value", FIELD_W, |p, rect| {
        gradient(p, rect, 4, |t| colorpick::hsv_to_rgb([h0, s0, t]))
    });
    if changed {
        sp.set_hsv(h, s / 100.0, v / 100.0);
    }
    changed | alpha_row(ui, sp)
}

/// Hue, saturation, lightness and alpha sliders.
pub fn hsl_sliders(ui: &mut Ui, sp: &mut Spaces) -> bool {
    let [h0, s0, l0] = sp.hsl;
    let (mut h, mut s, mut l) = (h0, s0 * 100.0, l0 * 100.0);
    let mut changed = false;
    changed |= slider_row(ui, "sel_h", "H", &mut h, (0.0, 360.0), "°", "Hue", FIELD_W, |p, rect| {
        gradient(p, rect, 36, |t| colorpick::hsl_to_rgb([t * 360.0, 1.0, 0.5]))
    });
    changed |= slider_row(ui, "sel_s", "S", &mut s, (0.0, 100.0), "%", "Saturation", FIELD_W, |p, rect| {
        gradient(p, rect, 4, |t| colorpick::hsl_to_rgb([h0, t, l0]))
    });
    changed |= slider_row(ui, "sel_l", "L", &mut l, (0.0, 100.0), "%", "Lightness", FIELD_W, |p, rect| {
        gradient(p, rect, 24, |t| colorpick::hsl_to_rgb([h0, s0, t]))
    });
    if changed {
        sp.set_hsl(h, s / 100.0, l / 100.0);
    }
    changed | alpha_row(ui, sp)
}

/// A gray slider (0...255 on the color's luma) and alpha.
pub fn gray_slider(ui: &mut Ui, sp: &mut Spaces) -> bool {
    let rgb = colorpick::unpack(sp.rgba()).0;
    let mut g = (colorpick::luma(rgb) * 255.0).round();
    let mut changed =
        slider_row(ui, "sel_gray", "Gray", &mut g, (0.0, 255.0), "", "Gray level (luma)", 76.0, |p, rect| {
            gradient(p, rect, 4, |t| [t; 3])
        });
    if changed {
        let a = (sp.hsva.a * 255.0).round() as u32;
        let v = g.round().clamp(0.0, 255.0) as u32;
        sp.set_rgba(v << 24 | v << 16 | v << 8 | a);
    }
    changed |= alpha_row(ui, sp);
    changed
}

// MARK: Discs

const DISC: f32 = 136.0;
const BAR: f32 = 14.0;

/// The texture of a disc bitmap, re-rendered when `key` changes.
fn disc_texture(
    ctx: &Context,
    slot: &mut Option<DiscTex>,
    name: &str,
    key: (usize, u8, i32),
    render: impl FnOnce(&mut [u8], usize),
) -> TextureId {
    if slot.as_ref().map(|t| t.key) != Some(key) {
        let n = key.0;
        let mut buf = vec![0u8; n * n * 4];
        render(&mut buf, n);
        let image = ColorImage::from_rgba_unmultiplied([n, n], &buf);
        match slot {
            Some(t) => {
                t.tex.set(image, TextureOptions::LINEAR);
                t.key = key;
            }
            None => *slot = Some(DiscTex { key, tex: ctx.load_texture(name, image, TextureOptions::LINEAR) }),
        }
    }
    slot.as_ref().map_or(TextureId::default(), |t| t.tex.id())
}

/// The pointer position as disc coordinates (-1...1, y down) when the disc is pressed.
fn disc_pointer(response: &egui::Response, rect: Rect) -> Option<(f64, f64)> {
    if !response.is_pointer_button_down_on() {
        return None;
    }
    let p = response.interact_pointer_pos()?;
    let half = rect.width() / 2.0;
    Some((((p.x - rect.center().x) / half) as f64, ((p.y - rect.center().y) / half) as f64))
}

fn disc_point(rect: Rect, x: f64, y: f64) -> Pos2 {
    let half = rect.width() / 2.0;
    rect.center() + vec2(x as f32 * half, y as f32 * half)
}

/// The picked-point ring used on every disc.
fn marker(painter: &Painter, at: Pos2) {
    painter.circle_stroke(at, 7.0, Stroke::new(1.0, Color32::from_black_alpha(102)));
    painter.circle_stroke(at, 6.0, Stroke::new(2.0, Color32::WHITE));
}

/// The disc beside a 14 pt vertical track, or beside a column of fields: lays out `side` and the rest.
fn disc_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        add(ui);
    });
}

/// A field-styled button spanning the width.
fn field_button(ui: &mut Ui, text: &str, help: &str) -> bool {
    let c = Colors::of(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), metric::CONTROL), Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        c.pressed
    } else if response.hovered() {
        c.hover
    } else {
        c.field
    };
    ui.painter().rect_filled(rect, radius::FIELD, fill);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, ui_font(font::BODY), c.text);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, text));
    response.on_hover_text(help).clicked()
}

/// Hue/saturation wheel with a value (or lightness) bar, harmony markers, H/S/V (or L) fields and the model and
/// harmony menus.
pub fn wheel(ui: &mut Ui, pk: &mut PickerState, sp: &mut Spaces) -> bool {
    let model = pk.wheel_model;
    let (h0, s0, v0) = match model {
        WheelModel::Hsv => (sp.hsva.h, sp.hsva.s, sp.hsva.v),
        WheelModel::Hsl => (sp.hsl[0], sp.hsl[1], sp.hsl[2]),
    };
    let (mut h, mut s, mut v) = (h0, s0, v0);
    let ppp = ui.ctx().pixels_per_point();
    let px = (DISC * ppp).round() as usize;
    let model_id = if model == WheelModel::Hsl { 1u8 } else { 0 };
    let tex = disc_texture(
        ui.ctx(),
        &mut pk.wheel_tex,
        "color_wheel",
        (px, model_id, (v0 * 1024.0).round() as i32),
        |buf, n| colorpick::render_wheel(buf, n, model, v0),
    );
    let harmony = pk.harmony;
    let value_name = if model == WheelModel::Hsl { "L" } else { "V" };

    disc_row(ui, |ui| {
        // The wheel.
        let (rect, response) = ui.allocate_exact_size(vec2(DISC, DISC), Sense::click_and_drag());
        let painter = ui.painter();
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Color wheel"));
        painter.image(tex, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        painter.circle_stroke(rect.center(), DISC / 2.0, Stroke::new(1.0, Color32::from_black_alpha(46)));
        if let Some((x, y)) = disc_pointer(&response, rect) {
            (h, s) = colorpick::wheel_hit(x, y, h0);
        }
        let at = {
            let (x, y) = colorpick::wheel_point(h, s);
            disc_point(rect, x, y)
        };
        let hues = colorpick::harmony_hues(h, harmony);
        for hue in &hues {
            let (x, y) = colorpick::wheel_point(*hue, s);
            let p = disc_point(rect, x, y);
            painter.line_segment([rect.center(), p], Stroke::new(1.0, Color32::from_white_alpha(120)));
        }
        for hue in &hues {
            let (x, y) = colorpick::wheel_point(*hue, s);
            let p = disc_point(rect, x, y);
            painter.circle_filled(p, 4.5, rgb32(colorpick::wheel_color(model, *hue, s, v)));
            painter.circle_stroke(p, 5.0, Stroke::new(1.5, Color32::WHITE));
            painter.circle_stroke(p, 6.0, Stroke::new(1.0, Color32::from_black_alpha(90)));
        }
        marker(painter, at);

        // The value / lightness bar: bright at the top.
        let (bar, bresp) = ui.allocate_exact_size(vec2(BAR, DISC), Sense::click_and_drag());
        bresp.widget_info(|| WidgetInfo::slider(true, v, if model == WheelModel::Hsl { "Lightness" } else { "Value" }));
        let color_at = |value: f64| colorpick::wheel_color(model, h, s, value);
        widgets::draw_gradient(ui.painter(), bar, 7.0, 1, 24, |pos| {
            rgb32(color_at(1.0 - ((pos.y - bar.top()) / bar.height()).clamp(0.0, 1.0) as f64))
        });
        if bresp.is_pointer_button_down_on()
            && let Some(p) = bresp.interact_pointer_pos()
        {
            v = 1.0 - (((p.y - bar.top() - 7.0) / (bar.height() - 14.0).max(1.0)).clamp(0.0, 1.0)) as f64;
        }
        let thumb = pos2(bar.center().x, bar.top() + 7.0 + (bar.height() - 14.0) * (1.0 - v as f32));
        let painter = ui.painter();
        painter.circle_filled(thumb + vec2(0.0, 0.5), 8.0, Color32::from_black_alpha(22));
        painter.circle_filled(thumb, 7.0, Color32::WHITE);
        painter.circle_stroke(thumb, 6.5, Stroke::new(1.0, Color32::from_black_alpha(89)));
        bresp.on_hover_text(if model == WheelModel::Hsl { "Lightness" } else { "Value" });

        // A preview of the color and its three fields fill the rest of the row.
        ui.vertical(|ui| {
            let gap = (DISC - 4.0 * metric::CONTROL) / 3.0;
            ui.spacing_mut().item_spacing.y = gap;
            let (preview, _) = ui.allocate_exact_size(vec2(ui.available_width(), metric::CONTROL), Sense::hover());
            widgets::paint_swatch(
                ui.painter(),
                preview,
                colorpick::pack(colorpick::wheel_color(model, h, s, v), sp.hsva.a),
                5.0,
                4.0,
            );
            let w = ui.available_width();
            let hue = NumberOpts { range: (0.0, 359.0), unit: "°", width: Some(w), help: "Hue", ..Default::default() };
            let mut hh = h;
            if ds_number_field(ui, "wheel_h", "H", &mut hh, &hue).changed {
                h = hh;
            }
            let pct = |help| NumberOpts { step: 0.5, unit: "%", width: Some(w), help, ..Default::default() };
            let mut ss = s * 100.0;
            if ds_number_field(ui, "wheel_s", "S", &mut ss, &pct("Saturation")).changed {
                s = ss / 100.0;
            }
            let mut vv = v * 100.0;
            let help = if model == WheelModel::Hsl { "Lightness" } else { "Value" };
            if ds_number_field(ui, "wheel_v", value_name, &mut vv, &pct(help)).changed {
                v = vv / 100.0;
            }
        });
    });

    // The wheel's model and harmony.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let w = (ui.available_width() - 8.0) / 2.0;
        if let Some(i) =
            ds_popup(ui, "wheel_model", model_id as usize, &WHEEL_MODELS, Some(w), false, "Color model of the wheel")
        {
            pk.wheel_model = WheelModel::from_u32(i as u32);
        }
        let current = Harmony::ALL.iter().position(|x| *x == pk.harmony).unwrap_or(0);
        if let Some(i) =
            ds_popup(ui, "wheel_harmony", current, &HARMONIES, Some(w), false, "Mark harmonious colors on the wheel")
        {
            pk.harmony = Harmony::from_u32(i as u32);
        }
    });

    if (h, s, v) == (h0, s0, v0) {
        return false;
    }
    match model {
        WheelModel::Hsv => sp.set_hsv(h, s, v),
        WheelModel::Hsl => sp.set_hsl(h, s, v),
    }
    true
}

/// The normal-map picker: a hemisphere disc, X / Y / Z fields and a button for the flat normal.
pub fn normal_disc(ui: &mut Ui, pk: &mut PickerState, sp: &mut Spaces) -> bool {
    let (rgb, alpha) = colorpick::unpack(sp.rgba());
    let n = colorpick::normal_from_rgb(rgb);
    let mut new_n = None;
    let px = (DISC * ui.ctx().pixels_per_point()).round() as usize;
    let tex = disc_texture(ui.ctx(), &mut pk.normal_tex, "normal_disc", (px, 0, 0), colorpick::render_normal_disc);
    disc_row(ui, |ui| {
        let (rect, response) = ui.allocate_exact_size(vec2(DISC, DISC), Sense::click_and_drag());
        let painter = ui.painter();
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Normal map disc"));
        painter.image(tex, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        painter.circle_stroke(rect.center(), DISC / 2.0, Stroke::new(1.0, Color32::from_black_alpha(46)));
        if let Some((x, y)) = disc_pointer(&response, rect) {
            new_n = Some(colorpick::normal_from_disc(x, y));
        }
        let shown = new_n.unwrap_or(n);
        let (x, y) = colorpick::normal_to_disc(shown);
        marker(ui.painter(), disc_point(rect, x, y));
        response.on_hover_text("Tangent-space normal: drag to tilt. The center is flat (#8080FF).");

        ui.vertical(|ui| {
            let gap = (DISC - 4.0 * metric::CONTROL) / 3.0;
            ui.spacing_mut().item_spacing.y = gap;
            let w = ui.available_width();
            let opts = |help| NumberOpts {
                range: (-1.0, 1.0),
                step: 0.01,
                digits: 2,
                width: Some(w),
                help,
                ..Default::default()
            };
            for (axis, (id, label, help)) in
                [("normal_x", "X", "Normal X (right)"), ("normal_y", "Y", "Normal Y (up)")].into_iter().enumerate()
            {
                let mut v = shown[axis];
                if ds_number_field(ui, id, label, &mut v, &opts(help)).changed {
                    new_n = Some(colorpick::normal_with_xy(shown, axis, v));
                }
            }
            let mut z = shown[2];
            let zopts = NumberOpts { range: (0.0, 1.0), ..opts("Normal Z (out of the surface)") };
            if ds_number_field(ui, "normal_z", "Z", &mut z, &zopts).changed {
                new_n = Some(colorpick::normal_with_z(shown, z));
            }
            if field_button(ui, "Flat", "The flat normal, #8080FF") {
                new_n = Some([0.0, 0.0, 1.0]);
            }
        });
    });
    let Some(new_n) = new_n else { return false };
    let rgba = colorpick::pack(colorpick::normal_to_rgb(new_n), alpha);
    if rgba == sp.rgba() {
        return false;
    }
    sp.set_rgba(rgba);
    true
}
