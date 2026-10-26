//! Core calls for tilemap layers, auto-tiling terrains, slices and user data.
//!
//! They follow `TilemapView.swift`, `AutotileView.swift` and `SlicesView.swift`: every mutation touches the
//! document and names its undo step like the macOS app. Calls the Swift app makes inside a drag (`stroke`
//! calls in `pixmith_autotile.h`, `pixmith_slice_set_bounds`) do not label anything; the caller opens the step
//! with `begin_edit` first.

use crate::doc::{Doc, take_string};
use pixmith_core::document::{slices as sl, tilemap as tm};
use pixmith_core::gamedev::autotile as at;
use std::ffi::CString;

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

/// A slice as it is at one frame (`SliceInfo`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SliceInfo {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// 9-patch center `(x, y, w, h)` relative to the slice.
    pub center: Option<(i32, i32, i32, i32)>,
    pub pivot: Option<(i32, i32)>,
}

impl Doc {
    /// Labels the newest undo step and marks the document changed when `ok` (`done` in `AutotileView.swift`).
    fn done(&mut self, ok: bool, label: &str) -> bool {
        if ok {
            self.label_last_step(label);
            self.touch();
        }
        ok
    }

    // MARK: Tilesets

    /// Tileset index of `layer`, if it is a tilemap layer.
    pub fn tileset_of_layer(&self, layer: u32) -> Option<u32> {
        let i = unsafe { tm::pixmith_layer_tileset(self.raw(), layer) };
        (i >= 0).then_some(i as u32)
    }

    pub fn tileset_total(&self) -> u32 {
        unsafe { tm::pixmith_tileset_count(self.raw()) }
    }

    pub fn tile_size(&self, ts: u32) -> (u32, u32) {
        unsafe { (tm::pixmith_tileset_tile_width(self.raw(), ts), tm::pixmith_tileset_tile_height(self.raw(), ts)) }
    }

    pub fn tile_count(&self, ts: u32) -> u32 {
        unsafe { tm::pixmith_tileset_tile_count(self.raw(), ts) }
    }

    /// The number shown on the first real tile (`tileBase`).
    pub fn tile_base(&self, ts: u32) -> i32 {
        unsafe { tm::pixmith_tileset_base_index(self.raw(), ts) }
    }

    pub fn tileset_title(&self, ts: u32) -> String {
        unsafe { take_string(tm::pixmith_tileset_name(self.raw(), ts)) }
    }

    /// One tile as straight-alpha RGBA, `tile_size` wide and high.
    pub fn tile_rgba(&self, ts: u32, tile: u32) -> Option<Vec<u8>> {
        let (w, h) = self.tile_size(ts);
        let mut buf = vec![0u8; w as usize * h as usize * 4];
        if buf.is_empty() {
            return None;
        }
        unsafe { tm::pixmith_tileset_tile_pixels(self.raw(), ts, tile, buf.as_mut_ptr(), buf.len()) }.then_some(buf)
    }

    /// Other tilesets with the tile size of `layer`'s, which it could switch to (`compatibleTilesets`).
    pub fn compatible_tilesets(&self, layer: u32) -> Vec<u32> {
        let Some(cur) = self.tileset_of_layer(layer) else { return Vec::new() };
        let key = |t: u32| (self.tile_size(t), self.tileset_iso_cell(t));
        let want = key(cur);
        (0..self.tileset_total()).filter(|&t| t != cur && key(t) == want).collect()
    }

    /// Adds a tilemap layer above `above` and returns its index (`newTilemapLayer`).
    pub fn new_tilemap_layer(&mut self, above: u32, tile_w: u32, tile_h: u32) -> u32 {
        let i = unsafe { tm::pixmith_add_tilemap_layer(self.raw(), above, tile_w, tile_h) };
        self.label_last_step("New Tilemap Layer");
        self.touch();
        i
    }

