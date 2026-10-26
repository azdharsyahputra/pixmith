//! The Tween panel (`Panels/TweenView.swift`): a floating panel with the Key, Dope sheet, Pivot and options,
//! Canvas and Bake sections, and the Bezier curve editor. The logic is in the core (`gamedev/tween.rs`);
//! this file only drives it, the way the Swift view does. The canvas overlay and handles are in
//! `tween_canvas.rs`.

use super::form::{self, ORANGE};
use super::{TweenEase, TweenKey, TweenProp};
use crate::app::{DocTab, PixmithApp};
use crate::theme::{Colors, font, metric, radius, space, ui_font};
use crate::ui::widgets::{self, SegGlyph};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as icon;

/// Dope sheet metrics.
const CELL: f32 = 24.0;
const ROW_HEIGHT: f32 = 22.0;
const LABEL_WIDTH: f32 = 32.0;
const FORM_LABEL: f32 = 84.0;
/// The Bezier editor's curve box, and the value range it shows vertically.
const CURVE_SIZE: f32 = 156.0;
const Y_LO: f64 = -0.5;
const Y_HI: f64 = 1.5;

const PRESETS: [(&str, [f64; 4]); 7] = [
    ("Linear", [0.0, 0.0, 1.0, 1.0]),
    ("Ease", [0.25, 0.1, 0.25, 1.0]),
    ("Ease In", [0.42, 0.0, 1.0, 1.0]),
    ("Ease Out", [0.0, 0.0, 0.58, 1.0]),
    ("Ease In-Out", [0.42, 0.0, 0.58, 1.0]),
    ("Snap", [0.8, 0.0, 0.2, 1.0]),
    ("Back", [0.34, 1.56, 0.64, 1.0]),
];

/// A key being dragged along its dope sheet row.
#[derive(Clone, Copy, Debug)]
pub struct DopeDrag {
    pub prop: TweenProp,
    pub frame: u32,
    pub dx: f32,
}

/// UI state of the panel and the canvas (`TweenUI` plus the view's `@State`).
pub struct TweenState {
    pub open: bool,
    pub edit_on_canvas: bool,
    pub auto_key: bool,
    pub sel_prop: TweenProp,
    pub sel_frame: Option<u32>,
    pub value_text: String,
    pub ease: TweenEase,
    pub bezier: [f64; 4],
    pub pivot_x: String,
    pub pivot_y: String,
    pub copy_from: String,
    pub copy_to: String,
    pub bake_mode: usize,
    pub bake_from: String,
    pub bake_to: String,
    pub bake_new_layer: bool,
    pub bake_keep: bool,
    /// (document, layer, frame, property) the key fields were loaded for.
    loaded: Option<(usize, u32, u32, TweenProp)>,
    loaded_pivot: Option<(usize, u32)>,
    dope_drag: Option<DopeDrag>,
    bezier_drag: Option<usize>,
    pub(super) canvas_drag: Option<super::tween_canvas::CanvasDrag>,
}

impl Default for TweenState {
    fn default() -> TweenState {
        TweenState {
            open: false,
            edit_on_canvas: false,
            auto_key: true,
            sel_prop: TweenProp::PosX,
            sel_frame: None,
            value_text: "0".into(),
            ease: TweenEase::Linear,
            bezier: [0.25, 0.1, 0.25, 1.0],
            pivot_x: String::new(),
            pivot_y: String::new(),
            copy_from: "1".into(),
            copy_to: "1".into(),
            bake_mode: 0,
            bake_from: "1".into(),
            bake_to: "1".into(),
            bake_new_layer: false,
            bake_keep: false,
            loaded: None,
            loaded_pivot: None,
            dope_drag: None,
            bezier_drag: None,
            canvas_drag: None,
        }
    }
}

