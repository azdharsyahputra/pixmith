//! Core calls used by the menus and app-level dialogs: filters, color curves, scale and canvas size, color
//! mode extras, the history list and the cel link sheet.
//!
//! One-to-one with `FiltersView.swift`, `CurvesEditor.swift`, `ColorModeView.swift`, `HistoryPanel.swift`,
//! `Selection.swift` (canvas size) and `LayerTools.swift` (cel links).

use crate::doc::{Doc, last_error, take_string};
use pixmith_core::document::color_mode as cm;
use pixmith_core::document::layers;
use pixmith_core::history;
use pixmith_core::paint::{filters, selection};

/// The rectangle that limits a filter: the selection's bounds, or `None` for the whole canvas.
pub type FilterRegion = Option<(i32, i32, u32, u32)>;

fn region_args(region: FilterRegion) -> (i32, i32, u32, u32) {
    region.unwrap_or((0, 0, 0, 0))
}

impl Doc {
    // SAFETY note for this file: `self.raw()` is a live document owned by `self`, and every buffer passed is
    // sized for the length given with it.

    /// The bounds of the selection, used as the filter region (`pixmith_sel_bounds`); `None` without one.
    pub fn filter_region(&self) -> FilterRegion {
        let mut b = [0i32; 4];
        // SAFETY: `b` has the four slots the core writes.
        let any = unsafe { selection::pixmith_sel_bounds(self.raw(), b.as_mut_ptr()) };
        any.then_some((b[0], b[1], b[2].max(0) as u32, b[3].max(0) as u32))
    }

    /// The frame as it would look after the filter, as straight-alpha RGBA (`pixmith_filter_preview`).
    #[allow(clippy::too_many_arguments)]
    pub fn filter_preview(
        &self,
        kind: u32,
        target: u32,
        layer: u32,
        frame: u32,
        region: FilterRegion,
        params: &[i32],
        out: &mut Vec<u8>,
    ) -> bool {
        out.resize(self.width() as usize * self.height() as usize * 4, 0);
        let (x, y, w, h) = region_args(region);
        unsafe {
            filters::pixmith_filter_preview(
                self.raw(),
                kind,
                target,
                layer,
                frame,
                x,
                y,
                w,
                h,
                params.as_ptr(),
                params.len(),
                out.as_mut_ptr(),
                out.len(),
            )
        }
    }

    /// Applies a filter as one undo step labeled "Filter: <title>". Returns true if pixels changed.
    #[allow(clippy::too_many_arguments)]
    pub fn filter_apply(
        &mut self,
        title: &str,
        kind: u32,
        target: u32,
        layer: u32,
        frame: u32,
        region: FilterRegion,
        params: &[i32],
    ) -> bool {
        let (x, y, w, h) = region_args(region);
        let ok = unsafe {
            filters::pixmith_filter_apply(
                self.raw(),
                kind,
                target,
                layer,
                frame,
                x,
                y,
                w,
                h,
                params.as_ptr(),
                params.len(),
            )
        };
        if ok {
            self.label_last_step(&format!("Filter: {title}"));
            self.touch();
        }
        ok
    }

    /// The 256-entry lookup table of one curve, from the core so previews match the filter (`lut`).
    pub fn curve_lut(points: &[(i32, i32)]) -> [u8; 256] {
        let flat: Vec<i32> = points.iter().flat_map(|&(x, y)| [x, y]).collect();
        let mut out = [0u8; 256];
        // SAFETY: `flat` holds `points.len()` pairs and `out` has 256 bytes.
        unsafe { filters::pixmith_curve_lut(flat.as_ptr(), points.len(), out.as_mut_ptr()) };
        out
    }

    /// Nearest-neighbor resize of the whole sprite (one undo step "Scale Sprite").
    pub fn scale_sprite(&mut self, width: u32, height: u32) -> bool {
        let ok = unsafe { filters::pixmith_scale_sprite(self.raw(), width, height) };
        if ok {
            self.label_last_step("Scale Sprite");
            self.touch();
        }
        ok
    }

    /// Resizes the canvas of every frame and layer; the offset is where the old pixels land.
    pub fn resize_canvas(&mut self, width: u32, height: u32, dx: i32, dy: i32) -> bool {
        let ok = unsafe { selection::pixmith_resize_canvas(self.raw(), width, height, dx, dy) };
        if ok {
            self.label_last_step("Canvas Size");
            self.touch();
        }
        ok
    }

    // MARK: Color mode

    /// Converts to `mode` (0 RGB, 1 grayscale, 2 indexed); `dither` is a Bayer matrix size, 0 for none.
    pub fn convert_color_mode_dithered(&mut self, mode: u32, dither: u32) -> Result<(), String> {
        if !unsafe { cm::pixmith_cm_convert(self.raw(), mode, dither) } {
            return Err(last_error());
        }
        self.label_last_step("Convert Color Mode");
        self.touch();
        Ok(())
    }

    /// Snaps all pixels to the palette (indexed) or to gray (grayscale); one undo step.
    pub fn snap_pixels_to_palette(&mut self) -> bool {
        let ok = unsafe { cm::pixmith_cm_conform_all(self.raw()) };
        if ok {
            self.touch();
        }
        ok
    }

    /// Replaces the palette with the sprite's colors; `max_colors` 0 keeps them exact.
    pub fn palette_from_sprite(&mut self, max_colors: u32) -> Result<(), String> {
        if unsafe { cm::pixmith_cm_palette_from_sprite(self.raw(), max_colors) } == 0 {
            return Err(last_error());
        }
        self.touch();
        Ok(())
    }

