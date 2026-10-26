//! Canvas input and overlays for the terrain brush, the tile stamp and the slice tool
//! (`handleTerrainMouse`, `handleTileMouse`, `drawTileGrid`, `drawMissingCells`, `drawSlices`).

use super::controls::{system_green, system_orange};
use super::{Drag, DragKind, RectDrag, TerrainTool, TilemapState, slices};
use crate::app::{DocTab, Tool};
use crate::theme::Colors;
use crate::ui::iso::{self, Iso};
use eframe::egui::{self, Color32, Painter, PointerButton, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};

/// What the canvas knows about the pointer this frame.
pub struct Pointer {
    pub pos: Option<Pos2>,
    /// Sprite pixel under the pointer.
    pub pixel: Option<(i32, i32)>,
    pub hovered: bool,
    /// Another gesture (pan, a drawing tool) owns the pointer.
    pub busy: bool,
    /// Where the sprite sits on screen, and points per sprite pixel.
    pub srect: Rect,
    pub zoom: f32,
    /// Mirror flags of the drawing options: bit 0 horizontal, bit 1 vertical.
    pub mirror: u32,
}

fn in_bounds(tab: &DocTab, p: (i32, i32)) -> bool {
    p.0 >= 0 && p.1 >= 0 && (p.0 as u32) < tab.doc.width() && (p.1 as u32) < tab.doc.height()
}

fn begin_edit(tab: &mut DocTab, label: &str) {
    tab.playing = false;
    tab.doc.begin_edit(label);
}

/// Mouse input for the terrain brush, the tile stamp and the slice tool, in the order of `CanvasNSView.mouseDown`.
/// Returns true when the event is theirs, so the canvas skips the drawing tools.
pub fn canvas_interact(
    tm: &mut TilemapState,
    tab: &mut DocTab,
    tool: Tool,
    ptr: &Pointer,
    ctx: &egui::Context,
) -> bool {
    tm.refresh_cache(tab);
    let doc_id = tab.doc.raw() as usize;
    if tm.drag.is_some_and(|d| d.doc != doc_id) {
        tm.drag = None;
    }
    if let Some(d) = tm.drag {
        return drag_step(tm, tab, ptr, ctx, d);
    }
    if ptr.busy || !ptr.hovered {
        return false;
    }
    let (pressed_primary, pressed_secondary) = ctx.input(|i| {
        (i.pointer.button_pressed(PointerButton::Primary), i.pointer.button_pressed(PointerButton::Secondary))
    });
    if !(pressed_primary || pressed_secondary) {
        return false;
    }
    let button = if pressed_secondary { PointerButton::Secondary } else { PointerButton::Primary };
    let mods = ctx.input(|i| i.modifiers);
    let layer = tab.layer;
    let on_tilemap = tab.doc.layer_is_tilemap(layer);
    let terrain = tm.brush && on_tilemap && tab.doc.terrain_mode(layer) >= 0;
    let stamp = tm.stamp && on_tilemap && matches!(tool, Tool::Pencil | Tool::Eraser);
    let slice = tool == Tool::Slice && !mods.alt;
    if !(terrain || stamp || slice) {
        return false;
    }
    let Some(p) = ptr.pixel.filter(|p| in_bounds(tab, *p)) else { return true };
    let mut drag = Drag { kind: DragKind::Terrain, button, last: p, doc: doc_id };
    if terrain {
        terrain_event(tm, tab, ptr, button, mods.shift, p, true);
    } else if stamp {
        let mut open = false;
        stamp_event(tm, tab, p, mods, &mut open);
        drag.kind = DragKind::Stamp { edit_open: open };
    } else {
        slices::press(tm, tab, ptr, p);
        drag.kind = DragKind::Slice;
    }
    tm.drag = Some(drag);
    true
}

fn drag_step(tm: &mut TilemapState, tab: &mut DocTab, ptr: &Pointer, ctx: &egui::Context, mut d: Drag) -> bool {
    let (down, mods) = ctx.input(|i| (i.pointer.button_down(d.button), i.modifiers));
    if !down {
        match d.kind {
            DragKind::Terrain => finish_terrain(tm, tab, ptr),
            DragKind::Slice => {
                let double = ctx.input(|i| i.pointer.button_double_clicked(d.button));
                slices::release(tm, tab, double);
            }
            DragKind::Stamp { .. } => {}
        }
        tm.drag = None;
        return true;
    }
    let Some(p) = ptr.pixel else { return true };
    match d.kind {
        DragKind::Terrain => {
            if p != d.last {
                terrain_event(tm, tab, ptr, d.button, mods.shift, p, false);
            }
        }
        DragKind::Stamp { mut edit_open } => {
            if p != d.last {
                stamp_event(tm, tab, p, mods, &mut edit_open);
                d.kind = DragKind::Stamp { edit_open };
            }
        }
        DragKind::Slice => slices::drag(tm, tab, p),
    }
    d.last = p;
    tm.drag = Some(d);
    true
}

