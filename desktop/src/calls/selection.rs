//! Core calls used by the selection, clipboard and free transform code. Keep the `unsafe` here.
//!
//! One-to-one with `Selection.swift` and `FreeTransform.swift` (`pixmith_selection.h`, `pixmith_transform.h`).
//! The methods only call the core; labelling the undo step and `touch()` are the caller's job, exactly where
//! the Swift code calls `labelLastStep` and `changed()`.

use crate::doc::Doc;
use crate::ui::selection::{CopiedPixels, SelMode, XfDrag};
use pixmith_core::paint::{selection as sel, transform as xf};

/// The 8 handle positions and the pivot of the free transform.
pub type XfGeometry = ([(f64, f64); 8], (f64, f64));

impl Doc {
    // SAFETY note for this file: `self.raw()` is a live document owned by `self`, and every buffer passed is
    // valid for the length given for the duration of the call.

    // MARK: Selection

    pub fn sel_active(&self) -> bool {
        unsafe { sel::pixmith_sel_active(self.raw()) }
    }

    /// Bounding box `[x, y, w, h]` of the selection.
    pub fn sel_bounds(&self) -> Option<[i32; 4]> {
        let mut b = [0i32; 4];
        unsafe { sel::pixmith_sel_bounds(self.raw(), b.as_mut_ptr()) }.then_some(b)
    }

    pub fn sel_contains(&self, x: i32, y: i32) -> bool {
        unsafe { sel::pixmith_sel_contains(self.raw(), x, y) }
    }

    pub fn sel_all(&mut self) {
        unsafe { sel::pixmith_sel_all(self.raw()) }
    }

    pub fn sel_clear(&mut self) {
        unsafe { sel::pixmith_sel_clear(self.raw()) }
    }

    pub fn sel_invert(&mut self) {
        unsafe { sel::pixmith_sel_invert(self.raw()) }
    }

    pub fn sel_rect(&mut self, a: (i32, i32), b: (i32, i32)) {
        unsafe { sel::pixmith_sel_rect(self.raw(), a.0, a.1, b.0, b.1) }
    }

    /// The outline as `[x0, y0, x1, y1]` segments in pixel-corner coordinates.
    pub fn sel_outline(&self) -> Vec<[i32; 4]> {
        let n = unsafe { sel::pixmith_sel_outline(self.raw(), std::ptr::null_mut(), 0) };
        if n == 0 {
            return Vec::new();
        }
        let mut flat = vec![0i32; n * 4];
        let got = unsafe { sel::pixmith_sel_outline(self.raw(), flat.as_mut_ptr(), n) };
        flat.as_chunks::<4>().0.iter().take(got.min(n)).copied().collect()
    }

    /// Starts an interactive selection (one undo step).
    pub fn sel_begin(&mut self, mode: SelMode) {
        unsafe { sel::pixmith_sel_begin(self.raw(), mode as u32) }
    }

    pub fn sel_end(&mut self) {
        unsafe { sel::pixmith_sel_end(self.raw()) }
    }