impl TweenState {
    /// View > Tween: opens or closes the panel. Closing also turns off "Edit on canvas".
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if !self.open {
            self.edit_on_canvas = false;
            self.canvas_drag = None;
        }
    }

    fn bezier_floats(&self) -> [f32; 4] {
        self.bezier.map(|v| v as f32)
    }

    /// Fills the key fields from the key at the current frame, or from the value there (`loadKey`).
    pub fn load_key(&mut self, tab: &DocTab) {
        let keys = tab.doc.tween_keys(tab.layer, self.sel_prop);
        if let Some(k) = keys.iter().find(|k| k.frame == tab.frame) {
            self.value_text = fmt_g(k.value as f64);
            self.ease = k.ease;
            if k.ease == TweenEase::Bezier {
                self.bezier = k.bezier.map(|v| v as f64);
            }
            self.sel_frame = Some(k.frame);
        } else {
            self.value_text = fmt_g(tab.doc.tween_value(tab.layer, self.sel_prop, tab.frame) as f64);
        }
    }

    pub fn load_pivot(&mut self, tab: &DocTab) {
        if let Some((x, y)) = tab.doc.tween_pivot(tab.layer) {
            self.pivot_x = fmt_g(x as f64);
            self.pivot_y = fmt_g(y as f64);
        }
    }

    /// The `onAppear` / `onChange` reloads: when the document, layer, frame or property changed.
    pub fn sync(&mut self, tab: &DocTab) {
        let doc = tab.doc.raw() as usize;
        let key = (doc, tab.layer, tab.frame, self.sel_prop);
        if self.loaded != Some(key) {
            self.loaded = Some(key);
            self.load_key(tab);
        }
        if self.loaded_pivot != Some((doc, tab.layer)) {
            self.loaded_pivot = Some((doc, tab.layer));
            self.load_pivot(tab);
        }
    }

    fn current_key(&self, tab: &DocTab) -> Option<TweenKey> {
        tab.doc.tween_keys(tab.layer, self.sel_prop).into_iter().find(|k| k.frame == tab.frame)
    }

    // MARK: Actions (the buttons of the panel)

    /// "Set Key at Current Frame".
    pub fn set_key(&mut self, tab: &mut DocTab) {
        let value = self.value_text.trim().parse::<f32>().unwrap_or(0.0);
        let (layer, frame, prop, ease) = (tab.layer, tab.frame, self.sel_prop, self.ease);
        if tab.doc.tween_set_key(layer, prop, frame, value, ease, self.bezier_floats()) {
            tab.changed();
        }
        self.sel_frame = Some(tab.frame);
    }

    /// "Delete Key".
    pub fn delete_key(&mut self, tab: &mut DocTab) {
        let frame = self.sel_frame.unwrap_or(tab.frame);
        if tab.doc.tween_remove_key(tab.layer, self.sel_prop, frame) {
            tab.changed();
        }
    }

    /// The Ease to next popup changed to `ease`.
    pub fn choose_ease(&mut self, tab: &mut DocTab, ease: TweenEase) {
        let had = self.current_key(tab);
        self.ease = ease;
        if let Some(key) = had
            && (ease != TweenEase::Bezier || key.ease != TweenEase::Bezier)
            && tab.doc.tween_set_ease(tab.layer, self.sel_prop, tab.frame, ease, self.bezier_floats())
        {
            tab.changed();
        }
    }

    /// The Bezier handles were released: write the curve to the key at the current frame.
    pub fn commit_bezier(&mut self, tab: &mut DocTab) {
        if self.current_key(tab).is_some()
            && tab.doc.tween_set_ease(tab.layer, self.sel_prop, tab.frame, TweenEase::Bezier, self.bezier_floats())
        {
            tab.changed();
        }
    }

    /// Copy: the keys between the two frames (1-based fields).
    pub fn copy_keys(&self, tab: &DocTab) -> u32 {
        let (from, to) = (frame_field(&self.copy_from), frame_field(&self.copy_to));
        tab.doc.tween_copy_keys(tab.layer, from, to)
    }

    /// "Paste at Current Frame".
    pub fn paste_keys(&mut self, tab: &mut DocTab) {
        if tab.doc.tween_paste_keys(tab.layer, tab.frame) {
            tab.changed();
        }
    }

    pub fn apply_pivot(&mut self, tab: &mut DocTab) {
        if let (Ok(x), Ok(y)) = (self.pivot_x.trim().parse::<f32>(), self.pivot_y.trim().parse::<f32>())
            && tab.doc.tween_set_pivot(tab.layer, x, y, true)
        {
            tab.changed();
        }
    }

    pub fn center_pivot(&mut self, tab: &mut DocTab) {
        if tab.doc.tween_set_pivot(tab.layer, 0.0, 0.0, false) {
            tab.changed();
        }
        self.load_pivot(tab);
    }

    /// "Bake" (`keep` false) and "Bake & Keep Source" (`keep` true).
    pub fn bake(&mut self, tab: &mut DocTab, keep: bool) {
        let from = if self.bake_mode == 2 { tab.frame } else { frame_field(&self.bake_from) };
        let to = frame_field(&self.bake_to);
        if tab.doc.tween_bake(tab.layer, self.bake_mode as u32, from, to, self.bake_new_layer, keep) {
            tab.changed();
        }
    }

    pub fn bake_all(&mut self, tab: &mut DocTab) {
        if tab.doc.tween_bake_all(self.bake_keep, self.bake_new_layer) {
            tab.changed();
        }
    }

    /// Drags a key to another frame in its row (`DopeSheet` drag end).
    pub fn move_key(&mut self, tab: &mut DocTab, prop: TweenProp, from: u32, to: u32) {
        if to != from && tab.doc.tween_move_key(tab.layer, prop, from, to) {
            tab.changed();
        }
        self.sel_prop = prop;
        self.sel_frame = Some(to);
    }
}

/// A 1-based frame field as a 0-based frame (`(Int(text) ?? 1) - 1`, at least 0).
fn frame_field(text: &str) -> u32 {
    (text.trim().parse::<i64>().unwrap_or(1) - 1).max(0) as u32
}

