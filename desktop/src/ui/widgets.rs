//! Reusable controls, one-to-one with the `DS*` views in `DesignSystem.swift`.

use crate::theme::{Colors, radius};
use eframe::egui::{self, Response, Sense, Ui};

/// A square, icon-only tool button with the selected fill when `active` (`ToolButton`).
pub fn tool_button(ui: &mut Ui, icon: &str, active: bool, size: f32) -> Response {
    let c = Colors::of(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), Sense::click());
    let fill = if active {
        c.selected
    } else if response.is_pointer_button_down_on() {
        c.pressed
    } else if response.hovered() {
        c.hover
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, radius::CONTROL, fill);
    let fg = if active { c.accent } else { c.text };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(16.0), fg);
    response
}

/// An icon button without a selected state (`IconButton`).
pub fn icon_button(ui: &mut Ui, icon: &str, size: f32) -> Response {
    tool_button(ui, icon, false, size)
}

/// The short horizontal rule between tool groups in the rail (`DSRailDivider`).
pub fn rail_divider(ui: &mut Ui) {
    let c = Colors::of(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 13.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, c.separator));
}

// ---- Bar controls (`DSChip`, `DSNumberField`, `DSSegmented`, `DSPopup`, `HintLabel`, `DSOverlayChip`) ----

use crate::theme::{self, font, metric, space};

/// The semibold family of `theme::strong_font`, or the regular one while the fonts are still being installed
/// (they take effect one frame after `theme::install`).
pub fn strong(ctx: &egui::Context, size: f32) -> egui::FontId {
    let family = egui::FontFamily::Name("strong".into());
    if ctx.fonts(|f| f.families().contains(&family)) { egui::FontId::new(size, family) } else { theme::ui_font(size) }
}

/// A toggle icon button for tool options (`DSChip` without a title): 28 pt, accent-tinted when on.
pub fn bar_chip(ui: &mut Ui, icon: &str, on: bool, help: &str) -> Response {
    bar_chip_with(ui, on, help, |p, rect, fg| {
        p.text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(14.0), fg);
    })
}

/// `bar_chip` with a custom glyph, drawn into the button's rect in the state's foreground color.
pub fn bar_chip_with(
    ui: &mut Ui,
    on: bool,
    help: &str,
    glyph: impl FnOnce(&egui::Painter, egui::Rect, egui::Color32),
) -> Response {
    let c = Colors::of(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(metric::ICON_BUTTON, metric::ICON_BUTTON), Sense::click());
    let hover = response.hovered();
    let down = response.is_pointer_button_down_on();
    let fill = if on {
        if hover || down { c.selected_hover } else { c.selected }
    } else if down {
        c.pressed
    } else if hover {
        c.hover
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, radius::CONTROL, fill);
    let fg = if on {
        c.accent
    } else if hover {
        c.text
    } else {
        c.text_secondary
    };
    glyph(ui.painter(), rect, fg);
    response.on_hover_text(help)
}

/// The hairline between option groups in a bar (`DSGroupDivider`): 20 pt tall with 4 pt margins.
pub fn group_divider(ui: &mut Ui) {
    let c = Colors::of(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0 + 2.0 * space::XS, 20.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), egui::Stroke::new(1.0, c.separator));
}

/// A shortcut shown as a little keycap (`KeyHint`).
pub fn key_hint(ui: &mut Ui, text: &str) {
    let c = Colors::of(ui.ctx());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), strong(ui.ctx(), 10.0), c.text_secondary);
    let size = egui::vec2((galley.size().x + 10.0).max(16.0), 16.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, radius::CHIP, c.field);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, c.text_secondary);
}

/// Keycaps followed by a short description (`HintLabel`).
pub fn hint_label(ui: &mut Ui, keys: &[&str], text: &str) {
    let c = Colors::of(ui.ctx());
    ui.spacing_mut().item_spacing.x = space::XS;
    for k in keys {
        key_hint(ui, k);
    }
    ui.label(egui::RichText::new(text).font(theme::ui_font(font::CAPTION)).color(c.text_secondary));
}

