//! Small controls private to the timeline: icon buttons with the `DSIconButtonStyle` states, the transport
//! glyphs, the group divider and the label-scrub number field (`DSNumberField`).

use crate::theme::{Colors, font, radius, ui_font};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, Margin, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, TextEdit, Ui, Vec2, vec2,
};

/// How a button looks (`DSIconButtonStyle`).
#[derive(Clone, Copy)]
pub struct Look {
    pub selected: bool,
    pub destructive: bool,
    pub disabled: bool,
    pub radius: u8,
}

impl Default for Look {
    fn default() -> Look {
        Look { selected: false, destructive: false, disabled: false, radius: radius::CONTROL }
    }
}

pub fn fade(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a)
}

/// Draws the button background for the current hover / pressed state and returns the glyph color.
pub fn chrome(ui: &Ui, rect: Rect, resp: &Response, look: Look) -> Color32 {
    let c = Colors::of(ui.ctx());
    let hover = resp.hovered() && !look.disabled;
    let pressed = resp.is_pointer_button_down_on() && !look.disabled;
    let fill = if look.selected {
        if pressed || hover { c.selected_hover } else { c.selected }
    } else if pressed {
        c.pressed
    } else if hover {
        c.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(look.radius), fill);
    let fg = if look.selected {
        c.accent
    } else if look.destructive {
        if hover { c.destructive } else { c.text_secondary }
    } else if hover {
        c.text
    } else {
        c.text_secondary
    };
    if look.disabled { fade(fg, 0.35) } else { fg }
}

/// A square button; `draw` paints the glyph centered in the returned rect.
pub fn button(
    ui: &mut Ui,
    size: Vec2,
    look: Look,
    tip: &str,
    draw: impl FnOnce(&egui::Painter, Rect, Color32),
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, if look.disabled { Sense::hover() } else { Sense::click() });
    let fg = chrome(ui, rect, &resp, look);
    draw(ui.painter(), rect, fg);
    if !tip.is_empty() { resp.on_hover_text(tip) } else { resp }
}

/// An icon-only button with a Phosphor glyph (`IconButton`: 28 pt hit target, 14 pt icon; compact is 24 / 13).
pub fn icon_button(ui: &mut Ui, glyph: &str, size: f32, icon: f32, look: Look, tip: &str) -> Response {
    button(ui, vec2(size, size), look, tip, |p, rect, fg| {
        // Phosphor glyphs are drawn a little larger than the SF Symbols they stand in for.
        p.text(rect.center(), Align2::CENTER_CENTER, glyph, ui_font(icon * 1.15), fg);
    })
}

#[derive(Clone, Copy, PartialEq)]
pub enum Transport {
    Back,
    Play,
    Pause,
    Forward,
}

/// The filled SF Symbols `backward.frame.fill`, `play.fill`, `pause.fill` and `forward.frame.fill`.
pub fn transport_button(ui: &mut Ui, kind: Transport, look: Look, tip: &str) -> Response {
    button(ui, vec2(28.0, 28.0), look, tip, |p, rect, fg| {
        let ctr = rect.center();
        let tri = |x0: f32, x1: f32, h: f32| {
            // x0 is the base, x1 the tip.
            Shape::convex_polygon(
                vec![
                    Pos2::new(ctr.x + x0, ctr.y - h / 2.0),
                    Pos2::new(ctr.x + x1, ctr.y),
                    Pos2::new(ctr.x + x0, ctr.y + h / 2.0),
                ],
                fg,
                Stroke::new(1.0, fg),
            )
        };
        let bar = |x0: f32, x1: f32, h: f32| {
            Shape::rect_filled(
                Rect::from_min_max(Pos2::new(ctr.x + x0, ctr.y - h / 2.0), Pos2::new(ctr.x + x1, ctr.y + h / 2.0)),
                1.0,
                fg,
            )
        };
        match kind {
            Transport::Back => {
                p.add(tri(2.0, -7.5, 10.5));
                p.add(bar(3.5, 7.0, 11.0));
            }
            Transport::Forward => {
                p.add(bar(-7.0, -3.5, 11.0));
                p.add(tri(-2.0, 7.5, 10.5));
            }
            Transport::Play => {
                p.add(tri(-5.0, 6.0, 12.0));
            }
            Transport::Pause => {
                p.add(bar(-5.0, -1.5, 12.0));
                p.add(bar(1.5, 5.0, 12.0));
            }
        }
    })
}

