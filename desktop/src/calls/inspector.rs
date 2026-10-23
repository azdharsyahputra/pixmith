//! Core calls used by the inspector code. Keep the `unsafe` here; panels call these safe methods.
//!
//! These follow `SpriteModel.swift`, `LayerTools.swift`, `ColorModeView.swift` and `PaletteEditor.swift`: each
//! mutation touches the document (dirty flag, thumbnails) and labels its undo step the way the macOS app does.

use crate::doc::{Doc, last_error, take_string};
use pixmith_core::document::{color_mode as cm, layers};
use pixmith_core::ffi::document as core;
use pixmith_core::palette::{edit, io, presets};
use std::ffi::CString;
use std::path::Path;

fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

impl Doc {
    /// Starts an undoable edit; call once before a drag that edits live (`pixmith_begin_stroke`).
    pub fn begin_stroke(&mut self) {
        // SAFETY: live document.
        unsafe { core::pixmith_begin_stroke(self.raw()) }
    }

    // MARK: Layers

    pub fn set_layer_opacity(&mut self, layer: u32, opacity: u8) {
        unsafe { core::pixmith_set_layer_opacity(self.raw(), layer, opacity) };
        self.touch();
    }

    /// The side panel's opacity slider: one undo step per drag. Pass `first` for the first change of a drag
    /// (or for any typed value), which starts the checkpoint.
    pub fn set_layer_opacity_undoable(&mut self, layer: u32, opacity: u8, first: bool) {
        if first {
            self.begin_stroke();
            self.label_last_step("Layer Opacity");
        }
        self.set_layer_opacity(layer, opacity);
    }

    pub fn set_layer_blend_mode(&mut self, layer: u32, blend: u16) -> bool {
        let name = self.layer_name(layer);
        let (opacity, visible) = (self.layer_opacity(layer), self.layer_visible(layer));
        self.set_layer_props(layer, &name, opacity, blend, visible)
    }

    // MARK: Cels

    /// Opacity of the cel, or `None` if it is empty.
    pub fn cel_opacity(&self, layer: u32, frame: u32) -> Option<u8> {
        let v = unsafe { layers::pixmith_cel_opacity(self.raw(), layer, frame) };
        (v >= 0).then_some(v as u8)
    }

    pub fn set_cel_opacity(&mut self, layer: u32, frame: u32, opacity: u8) -> bool {
        let ok = unsafe { layers::pixmith_cel_set_opacity(self.raw(), layer, frame, opacity) };
        if ok {
            self.label_last_step("Cel Opacity");
            self.touch();
        }
        ok
    }

    pub fn unlink_cel(&mut self, layer: u32, frame: u32) -> bool {
        let ok = unsafe { layers::pixmith_cel_unlink(self.raw(), layer, frame) };
        if ok {
            self.label_last_step("Unlink Cel");
            self.touch();
        }
        ok
    }

    // MARK: Sprite

    pub fn total_duration(&self) -> u32 {
        (0..self.frame_count()).map(|f| self.frame_duration(f)).sum()
    }

    /// 0 RGB, 1 grayscale, 2 indexed (index into [`COLOR_MODES`]).
    pub fn color_mode(&self) -> u32 {
        unsafe { cm::pixmith_cm_get_mode(self.raw()) }
    }

    pub fn transparent_index(&self) -> u32 {
        unsafe { cm::pixmith_cm_transparent_index(self.raw()) }
    }

    /// Converts the sprite to `mode` (one undo step).
    pub fn convert_color_mode(&mut self, mode: u32) -> Result<(), String> {
        if !unsafe { cm::pixmith_cm_convert(self.raw(), mode, 0) } {
            return Err(last_error());
        }
        self.label_last_step("Convert Color Mode");
        self.touch();
        Ok(())
    }

    /// Constrains a drawing color to the color mode (`snapDrawColor`).
    pub fn snap_color(&self, rgba: u32) -> u32 {
        unsafe { cm::pixmith_cm_snap_color(self.raw(), rgba) }
    }

    // MARK: Palette

    pub fn palette(&self) -> Vec<u32> {
        let n = unsafe { core::pixmith_palette_count(self.raw()) };
        (0..n).map(|i| unsafe { core::pixmith_palette_color(self.raw(), i) }).collect()
    }

    /// Appends `rgba` unless the palette already has it (`addColorToPalette`). Returns true if it was added.
    pub fn palette_add(&mut self, rgba: u32) -> bool {
        let before = self.palette().len();
        unsafe { core::pixmith_palette_add(self.raw(), rgba) };
        self.touch();
        self.palette().len() > before
    }

    pub fn insert_palette_entry(&mut self, at: u32, rgba: u32) -> Result<(), String> {
        if !unsafe { edit::pixmith_pe_insert(self.raw(), at, rgba) } {
            return Err(last_error());
        }
        self.label_last_step("Insert Palette Color");
        self.touch();
        Ok(())
    }

    /// Duplicates the entries; true if anything was added.
    pub fn duplicate_palette_entries(&mut self, indices: &[u32]) -> bool {
        let n = unsafe { edit::pixmith_pe_duplicate(self.raw(), indices.as_ptr(), indices.len() as u32) };
        if n > 0 {
            self.label_last_step("Duplicate Palette Colors");
            self.touch();
        }
        n > 0
    }

    pub fn remove_palette_entries(&mut self, indices: &[u32]) -> bool {
        let ok = unsafe { edit::pixmith_pe_remove(self.raw(), indices.as_ptr(), indices.len() as u32) };
        if ok {
            self.label_last_step("Remove Palette Colors");
            self.touch();
        }
        ok
    }

    /// Live edit of an entry; `begin` starts a new edit session (one undo step).
    pub fn edit_palette_entry(&mut self, index: u32, rgba: u32, begin: bool) -> bool {
        let ok = unsafe { edit::pixmith_pe_set_live(self.raw(), index, rgba, begin) };
        if ok {
            if begin {
                self.label_last_step("Edit Palette Color");
            }
            self.touch();
        }
        ok
    }

    /// Sorts by hue (`true`) or luminance (`false`).
    pub fn sort_palette(&mut self, by_hue: bool) {
        unsafe { io::pixmith_palette_sort(self.raw(), if by_hue { 0 } else { 1 }) };
        self.label_last_step(if by_hue { "Sort Palette by Hue" } else { "Sort Palette by Luminance" });
        self.touch();
    }

    pub fn palette_preset_names() -> Vec<String> {
        let n = presets::pixmith_pe_preset_count();
        (0..n).map(|i| unsafe { take_string(presets::pixmith_pe_preset_name(i)) }).collect()
    }

    /// Replaces the palette with a built-in preset (one undo step).
    pub fn load_palette_preset(&mut self, index: u32) -> Result<(), String> {
        if !unsafe { presets::pixmith_pe_preset_load(self.raw(), index) } {
            return Err(last_error());
        }
        self.label_last_step("Load Palette Preset");
        self.touch();
        Ok(())
    }

    pub fn load_palette(&mut self, path: &Path) -> Result<(), String> {
        let c = cstring(&path.to_string_lossy());
        if !unsafe { io::pixmith_palette_load(self.raw(), c.as_ptr()) } {
            return Err(last_error());
        }
        self.label_last_step("Load Palette");
        self.touch();
        Ok(())
    }

    pub fn save_palette(&self, path: &Path) -> Result<(), String> {
        let c = cstring(&path.to_string_lossy());
        if !unsafe { io::pixmith_palette_save(self.raw(), c.as_ptr()) } {
            return Err(last_error());
        }
        Ok(())
    }
}