/// A number field with a label and unit (`DSNumberField`): drag the label to scrub (Shift for 10x), or type.
/// Returns true when `value` changed.
pub fn bar_number_field(
    ui: &mut Ui,
    label: &str,
    value: &mut u32,
    range: (u32, u32),
    unit: &str,
    width: f32,
    help: &str,
) -> bool {
    let c = Colors::of(ui.ctx());
    let id = ui.make_persistent_id(("bar_number_field", label));
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, metric::CONTROL), Sense::hover());
    let label_galley = ui.painter().layout_no_wrap(label.to_owned(), strong(ui.ctx(), font::CAPTION), c.text_tertiary);
    let unit_galley = ui.painter().layout_no_wrap(unit.to_owned(), theme::ui_font(font::CAPTION), c.text_tertiary);
    let pad = space::SM;
    let label_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + pad, rect.top()),
        egui::vec2(label_galley.size().x, rect.height()),
    );
    let scrub = ui.interact(label_rect.expand2(egui::vec2(2.0, 0.0)), id.with("scrub"), Sense::drag());
    if scrub.hovered() || scrub.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let mut changed = false;
    let start_id = id.with("start");
    if scrub.drag_started() {
        ui.data_mut(|d| d.insert_temp(start_id, *value));
    }
    if scrub.dragged() {
        let start: u32 = ui.data(|d| d.get_temp(start_id)).unwrap_or(*value);
        let (origin, pos, shift) =
            ui.input(|i| (i.pointer.press_origin(), i.pointer.interact_pos(), i.modifiers.shift));
        if let (Some(o), Some(p)) = (origin, pos) {
            let dx = (p.x - o.x) * if shift { 10.0 } else { 1.0 };
            let v = (start as f32 + dx).round().clamp(range.0 as f32, range.1 as f32) as u32;
            if v != *value {
                *value = v;
                changed = true;
            }
        }
    }
    let scrubbing = scrub.dragged();
    let focused = ui.memory(|m| m.has_focus(id));
    let (unit_w, gap) = if unit.is_empty() { (0.0, 0.0) } else { (unit_galley.size().x, space::XS) };
    let edit_rect = egui::Rect::from_min_max(
        egui::pos2(label_rect.right() + space::XS, rect.top()),
        egui::pos2(rect.right() - pad - unit_w - gap, rect.bottom()),
    );
    // Chrome.
    ui.painter().rect_filled(rect, radius::FIELD, c.field);
    if focused {
        ui.painter().rect_stroke(rect, radius::FIELD, egui::Stroke::new(1.5, c.accent_line), egui::StrokeKind::Inside);
    }
    let label_color = if scrubbing { c.accent } else { c.text_tertiary };
    let label_galley = ui.painter().layout_no_wrap(label.to_owned(), strong(ui.ctx(), font::CAPTION), label_color);
    ui.painter().galley(
        egui::pos2(label_rect.left(), rect.center().y - label_galley.size().y / 2.0),
        label_galley,
        label_color,
    );
    if !unit.is_empty() {
        ui.painter().galley(
            egui::pos2(rect.right() - pad - unit_w, rect.center().y - unit_galley.size().y / 2.0),
            unit_galley,
            c.text_tertiary,
        );
    }
    // The editable value.
    let buf_id = id.with("buf");
    let mut text: String =
        if focused { ui.data(|d| d.get_temp(buf_id)).unwrap_or_else(|| value.to_string()) } else { value.to_string() };
    // A child ui, so the surrounding layout continues after the whole field.
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect));
    let edit = child.add_sized(
        edit_rect.size(),
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(theme::ui_font(font::BODY))
            .text_color(c.text)
            .horizontal_align(egui::Align::RIGHT)
            .vertical_align(egui::Align::Center),
    );
    if edit.has_focus() {
        ui.data_mut(|d| d.insert_temp(buf_id, text.clone()));
    }
    if edit.lost_focus() {
        if let Ok(v) = text.trim().parse::<f64>() {
            let v = v.round().clamp(range.0 as f64, range.1 as f64) as u32;
            if v != *value {
                *value = v;
                changed = true;
            }
        }
        ui.data_mut(|d| d.remove_temp::<String>(buf_id));
    }
    ui.interact(rect, id.with("tip"), Sense::hover()).on_hover_text(help);
    changed
}

/// What a segment of `bar_segmented` shows.
#[derive(Clone, Copy)]
pub enum SegGlyph<'a> {
    Icon(&'a str),
    Text(&'a str),
    /// A solid square (`square.fill`).
    Square,
    /// A solid circle (`circle.fill`).
    Circle,
}

/// A segmented control (`DSSegmented`): a recessed track with a raised pill. Returns the newly picked index.
pub fn bar_segmented(ui: &mut Ui, selected: usize, options: &[(SegGlyph, &str)]) -> Option<usize> {
    let c = Colors::of(ui.ctx());
    let height = metric::CONTROL - 4.0;
    let widths: Vec<f32> = options
        .iter()
        .map(|(g, _)| match g {
            SegGlyph::Text(t) => {
                ui.painter().layout_no_wrap((*t).to_owned(), strong(ui.ctx(), font::CAPTION), c.text).size().x
                    + 2.0 * space::SM
            }
            _ => 22.0,
        })
        .collect();
    let total = widths.iter().sum::<f32>() + 2.0 * (options.len().saturating_sub(1)) as f32 + 4.0;
    let (track, _) = ui.allocate_exact_size(egui::vec2(total, metric::CONTROL), Sense::hover());
    ui.painter().rect_filled(track, radius::FIELD, c.field);
    let mut x = track.left() + 2.0;
    let mut picked = None;
    for (i, ((glyph, help), w)) in options.iter().zip(&widths).enumerate() {
        let rect = egui::Rect::from_min_size(egui::pos2(x, track.top() + 2.0), egui::vec2(*w, height));
        let response = ui.interact(rect, ui.id().with(("seg", i)), Sense::click());
        let on = i == selected;
        if on {
            ui.painter().rect_filled(rect, radius::CHIP, c.hover);
        }
        let fg = if on { c.text } else { c.text_tertiary };
        let p = ui.painter();
        match glyph {
            SegGlyph::Icon(s) => {
                p.text(rect.center(), egui::Align2::CENTER_CENTER, *s, egui::FontId::proportional(11.0), fg);
            }
            SegGlyph::Text(s) => {
                p.text(rect.center(), egui::Align2::CENTER_CENTER, *s, strong(ui.ctx(), font::CAPTION), fg);
            }
            SegGlyph::Square => {
                p.rect_filled(egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(9.0)), 1.0, fg);
            }
            SegGlyph::Circle => {
                p.circle_filled(rect.center(), 5.0, fg);
            }
        }
        if response.on_hover_text(*help).clicked() {
            picked = Some(i);
        }
        x += w + 2.0;
    }
    picked
}

/// A dropdown in the field style (`DSPopup`). Returns true when `selected` changed.
pub fn bar_popup(ui: &mut Ui, selected: &mut usize, options: &[&str], width: f32, help: &str) -> bool {
    let c = Colors::of(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, metric::CONTROL), Sense::click());
    ui.painter().rect_filled(rect, radius::FIELD, c.field);
    ui.painter().text(
        egui::pos2(rect.left() + space::SM, rect.center().y),
        egui::Align2::LEFT_CENTER,
        options.get(*selected).copied().unwrap_or(""),
        theme::ui_font(font::BODY),
        c.text,
    );
    ui.painter().text(
        egui::pos2(rect.right() - space::SM, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        egui_phosphor::regular::CARET_DOWN,
        theme::ui_font(9.0),
        c.text_tertiary,
    );
    let mut changed = false;
    egui::Popup::menu(&response).show(|ui| {
        ui.set_min_width(width - 16.0);
        for (i, name) in options.iter().enumerate() {
            let mark = if i == *selected { egui_phosphor::regular::CHECK } else { "  " };
            if ui.button(format!("{mark}  {name}")).clicked() {
                if i != *selected {
                    *selected = i;
                    changed = true;
                }
                ui.close();
            }
        }
    });
    response.on_hover_text(help);
    changed
}

