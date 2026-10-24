//! Core calls for WebP, JPEG, BMP, TGA, ICO, PCX and FLI/FLC (`pixmith_formats.h`; the macOS counterpart is
//! `Dialogs/ImageFormats.swift`): opening them as new sprites and exporting a frame or the animation.

use crate::doc::{Doc, last_error};
use pixmith_core::formats::json::{self, Value};
use pixmith_core::formats::raster::abi;
use std::ffi::{CStr, CString};
use std::path::Path;

fn c_path(path: &Path) -> Result<CString, String> {
    CString::new(path.to_string_lossy().as_bytes()).map_err(|_| "path contains a NUL byte".to_string())
}

/// What `pixmith_image_info` says about a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    /// Extension of the format ("ico", "webp", ...).
    pub format: String,
    pub name: String,
    pub animated: bool,
    /// For icons: the larger side of each image, ascending.
    pub sizes: Vec<u32>,
}

impl Doc {
    /// Opens any supported image file (format detected from the content) as a new sprite. `ico_size` picks one size of
    /// an icon; without it the largest opens.
    pub fn import_image(path: &Path, ico_size: Option<u32>) -> Result<Doc, String> {
        let c = c_path(path)?;
        let options = CString::new(ico_size.map_or(String::new(), |s| format!("{{\"ico_size\": {s}}}")))
            .map_err(|_| "invalid options".to_string())?;
        // SAFETY: both strings are valid for the duration of the call.
        let ptr = unsafe { abi::pixmith_import_image_ex(c.as_ptr(), options.as_ptr()) };
        if ptr.is_null() {
            return Err(last_error());
        }
        // SAFETY: the core just allocated `ptr` and nothing else owns it.
        Ok(unsafe { Doc::from_raw(ptr, None) })
    }

    /// What kind of image a file is and, for icons, which sizes it holds.
    pub fn image_info(path: &Path) -> Result<ImageInfo, String> {
        let c = c_path(path)?;
        // SAFETY: `c` is valid for the call; the result is freed below.
        let ptr = unsafe { abi::pixmith_image_info(c.as_ptr()) };
        if ptr.is_null() {
            return Err(last_error());
        }
        // SAFETY: the core returned a NUL-terminated string that we free right after copying it.
        let text = unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() };
        // SAFETY: `ptr` came from `pixmith_image_info`.
        unsafe { pixmith_core::ffi::pixmith_string_free(ptr) };
        let v = json::parse(&text)?;
        let text_of = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        Ok(ImageInfo {
            format: text_of("format"),
            name: text_of("name"),
            animated: matches!(v.get("animated"), Some(Value::Bool(true))),
            sizes: v
                .get("sizes")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_f64).map(|n| n as u32).collect())
                .unwrap_or_default(),
        })
    }

    /// Writes one frame (0-based; `None`: the whole animation in WebP and FLC) at `scale` as the image format of the
    /// file name. `options_json` holds the format settings (see `pixmith_formats.h`).
    pub fn export_image(&self, frame: Option<u32>, scale: u32, path: &Path, options_json: &str) -> Result<(), String> {
        let c = c_path(path)?;
        let options = CString::new(options_json).map_err(|_| "invalid options".to_string())?;
        // SAFETY: `self.raw()` is live; both strings outlive the call.
        let ok = unsafe {
            abi::pixmith_export_image(self.raw(), frame.unwrap_or(abi::ALL_FRAMES), scale, c.as_ptr(), options.as_ptr())
        };
        if ok { Ok(()) } else { Err(last_error()) }
    }
}

#[cfg(test)]
mod tests {
    include!("images_tests.rs");
}
