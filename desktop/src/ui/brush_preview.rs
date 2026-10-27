//! The brush cursor preview: the pixels a click would paint, drawn under the pointer (`BrushPreview.swift`).
//!
//! The footprint follows the core's brush rule (`Painter::dot`, `retouch::brush_cells`): a square or round brush of
//! the current size, or the pixels of a custom brush, centered on the pixel under the pointer, plus the mirror
//! images the symmetry toggles add, clipped to the sprite. It is snapped to whole sprite pixels; the outline is one
//! screen pixel wide at any zoom.

use crate::app::{DocTab, Tool};
use crate::ui::canvas::{CanvasState, flag};
use crate::ui::tilemap::TilemapState;
use eframe::egui::{self, Color32, Key, Mesh, Painter, PointerButton, Pos2, Rect, Shape, Stroke, pos2};
use pixmith_core::paint::ink;
use std::collections::HashSet;

/// Opacity of the color fill, out of 255 (about 60%).
const FILL_ALPHA: u32 = 153;
/// Below this zoom the inner (light) half of the outline would cover the whole pixel, so only the dark half shows.
const INNER_MIN_ZOOM: f32 = 3.0;

/// How the footprint is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// The current color at about 60% opacity, with an outline.
    Filled,
    /// The outline only.
    Outline,
    /// The disc the spray scatters dots in, as an outline.
    Spray,
}

/// The tools that show a preview, and how.
pub fn look(tool: Tool) -> Option<Look> {
    match tool {
        Tool::Pencil
        | Tool::Line
        | Tool::Rectangle
        | Tool::Ellipse
        | Tool::Curve
        | Tool::ShapePolygon
        | Tool::Contour => Some(Look::Filled),
        Tool::Eraser | Tool::Blur | Tool::Jumble => Some(Look::Outline),
        Tool::Spray => Some(Look::Spray),
        _ => None,
    }
}

/// Pixels the brush centered on `p` covers, unclipped and unmirrored. Reads the global brush of the core.
pub fn brush_cells(size: u32, p: (i32, i32)) -> Vec<(i32, i32)> {
    let settings = ink::settings();
    if let Some(b) = &settings.custom {
        let (ox, oy) = (p.0 - b.w / 2, p.1 - b.h / 2);
        return b.cache().pixels.iter().map(|&(x, y, _)| (ox + x, oy + y)).collect();
    }
    let size = size.clamp(1, 64) as i32;
    let half = (size - 1) / 2;
    let mut out = Vec::with_capacity((size * size) as usize);
    for row in 0..size {
        // A round brush trims `l` pixels from both ends of each row.
        let l = if settings.circle && size > 2 {
            let r = size as f64 / 2.0;
            let dy = row as f64 + 0.5 - r;
            ((r - (r * r - dy * dy).max(0.0).sqrt() - 0.5 - 1e-9).ceil() as i32).clamp(0, size - 1)
        } else {
            0
        };
        for col in l..size - l {
            out.push((p.0 - half + col, p.1 - half + row));
        }
    }
    out
}

/// The points `p` and its mirror images for the symmetry flags of the drawing calls. `axes` are twice the
/// positions of the mirror guides (`DocTab::mirror_x2`, `mirror_y2`): a pixel `x` has its image at `axes.0 - 1 - x`.
fn mirrors(p: (i32, i32), axes: (i32, i32), sym: u32) -> Vec<(i32, i32)> {
    let (ax, ay) = axes;
    let mut out = vec![p];
    if sym & flag::MIRROR_X != 0 {
        out.push((ax - 1 - p.0, p.1));
    }
    if sym & flag::MIRROR_Y != 0 {
        out.push((p.0, ay - 1 - p.1));
    }
    if sym & flag::MIRROR_X != 0 && sym & flag::MIRROR_Y != 0 {
        out.push((ax - 1 - p.0, ay - 1 - p.1));
    }
    out
}

/// Pixels of the `dims` sprite a click at `p` paints with a brush of `size` and the symmetry flags `sym`.
pub fn painted_cells(size: u32, p: (i32, i32), dims: (u32, u32), axes: (i32, i32), sym: u32) -> HashSet<(i32, i32)> {
    let (w, h) = (dims.0 as i32, dims.1 as i32);
    let mut out = HashSet::new();
    for c in brush_cells(size, p) {
        for (x, y) in mirrors(c, axes, sym) {
            if (0..w).contains(&x) && (0..h).contains(&y) {
                out.insert((x, y));
            }
        }
    }
    out
}