    /// Converts a normal layer to a tilemap (`convertToTilemap`). False when it is not a normal layer.
    pub fn convert_layer_to_tilemap(&mut self, layer: u32, tile_w: u32, tile_h: u32, flip_aware: bool) -> bool {
        let ok = unsafe { tm::pixmith_convert_to_tilemap_ex(self.raw(), layer, tile_w, tile_h, flip_aware) };
        self.done(ok, "Convert to Tilemap")
    }

    pub fn convert_layer_to_normal(&mut self, layer: u32) -> bool {
        let ok = unsafe { tm::pixmith_convert_to_normal(self.raw(), layer) };
        self.done(ok, "Convert to Layer")
    }

    /// Adds an empty tile and returns its index.
    pub fn add_tile(&mut self, ts: u32) -> Option<u32> {
        let i = unsafe { tm::pixmith_tileset_add_tile(self.raw(), ts) };
        if i > 0 {
            self.label_last_step("Add Tile");
        }
        self.touch();
        (i > 0).then_some(i)
    }

    pub fn change_tileset(&mut self, layer: u32, ts: u32) -> bool {
        let ok = unsafe { tm::pixmith_layer_assign_tileset(self.raw(), layer, ts) };
        self.done(ok, "Change Tileset")
    }

    pub fn rename_tileset(&mut self, ts: u32, name: &str) -> bool {
        let c = cstr(name);
        let ok = unsafe { tm::pixmith_tileset_rename(self.raw(), ts, c.as_ptr()) };
        self.done(ok, "Rename Tileset")
    }

    pub fn duplicate_tile(&mut self, ts: u32, tile: u32) -> Option<u32> {
        let n = unsafe { tm::pixmith_tileset_duplicate_tile(self.raw(), ts, tile) };
        if n > 0 {
            self.label_last_step("Duplicate Tile");
        }
        self.touch();
        (n > 0).then_some(n)
    }

    pub fn delete_tile(&mut self, ts: u32, tile: u32) -> bool {
        let ok = unsafe { tm::pixmith_tileset_delete_tile(self.raw(), ts, tile) };
        self.done(ok, "Delete Tile")
    }

    // MARK: Painting tiles

    /// Places `value` (tile index plus flip flags; 0 erases) at canvas pixel `(x, y)`. Open the undo step with
    /// `begin_edit("Stamp Tile")` first.
    pub fn stamp_tile(&mut self, layer: u32, frame: u32, x: i32, y: i32, value: u32) -> bool {
        let ok = unsafe { tm::pixmith_tile_stamp(self.raw(), layer, frame, x, y, value) };
        if ok {
            self.touch();
        }
        ok
    }

    /// Tile value (index plus flip flags) under canvas pixel `(x, y)`.
    pub fn tile_value_at(&self, layer: u32, frame: u32, x: i32, y: i32) -> u32 {
        unsafe { tm::pixmith_tile_at(self.raw(), layer, frame, x, y) }
    }

    /// Offset of the tile grid inside the canvas, each in `[0, tile size)`.
    pub fn tile_grid_origin(&self, layer: u32, frame: u32) -> (i32, i32) {
        let mut out = [0i32; 2];
        unsafe { tm::pixmith_tile_grid_origin(self.raw(), layer, frame, out.as_mut_ptr()) };
        (out[0], out[1])
    }

    // MARK: Isometric tilemaps

    /// Adds an isometric tilemap layer above `above`: `tile_w` x `tile_h` images on `cell_w` x `cell_h` diamonds,
    /// cell (0, 0)'s box at canvas pixel `offset`. Returns the layer's index.
    pub fn new_iso_tilemap_layer(&mut self, above: u32, tile: (u32, u32), cell: (u32, u32), offset: (i32, i32)) -> u32 {
        let i = unsafe {
            tm::pixmith_add_iso_tilemap_layer(self.raw(), above, tile.0, tile.1, cell.0, cell.1, offset.0, offset.1)
        };
        self.label_last_step("New Isometric Tilemap Layer");
        self.touch();
        i
    }

