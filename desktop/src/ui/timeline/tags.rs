//! Tags: the model, the greedy row layout and the bar painting (`TagsView.swift`).

use crate::doc::Doc;
use crate::theme::{Colors, radius};
use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Vec2};

/// Geometry shared with the frame strip so tag bars line up with the frames below them (`enum Timeline`).
pub const CELL_WIDTH: f32 = 32.0;
pub const BAR_HEIGHT: f32 = 14.0;
/// Vertical space between stacked tag rows.
pub const ROW_GAP: f32 = 2.0;
/// Height of the frame number row.
pub const NUMBER_ROW: f32 = 20.0;

/// One animation tag; `color` is 0xRRGGBB.
#[derive(Clone, Debug, PartialEq)]
pub struct TagInfo {
    pub name: String,
    pub from: u32,
    pub to: u32,
    pub direction: u8,
    pub repeat: u32,
    pub color: u32,
}

pub const DIRECTIONS: [&str; 4] = ["Forward", "Reverse", "Ping-pong", "Ping-pong reverse"];

/// Default values for a new tag over `frame` (`newTagDraft`).
pub fn new_tag_draft(doc: &Doc, frame: u32) -> TagInfo {
    const COLORS: [u32; 6] = [0x3B82F6, 0xEF4444, 0x22C55E, 0xEAB308, 0xA855F7, 0xF97316];
    let n = doc.tag_count();
    TagInfo {
        name: format!("Tag {}", n + 1),
        from: frame,
        to: frame,
        direction: 0,
        repeat: 0,
        color: COLORS[n as usize % COLORS.len()],
    }
}

pub fn color32(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Blends toward `other` by `t` in sRGB (`Color.mix`).
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Greedy row assignment: a tag goes into the first row whose last tag ends before it starts.
pub struct TagLayout {
    /// (tag index, tag, row)
    pub items: Vec<(u32, TagInfo, usize)>,
    pub rows: usize,
}

impl TagLayout {
    pub fn new(doc: &Doc) -> TagLayout {
        let mut tags: Vec<(u32, TagInfo)> = (0..doc.tag_count()).filter_map(|i| doc.tag(i).map(|t| (i, t))).collect();
        tags.sort_by_key(|(_, t)| t.from);
        let mut ends: Vec<u32> = Vec::new();
        let mut items = Vec::new();
        for (i, t) in tags {
            if let Some(r) = ends.iter().position(|&e| e < t.from) {
                ends[r] = t.to;
                items.push((i, t, r));
            } else {
                ends.push(t.to);
                items.push((i, t, ends.len() - 1));
            }
        }
        TagLayout { items, rows: ends.len() }
    }

    /// Height of the tag area above the frame numbers; always at least one row so the header does not jump.
    pub fn height(&self) -> f32 {
        self.rows.max(1) as f32 * (BAR_HEIGHT + ROW_GAP) + 4.0
    }
}

/// Top-left of a tag bar relative to the start of the frame strip.
pub fn bar_rect(origin: Pos2, tag: &TagInfo, row: usize) -> Rect {
    let w = (tag.to.saturating_sub(tag.from) + 1) as f32 * CELL_WIDTH - 2.0;
    Rect::from_min_size(
        origin + Vec2::new(tag.from as f32 * CELL_WIDTH + 1.0, 4.0 + row as f32 * (BAR_HEIGHT + ROW_GAP)),
        Vec2::new(w, BAR_HEIGHT),
    )
}

/// Paints one bar (`TagBar`): the color at 28% (42% when the current frame is inside) with its name.
pub fn paint_bar(painter: &egui::Painter, rect: Rect, tag: &TagInfo, selected: bool, dark: bool, c: &Colors) {
    let _ = c;
    let base = color32(tag.color);
    painter.rect_filled(
        rect,
        CornerRadius::same(radius::CHIP),
        base.gamma_multiply(if selected { 0.42 } else { 0.28 }),
    );
    let text = if dark { mix(base, Color32::WHITE, 0.6) } else { mix(base, Color32::BLACK, 0.45) };
    let clipped = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    clipped.text(
        egui::pos2(rect.left() + 6.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &tag.name,
        crate::ui::widgets::strong(painter.ctx(), 10.0),
        text,
    );
}
