//! A safe handle around one core document.
//!
//! The front end talks to the core through the same `pixmith_*` entry points the macOS app uses, so both
//! front ends get the same behavior (panic guards, undo checkpoints, linked cels, stroke state). Add a
//! method here whenever a panel needs another entry point; keep the `unsafe` inside this file.

use pixmith_core::Editor;
use pixmith_core::ffi::{self, document as core, live};
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};

/// One open document.
pub struct Doc {
    ptr: *mut Editor,
    /// Where it was opened from or last saved to.
    pub path: Option<PathBuf>,
    /// A name given to a document that has no file yet (double-click its tab); Save As starts with it.
    pub name: Option<String>,
    /// The image, GIF or sheet an imported document came from: Save As starts in its folder with its name.
    pub import_source: Option<PathBuf>,
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
        Doc { ptr: core::pixmith_new(width, height), path: None, name: None, import_source: None }
    }

    pub fn open(path: &Path) -> Result<Doc, String> {
        let c = c_path(path)?;
        // SAFETY: `c` is a valid C string for the duration of the call.
        let ptr = unsafe { core::pixmith_open(c.as_ptr()) };
        if ptr.is_null() {
            return Err(last_error());
        }
        Ok(Doc { ptr, path: Some(path.to_path_buf()), name: None, import_source: None })
    }

    /// Takes ownership of a document the core allocated (an import, say).
    ///
    /// # Safety
    /// `ptr` must be a live, non-null document that nothing else frees.
    pub(crate) unsafe fn from_raw(ptr: *mut Editor, path: Option<PathBuf>) -> Doc {
        Doc { ptr, path, name: None, import_source: None }
    }

    /// Remembers the file an import was read from (see `import_source`).
    pub fn with_import_source(mut self, source: &Path) -> Doc {
        self.import_source = Some(source.to_path_buf());
        self
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

    /// How `frame` differs from the frame before it (`wrap`: the first frame of a tag compares with that tag's last
    /// one, and frames outside tags wrap over the document; otherwise frame 0 has no reference): the composite of the
    /// visible layers, or one layer's cel (`layer`). `mask` is resized to one byte per pixel (0 same, 1 changed,
    /// 2 appeared, 3 disappeared). `None` for a frame or layer that does not exist.
    pub fn frame_diff(
        &self,
        frame: u32,
        layer: Option<u32>,
        wrap: bool,
        mask: &mut Vec<u8>,
    ) -> Option<pixmith_core::render::diff::PixmithFrameDiff> {
        use pixmith_core::render::diff::{PixmithFrameDiff, pixmith_frame_diff_mode};
        mask.resize(self.width() as usize * self.height() as usize, 0);
        let mut out = PixmithFrameDiff::default();
        let layer = layer.map_or(-1, |l| l.min(i32::MAX as u32) as i32);
        let mode = if wrap { 0 } else { 2 }; // tag, none
        // SAFETY: `self.ptr` is a live document and `mask` holds `mask.len()` bytes.
        unsafe { pixmith_frame_diff_mode(self.ptr, frame, layer, mode, mask.as_mut_ptr(), mask.len(), &mut out) }
            .then_some(out)
    }

    /// Marks the document changed; call after every mutation (`SpriteModel.changed()` calls `pixmith_touch`).
    pub fn touch(&mut self) {
        unsafe { pixmith_core::history::pixmith_touch(self.ptr) }
    }

    /// Unique for the life of the process, unlike the document's address.
    pub fn uid(&self) -> u64 {
        // SAFETY: `self.ptr` is a live document.
        unsafe { (*self.ptr).uid() }
    }

    /// Identifies the state of the document's history; it differs after every edit, undo and redo.
    pub fn state_id(&self) -> u64 {
        // SAFETY: `self.ptr` is a live document.
        unsafe { (*self.ptr).states.current_id() }
    }

    /// The document as `.pxmt` bytes (Live mode).
    pub fn encode(&mut self) -> Result<Vec<u8>, String> {
        let mut len = 0usize;
        // SAFETY: `self.ptr` is a live document and `len` is a valid out pointer.
        let bytes = unsafe { live::pixmith_encode_document(self.ptr, &mut len) };
        if bytes.is_null() {
            return Err(last_error());
        }
        // SAFETY: the core returned `len` readable bytes, which are freed right after the copy.
        let out = unsafe { std::slice::from_raw_parts(bytes, len) }.to_vec();
        unsafe { live::pixmith_bytes_free(bytes, len) };
        Ok(out)
    }

    /// A new, unnamed document from `.pxmt` or `.aseprite` bytes (Live mode). Nothing is created on failure.
    pub fn from_bytes(bytes: &[u8]) -> Result<Doc, String> {
        // SAFETY: the bytes are readable for their length; a non-null result is a document we now own.
        let ptr = unsafe { live::pixmith_decode_document(bytes.as_ptr(), bytes.len()) };
        if ptr.is_null() {
            return Err(last_error());
        }
        Ok(Doc { ptr, path: None, name: None, import_source: None })
    }

    /// Swaps in `.pxmt` or `.aseprite` bytes as one undo step named `label` (Live mode). The document is
    /// unchanged on failure.
    pub fn apply_document(&mut self, bytes: &[u8], label: &str) -> Result<(), String> {
        let label = CString::new(label.replace('\0', "")).unwrap_or_default();
        // SAFETY: `self.ptr` is a live document; the bytes and the label outlive the call.
        if unsafe { live::pixmith_apply_document(self.ptr, bytes.as_ptr(), bytes.len(), label.as_ptr()) } {
            Ok(())
        } else {
            Err(last_error())
        }
    }

    /// The selection's bounding rectangle with one mask byte per pixel (Live mode); `None` when nothing is selected.
    pub fn live_selection(&self) -> Option<live::Region> {
        let (mut x, mut y, mut w, mut h, mut len) = (0i32, 0i32, 0u32, 0u32, 0usize);
        // SAFETY: `self.ptr` is a live document and every out pointer is valid.
        let bytes = unsafe { live::pixmith_live_selection(self.ptr, &mut x, &mut y, &mut w, &mut h, &mut len) };
        if bytes.is_null() {
            return None;
        }
        // SAFETY: the core returned `len` readable bytes, which are freed right after the copy.
        let mask = unsafe { std::slice::from_raw_parts(bytes, len) }.to_vec();
        unsafe { live::pixmith_bytes_free(bytes, len) };
        Some(live::Region { x, y, w, h, mask })
    }

    /// Like `apply_document`, also keeping, clearing or replacing the selection in the same undo step.
    pub fn apply_document_selection(
        &mut self,
        bytes: &[u8],
        label: &str,
        selection: &live::SelectionChange,
    ) -> Result<(), String> {
        let label = CString::new(label.replace('\0', "")).unwrap_or_default();
        let (mode, region) = match selection {
            live::SelectionChange::Keep => (0, None),
            live::SelectionChange::Clear => (1, None),
            live::SelectionChange::Set(r) => (2, Some(r)),
        };
        let (x, y, w, h, mask) = region.map_or((0, 0, 0, 0, &[][..]), |r| (r.x, r.y, r.w, r.h, &r.mask[..]));
        // SAFETY: `self.ptr` is a live document; the bytes, the label and the mask outlive the call.
        let ok = unsafe {
            live::pixmith_apply_document_selection(
                self.ptr,
                bytes.as_ptr(),
                bytes.len(),
                label.as_ptr(),
                mode,
                x,
                y,
                w,
                h,
                mask.as_ptr(),
                mask.len(),
            )
        };
        if ok { Ok(()) } else { Err(last_error()) }
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