    /// Makes an unused tileset isometric with `cell` diamonds, or square again with `None` (false while a layer
    /// shows a tile of it).
    pub fn set_tileset_iso(&mut self, ts: u32, cell: Option<(u32, u32)>) -> bool {
        let (w, h) = cell.unwrap_or((0, 0));
        let ok = unsafe { tm::pixmith_tileset_set_iso(self.raw(), ts, w, h) };
        self.done(ok, if cell.is_some() { "Make Tileset Isometric" } else { "Make Tileset Square" })
    }

    pub fn tileset_is_iso(&self, ts: u32) -> bool {
        unsafe { tm::pixmith_tileset_is_iso(self.raw(), ts) }
    }

    /// The diamond cell size of an isometric tileset.
    pub fn tileset_iso_cell(&self, ts: u32) -> Option<(u32, u32)> {
        let mut out = [0u32; 2];
        unsafe { tm::pixmith_tileset_iso_cell(self.raw(), ts, out.as_mut_ptr()) }.then_some((out[0], out[1]))
    }

    /// The layer is an isometric tilemap layer.
    pub fn layer_is_iso(&self, layer: u32) -> bool {
        unsafe { tm::pixmith_layer_is_iso(self.raw(), layer) }
    }

    /// Canvas pixel of cell (0, 0)'s box of an isometric layer (its elevation offset).
    pub fn layer_grid_offset(&self, layer: u32) -> Option<(i32, i32)> {
        let mut out = [0i32; 2];
        unsafe { tm::pixmith_layer_grid_offset(self.raw(), layer, out.as_mut_ptr()) }.then_some((out[0], out[1]))
    }

    pub fn set_layer_grid_offset(&mut self, layer: u32, offset: (i32, i32)) -> bool {
        let ok = unsafe { tm::pixmith_layer_set_grid_offset(self.raw(), layer, offset.0, offset.1) };
        self.done(ok, "Layer Offset")
    }

    /// The cell containing canvas pixel `(x, y)` (diamond for isometric layers, `pixel / tile size` otherwise).
    pub fn layer_cell_at(&self, layer: u32, x: i32, y: i32) -> Option<(i32, i32)> {
        let mut out = [0i32; 2];
        unsafe { tm::pixmith_layer_cell_at(self.raw(), layer, x, y, out.as_mut_ptr()) }.then_some((out[0], out[1]))
    }

    /// Canvas pixel of the top-left corner of cell `(i, j)`'s box.
    pub fn layer_cell_origin(&self, layer: u32, i: i32, j: i32) -> Option<(i32, i32)> {
        let mut out = [0i32; 2];
        unsafe { tm::pixmith_layer_cell_origin(self.raw(), layer, i, j, out.as_mut_ptr()) }.then_some((out[0], out[1]))
    }

    /// The tile value in cell `(i, j)`.
    pub fn tile_value_in_cell(&self, layer: u32, frame: u32, i: i32, j: i32) -> u32 {
        unsafe { tm::pixmith_tile_in_cell(self.raw(), layer, frame, i, j) }
    }

    /// Like [`Doc::stamp_tile`], by cell. Open the undo step with `begin_edit` first.
    pub fn stamp_tile_cell(&mut self, layer: u32, frame: u32, i: i32, j: i32, value: u32) -> bool {
        let ok = unsafe { tm::pixmith_tile_stamp_cell(self.raw(), layer, frame, i, j, value) };
        if ok {
            self.touch();
        }
        ok
    }

    /// The tile you see at canvas pixel `(x, y)` of an isometric layer: its cell and tile value.
    pub fn tile_hit_at(&self, layer: u32, frame: u32, x: i32, y: i32) -> Option<((i32, i32), u32)> {
        let mut out = [0i32; 3];
        unsafe { tm::pixmith_tile_hit_at(self.raw(), layer, frame, x, y, out.as_mut_ptr()) }
            .then_some(((out[0], out[1]), out[2] as u32))
    }