fn changed(tab: &mut DocTab) {
    tab.changed();
}

/// Plain click places the selected tile, Shift erases, Option picks the tile under the cursor.
fn stamp_event(tm: &mut TilemapState, tab: &mut DocTab, p: (i32, i32), mods: egui::Modifiers, edit_open: &mut bool) {
    let (layer, frame) = (tab.layer, tab.frame);
    if mods.alt {
        // Isometric layers pick the tile you see, which for a tall tile may belong to a cell behind the diamond.
        let v = if tab.doc.layer_is_iso(layer) {
            tab.doc.tile_hit_at(layer, frame, p.0, p.1).map_or(0, |(_, v)| v)
        } else {
            tab.doc.tile_value_at(layer, frame, p.0, p.1)
        };
        if v & 0x1FFF_FFFF != 0 {
            tm.select_value(v);
        }
        return;
    }
    if !*edit_open {
        begin_edit(tab, "Stamp Tile");
        *edit_open = true;
    }
    let value = if mods.shift { 0 } else { tm.value() };
    if tab.doc.stamp_tile(layer, frame, p.0, p.1, value) {
        changed(tab);
    }
}

/// One mouse event of the terrain brush. Left click paints, Shift or right click erases.
fn terrain_event(
    tm: &mut TilemapState,
    tab: &mut DocTab,
    ptr: &Pointer,
    button: PointerButton,
    shift: bool,
    p: (i32, i32),
    began: bool,
) {
    let erase = shift || button == PointerButton::Secondary;
    let t = if erase { 0 } else { tm.terrain };
    let (layer, frame) = (tab.layer, tab.frame);
    let mut dirty = false;
    match tm.terrain_tool {
        TerrainTool::Pick => {
            if began {
                let id = tab.doc.terrain_at(layer, frame, p.0, p.1);
                if id != 0 {
                    tm.terrain = id;
                }
            }
        }
        TerrainTool::Rect => {
            if began {
                tm.rect = Some(RectDrag { start: p, end: p, erase });
            } else if let Some(r) = &mut tm.rect {
                r.end = p;
            }
        }
        TerrainTool::Fill => {
            if began {
                begin_edit(tab, "Terrain Fill");
                dirty = tab.doc.flood_terrain(layer, frame, p.0, p.1, t, ptr.mirror);
            }
        }
        TerrainTool::Override | TerrainTool::Auto => {
            let (label, tile) =
                if tm.terrain_tool == TerrainTool::Override { ("Manual Tile", tm.value()) } else { ("Auto Tile", 0) };
            if began {
                begin_edit(tab, label);
            }
            dirty = tab.doc.set_tile_override(layer, frame, p.0, p.1, tile);
        }
        TerrainTool::Paint => {
            if began {
                begin_edit(tab, "Terrain Brush");
                tm.last_cell = None;
                tm.last_iso_cell = None;
            }
            let mut points = vec![p];
            if let Some(a) = tm.last_cell
                && (a.0 - p.0).abs() + (a.1 - p.1).abs() > 1
            {
                // Fill the gap of a fast drag so the stroke has no holes.
                let n = (a.0 - p.0).abs().max((a.1 - p.1).abs());
                points = (1..=n).map(|k| (a.0 + (p.0 - a.0) * k / n, a.1 + (p.1 - a.1) * k / n)).collect();
            }
            let iso = tab.doc.layer_is_iso(layer);
            for q in points {
                if iso {
                    // The brush works on cells: many pixels of a stroke share one diamond.
                    let cell = tab.doc.layer_cell_at(layer, q.0, q.1);
                    if cell == tm.last_iso_cell {
                        continue;
                    }
                    tm.last_iso_cell = cell;
                }
                dirty |= tab.doc.paint_terrain_brush(layer, frame, q.0, q.1, t, tm.brush_size, tm.round, ptr.mirror);
            }
            tm.last_cell = Some(p);
        }
    }
    if dirty {
        changed(tab);
    }
}

/// Commits a rectangle drag (`handleTerrainMouseUp`).
fn finish_terrain(tm: &mut TilemapState, tab: &mut DocTab, ptr: &Pointer) {
    if tm.terrain_tool != TerrainTool::Rect {
        return;
    }
    let Some(r) = tm.rect.take() else { return };
    begin_edit(tab, "Terrain Rectangle");
    let (layer, frame) = (tab.layer, tab.frame);
    let terrain = if r.erase { 0 } else { tm.terrain };
    if tab.doc.paint_terrain_rect(layer, frame, r.start, r.end, terrain, ptr.mirror) {
        changed(tab);
    }
}

