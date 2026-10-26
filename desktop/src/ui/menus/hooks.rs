//! Menu entries whose feature lives in another area. The menu bar calls these, shows their state and keeps
//! their shortcuts; each forwards to the module that owns the feature.

use crate::app::PixmithApp;
use crate::doc::Doc;
use crate::ui::tools::view::GridType;
use crate::ui::{selection, tilemap, tools, transform};
use eframe::egui;
use std::path::{Path, PathBuf};

// MARK: Selection and transform (`Selection.swift`, `FreeTransform.swift`)

pub fn has_selection(app: &PixmithApp) -> bool {
    selection::has_selection(app)
}

pub fn cut(app: &mut PixmithApp) {
    selection::copy(app, true);
}

pub fn copy(app: &mut PixmithApp) {
    selection::copy(app, false);
}

pub fn paste(app: &mut PixmithApp) {
    selection::paste(app);
}

/// Edit > Delete
pub fn delete_selection(app: &mut PixmithApp) {
    selection::delete_selection(app);
}

/// Edit > Free Transform
pub fn free_transform(app: &mut PixmithApp) {
    transform::start(app);
}

/// `op` is 0 flip horizontal, 1 flip vertical, 2 rotate 90 clockwise, 3 rotate 90 counterclockwise.
pub fn transform(app: &mut PixmithApp, op: u32) {
    selection::transform_selection(app, op);
}

pub fn select_all(app: &mut PixmithApp) {
    selection::select_all(app);
}

pub fn deselect(app: &mut PixmithApp) {
    selection::deselect(app);
}

pub fn invert_selection(app: &mut PixmithApp) {
    selection::invert_selection(app);
}

/// `kind` is 0 expand, 1 contract, 2 border.
pub fn modify_selection(app: &mut PixmithApp, kind: u32) {
    selection::modify_selection(app, kind);
}

pub fn select_by_color(app: &mut PixmithApp) {
    selection::select_by_color(app);
}

/// Select > Isometric Tile
pub fn select_iso_tile(app: &mut PixmithApp) {
    selection::select_iso_tile(app);
}

/// Whether the custom grid is on and isometric (the check mark of View > Isometric Grid).
pub fn iso_grid_shown(app: &PixmithApp) -> bool {
    let s = &app.canvas.tools.settings;
    s.custom_grid && s.custom_grid_type == GridType::Isometric
}

/// View > Isometric Grid: shows the isometric grid, or hides the custom grid when it is already showing.
pub fn toggle_iso_grid(app: &mut PixmithApp) {
    let on = iso_grid_shown(app);
    let s = &mut app.canvas.tools.settings;
    s.custom_grid = !on;
    s.custom_grid_type = GridType::Isometric;
}

/// Sprite > Crop to Selection
pub fn crop_to_selection(app: &mut PixmithApp) {
    selection::crop_to_selection(app);
}

/// Drawing > Brush from Selection
pub fn brush_from_selection(app: &mut PixmithApp) {
    tools::brush_from_selection(app);
}

/// Drawing > Reset Brush
pub fn reset_brush(app: &mut PixmithApp) {
    tools::reset_brush(app);
}

pub fn has_custom_brush(app: &PixmithApp) -> bool {
    app.canvas.opts.custom_brush.is_some()
}

// MARK: Slices and user data (`SlicesView.swift`)

pub fn has_selected_slice(app: &PixmithApp) -> bool {
    tilemap::has_selected_slice(app)
}

pub fn slice_properties(app: &mut PixmithApp) {
    tilemap::slice_properties(app);
}

pub fn delete_slice(app: &mut PixmithApp) {
    tilemap::delete_selected_slice(app);
}

/// `kind` is 0 sprite, 1 layer, 2 cel, 3 tag.
pub fn user_data(app: &mut PixmithApp, kind: u32) {
    tilemap::user_data_sheet(app, kind);
}

// MARK: View options, preview and reference (`ViewSettings.swift`, `PreviewWindow.swift`, `ViewOptionsWindow.swift`)

const TILED: [tools::view::TiledMode; 4] =
    [tools::view::TiledMode::None, tools::view::TiledMode::X, tools::view::TiledMode::Y, tools::view::TiledMode::Both];

/// 0 none, 1 tiled in X, 2 tiled in Y, 3 tiled in both axes.
pub fn tiled_mode(app: &PixmithApp) -> usize {
    let mode = tools::tiled_mode(app);
    TILED.iter().position(|&m| m == mode).unwrap_or(0)
}

pub fn set_tiled_mode(app: &mut PixmithApp, mode: usize) {
    tools::set_tiled_mode(app, TILED.get(mode).copied().unwrap_or_default());
}

pub fn preview_open(app: &PixmithApp) -> bool {
    tools::is_preview_open(app)
}

/// View > Preview
pub fn toggle_preview(app: &mut PixmithApp) {
    tools::toggle_preview(app);
}

pub fn has_reference_image(app: &PixmithApp) -> bool {
    tools::has_reference(app)
}

pub fn show_reference(app: &PixmithApp) -> bool {
    app.canvas.tools.settings.show_reference
}

pub fn set_show_reference(app: &mut PixmithApp, on: bool) {
    app.canvas.tools.settings.show_reference = on;
}

/// View > Load Reference Image…
pub fn load_reference_image(app: &mut PixmithApp, ctx: &egui::Context) {
    tools::choose_reference(app, ctx);
}

/// View > View Options…
pub fn view_options(app: &mut PixmithApp) {
    tools::toggle_view_options(app);
}

/// View > Tween
pub fn toggle_tween(app: &mut PixmithApp) {
    app.toggle_tween_panel();
}

// MARK: Tilemap (`TilemapView.swift`)

pub fn new_tilemap_layer(app: &mut PixmithApp) {
    tilemap::new_tilemap_layer_sheet(app);
}

