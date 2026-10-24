//! A safe handle around one core document.
//!
//! The front end talks to the core through the same `pixmith_*` entry points the macOS app uses, so both
//! front ends get the same behavior (panic guards, undo checkpoints, linked cels, stroke state). Add a
//! method here whenever a panel needs another entry point; keep the `unsafe` inside this file.

use pixmith_core::Editor;
use pixmith_core::ffi::{self, document as core};
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};

/// One open document.
pub struct Doc {
    ptr: *mut Editor,
    /// Where it was opened from or last saved to.
    pub path: Option<PathBuf>,
    /// A name given to a document that has no file yet (double-click its tab); Save As starts with it.
    pub name: Option<String>,
}

/// The message of the last failing core call on this thread.
pub fn last_error() -> String {
    // SAFETY: the core returns a valid, NUL-terminated thread-local string.
    unsafe { CStr::from_ptr(ffi::pixmith_last_error()) }.to_string_lossy().into_owned()
}

fn c_path(path: &Path) -> Result<CString, String> {
    CString::new(path.to_string_lossy().as_bytes()).map_err(|_| "path contains a NUL byte".to_string())
}

/// Takes ownership of a string the core allocated.
///
/// # Safety
/// `s` must be null or a string returned by the core that has not been freed.
pub(crate) unsafe fn take_string(s: *mut std::ffi::c_char) -> String {
    if s.is_null() {
        return String::new();
    }
    let out = unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned();
    unsafe { ffi::pixmith_string_free(s) };
    out
}

impl Doc {
    pub fn new(width: u32, height: u32) -> Doc {
        Doc { ptr: core::pixmith_new(width, height), path: None, name: None }
    }

    pub fn open(path: &Path) -> Result<Doc, String> {
        let c = c_path(path)?;
        // SAFETY: `c` is a valid C string for the duration of the call.
        let ptr = unsafe { core::pixmith_open(c.as_ptr()) };
        if ptr.is_null() {
            return Err(last_error());
        }
        Ok(Doc { ptr, path: Some(path.to_path_buf()), name: None })
    }

    /// Takes ownership of a document the core allocated (an import, say).
    ///
    /// # Safety
    /// `ptr` must be a live, non-null document that nothing else frees.
    pub(crate) unsafe fn from_raw(ptr: *mut Editor, path: Option<PathBuf>) -> Doc {
        Doc { ptr, path, name: None }
    }

    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        let c = c_path(path)?;
        // SAFETY: `self.ptr` is a live document; `c` outlives the call.
        if unsafe { core::pixmith_save(self.ptr, c.as_ptr()) } {
            self.path = Some(path.to_path_buf());
            Ok(())
        } else {
            Err(last_error())
        }
    }

    /// The raw document pointer, for calling entry points not wrapped yet. Prefer adding a method here.
    pub fn raw(&self) -> *mut Editor {
        self.ptr
    }

    pub fn width(&self) -> u32 {
        unsafe { core::pixmith_width(self.ptr) }
    }

    pub fn height(&self) -> u32 {
        unsafe { core::pixmith_height(self.ptr) }
    }

    pub fn layer_count(&self) -> u32 {
        unsafe { core::pixmith_layer_count(self.ptr) }
    }

    pub fn frame_count(&self) -> u32 {
        unsafe { core::pixmith_frame_count(self.ptr) }
    }

    pub fn layer_name(&self, layer: u32) -> String {
        unsafe { take_string(core::pixmith_layer_name(self.ptr, layer)) }
    }

    pub fn is_dirty(&self) -> bool {
        unsafe { pixmith_core::history::pixmith_is_dirty(self.ptr) }
    }

    /// Composites `frame` into `out` as straight-alpha RGBA, resizing it to fit.
    pub fn render(&self, frame: u32, out: &mut Vec<u8>) -> bool {
        out.resize(self.width() as usize * self.height() as usize * 4, 0);
        unsafe { core::pixmith_render(self.ptr, frame, out.as_mut_ptr(), out.len()) }
    }

    /// Marks the document changed; call after every mutation (`SpriteModel.changed()` calls `pixmith_touch`).
    pub fn touch(&mut self) {
        unsafe { pixmith_core::history::pixmith_touch(self.ptr) }
    }

    pub fn undo(&mut self) -> bool {
        unsafe { core::pixmith_undo(self.ptr) }
    }

    pub fn redo(&mut self) -> bool {
        unsafe { core::pixmith_redo(self.ptr) }
    }
}

impl Drop for Doc {
    fn drop(&mut self) {
        // SAFETY: the pointer came from the core and is freed exactly once.
        unsafe { core::pixmith_free(self.ptr) }
    }
}