// MARK: Overlays

/// Tile grid, terrain rectangle preview, missing-tile markers and slices, in sprite space scaled by the zoom
/// (`drawTileGrid`, `drawMissingCells`, `drawSlices`). `srect` is the sprite on screen; `cursor` is the sprite
/// pixel under the pointer.
pub fn canvas_paint(
    painter: &Painter,
    tm: &TilemapState,
    tab: &DocTab,
    srect: Rect,
    ppp: f32,
    dark: bool,
    cursor: Option<(i32, i32)>,
) {
    let c = if dark { Colors::dark() } else { Colors::light() };
    let layer = tab.layer;
    if let Some(ts) = tab.doc.tileset_of_layer(layer) {
        if let Some(grid) = super::active_iso(tab) {
            iso_overlays(painter, tm, tab, srect, ppp, dark, &grid, cursor);
        } else {
            let (tw, th) = tab.doc.tile_size(ts);
            tile_grid(painter, tab, srect, ppp, dark, (tw, th));
            terrain_overlay(painter, tm, tab, srect, dark, (tw, th));
        }
    }
    slices::paint(painter, tm, tab, srect, &c);
}

/// A diamond (or any convex polygon) in sprite pixel coordinates, filled and outlined.
fn poly(painter: &Painter, srect: Rect, zoom: f32, pts: &[(f64, f64)], fill: Color32, stroke: Stroke) {
    let screen: Vec<Pos2> = pts.iter().map(|&(x, y)| srect.min + vec2(x as f32, y as f32) * zoom).collect();
    painter.add(egui::Shape::convex_polygon(screen, fill, stroke));
}

/// The cells the active tool would change, in the isometric grid: the stamp's cell, the terrain brush's block, or
/// the cells of the rectangle being dragged.
fn tool_cells(tm: &TilemapState, tab: &DocTab, cursor: (i32, i32)) -> Vec<(i32, i32)> {
    let layer = tab.layer;
    let cell_of = |p: (i32, i32)| tab.doc.layer_cell_at(layer, p.0, p.1);
    let Some(cell) = cell_of(cursor) else { return Vec::new() };
    if tm.brush && tab.doc.terrain_mode(layer) >= 0 {
        return match tm.terrain_tool {
            TerrainTool::Paint => pixmith_core::gamedev::autotile::brush_cells(cell.0, cell.1, tm.brush_size, tm.round),
            TerrainTool::Rect => match tm.rect.and_then(|r| Some((cell_of(r.start)?, cell_of(r.end)?))) {
                Some((a, b)) => pixmith_core::gamedev::autotile::rect_cells(a.0, a.1, b.0, b.1),
                None => vec![cell],
            },
            _ => vec![cell],
        };
    }
    if tm.stamp { vec![cell] } else { Vec::new() }
}

/// Grid lines of the active isometric layer, the cells under the tool, and cells that use a fallback tile.
#[allow(clippy::too_many_arguments)]
fn iso_overlays(
    painter: &Painter,
    tm: &TilemapState,
    tab: &DocTab,
    srect: Rect,
    ppp: f32,
    dark: bool,
    grid: &Iso,
    cursor: Option<(i32, i32)>,
) {
    let z = tab.zoom;
    let vis = srect.intersect(painter.clip_rect());
    if vis.is_positive() && (grid.w as f32 * z >= 4.0) && (grid.h as f32 * z >= 4.0) {
        iso::paint(painter, grid, srect.min, z, ppp, vis, system_orange(dark).gamma_multiply(0.7));
    }
    let painter = painter.with_clip_rect(srect.intersect(painter.clip_rect()));
    let accent = system_green(dark);
    let stroke = Stroke::new(1.5, accent);
    if tm.brush
        && tm.terrain_tool == TerrainTool::Rect
        && let Some(r) = tm.rect
        && let (Some(a), Some(b)) =
            (tab.doc.layer_cell_at(tab.layer, r.start.0, r.start.1), tab.doc.layer_cell_at(tab.layer, r.end.0, r.end.1))
    {
        // A rectangle of cells is a diamond on screen: its corners are corners of the four extreme cells.
        let (i0, i1, j0, j1) = (a.0.min(b.0) as i64, a.0.max(b.0) as i64, a.1.min(b.1) as i64, a.1.max(b.1) as i64);
        let pts = [
            grid.cell_corners((i0, j0))[0],
            grid.cell_corners((i1, j0))[1],
            grid.cell_corners((i1, j1))[2],
            grid.cell_corners((i0, j1))[3],
        ];
        let fill = if r.erase { Color32::from_rgba_unmultiplied(255, 69, 58, 50) } else { accent.gamma_multiply(0.25) };
        poly(&painter, srect, z, &pts, fill, stroke);
    } else if let Some(cursor) = cursor.filter(|p| in_bounds(tab, *p)) {
        for cell in tool_cells(tm, tab, cursor) {
            let pts = grid.cell_corners((cell.0 as i64, cell.1 as i64));
            poly(&painter, srect, z, &pts, accent.gamma_multiply(0.22), stroke);
        }
    }
    if tab.doc.terrain_mode(tab.layer) >= 0 {
        let red = if dark { Color32::from_rgb(0xFF, 0x45, 0x3A) } else { Color32::from_rgb(0xFF, 0x3B, 0x30) };
        let stroke = Stroke::new(2.0, red);
        let to_screen = |x: f64, y: f64| srect.min + vec2(x as f32, y as f32) * z;
        for &(x, y) in &tm.cache.missing {
            let cell = grid.cell_at(x + grid.w as i32 / 2, y + grid.h as i32 / 2);
            let pts = grid.cell_corners(cell);
            poly(&painter, srect, z, &pts, Color32::TRANSPARENT, stroke);
            painter.line_segment([to_screen(pts[0].0, pts[0].1), to_screen(pts[2].0, pts[2].1)], stroke);
            painter.line_segment([to_screen(pts[1].0, pts[1].1), to_screen(pts[3].0, pts[3].1)], stroke);
        }
    }
}