/// C's `%g` for a float: six significant digits, no trailing zeros.
pub fn fmt_g(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return if v.is_finite() { "0".into() } else { v.to_string() };
    }
    let exp = v.abs().log10().floor() as i32;
    // Rounding can carry into the next power of ten.
    let exp = if format!("{:.5e}", v.abs()).split('e').nth(1).and_then(|e| e.parse::<i32>().ok()).unwrap_or(exp) != exp
    {
        exp + 1
    } else {
        exp
    };
    if !(-4..6).contains(&exp) {
        let s = format!("{:.5e}", v);
        let (mant, e) = s.split_once('e').unwrap_or((&s, "0"));
        let mant = if mant.contains('.') { mant.trim_end_matches('0').trim_end_matches('.') } else { mant };
        let e: i32 = e.parse().unwrap_or(0);
        return format!("{mant}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
    }
    let s = format!("{:.*}", (5 - exp).max(0) as usize, v);
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// Draws the panel when it is open.
pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    if !app.gamedev.tween.open {
        return;
    }
    let c = Colors::of(ctx);
    let PixmithApp { tabs, active, gamedev, .. } = app;
    let (st, tab) = (&mut gamedev.tween, &mut tabs[*active]);
    st.sync(tab);
    let mut open = true;
    let frame = egui::Frame::new()
        .fill(c.panel)
        .stroke(Stroke::new(1.0, c.separator))
        .corner_radius(egui::CornerRadius::same(radius::PANEL))
        .inner_margin(egui::Margin::ZERO)
        .shadow(egui::Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(80) });
    egui::Window::new("Tween")
        .id(egui::Id::new("tween_panel"))
        .open(&mut open)
        .frame(frame)
        .collapsible(false)
        .resizable(true)
        .min_size(vec2(400.0, 520.0))
        .default_size(vec2(440.0, 680.0))
        .default_pos(pos2((ctx.content_rect().right() - 440.0 - 16.0).max(0.0), 52.0))
        .show(ctx, |ui| panel(ui, st, tab));
    if !open {
        st.toggle();
        tab.changed();
    }
}

/// The panel's content: the layer header and the scrolling sections.
pub fn panel(ui: &mut Ui, st: &mut TweenState, tab: &mut DocTab) {
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    header(ui, tab);
    egui::ScrollArea::vertical().id_salt("tween_scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let frames = tab.doc.frame_count();
        widgets::inspector_section(
            ui,
            "tween.key",
            "Key",
            Some(st.sel_prop.title()),
            |_| {},
            |ui| key_section(ui, st, tab),
        );
        widgets::inspector_section(
            ui,
            "tween.dope",
            "Dope sheet",
            Some(&format!("{frames} frames")),
            |_| {},
            |ui| {
                ui.spacing_mut().item_spacing.y = space::MD;
                dope_sheet(ui, st, tab);
                copy_section(ui, st, tab);
            },
        );
        widgets::inspector_section(
            ui,
            "tween.pivot",
            "Pivot and options",
            None,
            |_| {},
            |ui| pivot_section(ui, st, tab),
        );
        widgets::inspector_section(ui, "tween.canvas", "Canvas", None, |_| {}, |ui| canvas_section(ui, st));
        widgets::inspector_section(ui, "tween.bake", "Bake", None, |_| {}, |ui| bake_section(ui, st, tab));
    });
}

/// `DSToolbar`: curve icon, the layer's name and "layer n · frame m".
fn header(ui: &mut Ui, tab: &DocTab) {
    let c = Colors::of(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), metric::BAR_HEIGHT), Sense::hover());
    let p = ui.painter();
    p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, c.separator_soft));
    let cy = rect.center().y;
    let mut x = rect.left() + space::MD;
    p.text(
        pos2(x + 6.0, cy),
        egui::Align2::CENTER_CENTER,
        icon::BEZIER_CURVE,
        egui::FontId::proportional(12.0),
        c.accent,
    );
    x += 12.0 + space::SM;
    let name = tab.doc.layer_name(tab.layer);
    let g = p.layout_no_wrap(name, ui_font(font::TITLE), c.text);
    let w = g.size().x.min(rect.right() - x - 80.0);
    form::paint_heavy(
        &p.with_clip_rect(Rect::from_min_max(pos2(x, rect.top()), pos2(x + w, rect.bottom()))),
        pos2(x, cy - g.size().y / 2.0),
        g,
        c.text,
        0.5,
    );
    x += w + space::SM;
    let sub = format!("layer {} \u{b7} frame {}", tab.layer + 1, tab.frame + 1);
    p.text(pos2(x, cy), egui::Align2::LEFT_CENTER, sub, ui_font(font::CAPTION), c.text_tertiary);
}