/// A pill toggle with a title ("Contiguous"): `DSChip` with text.
pub fn bar_text_chip(ui: &mut Ui, title: &str, on: bool, help: &str) -> Response {
    let c = Colors::of(ui.ctx());
    let hover_color = if on { c.accent } else { c.text };
    let galley = ui.painter().layout_no_wrap(title.to_owned(), strong(ui.ctx(), font::CAPTION), hover_color);
    let size = egui::vec2(galley.size().x + 2.0 * (space::SM + 2.0), metric::CONTROL + 2.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let (hover, down) = (response.hovered(), response.is_pointer_button_down_on());
    let fill = if on {
        if hover || down { c.selected_hover } else { c.selected }
    } else if down {
        c.pressed
    } else if hover {
        c.hover
    } else {
        c.field
    };
    ui.painter().rect_filled(rect, radius::CONTROL, fill);
    let fg = if on {
        c.accent
    } else if hover {
        c.text
    } else {
        c.text_secondary
    };
    let galley = ui.painter().layout_no_wrap(title.to_owned(), strong(ui.ctx(), font::CAPTION), fg);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    response.on_hover_text(help)
}

/// A quiet status chip over the canvas (`DSOverlayChip`), drawn at `left_top`. Returns its width.
pub fn overlay_chip(painter: &egui::Painter, c: &Colors, left_top: egui::Pos2, icon: Option<&str>, text: &str) -> f32 {
    let color = if icon.is_none() { c.text_tertiary } else { c.text };
    let galley = painter.layout_no_wrap(text.to_owned(), theme::ui_font(font::CAPTION), color);
    let icon_galley =
        icon.map(|i| painter.layout_no_wrap(i.to_owned(), egui::FontId::proportional(10.0), c.text_tertiary));
    let inner = galley.size().x + icon_galley.as_ref().map_or(0.0, |g| g.size().x + 6.0);
    let rect = egui::Rect::from_min_size(left_top, egui::vec2(inner + 2.0 * space::SM, 22.0));
    painter.rect_filled(rect, radius::FIELD, c.overlay);
    let mut x = rect.left() + space::SM;
    if let Some(g) = icon_galley {
        painter.galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g.clone(), c.text_tertiary);
        x += g.size().x + 6.0;
    }
    painter.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, color);
    rect.width()
}

// ---------------------------------------------------------------------------------------------------------------
// Inspector controls: `InspectorSection`, `DSNumberField`, `DSPopup`, `Swatch`, `DSChecker`, text fields, icon
// buttons and menus from `DesignSystem.swift`.
// ---------------------------------------------------------------------------------------------------------------

use crate::theme::ui_font;

/// Paints `galley` heavier than regular by drawing it twice, `weight` points apart. The system font on macOS is
/// one weight only, so this stands in for the medium and semibold weights of the Swift app.
fn paint_heavy(painter: &Painter, pos: Pos2, galley: std::sync::Arc<egui::Galley>, color: Color32, weight: f32) {
    painter.galley(pos, galley.clone(), color);
    painter.galley(pos + vec2(weight, 0.0), galley, color);
}

use eframe::egui::{
    Align, Align2, Color32, CursorIcon, Id, Layout, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, TextEdit,
    UiBuilder, pos2, vec2,
};
use std::hash::Hash;