    // MARK: History

    /// The label of history step `i`: applied steps oldest first, then the undone ones.
    pub fn history_label(&self, i: u32) -> String {
        unsafe { take_string(history::pixmith_history_label(self.raw(), i)) }
    }

    /// Undoes or redoes until exactly `position` steps are applied (0 is the original document).
    pub fn history_goto(&mut self, position: u32) -> bool {
        let ok = unsafe { history::pixmith_history_goto(self.raw(), position) };
        if ok {
            self.touch();
        }
        ok
    }

    // MARK: Cels

    /// Makes the cels of frames `from..=to` copies of the one in `from` (`linkCels`).
    pub fn link_cels(&mut self, layer: u32, from: u32, to: u32) -> bool {
        let ok = unsafe { layers::pixmith_cel_link(self.raw(), layer, from, to) };
        if ok {
            self.label_last_step("Link Cels");
            self.touch();
        }
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An 8 x 8 sprite with one red pixel at (1, 1).
    fn red_dot() -> Doc {
        let mut d = Doc::new(8, 8);
        d.begin_edit("Pencil");
        d.stroke_begin(0, 0, 1, 1, 0xFF0000FF, 1, 0);
        d.stroke_end();
        d.touch();
        d
    }

    fn pixel(d: &Doc, x: i32, y: i32) -> u32 {
        let mut buf = Vec::new();
        assert!(d.render(0, &mut buf));
        Doc::pixel_of(&buf, d.width(), x, y).unwrap()
    }

    #[test]
    fn filter_apply_is_one_labeled_undo_step() {
        let mut d = red_dot();
        let before = d.history_counts().0;
        assert!(d.filter_apply("Invert", 1, 0, 0, 0, None, &[]));
        assert_eq!(pixel(&d, 1, 1), 0x00FFFFFF);
        assert_eq!(d.history_counts(), (before + 1, 0));
        assert_eq!(d.history_label(before), "Filter: Invert");
        assert!(d.undo());
        assert_eq!(pixel(&d, 1, 1), 0xFF0000FF);
    }

    #[test]
    fn filter_preview_leaves_the_document_alone() {
        let d = red_dot();
        let counts = d.history_counts();
        let mut out = Vec::new();
        assert!(d.filter_preview(1, 0, 0, 0, None, &[], &mut out));
        assert_eq!(Doc::pixel_of(&out, 8, 1, 1), Some(0x00FFFFFF));
        assert_eq!(pixel(&d, 1, 1), 0xFF0000FF);
        assert_eq!(d.history_counts(), counts);
    }

    #[test]
    fn replace_color_uses_the_parameter_block() {
        let mut d = red_dot();
        let params = [0xFF0000FFu32 as i32, 0x00FF00FFu32 as i32, 0];
        assert!(d.filter_apply("Replace Color", 0, 0, 0, 0, None, &params));
        assert_eq!(pixel(&d, 1, 1), 0x00FF00FF);
    }

    #[test]
    fn curve_lut_comes_from_the_core() {
        let identity = Doc::curve_lut(&[(0, 0), (255, 255)]);
        assert!((0..256).all(|i| identity[i] == i as u8));
        let flat = Doc::curve_lut(&[(0, 128), (255, 128)]);
        assert!(flat.iter().all(|&v| v == 128));
    }

    #[test]
    fn scale_and_canvas_size_change_every_frame() {
        let mut d = red_dot();
        assert!(d.scale_sprite(16, 16));
        assert_eq!((d.width(), d.height()), (16, 16));
        assert_eq!(pixel(&d, 2, 2), 0xFF0000FF);
        let n = d.history_counts().0;
        assert_eq!(d.history_label(n - 1), "Scale Sprite");
        assert!(d.resize_canvas(20, 18, 2, 1));
        assert_eq!((d.width(), d.height()), (20, 18));
        assert_eq!(pixel(&d, 4, 3), 0xFF0000FF);
        assert_eq!(d.history_label(d.history_counts().0 - 1), "Canvas Size");
    }

    #[test]
    fn history_goto_jumps_and_labels_read_back() {
        let mut d = red_dot();
        d.filter_apply("Invert", 1, 0, 0, 0, None, &[]);
        d.filter_apply("Blur", 9, 0, 0, 0, None, &[]);
        let (applied, undone) = d.history_counts();
        assert_eq!(undone, 0);
        assert!(d.history_goto(applied - 2));
        assert_eq!(d.history_counts(), (applied - 2, 2));
        assert_eq!(pixel(&d, 1, 1), 0xFF0000FF);
        // Undone steps follow the applied ones, the next one to redo first.
        assert_eq!(d.history_label(applied - 2), "Filter: Invert");
        assert!(d.history_goto(applied));
        assert_eq!(d.history_counts().1, 0);
    }

    #[test]
    fn color_mode_extras() {
        let mut d = red_dot();
        d.palette_from_sprite(0).expect("the sprite has a visible pixel");
        assert!(d.palette().contains(&0xFF0000FF));
        d.convert_color_mode_dithered(2, 0).expect("indexed");
        assert_eq!(d.color_mode(), 2);
        d.snap_pixels_to_palette();
        assert!(Doc::new(4, 4).palette_from_sprite(0).is_err(), "an empty sprite has no colors");
    }

    #[test]
    fn link_cels_shares_the_image() {
        let mut d = red_dot();
        d.add_frame(0, false);
        assert!(!d.cel_is_linked(0, 1));
        assert!(d.link_cels(0, 0, 1));
        assert!(d.cel_is_linked(0, 1));
    }
}
