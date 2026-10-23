//! Layers x frames: the layers column on the left, the frame numbers, tag bars and cel grid on the right
//! (`TimelineGrid`, `TimelineCanvas`). Click selects a cel, shift-click a range, dragging moves the selection
//! (alt copies).

use super::controls::fade;
use super::sheets::TagSheet;
use super::tags::{self, CELL_WIDTH, NUMBER_ROW, TagLayout};
use super::{CelRect, Press, TimelineState, header, layers, playback};
use crate::app::DocTab;
use crate::theme::{Colors, ui_font};
use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, Sense, Shape, Stroke, StrokeKind, Theme, pos2, vec2};
use egui_phosphor::regular as icon;

const NAME_WIDTH: f32 = 200.0;
const ROW: f32 = layers::ROW_HEIGHT;

/// Layer indices shown on the rows, top to bottom. Children of a collapsed group are hidden.
fn visible_rows(tab: &DocTab, st: &TimelineState) -> Vec<u32> {
    let nl = tab.doc.layer_count();
    let mut hidden = std::collections::HashSet::new();
    for &g in &st.collapsed_groups {
        if g < nl && tab.doc.layer_is_group(g) {
            let mut j = g + 1;
            while j < nl && tab.doc.layer_level(j) > tab.doc.layer_level(g) {
                hidden.insert(j);
                j += 1;
            }
        }
    }
    (0..nl).rev().filter(|l| !hidden.contains(l)).collect()
}

pub fn show(tab: &mut DocTab, st: &mut TimelineState, active: usize, ui: &mut egui::Ui, area: Rect) {
    let ctx = ui.ctx().clone();
    let c = Colors::of(&ctx);
    let dark = ctx.theme() == Theme::Dark;
    let (nl, nf) = (tab.doc.layer_count(), tab.doc.frame_count());

    // The dot grid, refreshed whenever the document changes.
    let key = (active, tab.revision, nl, nf);
    if st.cells_key != Some(key) {
        st.cells = tab.doc.cel_grid();
        st.cells_key = Some(key);
    }
    // Drop state that refers to layers or frames that no longer exist.
    st.collapsed_groups.retain(|&g| g < nl);

    let rows = visible_rows(tab, st);
    let tag_layout = TagLayout::new(&tab.doc);
    let header_h = tag_layout.height() + NUMBER_ROW;

    let names = Rect::from_min_size(area.min, vec2(NAME_WIDTH, area.height()));
    let names_head = Rect::from_min_size(names.min, vec2(NAME_WIDTH, header_h));
    let names_body = Rect::from_min_max(pos2(names.left(), names.top() + header_h), names.max);
    let frames_head =
        Rect::from_min_max(pos2(area.left() + NAME_WIDTH, area.top()), pos2(area.right(), area.top() + header_h));
    let body = Rect::from_min_max(pos2(area.left() + NAME_WIDTH, area.top() + header_h), area.max);

    // Scrolling: one offset shared by the layers column, the frame header and the grid.
    let content = vec2((nf as f32 + 1.0) * CELL_WIDTH, rows.len() as f32 * ROW);
    let max_scroll = vec2((content.x - body.width()).max(0.0), (content.y - body.height()).max(0.0));
    if ui.rect_contains_pointer(area) {
        let d = ui.input(|i| i.smooth_scroll_delta);
        st.scroll -= d;
    }
    st.scroll = st.scroll.max(egui::Vec2::ZERO).min(max_scroll);
    let scroll = st.scroll;

    // Layers column.
    layers::header(ui, tab, st, names_head);
    layers::rows(ui, tab, st, &rows, names_body, scroll.y);
    ui.painter().rect_filled(
        Rect::from_min_size(pos2(names.right() - 1.0, names.top()), vec2(1.0, names.height())),
        0.0,
        c.separator_soft,
    );

    frame_header(ui, tab, st, &c, dark, &tag_layout, frames_head, scroll.x);
    cel_body(ui, tab, st, &c, dark, &rows, body, scroll);
}

fn guide_color(dark: bool) -> Color32 {
    if dark { Color32::from_white_alpha(9) } else { Color32::from_black_alpha(13) }
}