/// A color from `0xRRGGBBAA` (straight alpha), like the core's colors.
pub fn rgba_color(v: u32) -> Color32 {
    Color32::from_rgba_unmultiplied((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Dims a color the way SwiftUI's `.disabled` does (35% opacity).
fn dimmed(enabled: bool, c: Color32) -> Color32 {
    if enabled { c } else { c.gamma_multiply(0.35) }
}

// MARK: Geometry

/// The outline of a rounded rectangle, clockwise, for clipping gradients and checkerboards to it.
pub fn rounded_rect_polygon(rect: Rect, r: f32) -> Vec<Pos2> {
    use std::f32::consts::PI;
    let r = r.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    if r <= 0.0 {
        return vec![rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    }
    let seg = 6;
    let corners = [
        (rect.min.x + r, rect.min.y + r, PI),
        (rect.max.x - r, rect.min.y + r, 1.5 * PI),
        (rect.max.x - r, rect.max.y - r, 0.0),
        (rect.min.x + r, rect.max.y - r, 0.5 * PI),
    ];
    let mut pts = Vec::with_capacity(4 * (seg + 1));
    for (cx, cy, a0) in corners {
        for k in 0..=seg {
            let a = a0 + (k as f32 / seg as f32) * 0.5 * PI;
            pts.push(pos2(cx + r * a.cos(), cy + r * a.sin()));
        }
    }
    pts
}

/// Where the segment `s`-`e` crosses the infinite line through `a` and `b`.
fn line_crossing(s: Pos2, e: Pos2, a: Pos2, b: Pos2) -> Pos2 {
    let (d1, d2) = (e - s, b - a);
    let denom = d1.x * d2.y - d1.y * d2.x;
    if denom.abs() < f32::EPSILON {
        return e;
    }
    let t = ((a.x - s.x) * d2.y - (a.y - s.y) * d2.x) / denom;
    s + d1 * t
}

/// Sutherland-Hodgman: `subject` clipped to the convex polygon `clip`.
pub fn clip_convex(subject: &[Pos2], clip: &[Pos2]) -> Vec<Pos2> {
    let n = clip.len();
    let area: f32 = (0..n).map(|i| clip[i].x * clip[(i + 1) % n].y - clip[(i + 1) % n].x * clip[i].y).sum();
    let sign = if area >= 0.0 { 1.0 } else { -1.0 };
    let inside = |a: Pos2, b: Pos2, p: Pos2| sign * ((b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)) >= 0.0;
    let mut out = subject.to_vec();
    for i in 0..n {
        let (a, b) = (clip[i], clip[(i + 1) % n]);
        let input = std::mem::take(&mut out);
        let Some(&last) = input.last() else { break };
        let mut s = last;
        for &e in &input {
            if inside(a, b, e) {
                if !inside(a, b, s) {
                    out.push(line_crossing(s, e, a, b));
                }
                out.push(e);
            } else if inside(a, b, s) {
                out.push(line_crossing(s, e, a, b));
            }
            s = e;
        }
    }
    out
}

fn quad(r: Rect) -> [Pos2; 4] {
    [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]
}

fn add_fan(mesh: &mut Mesh, pts: &[Pos2], color_at: &impl Fn(Pos2) -> Color32) {
    if pts.len() < 3 {
        return;
    }
    let base = mesh.vertices.len() as u32;
    for &p in pts {
        mesh.colored_vertex(p, color_at(p));
    }
    for i in 1..pts.len() as u32 - 1 {
        mesh.add_triangle(base, base + i, base + i + 1);
    }
}

/// True when `cell` lies in the part of `rect` that the corner rounding of radius `r` does not touch.
fn clear_of_corners(cell: Rect, rect: Rect, r: f32) -> bool {
    (cell.min.x >= rect.min.x + r && cell.max.x <= rect.max.x - r)
        || (cell.min.y >= rect.min.y + r && cell.max.y <= rect.max.y - r)
}

/// Fills `rect` (rounded by `radius`) with a gradient: `color_at` is evaluated at the vertices of an `nx` by `ny`
/// grid, which is exact for gradients that are linear in each cell.
pub fn draw_gradient(
    painter: &Painter,
    rect: Rect,
    radius: f32,
    nx: usize,
    ny: usize,
    color_at: impl Fn(Pos2) -> Color32,
) {
    let poly = rounded_rect_polygon(rect, radius);
    let mut mesh = Mesh::default();
    let (dx, dy) = (rect.width() / nx as f32, rect.height() / ny as f32);
    for j in 0..ny {
        for i in 0..nx {
            let cell = Rect::from_min_size(rect.min + vec2(i as f32 * dx, j as f32 * dy), vec2(dx, dy));
            if clear_of_corners(cell, rect, radius) {
                add_fan(&mut mesh, &quad(cell), &color_at);
            } else {
                add_fan(&mut mesh, &clip_convex(&quad(cell), &poly), &color_at);
            }
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// Two-tone checkerboard for showing transparency (`DSChecker`), clipped to a rounded rectangle.
pub fn draw_checker(painter: &Painter, rect: Rect, cell: f32, radius: f32) {
    painter.rect_filled(rect, radius, Color32::from_gray(235));
    let dark = Color32::from_gray(184);
    let poly = rounded_rect_polygon(rect, radius);
    let mut mesh = Mesh::default();
    let cols = (rect.width() / cell).ceil() as i32;
    let rows = (rect.height() / cell).ceil() as i32;
    for r in 0..rows {
        for c in 0..cols {
            if (r + c) % 2 == 0 {
                continue;
            }
            let cr = Rect::from_min_size(rect.min + vec2(c as f32 * cell, r as f32 * cell), vec2(cell, cell))
                .intersect(rect);
            if cr.width() <= 0.0 || cr.height() <= 0.0 {
                continue;
            }
            if clear_of_corners(cr, rect, radius) {
                mesh.add_colored_rect(cr, dark);
            } else {
                add_fan(&mut mesh, &clip_convex(&quad(cr), &poly), &|_| dark);
            }
        }
    }
    painter.add(Shape::mesh(mesh));
}

// MARK: Swatches

/// Paints a color chip: transparency checkerboard, the color and a faint outline (`Swatch`, without the ring).
pub fn paint_swatch(painter: &Painter, rect: Rect, rgba: u32, radius: f32, cell: f32) {
    if rgba & 0xFF != 0xFF {
        draw_checker(painter, rect, cell, radius);
    }
    painter.rect_filled(rect, radius, rgba_color(rgba));
    painter.rect_stroke(rect, radius, Stroke::new(1.0, Color32::from_white_alpha(18)), StrokeKind::Middle);
}

/// A color chip of `size` points with an optional selection ring (`Swatch`).
pub fn swatch(ui: &mut Ui, rgba: u32, size: f32, selected: bool, radius: f32) -> Response {
    let c = Colors::of(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    paint_swatch(ui.painter(), rect, rgba, radius, (size / 5.0).max(2.0));
    if selected {
        ui.painter().rect_stroke(rect.expand(2.5), radius + 1.5, Stroke::new(2.0, c.accent), StrokeKind::Middle);
    }
    response
}

// MARK: Icon buttons and menus

/// A fixed-size icon button with hover, pressed and selected states (`IconButton` with `DSIconButtonStyle`).
/// Header actions use `size` 24 and `icon_size` 13; the standard button is 28 and 14.
pub fn ds_icon_button(
    ui: &mut Ui,
    icon: &str,
    size: f32,
    icon_size: f32,
    selected: bool,
    destructive: bool,
) -> Response {
    let c = Colors::of(ui.ctx());
    let enabled = ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hover = response.hovered() && enabled;
    let down = response.is_pointer_button_down_on() && enabled;
    let fill = if selected {
        if down || hover { c.selected_hover } else { c.selected }
    } else if down {
        c.pressed
    } else if hover {
        c.hover
    } else {
        Color32::TRANSPARENT
    };
    let fg = if selected {
        c.accent
    } else if destructive {
        if hover { c.destructive } else { c.text_secondary }
    } else if hover {
        c.text
    } else {
        c.text_secondary
    };
    ui.painter().rect_filled(rect, radius::CONTROL, fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(icon_size * 1.15),
        dimmed(enabled, fg),
    );
    response
}

/// An icon-only menu button, the "more" menus of section headers (`DSMenuButton`).
pub fn ds_menu_button(ui: &mut Ui, icon: &str, help: &str, add_contents: impl FnOnce(&mut Ui)) -> Response {
    let response = ds_icon_button(ui, icon, metric::CONTROL, 13.0, false, false).on_hover_text(help);
    egui::Popup::menu(&response).show(add_contents);
    response
}

// MARK: Fields

fn field_background(painter: &Painter, c: &Colors, rect: Rect, focused: bool, enabled: bool) {
    painter.rect_filled(rect, radius::FIELD, dimmed(enabled, c.field));
    if focused {
        painter.rect_stroke(rect, radius::FIELD, Stroke::new(1.5, c.accent_line), StrokeKind::Middle);
    }
}

/// A single-line text field in the Pixmith style: 24 pt high, field fill, accent border while focused
/// (`dsTextField`). Returns the text edit's response; check `lost_focus()` to commit.
pub fn ds_text_field(ui: &mut Ui, id: impl Hash + std::fmt::Debug, text: &mut String, width: Option<f32>) -> Response {
    let c = Colors::of(ui.ctx());
    let id = ui.id().with(id);
    let w = width.unwrap_or(ui.available_width());
    let (rect, _) = ui.allocate_exact_size(vec2(w, metric::CONTROL), Sense::hover());
    let focused = ui.memory(|m| m.has_focus(id));
    field_background(ui.painter(), &c, rect, focused, ui.is_enabled());
    let edit = TextEdit::singleline(text)
        .id(id)
        .font(ui_font(font::BODY))
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::ZERO)
        .vertical_align(Align::Center)
        .desired_width(rect.width() - 2.0 * space::SM);
    put_in(ui, rect.shrink2(vec2(space::SM, 0.0)), edit)
}

/// Adds `widget` filling `rect` without moving the layout cursor (unlike `Ui::put`).
pub fn put_in(ui: &mut Ui, rect: Rect, widget: impl egui::Widget) -> Response {
    let mut child = ui.new_child(
        UiBuilder::new().max_rect(rect).layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
    );
    child.add(widget)
}

/// The result of a [`ds_number_field`] frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct NumberResult {
    /// The value was changed (by typing, or by scrubbing; with `defer_scrub` only when the drag ends).
    pub changed: bool,
    /// The label is being dragged.
    pub scrubbing: bool,
    pub scrub_started: bool,
    pub scrub_ended: bool,
}

/// Options of [`ds_number_field`], matching the parameters of `DSNumberField`.
#[derive(Clone, Copy, Debug)]
pub struct NumberOpts<'a> {
    pub range: (f64, f64),
    /// Value change per point of horizontal drag.
    pub step: f64,
    pub unit: &'a str,
    pub digits: usize,
    /// `None` fills the available width.
    pub width: Option<f32>,
    pub help: &'a str,
    /// A scrub shows the live value but writes it only when the drag ends (one undo step).
    pub defer_scrub: bool,
}

impl Default for NumberOpts<'_> {
    fn default() -> Self {
        NumberOpts {
            range: (0.0, 100.0),
            step: 1.0,
            unit: "",
            digits: 0,
            width: Some(80.0),
            help: "",
            defer_scrub: false,
        }
    }
}

fn clamp_number(v: f64, o: &NumberOpts) -> f64 {
    let p = 10f64.powi(o.digits as i32);
    ((v * p).round() / p).clamp(o.range.0, o.range.1)
}

fn format_number(v: f64, digits: usize) -> String {
    let s = format!("{:.*}", digits, v);
    if digits > 0 && s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// A number field with a label and optional unit. Drag the label to scrub the value (Shift for 10x), or type
/// (`DSNumberField`).
pub fn ds_number_field(
    ui: &mut Ui,
    id: impl Hash + std::fmt::Debug,
    label: &str,
    value: &mut f64,
    o: &NumberOpts,
) -> NumberResult {
    let c = Colors::of(ui.ctx());
    let enabled = ui.is_enabled();
    let id = ui.id().with(id);
    let (text_id, buf_id, scrub_id, live_id) = (id.with("text"), id.with("buf"), id.with("scrub"), id.with("live"));
    let w = o.width.unwrap_or(ui.available_width());
    let (rect, _) = ui.allocate_exact_size(vec2(w, metric::CONTROL), Sense::hover());
    let painter = ui.painter().clone();
    let focused = ui.memory(|m| m.has_focus(text_id));
    field_background(&painter, &c, rect, focused, enabled);

    let mut res = NumberResult::default();
    let mut start: Option<f64> = ui.data(|d| d.get_temp(scrub_id));
    let mut live: Option<f64> = ui.data(|d| d.get_temp(live_id));

    // The label doubles as the scrub handle.
    let label_color = if start.is_some() { c.accent } else { c.text_tertiary };
    let galley =
        painter.layout_no_wrap(label.to_owned(), strong(ui.ctx(), font::CAPTION), dimmed(enabled, label_color));
    let label_rect = Rect::from_min_size(rect.min + vec2(space::SM, 0.0), vec2(galley.size().x, rect.height()));
    paint_heavy(
        &painter,
        pos2(label_rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        dimmed(enabled, label_color),
        0.3,
    );
    let label_resp = ui.interact(label_rect, id.with("label"), if enabled { Sense::drag() } else { Sense::hover() });
    if label_resp.hovered() || label_resp.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if label_resp.drag_started() {
        start = Some(*value);
        res.scrub_started = true;
    }
    if let (true, Some(s0)) = (label_resp.dragged(), start) {
        let dx = ui.input(|i| Some(i.pointer.interact_pos()?.x - i.pointer.press_origin()?.x)).unwrap_or(0.0) as f64;
        let fast = if ui.input(|i| i.modifiers.shift) { 10.0 } else { 1.0 };
        let v = clamp_number(s0 + dx * o.step * fast, o);
        if o.defer_scrub {
            live = Some(v);
        } else if v != *value {
            *value = v;
            res.changed = true;
        }
    }
    if label_resp.drag_stopped() {
        start = None;
        if let Some(l) = live.take() {
            *value = l;
            res.changed = true;
        }
        res.scrub_ended = true;
    }
    res.scrubbing = start.is_some();
    ui.data_mut(|d| {
        match start {
            Some(s) => {
                d.insert_temp(scrub_id, s);
            }
            None => {
                d.remove_temp::<f64>(scrub_id);
            }
        }
        match live {
            Some(l) => {
                d.insert_temp(live_id, l);
            }
            None => {
                d.remove_temp::<f64>(live_id);
            }
        }
    });

    // Unit, then the editable text between label and unit.
    let mut right = rect.right() - space::SM;
    if !o.unit.is_empty() {
        let g = painter.layout_no_wrap(o.unit.to_owned(), ui_font(font::CAPTION), dimmed(enabled, c.text_tertiary));
        let w = g.size().x;
        painter.galley(pos2(right - w, rect.center().y - g.size().y / 2.0), g, c.text_tertiary);
        right -= w + space::XS;
    }
    let text_rect = Rect::from_min_max(
        pos2(label_rect.right() + space::XS, rect.top()),
        pos2(right.max(label_rect.right() + 8.0), rect.bottom()),
    );
    let shown = live.unwrap_or(*value);
    let mut buf = if focused {
        ui.data(|d| d.get_temp::<String>(buf_id)).unwrap_or_else(|| format_number(shown, o.digits))
    } else {
        format_number(shown, o.digits)
    };
    let edit = TextEdit::singleline(&mut buf)
        .id(text_id)
        .font(ui_font(font::BODY))
        .horizontal_align(Align::RIGHT)
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::ZERO)
        .text_color(dimmed(enabled, c.text))
        .vertical_align(Align::Center)
        .desired_width(text_rect.width());
    let r = put_in(ui, text_rect, edit);
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(buf_id, buf.clone()));
    }
    if r.lost_focus() {
        ui.data_mut(|d| d.remove_temp::<String>(buf_id));
        if let Ok(v) = buf.trim().parse::<f64>() {
            let v = clamp_number(v, o);
            if v != *value {
                *value = v;
                res.changed = true;
            }
        }
    }
    if !o.help.is_empty() {
        label_resp.on_hover_text(o.help);
    }
    res
}

/// A dropdown in the field style (`DSPopup`). `width` `None` hugs the title, `fill` stretches to the available
/// width. Returns the newly chosen index.
pub fn ds_popup(
    ui: &mut Ui,
    id: impl Hash + std::fmt::Debug,
    selected: usize,
    options: &[&str],
    width: Option<f32>,
    fill: bool,
    help: &str,
) -> Option<usize> {
    let c = Colors::of(ui.ctx());
    let enabled = ui.is_enabled();
    let id = ui.id().with(id);
    let title = options.get(selected).copied().unwrap_or("");
    let galley = ui.painter().layout_no_wrap(title.to_owned(), ui_font(font::BODY), dimmed(enabled, c.text));
    let w = width.unwrap_or_else(|| {
        if fill { ui.available_width() } else { space::SM + galley.size().x + space::XS + 8.0 + space::SM }
    });
    let (rect, _) = ui.allocate_exact_size(vec2(w, metric::CONTROL), Sense::hover());
    let response = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    let painter = ui.painter();
    painter.rect_filled(rect, radius::FIELD, dimmed(enabled, c.field));
    painter.galley(pos2(rect.left() + space::SM, rect.center().y - galley.size().y / 2.0), galley, c.text);
    // chevron.down, 9 pt semibold
    let (cx, cy) = (rect.right() - space::SM - 4.0, rect.center().y);
    let stroke = Stroke::new(1.5, dimmed(enabled, c.text_tertiary));
    painter.line_segment([pos2(cx - 3.6, cy - 1.8), pos2(cx, cy + 1.8)], stroke);
    painter.line_segment([pos2(cx, cy + 1.8), pos2(cx + 3.6, cy - 1.8)], stroke);
    let response = if help.is_empty() { response } else { response.on_hover_text(help) };
    let mut chosen = None;
    egui::Popup::menu(&response).width(rect.width()).show(|ui| {
        for (i, o) in options.iter().enumerate() {
            if ui.selectable_label(i == selected, *o).clicked() {
                chosen = Some(i);
            }
        }
    });
    chosen
}

// MARK: Inspector section

/// Whether the section `id` is collapsed (`panel.<id>.collapsed` in the macOS app).
pub fn section_collapsed(ctx: &egui::Context, id: &str) -> bool {
    ctx.data(|d| d.get_temp::<bool>(Id::new(("inspector_section", id)))).unwrap_or(false)
}

/// Expands or collapses the section `id`.
pub fn set_section_collapsed(ctx: &egui::Context, id: &str, collapsed: bool) {
    ctx.data_mut(|d| d.insert_temp(Id::new(("inspector_section", id)), collapsed));
}

/// A collapsible inspector section: a 36 pt header with chevron, title, quiet `subtitle` and trailing actions,
/// then the content inset by 16 pt, then a hairline (`InspectorSection`).
///
/// `trailing` is laid out right to left inside the header: add the rightmost button first.
pub fn inspector_section(
    ui: &mut Ui,
    id: &str,
    title: &str,
    subtitle: Option<&str>,
    trailing: impl FnOnce(&mut Ui),
    content: impl FnOnce(&mut Ui),
) {
    let c = Colors::of(ui.ctx());
    let collapsed = section_collapsed(ui.ctx(), id);
    let width = ui.available_width();
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let (header, _) = ui.allocate_exact_size(vec2(width, metric::SECTION_HEADER), Sense::hover());

        // Trailing actions first, so the title's hit area ends where they start.
        let mut tui = ui.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_max(header.min, pos2(header.right() - space::SM, header.bottom())))
                .layout(Layout::right_to_left(Align::Center)),
        );
        tui.spacing_mut().item_spacing = vec2(space::XS, 0.0);
        trailing(&mut tui);
        let toggle_right = tui.min_rect().left().min(header.right() - space::SM);
        let toggle_rect = Rect::from_min_max(header.min, pos2(toggle_right, header.bottom()));

        let response = ui.interact(toggle_rect, Id::new(("inspector_section_header", id)), Sense::click());
        if response.clicked() {
            set_section_collapsed(ui.ctx(), id, !collapsed);
        }
        response.on_hover_text(format!("{} {title}", if collapsed { "Expand" } else { "Collapse" }));

        let painter = ui.painter();
        let cy = header.center().y;
        let cx = header.left() + metric::INSET + 6.0;
        let stroke = Stroke::new(1.5, c.text_tertiary);
        if collapsed {
            painter.line_segment([pos2(cx - 1.8, cy - 3.6), pos2(cx + 1.8, cy)], stroke);
            painter.line_segment([pos2(cx + 1.8, cy), pos2(cx - 1.8, cy + 3.6)], stroke);
        } else {
            painter.line_segment([pos2(cx - 3.6, cy - 1.8), pos2(cx, cy + 1.8)], stroke);
            painter.line_segment([pos2(cx, cy + 1.8), pos2(cx + 3.6, cy - 1.8)], stroke);
        }
        let mut x = header.left() + metric::INSET + 12.0 + space::XS;
        let tg = painter.layout_no_wrap(title.to_owned(), strong(ui.ctx(), font::CAPTION), c.text);
        let tw = tg.size().x;
        paint_heavy(painter, pos2(x, cy - tg.size().y / 2.0), tg, c.text, 0.5);
        x += tw + space::XS;
        if let Some(sub) = subtitle {
            let sg = painter.layout_no_wrap(sub.to_owned(), ui_font(font::CAPTION), c.text_tertiary);
            let clip =
                painter.with_clip_rect(Rect::from_min_max(pos2(x, header.top()), pos2(toggle_right, header.bottom())));
            clip.galley(pos2(x, cy - sg.size().y / 2.0), sg, c.text_tertiary);
        }

        if !collapsed {
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: metric::INSET as i8,
                    right: metric::INSET as i8,
                    top: 0,
                    bottom: 14,
                })
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.spacing_mut().item_spacing = vec2(space::SM, space::SM);
                    content(ui);
                });
        }
        let (line, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
        ui.painter().rect_filled(line, 0.0, c.separator_soft);
    });
}

