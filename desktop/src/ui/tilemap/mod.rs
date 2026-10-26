//! Tilemaps, auto-tiling and slices (`TilemapView.swift`, `AutotileView.swift`, `SlicesView.swift`).
//!
//! - [`inspector_sections`] draws the Tileset (with the Terrain controls) and Slices sections.
//! - [`bar_title`] / [`bar_options`] replace the context bar's tool options while the terrain brush or the tile
//!   stamp is on (`ToolOptionsBar`).
//! - [`canvas_interact`] and [`canvas_paint`] are the canvas hooks: the terrain brush, tile stamp and slice tool,
//!   and the tile grid, missing-tile and slice overlays.
//! - The `pub fn`s taking the app (`new_tilemap_layer_sheet`, `convert_to_normal`, `toggle_stamp`, ...) are what
//!   the Tilemap, Layer and Slice menu items call.

mod bar;
mod canvas;
mod controls;
mod iso_panel;
mod sheets;
mod slices;
mod tileset;

pub use crate::calls::SliceInfo;
pub use bar::{bar_options, bar_title};
pub use canvas::{Pointer, canvas_interact, canvas_paint};
pub use sheets::show_sheets;
pub use slices::slices_section;
pub use tileset::{MenuAction, apply_menu_action, menu_items, tileset_section};

use crate::app::{DocTab, PixmithApp, Tool};
use crate::ui::iso::{self, Iso};
use eframe::egui;
use std::collections::HashMap;

/// Flip flags of a tile value (`TilemapState.flipX` ...).
pub const FLIP_X: u32 = 0x8000_0000;
pub const FLIP_Y: u32 = 0x4000_0000;
pub const FLIP_D: u32 = 0x2000_0000;

/// The six terrain tools of the context bar (`AutotileState.Tool`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainTool {
    Paint,
    Rect,
    Fill,
    Pick,
    Override,
    Auto,
}

impl TerrainTool {
    pub const ALL: [TerrainTool; 6] = [
        TerrainTool::Paint,
        TerrainTool::Rect,
        TerrainTool::Fill,
        TerrainTool::Pick,
        TerrainTool::Override,
        TerrainTool::Auto,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TerrainTool::Paint => "Brush",
            TerrainTool::Rect => "Rect",
            TerrainTool::Fill => "Fill",
            TerrainTool::Pick => "Pick",
            TerrainTool::Override => "Stamp",
            TerrainTool::Auto => "Auto",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            TerrainTool::Paint => "Paint terrain. Shift or right click erases.",
            TerrainTool::Rect => "Drag a rectangle of terrain. Shift or right click erases.",
            TerrainTool::Fill => "Flood fill a region with terrain. Shift or right click erases.",
            TerrainTool::Pick => "Pick the terrain under the cursor.",
            TerrainTool::Override => "Place the selected tile by hand (an override that auto-tiling never replaces).",
            TerrainTool::Auto => "Remove a manual tile so the automatic result returns.",
        }
    }

    pub fn icon(self) -> &'static str {
        use egui_phosphor::regular as i;
        match self {
            TerrainTool::Paint => i::PAINT_BRUSH,
            TerrainTool::Rect => i::SELECTION,
            TerrainTool::Fill => i::DROP,
            TerrainTool::Pick => i::EYEDROPPER,
            TerrainTool::Override => i::COPY,
            TerrainTool::Auto => i::MAGIC_WAND,
        }
    }
}

/// Auto-tile modes in the order of the core (`AutotileState.modeNames`).
pub const MODE_NAMES: [&str; 3] = ["Sides (16)", "Blob (47)", "Corners / dual grid (16)"];

/// The sheets of the tilemap menu (`TilemapState.Sheet`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sheet {
    NewLayer,
    Convert,
    Rename,
}

/// The values of the New Tilemap Layer and Convert to Tilemap sheets.
///
/// `iso` is the grid shape: square cells (`tile_w` x `tile_h`) or isometric diamonds (`cell_w` x `cell_h`, layout
/// Diamond Down) that `tile_w` x `tile_h` tile images, possibly taller than the diamond, sit on. `origin` is the
/// canvas pixel of the first diamond's bounding box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SizeForm {
    pub iso: bool,
    pub tile_w: u32,
    pub tile_h: u32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub origin: (i32, i32),
    pub flip_aware: bool,
}

impl Default for SizeForm {
    fn default() -> SizeForm {
        SizeForm {
            iso: false,
            tile_w: 16,
            tile_h: 16,
            cell_w: iso::DEFAULT_W as u32,
            cell_h: iso::DEFAULT_H as u32,
            origin: (0, 0),
            flip_aware: true,
        }
    }
}