fn key_section(ui: &mut Ui, st: &mut TweenState, tab: &mut DocTab) {
    ui.spacing_mut().item_spacing.y = space::SM;
    form::form_row(ui, FORM_LABEL, "Property", |ui| {
        let titles: Vec<&str> = TweenProp::ALL.iter().map(|p| p.title()).collect();
        let selected = TweenProp::ALL.iter().position(|p| *p == st.sel_prop).unwrap_or(0);
        if let Some(i) = widgets::ds_popup(ui, "prop", selected, &titles, None, true, "Property to key") {
            st.sel_prop = TweenProp::ALL[i];
            st.sync(tab);
        }
    });
    form::form_row(ui, FORM_LABEL, "Value", |ui| {
        widgets::ds_text_field(ui, "value", &mut st.value_text, Some(96.0));
    });
    form::form_row(ui, FORM_LABEL, "Ease to next", |ui| {
        let titles: Vec<&str> = TweenEase::ALL.iter().map(|e| e.title()).collect();
        let selected = TweenEase::ALL.iter().position(|e| *e == st.ease).unwrap_or(0);
        if let Some(i) =
            widgets::ds_popup(ui, "ease", selected, &titles, None, true, "How the value moves to the next key")
        {
            st.choose_ease(tab, TweenEase::ALL[i]);
        }
    });
    if st.ease == TweenEase::Bezier {
        ui.add_space(space::XS);
        let commit = bezier_editor(ui, &mut st.bezier, &mut st.bezier_drag);
        if commit {
            st.commit_bezier(tab);
        }
        ui.add_space(space::XS);
    }
    ui.add_space(space::XS);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        if widgets::primary_button(ui, "Set Key at Current Frame", true).clicked() {
            st.set_key(tab);
        }
        let any = !tab.doc.tween_keys(tab.layer, st.sel_prop).is_empty();
        ui.add_enabled_ui(any, |ui| {
            if widgets::secondary_button(ui, "Delete Key", false).clicked() {
                st.delete_key(tab);
            }
        });
    });
}

/// The Bezier curve editor: a curve box with two draggable handles, a Presets menu and the control values.
/// Returns true when a drag or a preset changed the curve and it should be written to the key.
pub fn bezier_editor(ui: &mut Ui, p: &mut [f64; 4], dragging: &mut Option<usize>) -> bool {
    let c = Colors::of(ui.ctx());
    let mut commit = false;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = space::MD;
        let (rect, _) = ui.allocate_exact_size(vec2(CURVE_SIZE, CURVE_SIZE), Sense::hover());
        let to_view = |x: f64, y: f64| -> Pos2 {
            pos2(
                rect.left() + x as f32 * CURVE_SIZE,
                rect.top() + CURVE_SIZE * (1.0 - ((y - Y_LO) / (Y_HI - Y_LO)) as f32),
            )
        };
        // Pointer handling: grab the nearer handle on press, follow the pointer, commit on release.
        let (pos, down, pressed) =
            ui.input(|i| (i.pointer.interact_pos(), i.pointer.primary_down(), i.pointer.primary_pressed()));
        if let (Some(pos), true, None) = (pos, pressed, *dragging)
            && rect.contains(pos)
        {
            let (a, b) = (to_view(p[0], p[1]), to_view(p[2], p[3]));
            *dragging = Some(if a.distance(pos) <= b.distance(pos) { 0 } else { 1 });
        }
        if let (Some(h), Some(pos), true) = (*dragging, pos, down) {
            let x = (((pos.x - rect.left()) / CURVE_SIZE) as f64).clamp(0.0, 1.0);
            let y = Y_LO + (1.0 - ((pos.y - rect.top()) / CURVE_SIZE) as f64) * (Y_HI - Y_LO);
            p[h * 2] = x;
            p[h * 2 + 1] = y.clamp(Y_LO, Y_HI);
        }
        if dragging.is_some() && !down {
            *dragging = None;
            commit = true;
        }
        draw_curve(ui, rect, p, &to_view, &c);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = space::SM;
            presets_menu(ui, p, &mut commit);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let mono = egui::FontId::monospace(font::CAPTION);
                ui.label(
                    egui::RichText::new(format!("{:.2}   {:.2}", p[0], p[1]))
                        .font(mono.clone())
                        .color(c.text_secondary),
                );
                ui.label(egui::RichText::new(format!("{:.2}   {:.2}", p[2], p[3])).font(mono).color(c.text_secondary));
            });
            widgets::note(ui, "Drag the two handles to shape the ease.");
        });
    });
    commit
}