pub fn convert_to_tilemap(app: &mut PixmithApp) {
    tilemap::convert_layer_sheet(app);
}

pub fn convert_to_normal(app: &mut PixmithApp) {
    tilemap::convert_to_normal(app);
}

pub fn tile_stamp(app: &PixmithApp) -> bool {
    app.tilemap.stamp
}

pub fn set_tile_stamp(app: &mut PixmithApp, on: bool) {
    if app.tilemap.stamp != on {
        tilemap::toggle_stamp(app);
    }
}

// MARK: Files: import, scripts, Godot (`FileActions.swift`, `Scripts.swift`, `GodotExport.swift`)

fn pick(title: &str, name: &str, extensions: &[&str]) -> Option<PathBuf> {
    rfd::FileDialog::new().set_title(title).add_filter(name, extensions).pick_file()
}

/// Opens an imported document in a tab, or reports why it failed.
fn present(app: &mut PixmithApp, source: &Path, doc: Result<Doc, String>) {
    match doc {
        Ok(doc) => {
            let mut doc = doc.with_import_source(source);
            // An import is unsaved work (`SpriteModel.changed()`).
            doc.touch();
            app.present(doc);
        }
        Err(message) => app.error = Some(message),
    }
}

/// File > Import > Sprite Sheet as Frames… (asks for the cell size next).
pub fn import_sheet_as_frames(app: &mut PixmithApp) {
    if let Some(path) = pick("Import Sprite Sheet as Frames", "PNG", &["png"]) {
        let (w, h) = (app.tab().doc.width().clamp(1, 4096), app.tab().doc.height().clamp(1, 4096));
        app.files.sheet_import = Some((path, w, h));
    }
}

/// The cell-size question of [`import_sheet_as_frames`], while it is open.
pub fn sheet_import_dialog(app: &mut PixmithApp, ctx: &egui::Context) {
    use crate::ui::widgets::{self, DialogAction};
    let Some((path, mut w, mut h)) = app.files.sheet_import.clone() else { return };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let action = widgets::dialog(ctx, "sheet-import", 300.0, "Import Sprite Sheet", Some(&name), |ui| {
        widgets::form_row(ui, "Cell width", |ui| widgets::number_field(ui, "W", &mut w, 1..=4096, "px", 120.0));
        widgets::form_row(ui, "Cell height", |ui| widgets::number_field(ui, "H", &mut h, 1..=4096, "px", 120.0));
        widgets::dialog_buttons(ui, None, Some("Cancel"), "Import", true)
    });
    app.files.sheet_import = Some((path.clone(), w, h));
    match action {
        DialogAction::Primary => {
            app.files.sheet_import = None;
            present(app, &path, Doc::import_sheet(&path, w, h));
        }
        DialogAction::Cancel => app.files.sheet_import = None,
        _ => {}
    }
}

/// File > Import > Sprite Sheet with JSON…
pub fn import_sheet_with_json(app: &mut PixmithApp) {
    if let Some(path) = pick("Import Sprite Sheet with JSON", "Aseprite JSON", &["json"]) {
        present(app, &path, Doc::import_sheet_json(&path));
    }
}

/// File > Import > Animated GIF as New Sprite…
pub fn import_gif_as_sprite(app: &mut PixmithApp) {
    if let Some(path) = pick("Import Animated GIF", "GIF", &["gif"]) {
        present(app, &path, Doc::import_gif(&path));
    }
}

/// File > Import > PNG as New Layer…
pub fn import_png_as_layer(app: &mut PixmithApp) {
    let Some(path) = pick("Import PNG as New Layer", "PNG", &["png"]) else { return };
    let tab = app.tab_mut();
    let (layer, frame) = (tab.layer, tab.frame);
    match tab.doc.import_png_layer(layer, frame, &path) {
        Ok(index) => {
            tab.layer = index;
            tab.changed();
        }
        Err(message) => app.error = Some(message),
    }
}

/// Names of the scripts in the scripts folder, for File > Scripts.
pub fn script_names(app: &PixmithApp) -> Vec<String> {
    app.gamedev
        .scripts
        .cached()
        .iter()
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect()
}

/// Runs the script called `name`.
pub fn run_script(app: &mut PixmithApp, name: &str) {
    let path = app.scripts().into_iter().find(|p| p.file_stem().is_some_and(|s| s.to_string_lossy() == name));
    if let Some(path) = path {
        app.run_script_at(&path);
    }
}

/// File > Scripts > Run Script…
pub fn run_script_from_file(app: &mut PixmithApp) {
    app.run_script_dialog();
}

/// File > Scripts > Open Scripts Folder
pub fn open_scripts_folder(app: &mut PixmithApp) {
    app.open_scripts_folder();
}

/// File > Scripts > Install Example Scripts
pub fn install_example_scripts(app: &mut PixmithApp) {
    app.install_example_scripts();
}

/// File > Scripts > Rescan
pub fn rescan_scripts(app: &mut PixmithApp) {
    app.rescan_scripts();
}

/// File > Export to Godot…
pub fn export_to_godot(app: &mut PixmithApp) {
    app.open_godot_export();
}

/// File > Re-export to Godot
pub fn reexport_to_godot(app: &mut PixmithApp) {
    app.reexport_godot();
}

/// File > Export Tileset to Godot…
pub fn export_tileset_to_godot(app: &mut PixmithApp) {
    app.open_tileset_export();
}

/// File > Re-export Tileset to Godot
pub fn reexport_tileset_to_godot(app: &mut PixmithApp) {
    app.reexport_tileset();
}

/// Enables File > Re-export Tileset to Godot.
pub fn has_godot_tileset_settings(app: &PixmithApp) -> bool {
    app.has_tileset_export_settings()
}