impl SizeForm {
    /// Takes the cell size and origin from an isometric grid; flat tiles fill the diamond.
    pub fn use_grid(&mut self, grid: &Iso) {
        self.cell_w = grid.w as u32;
        self.cell_h = grid.h as u32;
        self.origin = (grid.x as i32, grid.y as i32);
        self.tile_w = self.cell_w;
        self.tile_h = self.cell_h;
    }

    /// Switches the shape. Isometric tile images start at the diamond's size; square ones at 16 x 16.
    pub fn set_iso(&mut self, on: bool) {
        if self.iso == on {
            return;
        }
        self.iso = on;
        if on {
            self.tile_w = self.cell_w;
            self.tile_h = self.cell_h;
        } else {
            self.tile_w = 16;
            self.tile_h = 16;
        }
    }

    /// Changes the diamond size; tile images that matched the old diamond follow it.
    pub fn set_cell(&mut self, w: u32, h: u32) {
        if self.tile_w == self.cell_w {
            self.tile_w = w;
        }
        if self.tile_h == self.cell_h {
            self.tile_h = h;
        }
        self.cell_w = w;
        self.cell_h = h;
    }

    /// Tile image size and, for isometric grids, the diamond size after clamping to what the core takes.
    pub fn tile(&self) -> (u32, u32) {
        let max = if self.iso { 1024 } else { 256 };
        (self.tile_w.clamp(1, max), self.tile_h.clamp(1, max))
    }

    pub fn cell(&self) -> (u32, u32) {
        (iso::clamp_side(self.cell_w as i32) as u32, iso::clamp_side(self.cell_h as i32) as u32)
    }
}

/// The isometric grid of the active layer when it is an isometric tilemap layer: its diamond size at its own
/// offset. View > Isometric Grid and Select > Isometric Tile follow it.
pub fn active_iso(tab: &DocTab) -> Option<Iso> {
    let layer = tab.layer;
    if !tab.doc.layer_is_iso(layer) {
        return None;
    }
    let cell = tab.doc.tileset_iso_cell(tab.doc.tileset_of_layer(layer)?)?;
    let (x, y) = tab.doc.layer_grid_offset(layer)?;
    Some(Iso::new(cell.0 as i32, cell.1 as i32, x, y))
}

/// What a drag on the canvas is doing for these tools.
#[derive(Clone, Copy, Debug)]
pub(super) enum DragKind {
    Terrain,
    Stamp { edit_open: bool },
    Slice,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Drag {
    pub kind: DragKind,
    pub button: egui::PointerButton,
    pub last: (i32, i32),
    /// The document the drag started in; a tab switch ends it.
    pub doc: usize,
}

/// The slice tool's mode for the drag in progress (`SliceState.Mode`).
#[derive(Clone, Copy, Debug)]
pub(super) enum SliceMode {
    Create,
    Move,
    Resize(i32, i32),
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SliceDrag {
    pub mode: SliceMode,
    pub start: (i32, i32),
    pub original: (i32, i32, i32, i32),
    pub began: bool,
}

/// Rectangle of the terrain rect tool while it is dragged.
#[derive(Clone, Copy, Debug)]
pub(super) struct RectDrag {
    pub start: (i32, i32),
    pub end: (i32, i32),
    pub erase: bool,
}

/// The values of the Slice Properties sheet (`SliceSheet`).
#[derive(Clone, Debug)]
pub struct SliceSheet {
    pub index: u32,
    pub name: String,
    pub nine: bool,
    pub center: [f64; 4],
    pub pivot_on: bool,
    pub pivot: [f64; 2],
    pub text: String,
    pub has_color: bool,
    pub color: egui::Color32,
}

/// The values of the User Data sheet (`UserDataSheet`).
#[derive(Clone, Debug)]
pub struct UserDataSheet {
    /// 0 sprite, 1 layer, 2 cel, 3 tag.
    pub kind: u32,
    pub tag: usize,
    pub text: String,
    pub has_color: bool,
    pub color: egui::Color32,
}

/// Cached answers of the core that cost a pass over the map; refreshed when the document changes.
#[derive(Default)]
pub(super) struct Cache {
    pub key: Option<(usize, u64, u32, u32)>,
    pub missing: Vec<(i32, i32)>,
    pub report: Vec<String>,
}

/// UI state for tilemap, terrain and slice editing that is independent of the document.
pub struct TilemapState {
    /// While on, canvas clicks place the selected tile instead of drawing pixels.
    pub stamp: bool,
    pub selected: u32,
    pub flip_x: bool,
    pub flip_y: bool,
    pub flip_d: bool,
    pub sheet: Option<Sheet>,
    /// Values of the New Tilemap Layer / Convert sheet.
    pub form: SizeForm,
    /// The isometric grid of the view settings (View > Custom Grid), where a new isometric layer starts.
    pub iso_default: Iso,
    sheet_name: String,