fn presets_menu(ui: &mut Ui, p: &mut [f64; 4], commit: &mut bool) {
    let c = Colors::of(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(112.0, metric::CONTROL), Sense::click());
    let fill = if response.hovered() || response.is_pointer_button_down_on() { c.hover } else { c.field };
    ui.painter().rect_filled(rect, radius::FIELD, fill);
    ui.painter().text(
        pos2(rect.left() + space::SM, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "Presets",
        ui_font(font::BODY),
        c.text,
    );
    ui.painter().text(
        pos2(rect.right() - space::SM, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        icon::CARET_DOWN,
        egui::FontId::proportional(9.0),
        c.text_tertiary,
    );
    let response = response.on_hover_text("Curve presets");
    egui::Popup::menu(&response).show(|ui| {
        for (name, v) in PRESETS {
            if ui.button(name).clicked() {
                *p = v;
                *commit = true;
                ui.close();
            }
        }
    });
}

fn draw_curve(ui: &Ui, rect: Rect, p: &[f64; 4], to_view: &dyn Fn(f64, f64) -> Pos2, c: &Colors) {
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, radius::FIELD, c.recessed);
    let (top, bottom) = (to_view(0.0, 1.0).y, to_view(0.0, 0.0).y);
    painter.rect_stroke(
        Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), bottom)),
        0.0,
        Stroke::new(1.0, c.separator),
        StrokeKind::Middle,
    );
    painter.extend(Shape::dashed_line(
        &[to_view(0.0, 0.0), to_view(1.0, 1.0)],
        Stroke::new(1.0, c.text_tertiary.gamma_multiply(0.7)),
        3.0,
        3.0,
    ));
    let (a, b) = (to_view(p[0], p[1]), to_view(p[2], p[3]));
    let line = Stroke::new(1.0, c.text_secondary);
    painter.line_segment([to_view(0.0, 0.0), a], line);
    painter.line_segment([to_view(1.0, 1.0), b], line);
    let mut pts = vec![to_view(0.0, 0.0)];
    for i in 1..=48 {
        let u = i as f64 / 48.0;
        let m = 1.0 - u;
        let x = 3.0 * p[0] * u * m * m + 3.0 * p[2] * u * u * m + u * u * u;
        let y = 3.0 * p[1] * u * m * m + 3.0 * p[3] * u * u * m + u * u * u;
        pts.push(to_view(x, y));
    }
    painter.add(Shape::line(pts, Stroke::new(2.0, c.accent)));
    for pt in [a, b] {
        painter.circle(pt, 5.0, Color32::WHITE, Stroke::new(2.0, c.accent));
    }
}

/// The dope sheet: frame numbers, then one row per property with its keys as diamonds.
fn dope_sheet(ui: &mut Ui, st: &mut TweenState, tab: &mut DocTab) {
    let c = Colors::of(ui.ctx());
    let frames = tab.doc.frame_count();
    let range = tab.doc.tween_range(tab.layer);
    egui::ScrollArea::horizontal().id_salt("dope_scroll").auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
        // Frame numbers.
        let width = LABEL_WIDTH + frames as f32 * CELL;
        let (header, _) = ui.allocate_exact_size(vec2(width, 18.0), Sense::hover());
        for f in 0..frames {
            let cell = Rect::from_min_size(
                pos2(header.left() + LABEL_WIDTH + f as f32 * CELL, header.top()),
                vec2(CELL, 18.0),
            );
            let current = f == tab.frame;
            let color = if current { c.text } else { c.text_tertiary };
            let g = ui.painter().layout_no_wrap((f + 1).to_string(), egui::FontId::proportional(10.0), color);
            form::paint_heavy(ui.painter(), cell.center() - g.size() / 2.0, g, color, if current { 0.3 } else { 0.0 });
            if ui.interact(cell, ui.id().with(("dope_frame", f)), Sense::click()).clicked() {
                tab.frame = f;
            }
        }
        for prop in TweenProp::ALL {
            dope_row(ui, st, tab, prop, frames, range, width);
        }
        ui.add_space(space::XS);
    });
}

