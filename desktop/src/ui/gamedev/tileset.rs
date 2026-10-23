//! Export a tileset to Godot 4: a TileSet `.tres` and its atlas PNG, optionally a scene
//! (`Dialogs/GodotTilesetExport.swift`). The settings are remembered per document (keyed by its file path).

use super::TilesetSettings;
use super::form;
use crate::app::PixmithApp;
use crate::theme::{Colors, font, metric, radius, space};
use crate::ui::widgets::{self, DialogAction, NumberOpts};
use eframe::egui::{self, Color32, Sense, Stroke, StrokeKind, Ui, vec2};
const LABEL_WIDTH: f32 = 96.0;
const MODES: [&str; 3] = ["Match Corners and Sides (blob)", "Match Corners", "Match Sides"];
const COLLISIONS: [&str; 3] = ["None", "Full tile square", "Traced from opaque pixels"];
const SIZES: [u32; 5] = [1024, 2048, 4096, 8192, 16384];

pub struct TilesetSheet {
    pub s: TilesetSettings,
    /// "0: Tileset 1", ... for the tileset popup.
    pub names: Vec<String>,
    /// The result of the last Preview.
    pub preview: String,
    /// An error or warning from the last export, shown in place of the preview.
    pub result: Option<String>,
}

impl TilesetSheet {
    fn ready(&self) -> bool {
        !self.s.project_root.is_empty() && !self.s.output_path.is_empty()
    }
}

/// Opens the sheet (`exportGodotTileset`).
pub fn open(app: &mut PixmithApp) {
    let tab = app.tab();
    let count = tab.doc.tileset_count();
    if count == 0 {
        app.error = Some("This sprite has no tileset. Add a tilemap layer first.".into());
        return;
    }
    let mut s = match app.gamedev.prefs.tileset_settings(tab.doc.path.as_deref()) {
        Some(saved) => saved,
        None => TilesetSettings { tileset: tab.doc.layer_tileset(tab.layer).unwrap_or(0), ..Default::default() },
    };
    s.tileset = s.tileset.min(count - 1);
    let names = (0..count).map(|i| format!("{i}: {}", tab.doc.tileset_name(i))).collect();
    app.gamedev.tileset = Some(TilesetSheet { s, names, preview: String::new(), result: None });
}

/// Whether Re-export has saved settings to repeat (`hasGodotExportSettings`).
pub fn has_settings(app: &PixmithApp) -> bool {
    app.gamedev.prefs.tileset_settings(app.tab().doc.path.as_deref()).is_some_and(|s| !s.output_path.is_empty())
}

/// Runs an export; returns an error or warning text, or `None` on a clean export (`runGodotExport`).
pub fn run(app: &mut PixmithApp, s: &TilesetSettings) -> Option<String> {
    match app.tab().doc.tileset_export(s) {
        Err(e) => Some(e),
        Ok(warnings) => {
            let path = app.tab().doc.path.clone();
            app.gamedev.prefs.set_tileset_settings(path.as_deref(), s);
            (!warnings.is_empty()).then(|| format!("Exported with warnings:\n{warnings}"))
        }
    }
}

/// Repeats the last export of this document with its remembered settings (`reexportGodotTileset`).
pub fn reexport(app: &mut PixmithApp) {
    let Some(s) =
        app.gamedev.prefs.tileset_settings(app.tab().doc.path.as_deref()).filter(|s| !s.output_path.is_empty())
    else {
        return;
    };
    if let Some(message) = run(app, &s) {
        app.error = Some(message);
    }
}

pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(mut sheet) = app.gamedev.tileset.take() else { return };
    let tileset_name = app.tab().doc.tileset_name(sheet.s.tileset.min(sheet.names.len().saturating_sub(1) as u32));
    let mut preview_now = false;
    let action = widgets::dialog(
        ctx,
        "tileset",
        580.0,
        "Export Tileset to Godot 4",
        Some(
            "Writes a TileSet .tres and its atlas PNG. Godot 4 only: set Default Texture Filter to Nearest to keep \
             pixels sharp.",
        ),
        |ui| {
            body(ui, &mut sheet, &tileset_name, &mut preview_now);
            widgets::dialog_buttons(ui, None, Some("Cancel"), "Export", sheet.ready())
        },
    );
    if preview_now {
        sheet.preview = if sheet.ready() {
            app.tab().doc.tileset_preview(&sheet.s)
        } else {
            "Choose the project folder and the .tres file first.".into()
        };
        sheet.result = None;
    }
    match action {
        DialogAction::Primary => match run(app, &sheet.s) {
            None => {}
            Some(message) => {
                let warnings = message.starts_with("Exported with warnings");
                sheet.result = Some(message);
                if !warnings {
                    app.gamedev.tileset = Some(sheet);
                }
            }
        },
        DialogAction::Cancel => {}
        _ => app.gamedev.tileset = Some(sheet),
    }
}

