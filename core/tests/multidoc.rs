//! Several documents used alternately through the C ABI (what the tabbed app does) must not interfere.

use pixmith_core::document::layers::{pixmith_cels_copy, pixmith_cels_has_clip, pixmith_cels_paste};
use pixmith_core::ffi::document::{
    pixmith_add_frame, pixmith_add_layer, pixmith_begin_stroke, pixmith_draw_line, pixmith_frame_count, pixmith_free,
    pixmith_layer_count, pixmith_new, pixmith_redo, pixmith_render, pixmith_undo,
};
use pixmith_core::history::{
    pixmith_begin_transaction, pixmith_end_transaction, pixmith_history_counts, pixmith_is_dirty, pixmith_mark_saved,
};
use pixmith_core::paint::selection::{
    pixmith_float_active, pixmith_sel_active, pixmith_sel_begin, pixmith_sel_end, pixmith_sel_paste, pixmith_sel_rect,
};
use pixmith_core::paint::shapes::{
    pixmith_stroke_begin, pixmith_stroke_end, pixmith_stroke_end_doc, pixmith_stroke_to,
};

const RED: u32 = 0xFF0000FF;
const BLUE: u32 = 0x0000FFFF;

unsafe fn px(d: *mut pixmith_core::document::Editor, x: usize, y: usize) -> [u8; 4] {
    let (w, h) = (8usize, 8usize);
    let mut buf = vec![0u8; w * h * 4];
    assert!(pixmith_render(d, 0, buf.as_mut_ptr(), buf.len()));
    let i = (y * w + x) * 4;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

unsafe fn undo_count(d: *mut pixmith_core::document::Editor) -> (u32, u32) {
    let (mut u, mut r) = (0, 0);
    pixmith_history_counts(d, &mut u, &mut r);
    (u, r)
}

#[test]
fn interleaved_strokes_do_not_cross() {
    unsafe {
        let (a, b) = (pixmith_new(8, 8), pixmith_new(8, 8));
        pixmith_begin_stroke(a);
        pixmith_stroke_begin(a, 0, 0, 0, 0, RED, 1, 0);
        pixmith_begin_stroke(b);
        pixmith_stroke_begin(b, 0, 0, 0, 7, BLUE, 1, 0);
        // Alternate drags: each document keeps its own stroke, color and path.
        pixmith_stroke_to(a, 3, 0);
        pixmith_stroke_to(b, 3, 7);
        pixmith_stroke_to(a, 5, 0);
        pixmith_stroke_end_doc(a);
        pixmith_stroke_to(a, 7, 0); // ended: no effect
        pixmith_stroke_to(b, 5, 7);
        pixmith_stroke_end();
        assert_eq!(px(a, 5, 0), [255, 0, 0, 255]);
        assert_eq!(px(a, 7, 0), [0, 0, 0, 0]);
        assert_eq!(px(a, 5, 7), [0, 0, 0, 0], "A never receives B's stroke");
        assert_eq!(px(b, 5, 7), [0, 0, 255, 255]);
        assert_eq!(px(b, 5, 0), [0, 0, 0, 0], "B never receives A's stroke");
        assert_eq!(undo_count(a), (1, 0));
        assert_eq!(undo_count(b), (1, 0));
        pixmith_free(a);
        pixmith_free(b);
    }
}

#[test]
fn freeing_a_document_drops_its_stroke() {
    unsafe {
        let a = pixmith_new(8, 8);
        pixmith_begin_stroke(a);
        pixmith_stroke_begin(a, 0, 0, 0, 0, RED, 1, 0);
        pixmith_free(a);
        // Whatever the allocator hands out next (possibly the same address) starts without a stroke.
        for _ in 0..8 {
            let n = pixmith_new(8, 8);
            pixmith_stroke_to(n, 4, 4);
            assert_eq!(px(n, 4, 4), [0, 0, 0, 0]);
            assert_eq!(undo_count(n), (0, 0));
            pixmith_free(n);
        }
    }
}

#[test]
fn undo_selection_and_dirty_state_are_per_document() {
    unsafe {
        let (a, b) = (pixmith_new(8, 8), pixmith_new(8, 8));
        pixmith_mark_saved(a);
        pixmith_mark_saved(b);

        pixmith_begin_stroke(a);
        pixmith_draw_line(a, 0, 0, 1, 1, 1, 1, RED, 1);
        pixmith_begin_transaction(a);
        pixmith_begin_stroke(b); // B is not in A's transaction
        pixmith_draw_line(b, 0, 0, 2, 2, 2, 2, BLUE, 1);
        pixmith_begin_stroke(a);
        pixmith_draw_line(a, 0, 0, 3, 3, 3, 3, RED, 1);
        pixmith_begin_stroke(a); // swallowed by A's transaction
        pixmith_draw_line(a, 0, 0, 4, 4, 4, 4, RED, 1);
        pixmith_end_transaction(a);
        pixmith_begin_stroke(b);
        pixmith_draw_line(b, 0, 0, 5, 5, 5, 5, BLUE, 1);

        assert_eq!(undo_count(a), (2, 0), "transaction folded 3 steps... into 2");
        assert_eq!(undo_count(b), (2, 0), "B is unaffected by A's transaction");
        assert!(pixmith_is_dirty(a) && pixmith_is_dirty(b));

        // Selection state is per document.
        pixmith_sel_begin(a, 0);
        pixmith_sel_rect(a, 0, 0, 3, 3);
        pixmith_sel_end(a);
        assert!(pixmith_sel_active(a));
        assert!(!pixmith_sel_active(b));

        // Undo in A leaves B alone.
        assert!(pixmith_undo(a));
        assert_eq!(px(b, 2, 2), [0, 0, 255, 255]);
        assert_eq!(px(b, 5, 5), [0, 0, 255, 255]);
        assert!(pixmith_is_dirty(b));
        pixmith_mark_saved(b);
        assert!(pixmith_undo(a));
        assert!(pixmith_redo(a));
        assert!(!pixmith_is_dirty(b), "saving or undoing A does not change B's saved state");
        assert_eq!(undo_count(b), (2, 0));

        // A floating paste in A is not in B.
        let data = [255u8; 2 * 2 * 4];
        assert!(pixmith_sel_paste(a, 0, 0, 2, 2, data.as_ptr(), 4, 4));
        assert!(pixmith_float_active(a));
        assert!(!pixmith_float_active(b));
        pixmith_free(a);
        pixmith_free(b);
    }
}

#[test]
fn cel_clipboard_across_documents() {
    unsafe {
        let (a, b, c) = (pixmith_new(8, 8), pixmith_new(8, 8), pixmith_new(4, 4));
        pixmith_begin_stroke(a);
        pixmith_draw_line(a, 0, 0, 2, 2, 2, 2, RED, 1);
        assert!(pixmith_cels_copy(a, 0, 0, 0, 0));
        assert!(pixmith_cels_has_clip());

        // Same size: pasting into another document works and only changes that document.
        assert_eq!(pixmith_frame_count(b), 1);
        assert!(pixmith_cels_paste(b, 0, 0));
        assert_eq!(px(b, 2, 2), [255, 0, 0, 255]);
        assert_eq!(undo_count(b), (1, 0));
        assert_eq!(undo_count(a), (1, 0));

        // Different size: refused cleanly, nothing changes.
        assert!(!pixmith_cels_paste(c, 0, 0));
        assert_eq!(undo_count(c), (0, 0));

        // Paste past the document's cels is refused, not a crash.
        let _ = pixmith_add_layer(b, 0);
        assert_eq!(pixmith_layer_count(b), 2);
        assert!(!pixmith_cels_paste(b, 5, 0));
        assert!(!pixmith_cels_paste(b, 0, 9));
        let _ = pixmith_add_frame(a, 0, false);
        pixmith_free(a);
        // The clipboard holds its own cels, so it survives the source document being freed.
        assert!(pixmith_cels_paste(b, 1, 0));
        pixmith_free(b);
        pixmith_free(c);
    }
}
