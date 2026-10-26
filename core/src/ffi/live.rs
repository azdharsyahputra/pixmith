//! Live mode support: a document as bytes and back, for the Pixmith Live protocol (`docs/LIVE_PROTOCOL.md`).
//!
//! A front end hands a client the document as `.pxmt` bytes ([`encode_document`]); the client edits its copy and
//! sends the result back, which [`apply_document`] swaps in as one labelled undo step.

use super::{editor, editor_quiet, set_error};
use crate::document::Editor;
use crate::formats::pxmt;
use std::ffi::{c_char, CStr};

/// The document as `.pxmt` bytes (sprite, thumbnail, metadata).
pub fn encode_document(e: &mut Editor) -> Result<Vec<u8>, String> {
    e.sprite.flush_links();
    pxmt::write(&e.sprite)
}

/// Replaces the sprite with the document in `bytes` (`.pxmt` or `.aseprite`) as ONE undo step named `label`
/// (unnamed when empty). The selection is kept unless the canvas size changes. When the bytes cannot be decoded
/// or fail validation the editor is left untouched.
pub fn apply_document(e: &mut Editor, bytes: &[u8], label: &str) -> Result<(), String> {
    apply_document_with(e, bytes, label, &SelectionChange::Keep)
}

/// The selection as a region of the canvas: its bounding rectangle and one byte per pixel inside it (row-major,
/// 1 selected, 0 not).
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub mask: Vec<u8>,
}

/// What `apply_document_with` does with the selection.
#[derive(Clone, Debug, PartialEq)]
pub enum SelectionChange {
    /// Kept, unless the canvas size changes (then it is cleared).
    Keep,
    Clear,
    /// Replaced; the region is in the coordinates of the new document.
    Set(Region),
}

/// A new editor holding the document in `bytes` (`.pxmt` or `.aseprite`), decoded and validated before anything
/// else happens: a front end creates the tab only when this succeeded, so a failed request leaves nothing behind.
pub fn decode_document(bytes: &[u8]) -> Result<Editor, String> {
    pxmt::read_document(bytes).map(Editor::new)
}

/// The active selection as a [`Region`], `None` when nothing is selected.
pub fn selection_region(e: &Editor) -> Option<Region> {
    let (w, h) = (e.sprite.width, e.sprite.height);
    let sel = e.sel.active(w, h)?;
    let (x, y, bw, bh) = sel.bounds()?;
    let mut mask = Vec::with_capacity(bw as usize * bh as usize);
    for row in y..y + bh {
        for col in x..x + bw {
            mask.push(u8::from(sel.get(col, row)));
        }
    }
    Some(Region { x, y, w: bw as u32, h: bh as u32, mask })
}

fn selection_from(region: &Region, w: u32, h: u32) -> Result<crate::paint::selection::Selection, String> {
    let cells = (region.w as usize).checked_mul(region.h as usize);
    if cells != Some(region.mask.len()) {
        return Err(format!("the selection mask has {} bytes, not {} x {}", region.mask.len(), region.w, region.h));
    }
    let mut sel = crate::paint::selection::Selection::new(w, h, false);
    for row in 0..region.h as i64 {
        for col in 0..region.w as i64 {
            let (x, y) = (region.x as i64 + col, region.y as i64 + row);
            let inside = (0..w as i64).contains(&x) && (0..h as i64).contains(&y);
            if inside && region.mask[(row * region.w as i64 + col) as usize] != 0 {
                sel.bits[(y * w as i64 + x) as usize] = true;
            }
        }
    }
    Ok(sel)
}