// --- Files and dialogs: buttons, form rows, number field, popup, dialog frame (`DSDialog`, `DSFormRow`) ---

use eframe::egui::Margin;
use std::ops::RangeInclusive;

/// An icon button with its own box and glyph size (`IconButton`: 28/14 standard, 24/13 compact).
pub fn icon_button_sized(ui: &mut Ui, icon: &str, size: f32, icon_size: f32) -> Response {
    let c = Colors::of(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        c.pressed
    } else if response.hovered() {
        c.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, radius::CONTROL, fill);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(icon_size), c.text);
    response
}

/// Accent-filled button (`DSPrimaryButtonStyle`): the default action of a dialog.
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let c = Colors::of(ui.ctx());
    let width = ui.painter().layout_no_wrap(text.into(), strong(ui.ctx(), font::BODY_LARGE), Color32::WHITE).size().x;
    let size = egui::vec2((width + 2.0 * space::MD).max(64.0), 28.0);
    let (rect, response) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let alpha = if !enabled {
        0.4
    } else if response.is_pointer_button_down_on() {
        0.75
    } else if response.hovered() {
        0.92
    } else {
        1.0
    };
    ui.painter().rect_filled(rect, radius::CONTROL, c.accent.gamma_multiply(alpha));
    let text_color = Color32::WHITE.gamma_multiply(if enabled { 1.0 } else { 0.4 });
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, strong(ui.ctx(), font::BODY_LARGE), text_color);
    response
}

