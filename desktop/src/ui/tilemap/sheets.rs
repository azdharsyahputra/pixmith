//! The tilemap and slice sheets: New Tilemap Layer, Convert to Tilemap, Rename Tileset, Slice Properties and
//! User Data (`TileSizeSheet`, `TilesetRenameSheet`, `SliceSheet`, `UserDataSheet`).

use super::{Sheet, slices};
use crate::app::PixmithApp;
use crate::theme::{self, Colors, font, radius, space};
use crate::ui::widgets::{self, DialogAction, NumberOpts, ds_number_field, ds_popup, ds_text_field};
use eframe::egui::{self, Color32, Sense, TextEdit, Ui, vec2};

/// Draws the open sheet, if any. Called once per frame after the file dialogs; it stays out of the way while
/// another dialog is up.
pub fn show_sheets(app: &mut PixmithApp, ctx: &egui::Context) {
    if app.error.is_some() || app.files.prompt.is_some() || app.files.new_sheet.is_some() || app.files.export.is_some()
    {
        return;
    }
    if let Some(sheet) = app.tilemap.sheet {
        match sheet {
            Sheet::Rename => rename_sheet(app, ctx),
            _ => size_sheet(app, ctx, sheet == Sheet::Convert),
        }
    } else if app.tilemap.slice_sheet.is_some() {
        slice_sheet(app, ctx);
    } else if app.tilemap.user_sheet.is_some() {
        user_sheet(app, ctx);
    }
}

fn size_sheet(app: &mut PixmithApp, ctx: &egui::Context, convert: bool) {
    let tm = &mut app.tilemap;
    let (mut w, mut h, mut flip) = (tm.sheet_w, tm.sheet_h, tm.sheet_flip_aware);
    let title = if convert { "Convert to Tilemap" } else { "New Tilemap Layer" };
    let action = widgets::dialog(ctx, "tilemap-size", 320.0, title, None, |ui| {
        widgets::form_row(ui, "Tile width", |ui| widgets::number_field(ui, "W", &mut w, 1..=256, "px", 120.0));
        widgets::form_row(ui, "Tile height", |ui| widgets::number_field(ui, "H", &mut h, 1..=256, "px", 120.0));
        if convert {
            ui.checkbox(&mut flip, "Share flipped copies of tiles");
        }
        widgets::dialog_buttons(ui, None, Some("Cancel"), if convert { "Convert" } else { "Create" }, true)
    });
    tm.sheet_w = w;
    tm.sheet_h = h;
    tm.sheet_flip_aware = flip;
    match action {
        DialogAction::Primary => {
            tm.sheet = None;
            let (w, h) = (w.clamp(1, 256), h.clamp(1, 256));
            let tab = &mut app.tabs[app.active];
            let layer = tab.layer;
            if convert {
                if tab.doc.convert_layer_to_tilemap(layer, w, h, flip) {
                    app.tilemap.selected = 1;
                    tab.changed();
                } else {
                    app.error = Some("Only normal layers can be converted to tilemaps.".to_string());
                }
            } else {
                tab.playing = false;
                tab.layer = tab.doc.new_tilemap_layer(layer, w, h);
                app.tilemap.selected = 1;
                tab.changed();
            }
        }
        DialogAction::Cancel => tm.sheet = None,
        _ => {}
    }
}

fn rename_sheet(app: &mut PixmithApp, ctx: &egui::Context) {
    let tm = &mut app.tilemap;
    let mut name = std::mem::take(&mut tm.sheet_name);
    let action = widgets::dialog(ctx, "tileset-rename", 300.0, "Rename Tileset", None, |ui| {
        ds_text_field(ui, "tileset_name", &mut name, None);
        widgets::dialog_buttons(ui, None, Some("Cancel"), "Rename", true)
    });
    tm.sheet_name = name.clone();
    match action {
        DialogAction::Primary => {
            tm.sheet = None;
            let tab = &mut app.tabs[app.active];
            if let Some(ts) = tab.doc.tileset_of_layer(tab.layer)
                && tab.doc.rename_tileset(ts, &name)
            {
                tab.changed();
            }
        }
        DialogAction::Cancel => tm.sheet = None,
        _ => {}
    }
}

/// Text and optional color of an object's user data (`UserDataFields`).
fn user_data_fields(ui: &mut Ui, text: &mut String, has_color: &mut bool, color: &mut Color32) {
    let c = Colors::of(ui.ctx());
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = space::SM;
        ui.label(
            egui::RichText::new("User data").font(widgets::strong(ui.ctx(), font::CAPTION)).color(c.text_secondary),
        );
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::hover());
        ui.painter().rect_filled(rect, radius::FIELD, c.field);
        ui.put(
            rect.shrink(space::XS),
            TextEdit::multiline(text)
                .font(theme::ui_font(font::BODY))
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .desired_width(rect.width() - 2.0 * space::XS),
        );
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            ui.checkbox(has_color, "Color");
            ui.add_enabled_ui(*has_color, |ui| {
                ui.color_edit_button_srgba(color);
            });
        });
    });
}