/// [`apply_document`] that can also clear or replace the selection, still as the same one undo step.
pub fn apply_document_with(
    e: &mut Editor,
    bytes: &[u8],
    label: &str,
    selection: &SelectionChange,
) -> Result<(), String> {
    let next = pxmt::read_document(bytes)?;
    let new_mask = match selection {
        SelectionChange::Set(region) => Some(selection_from(region, next.width, next.height)?),
        _ => None,
    };
    let resized = (next.width, next.height) != (e.sprite.width, e.sprite.height);
    e.checkpoint();
    e.label_last(label);
    e.sprite = next;
    match (selection, new_mask) {
        (SelectionChange::Set(_), Some(mask)) => {
            e.sel.mask = crate::paint::selection::nonempty(mask);
            e.sel.reset_interaction();
        }
        (SelectionChange::Clear, _) => {
            e.sel.mask = None;
            e.sel.reset_interaction();
        }
        _ if resized => {
            e.sel.mask = None;
            e.sel.reset_interaction();
        }
        _ => {}
    }
    Ok(())
}

/// The document as .pxmt bytes. `*len` receives the size. NULL on failure. Free with `pixmith_bytes_free`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_encode_document(doc: *const Editor, len: *mut usize) -> *mut u8 {
    crate::ffi::guarded("pixmith_encode_document", std::ptr::null_mut(), || {
        let Some(e) = editor_quiet(doc as *mut Editor) else {
            set_error("no document");
            return std::ptr::null_mut();
        };
        match encode_document(e) {
            Ok(bytes) => {
                let boxed = bytes.into_boxed_slice();
                if !len.is_null() {
                    *len = boxed.len();
                }
                Box::into_raw(boxed) as *mut u8
            }
            Err(msg) => {
                set_error(msg);
                std::ptr::null_mut()
            }
        }
    })
}

/// Identifies the state the document is in: it differs after every edit, undo, redo and `pixmith_touch`, so a front
/// end can tell that a document changed. 0 for a null document.
#[no_mangle]
pub unsafe extern "C" fn pixmith_state_id(doc: *const Editor) -> u64 {
    crate::ffi::guarded("pixmith_state_id", std::ptr::null_mut(), || {
        (doc as *mut Editor).as_ref().map_or(0, |e| e.states.current_id())
    })
}

/// The selection's bounding rectangle (`*x`, `*y`, `*w`, `*h`) and its mask, one byte per pixel inside the rectangle
/// (row-major, 1 selected), `*len` = w * h. NULL when nothing is selected. Free with `pixmith_bytes_free`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_live_selection(
    doc: *const Editor,
    x: *mut i32,
    y: *mut i32,
    w: *mut u32,
    h: *mut u32,
    len: *mut usize,
) -> *mut u8 {
    crate::ffi::guarded("pixmith_live_selection", std::ptr::null_mut(), || {
        let Some(e) = editor_quiet(doc as *mut Editor) else { return std::ptr::null_mut() };
        let Some(r) = selection_region(e) else { return std::ptr::null_mut() };
        for (p, v) in [(x, r.x), (y, r.y)] {
            if !p.is_null() {
                *p = v;
            }
        }
        for (p, v) in [(w, r.w), (h, r.h)] {
            if !p.is_null() {
                *p = v;
            }
        }
        if !len.is_null() {
            *len = r.mask.len();
        }
        Box::into_raw(r.mask.into_boxed_slice()) as *mut u8
    })
}

/// A new document from `bytes` (`.pxmt` or `.aseprite`), not changed or dirty, with an empty history. NULL (see
/// `pixmith_last_error`) when the bytes cannot be decoded or validated. Free with `pixmith_free`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_decode_document(bytes: *const u8, len: usize) -> *mut Editor {
    crate::ffi::guarded("pixmith_decode_document", std::ptr::null_mut(), || {
        if bytes.is_null() {
            set_error("no document bytes");
            return std::ptr::null_mut();
        }
        match decode_document(std::slice::from_raw_parts(bytes, len)) {
            Ok(e) => Box::into_raw(Box::new(e)),
            Err(msg) => {
                set_error(msg);
                std::ptr::null_mut()
            }
        }
    })
}

