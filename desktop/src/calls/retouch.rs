//! Core calls of the Blur and Jumble brushes (`pixmith_retouch_*`, `DrawingTools.swift` `retouch`).

use crate::doc::Doc;
use pixmith_core::paint::retouch;

impl Doc {
    // SAFETY note for this file: `self.raw()` is a live document owned by `self`.

    /// Starts a Blur (`jumble == false`) or Jumble stroke at `at`; call `begin_edit` first for the undo step.
    #[allow(clippy::too_many_arguments)]
    pub fn retouch_begin(
        &mut self,
        layer: u32,
        frame: u32,
        at: (i32, i32),
        jumble: bool,
        size: u32,
        strength: u32,
        seed: u32,
        flags: u32,
    ) {
        let kind = if jumble { retouch::KIND_JUMBLE } else { retouch::KIND_BLUR };
        unsafe {
            retouch::pixmith_retouch_begin(self.raw(), layer, frame, at.0, at.1, kind, size, strength, seed, flags)
        }
    }

    /// Continues the running stroke with a line of dabs to `at`.
    pub fn retouch_to(&mut self, at: (i32, i32)) {
        unsafe { retouch::pixmith_retouch_to(self.raw(), at.0, at.1) }
    }

    /// Resizes the running stroke (pen pressure).
    pub fn retouch_size(&mut self, size: u32) {
        unsafe { retouch::pixmith_retouch_size(self.raw(), size) }
    }

    /// Ends this document's running Blur or Jumble stroke.
    pub fn retouch_end(&mut self) {
        retouch::pixmith_retouch_end_doc(self.raw())
    }
}