fn dope_row(
    ui: &mut Ui,
    st: &mut TweenState,
    tab: &mut DocTab,
    prop: TweenProp,
    frames: u32,
    range: Option<(u32, u32)>,
    width: f32,
) {
    let c = Colors::of(ui.ctx());
    let keys = tab.doc.tween_keys(tab.layer, prop);
    let row_selected = prop == st.sel_prop;
    let (row, _) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
    let track = Rect::from_min_size(pos2(row.left() + LABEL_WIDTH, row.top()), vec2(frames as f32 * CELL, ROW_HEIGHT));
    let p = ui.painter();
    let label_color = if row_selected { c.text } else { c.text_tertiary };
    let label = p.layout_no_wrap(prop.short().to_owned(), ui_font(font::CAPTION), label_color);
    form::paint_heavy(p, pos2(row.left(), row.center().y - label.size().y / 2.0), label, label_color, 0.3);
    p.rect_filled(track, radius::CHIP, if row_selected { c.selected } else { c.field });
    let col = Rect::from_min_size(pos2(track.left() + tab.frame as f32 * CELL, track.top()), vec2(CELL, ROW_HEIGHT));
    p.rect_filled(col.intersect(track), 0.0, c.accent.gamma_multiply(0.22));
    if let Some((lo, hi)) = range {
        let capsule = Rect::from_min_size(
            pos2(track.left() + lo as f32 * CELL + 3.0, track.center().y - 1.0),
            vec2(((hi - lo + 1) as f32 * CELL - 6.0).max(0.0), 2.0),
        );
        p.rect_filled(capsule, 1.0, c.text_tertiary.gamma_multiply(0.45));
    }
    for k in &keys {
        let selected = st.sel_prop == prop && st.sel_frame == Some(k.frame);
        let shift = st.dope_drag.filter(|d| d.prop == prop && d.frame == k.frame).map_or(0.0, |d| d.dx);
        let cell = Rect::from_min_size(pos2(track.left() + k.frame as f32 * CELL, track.top()), vec2(CELL, ROW_HEIGHT));
        let response = ui.interact(cell, ui.id().with(("dope_key", prop.code(), k.frame)), Sense::click_and_drag());
        if response.dragged() {
            st.dope_drag = Some(DopeDrag {
                prop,
                frame: k.frame,
                dx: response.drag_delta().x
                    + st.dope_drag.filter(|d| d.frame == k.frame && d.prop == prop).map_or(0.0, |d| d.dx),
            });
        }
        if response.drag_stopped() {
            let dx = st.dope_drag.take().map_or(0.0, |d| d.dx);
            let to = (k.frame as i64 + (dx / CELL).round() as i64).clamp(0, frames as i64 - 1) as u32;
            st.move_key(tab, prop, k.frame, to);
        } else if response.clicked() {
            st.sel_prop = prop;
            st.sel_frame = Some(k.frame);
            tab.frame = k.frame;
        }
        let size = if selected { 12.0 } else { 10.0 };
        let center = cell.center() + vec2(shift, 0.0);
        let r = size / 2.0 + 0.5;
        ui.painter().add(Shape::convex_polygon(
            vec![
                pos2(center.x, center.y - r),
                pos2(center.x + r, center.y),
                pos2(center.x, center.y + r),
                pos2(center.x - r, center.y),
            ],
            if selected { c.text } else { c.accent },
            Stroke::NONE,
        ));
    }
}

fn copy_section(ui: &mut Ui, st: &mut TweenState, tab: &mut DocTab) {
    let c = Colors::of(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM - 2.0;
        ui.label(egui::RichText::new("Keys").font(ui_font(font::BODY)).color(c.text_secondary));
        widgets::ds_text_field(ui, "copy_from", &mut st.copy_from, Some(46.0));
        ui.label(egui::RichText::new("to").font(ui_font(font::BODY)).color(c.text_tertiary));
        widgets::ds_text_field(ui, "copy_to", &mut st.copy_to, Some(46.0));
        if form::bar_button(ui, "Copy").on_hover_text("Copy the keys between these frames").clicked() {
            st.copy_keys(tab);
        }
        if form::bar_button(ui, "Paste at Current Frame").clicked() {
            st.paste_keys(tab);
        }
    });
}

fn pivot_section(ui: &mut Ui, st: &mut TweenState, tab: &mut DocTab) {
    ui.spacing_mut().item_spacing.y = space::SM;
    form::form_row(ui, FORM_LABEL, "Pivot", |ui| {
        widgets::ds_text_field(ui, "pivot_x", &mut st.pivot_x, Some(60.0));
        widgets::ds_text_field(ui, "pivot_y", &mut st.pivot_y, Some(60.0));
        if form::bar_button(ui, "Apply").clicked() {
            st.apply_pivot(tab);
        }
        if form::bar_button(ui, "Center").clicked() {
            st.center_pivot(tab);
        }
    });
    parent_row(ui, tab);
    for (flag, title) in [
        (0u32, "RotSprite rotation (off = nearest, faster)"),
        (1, "Snap scale to integer"),
        (2, "Ignore opacity track"),
    ] {
        let mut on = tab.doc.tween_flag(tab.layer, flag);
        if form::check(ui, &format!("flag{flag}"), &mut on, title) && tab.doc.tween_set_flag(tab.layer, flag, on) {
            tab.changed();
        }
    }
    if let Some(w) = tab.doc.tween_warning(tab.layer) {
        let c = Colors::of(ui.ctx());
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = space::XS + 2.0;
            ui.label(egui::RichText::new(icon::WARNING).size(11.0).color(ORANGE));
            ui.add(
                egui::Label::new(egui::RichText::new(w).font(ui_font(font::CAPTION)).color(c.text_secondary)).wrap(),
            );
        });
    }
}