/// A stretch of the footprint's boundary on one grid line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Edge {
    horizontal: bool,
    /// Which way the outside lies across the line, -1 or 1.
    out: i32,
    /// The grid line, in pixels along the axis the edge does not run on.
    line: i32,
    from: i32,
    to: i32,
}

/// The boundary of `cells`, with neighboring unit edges joined.
fn edges(cells: &HashSet<(i32, i32)>) -> Vec<Edge> {
    let mut all = Vec::new();
    for &(x, y) in cells {
        let mut push = |horizontal, out, line, from| all.push(Edge { horizontal, out, line, from, to: from + 1 });
        if !cells.contains(&(x, y - 1)) {
            push(true, -1, y, x);
        }
        if !cells.contains(&(x, y + 1)) {
            push(true, 1, y + 1, x);
        }
        if !cells.contains(&(x - 1, y)) {
            push(false, -1, x, y);
        }
        if !cells.contains(&(x + 1, y)) {
            push(false, 1, x + 1, y);
        }
    }
    all.sort_unstable();
    let mut out: Vec<Edge> = Vec::with_capacity(all.len());
    for e in all {
        match out.last_mut() {
            Some(l) if (l.horizontal, l.out, l.line) == (e.horizontal, e.out, e.line) && l.to == e.from => l.to = e.to,
            _ => out.push(e),
        }
    }
    out
}

/// What the preview needs to know about this frame.
pub struct Args<'a> {
    pub state: &'a CanvasState,
    pub tilemap: &'a TilemapState,
    pub tab: &'a DocTab,
    pub tool: Tool,
    /// The primary color, `0xRRGGBBAA`.
    pub color: u32,
    pub size: u32,
    pub srect: Rect,
    pub ppp: f32,
    /// The pointer is over the canvas.
    pub hovered: bool,
    /// The selection tools, a transform or warp, or the tween overlay have the pointer.
    pub owned: bool,
    pub pointer: Option<Pos2>,
}

/// Whether a tilemap mode takes the painting tools' clicks (`tilemap::canvas_interact`).
fn tilemap_owns(tm: &TilemapState, tab: &DocTab, tool: Tool) -> bool {
    let layer = tab.layer;
    tab.doc.layer_is_tilemap(layer)
        && ((tm.brush && tab.doc.terrain_mode(layer) >= 0) || (tm.stamp && matches!(tool, Tool::Pencil | Tool::Eraser)))
}

/// The pixel under the pointer and how to draw the preview there, or `None` while it is hidden.
fn target(a: &Args, ctx: &egui::Context) -> Option<(Look, (i32, i32))> {
    let look = look(a.tool)?;
    let pos = a.pointer.filter(|_| a.hovered && !a.owned)?;
    let (space, alt, middle) =
        ctx.input(|i| (i.key_down(Key::Space), i.modifiers.alt, i.pointer.button_down(PointerButton::Middle)));
    // Panning, Alt (which picks a color) and the shapes while they are dragged or half drawn.
    if space || alt || middle || a.state.panning() || a.state.tools.draw.session.is_some() {
        return None;
    }
    if a.state.gesture_tool().is_some_and(is_shape) || tilemap_owns(a.tilemap, a.tab, a.tool) {
        return None;
    }
    let p = (pos - a.srect.min) / a.tab.zoom;
    Some((look, (p.x.floor() as i32, p.y.floor() as i32)))
}

/// The tools whose drag draws a shape, which hides the preview.
fn is_shape(tool: Tool) -> bool {
    matches!(tool, Tool::Line | Tool::Rectangle | Tool::Ellipse | Tool::Curve | Tool::ShapePolygon | Tool::Contour)
}