/// Tag bars, frame numbers and the "+" button.
#[allow(clippy::too_many_arguments)]
fn frame_header(
    ui: &mut egui::Ui,
    tab: &mut DocTab,
    st: &mut TimelineState,
    c: &Colors,
    dark: bool,
    tag_layout: &TagLayout,
    rect: Rect,
    scroll_x: f32,
) {
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    // Faint frame-column guides.
    let mut x = rect.left() + CELL_WIDTH - 0.5 - scroll_x.rem_euclid(CELL_WIDTH);
    while x < rect.right() {
        painter.vline(x, rect.y_range(), Stroke::new(1.0, guide_color(dark)));
        x += CELL_WIDTH;
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let ui = &mut child;
    let origin = pos2(rect.left() - scroll_x, rect.top());

    // Tag bars.
    for (index, tag, row) in &tag_layout.items {
        let bar = tags::bar_rect(origin, tag, *row);
        let selected = tab.frame >= tag.from && tab.frame <= tag.to;
        let resp = ui.interact(bar, ui.id().with(("tag", *index)), Sense::click());
        tags::paint_bar(&painter, bar, tag, selected, dark, c);
        let resp = resp.on_hover_text(format!(
            "{} (frames {}\u{2013}{}). Double-click to edit.",
            tag.name,
            tag.from + 1,
            tag.to + 1
        ));
        if resp.double_clicked() {
            st.tag_sheet = Some(TagSheet { index: Some(*index), tag: tag.clone() });
        } else if resp.clicked() {
            playback::stop(tab);
            tab.frame = tag.from;
        }
        resp.context_menu(|ui| {
            if ui.button("Edit Tag\u{2026}").clicked() {
                st.tag_sheet = Some(TagSheet { index: Some(*index), tag: tag.clone() });
                ui.close();
            }
            if ui.button("Delete Tag").clicked() {
                if tab.doc.remove_tag(*index) {
                    tab.changed();
                }
                ui.close();
            }
        });
    }

    // Frame numbers.
    let nf = tab.doc.frame_count();
    let numbers_top = rect.bottom() - NUMBER_ROW;
    for f in 0..nf {
        let cell =
            Rect::from_min_size(pos2(origin.x + f as f32 * CELL_WIDTH, numbers_top), vec2(CELL_WIDTH, NUMBER_ROW));
        if cell.right() < rect.left() || cell.left() > rect.right() {
            continue;
        }
        let current = f == tab.frame;
        let resp = ui.interact(cell, ui.id().with(("frame_no", f)), Sense::click());
        let (font, color) = if current {
            (crate::ui::widgets::strong(ui.ctx(), 11.0), c.text)
        } else {
            (ui_font(11.0), c.text_tertiary)
        };
        painter.text(cell.center(), Align2::CENTER_CENTER, (f + 1).to_string(), font, color);
        if resp.clicked() {
            playback::stop(tab);
            tab.frame = f;
            st.range = None;
            st.focused = true;
        }
    }
    // The "+" after the last frame.
    let plus = Rect::from_min_size(pos2(origin.x + nf as f32 * CELL_WIDTH, numbers_top), vec2(CELL_WIDTH, NUMBER_ROW));
    let resp = ui.interact(plus, ui.id().with("frame_plus"), Sense::click());
    let fg = super::controls::chrome(ui, plus, &resp, super::controls::Look { radius: 4, ..Default::default() });
    painter.text(plus.center(), Align2::CENTER_CENTER, icon::PLUS, ui_font(11.5), fg);
    if resp.on_hover_text("New frame").clicked() {
        header::add_frame(tab, false);
    }
}

/// A cell index at `pos` in the grid, if the grid has any rows.
fn cell_at(pos: egui::Pos2, body: Rect, scroll: egui::Vec2, rows: &[u32], nf: u32) -> Option<(u32, u32)> {
    if rows.is_empty() || nf == 0 {
        return None;
    }
    let x = pos.x - body.left() + scroll.x;
    let y = pos.y - body.top() + scroll.y;
    let f = ((x / CELL_WIDTH).floor().max(0.0) as u32).min(nf - 1);
    let r = ((y / ROW).floor().max(0.0) as usize).min(rows.len() - 1);
    Some((rows[r], f))
}

/// The cels: highlights, guides, dots, selection, and the mouse handling.
#[allow(clippy::too_many_arguments)]
fn cel_body(
    ui: &mut egui::Ui,
    tab: &mut DocTab,
    st: &mut TimelineState,
    c: &Colors,
    dark: bool,
    rows: &[u32],
    body: Rect,
    scroll: egui::Vec2,
) {
    let (nl, nf) = (tab.doc.layer_count(), tab.doc.frame_count());
    let resp = ui.interact(body, ui.id().with("cel_body"), Sense::click_and_drag());
    let painter = ui.painter().with_clip_rect(body.intersect(ui.clip_rect()));
    let origin = body.min - scroll;
    let accent = c.accent;
    let selected = st.selected_cels(tab);
    let dot = if dark { Color32::from_gray(163) } else { Color32::from_gray(102) };
    let faint = if dark { Color32::from_gray(117) } else { Color32::from_gray(153) };

    let row_top = |r: usize| origin.y + r as f32 * ROW;
    let row_of = |layer: u32| rows.iter().position(|&l| l == layer);

    // Selected layer row and the current frame column.
    if let Some(r) = row_of(tab.layer) {
        painter.rect_filled(
            Rect::from_min_size(pos2(body.left(), row_top(r)), vec2(body.width(), ROW)),
            0.0,
            c.selected,
        );
    }
    let col = Rect::from_min_size(
        pos2(origin.x + tab.frame as f32 * CELL_WIDTH, body.top()),
        vec2(CELL_WIDTH, body.height()),
    );
    painter.rect_filled(col, 0.0, if dark { Color32::from_white_alpha(10) } else { Color32::from_black_alpha(10) });
    // Frame-column guides to the right and bottom edge.
    let mut x = body.left() + CELL_WIDTH - 0.5 - scroll.x.rem_euclid(CELL_WIDTH);
    while x < body.right() {
        painter.vline(x, body.y_range(), Stroke::new(1.0, guide_color(dark)));
        x += CELL_WIDTH;
    }

    let marks = st.cells.len() == (nl * nf) as usize;
    let first_f = (scroll.x / CELL_WIDTH).floor().max(0.0) as u32;
    let last_f = (((scroll.x + body.width()) / CELL_WIDTH).ceil() as u32).min(nf);
    let first_r = (scroll.y / ROW).floor().max(0.0) as usize;
    let last_r = (((scroll.y + body.height()) / ROW).ceil() as usize).min(rows.len());
    for (r, &l) in rows.iter().enumerate().take(last_r).skip(first_r) {
        let group = tab.doc.layer_is_group(l);
        let dim = if tab.doc.layer_visible(l) { 1.0 } else { 0.4 };
        for f in first_f..last_f {
            let cell = Rect::from_min_size(pos2(origin.x + f as f32 * CELL_WIDTH, row_top(r)), vec2(CELL_WIDTH, ROW));
            if selected.contains(l, f) && selected.is_multi() {
                painter.rect_filled(cell, 0.0, with_alpha(accent, 0.30));
            }
            if !marks || group {
                continue;
            }
            let bits = st.cells[(l * nf + f) as usize];
            if bits & 1 == 0 {
                continue;
            }
            let current = l == tab.layer && f == tab.frame;
            let (ctr, ink) = (cell.center(), if current { accent } else { dot });
            let ink_faint = if current { accent } else { faint };
            if bits & 4 != 0 {
                // Linked cel: a short bar, longer on the current frame.
                let w = if f == tab.frame { 14.0 } else { 4.0 };
                painter.rect_filled(
                    Rect::from_center_size(ctr, vec2(w, 4.0)),
                    CornerRadius::same(2),
                    fade(ink_faint, dim),
                );
            } else if bits & 2 != 0 {
                painter.circle_filled(ctr, 5.0, fade(ink, dim));
            } else {
                painter.circle_stroke(ctr, 4.25, Stroke::new(1.5, fade(ink_faint, dim)));
            }
        }
    }
    if let Some(r) = row_of(tab.layer) {
        let cur =
            Rect::from_min_size(pos2(origin.x + tab.frame as f32 * CELL_WIDTH, row_top(r)), vec2(CELL_WIDTH, ROW));
        painter.rect_stroke(
            cur.shrink(2.0),
            CornerRadius::same(4),
            Stroke::new(1.0, c.accent_line),
            StrokeKind::Middle,
        );
    }
    // The outline of the cels being dragged.
    if let Some(p) = st.press.filter(|p| p.delta != (0, 0)) {
        let moved = p.rect.shifted(p.delta.0, p.delta.1);
        let span = |layer: u32| row_of(layer);
        if let (Some(ra), Some(rb)) = (span(moved.l0).or(span(moved.l1)), span(moved.l1).or(span(moved.l0))) {
            let (top, bottom) = (ra.min(rb), ra.max(rb));
            let r = Rect::from_min_max(
                pos2(origin.x + moved.f0 as f32 * CELL_WIDTH, row_top(top)),
                pos2(origin.x + (moved.f1 + 1) as f32 * CELL_WIDTH, row_top(bottom) + ROW),
            )
            .shrink(1.0);
            let pts = vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            painter.extend(Shape::dashed_line(&pts, Stroke::new(1.5, accent), 4.0, 3.0));
        }
    }

    // Mouse: a press selects (or begins a drag of the selection), dragging moves, release applies.
    let (pressed, down, released, pos, shift, alt) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.primary_released(),
            i.pointer.latest_pos(),
            i.modifiers.shift,
            i.modifiers.alt,
        )
    });
    if pressed
        && resp.contains_pointer()
        && !st.press_active
        && let Some(c0) = pos.and_then(|p| cell_at(p, body, scroll, rows, nf))
    {
        st.press_active = true;
        begin(tab, st, c0, shift);
    }
    if st.press_active {
        if let (Some(press), Some(p)) = (st.press.as_mut(), pos)
            && let Some(c1) = cell_at(p, body, scroll, rows, nf)
        {
            let r = press.rect;
            let dl = (c1.0 as i64 - press.start.0 as i64).clamp(-(r.l0 as i64), nl as i64 - 1 - r.l1 as i64);
            let df = (c1.1 as i64 - press.start.1 as i64).clamp(-(r.f0 as i64), nf as i64 - 1 - r.f1 as i64);
            press.delta = (dl as i32, df as i32);
        }
        if released || !down {
            end(tab, st, alt);
            st.press_active = false;
        }
    }
}

