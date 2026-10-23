//! Core calls for File > Import (`FileActions.swift` `importGIFAsSprite`, `importSheetWithJSON`,
//! `importSheetAsFrames`, `importPNGAsLayer`).

use crate::doc::{Doc, last_error};
use pixmith_core::formats::{animation, import, sheet_import};
use std::ffi::CString;
use std::path::Path;

fn c_path(path: &Path) -> Result<CString, String> {
    CString::new(path.to_string_lossy().as_bytes()).map_err(|_| "path contains a NUL byte".to_string())
}

/// Takes ownership of a document the core just created, or reports why it failed.
fn adopt(ptr: *mut pixmith_core::Editor) -> Result<Doc, String> {
    if ptr.is_null() {
        return Err(last_error());
    }
    // SAFETY: the core just allocated `ptr` and nothing else owns it.
    Ok(unsafe { Doc::from_raw(ptr, None) })
}

impl Doc {
    /// An animated GIF as a new sprite: frames, delays and disposal applied.
    pub fn import_gif(path: &Path) -> Result<Doc, String> {
        let c = c_path(path)?;
        adopt(unsafe { animation::pixmith_import_gif(c.as_ptr()) })
    }

    /// A sprite sheet PNG with its Aseprite JSON next to it (the PNG is found from the JSON).
    pub fn import_sheet_json(json: &Path) -> Result<Doc, String> {
        let c = c_path(json)?;
        adopt(unsafe { sheet_import::pixmith_import_sheet_json(c.as_ptr(), std::ptr::null()) })
    }

    /// A sprite sheet PNG cut into frames of `cell_w` x `cell_h`.
    pub fn import_sheet(path: &Path, cell_w: u32, cell_h: u32) -> Result<Doc, String> {
        let c = c_path(path)?;
        adopt(unsafe { import::pixmith_import_sheet(c.as_ptr(), cell_w, cell_h) })
    }

    /// A PNG as a new layer above `above` in `frame`; returns the new layer's index.
    pub fn import_png_layer(&mut self, above: u32, frame: u32, path: &Path) -> Result<u32, String> {
        let c = c_path(path)?;
        let index = unsafe { import::pixmith_import_png_layer(self.raw(), above, frame, c.as_ptr()) };
        if index < 0 {
            return Err(last_error());
        }
        self.label_last_step("Import PNG as Layer");
        self.touch();
        Ok(index as u32)
    }
}
