//! Core calls of the pixel-art warp (`pixmith_warp.h`): the handle region, the live preview and the apply.

use crate::doc::Doc;
use pixmith_core::paint::warp::{self, Warp};

/// The warp's offsets and pins as the flat arrays of the C ABI.
fn flat(w: &Warp) -> (Vec<f64>, Vec<f64>) {
    let offsets = w.offsets.iter().flat_map(|o| [o.0, o.1]).collect();
    let pins = w.pins.iter().flat_map(|p| [p.x, p.y, p.dx, p.dy, p.radius]).collect();
    (offsets, pins)
}

impl Doc {
    // SAFETY note for this file: `self.raw()` is a live document owned by `self`, and every buffer passed is
    // sized for the length given with it.

    /// Where the warp's handle grid lies: x, y, width, height of the selection, or of the canvas.
    pub fn warp_bounds(&self) -> (i32, i32, u32, u32) {
        let mut b = [0i32; 4];
        // SAFETY: `b` has the four slots the core writes.
        unsafe { warp::pixmith_warp_bounds(self.raw(), b.as_mut_ptr()) };
        (b[0], b[1], b[2].max(0) as u32, b[3].max(0) as u32)
    }

    /// The frame as it would look after the warp, as straight-alpha RGBA (`pixmith_warp_preview`).
    pub fn warp_preview(&self, layer: u32, frame: u32, w: &Warp, out: &mut Vec<u8>) -> bool {
        out.resize(self.width() as usize * self.height() as usize * 4, 0);
        let (offsets, pins) = flat(w);
        unsafe {
            warp::pixmith_warp_preview(
                self.raw(),
                layer,
                frame,
                w.cols,
                w.rows,
                offsets.as_ptr(),
                w.samples,
                w.smooth,
                pins.as_ptr(),
                w.pins.len(),
                out.as_mut_ptr(),
                out.len(),
            )
        }
    }

    /// Warps the cel as one undo step labeled "Warp". Returns true if pixels changed.
    pub fn warp_apply(&mut self, layer: u32, frame: u32, w: &Warp) -> bool {
        let (offsets, pins) = flat(w);
        let ok = unsafe {
            warp::pixmith_warp_apply(
                self.raw(),
                layer,
                frame,
                w.cols,
                w.rows,
                offsets.as_ptr(),
                w.samples,
                w.smooth,
                pins.as_ptr(),
                w.pins.len(),
            )
        };
        if ok {
            self.label_last_step("Warp");
            self.touch();
        }
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red_dot() -> Doc {
        let mut d = Doc::new(8, 8);
        d.begin_edit("Pencil");
        d.stroke_begin(0, 0, 1, 1, 0xFF0000FF, 1, 0);
        d.stroke_end();
        d.touch();
        d
    }

    #[test]
    fn warp_apply_is_one_labeled_undo_step_and_preview_leaves_the_document_alone() {
        let mut d = red_dot();
        let mut w = Warp::new(2, 2, d.warp_bounds());
        assert_eq!(d.warp_bounds(), (0, 0, 8, 8));
        w.offsets.fill((1.0, 0.0));
        let counts = d.history_counts();
        let mut out = Vec::new();
        assert!(d.warp_preview(0, 0, &w, &mut out));
        assert_eq!(Doc::pixel_of(&out, 8, 2, 1), Some(0xFF0000FF));
        assert_eq!(d.history_counts(), counts);
        assert!(d.warp_apply(0, 0, &w));
        assert_eq!(d.history_counts(), (counts.0 + 1, 0));
        assert_eq!(d.history_label(counts.0), "Warp");
        assert!(d.undo());
        assert!(!d.warp_apply(0, 0, &Warp::new(2, 2, d.warp_bounds())));
    }
}
