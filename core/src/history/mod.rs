//! Identifies document states so "modified" can be decided by comparing with the saved state,
//! which makes undoing back to the saved state clean again.

pub(crate) mod buffer_pool;
pub mod store;

use crate::document::Editor;

/// Parallel to the editor's undo/redo stacks: each snapshot has the id of the state it holds.
#[derive(Debug, Default)]
pub struct States {
    current: u64,
    saved: u64,
    next: u64,
    undo: Vec<u64>,
    redo: Vec<u64>,
}

impl States {
    fn fresh(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    /// A snapshot of the current state was pushed; whatever follows is a new state.
    /// `trimmed` is true when the oldest snapshot was dropped.
    pub fn on_checkpoint(&mut self, trimmed: bool) {
        self.undo.push(self.current);
        if trimmed {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.current = self.fresh();
    }

    pub fn on_undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(self.current);
            self.current = prev;
        }
    }

    /// All snapshots were discarded: no state the document can return to has the saved id.
    pub fn drop_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.current = self.fresh();
    }

    /// The redo steps were discarded without a new state (the document itself did not change).
    pub fn clear_redo(&mut self) {
        self.redo.clear();
    }

    /// The oldest snapshot was dropped to save memory. If it held the saved state, the document
    /// stays "modified" for good: no state it can return to has the saved id any more.
    pub fn drop_oldest_undo(&mut self) {
        if !self.undo.is_empty() {
            self.undo.remove(0);
        }
    }

    /// The latest snapshot was discarded without becoming a redo step.
    pub fn on_rollback(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.current = prev;
        }
    }

    pub fn on_redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(self.current);
            self.current = next;
        }
    }

    /// An edit that does not go through the undo stack (e.g. toggling visibility).
    pub fn touch(&mut self) {
        self.current = self.fresh();
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.current;
    }

    pub fn is_dirty(&self) -> bool {
        self.current != self.saved
    }

    /// Identifies the state the document is in; it differs after every checkpoint, undo, redo and touch
    /// (front ends use it to notice that a document changed).
    pub fn current_id(&self) -> u64 {
        self.current
    }
}

/// Sets how many bytes of memory the undo history of a document may hold beyond the live
/// document (default 1 GiB); older steps are dropped to fit. The step count stays capped at 100.
#[no_mangle]
pub unsafe extern "C" fn pixmith_set_undo_budget(doc: *mut Editor, bytes: u64) {
    crate::ffi::guarded("pixmith_set_undo_budget", doc, || {
        if let Some(e) = doc.as_mut() {
            e.set_undo_budget(usize::try_from(bytes).unwrap_or(usize::MAX));
        }
    })
}

/// Bytes of memory only the undo and redo history keeps alive.
#[no_mangle]
pub unsafe extern "C" fn pixmith_undo_bytes(doc: *const Editor) -> u64 {
    crate::ffi::guarded("pixmith_undo_bytes", std::ptr::null_mut(), || {
        doc.as_ref().map_or(0, |e| e.history_bytes() as u64)
    })
}

/// Whether the document differs from the last saved (or opened) state.
#[no_mangle]
pub unsafe extern "C" fn pixmith_is_dirty(doc: *const Editor) -> bool {
    crate::ffi::guarded("pixmith_is_dirty", std::ptr::null_mut(), || doc.as_ref().is_some_and(|e| e.states.is_dirty()))
}

/// Records the current state as the saved one.
#[no_mangle]
pub unsafe extern "C" fn pixmith_mark_saved(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_mark_saved", doc, || {
        if let Some(e) = doc.as_mut() {
            e.states.mark_saved();
        }
    })
}

/// Marks an edit that bypassed the undo stack. Call after every change that did not checkpoint.
#[no_mangle]
pub unsafe extern "C" fn pixmith_touch(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_touch", doc, || {
        if let Some(e) = doc.as_mut() {
            e.states.touch();
        }
    })
}

/// Starts a transaction: every checkpoint inside it after the first is ignored, so the operations
/// done before the matching `pixmith_end_transaction` undo as one step. Transactions nest.
#[no_mangle]
pub unsafe extern "C" fn pixmith_begin_transaction(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_begin_transaction", doc, || {
        if let Some(e) = doc.as_mut() {
            e.begin_transaction();
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_end_transaction(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_end_transaction", doc, || {
        if let Some(e) = doc.as_mut() {
            e.end_transaction();
        }
    })
}