fn body(ui: &mut Ui, sheet: &mut TilesetSheet, tileset_name: &str, preview_now: &mut bool) {
    let c = Colors::of(ui.ctx());
    let s = &mut sheet.s;
    form::form_row(ui, LABEL_WIDTH, "Tileset", |ui| {
        let options: Vec<&str> = sheet.names.iter().map(String::as_str).collect();
        if let Some(i) = widgets::ds_popup(ui, "tileset", s.tileset as usize, &options, None, true, "Tileset to export")
        {
            s.tileset = i as u32;
        }
    });
    form::form_row(ui, LABEL_WIDTH, "Project folder", |ui| {
        let button = form::bar_button_width(ui, "Choose\u{2026}") + space::SM;
        form::text_field(
            ui,
            "root",
            &mut s.project_root,
            "Folder with project.godot",
            Some(ui.available_width() - button),
        );
        if form::bar_button(ui, "Choose\u{2026}").clicked()
            && let Some(dir) = rfd::FileDialog::new()
                .set_title("Choose the Godot project folder (contains project.godot)")
                .pick_folder()
        {
            s.project_root = dir.to_string_lossy().into_owned();
        }
    });
    form::form_row(ui, LABEL_WIDTH, "TileSet file", |ui| {
        let button = form::bar_button_width(ui, "Choose\u{2026}") + space::SM;
        form::text_field(
            ui,
            "tres",
            &mut s.output_path,
            ".tres inside the project",
            Some(ui.available_width() - button),
        );
        if form::bar_button(ui, "Choose\u{2026}").clicked() {
            let name = if tileset_name.is_empty() { "tiles" } else { tileset_name };
            let mut dialog = rfd::FileDialog::new()
                .set_title("Save inside the Godot project. The atlas PNG is written next to it.")
                .add_filter("Godot resource", &["tres"])
                .set_file_name(format!("{name}.tres"));
            if !s.project_root.is_empty() {
                dialog = dialog.set_directory(&s.project_root);
            }
            if let Some(file) = dialog.save_file() {
                s.output_path = file.to_string_lossy().into_owned();
            }
        }
    });
    form::hairline(ui);
    form::form_row(ui, LABEL_WIDTH, "Terrain mode", |ui| {
        if let Some(i) =
            widgets::ds_popup(ui, "mode", s.mode as usize, &MODES, None, true, "How Godot matches terrain tiles")
        {
            s.mode = i as u32;
        }
    });
    form::form_row(ui, LABEL_WIDTH, "Terrain name", |ui| {
        let swatch = 36.0 + space::SM;
        form::text_field(
            ui,
            "terrain",
            &mut s.terrain_name,
            "Default: tileset name",
            Some(ui.available_width() - swatch),
        );
        color_button(ui, &mut s.color);
    });
    form::form_row(ui, LABEL_WIDTH, "Collision", |ui| {
        if let Some(i) = widgets::ds_popup(
            ui,
            "collision",
            s.collision as usize,
            &COLLISIONS,
            None,
            true,
            "Physics shape for each tile",
        ) {
            s.collision = i as u32;
        }
    });
    form::hairline(ui);
    form::form_row(ui, LABEL_WIDTH, "Atlas", |ui| {
        let field = |ui: &mut Ui, id: &str, label: &str, value: &mut u32, max: f64, width: f32, help: &str| {
            let mut v = *value as f64;
            let o = NumberOpts { range: (0.0, max), unit: "px", width: Some(width), help, ..Default::default() };
            if widgets::ds_number_field(ui, id, label, &mut v, &o).changed {
                *value = v as u32;
            }
        };
        field(ui, "margin", "Margin", &mut s.margin, 64.0, 116.0, "Space around the atlas");
        field(ui, "gap", "Gap", &mut s.separation, 64.0, 100.0, "Separation between tiles");
        field(ui, "extrude", "Extrude", &mut s.extrude, 16.0, 120.0, "Repeat edge pixels to stop bleeding");
    });
    form::form_row(ui, LABEL_WIDTH, "Max texture", |ui| {
        let labels: Vec<String> = SIZES.iter().map(|n| format!("{n} px")).collect();
        let options: Vec<&str> = labels.iter().map(String::as_str).collect();
        let selected = SIZES.iter().position(|n| *n == s.max_texture).unwrap_or(2);
        if let Some(i) = widgets::ds_popup(ui, "max_texture", selected, &options, None, false, "Largest atlas page") {
            s.max_texture = SIZES[i];
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(LABEL_WIDTH);
        form::check(ui, "scene", &mut s.write_scene, "Also write a scene (.tscn) with the tilemap layers painted");
    });
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = space::XS + 2.0;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Preview").font(widgets::strong(ui.ctx(), font::CAPTION)).color(c.text));
            form::right_to_left(ui, |ui| {
                if form::bar_button(ui, "Preview").on_hover_text("Show what will be written").clicked() {
                    *preview_now = true;
                }
            });
        });
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 96.0), Sense::hover());
        ui.painter().rect_filled(rect, radius::FIELD, c.recessed);
        let (text, color) = match (&sheet.result, sheet.preview.is_empty()) {
            (Some(r), _) => (r.as_str(), c.text_secondary),
            (None, true) => ("Press Preview to see what will be written.", c.text_tertiary),
            (None, false) => (sheet.preview.as_str(), c.text_secondary),
        };
        let mut inner = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink(space::SM + 2.0))
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
        );
        inner.set_clip_rect(rect.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical().id_salt("tileset_preview").auto_shrink([false, false]).show(&mut inner, |ui| {
            ui.add(
                egui::Label::new(egui::RichText::new(text).font(egui::FontId::monospace(font::CAPTION)).color(color))
                    .wrap()
                    .selectable(true),
            );
        });
    });
}

