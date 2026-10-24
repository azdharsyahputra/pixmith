//! Drawing shared by the Home screen, the project view and the project switcher: the grid geometry, thumbnail
//! boxes on a checkerboard, ellipsized labels and relative dates.

use super::jobs::{ThumbKey, ThumbState, Thumbs};
use crate::theme::{Colors, font, radius, ui_font};
use crate::ui::widgets;
use eframe::egui::emath::GuiRounding;
use eframe::egui::{self, Align2, Color32, Mesh, Painter, Rect, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as icon;
use std::sync::Arc;

/// Narrowest a card gets before the grid drops a column.
pub const MIN_CARD_WIDTH: f32 = 200.0;
/// Space between cards, both ways.
pub const GAP: f32 = 20.0;
/// Height of the name and details under a thumbnail.
pub const TEXT_HEIGHT: f32 = 40.0;
/// Gap between a thumbnail and its name.
pub const TEXT_GAP: f32 = 8.0;

/// How a row of cards is laid out in `width` points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub cols: usize,
    pub card_w: f32,
    pub thumb_h: f32,
    pub row_h: f32,
}

impl Grid {
    pub fn new(width: f32) -> Grid {
        let cols = (((width + GAP) / (MIN_CARD_WIDTH + GAP)).floor() as usize).max(1);
        let card_w = ((width - GAP * (cols - 1) as f32) / cols as f32).floor().max(120.0);
        let thumb_h = (card_w * 0.7).round();
        Grid { cols, card_w, thumb_h, row_h: thumb_h + TEXT_GAP + TEXT_HEIGHT + GAP }
    }

    /// The thumbnail box of item `i` when the grid starts at `origin`.
    pub fn thumb_rect(&self, origin: egui::Pos2, i: usize) -> Rect {
        let (row, col) = (i / self.cols, i % self.cols);
        Rect::from_min_size(
            origin + vec2(col as f32 * (self.card_w + GAP), row as f32 * self.row_h),
            vec2(self.card_w, self.thumb_h),
        )
    }

    /// The whole card (thumbnail and text) of item `i`.
    pub fn card_rect(&self, origin: egui::Pos2, i: usize) -> Rect {
        let t = self.thumb_rect(origin, i);
        Rect::from_min_max(t.min, pos2(t.right(), t.bottom() + TEXT_GAP + TEXT_HEIGHT))
    }

    pub fn rows(&self, items: usize) -> usize {
        items.div_ceil(self.cols)
    }

    /// Height of `items` cards, without the trailing gap.
    pub fn height(&self, items: usize) -> f32 {
        if items == 0 { 0.0 } else { self.rows(items) as f32 * self.row_h - GAP }
    }

    /// The range of item indexes whose cards may touch `visible` (a rect in the same space as `origin`).
    pub fn visible(&self, origin: egui::Pos2, visible: Rect, items: usize) -> std::ops::Range<usize> {
        let first_row = ((visible.top() - origin.y) / self.row_h).floor().max(0.0) as usize;
        let last_row = ((visible.bottom() - origin.y) / self.row_h).floor().max(0.0) as usize;
        let start = (first_row * self.cols).min(items);
        let end = ((last_row + 1) * self.cols).min(items);
        start..end
    }
}

/// A layout of `text` on one row, cut with an ellipsis when wider than `max_width`.
pub fn fit_text(ui: &Ui, text: &str, font: egui::FontId, color: Color32, max_width: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap =
        egui::text::TextWrapping { max_width, max_rows: 1, break_anywhere: true, overflow_character: Some('\u{2026}') };
    ui.painter().layout_job(job)
}

/// The name line and the details line under a thumbnail.
pub fn caption(ui: &Ui, c: &Colors, card: Rect, thumb_bottom: f32, name: &str, details: &str, dim: bool) {
    let x = card.left() + 2.0;
    let width = card.width() - 4.0;
    let name_color = if dim { c.text_secondary } else { c.text };
    let galley = fit_text(ui, name, widgets::strong(ui.ctx(), font::BODY_LARGE), name_color, width);
    ui.painter().galley(pos2(x, thumb_bottom + TEXT_GAP), galley, name_color);
    let galley = fit_text(ui, details, ui_font(font::CAPTION), c.text_secondary, width);
    ui.painter().galley(pos2(x, thumb_bottom + TEXT_GAP + 20.0), galley, c.text_secondary);
}