fn tile_grid(painter: &Painter, tab: &DocTab, srect: Rect, ppp: f32, dark: bool, (tw, th): (u32, u32)) {
    let z = tab.zoom;
    let (sx, sy) = (tw as f32 * z, th as f32 * z);
    if tw == 0 || th == 0 || sx < 4.0 || sy < 4.0 {
        return;
    }
    let (ox, oy) = tab.doc.tile_grid_origin(tab.layer, tab.frame);
    let stroke = Stroke::new(1.0 / ppp, system_orange(dark).gamma_multiply(0.7));
    let painter = painter.with_clip_rect(srect.intersect(painter.clip_rect()));
    let mut x = ox as f32 * z;
    while x <= srect.width() + 0.5 {
        painter.vline(srect.left() + x, srect.y_range(), stroke);
        x += sx;
    }
    let mut y = oy as f32 * z;
    while y <= srect.height() + 0.5 {
        painter.hline(srect.x_range(), srect.top() + y, stroke);
        y += sy;
    }
}

fn terrain_overlay(painter: &Painter, tm: &TilemapState, tab: &DocTab, srect: Rect, dark: bool, (tw, th): (u32, u32)) {
    if tab.doc.terrain_mode(tab.layer) < 0 {
        return;
    }
    let z = tab.zoom;
    let to_screen = |x: f32, y: f32| srect.min + vec2(x, y) * z;
    if tm.brush
        && tm.terrain_tool == TerrainTool::Rect
        && let Some(r) = tm.rect
        && tw > 0
        && th > 0
    {
        let (tw, th) = (tw as i32, th as i32);
        let x0 = r.start.0.min(r.end.0).div_euclid(tw) * tw;
        let x1 = (r.start.0.max(r.end.0).div_euclid(tw) + 1) * tw;
        let y0 = r.start.1.min(r.end.1).div_euclid(th) * th;
        let y1 = (r.start.1.max(r.end.1).div_euclid(th) + 1) * th;
        let rect = Rect::from_min_max(to_screen(x0 as f32, y0 as f32), to_screen(x1 as f32, y1 as f32));
        painter.rect_filled(rect, 0.0, system_green(dark).gamma_multiply(0.25));
    }
    let red = if dark { Color32::from_rgb(0xFF, 0x45, 0x3A) } else { Color32::from_rgb(0xFF, 0x3B, 0x30) };
    for &(x, y) in &tm.cache.missing {
        let r = Rect::from_min_size(to_screen(x as f32, y as f32), vec2(tw as f32, th as f32) * z);
        painter.rect_stroke(r.shrink(1.0), 0.0, Stroke::new(2.0, red), StrokeKind::Middle);
        painter.line_segment([r.left_top(), r.right_bottom()], Stroke::new(2.0, red));
        painter.line_segment([r.right_top(), r.left_bottom()], Stroke::new(2.0, red));
    }
}

/// Whether `p` lies within the sprite on screen; used by tests.
#[allow(dead_code)]
pub(super) fn pixel_pos(srect: Rect, zoom: f32, p: (i32, i32)) -> Pos2 {
    pos2(srect.left() + (p.0 as f32 + 0.5) * zoom, srect.top() + (p.1 as f32 + 0.5) * zoom)
}