/// Names the most recent undo step (only if it has no name yet). Call right after the operation.
#[no_mangle]
pub unsafe extern "C" fn pixmith_checkpoint_label(doc: *mut Editor, label: *const std::ffi::c_char) {
    crate::ffi::guarded("pixmith_checkpoint_label", doc, || {
        if let (Some(e), false) = (doc.as_mut(), label.is_null()) {
            e.label_last(&std::ffi::CStr::from_ptr(label).to_string_lossy());
        }
    })
}

/// Number of undo steps (applied) and redo steps (undone).
#[no_mangle]
pub unsafe extern "C" fn pixmith_history_counts(doc: *const Editor, undo: *mut u32, redo: *mut u32) {
    crate::ffi::guarded("pixmith_history_counts", std::ptr::null_mut(), || {
        let Some(e) = doc.as_ref() else { return };
        let (u, r) = e.history_len();
        if !undo.is_null() {
            *undo = u as u32;
        }
        if !redo.is_null() {
            *redo = r as u32;
        }
    })
}

/// Label of history step `index`: applied steps oldest first, then undone steps (next to redo first).
/// An unlabeled step reads "Edit". Free the result with `pixmith_string_free`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_history_label(doc: *const Editor, index: u32) -> *mut std::ffi::c_char {
    crate::ffi::guarded("pixmith_history_label", std::ptr::null_mut(), || {
        let Some(label) = doc.as_ref().and_then(|e| e.history_label(index as usize)) else {
            return std::ptr::null_mut();
        };
        let label = if label.is_empty() { "Edit" } else { label };
        std::ffi::CString::new(label.replace('\0', "")).map_or(std::ptr::null_mut(), |c| c.into_raw())
    })
}