/// The canvas checkerboard in the editor's default colors, clipped to a rounded rectangle.
pub fn paint_checker(painter: &Painter, rect: Rect, dark_theme: bool, r: f32) {
    let (light, dark) = crate::ui::menus::prefs::default_checker(dark_theme);
    let (light, dark) = (widgets::rgba_color(light), widgets::rgba_color(dark));
    painter.rect_filled(rect, r, light);
    let cell = 10.0;
    let poly = widgets::rounded_rect_polygon(rect, r);
    let mut mesh = Mesh::default();
    let cols = (rect.width() / cell).ceil() as i32;
    let rows = (rect.height() / cell).ceil() as i32;
    for row in 0..rows {
        for col in 0..cols {
            if (row + col) % 2 == 0 {
                continue;
            }
            let cell_rect =
                Rect::from_min_size(rect.min + vec2(col as f32 * cell, row as f32 * cell), vec2(cell, cell))
                    .intersect(rect);
            let in_corner = (cell_rect.left() < rect.left() + r || cell_rect.right() > rect.right() - r)
                && (cell_rect.top() < rect.top() + r || cell_rect.bottom() > rect.bottom() - r);
            if !in_corner {
                mesh.add_colored_rect(cell_rect, dark);
                continue;
            }
            let quad = [cell_rect.left_top(), cell_rect.right_top(), cell_rect.right_bottom(), cell_rect.left_bottom()];
            let clipped = widgets::clip_convex(&quad, &poly);
            if clipped.len() >= 3 {
                let base = mesh.vertices.len() as u32;
                for p in &clipped {
                    mesh.colored_vertex(*p, dark);
                }
                for k in 1..clipped.len() as u32 - 1 {
                    mesh.add_triangle(base, base + k, base + k + 1);
                }
            }
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// A thumbnail box: checkerboard, the preview scaled in whole steps when it can (so pixel art stays sharp),
/// a hairline border. `state` `None` draws the empty box.
pub fn paint_thumb(ui: &Ui, rect: Rect, state: Option<&ThumbState>, c: &Colors, dark_theme: bool) {
    let r = radius::PANEL as f32;
    paint_checker(ui.painter(), rect, dark_theme, r);
    match state {
        Some(ThumbState::Ready { texture, width, height }) => {
            let pad = 8.0;
            let fit = ((rect.width() - 2.0 * pad) / *width as f32).min((rect.height() - 2.0 * pad) / *height as f32);
            let scale = if fit >= 1.0 { fit.floor() } else { fit.max(0.01) };
            let size = vec2(*width as f32 * scale, *height as f32 * scale);
            let ppp = ui.ctx().pixels_per_point();
            let dest = Rect::from_center_size(rect.center(), size).round_to_pixels(ppp);
            ui.painter().image(texture.id(), dest, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        Some(ThumbState::Failed) => {
            ui.painter().rect_filled(rect, r, c.panel.gamma_multiply(0.85));
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                icon::WARNING,
                egui::FontId::proportional(22.0),
                c.text_tertiary,
            );
        }
        Some(ThumbState::Loading) | None => {}
    }
    ui.painter().rect_stroke(rect, r, Stroke::new(1.0, c.separator), StrokeKind::Inside);
}

/// Draws the thumbnail of `key`'s file (starting its load) in `rect`.
pub fn thumb_for(ui: &Ui, thumbs: &mut Thumbs, key: &ThumbKey, rect: Rect, c: &Colors, dark_theme: bool) {
    let state = thumbs.get(ui.ctx(), key);
    paint_thumb(ui, rect, Some(state), c, dark_theme);
}

/// The selection ring or hover outline around a thumbnail.
pub fn ring(ui: &Ui, rect: Rect, c: &Colors, selected: bool, hovered: bool) {
    let r = radius::PANEL as f32;
    if selected {
        ui.painter().rect_stroke(rect.expand(3.0), r + 3.0, Stroke::new(2.0, c.accent), StrokeKind::Middle);
    } else if hovered {
        ui.painter().rect_stroke(rect.expand(2.0), r + 2.0, Stroke::new(1.5, c.accent_line), StrokeKind::Middle);
    }
}

/// "just now", "5 min ago", "3 h ago", "Yesterday", "4 d ago", else the date.
pub fn ago(secs: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    ago_at(secs, now)
}

pub fn ago_at(secs: u64, now: u64) -> String {
    let d = now.saturating_sub(secs);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86_399 => format!("{} h ago", d / 3600),
        86_400..=172_799 => "Yesterday".into(),
        172_800..=604_799 => format!("{} d ago", d / 86_400),
        _ => {
            let (y, m, day) = crate::ui::gamedev::autosave::civil_from_days((secs / 86_400) as i64);
            const MONTHS: [&str; 12] =
                ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            format!("{} {day}, {y}", MONTHS[m as usize - 1])
        }
    }
}

/// "1 canvas", "12 canvases", "Empty".
pub fn canvas_count(n: usize) -> String {
    match n {
        0 => "Empty".into(),
        1 => "1 canvas".into(),
        n => format!("{n} canvases"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_follow_the_width() {
        assert_eq!(Grid::new(300.0).cols, 1);
        assert_eq!(Grid::new(440.0).cols, 2);
        let g = Grid::new(1000.0);
        assert_eq!(g.cols, 4);
        assert!(g.card_w * 4.0 + GAP * 3.0 <= 1000.0 + 0.5);
    }

    #[test]
    fn only_the_visible_rows_are_asked_for() {
        let g = Grid::new(1000.0);
        let origin = pos2(0.0, 0.0);
        let view = Rect::from_min_size(pos2(0.0, 3.0 * g.row_h + 5.0), vec2(1000.0, 2.0 * g.row_h));
        let range = g.visible(origin, view, 1000);
        assert_eq!(range, 12..24);
        assert_eq!(g.visible(origin, view, 14), 12..14);
        assert_eq!(g.visible(origin, view, 0), 0..0);
    }

    #[test]
    fn dates_read_naturally() {
        let now = 1_800_000_000;
        assert_eq!(ago_at(now - 5, now), "just now");
        assert_eq!(ago_at(now - 300, now), "5 min ago");
        assert_eq!(ago_at(now - 7200, now), "2 h ago");
        assert_eq!(ago_at(now - 90_000, now), "Yesterday");
        assert_eq!(ago_at(now - 3 * 86_400, now), "3 d ago");
        assert_eq!(ago_at(1_729_700_000, now), "Oct 23, 2024");
        assert_eq!(canvas_count(0), "Empty");
        assert_eq!(canvas_count(1), "1 canvas");
        assert_eq!(canvas_count(3), "3 canvases");
    }
}