/// Frees bytes returned by `pixmith_encode_document`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_bytes_free(bytes: *mut u8, len: usize) {
    crate::ffi::guarded("pixmith_bytes_free", std::ptr::null_mut(), || {
        if !bytes.is_null() {
            drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(bytes, len)));
        }
    })
}

/// Replaces the document's sprite with `bytes` (.pxmt or .aseprite) as ONE undo step named `label`. The selection
/// is kept unless the canvas size changes. False (document unchanged) if the bytes can't be decoded or validated.
#[no_mangle]
pub unsafe extern "C" fn pixmith_apply_document(
    doc: *mut Editor,
    bytes: *const u8,
    len: usize,
    label: *const c_char,
) -> bool {
    crate::ffi::guarded("pixmith_apply_document", doc, || {
        if bytes.is_null() {
            set_error("no document bytes");
            return false;
        }
        let Some(e) = editor(doc) else {
            set_error("no document");
            return false;
        };
        let label = if label.is_null() { String::new() } else { CStr::from_ptr(label).to_string_lossy().into_owned() };
        match apply_document(e, std::slice::from_raw_parts(bytes, len), &label) {
            Ok(()) => true,
            Err(msg) => {
                set_error(msg);
                false
            }
        }
    })
}

/// `pixmith_apply_document` that also handles the selection, in the same undo step. `mode`: 0 keep it (cleared when
/// the canvas size changes), 1 clear it, 2 replace it with the region (`x`, `y`, `w`, `h`, `mask` of w * h bytes, in
/// the coordinates of the new document).
#[no_mangle]
pub unsafe extern "C" fn pixmith_apply_document_selection(
    doc: *mut Editor,
    bytes: *const u8,
    len: usize,
    label: *const c_char,
    mode: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    mask: *const u8,
    mask_len: usize,
) -> bool {
    crate::ffi::guarded("pixmith_apply_document_selection", doc, || {
        if bytes.is_null() {
            set_error("no document bytes");
            return false;
        }
        let Some(e) = editor(doc) else {
            set_error("no document");
            return false;
        };
        let change = match mode {
            0 => SelectionChange::Keep,
            1 => SelectionChange::Clear,
            2 if !mask.is_null() || mask_len == 0 => SelectionChange::Set(Region {
                x,
                y,
                w,
                h,
                mask: if mask_len == 0 { Vec::new() } else { std::slice::from_raw_parts(mask, mask_len).to_vec() },
            }),
            _ => {
                set_error("bad selection arguments");
                return false;
            }
        };
        let label = if label.is_null() { String::new() } else { CStr::from_ptr(label).to_string_lossy().into_owned() };
        match apply_document_with(e, std::slice::from_raw_parts(bytes, len), &label, &change) {
            Ok(()) => true,
            Err(msg) => {
                set_error(msg);
                false
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Sprite;
    use std::ffi::CString;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn encode(e: &mut Editor) -> Vec<u8> {
        let mut len = 0usize;
        let p = unsafe { pixmith_encode_document(e, &mut len) };
        assert!(!p.is_null());
        let v = unsafe { std::slice::from_raw_parts(p, len) }.to_vec();
        unsafe { pixmith_bytes_free(p, len) };
        v
    }

    fn apply(e: &mut Editor, bytes: &[u8], label: &str) -> bool {
        let l = CString::new(label).unwrap();
        unsafe { pixmith_apply_document(e, bytes.as_ptr(), bytes.len(), l.as_ptr()) }
    }

    #[test]
    fn decoding_builds_a_whole_document_or_nothing() {
        let mut a = Editor::new(Sprite::new(8, 8));
        a.sprite.draw_line(0, 0, 1, 1, 5, 5, RED, 1);
        let bytes = encode(&mut a);
        let p = unsafe { pixmith_decode_document(bytes.as_ptr(), bytes.len()) };
        assert!(!p.is_null());
        let b = unsafe { Box::from_raw(p) };
        assert_eq!(b.sprite.render(0), a.sprite.render(0));
        assert!(unsafe { pixmith_decode_document(bytes.as_ptr(), bytes.len() / 2) }.is_null());
        assert!(unsafe { pixmith_decode_document(std::ptr::null(), 0) }.is_null());
    }

    #[test]
    fn encode_then_apply_round_trips() {
        let mut a = Editor::new(Sprite::new(8, 8));
        a.sprite.draw_line(0, 0, 1, 1, 5, 5, RED, 1);
        let bytes = encode(&mut a);
        assert!(pxmt::is_pxmt(&bytes));
        let mut b = Editor::new(Sprite::new(8, 8));
        assert!(apply(&mut b, &bytes, "Draw"));
        assert_eq!(b.sprite.render(0), a.sprite.render(0));
        assert_eq!(encode(&mut b).len(), bytes.len());
    }

    #[test]
    fn apply_is_one_labelled_undo_step() {
        let mut a = Editor::new(Sprite::new(8, 8));
        a.sprite.draw_line(0, 0, 1, 1, 5, 5, RED, 1);
        let bytes = encode(&mut a);
        let mut b = Editor::new(Sprite::new(8, 8));
        let blank = b.sprite.render(0);
        assert!(apply(&mut b, &bytes, "AI: Draw a line"));
        assert_eq!(b.history_len(), (1, 0));
        assert_eq!(b.history_label(0), Some("AI: Draw a line"));
        assert!(b.undo());
        assert_eq!(b.sprite.render(0), blank);
        assert!(b.redo());
        assert_eq!(b.sprite.render(0), a.sprite.render(0));
        assert!(b.undo());
        assert_eq!(b.history_len(), (0, 1));
    }

    #[test]
    fn bad_bytes_leave_the_document_untouched() {
        let mut e = Editor::new(Sprite::new(4, 4));
        e.sprite.draw_line(0, 0, 0, 0, 3, 3, BLUE, 1);
        let before = e.sprite.clone();
        for bytes in [&b""[..], b"hello", b"PXMT\0\0\0\0garbage", &[0xA5, 0xE0, 1, 2, 3][..]] {
            assert!(!apply(&mut e, bytes, "bad"));
        }
        let mut truncated = encode(&mut Editor::new(Sprite::new(4, 4)));
        truncated.truncate(truncated.len() / 2);
        assert!(!apply(&mut e, &truncated, "bad"));
        assert!(!unsafe { pixmith_apply_document(&mut e, std::ptr::null(), 0, std::ptr::null()) });
        assert_eq!(e.sprite, before);
        assert_eq!(e.history_len(), (0, 0), "a failed apply takes no undo step");
    }

    #[test]
    fn aseprite_bytes_are_accepted() {
        let mut a = Editor::new(Sprite::new(6, 3));
        a.sprite.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        let ase = crate::formats::ase::write(&a.sprite);
        let mut b = Editor::new(Sprite::new(2, 2));
        assert!(apply(&mut b, &ase, "from aseprite"));
        assert_eq!((b.sprite.width, b.sprite.height), (6, 3));
    }

    #[test]
    fn selection_survives_unless_the_canvas_size_changes() {
        let mut e = Editor::new(Sprite::new(8, 8));
        crate::paint::selection::select_rect(&mut e, (1, 1), (4, 4));
        assert!(e.sel.active(8, 8).is_some());
        let mut other = Editor::new(Sprite::new(8, 8));
        other.sprite.draw_line(0, 0, 0, 0, 7, 7, RED, 1);
        assert!(apply(&mut e, &encode(&mut other), "same size"));
        assert!(e.sel.active(8, 8).is_some(), "same canvas size keeps the selection");
        let mut bigger = Editor::new(Sprite::new(16, 16));
        assert!(apply(&mut e, &encode(&mut bigger), "bigger"));
        assert!(e.sel.mask.is_none());
        assert!(e.undo());
        assert_eq!((e.sprite.width, e.sprite.height), (8, 8));
        assert!(e.undo());
        assert!(e.sel.active(8, 8).is_some(), "undo brings the selection back");
    }

    #[test]
    fn state_id_changes_with_every_edit_and_returns_on_undo() {
        let mut e = Editor::new(Sprite::new(4, 4));
        let id = |e: &Editor| unsafe { pixmith_state_id(e) };
        let first = id(&e);
        e.checkpoint();
        let second = id(&e);
        e.states.touch();
        let third = id(&e);
        assert!(first != second && second != third && first != third);
        assert!(e.undo());
        assert_eq!(id(&e), first, "undo returns to the state before the step");
        assert_eq!(unsafe { pixmith_state_id(std::ptr::null()) }, 0);
    }

    #[test]
    fn selection_region_round_trips_and_applies_in_the_same_undo_step() {
        let mut e = Editor::new(Sprite::new(8, 8));
        assert_eq!(selection_region(&e), None);
        crate::paint::selection::select_rect(&mut e, (2, 1), (4, 2));
        let r = selection_region(&e).unwrap();
        assert_eq!((r.x, r.y, r.w, r.h), (2, 1, 3, 2));
        assert_eq!(r.mask, vec![1; 6]);
        // Through the C functions.
        let (mut x, mut y, mut w, mut h, mut len) = (0, 0, 0, 0, 0usize);
        let p = unsafe { pixmith_live_selection(&e, &mut x, &mut y, &mut w, &mut h, &mut len) };
        assert!(!p.is_null());
        assert_eq!((x, y, w, h, len), (2, 1, 3, 2, 6));
        unsafe { pixmith_bytes_free(p, len) };

        let mut other = Editor::new(Sprite::new(8, 8));
        other.sprite.draw_line(0, 0, 0, 0, 7, 7, RED, 1);
        let bytes = encode(&mut other);
        let steps = e.history_len().0;
        let region = Region { x: 0, y: 0, w: 2, h: 2, mask: vec![1, 0, 1, 1] };
        apply_document_with(&mut e, &bytes, "AI: select", &SelectionChange::Set(region.clone())).unwrap();
        assert_eq!(e.history_len().0, steps + 1, "one undo step");
        let got = selection_region(&e).unwrap();
        assert_eq!((got.x, got.y, got.w, got.h), (0, 0, 2, 2));
        assert_eq!(got.mask, region.mask);
        // One undo takes back both the pixels and the selection.
        assert!(e.undo());
        assert_eq!(selection_region(&e).unwrap().w, 3);
        assert!(e.redo());
        assert_eq!(selection_region(&e).unwrap().mask, region.mask);
        apply_document_with(&mut e, &bytes, "AI: clear", &SelectionChange::Clear).unwrap();
        assert_eq!(selection_region(&e), None);
        // A mask of the wrong size is refused and nothing changes.
        let before = e.sprite.clone();
        let bad = Region { x: 0, y: 0, w: 2, h: 2, mask: vec![1] };
        assert!(apply_document_with(&mut e, &bytes, "x", &SelectionChange::Set(bad)).is_err());
        assert_eq!(e.sprite, before);
        // Parts outside the canvas are ignored.
        let far = Region { x: 6, y: 6, w: 4, h: 4, mask: vec![1; 16] };
        apply_document_with(&mut e, &bytes, "AI: far", &SelectionChange::Set(far)).unwrap();
        let r = selection_region(&e).unwrap();
        assert_eq!((r.x, r.y, r.w, r.h), (6, 6, 2, 2));
    }

    #[test]
    fn null_arguments_are_tolerated() {
        unsafe {
            let mut len = 0usize;
            assert!(pixmith_encode_document(std::ptr::null(), &mut len).is_null());
            pixmith_bytes_free(std::ptr::null_mut(), 0);
            assert!(!pixmith_apply_document(std::ptr::null_mut(), b"x".as_ptr(), 1, std::ptr::null()));
        }
    }
}