fn with_alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a * 255.0).round() as u8)
}

/// Mouse down on a cel (`begin`).
fn begin(tab: &mut DocTab, st: &mut TimelineState, c: (u32, u32), shift: bool) {
    let (layer, frame) = c;
    st.focused = true;
    playback::stop(tab);
    st.press = None;
    let group = tab.doc.layer_is_group(layer);
    if shift {
        let a = st.anchor.unwrap_or((tab.layer, tab.frame));
        st.range = Some(CelRect { l0: a.0.min(layer), l1: a.0.max(layer), f0: a.1.min(frame), f1: a.1.max(frame) });
        tab.frame = frame;
        if !group {
            tab.layer = layer;
        }
        return;
    }
    let current = st.selected_cels(tab);
    if !current.contains(layer, frame) || group {
        st.range = None;
        if !group {
            tab.layer = layer;
        }
        tab.frame = frame;
        st.anchor = Some(c);
    }
    if group {
        return;
    }
    st.press = Some(Press { start: c, rect: st.selected_cels(tab), delta: (0, 0) });
}

/// Mouse up (`dragEnded`): a click inside a multi-cel selection narrows it; a drag moves or copies the cels.
fn end(tab: &mut DocTab, st: &mut TimelineState, copy: bool) {
    let Some(press) = st.press.take() else { return };
    let (dl, df) = press.delta;
    let start = press.start;
    if (dl, df) == (0, 0) {
        if press.rect.is_multi() {
            st.range = None;
            tab.layer = start.0;
            tab.frame = start.1;
            st.anchor = Some(start);
        }
        return;
    }
    let r = press.rect;
    if !tab.doc.move_cels((r.l0, r.l1), (r.f0, r.f1), dl, df, copy) {
        return;
    }
    playback::stop(tab);
    tab.changed();
    let moved = r.shifted(dl, df);
    st.range = r.is_multi().then_some(moved);
    let target = ((start.0 as i64 + dl as i64).max(0) as u32, (start.1 as i64 + df as i64).max(0) as u32);
    st.anchor = Some(target);
    if !tab.doc.layer_is_group(target.0) {
        tab.layer = target.0;
    }
    tab.frame = target.1;
}
