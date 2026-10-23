//! Context bar options while the terrain brush or the tile stamp is on (`ToolOptionsBar`, `TerrainOptions`).

use super::{TerrainTool, TilemapState};
use crate::app::PixmithApp;
use crate::theme::{self, Colors, font, metric, radius, space};
use crate::ui::widgets::{self, SegGlyph};
use eframe::egui::{self, Sense, Ui, pos2, vec2};
use egui_phosphor::regular as icon;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Tool,
    Terrain,
    Stamp,
}

/// The terrain brush and tile stamp replace the drawing tool's options while they are on.
fn mode(app: &PixmithApp) -> Mode {
    let tab = app.tab();
    if !tab.doc.layer_is_tilemap(tab.layer) {
        return Mode::Tool;
    }
    if app.tilemap.brush && tab.doc.terrain_mode(tab.layer) >= 0 {
        return Mode::Terrain;
    }
    if app.tilemap.stamp { Mode::Stamp } else { Mode::Tool }
}

/// The bar's title when a tilemap mode is on ("Terrain Brush", "Tile Stamp"), else `None`.
pub fn bar_title(app: &PixmithApp) -> Option<&'static str> {
    match mode(app) {
        Mode::Terrain => Some("Terrain Brush"),
        Mode::Stamp => Some("Tile Stamp"),
        Mode::Tool => None,
    }
}

/// Draws the options of the terrain brush or the tile stamp. Returns false when neither is on, and the caller
/// draws the tool's own options.
pub fn bar_options(app: &mut PixmithApp, ui: &mut Ui) -> bool {
    let m = mode(app);
    if m == Mode::Tool {
        return false;
    }
    if m == Mode::Stamp {
        group(ui, |ui| {
            widgets::hint_label(ui, &["\u{21e7}"], "erase");
            widgets::hint_label(ui, &["\u{2325}"], "pick");
        });
    } else {
        terrain_options(app, ui);
    }
    true
}

/// Items of one option group, separated from the previous one by a hairline.
fn group(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM - 2.0;
        widgets::group_divider(ui);
        content(ui);
    });
}

fn terrain_options(app: &mut PixmithApp, ui: &mut Ui) {
    let PixmithApp { tabs, active, tilemap: tm, .. } = app;
    let tab = &tabs[*active];
    let layer = tab.layer;
    ui.spacing_mut().item_spacing.x = space::SM - 2.0;
    widgets::group_divider(ui);
    terrain_menu(ui, tab.doc.terrain_ids(layer), tm, |id| {
        (tab.doc.terrain_name(layer, id), tab.doc.terrain_color(layer, id))
    });
    widgets::group_divider(ui);
    let options: Vec<(SegGlyph, String)> =
        TerrainTool::ALL.iter().map(|t| (SegGlyph::Icon(t.icon()), format!("{}: {}", t.name(), t.help()))).collect();
    let refs: Vec<(SegGlyph, &str)> = options.iter().map(|(g, h)| (*g, h.as_str())).collect();
    let selected = TerrainTool::ALL.iter().position(|t| *t == tm.terrain_tool).unwrap_or(0);
    if let Some(i) = widgets::bar_segmented(ui, selected, &refs) {
        tm.terrain_tool = TerrainTool::ALL[i];
    }
    if tm.terrain_tool == TerrainTool::Paint {
        widgets::group_divider(ui);
        widgets::bar_number_field(ui, "Size", &mut tm.brush_size, (1, 16), "", 76.0, "Brush size in cells");
        if widgets::bar_chip_with(ui, tm.round, "Round brush", |p, rect, fg| {
            p.circle_filled(rect.center(), 6.0, fg);
        })
        .clicked()
        {
            tm.round = !tm.round;
        }
    }
    widgets::group_divider(ui);
    if widgets::bar_chip(ui, icon::HASH, tm.show_masks, "Show the mask number on each tile").clicked() {
        tm.show_masks = !tm.show_masks;
    }
    if matches!(tm.terrain_tool, TerrainTool::Paint | TerrainTool::Rect | TerrainTool::Fill) {
        widgets::hint_label(ui, &["\u{21e7}"], "erase");
    }
}

/// The terrain popup: a swatch, the name and a chevron on a field (`terrainMenu`).
fn terrain_menu(ui: &mut Ui, ids: Vec<u32>, tm: &mut TilemapState, info: impl Fn(u32) -> (String, u32)) {
    let c = Colors::of(ui.ctx());
    let current = if ids.contains(&tm.terrain) { tm.terrain } else { ids.first().copied().unwrap_or(0) };
    let (name, color) = info(current);
    let g = ui.painter().layout_no_wrap(name, theme::ui_font(font::BODY), c.text);
    let w = space::SM + 12.0 + space::XS + 2.0 + g.size().x + space::XS + 2.0 + 9.0 + space::SM;
    let (rect, response) = ui.allocate_exact_size(vec2(w, metric::CONTROL), Sense::click());
    ui.painter().rect_filled(rect, radius::FIELD, c.field);
    let swatch = egui::Rect::from_center_size(pos2(rect.left() + space::SM + 6.0, rect.center().y), vec2(12.0, 12.0));
    super::controls::terrain_swatch(ui.painter(), swatch, color);
    let text_x = swatch.right() + space::XS + 2.0;
    ui.painter().galley(pos2(text_x, rect.center().y - g.size().y / 2.0), g.clone(), c.text);
    let (cx, cy) = (rect.right() - space::SM - 4.5, rect.center().y);
    let stroke = egui::Stroke::new(1.5, c.text_tertiary);
    ui.painter().line_segment([pos2(cx - 3.6, cy - 1.8), pos2(cx, cy + 1.8)], stroke);
    ui.painter().line_segment([pos2(cx, cy + 1.8), pos2(cx + 3.6, cy - 1.8)], stroke);
    let response = response.on_hover_text("Terrain to paint");
    egui::Popup::menu(&response).show(|ui| {
        for &id in &ids {
            let (name, _) = info(id);
            let mark = if id == current { icon::CHECK } else { "  " };
            if ui.button(format!("{mark}  {name}")).clicked() {
                tm.terrain = id;
                ui.close();
            }
        }
    });
}