/// Field-filled button (`DSSecondaryButtonStyle`); `destructive` paints the label red.
pub fn secondary_button(ui: &mut Ui, text: &str, destructive: bool) -> Response {
    let c = Colors::of(ui.ctx());
    let font = ui_font(font::BODY_LARGE);
    let width = ui.painter().layout_no_wrap(text.into(), font.clone(), c.text).size().x;
    let size = egui::vec2((width + 2.0 * space::MD).max(64.0), 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        c.pressed
    } else if response.hovered() {
        c.hover
    } else {
        c.field
    };
    ui.painter().rect_filled(rect, radius::CONTROL, fill);
    let fg = if destructive { c.destructive } else { c.text };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, font, fg);
    response
}

/// A label-left, control-right row with an aligned label column (`DSFormRow`).
pub fn form_row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let c = Colors::of(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(92.0, metric::CONTROL), Sense::hover());
        ui.painter().text(
            egui::pos2(rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            ui_font(font::BODY),
            c.text_secondary,
        );
        add(ui)
    })
    .inner
}

/// A caption under a control (`DSNote`).
pub fn note(ui: &mut Ui, text: &str) {
    let c = Colors::of(ui.ctx());
    ui.label(egui::RichText::new(text).font(ui_font(font::CAPTION)).color(c.text_secondary));
}

