//! Core calls used by the canvas code. Keep the `unsafe` here; panels call these safe methods.
//!
//! Each method mirrors what `SpriteModel` / `DrawingTools.swift` does with the same `pixmith_*` entry point.

use crate::doc::Doc;
use crate::ui::canvas::ShapeKind;
use pixmith_core::ffi::document as core;
use pixmith_core::history;
use pixmith_core::paint::{ink, shapes};
use std::ffi::CString;

impl Doc {
    /// Starts an undoable edit (`pixmith_begin_stroke`) and names the history step.
    pub fn begin_edit(&mut self, label: &str) {
        // SAFETY: `raw()` is a live document; `c` outlives the call.
        unsafe { core::pixmith_begin_stroke(self.raw()) };
        if let Ok(c) = CString::new(label) {
            unsafe { history::pixmith_checkpoint_label(self.raw(), c.as_ptr()) };
        }
    }

    /// Sets the ink mode for the next gesture (0 simple, 1 alpha, 2 lock alpha, 3 behind, 4 shading).
    pub fn set_ink(&mut self, mode: u32, reverse: bool, ramp: &[u32]) {
        let p = if ramp.is_empty() { std::ptr::null() } else { ramp.as_ptr() };
        // SAFETY: `p` is null or valid for `ramp.len()` entries.
        unsafe { ink::pixmith_ink_set(mode, reverse, p, ramp.len()) }
    }

    /// Round instead of square brush for sizes above one (global in the core).
    pub fn set_brush_circle(circle: bool) {
        ink::pixmith_brush_circle(circle)
    }

    /// Starts a freehand stroke at `(x, y)`; call `begin_edit` first.
    #[allow(clippy::too_many_arguments)]
    pub fn stroke_begin(&mut self, layer: u32, frame: u32, x: i32, y: i32, color: u32, size: u32, flags: u32) {
        // SAFETY: `raw()` is a live document.
        unsafe { shapes::pixmith_stroke_begin(self.raw(), layer, frame, x, y, color, size, flags) }
    }

    pub fn stroke_to(&mut self, x: i32, y: i32) {
        // SAFETY: `raw()` is a live document.
        unsafe { shapes::pixmith_stroke_to(self.raw(), x, y) }
    }

    /// Ends this document's running stroke.
    pub fn stroke_end(&mut self) {
        shapes::pixmith_stroke_end_doc(self.raw())
    }

    /// Draws a line, rectangle or ellipse. With `restore`, first rewinds to the checkpoint of `begin_edit`, so
    /// repeated calls while dragging are a live preview inside one undo step.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_shape(
        &mut self,
        kind: ShapeKind,
        layer: u32,
        frame: u32,
        a: (i32, i32),
        b: (i32, i32),
        color: u32,
        size: u32,
        flags: u32,
        restore: bool,
    ) {
        // SAFETY: `raw()` is a live document.
        unsafe {
            shapes::pixmith_shape(
                self.raw(),
                kind as u32,
                layer,
                frame,
                a.0,
                a.1,
                b.0,
                b.1,
                color,
                size,
                flags,
                restore,
            )
        }
    }

    /// Paint bucket: a solid fill through the current ink (`pixmith_fill_pattern`, pattern 0, as the Swift app does).
    #[allow(clippy::too_many_arguments)]
    pub fn fill_at(
        &mut self,
        layer: u32,
        frame: u32,
        x: i32,
        y: i32,
        color: u32,
        other: u32,
        contiguous: bool,
        flags: u32,
    ) {
        // SAFETY: `raw()` is a live document.
        unsafe { ink::pixmith_fill_pattern(self.raw(), layer, frame, x, y, color, other, 0, contiguous, flags) }
    }

    /// Discards the newest undo step and restores its state (cancelled previews).
    pub fn rollback(&mut self) -> bool {
        // SAFETY: `raw()` is a live document.
        unsafe { shapes::pixmith_rollback(self.raw()) }
    }

    /// Straight-alpha pixel `(x, y)` of a buffer returned by `render`, as 0xRRGGBBAA.
    pub fn pixel_of(buf: &[u8], width: u32, x: i32, y: i32) -> Option<u32> {
        if x < 0 || y < 0 || x as u32 >= width {
            return None;
        }
        let i = (y as usize * width as usize + x as usize) * 4;
        buf.get(i..i + 4).map(|p| u32::from_be_bytes([p[0], p[1], p[2], p[3]]))
    }
}