/// Undoes or redoes until exactly `position` steps are applied (0 = before the first step).
#[no_mangle]
pub unsafe extern "C" fn pixmith_history_goto(doc: *mut Editor, position: u32) -> bool {
    crate::ffi::guarded("pixmith_history_goto", doc, || {
        doc.as_mut().is_some_and(|e| {
            e.dirty.mark_all();
            e.history_goto(position as usize)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];

    fn px(e: &Editor) -> Vec<u8> {
        e.sprite.render(0)
    }

    #[test]
    fn no_op_edits_record_no_undo_step() {
        let mut e = Editor::new(crate::document::Sprite::new(2, 2));
        let steps = |e: &Editor| e.history_len().0;
        let name = std::ffi::CString::new("Layer 1").unwrap();
        unsafe { crate::ffi::document::pixmith_rename_layer(&mut e, 0, name.as_ptr()) };
        assert_eq!(steps(&e), 0, "renaming to the same name");
        assert!(crate::document::slices::set_user_data(&mut e, 0, 0, 0, None, None));
        assert_eq!(steps(&e), 0, "unchanged user data");
        assert!(!unsafe { crate::palette::io::pixmith_palette_remove(&mut e, 99) });
        assert_eq!(steps(&e), 0, "removing a missing palette entry");

        e.sprite.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        assert!(crate::paint::selection::delete(&mut e, 0, 0));
        assert_eq!(steps(&e), 1);
        assert!(!crate::paint::selection::delete(&mut e, 0, 0));
        assert_eq!(steps(&e), 1, "deleting nothing");

        let i = crate::document::slices::add_slice(&mut e, crate::document::slices::Rect { x: 0, y: 0, w: 1, h: 1 });
        assert_eq!(steps(&e), 2);
        let slice_name = e.sprite.extra.slices[i].name.clone();
        assert!(crate::document::slices::set_props(&mut e, i, 0, &slice_name, None, None));
        assert_eq!(steps(&e), 2, "unchanged slice properties");
        assert!(crate::document::slices::set_props(&mut e, i, 0, "renamed", None, None));
        assert_eq!(steps(&e), 3);
    }

    #[test]
    fn transaction_collapses_checkpoints_into_one_undo_step() {
        let mut e = Editor::new(crate::document::Sprite::new(2, 2));
        e.begin_transaction();
        for x in 0..2 {
            e.checkpoint();
            e.sprite.draw_line(0, 0, x, 0, x, 0, RED, 1);
        }
        e.end_transaction();
        assert_eq!(e.history_len(), (1, 0));
        assert!(e.undo());
        assert_eq!(px(&e), vec![0; 16]);
        assert!(!e.undo());
        assert!(e.redo());
        assert_eq!(&px(&e)[0..8], &[255, 0, 0, 255, 255, 0, 0, 255]);
    }

    #[test]
    fn transactions_nest_and_reset() {
        let mut e = Editor::new(crate::document::Sprite::new(2, 2));
        e.begin_transaction();
        e.checkpoint();
        e.begin_transaction();
        e.checkpoint();
        e.end_transaction();
        e.checkpoint(); // still inside the outer transaction
        e.end_transaction();
        assert_eq!(e.history_len().0, 1);
        e.checkpoint(); // outside again: records normally
        e.checkpoint();
        assert_eq!(e.history_len().0, 3);
        e.end_transaction(); // unbalanced end is harmless
        e.checkpoint();
        assert_eq!(e.history_len().0, 4);
    }

    #[test]
    fn layer_operations_in_a_transaction_undo_together() {
        use crate::document::layers::{pixmith_layer_add_group, pixmith_layer_move};
        let mut e = Editor::new(crate::document::Sprite::new(1, 1));
        e.sprite.add_layer(0);
        e.sprite.add_layer(1);
        let names = |e: &Editor| e.sprite.layers.iter().map(|l| (l.name.clone(), l.child_level)).collect::<Vec<_>>();
        let before = names(&e);
        unsafe {
            pixmith_begin_transaction(&mut e);
            assert!(pixmith_layer_move(&mut e, 0, true) >= 0);
            assert!(pixmith_layer_add_group(&mut e, 0) >= 0);
            assert!(pixmith_layer_move(&mut e, 1, true) >= 0);
            pixmith_end_transaction(&mut e);
        }
        assert_eq!(e.history_len().0, 1);
        let after = names(&e);
        assert_ne!(after, before);
        assert!(e.undo());
        assert_eq!(names(&e), before);
        assert!(e.redo());
        assert_eq!(names(&e), after);
    }

    #[test]
    fn empty_transaction_records_nothing() {
        let mut e = Editor::new(crate::document::Sprite::new(1, 1));
        e.begin_transaction();
        e.end_transaction();
        assert_eq!(e.history_len(), (0, 0));
        e.begin_transaction();
        e.end_transaction();
        e.checkpoint();
        assert_eq!(e.history_len(), (1, 0));
    }

    #[test]
    fn transaction_keeps_dirty_state_and_selection_undo_in_step() {
        let mut e = Editor::new(crate::document::Sprite::new(2, 2));
        e.states.mark_saved();
        e.begin_transaction();
        e.checkpoint();
        e.checkpoint();
        e.end_transaction();
        assert!(e.states.is_dirty());
        assert!(e.undo());
        assert!(!e.states.is_dirty());
        assert!(e.redo());
        assert!(e.states.is_dirty());
        // The selection stacks stayed aligned with the sprite stacks: more undo/redo cycles must not panic or drift.
        assert!(e.undo());
        assert!(e.redo());
        e.checkpoint();
        assert!(e.undo());
        assert!(!e.states.is_dirty() || e.history_len().0 == 1);
    }

    #[test]
    fn labels_follow_steps_through_undo_redo_and_trim() {
        let mut e = Editor::new(crate::document::Sprite::new(1, 1));
        for name in ["Draw", "Fill", "Move"] {
            e.checkpoint();
            e.label_last(name);
        }
        e.label_last("ignored"); // already named
        assert_eq!((0..3).map(|i| e.history_label(i).unwrap()).collect::<Vec<_>>(), ["Draw", "Fill", "Move"]);
        assert!(e.undo());
        // Applied: Draw, Fill; undone: Move.
        assert_eq!(e.history_len(), (2, 1));
        assert_eq!(e.history_label(2), Some("Move"));
        assert!(e.undo());
        assert_eq!(e.history_label(1), Some("Fill"));
        assert_eq!(e.history_label(2), Some("Move"));
        assert!(e.redo());
        assert_eq!(e.history_label(1), Some("Fill"));
        assert_eq!(e.history_label(2), Some("Move"));
        // A new edit drops the redo labels.
        e.checkpoint();
        assert_eq!(e.history_len(), (3, 0));
        assert_eq!(e.history_label(1), Some("Fill"));
        assert_eq!(e.history_label(2), Some(""));
        assert_eq!(e.history_label(3), None);
    }

    #[test]
    fn history_goto_jumps_in_both_directions() {
        let mut e = Editor::new(crate::document::Sprite::new(4, 1));
        for x in 0..4 {
            e.checkpoint();
            e.sprite.draw_line(0, 0, x, 0, x, 0, RED, 1);
        }
        let full = px(&e);
        assert!(e.history_goto(1));
        assert_eq!(px(&e).chunks(4).filter(|p| p[3] != 0).count(), 1);
        assert_eq!(e.history_len(), (1, 3));
        assert!(e.history_goto(4));
        assert_eq!(px(&e), full);
        assert!(!e.history_goto(5));
        assert!(e.history_goto(0));
        assert_eq!(px(&e), vec![0; 16]);
    }

    #[test]
    fn undo_limit_trims_labels_with_snapshots() {
        let mut e = Editor::new(crate::document::Sprite::new(1, 1));
        for i in 0..crate::document::UNDO_LIMIT + 5 {
            e.checkpoint();
            e.label_last(&i.to_string());
        }
        assert_eq!(e.history_len().0, crate::document::UNDO_LIMIT);
        assert_eq!(e.history_label(0), Some("5"));
        assert_eq!(
            e.history_label(crate::document::UNDO_LIMIT - 1).map(str::to_string),
            Some((crate::document::UNDO_LIMIT + 4).to_string())
        );
    }

    #[test]
    fn editor_tracks_saved_state_through_undo() {
        use crate::document::Sprite;
        let mut e = Editor::new(Sprite::new(2, 2));
        unsafe {
            assert!(!pixmith_is_dirty(&e));
            e.checkpoint();
            e.sprite.draw_line(0, 0, 0, 0, 1, 1, [1, 2, 3, 255], 1);
            assert!(pixmith_is_dirty(&e));
            pixmith_mark_saved(&mut e);
            assert!(!pixmith_is_dirty(&e));
            e.checkpoint();
            assert!(pixmith_is_dirty(&e));
            assert!(e.undo());
            assert!(!pixmith_is_dirty(&e));
            pixmith_touch(&mut e);
            assert!(pixmith_is_dirty(&e));
        }
    }

    #[test]
    fn undoing_to_the_saved_state_is_clean() {
        let mut h = States::default();
        assert!(!h.is_dirty());
        h.on_checkpoint(false);
        assert!(h.is_dirty());
        h.mark_saved();
        assert!(!h.is_dirty());
        h.on_checkpoint(false);
        h.touch();
        assert!(h.is_dirty());
        h.on_undo();
        assert!(!h.is_dirty());
        h.on_undo();
        assert!(h.is_dirty());
        h.on_redo();
        assert!(!h.is_dirty());
        h.on_redo();
        assert!(h.is_dirty());
    }

    #[test]
    fn new_edit_after_undo_drops_redo_and_stays_dirty() {
        let mut h = States::default();
        h.on_checkpoint(false);
        h.mark_saved();
        h.on_undo();
        assert!(h.is_dirty());
        h.on_checkpoint(false);
        h.on_redo();
        assert!(h.is_dirty());
        h.on_undo();
        assert!(h.is_dirty());
    }

    #[test]
    fn touch_after_save_without_checkpoint_is_dirty_until_undone() {
        let mut h = States::default();
        h.mark_saved();
        h.touch();
        assert!(h.is_dirty());
        h.on_checkpoint(false);
        h.on_undo();
        assert!(h.is_dirty());
    }

    #[test]
    fn trimmed_history_cannot_return_to_saved_state() {
        let mut h = States::default();
        h.mark_saved();
        for _ in 0..3 {
            h.on_checkpoint(false);
        }
        h.on_checkpoint(true);
        while !h.undo.is_empty() {
            h.on_undo();
        }
        assert!(h.is_dirty());
    }
}