    /// The point of a tile's image (from its top-left corner) that sits on the diamond's center.
    pub fn tile_pivot(&self, ts: u32, tile: u32) -> Option<(i32, i32)> {
        let mut out = [0i32; 2];
        unsafe { tm::pixmith_tile_pivot(self.raw(), ts, tile, out.as_mut_ptr()) }.then_some((out[0], out[1]))
    }

    pub fn tile_pivot_is_set(&self, ts: u32, tile: u32) -> bool {
        unsafe { tm::pixmith_tile_pivot_is_set(self.raw(), ts, tile) }
    }

    /// The pivot as Godot's `texture_origin` (offset from the image center).
    pub fn tile_texture_origin(&self, ts: u32, tile: u32) -> Option<(i32, i32)> {
        let mut out = [0i32; 2];
        unsafe { tm::pixmith_tile_texture_origin(self.raw(), ts, tile, out.as_mut_ptr()) }.then_some((out[0], out[1]))
    }

    pub fn set_tile_pivot(&mut self, ts: u32, tile: u32, pivot: (i32, i32)) -> bool {
        let ok = unsafe { tm::pixmith_tile_set_pivot(self.raw(), ts, tile, pivot.0, pivot.1) };
        self.done(ok, "Tile Origin")
    }

    pub fn clear_tile_pivot(&mut self, ts: u32, tile: u32) -> bool {
        let ok = unsafe { tm::pixmith_tile_clear_pivot(self.raw(), ts, tile) };
        self.done(ok, "Reset Tile Origin")
    }

    // MARK: Terrains

    /// The layer's auto-tile mode (0 sides, 1 blob, 2 corners), or -1 without terrain data.
    pub fn terrain_mode(&self, layer: u32) -> i32 {
        unsafe { at::pixmith_autotile_mode(self.raw(), layer) }
    }

    /// Terrain ids in priority order.
    pub fn terrain_ids(&self, layer: u32) -> Vec<u32> {
        let n = unsafe { at::pixmith_autotile_terrain_count(self.raw(), layer) };
        (0..n).map(|i| unsafe { at::pixmith_autotile_terrain_id(self.raw(), layer, i) }).collect()
    }

    pub fn terrain_name(&self, layer: u32, id: u32) -> String {
        let p = unsafe { at::pixmith_autotile_terrain_name(self.raw(), layer, id) };
        if p.is_null() { "?".to_string() } else { unsafe { take_string(p) } }
    }

    /// 0xRRGGBBAA.
    pub fn terrain_color(&self, layer: u32, id: u32) -> u32 {
        unsafe { at::pixmith_autotile_terrain_color(self.raw(), layer, id) }
    }

    pub fn terrain_at(&self, layer: u32, frame: u32, x: i32, y: i32) -> u32 {
        unsafe { at::pixmith_autotile_terrain_at(self.raw(), layer, frame, x, y) }
    }

    /// Creates the first terrain and turns auto-tiling on (`createTerrain`).
    pub fn create_terrain(&mut self, layer: u32, name: &str, mode: i32) -> bool {
        let c = cstr(name);
        let ok = unsafe { at::pixmith_autotile_set_terrain(self.raw(), layer, c.as_ptr(), 0x5AAA_46FF, mode) };
        self.done(ok, "Set Terrain")
    }

    /// Adds "Terrain N" in the next palette color and returns its id (`addTerrain`).
    pub fn add_terrain(&mut self, layer: u32) -> Option<u32> {
        const COLORS: [u32; 5] = [0x8B5A2BFF, 0x3C78D8FF, 0xE6B422FF, 0x9E9E9EFF, 0xC0504DFF];
        let n = unsafe { at::pixmith_autotile_terrain_count(self.raw(), layer) } as usize;
        let name = cstr(&format!("Terrain {}", n + 1));
        let id =
            unsafe { at::pixmith_autotile_add_terrain(self.raw(), layer, name.as_ptr(), COLORS[n % COLORS.len()]) };
        self.done(id != 0, "Add Terrain");
        (id != 0).then_some(id)
    }

