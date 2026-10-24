//! Core calls for the color profile, cel z-index and reference layers (`SpriteDataUI.swift`).

use crate::doc::{Doc, last_error, take_string};
use pixmith_core::document::spritedata as core;
use std::ffi::CString;
use std::path::Path;

impl Doc {
    // MARK: Color profile

    /// 0 none, 1 sRGB, 2 embedded ICC.
    pub fn color_profile_kind(&self) -> i32 {
        unsafe { core::pixmith_color_profile_kind(self.raw()) }
    }

    /// "None", "sRGB" or "Embedded ICC (name)".
    pub fn color_profile_label(&self) -> String {
        unsafe { take_string(core::pixmith_color_profile_label(self.raw())) }
    }

    /// Assigns sRGB (1) or removes the profile (0); pixels are not converted.
    pub fn set_color_profile(&mut self, kind: u32) -> bool {
        let ok = unsafe { core::pixmith_color_profile_set(self.raw(), kind) };
        if ok {
            self.label_last_step(if kind == 0 { "Remove Color Profile" } else { "Assign sRGB Profile" });
            self.touch();
        }
        ok
    }

    // MARK: Cel z-index

    /// The z-index of a cel, or `None` when the cel is empty.
    pub fn cel_z_index(&self, layer: u32, frame: u32) -> Option<i32> {
        let mut z = 0i32;
        unsafe { core::pixmith_cel_z_index(self.raw(), layer, frame, &mut z) }.then_some(z)
    }

    pub fn set_cel_z_index(&mut self, layer: u32, frame: u32, z: i32) -> bool {
        let ok = unsafe { core::pixmith_cel_set_z_index(self.raw(), layer, frame, z) };
        if ok {
            self.label_last_step("Cel Z-Index");
            self.touch();
        }
        ok
    }

    // MARK: Reference layers

    pub fn layer_is_reference(&self, layer: u32) -> bool {
        unsafe { core::pixmith_layer_is_reference(self.raw(), layer) }
    }

    pub fn layer_is_locked(&self, layer: u32) -> bool {
        unsafe { core::pixmith_layer_is_locked(self.raw(), layer) }
    }

    /// Turns a layer into a locked reference layer, or back into a normal one.
    pub fn set_layer_reference(&mut self, layer: u32, on: bool) -> bool {
        let ok = unsafe { core::pixmith_layer_set_reference(self.raw(), layer, on) };
        if ok {
            self.label_last_step(if on { "Reference Layer" } else { "Normal Layer" });
            self.touch();
        }
        ok
    }

    pub fn set_layer_locked(&mut self, layer: u32, locked: bool) -> bool {
        let ok = unsafe { core::pixmith_layer_set_locked(self.raw(), layer, locked) };
        if ok {
            self.label_last_step(if locked { "Lock Layer" } else { "Unlock Layer" });
            self.touch();
        }
        ok
    }

    /// New Reference Layer from File: any image the `image` crate reads, scaled to fit the canvas, above `above`.
    pub fn new_reference_layer(&mut self, above: u32, path: &Path) -> Result<u32, String> {
        let img = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?.into_rgba8();
        let name = path.file_stem().map_or_else(|| "Reference".to_string(), |n| n.to_string_lossy().into_owned());
        self.new_reference_layer_rgba(above, &name, img.as_raw(), img.width(), img.height())
    }

    pub fn new_reference_layer_rgba(
        &mut self,
        above: u32,
        name: &str,
        rgba: &[u8],
        w: u32,
        h: u32,
    ) -> Result<u32, String> {
        if rgba.len() < w as usize * h as usize * 4 {
            return Err("the image is smaller than its size says".into());
        }
        let name = CString::new(name.replace('\0', "")).unwrap_or_default();
        let index =
            unsafe { core::pixmith_layer_new_reference_rgba(self.raw(), above, name.as_ptr(), rgba.as_ptr(), w, h) };
        if index < 0 {
            return Err(last_error());
        }
        self.label_last_step("New Reference Layer");
        self.touch();
        Ok(index as u32)
    }

    /// How opaque the editor draws reference layers, in percent (0 hides them). A view setting, not an edit.
    pub fn reference_display(&self) -> u32 {
        unsafe { core::pixmith_reference_display(self.raw()) }
    }

    pub fn set_reference_display(&mut self, percent: u32) {
        unsafe { core::pixmith_set_reference_display(self.raw(), percent) }
    }
}