/// "Parent": the layer this one follows (thigh -> shin -> boot). The list leaves out the layer itself, its
/// descendants and groups it cannot be linked with, so no cycle can be picked.
fn parent_row(ui: &mut Ui, tab: &mut DocTab) {
    let layer = tab.layer;
    let current = tab.doc.tween_parent(layer);
    let mut ids: Vec<Option<u32>> = vec![None];
    let mut titles: Vec<String> = vec!["None".to_string()];
    for l in (0..tab.doc.layer_count()).rev() {
        if current == Some(l) || tab.doc.tween_can_parent(layer, l) {
            ids.push(Some(l));
            titles.push(format!("{} \u{b7} {}", l + 1, tab.doc.layer_name(l)));
        }
    }
    let selected = ids.iter().position(|p| *p == current).unwrap_or(0);
    let names: Vec<&str> = titles.iter().map(String::as_str).collect();
    let mut chosen = None;
    form::form_row(ui, FORM_LABEL, "Parent", |ui| {
        chosen = widgets::ds_popup(ui, "parent", selected, &names, None, true, "Follow another layer's motion");
    });
    if let Some(i) = chosen
        && ids[i] != current
        && tab.doc.tween_set_parent(layer, ids[i])
    {
        tab.changed();
    }
    widgets::note(ui, "The pivot and keys are relative to the parent. Bake All keeps parents and children together.");
}

fn canvas_section(ui: &mut Ui, st: &mut TweenState) {
    ui.spacing_mut().item_spacing.y = space::SM;
    form::check(ui, "edit", &mut st.edit_on_canvas, "Edit on canvas");
    widgets::note(ui, "Drag = move, corner = scale, top handle = rotate, orange dot = pivot.");
    let on = st.edit_on_canvas;
    ui.add_enabled_ui(on, |ui| {
        form::check(ui, "autokey", &mut st.auto_key, "Auto-key (create keys while dragging)");
    });
    widgets::note(ui, "Motion path: dots are frames, large dots are keys.");
}

