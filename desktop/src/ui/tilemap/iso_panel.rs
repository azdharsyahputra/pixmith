//! The isometric part of the Tileset section: the layer's grid offset (its elevation) and the selected tile's
//! origin for tall tiles.

use super::TilemapState;
use super::controls::{bar_button, caption, caption_medium};
use crate::app::DocTab;
use crate::theme::{Colors, space};
use crate::ui::widgets::{NumberOpts, ds_number_field};
use eframe::egui::{Align, Layout, Ui};

fn field(ui: &mut Ui, id: &str, label: &str, value: &mut i32, help: &str) -> bool {
    let mut v = *value as f64;
    let o = NumberOpts {
        range: (-16384.0, 16384.0),
        width: Some((ui.available_width() - space::SM).max(40.0) / 2.0),
        help,
        defer_scrub: true,
        ..Default::default()
    };
    let r = ds_number_field(ui, id, label, &mut v, &o);
    if r.changed {
        *value = v as i32;
    }
    r.changed
}

/// Offset fields and the tile origin of an isometric layer. Returns true when the document changed.
pub fn iso_section(ui: &mut Ui, tab: &mut DocTab, tm: &TilemapState, ts: u32) -> bool {
    let layer = tab.layer;
    let (Some(cell), Some(mut offset)) = (tab.doc.tileset_iso_cell(ts), tab.doc.layer_grid_offset(layer)) else {
        return false;
    };
    let c = Colors::of(ui.ctx());
    let mut changed = false;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = space::XS;
        ui.horizontal(|ui| {
            caption_medium(ui, "Isometric grid", c.text_tertiary);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                caption(ui, &format!("diamond {}\u{d7}{}", cell.0, cell.1), c.text_tertiary);
            });
        });
        let before = offset;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            field(ui, "iso_offset_x", "X", &mut offset.0, "Canvas pixel of the first cell's box, left");
            field(ui, "iso_offset_y", "Y", &mut offset.1, "Canvas pixel of the first cell's box, top");
        });
        let step = (cell.1 / 2) as i32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            if bar_button(ui, "Raise", None, false, false)
                .on_hover_text("Move this layer's grid up half a cell: one elevation level")
                .clicked()
            {
                offset.1 -= step;
            }
            if bar_button(ui, "Lower", None, false, false)
                .on_hover_text("Move this layer's grid down half a cell")
                .clicked()
            {
                offset.1 += step;
            }
        });
        if offset != before && tab.doc.set_layer_grid_offset(layer, offset) {
            changed = true;
        }
    });
    changed |= tile_origin(ui, tab, tm, ts);
    changed
}

/// Origin of the selected tile (Godot's `texture_origin`): how far the picture is shifted from the cell's center.
fn tile_origin(ui: &mut Ui, tab: &mut DocTab, tm: &TilemapState, ts: u32) -> bool {
    let tile = tm.selected;
    let (Some(origin), true) = (tab.doc.tile_texture_origin(ts, tile), tile != 0) else { return false };
    let c = Colors::of(ui.ctx());
    let (tw, th) = tab.doc.tile_size(ts);
    let is_set = tab.doc.tile_pivot_is_set(ts, tile);
    let mut value = origin;
    let mut reset = false;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = space::XS;
        ui.horizontal(|ui| {
            caption_medium(ui, &format!("Tile {tile} origin"), c.text_tertiary);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if is_set && bar_button(ui, "Reset", None, false, false).on_hover_text("Back to the default").clicked()
                {
                    reset = true;
                }
            });
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            field(
                ui,
                "iso_origin_x",
                "X",
                &mut value.0,
                "Shifts the picture from the diamond's center (Godot texture origin)",
            );
            field(
                ui,
                "iso_origin_y",
                "Y",
                &mut value.1,
                "Shifts the picture from the diamond's center (Godot texture origin)",
            );
        });
    });
    let mut changed = false;
    if reset {
        changed = tab.doc.clear_tile_pivot(ts, tile);
    } else if value != origin {
        // The pivot is the image point that sits on the diamond's center: the image center plus the origin.
        changed = tab.doc.set_tile_pivot(ts, tile, (value.0 + tw as i32 / 2, value.1 + th as i32 / 2));
    }
    changed
}