/// The terrain color well: a 36 x 24 swatch that opens a color picker.
fn color_button(ui: &mut Ui, color: &mut [f64; 4]) {
    let (rect, response) = ui.allocate_exact_size(vec2(36.0, metric::CONTROL), Sense::click());
    let mut c32 = Color32::from_rgba_unmultiplied(
        (color[0] * 255.0).round() as u8,
        (color[1] * 255.0).round() as u8,
        (color[2] * 255.0).round() as u8,
        (color[3] * 255.0).round() as u8,
    );
    ui.painter().rect_filled(rect, radius::FIELD, c32);
    ui.painter().rect_stroke(rect, radius::FIELD, Stroke::new(1.0, Color32::from_white_alpha(31)), StrokeKind::Inside);
    let response = response.on_hover_text("Terrain color in Godot's editor");
    egui::Popup::menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        if egui::color_picker::color_picker_color32(ui, &mut c32, egui::color_picker::Alpha::Opaque) {
            let [r, g, b, a] = c32.to_srgba_unmultiplied();
            *color = [r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, a as f64 / 255.0];
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Doc;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pixmith-tileset-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A 128 x 96 sprite with a 16 x 16 tilemap layer, the layer the sprite's tileset belongs to.
    fn tile_app() -> PixmithApp {
        let mut app = PixmithApp::default();
        let mut doc = Doc::new(128, 96);
        // SAFETY: the document is live.
        let layer = unsafe { pixmith_core::document::tilemap::pixmith_add_tilemap_layer(doc.raw(), 0, 16, 16) };
        // SAFETY: as above; the first tile is the empty one, these two have pixels.
        unsafe {
            pixmith_core::document::tilemap::pixmith_tileset_add_tile(doc.raw(), 0);
            pixmith_core::document::tilemap::pixmith_tileset_add_tile(doc.raw(), 0);
        }
        doc.touch();
        app.tabs[0] = crate::app::DocTab::new(doc);
        app.tabs[0].layer = layer;
        app
    }

    #[test]
    fn a_sprite_without_tilesets_says_so() {
        let mut app = PixmithApp::default();
        open(&mut app);
        assert_eq!(app.error.as_deref(), Some("This sprite has no tileset. Add a tilemap layer first."));
        assert!(app.gamedev.tileset.is_none());
    }

    #[test]
    fn the_sheet_opens_on_the_active_layers_tileset() {
        let mut app = tile_app();
        open(&mut app);
        let sheet = app.gamedev.tileset.as_ref().expect("sheet");
        assert_eq!(sheet.s.tileset, 0);
        assert_eq!(sheet.names.len(), 1);
        assert!(sheet.names[0].starts_with("0: "));
    }

    #[test]
    fn export_and_reexport_use_the_remembered_settings() {
        let dir = scratch("export");
        let project = dir.join("game");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("project.godot"), "config_version=5\n").unwrap();
        let file = dir.join("level.aseprite");
        let mut app = tile_app();
        app.tab_mut().doc.save_marked(&file).unwrap();
        assert!(!has_settings(&app));
        let s = TilesetSettings {
            project_root: project.to_string_lossy().into_owned(),
            output_path: project.join("art/ground.tres").to_string_lossy().into_owned(),
            collision: 1,
            terrain_name: "Grass".into(),
            ..Default::default()
        };
        let preview = app.tab().doc.tileset_preview(&s);
        assert!(!preview.is_empty());
        assert!(!project.join("art").exists(), "preview writes nothing");
        let result = run(&mut app, &s).expect("no auto-tile data: a warning");
        assert!(result.starts_with("Exported with warnings:"), "{result}");
        assert!(project.join("art/ground.tres").exists());
        assert!(has_settings(&app));
        std::fs::remove_dir_all(project.join("art")).unwrap();
        reexport(&mut app);
        assert!(app.error.as_deref().is_some_and(|e| e.starts_with("Exported with warnings")), "{:?}", app.error);
        assert!(project.join("art/ground.tres").exists(), "re-export wrote it again");
        // A sheet opened later starts from the remembered values.
        open(&mut app);
        assert_eq!(app.gamedev.tileset.as_ref().unwrap().s.terrain_name, "Grass");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn export_outside_the_project_is_an_error_text() {
        let dir = scratch("outside");
        let app = tile_app();
        let s = TilesetSettings {
            project_root: dir.join("game").to_string_lossy().into_owned(),
            output_path: dir.join("elsewhere/x.tres").to_string_lossy().into_owned(),
            ..Default::default()
        };
        assert!(app.tab().doc.tileset_export(&s).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