fn rgba_of(c: Color32) -> u32 {
    u32::from_be_bytes([c.r(), c.g(), c.b(), 0xFF])
}

fn int_field(ui: &mut Ui, id: &str, label: &str, value: &mut f64) {
    let opts = NumberOpts { range: (-99999.0, 99999.0), width: Some(72.0), ..Default::default() };
    ds_number_field(ui, id, label, value, &opts);
}

fn slice_sheet(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(mut s) = app.tilemap.slice_sheet.take() else { return };
    let action = widgets::dialog(ctx, "slice-props", 380.0, "Slice Properties", None, |ui| {
        widgets::form_row(ui, "Name", |ui| ds_text_field(ui, "slice_name", &mut s.name, None));
        ui.checkbox(&mut s.nine, "9-patch center");
        ui.add_enabled_ui(s.nine, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::XS + 2.0;
                for (i, label) in ["X", "Y", "W", "H"].iter().enumerate() {
                    int_field(ui, &format!("slice_center_{i}"), label, &mut s.center[i]);
                }
            });
        });
        ui.checkbox(&mut s.pivot_on, "Pivot");
        ui.add_enabled_ui(s.pivot_on, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::XS + 2.0;
                for (i, label) in ["X", "Y"].iter().enumerate() {
                    int_field(ui, &format!("slice_pivot_{i}"), label, &mut s.pivot[i]);
                }
            });
        });
        let c = Colors::of(ui.ctx());
        let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(line, 0.0, c.separator_soft);
        user_data_fields(ui, &mut s.text, &mut s.has_color, &mut s.color);
        widgets::dialog_buttons(ui, Some("Delete"), Some("Cancel"), "OK", true)
    });
    let tab = &mut app.tabs[app.active];
    match action {
        DialogAction::Primary => {
            let center = s.nine.then(|| s.center.map(|v| v as i32));
            let pivot = s.pivot_on.then(|| s.pivot.map(|v| v as i32));
            let color = s.has_color.then(|| rgba_of(s.color));
            let frame = tab.frame;
            tab.doc.in_transaction(|d| {
                d.set_slice_props(s.index, frame, &s.name, center, pivot);
                d.set_user_data(4, s.index, 0, &s.text, color);
                d.label_last_step("Slice Properties");
            });
            tab.doc.touch();
            tab.changed();
        }
        DialogAction::Destructive => {
            app.tilemap.slice_selected = Some(s.index);
            if slices::delete_selected(&mut app.tilemap, tab) {
                tab.changed();
            }
        }
        DialogAction::Cancel => {}
        DialogAction::None => app.tilemap.slice_sheet = Some(s),
    }
}

fn user_sheet(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(mut s) = app.tilemap.user_sheet.take() else { return };
    let tab = &mut app.tabs[app.active];
    let title = match s.kind {
        0 => "Sprite User Data".to_string(),
        1 => format!("Layer \u{201C}{}\u{201D} User Data", tab.doc.layer_name(tab.layer)),
        2 => format!("Cel User Data (frame {}, {})", tab.frame + 1, tab.doc.layer_name(tab.layer)),
        _ => "Tag User Data".to_string(),
    };
    let tags: Vec<String> =
        (0..tab.doc.tag_count()).map(|i| tab.doc.tag(i).map(|t| t.name).unwrap_or_default()).collect();
    let mut tag_changed = false;
    let action = widgets::dialog(ctx, "user-data", 380.0, &title, None, |ui| {
        if s.kind == 3 {
            widgets::form_row(ui, "Tag", |ui| {
                let names: Vec<&str> = tags.iter().map(String::as_str).collect();
                if let Some(i) = ds_popup(ui, "user_data_tag", s.tag, &names, None, true, "") {
                    s.tag = i;
                    tag_changed = true;
                }
            });
        }
        user_data_fields(ui, &mut s.text, &mut s.has_color, &mut s.color);
        widgets::dialog_buttons(ui, None, Some("Cancel"), "OK", true)
    });
    if tag_changed {
        slices::load_user_sheet(&mut s, tab);
    }
    match action {
        DialogAction::Primary => {
            let (a, b) = slices::user_target(&s, tab);
            let color = s.has_color.then(|| rgba_of(s.color));
            tab.doc.set_user_data(s.kind, a, b, &s.text, color);
            tab.doc.label_last_step("User Data");
            tab.doc.touch();
            tab.changed();
        }
        DialogAction::Cancel => {}
        _ => app.tilemap.user_sheet = Some(s),
    }
}
