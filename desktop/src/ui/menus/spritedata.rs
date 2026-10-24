//! Sprite > Sprite Properties… (`SpritePropertiesSheet` in `SpriteDataUI.swift`), the reference layer commands and
//! the signed z-index field of the Cel Properties sheet.

use super::controls::{divider, sheet};
use crate::app::PixmithApp;
use crate::theme::{Colors, font, ui_font};
use crate::ui::widgets::{self, DialogAction};
use eframe::egui::{self, Ui};

/// How reference layers are drawn in the editor (`ReferenceLayerViewMenu`): title and percent opacity.
pub const REFERENCE_DISPLAY: [(&str, u32); 4] =
    [("Hidden", 0), ("Opaque", 100), ("Semi-transparent (50%)", 50), ("Faint (25%)", 25)];

const COLOR_MODES: [&str; 3] = ["RGB Color", "Grayscale", "Indexed"];

/// Draws the Sprite Properties sheet; returns true once it is closed.
pub fn sprite_properties(app: &mut PixmithApp, ctx: &egui::Context) -> bool {
    let c = Colors::of(ctx);
    let tab = app.tab_mut();
    let (w, h, frames) = (tab.doc.width(), tab.doc.height(), tab.doc.frame_count());
    let mode = COLOR_MODES[(tab.doc.color_mode() as usize).min(2)];
    let kind = tab.doc.color_profile_kind();
    let label = tab.doc.color_profile_label();
    let mut changed = false;
    let text = |ui: &mut Ui, s: &str| {
        ui.label(egui::RichText::new(s).font(ui_font(font::BODY)).color(c.text));
    };
    let action = sheet(ctx, "sprite_properties", 400.0, "Sprite Properties", None, true, |ui| {
        widgets::form_row(ui, "Size", |ui| text(ui, &format!("{w} \u{d7} {h} px")));
        widgets::form_row(ui, "Frames", |ui| text(ui, &frames.to_string()));
        widgets::form_row(ui, "Color mode", |ui| text(ui, mode));
        divider(ui);
        widgets::form_row(ui, "Color profile", |ui| text(ui, &label));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add_enabled_ui(kind != 1, |ui| {
                if button(ui, "Assign sRGB") {
                    changed |= tab.doc.set_color_profile(1);
                }
            });
            ui.add_enabled_ui(kind != 0, |ui| {
                if button(ui, "Remove Profile") {
                    changed |= tab.doc.set_color_profile(0);
                }
            });
        });
        widgets::note(
            ui,
            "The profile is stored in the file and used by other programs. Changing it does not convert any pixels.",
        );
        widgets::dialog_buttons(ui, None, None, "Done", true)
    });
    if changed {
        tab.changed();
    }
    action != DialogAction::None
}

/// A secondary button that screen readers (and the UI tests) can find by its text.
pub fn button(ui: &mut Ui, text: &str) -> bool {
    let response = widgets::secondary_button(ui, text, false);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), text));
    response.clicked()
}

/// A signed whole-number field for the Cel Properties sheet; returns true when the value changed.
pub fn signed_field(ui: &mut Ui, value: &mut i32, range: std::ops::RangeInclusive<i32>) -> bool {
    ui.add(egui::DragValue::new(value).range(range).speed(0.2)).changed()
}

/// Layer > New Reference Layer from File…
pub fn new_reference_layer(app: &mut PixmithApp) {
    let picked = rfd::FileDialog::new()
        .set_title("New Reference Layer from File")
        .add_filter("Images", &["png", "jpg", "jpeg", "gif", "bmp"])
        .pick_file();
    let Some(path) = picked else { return };
    let tab = app.tab_mut();
    let above = tab.layer;
    match tab.doc.new_reference_layer(above, &path) {
        Ok(index) => {
            tab.layer = index;
            tab.changed();
        }
        Err(message) => app.error = Some(message),
    }
}

/// Layer > Reference Layer: turns the current layer into a reference layer or back.
pub fn toggle_reference(app: &mut PixmithApp) {
    let tab = app.tab_mut();
    let (layer, on) = (tab.layer, !tab.doc.layer_is_reference(tab.layer));
    if tab.doc.set_layer_reference(layer, on) {
        tab.changed();
    }
}

/// View > Reference Layers: how opaque the editor draws them. A view setting; the document is not modified.
pub fn set_reference_display(app: &mut PixmithApp, index: usize) {
    let percent = REFERENCE_DISPLAY[index.min(REFERENCE_DISPLAY.len() - 1)].1;
    let tab = app.tab_mut();
    tab.doc.set_reference_display(percent);
    tab.changed();
}

/// Whether menu entry `index` of [`REFERENCE_DISPLAY`] is the current setting.
pub fn reference_display_is(app: &PixmithApp, index: usize) -> bool {
    REFERENCE_DISPLAY.get(index).is_some_and(|&(_, p)| p == app.tab().doc.reference_display())
}