    /// Selects the area inside a closed path of `(x, y)` points.
    pub fn sel_poly(&mut self, points: &[(i32, i32)]) {
        let flat: Vec<i32> = points.iter().flat_map(|p| [p.0, p.1]).collect();
        unsafe { sel::pixmith_sel_poly(self.raw(), flat.as_ptr(), points.len()) }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sel_wand(
        &mut self,
        layer: u32,
        frame: u32,
        x: i32,
        y: i32,
        tolerance: u32,
        contiguous: bool,
        mode: SelMode,
    ) -> bool {
        unsafe { sel::pixmith_sel_wand(self.raw(), layer, frame, x, y, tolerance, contiguous, mode as u32) }
    }

    /// `kind`: 0 expand, 1 contract, 2 border.
    pub fn sel_modify(&mut self, kind: u32, n: u32) -> bool {
        unsafe { sel::pixmith_sel_modify(self.raw(), kind, n) }
    }

    // MARK: Move, delete, clipboard

    pub fn sel_move_begin(&mut self, layer: u32, frame: u32) -> bool {
        unsafe { sel::pixmith_sel_move_begin(self.raw(), layer, frame) }
    }

    pub fn sel_move_to(&mut self, dx: i32, dy: i32) {
        unsafe { sel::pixmith_sel_move_to(self.raw(), dx, dy) }
    }

    pub fn sel_move_end(&mut self) {
        unsafe { sel::pixmith_sel_move_end(self.raw()) }
    }

    pub fn sel_nudge(&mut self, layer: u32, frame: u32, dx: i32, dy: i32) -> bool {
        unsafe { sel::pixmith_sel_nudge(self.raw(), layer, frame, dx, dy) }
    }

    pub fn sel_delete(&mut self, layer: u32, frame: u32) -> bool {
        unsafe { sel::pixmith_sel_delete(self.raw(), layer, frame) }
    }

    /// Flip or rotate the selection (or the cel): 0 flip horizontal, 1 flip vertical, 2 rotate clockwise,
    /// 3 rotate counter-clockwise.
    pub fn sel_transform(&mut self, layer: u32, frame: u32, op: u32) -> bool {
        unsafe { sel::pixmith_sel_transform(self.raw(), layer, frame, op) }
    }

    pub fn sel_copy(&self, layer: u32, frame: u32) -> Option<CopiedPixels> {
        let mut b = [0i32; 4];
        unsafe { sel::pixmith_sel_copy_bounds(self.raw(), b.as_mut_ptr()) };
        let (w, h) = (b[2].max(0) as u32, b[3].max(0) as u32);
        if w == 0 || h == 0 {
            return None;
        }
        let mut rgba = vec![0u8; w as usize * h as usize * 4];
        unsafe { sel::pixmith_sel_copy(self.raw(), layer, frame, rgba.as_mut_ptr(), rgba.len()) }
            .then_some(CopiedPixels { x: b[0], y: b[1], w, h, rgba })
    }

    /// Pastes straight-alpha RGBA as a floating selection at `(x, y)`.
    #[allow(clippy::too_many_arguments)]
    pub fn sel_paste(&mut self, layer: u32, frame: u32, w: u32, h: u32, rgba: &[u8], x: i32, y: i32) -> bool {
        if rgba.len() < w as usize * h as usize * 4 {
            return false;
        }
        unsafe { sel::pixmith_sel_paste(self.raw(), layer, frame, w, h, rgba.as_ptr(), x, y) }
    }

    pub fn crop_to_selection(&mut self) -> bool {
        unsafe { sel::pixmith_crop_to_selection(self.raw()) }
    }

    // MARK: Floating selection

    pub fn float_active(&self) -> bool {
        unsafe { sel::pixmith_float_active(self.raw()) }
    }

    pub fn float_commit(&mut self) {
        unsafe { sel::pixmith_float_commit(self.raw()) }
    }

    pub fn float_cancel(&mut self) -> bool {
        unsafe { sel::pixmith_float_cancel(self.raw()) }
    }

    // MARK: Free transform

    pub fn xf_begin(&mut self, layer: u32, frame: u32, rotsprite: bool) -> bool {
        unsafe { xf::pixmith_xf_begin(self.raw(), layer, frame, rotsprite) }
    }

    pub fn xf_active(&self) -> bool {
        unsafe { xf::pixmith_xf_active(self.raw()) }
    }

    pub fn xf_commit(&mut self) {
        unsafe { xf::pixmith_xf_commit(self.raw()) }
    }

    pub fn xf_cancel(&mut self) -> bool {
        unsafe { xf::pixmith_xf_cancel(self.raw()) }
    }

    pub fn xf_set_rotsprite(&mut self, on: bool) {
        unsafe { xf::pixmith_xf_set_rotsprite(self.raw(), on) }
    }

    /// The box point (`hx`, `hy` in -1..=1) that typed width and height keep fixed.
    pub fn xf_set_anchor(&mut self, hx: i32, hy: i32) {
        unsafe { xf::pixmith_xf_set_anchor(self.raw(), hx, hy) }
    }

    /// The 8 handle positions (TL, T, TR, L, R, BL, B, BR) and the pivot, in canvas coordinates.
    pub fn xf_geometry(&self) -> Option<XfGeometry> {
        let mut g = [0f64; 18];
        if !unsafe { xf::pixmith_xf_geometry(self.raw(), g.as_mut_ptr()) } {
            return None;
        }
        let mut handles = [(0.0, 0.0); 8];
        for (i, h) in handles.iter_mut().enumerate() {
            *h = (g[i * 2], g[i * 2 + 1]);
        }
        Some((handles, (g[16], g[17])))
    }

    /// X, Y, width, height, angle, skew x, skew y.
    pub fn xf_fields(&self) -> [f64; 7] {
        let mut f = [0f64; 7];
        unsafe { xf::pixmith_xf_fields(self.raw(), f.as_mut_ptr()) };
        f
    }

    pub fn xf_set_field(&mut self, field: u32, value: f64) {
        unsafe { xf::pixmith_xf_set_field(self.raw(), field, value) }
    }

    pub fn xf_drag_begin(&mut self) {
        unsafe { xf::pixmith_xf_drag_begin(self.raw()) }
    }

    /// One step of a handle drag; `start` is where it began and `to` the pointer, in canvas coordinates.
    pub fn xf_drag(&mut self, kind: XfDrag, handle: (i32, i32), flag: bool, start: (f64, f64), to: (f64, f64)) {
        unsafe { xf::pixmith_xf_drag(self.raw(), kind as u32, handle.0, handle.1, flag, start.0, start.1, to.0, to.1) }
    }
}