    pub fn rename_terrain(&mut self, layer: u32, id: u32, name: &str) -> bool {
        let color = self.terrain_color(layer, id);
        let c = cstr(name);
        let ok = unsafe { at::pixmith_autotile_edit_terrain(self.raw(), layer, id, c.as_ptr(), color) };
        self.done(ok, "Rename Terrain")
    }

    pub fn remove_terrain(&mut self, layer: u32, id: u32) -> bool {
        let ok = unsafe { at::pixmith_autotile_remove_terrain(self.raw(), layer, id) };
        self.done(ok, "Remove Terrain")
    }

    /// Moves a terrain to priority position `to` (0 is the highest).
    pub fn move_terrain(&mut self, layer: u32, id: u32, to: i64) -> bool {
        let ok = unsafe { at::pixmith_autotile_move_terrain(self.raw(), layer, id, to.max(0) as u32) };
        self.done(ok, "Terrain Priority")
    }

    pub fn set_terrain_mode(&mut self, layer: u32, mode: i32) -> bool {
        let ok = unsafe { at::pixmith_autotile_set_mode(self.raw(), layer, mode) };
        self.done(ok, "Terrain Mode")
    }

    // MARK: Tile data

    /// A tile's mask in the layer's mode, or -1 when it has no terrain data.
    pub fn tile_mask(&self, layer: u32, tile: u32) -> i32 {
        unsafe { at::pixmith_autotile_tile_mask(self.raw(), layer, tile) }
    }

    pub fn tile_terrain(&self, layer: u32, tile: u32) -> i32 {
        unsafe { at::pixmith_autotile_tile_terrain(self.raw(), layer, tile) }
    }

    pub fn tile_peer(&self, layer: u32, tile: u32, dir: u32) -> i32 {
        unsafe { at::pixmith_autotile_tile_peer(self.raw(), layer, tile, dir) }
    }

    /// Flip bits a tile may be reused with: 1 X, 2 Y, 4 diagonal.
    pub fn tile_sym(&self, layer: u32, tile: u32) -> u32 {
        unsafe { at::pixmith_autotile_tile_sym(self.raw(), layer, tile) }
    }

    pub fn tile_weight(&self, layer: u32, tile: u32) -> f32 {
        unsafe { at::pixmith_autotile_tile_weight(self.raw(), layer, tile) }
    }

    /// `mask < 0` removes the tile's terrain data.
    pub fn set_tile_mask(&mut self, layer: u32, tile: u32, mask: i32) -> bool {
        let ok = unsafe { at::pixmith_autotile_set_tile_mask(self.raw(), layer, tile, mask) };
        self.done(ok, "Set Tile Mask")
    }

    pub fn set_tile_weight(&mut self, layer: u32, tile: u32, weight: f32) -> bool {
        let ok = unsafe { at::pixmith_autotile_set_tile_weight(self.raw(), layer, tile, weight) };
        self.done(ok, "Set Tile Weight")
    }

    pub fn set_tile_peer(&mut self, layer: u32, tile: u32, dir: u32, terrain: u32) -> bool {
        let ok = unsafe { at::pixmith_autotile_set_tile_peer(self.raw(), layer, tile, dir, terrain) };
        self.done(ok, "Set Tile Peer")
    }

    pub fn set_tile_terrain(&mut self, layer: u32, tile: u32, terrain: u32) -> bool {
        let ok = unsafe { at::pixmith_autotile_set_tile_terrain(self.raw(), layer, tile, terrain) };
        self.done(ok, "Set Tile Terrain")
    }

    pub fn set_tile_sym(&mut self, layer: u32, tile: u32, sym: u32) -> bool {
        let ok = unsafe { at::pixmith_autotile_set_tile_sym(self.raw(), layer, tile, sym) };
        self.done(ok, "Set Tile Symmetry")
    }

    pub fn generate_template(&mut self, layer: u32, reduced: bool) -> bool {
        let n = unsafe { at::pixmith_autotile_generate_template(self.raw(), layer, reduced) };
        self.done(n > 0, "Generate Template")
    }