/// The vertical rule between button groups (`DSGroupDivider`): 20 pt tall with 4 pt margins.
pub fn group_divider(ui: &mut Ui) {
    let c = Colors::of(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(1.0 + 8.0, 20.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, c.separator));
}

/// A label-scrub scrub in progress.
#[derive(Clone, Copy)]
pub struct Scrub {
    pub field: u8,
    pub start: f64,
    pub live: i64,
}

/// A number field (`DSNumberField`): label left (drag it to scrub, Shift for 10x), value right, optional unit.
/// Returns the new value when it changes. With `defer` a scrub shows the live value but reports it on release.
#[allow(clippy::too_many_arguments)]
pub fn number_field(
    ui: &mut Ui,
    field: u8,
    buf: &mut String,
    scrub: &mut Option<Scrub>,
    label: &str,
    value: i64,
    range: (i64, i64),
    unit: &str,
    width: f32,
    defer: bool,
    tip: &str,
) -> Option<i64> {
    let c = Colors::of(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    let mut out = None;
    let shown = match scrub {
        Some(s) if s.field == field && defer => s.live,
        _ => value,
    };
    let label_g = ui.painter().layout_no_wrap(
        label.to_owned(),
        crate::ui::widgets::strong(ui.ctx(), font::CAPTION),
        c.text_tertiary,
    );
    let unit_g = (!unit.is_empty())
        .then(|| ui.painter().layout_no_wrap(unit.to_owned(), ui_font(font::CAPTION), c.text_tertiary));
    let label_rect =
        Rect::from_min_size(Pos2::new(rect.left() + 8.0, rect.center().y - 10.0), vec2(label_g.size().x, 20.0));
    let unit_w = unit_g.as_ref().map_or(0.0, |g| g.size().x + 4.0);
    let edit_rect = Rect::from_min_max(
        Pos2::new(label_rect.right() + 4.0, rect.top() + 1.0),
        Pos2::new(rect.right() - 8.0 - unit_w, rect.bottom() - 1.0),
    );

    // Scrub by dragging the label.
    let drag = ui.interact(label_rect, ui.id().with(("scrub", field)), Sense::drag());
    if drag.hovered() || drag.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if drag.drag_started() {
        *scrub = Some(Scrub { field, start: value as f64, live: value });
    }
    let scrubbing = matches!(scrub, Some(s) if s.field == field);
    if drag.dragged() && scrubbing {
        let shift = ui.input(|i| i.modifiers.shift);
        let dx = ui.input(|i| i.pointer.press_origin().zip(i.pointer.interact_pos()).map_or(0.0, |(a, b)| b.x - a.x));
        if let Some(s) = scrub {
            let v = (s.start + dx as f64 * if shift { 10.0 } else { 1.0 }).round() as i64;
            s.live = v.clamp(range.0, range.1);
            if !defer && s.live != value {
                out = Some(s.live);
            }
        }
    }
    if drag.drag_stopped()
        && let Some(s) = scrub.take()
        && defer
        && s.live != value
    {
        out = Some(s.live);
    }

    // Background and the typed value.
    let edit_id = ui.id().with(("edit", field));
    let focused = ui.memory(|m| m.has_focus(edit_id));
    ui.painter().rect_filled(rect, CornerRadius::same(radius::FIELD), c.field);
    if focused {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(radius::FIELD),
            Stroke::new(1.5, c.accent_line),
            StrokeKind::Inside,
        );
    }
    let label_color = if scrubbing { c.accent } else { c.text_tertiary };
    ui.painter().galley(label_rect.left_center() - vec2(0.0, label_g.size().y / 2.0), label_g, label_color);
    if let Some(g) = unit_g {
        ui.painter().galley(
            Pos2::new(rect.right() - 8.0 - g.size().x, rect.center().y - g.size().y / 2.0),
            g,
            c.text_tertiary,
        );
    }
    if !focused {
        *buf = shown.to_string();
    }
    let mut edit_ui = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect));
    let resp = edit_ui.add(
        TextEdit::singleline(buf)
            .id(edit_id)
            .frame(egui::Frame::NONE)
            .margin(Margin::ZERO)
            .font(ui_font(font::BODY))
            .text_color(c.text)
            .horizontal_align(Align::RIGHT)
            .vertical_align(Align::Center),
    );
    if resp.lost_focus() && !ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        if let Ok(v) = buf.trim().parse::<i64>() {
            let v = v.clamp(range.0, range.1);
            if v != value {
                out = Some(v);
            }
        }
        *buf = shown.to_string();
    }
    let _ = resp.on_hover_text(tip);
    out
}