/// An integer field with a scrubbable label and optional unit (`DSNumberField`). Drag the label to scrub
/// (Shift for 10x) or type. `label` must be unique within the dialog.
pub fn number_field(
    ui: &mut Ui,
    label: &str,
    value: &mut u32,
    range: RangeInclusive<u32>,
    unit: &str,
    width: f32,
) -> bool {
    let c = Colors::of(ui.ctx());
    let id = ui.id().with(("number_field", label));
    let edit_id = id.with("edit");
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, metric::CONTROL), Sense::hover());
    let focused = ui.memory(|m| m.has_focus(edit_id));
    let mut changed = false;

    ui.painter().rect_filled(rect, radius::FIELD, c.field);
    if focused {
        ui.painter().rect_stroke(
            rect.shrink(0.75),
            radius::FIELD,
            Stroke::new(1.5, c.accent_line),
            egui::StrokeKind::Inside,
        );
    }

    let label_w = ui.painter().layout_no_wrap(label.into(), ui_font(font::CAPTION), c.text).size().x;
    let label_rect = Rect::from_min_size(rect.min + egui::vec2(space::SM, 0.0), egui::vec2(label_w, metric::CONTROL));
    let scrub = ui.interact(label_rect.expand2(egui::vec2(2.0, 0.0)), id.with("scrub"), Sense::drag());
    if scrub.hovered() || scrub.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if scrub.dragged() {
        let fast = if ui.input(|i| i.modifiers.shift) { 10.0 } else { 1.0 };
        let carry_id = id.with("carry");
        let carry = ui.data(|d| d.get_temp::<f32>(carry_id)).unwrap_or(0.0) + scrub.drag_delta().x * fast;
        let whole = carry.trunc();
        ui.data_mut(|d| d.insert_temp(carry_id, carry - whole));
        let v = (*value as f32 + whole).clamp(*range.start() as f32, *range.end() as f32) as u32;
        changed |= v != *value;
        *value = v;
        ui.memory_mut(|m| m.surrender_focus(edit_id));
    }
    let label_color = if scrub.dragged() { c.accent } else { c.text_tertiary };
    ui.painter().text(
        egui::pos2(label_rect.left(), rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        strong(ui.ctx(), font::CAPTION),
        label_color,
    );

    let unit_w = if unit.is_empty() {
        0.0
    } else {
        ui.painter().layout_no_wrap(unit.into(), ui_font(font::CAPTION), c.text).size().x + space::XS
    };
    if !unit.is_empty() {
        ui.painter().text(
            egui::pos2(rect.right() - space::SM, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            unit,
            ui_font(font::CAPTION),
            c.text_tertiary,
        );
    }
    let edit_rect = Rect::from_min_max(
        egui::pos2(label_rect.right() + space::XS, rect.top()),
        egui::pos2(rect.right() - space::SM - unit_w, rect.bottom()),
    );
    let mut text =
        if focused { ui.data(|d| d.get_temp::<String>(edit_id)) } else { None }.unwrap_or_else(|| value.to_string());
    let out = ui.put(
        edit_rect,
        TextEdit::singleline(&mut text)
            .id(edit_id)
            .frame(egui::Frame::NONE)
            .margin(Margin::ZERO)
            .font(ui_font(font::BODY))
            .horizontal_align(Align::Max)
            .vertical_align(Align::Center)
            .desired_width(edit_rect.width()),
    );
    if out.changed() {
        text.retain(|ch| ch.is_ascii_digit());
        if let Ok(v) = text.parse::<u32>() {
            let v = v.clamp(*range.start(), *range.end());
            changed |= v != *value;
            *value = v;
        }
    }
    if out.has_focus() {
        ui.data_mut(|d| d.insert_temp(edit_id, text));
    }
    changed
}

/// A dropdown in the field style (`DSPopup`). `width` None fills the available width.
pub fn popup(ui: &mut Ui, options: &[&str], selected: &mut usize, width: Option<f32>) -> Response {
    let c = Colors::of(ui.ctx());
    let width = width.unwrap_or_else(|| ui.available_width());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, metric::CONTROL), Sense::click());
    let fill = if response.hovered() || response.is_pointer_button_down_on() { c.hover } else { c.field };
    ui.painter().rect_filled(rect, radius::FIELD, fill);
    let current = options.get(*selected).copied().unwrap_or("");
    ui.painter().text(
        egui::pos2(rect.left() + space::SM, rect.center().y),
        egui::Align2::LEFT_CENTER,
        current,
        ui_font(font::BODY),
        c.text,
    );
    ui.painter().text(
        egui::pos2(rect.right() - space::SM, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        egui_phosphor::regular::CARET_DOWN,
        egui::FontId::proportional(9.0),
        c.text_tertiary,
    );
    egui::Popup::menu(&response).width(width).show(|ui| {
        for (i, option) in options.iter().enumerate() {
            let label = if i == *selected {
                format!("{}  {option}", egui_phosphor::regular::CHECK)
            } else {
                format!("      {option}")
            };
            if ui.button(label).clicked() {
                *selected = i;
            }
        }
    });
    response
}