    pub fn assign_template_masks(&mut self, layer: u32) -> bool {
        let n = unsafe { at::pixmith_autotile_assign_masks_from_template(self.raw(), layer) };
        self.done(n > 0, "Assign Masks")
    }

    pub fn rebuild_terrain(&mut self, layer: u32) -> bool {
        let ok = unsafe { at::pixmith_autotile_rebuild(self.raw(), layer) };
        self.done(ok, "Rebuild Auto-tiles")
    }

    pub fn clear_overrides(&mut self, layer: u32, frame: u32) -> bool {
        let ok = unsafe { at::pixmith_autotile_clear_overrides(self.raw(), layer, frame) };
        self.done(ok, "Clear Manual Tiles")
    }

    pub fn terrain_edge_same(&self, layer: u32) -> bool {
        unsafe { at::pixmith_autotile_edge_same(self.raw(), layer) }
    }

    pub fn set_terrain_edge_same(&mut self, layer: u32, on: bool) -> bool {
        let ok = unsafe { at::pixmith_autotile_set_edge_same(self.raw(), layer, on) };
        self.done(ok, "Map Edge Option")
    }

    pub fn terrain_seed(&self, layer: u32) -> u32 {
        unsafe { at::pixmith_autotile_seed(self.raw(), layer) }
    }

    pub fn set_terrain_seed(&mut self, layer: u32, seed: u32) -> bool {
        let ok = unsafe { at::pixmith_autotile_set_seed(self.raw(), layer, seed) };
        self.done(ok, "Variation Seed")
    }

    // MARK: Terrain brush (stroke calls: `begin_edit` first)

    #[allow(clippy::too_many_arguments)]
    pub fn paint_terrain_brush(
        &mut self,
        layer: u32,
        frame: u32,
        x: i32,
        y: i32,
        terrain: u32,
        size: u32,
        round: bool,
        mirror: u32,
    ) -> bool {
        let ok =
            unsafe { at::pixmith_autotile_paint_brush(self.raw(), layer, frame, x, y, terrain, size, round, mirror) };
        if ok {
            self.touch();
        }
        ok
    }

    #[allow(clippy::too_many_arguments)]
    pub fn paint_terrain_rect(
        &mut self,
        layer: u32,
        frame: u32,
        a: (i32, i32),
        b: (i32, i32),
        terrain: u32,
        mirror: u32,
    ) -> bool {
        let ok = unsafe { at::pixmith_autotile_rect(self.raw(), layer, frame, a.0, a.1, b.0, b.1, terrain, mirror) };
        if ok {
            self.touch();
        }
        ok
    }

    pub fn flood_terrain(&mut self, layer: u32, frame: u32, x: i32, y: i32, terrain: u32, mirror: u32) -> bool {
        let ok = unsafe { at::pixmith_autotile_flood(self.raw(), layer, frame, x, y, terrain, mirror) };
        if ok {
            self.touch();
        }
        ok
    }

    /// A manual tile that auto-tiling never replaces (`tile` 0 clears it).
    pub fn set_tile_override(&mut self, layer: u32, frame: u32, x: i32, y: i32, tile: u32) -> bool {
        let ok = unsafe { at::pixmith_autotile_set_override(self.raw(), layer, frame, x, y, tile) };
        if ok {
            self.touch();
        }
        ok
    }

    /// Top-left canvas pixels of cells that use a fallback tile because no tile has their exact mask.
    pub fn missing_cells(&self, layer: u32, frame: u32) -> Vec<(i32, i32)> {
        let n = unsafe { at::pixmith_autotile_missing_cells(self.raw(), layer, frame, std::ptr::null_mut(), 0) };
        if n == 0 {
            return Vec::new();
        }
        let mut buf = vec![0i32; n * 2];
        unsafe { at::pixmith_autotile_missing_cells(self.raw(), layer, frame, buf.as_mut_ptr(), buf.len()) };
        (0..n).map(|k| (buf[k * 2], buf[k * 2 + 1])).collect()
    }