    /// While on, canvas clicks use the terrain tools on the current tilemap layer.
    pub brush: bool,
    pub terrain_tool: TerrainTool,
    pub terrain: u32,
    pub brush_size: u32,
    pub round: bool,
    pub show_masks: bool,
    new_name: String,
    new_mode: usize,
    weight_text: String,
    weight_of: Option<(usize, u32, u32)>,
    seed_text: String,
    last_cell: Option<(i32, i32)>,
    last_iso_cell: Option<(i32, i32)>,
    rect: Option<RectDrag>,

    /// Selected slice (view state only) and the sheets.
    pub slice_selected: Option<u32>,
    pub slice_sheet: Option<SliceSheet>,
    pub user_sheet: Option<UserDataSheet>,
    slice_drag: Option<SliceDrag>,

    drag: Option<Drag>,
    last_tool: Tool,
    last_doc: usize,
    cache: Cache,
    textures: HashMap<(usize, u32, u32), (u64, egui::TextureHandle)>,
}

impl Default for TilemapState {
    fn default() -> TilemapState {
        TilemapState {
            stamp: false,
            selected: 1,
            flip_x: false,
            flip_y: false,
            flip_d: false,
            sheet: None,
            form: SizeForm::default(),
            iso_default: Iso::new(iso::DEFAULT_W, iso::DEFAULT_H, 0, 0),
            sheet_name: String::new(),
            brush: false,
            terrain_tool: TerrainTool::Paint,
            terrain: 1,
            brush_size: 1,
            round: false,
            show_masks: false,
            new_name: "Terrain".to_string(),
            new_mode: 1,
            weight_text: "1".to_string(),
            weight_of: None,
            seed_text: "0".to_string(),
            last_cell: None,
            last_iso_cell: None,
            rect: None,
            slice_selected: None,
            slice_sheet: None,
            user_sheet: None,
            slice_drag: None,
            drag: None,
            last_tool: Tool::Pencil,
            last_doc: 0,
            cache: Cache::default(),
            textures: HashMap::new(),
        }
    }
}

impl TilemapState {
    /// The selected tile index with the current flip flags (`TilemapState.value`).
    pub fn value(&self) -> u32 {
        self.selected
            | if self.flip_x { FLIP_X } else { 0 }
            | if self.flip_y { FLIP_Y } else { 0 }
            | if self.flip_d { FLIP_D } else { 0 }
    }

    pub fn select_value(&mut self, v: u32) {
        self.selected = v & 0x1FFF_FFFF;
        self.flip_x = v & FLIP_X != 0;
        self.flip_y = v & FLIP_Y != 0;
        self.flip_d = v & FLIP_D != 0;
    }

    /// Turns stamp mode on or off; stamp mode switches the terrain brush off (`AutotileState`'s sink).
    pub fn set_stamp(&mut self, on: bool) {
        self.stamp = on;
        if on {
            self.brush = false;
        }
    }

    /// Choosing any tool, even the current one, leaves stamp mode and the terrain brush.
    pub fn leave_modes(&mut self) {
        self.stamp = false;
        self.brush = false;
    }

    /// Call once per frame: a changed tool leaves the modes, a changed document drops slice drags and selection
    /// (`resetSliceTransients`).
    pub fn sync(&mut self, tool: Tool, doc_id: usize) {
        if tool != self.last_tool {
            self.last_tool = tool;
            self.leave_modes();
        }
        if doc_id != self.last_doc {
            let first = self.last_doc == 0;
            self.last_doc = doc_id;
            if first {
                return;
            }
            self.slice_selected = None;
            self.slice_drag = None;
            self.drag = None;
            self.rect = None;
            self.last_cell = None;
            self.cache.key = None;
        }
    }

    /// The selected slice, if it still exists.
    pub fn selected_slice(&self, tab: &DocTab) -> Option<u32> {
        self.slice_selected.filter(|&i| i < tab.doc.slice_count())
    }