/// What the user chose in a dialog's button row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogAction {
    None,
    Destructive,
    Cancel,
    Primary,
}

/// The button row of `DSDialog`: an optional destructive button on the left, then Cancel and the primary
/// button on the right. Return triggers the primary button and Escape cancels.
pub fn dialog_buttons(
    ui: &mut Ui,
    destructive: Option<&str>,
    cancel: Option<&str>,
    primary: &str,
    enabled: bool,
) -> DialogAction {
    let mut action = DialogAction::None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        if let Some(title) = destructive
            && secondary_button(ui, title, true).clicked()
        {
            action = DialogAction::Destructive;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if primary_button(ui, primary, enabled).clicked() && enabled {
                action = DialogAction::Primary;
            }
            if let Some(title) = cancel
                && secondary_button(ui, title, false).clicked()
            {
                action = DialogAction::Cancel;
            }
        });
    });
    if enabled && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
        action = DialogAction::Primary;
    }
    if cancel.is_some() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        action = DialogAction::Cancel;
    }
    action
}

/// A modal sheet in the `DSDialog` style: title, optional subtitle, then `body` (form rows and the button
/// row). `width` is the full sheet width.
pub fn dialog<R>(
    ctx: &egui::Context,
    id: &str,
    width: f32,
    title: &str,
    subtitle: Option<&str>,
    body: impl FnOnce(&mut Ui) -> R,
) -> R {
    let c = Colors::of(ctx);
    let frame = egui::Frame::new()
        .fill(c.panel)
        .stroke(Stroke::new(1.0, c.separator))
        .corner_radius(egui::CornerRadius::same(radius::PANEL + 2))
        .inner_margin(Margin::same(20))
        .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(90) });
    egui::Modal::new(egui::Id::new(("dialog", id)))
        .frame(frame)
        .backdrop_color(Color32::from_black_alpha(80))
        .show(ctx, |ui| {
            ui.set_width(width - 40.0);
            ui.spacing_mut().item_spacing = egui::vec2(space::SM, space::MD);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = space::XS;
                ui.label(egui::RichText::new(title).font(strong(ui.ctx(), 15.0)));
                if let Some(s) = subtitle {
                    note(ui, s);
                }
            });
            ui.add_space(space::XS);
            body(ui)
        })
        .inner
}