/// Draws the preview over the sprite.
pub fn paint(painter: &Painter, a: &Args) {
    let Some((look, p)) = target(a, painter.ctx()) else { return };
    let dims = (a.tab.doc.width(), a.tab.doc.height());
    let axes = (a.tab.mirror_x2(), a.tab.mirror_y2());
    let sym = a.state.opts.sym_flags();
    if look == Look::Spray {
        return paint_spray(painter, a, p, axes, sym);
    }
    let cells = painted_cells(a.size, p, dims, axes, sym);
    if cells.is_empty() {
        return;
    }
    // A custom brush shows the outline of its pixels only.
    let look = if ink::settings().custom.is_some() { Look::Outline } else { look };

    let (z, ppp) = (a.tab.zoom, a.ppp);
    let snap = |v: f32| (v * ppp).round() / ppp;
    let xs = |x: i32| snap(a.srect.left() + x as f32 * z);
    let ys = |y: i32| snap(a.srect.top() + y as f32 * z);
    let mut mesh = Mesh::default();
    fn quad(mesh: &mut Mesh, x0: f32, y0: f32, x1: f32, y1: f32, c: Color32) {
        mesh.add_colored_rect(Rect::from_min_max(pos2(x0, y0), pos2(x1, y1)), c);
    }

    if look == Look::Filled {
        let fill = fill_color(a.color);
        let mut sorted: Vec<(i32, i32)> = cells.iter().map(|&(x, y)| (y, x)).collect();
        sorted.sort_unstable();
        let mut i = 0;
        while i < sorted.len() {
            let (y, x0) = sorted[i];
            let mut x1 = x0;
            while i + 1 < sorted.len() && sorted[i + 1] == (y, x1 + 1) {
                x1 += 1;
                i += 1;
            }
            quad(&mut mesh, xs(x0), ys(y), xs(x1 + 1), ys(y + 1), fill);
            i += 1;
        }
    }

    // Dark just outside the footprint, light just inside it: readable on any color.
    let t = 1.0 / ppp;
    let inner = z >= INNER_MIN_ZOOM;
    for e in edges(&cells) {
        let o = e.out as f32;
        if e.horizontal {
            let (y, x0, x1) = (ys(e.line), xs(e.from), xs(e.to));
            let (lo, hi) = if o < 0.0 { (y - t, y) } else { (y, y + t) };
            quad(&mut mesh, x0 - t, lo, x1 + t, hi, Color32::BLACK);
            if inner {
                let (lo, hi) = if o < 0.0 { (y, y + t) } else { (y - t, y) };
                quad(&mut mesh, x0, lo, x1, hi, Color32::WHITE);
            }
        } else {
            let (x, y0, y1) = (xs(e.line), ys(e.from), ys(e.to));
            let (lo, hi) = if o < 0.0 { (x - t, x) } else { (x, x + t) };
            quad(&mut mesh, lo, y0 - t, hi, y1 + t, Color32::BLACK);
            if inner {
                let (lo, hi) = if o < 0.0 { (x, x + t) } else { (x - t, x) };
                quad(&mut mesh, lo, y0, hi, y1, Color32::WHITE);
            }
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// The spray's reach as an outline around each mirror image of the pointer pixel.
fn paint_spray(painter: &Painter, a: &Args, p: (i32, i32), axes: (i32, i32), sym: u32) {
    let radius = (a.size * 3).max(3).clamp(1, 256) as f32 + 0.5;
    let t = 1.0 / a.ppp;
    let z = a.tab.zoom;
    let mut seen = HashSet::new();
    for c in mirrors(p, axes, sym) {
        if !seen.insert(c) {
            continue;
        }
        let center = pos2(a.srect.left() + (c.0 as f32 + 0.5) * z, a.srect.top() + (c.1 as f32 + 0.5) * z);
        painter.circle_stroke(center, radius * z + t * 0.5, Stroke::new(t, Color32::BLACK));
        painter.circle_stroke(center, radius * z - t * 0.5, Stroke::new(t, Color32::WHITE));
    }
}

/// The primary color at the preview's opacity.
fn fill_color(color: u32) -> Color32 {
    let alpha = (color & 0xFF) * FILL_ALPHA / 255;
    Color32::from_rgba_unmultiplied((color >> 24) as u8, (color >> 16) as u8, (color >> 8) as u8, alpha as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outline_of_a_block_is_four_joined_edges() {
        let cells: HashSet<(i32, i32)> = (0..3).flat_map(|x| (0..2).map(move |y| (x, y))).collect();
        let mut e = edges(&cells);
        e.sort();
        assert_eq!(e.len(), 4);
        assert!(e.contains(&Edge { horizontal: true, out: -1, line: 0, from: 0, to: 3 }));
        assert!(e.contains(&Edge { horizontal: true, out: 1, line: 2, from: 0, to: 3 }));
        assert!(e.contains(&Edge { horizontal: false, out: -1, line: 0, from: 0, to: 2 }));
        assert!(e.contains(&Edge { horizontal: false, out: 1, line: 3, from: 0, to: 2 }));
    }
}