    /// Refreshes [`Cache`] when the document, layer or frame changed.
    pub(super) fn refresh_cache(&mut self, tab: &DocTab) {
        let key = (tab.doc.raw() as usize, tab.revision, tab.layer, tab.frame);
        if self.cache.key == Some(key) {
            return;
        }
        self.cache.key = Some(key);
        if tab.doc.layer_is_tilemap(tab.layer) && tab.doc.terrain_mode(tab.layer) >= 0 {
            self.cache.missing = tab.doc.missing_cells(tab.layer, tab.frame);
            self.cache.report = tab.doc.missing_report(tab.layer, tab.frame);
        } else {
            self.cache.missing.clear();
            self.cache.report.clear();
        }
    }
}

/// The Tileset (with Terrain) and Slices sections of the inspector, in that order. Returns true when the document
/// changed.
pub fn inspector_sections(
    ui: &mut egui::Ui,
    tab: &mut DocTab,
    tm: &mut TilemapState,
    tool: Tool,
    error: &mut Option<String>,
) -> bool {
    let mut changed = tileset_section(ui, tab, tm, error);
    changed |= slices_section(ui, tab, tm, tool);
    changed
}

// MARK: Menu entry points

/// "New Tilemap Layer…" (Tilemap menu, layers menu, Tileset section menu).
pub fn new_tilemap_layer_sheet(app: &mut PixmithApp) {
    let grid = active_iso(app.tab());
    open_sheet(&mut app.tilemap, Sheet::NewLayer, grid);
}

/// "Convert Layer to Tilemap…". See [`can_convert_to_tilemap`].
pub fn convert_layer_sheet(app: &mut PixmithApp) {
    open_sheet(&mut app.tilemap, Sheet::Convert, None);
}

/// "Convert Tilemap to Normal Layer".
pub fn convert_to_normal(app: &mut PixmithApp) {
    let tab = &mut app.tabs[app.active];
    let layer = tab.layer;
    if tab.doc.convert_layer_to_normal(layer) {
        app.tilemap.stamp = false;
        tab.changed();
    }
}

/// "Tile Stamp Mode" toggle; only meaningful on a tilemap layer (`is_tilemap_layer`).
pub fn toggle_stamp(app: &mut PixmithApp) {
    let on = !app.tilemap.stamp;
    app.tilemap.set_stamp(on);
}

/// "Rename Tileset…".
pub fn rename_tileset_sheet(app: &mut PixmithApp) {
    open_sheet(&mut app.tilemap, Sheet::Rename, None);
}

/// The current layer is a tilemap layer (enables "Convert Tilemap to Normal Layer" and "Tile Stamp Mode").
pub fn is_tilemap_layer(app: &PixmithApp) -> bool {
    app.tab().doc.layer_is_tilemap(app.tab().layer)
}

/// "Convert Layer to Tilemap…" is enabled for normal layers only.
pub fn can_convert_to_tilemap(app: &PixmithApp) -> bool {
    let tab = app.tab();
    !tab.doc.layer_is_tilemap(tab.layer) && !tab.doc.layer_is_group(tab.layer)
}

/// "Slice Properties…" (also double-click on a slice); opens the sheet for the selected slice.
pub fn slice_properties(app: &mut PixmithApp) {
    if let Some(i) = app.tilemap.selected_slice(app.tab()) {
        slices::open_slice_sheet(app, i);
    }
}

/// "Delete Slice".
pub fn delete_selected_slice(app: &mut PixmithApp) {
    let tab = &mut app.tabs[app.active];
    if slices::delete_selected(&mut app.tilemap, tab) {
        tab.changed();
    }
}

/// A slice is selected (enables "Slice Properties…" and "Delete Slice").
pub fn has_selected_slice(app: &PixmithApp) -> bool {
    app.tilemap.selected_slice(app.tab()).is_some()
}

/// "Sprite User Data…" (kind 0), "Layer User Data…" (1), "Cel User Data…" (2), "Tag User Data…" (3, needs a tag).
pub fn user_data_sheet(app: &mut PixmithApp, kind: u32) {
    slices::open_user_sheet(app, kind);
}

/// Opens a size sheet. A new layer is isometric when the active layer is, and starts on its grid (`active`);
/// otherwise the shape stays as last chosen and an isometric one starts on the view settings' isometric grid.
/// Converting is square only.
fn open_sheet(tm: &mut TilemapState, sheet: Sheet, active: Option<Iso>) {
    let was_iso = tm.form.iso;
    tm.form = SizeForm::default();
    if sheet == Sheet::NewLayer {
        let grid = active.unwrap_or(tm.iso_default);
        tm.form.use_grid(&grid);
        if active.is_some() || was_iso {
            tm.form.iso = true;
        } else {
            tm.form.tile_w = 16;
            tm.form.tile_h = 16;
        }
    }
    tm.sheet = Some(sheet);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_value_round_trips_flip_flags() {
        let mut st = TilemapState { selected: 5, flip_x: true, flip_d: true, ..Default::default() };
        let v = st.value();
        assert_eq!(v, 5 | FLIP_X | FLIP_D);
        st.select_value(7 | FLIP_Y);
        assert_eq!((st.selected, st.flip_x, st.flip_y, st.flip_d), (7, false, true, false));
    }

    #[test]
    fn stamp_and_brush_exclude_each_other_and_tool_change_leaves_both() {
        let mut st = TilemapState { brush: true, ..Default::default() };
        st.set_stamp(true);
        assert!(st.stamp && !st.brush);
        st.sync(Tool::Eraser, 1);
        assert!(!st.stamp);
        st.brush = true;
        st.sync(Tool::Eraser, 1);
        assert!(st.brush, "same tool keeps the brush");
        st.sync(Tool::Pencil, 1);
        assert!(!st.brush);
    }
}
