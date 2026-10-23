//! Core calls of the drawing tools: spray, gradient, curve, polygon, text, custom brushes and pen pressure.
//!
//! One-to-one with `DrawingTools.swift`, `InkTools.swift` and `ShapeTools.swift`. They draw into the open undo step
//! (`Doc::begin_edit`); the caller then runs `Doc::touch` / `DocTab::changed`.

use crate::doc::Doc;
use pixmith_core::paint::{curves, font, ink, shapes};
use std::ffi::CString;

impl Doc {
    // SAFETY note for this file: `self.raw()` is a live document owned by `self`, and every pointer passed
    // outlives the call and is valid for the length given with it.

    /// One puff of spray paint around `at` (`SpriteModel.spray`).
    #[allow(clippy::too_many_arguments)]
    pub fn spray_at(
        &mut self,
        layer: u32,
        frame: u32,
        at: (i32, i32),
        color: u32,
        radius: u32,
        density: u32,
        seed: u32,
        flags: u32,
    ) {
        unsafe { shapes::pixmith_spray(self.raw(), layer, frame, at.0, at.1, color, radius, density, seed, flags) }
    }

    /// Linear or radial gradient from `a` to `b` over the cel or the selection. `dither` is 0 (smooth) or a Bayer
    /// matrix size. With `restore` it first rewinds to the checkpoint of `begin_edit`.
    #[allow(clippy::too_many_arguments)]
    pub fn gradient_draw(
        &mut self,
        layer: u32,
        frame: u32,
        a: (i32, i32),
        b: (i32, i32),
        c0: u32,
        c1: u32,
        radial: bool,
        dither: u32,
        restore: bool,
    ) {
        unsafe {
            ink::pixmith_gradient(self.raw(), layer, frame, a.0, a.1, b.0, b.1, c0, c1, radial as u32, dither, restore)
        }
    }

    /// Bezier curve through start, two control points and end. Always rewinds to the checkpoint first.
    pub fn curve_draw(&mut self, layer: u32, frame: u32, pts: &[(i32, i32); 4], color: u32, size: u32, flags: u32) {
        let v: [i32; 8] = [pts[0].0, pts[0].1, pts[1].0, pts[1].1, pts[2].0, pts[2].1, pts[3].0, pts[3].1];
        unsafe { curves::pixmith_curve(self.raw(), layer, frame, v.as_ptr(), color, size, flags, true) }
    }

    /// Closed polygon (flag 8 fills it, flag 32 leaves the path open). Always rewinds to the checkpoint first.
    pub fn polygon_draw(&mut self, layer: u32, frame: u32, pts: &[(i32, i32)], color: u32, size: u32, flags: u32) {
        let v: Vec<i32> = pts.iter().flat_map(|p| [p.0, p.1]).collect();
        unsafe { curves::pixmith_polygon(self.raw(), layer, frame, v.as_ptr(), pts.len(), color, size, flags, true) }
    }

    /// Text in the built-in 5x8 font, top-left at `at`, each font pixel `scale` pixels square.
    #[allow(clippy::too_many_arguments)]
    pub fn text_draw(
        &mut self,
        layer: u32,
        frame: u32,
        at: (i32, i32),
        text: &str,
        scale: u32,
        color: u32,
        flags: u32,
        restore: bool,
    ) {
        let c = CString::new(text.replace('\0', "")).unwrap_or_default();
        unsafe { font::pixmith_text(self.raw(), layer, frame, at.0, at.1, c.as_ptr(), scale, color, flags, restore) }
    }

    /// Pixel size of `text` in the built-in font.
    pub fn text_size_of(text: &str, scale: u32) -> (i32, i32) {
        let c = CString::new(text.replace('\0', "")).unwrap_or_default();
        let mut out = [0i32; 2];
        unsafe { font::pixmith_text_size(c.as_ptr(), scale, out.as_mut_ptr()) };
        (out[0], out[1])
    }

    /// Paints `color` through a `size.0` x `size.1` coverage mask (values above 127 are ink), e.g. rasterized
    /// system fonts.
    #[allow(clippy::too_many_arguments)]
    pub fn stamp_mask_draw(
        &mut self,
        layer: u32,
        frame: u32,
        at: (i32, i32),
        size: (u32, u32),
        mask: &[u8],
        color: u32,
        flags: u32,
        restore: bool,
    ) {
        if mask.len() < size.0 as usize * size.1 as usize {
            return;
        }
        unsafe {
            font::pixmith_stamp_mask(
                self.raw(),
                layer,
                frame,
                at.0,
                at.1,
                size.0,
                size.1,
                mask.as_ptr(),
                color,
                flags,
                restore,
            )
        }
    }

    /// Changes color and size of the running freehand stroke (pen pressure).
    pub fn stroke_style(&mut self, color: u32, size: u32) {
        unsafe { shapes::pixmith_stroke_style(self.raw(), color, size) }
    }

    /// Paint bucket with a dither pattern between `color` and `other` (0 solid, 1 checker, 2 25%, 3 50%, 4 75%).
    #[allow(clippy::too_many_arguments)]
    pub fn fill_pattern_at(
        &mut self,
        layer: u32,
        frame: u32,
        at: (i32, i32),
        color: u32,
        other: u32,
        pattern: u32,
        contiguous: bool,
        flags: u32,
    ) {
        unsafe {
            ink::pixmith_fill_pattern(self.raw(), layer, frame, at.0, at.1, color, other, pattern, contiguous, flags)
        }
    }

    /// Uses the selected pixels of the layer as the brush. False if nothing visible is selected.
    pub fn brush_from_selection(&self, layer: u32, frame: u32) -> bool {
        unsafe { ink::pixmith_brush_from_selection(self.raw(), layer, frame) }
    }

    /// Drops the custom brush (global in the core).
    pub fn brush_reset() {
        ink::pixmith_brush_reset()
    }

    /// Size of the custom brush, if one is set (global in the core).
    pub fn brush_info() -> Option<(i32, i32)> {
        let mut out = [0i32; 3];
        unsafe { ink::pixmith_brush_info(out.as_mut_ptr()) };
        (out[0] != 0).then_some((out[1], out[2]))
    }
}