    pub fn missing_report(&self, layer: u32, frame: u32) -> Vec<String> {
        let p = unsafe { at::pixmith_autotile_missing_report(self.raw(), layer, frame) };
        if p.is_null() {
            return Vec::new();
        }
        unsafe { take_string(p) }.split('\n').filter(|l| !l.is_empty()).map(String::from).collect()
    }

    // MARK: Slices

    pub fn slice_count(&self) -> u32 {
        unsafe { sl::pixmith_slice_count(self.raw()) }
    }

    pub fn slice_name(&self, i: u32) -> String {
        unsafe { take_string(sl::pixmith_slice_name(self.raw(), i)) }
    }

    /// The slice at `frame`, or `None` when it has no key at or before it.
    pub fn slice_info(&self, i: u32, frame: u32) -> Option<SliceInfo> {
        let mut v = [0i32; 13];
        if !unsafe { sl::pixmith_slice_get(self.raw(), i, frame, v.as_mut_ptr()) } {
            return None;
        }
        Some(SliceInfo {
            x: v[0],
            y: v[1],
            w: v[2],
            h: v[3],
            center: (v[4] != 0).then_some((v[5], v[6], v[7], v[8])),
            pivot: (v[9] != 0).then_some((v[10], v[11])),
        })
    }

    /// Creates a slice (one undo step, named "New Slice") and returns its index.
    pub fn add_slice(&mut self, x: i32, y: i32, w: u32, h: u32) -> Option<u32> {
        let i = unsafe { sl::pixmith_slice_add(self.raw(), x, y, w, h) };
        if i < 0 {
            return None;
        }
        self.label_last_step("New Slice");
        self.touch();
        Some(i as u32)
    }

    /// Moves or resizes a slice during a drag; open the step with `begin_edit` first.
    pub fn set_slice_bounds(&mut self, i: u32, frame: u32, x: i32, y: i32, w: u32, h: u32) -> bool {
        let ok = unsafe { sl::pixmith_slice_set_bounds(self.raw(), i, frame, x, y, w, h) };
        if ok {
            self.touch();
        }
        ok
    }

    /// Name, 9-patch center and pivot of a slice (no undo step of its own: wrap it in `in_transaction`).
    pub fn set_slice_props(
        &mut self,
        i: u32,
        frame: u32,
        name: &str,
        center: Option<[i32; 4]>,
        pivot: Option<[i32; 2]>,
    ) -> bool {
        let c = cstr(name);
        let cp = center.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
        let pp = pivot.as_ref().map_or(std::ptr::null(), |p| p.as_ptr());
        // SAFETY: `cp` and `pp` are null or point at arrays that outlive the call.
        unsafe { sl::pixmith_slice_set_props(self.raw(), i, frame, c.as_ptr(), cp, pp) }
    }

    pub fn remove_slice(&mut self, i: u32) -> bool {
        let ok = unsafe { sl::pixmith_slice_remove(self.raw(), i) };
        self.done(ok, "Delete Slice")
    }

    // MARK: User data

    /// Text of a user-data target: kind 0 sprite, 1 layer (a), 2 cel (a frame, b layer), 3 tag (a), 4 slice (a).
    pub fn user_data_text(&self, kind: u32, a: u32, b: u32) -> String {
        unsafe { take_string(sl::pixmith_userdata_text(self.raw(), kind, a, b)) }
    }

    pub fn user_data_color(&self, kind: u32, a: u32, b: u32) -> Option<u32> {
        let mut c = 0u32;
        unsafe { sl::pixmith_userdata_color(self.raw(), kind, a, b, &mut c) }.then_some(c)
    }

    /// Sets text and optional color of a target as one undo step (not labelled; see `label_last_step`).
    pub fn set_user_data(&mut self, kind: u32, a: u32, b: u32, text: &str, color: Option<u32>) -> bool {
        let c = cstr(text);
        unsafe { sl::pixmith_userdata_set(self.raw(), kind, a, b, c.as_ptr(), color.is_some(), color.unwrap_or(0)) }
    }
}