fn bake_section(ui: &mut Ui, st: &mut TweenState, tab: &mut DocTab) {
    let c = Colors::of(ui.ctx());
    ui.spacing_mut().item_spacing.y = space::SM;
    ui.horizontal(|ui| {
        ui.push_id("bake_mode", |ui| {
            let options = [
                (SegGlyph::Text("All keys"), "All keys"),
                (SegGlyph::Text("Selected frames"), "Selected frames"),
                (SegGlyph::Text("Current frame"), "Current frame"),
            ];
            if let Some(i) = widgets::bar_segmented(ui, st.bake_mode, &options) {
                st.bake_mode = i;
            }
        });
    });
    if st.bake_mode == 1 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::SM - 2.0;
            ui.label(egui::RichText::new("Frames").font(ui_font(font::BODY)).color(c.text_secondary));
            widgets::ds_text_field(ui, "bake_from", &mut st.bake_from, Some(52.0));
            ui.label(egui::RichText::new("to").font(ui_font(font::BODY)).color(c.text_tertiary));
            widgets::ds_text_field(ui, "bake_to", &mut st.bake_to, Some(52.0));
        });
    }
    form::check(ui, "bake_new", &mut st.bake_new_layer, "Into a new layer (original is hidden)");
    form::check(ui, "bake_keep", &mut st.bake_keep, "Keep source and keys (re-bakeable)");
    ui.add_space(space::XS);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        if widgets::primary_button(ui, "Bake", true).clicked() {
            let keep = st.bake_keep;
            st.bake(tab, keep);
        }
        if widgets::secondary_button(ui, "Bake & Keep Source", false).clicked() {
            st.bake(tab, true);
        }
        if widgets::secondary_button(ui, "Bake All Layers", false).clicked() {
            st.bake_all(tab);
        }
    });
    widgets::note(
        ui,
        "Source = layer cel (group: its composite) at the first key frame. Groups bake into a new layer. Partial ranges keep the keys.",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Doc;

    fn tab_with_dot() -> DocTab {
        let mut doc = Doc::new(32, 32);
        doc.begin_edit("Shape");
        doc.draw_shape(crate::ui::canvas::ShapeKind::Rect, 0, 0, (4, 4), (11, 11), 0xFF0000FF, 1, 8, false);
        doc.stroke_end();
        for _ in 0..7 {
            let last = doc.frame_count() - 1;
            doc.add_frame(last, true);
        }
        doc.touch();
        let mut tab = DocTab::new(doc);
        tab.layer = 0;
        tab
    }

    #[test]
    fn g_format_matches_printf() {
        assert_eq!(fmt_g(0.0), "0");
        assert_eq!(fmt_g(21.6383), "21.6383");
        assert_eq!(fmt_g(21.638329), "21.6383");
        assert_eq!(fmt_g(17.5), "17.5");
        assert_eq!(fmt_g(-3.0), "-3");
        assert_eq!(fmt_g(100000.0), "100000");
        assert_eq!(fmt_g(1234567.0), "1.23457e+06");
        assert_eq!(fmt_g(0.0001), "0.0001");
        assert_eq!(fmt_g(0.00001), "1e-05");
        assert_eq!(fmt_g(0.5), "0.5");
        assert_eq!(fmt_g(999999.9), "1e+06");
    }

    #[test]
    fn set_key_move_and_delete_are_labeled_undo_steps() {
        let mut tab = tab_with_dot();
        let mut st = TweenState::default();
        st.sync(&tab);
        st.value_text = "5".into();
        tab.frame = 0;
        st.set_key(&mut tab);
        tab.frame = 4;
        st.value_text = "25".into();
        st.set_key(&mut tab);
        let keys = tab.doc.tween_keys(0, TweenProp::PosX);
        assert_eq!(keys.iter().map(|k| (k.frame, k.value)).collect::<Vec<_>>(), [(0, 5.0), (4, 25.0)]);
        assert_eq!(tab.doc.tween_range(0), Some((0, 4)));
        assert_eq!(tab.doc.tween_value(0, TweenProp::PosX, 2), 15.0, "linear in between");
        st.move_key(&mut tab, TweenProp::PosX, 4, 6);
        assert_eq!(tab.doc.tween_keys(0, TweenProp::PosX)[1].frame, 6);
        assert_eq!((st.sel_prop, st.sel_frame), (TweenProp::PosX, Some(6)));
        st.sel_frame = Some(6);
        st.delete_key(&mut tab);
        assert_eq!(tab.doc.tween_keys(0, TweenProp::PosX).len(), 1);
        assert!(tab.doc.undo(), "removing the key is one step");
        assert_eq!(tab.doc.tween_keys(0, TweenProp::PosX).len(), 2);
    }

    #[test]
    fn ease_changes_apply_to_the_key_at_the_current_frame() {
        let mut tab = tab_with_dot();
        let mut st = TweenState::default();
        tab.frame = 1;
        st.value_text = "0".into();
        st.set_key(&mut tab);
        st.choose_ease(&mut tab, TweenEase::Hold);
        assert_eq!(tab.doc.tween_keys(0, TweenProp::PosX)[0].ease, TweenEase::Hold);
        st.choose_ease(&mut tab, TweenEase::Bezier);
        st.bezier = [0.1, 0.2, 0.3, 0.4];
        st.commit_bezier(&mut tab);
        let k = tab.doc.tween_keys(0, TweenProp::PosX)[0];
        assert_eq!(k.ease, TweenEase::Bezier);
        assert_eq!(k.bezier, [0.1, 0.2, 0.3, 0.4]);
        // Moving to another frame loads that frame's value.
        tab.frame = 3;
        st.sync(&tab);
        assert_eq!(st.value_text, "0");
        tab.frame = 1;
        st.sync(&tab);
        assert_eq!(st.ease, TweenEase::Bezier);
        assert_eq!(st.bezier, [0.1f32 as f64, 0.2f32 as f64, 0.3f32 as f64, 0.4f32 as f64]);
    }

    #[test]
    fn copy_paste_options_pivot_and_bake() {
        let mut tab = tab_with_dot();
        let mut st = TweenState::default();
        tab.frame = 0;
        st.value_text = "0".into();
        st.set_key(&mut tab);
        tab.frame = 2;
        st.value_text = "8".into();
        st.set_key(&mut tab);
        st.copy_from = "1".into();
        st.copy_to = "3".into();
        assert_eq!(st.copy_keys(&tab), 2);
        tab.frame = 4;
        st.paste_keys(&mut tab);
        let frames: Vec<u32> = tab.doc.tween_keys(0, TweenProp::PosX).iter().map(|k| k.frame).collect();
        assert_eq!(frames, [0, 2, 4, 6]);

        // Options and pivot.
        assert!(tab.doc.tween_flag(0, 0), "RotSprite is on by default");
        assert!(tab.doc.tween_set_flag(0, 0, false));
        assert!(!tab.doc.tween_flag(0, 0));
        st.pivot_x = "3".into();
        st.pivot_y = "4.5".into();
        st.apply_pivot(&mut tab);
        assert_eq!(tab.doc.tween_pivot(0), Some((3.0, 4.5)));
        st.center_pivot(&mut tab);
        assert_eq!(st.pivot_x, "8");

        // Bake the whole tween: the keys go and the cels hold the result (one undo step).
        st.bake_mode = 0;
        let before = tab.doc.history_counts().0;
        st.bake(&mut tab, false);
        assert_eq!(tab.doc.history_counts().0, before + 1);
        assert!(tab.doc.tween_keys(0, TweenProp::PosX).is_empty() || st.bake_keep);
    }

    #[test]
    fn dope_sheet_drag_clamps_to_the_frames() {
        let mut tab = tab_with_dot();
        let mut st = TweenState::default();
        tab.frame = 5;
        st.value_text = "1".into();
        st.set_key(&mut tab);
        st.move_key(&mut tab, TweenProp::PosX, 5, 7);
        assert_eq!(tab.doc.tween_keys(0, TweenProp::PosX)[0].frame, 7);
    }

    #[test]
    fn toggling_the_panel_resets_edit_on_canvas() {
        let mut st = TweenState::default();
        st.toggle();
        assert!(st.open);
        st.edit_on_canvas = true;
        st.toggle();
        assert!(!st.open && !st.edit_on_canvas);
    }
}
