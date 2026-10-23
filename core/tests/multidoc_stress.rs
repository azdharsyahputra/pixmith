//! Drives several documents alternately through the C ABI with a fixed-seed PRNG, the way the
//! app does when it has several tabs open. Invariants: nothing panics, an operation on one
//! document never changes another one (sprite, undo/redo counts, dirty flag), undo and dirty
//! tracking are per document, and process-wide state (the freehand stroke, the cel clipboard)
//! never leaks into a document that did not start it.

use pixmith_core::document::{color_mode as colormode, layers};
use pixmith_core::document::{Editor, Sprite};
use pixmith_core::ffi::document::{
    pixmith_add_frame, pixmith_add_layer, pixmith_begin_stroke, pixmith_fill, pixmith_frame_count, pixmith_free,
    pixmith_layer_count, pixmith_new, pixmith_redo, pixmith_remove_frame, pixmith_undo,
};
use pixmith_core::history;
use pixmith_core::paint::{curves, filters, ink, selection, shapes, transform};
use pixmith_core::palette::{edit as palette_edit, io as palette_io, presets as palette_presets};
use pixmith_core::scripting as script;
use std::ffi::{c_char, CString};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo + 1) as u64) as i32
    }

    fn color(&mut self) -> u32 {
        let c = self.next() as u32;
        // Mostly opaque, sometimes translucent or erasing.
        match self.below(8) {
            0 => c & 0xFFFF_FF7F,
            1 => c & 0xFFFF_FF00,
            _ => c | 0xFF,
        }
    }
}

/// Everything about a document that another document's operations must never change.
#[derive(Clone, PartialEq, Debug)]
struct Snap {
    sprite: Sprite,
    undo: u32,
    redo: u32,
    dirty: bool,
    float: bool,
    xform: bool,
}

unsafe fn snap(p: *mut Editor) -> Snap {
    let (mut undo, mut redo) = (0, 0);
    history::pixmith_history_counts(p, &mut undo, &mut redo);
    Snap {
        sprite: (*p).sprite.clone(),
        undo,
        redo,
        dirty: history::pixmith_is_dirty(p),
        float: selection::pixmith_float_active(p),
        xform: transform::pixmith_xf_active(p),
    }
}

struct Tab {
    ptr: *mut Editor,
    /// Sprite as it was before the last undoable edit, valid while nothing else touched the tab.
    pending_undo: Option<Sprite>,
    /// Sprite at the last `pixmith_mark_saved`.
    saved: Sprite,
    txn: u32,
}

const SIZES: [(u32, u32); 4] = [(16, 16), (16, 16), (24, 12), (8, 8)];

unsafe fn new_tab(rng: &mut Rng) -> Tab {
    let (w, h) = SIZES[rng.below(SIZES.len() as u64) as usize];
    let ptr = pixmith_new(w, h);
    assert!(!ptr.is_null());
    history::pixmith_mark_saved(ptr);
    Tab { saved: (*ptr).sprite.clone(), ptr, pending_undo: None, txn: 0 }
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap()
}

struct World {
    rng: Rng,
    tabs: Vec<Option<Tab>>,
    /// Slots that may have a freehand stroke running. Whether strokes of several documents can run
    /// at once is up to the core, so a flagged slot may or may not change; an unflagged one must not.
    stroke: Vec<bool>,
    log: Vec<String>,
}

impl World {
    unsafe fn new(seed: u64, docs: usize) -> World {
        let mut rng = Rng(seed | 1);
        let tabs = (0..docs).map(|_| Some(new_tab(&mut rng))).collect();
        World { rng, stroke: vec![false; docs], tabs, log: Vec::new() }
    }

    fn alive(&self) -> Vec<usize> {
        (0..self.tabs.len()).filter(|&i| self.tabs[i].is_some()).collect()
    }

    fn p(&self, d: usize) -> *mut Editor {
        self.tabs[d].as_ref().unwrap().ptr
    }

    unsafe fn pos(&mut self, d: usize) -> (u32, u32, i32, i32) {
        let (w, h) = ((*self.p(d)).sprite.width as i32, (*self.p(d)).sprite.height as i32);
        let (x, y) = (self.rng.range(-2, w + 1), self.rng.range(-2, h + 1));
        (0, 0, x, y)
    }

    unsafe fn layer_frame(&mut self, d: usize) -> (u32, u32) {
        let p = self.p(d);
        let l = self.rng.below(pixmith_layer_count(p) as u64) as u32;
        let f = self.rng.below(pixmith_frame_count(p) as u64) as u32;
        (l, f)
    }

    /// Finishes floating paste / free transform in `d`, as the app does before other edits.
    unsafe fn settle(&mut self, d: usize) {
        let p = self.p(d);
        if transform::pixmith_xf_active(p) {
            transform::pixmith_xf_commit(p);
        }
        if selection::pixmith_float_active(p) {
            selection::pixmith_float_commit(p);
        }
    }

    unsafe fn step(&mut self) {
        let alive = self.alive();
        let d = alive[self.rng.below(alive.len() as u64) as usize];
        let op = self.rng.below(34);
        let others: Vec<(usize, Snap)> = alive.iter().filter(|&&i| i != d).map(|&i| (i, snap(self.p(i)))).collect();
        let before = snap(self.p(d));
        self.log.push(format!("doc {d}: op {op}"));

        // Whether the operation may change this document, and whether it recorded an undo step.
        let mut touches = true;
        let mut undoable = false;
        let mut keep_pending = false;
        let mut freed = false;

        let p = self.p(d);
        // Like the app, any other edit of this document ends its running stroke (the mouse is up).
        if self.stroke[d] && !(0..=7).contains(&op) {
            shapes::pixmith_stroke_end();
            self.stroke.fill(false);
        }
        match op {
            // Freehand strokes, interleaved across documents.
            0..=3 => {
                self.settle(d);
                let (l, f) = self.layer_frame(d);
                let (_, _, x, y) = self.pos(d);
                let (color, size, flags) = (self.rng.color(), self.rng.range(1, 4) as u32, self.rng.below(32) as u32);
                pixmith_begin_stroke(p);
                shapes::pixmith_stroke_begin(p, l, f, x, y, color, size, flags);
                self.stroke[d] = true;
                for _ in 0..self.rng.below(5) {
                    let (_, _, x, y) = self.pos(d);
                    shapes::pixmith_stroke_to(p, x, y);
                }
                undoable = true;
            }
            4..=6 => {
                // Continue the process-wide stroke through this document. Only the owner may change.
                let (_, _, x, y) = self.pos(d);
                shapes::pixmith_stroke_to(p, x, y);
                touches = self.stroke[d];
                keep_pending = true;
            }
            7 => {
                shapes::pixmith_stroke_end();
                self.stroke.fill(false);
                touches = false;
                keep_pending = true;
            }
            // Shapes, curves, polygons.
            8 | 9 => {
                self.settle(d);
                let (l, f) = self.layer_frame(d);
                let (_, _, x0, y0) = self.pos(d);
                let (_, _, x1, y1) = self.pos(d);
                pixmith_begin_stroke(p);
                let kind = self.rng.below(3) as u32;
                let (c, s, fl) = (self.rng.color(), self.rng.range(1, 3) as u32, self.rng.below(32) as u32);
                shapes::pixmith_shape(p, kind, l, f, x0, y0, x1, y1, c, s, fl, false);
                shapes::pixmith_shape(p, kind, l, f, x1, y1, x0, y0, c, s, fl, true);
                undoable = true;
            }
            10 | 11 => {
                self.settle(d);
                let (l, f) = self.layer_frame(d);
                let pts: Vec<i32> = (0..8).map(|_| self.rng.range(-2, 20)).collect();
                pixmith_begin_stroke(p);
                let (c, s) = (self.rng.color(), self.rng.range(1, 3) as u32);
                if op == 10 {
                    curves::pixmith_curve(p, l, f, pts.as_ptr(), c, s, 0, false);
                } else {
                    curves::pixmith_polygon(p, l, f, pts.as_ptr(), 4, c, s, self.rng.below(16) as u32, false);
                }
                undoable = true;
            }
            // Fills and gradients.
            12 | 13 => {
                self.settle(d);
                let (l, f) = self.layer_frame(d);
                let (_, _, x, y) = self.pos(d);
                pixmith_begin_stroke(p);
                if op == 12 {
                    pixmith_fill(p, l, f, x, y, self.rng.color());
                } else {
                    shapes::pixmith_fill_mode(p, l, f, x, y, self.rng.color(), self.rng.below(2) == 0);
                }
                undoable = true;
            }
            14 => {
                self.settle(d);
                let (l, f) = self.layer_frame(d);
                let (_, _, x0, y0) = self.pos(d);
                let (_, _, x1, y1) = self.pos(d);
                pixmith_begin_stroke(p);
                ink::pixmith_gradient(
                    p,
                    l,
                    f,
                    x0,
                    y0,
                    x1,
                    y1,
                    self.rng.color(),
                    self.rng.color(),
                    self.rng.below(2) as u32,
                    [0, 2, 4][self.rng.below(3) as usize],
                    false,
                );
                undoable = true;
            }
            // Selection, floating paste, free transform: started here, left open while other
            // documents are edited.
            15 => {
                self.settle(d);
                let (_, _, x0, y0) = self.pos(d);
                let (_, _, x1, y1) = self.pos(d);
                selection::pixmith_sel_rect(p, x0, y0, x1, y1);
                touches = false; // the selection is not part of the sprite
            }
            16 | 17 => {
                self.settle(d);
                let (l, f) = self.layer_frame(d);
                let (w, h) = (self.rng.range(1, 6) as u32, self.rng.range(1, 6) as u32);
                let data: Vec<u8> = (0..w * h * 4).map(|_| self.rng.next() as u8).collect();
                let (_, _, x, y) = self.pos(d);
                undoable = selection::pixmith_sel_paste(p, l, f, w, h, data.as_ptr(), x, y);
                if undoable && self.rng.below(2) == 0 {
                    selection::pixmith_sel_move_to(p, self.rng.range(-3, 3), self.rng.range(-3, 3));
                }
            }
            18 => {
                self.settle(d);
                let (l, f) = self.layer_frame(d);
                undoable = transform::pixmith_xf_begin(p, l, f, self.rng.below(2) == 0);
                if undoable {
                    transform::pixmith_xf_set_field(p, 2, self.rng.range(-90, 90) as f64);
                }
            }
            19 => {
                // Adjust or end the transform / floating paste if one is running in this tab.
                if transform::pixmith_xf_active(p) {
                    transform::pixmith_xf_set_field(p, 2, self.rng.range(-45, 45) as f64);
                    keep_pending = true;
                } else if selection::pixmith_float_active(p) {
                    if self.rng.below(2) == 0 {
                        selection::pixmith_float_commit(p);
                    } else {
                        selection::pixmith_float_cancel(p);
                    }
                }
            }
            // Transactions: opened in one document while another is edited.
            20 => {
                pixmith_core::history::pixmith_begin_transaction(p);
                self.tabs[d].as_mut().unwrap().txn += 1;
                touches = false;
                keep_pending = true;
            }
            21 => {
                if self.tabs[d].as_ref().unwrap().txn > 0 {
                    history::pixmith_end_transaction(p);
                    self.tabs[d].as_mut().unwrap().txn -= 1;
                }
                touches = false;
                keep_pending = true;
            }
            // Timeline: cel clipboard copy here, paste elsewhere (any size), frames and layers.
            22 => {
                let (l, f) = self.layer_frame(d);
                let (l1, f1) = (l + self.rng.below(2) as u32, f + self.rng.below(2) as u32);
                layers::pixmith_cels_copy(
                    p,
                    l,
                    l1.min(pixmith_layer_count(p) - 1),
                    f,
                    f1.min(pixmith_frame_count(p) - 1),
                );
                touches = false;
                keep_pending = true;
            }
            23 | 24 => {
                self.settle(d);
                let (l, f) = self.layer_frame(d);
                undoable = layers::pixmith_cels_paste(p, l, f);
            }
            25 => {
                self.settle(d);
                if pixmith_frame_count(p) < 6 {
                    pixmith_add_frame(p, 0, self.rng.below(2) == 0);
                    undoable = true;
                } else {
                    undoable = pixmith_remove_frame(p, 1);
                }
            }
            26 => {
                self.settle(d);
                if pixmith_layer_count(p) < 4 {
                    pixmith_add_layer(p, 0);
                    undoable = true;
                }
            }
            // Lua.
            27 | 28 => {
                self.settle(d);
                let (l, f) = self.layer_frame(d);
                let (x, y) = (self.rng.range(0, 7), self.rng.range(0, 7));
                let src = if self.rng.below(6) == 0 {
                    "this is not lua".to_string()
                } else {
                    format!(
                        "sprite:setPixel({x}, {y}, color.rgba({}, {}, 7, 255), {l}, {f})\napp.undoLabel('stress')\napp.alert('d{d}')",
                        self.rng.below(256),
                        self.rng.below(256)
                    )
                };
                let src = cstr(&src);
                let mut msgs: *mut c_char = std::ptr::null_mut();
                let ok = script::pixmith_run_script_at(p, src.as_ptr(), l, f, &mut msgs);
                if !msgs.is_null() {
                    pixmith_core::ffi::pixmith_string_free(msgs);
                }
                if !ok {
                    assert!((*p).sprite == before.sprite, "a failed script must leave the document untouched");
                }
                undoable = ok;
            }
            // Palette and color mode.
            29 => {
                self.settle(d);
                undoable = match self.rng.below(6) {
                    0 => palette_presets::pixmith_pe_preset_load(p, self.rng.below(12) as u32),
                    1 => palette_edit::pixmith_pe_insert(p, 0, self.rng.color()),
                    2 => palette_edit::pixmith_pe_move(p, [0u32].as_ptr(), 1, 3) >= 0,
                    3 => palette_edit::pixmith_pe_ramp(p, 0, 1, 2, self.rng.below(2) == 0) >= 0,
                    4 => colormode::pixmith_cm_palette_from_sprite(p, 8) > 0,
                    _ => {
                        palette_io::pixmith_palette_sort(p, 1);
                        true
                    }
                };
            }
            30 => {
                self.settle(d);
                let mode = self.rng.below(3) as u32;
                undoable = colormode::pixmith_cm_convert(p, mode, [0, 4][self.rng.below(2) as usize]);
            }
            31 => {
                self.settle(d);
                let (l, f) = self.layer_frame(d);
                let (kind, params) = (1u32, [0i32; 0]);
                undoable = filters::pixmith_filter_apply(p, kind, 0, l, f, 0, 0, 0, 0, params.as_ptr(), 0);
            }
            // History.
            32 => {
                self.settle(d);
                let tab = self.tabs[d].as_mut().unwrap();
                while tab.txn > 0 {
                    history::pixmith_end_transaction(p);
                    tab.txn -= 1;
                }
                let pending = tab.pending_undo.take();
                match self.rng.below(4) {
                    0 | 1 => {
                        let was = snap(p);
                        if pixmith_undo(p) {
                            if let Some(expected) = pending {
                                assert!(
                                    (*p).sprite == expected,
                                    "undo must restore the state before the last edit (log: {:?})",
                                    &self.log[self.log.len().saturating_sub(6)..]
                                );
                            }
                            // And redo brings the edit back exactly.
                            assert!(pixmith_redo(p));
                            assert!((*p).sprite == was.sprite, "redo after undo restores the state");
                            if self.rng.below(2) == 0 {
                                assert!(pixmith_undo(p));
                            }
                        }
                    }
                    2 => {
                        pixmith_redo(p);
                    }
                    _ => {
                        let s = snap(p);
                        let target = self.rng.below((s.undo + s.redo + 1) as u64) as u32;
                        assert!(history::pixmith_history_goto(p, target));
                        assert_eq!(snap(p).undo, target);
                    }
                }
                keep_pending = false;
            }
            33 => {
                // Mark saved / save-state bookkeeping, and tab churn: close one tab and open a new
                // one in its slot (so a freed address is likely to be reused) while others go on.
                if self.rng.below(2) == 0 {
                    self.settle(d);
                    history::pixmith_mark_saved(p);
                    let tab = self.tabs[d].as_mut().unwrap();
                    tab.saved = (*p).sprite.clone();
                    touches = false;
                } else {
                    // The tab is closed, possibly with a transaction or stroke still open.
                    pixmith_free(p);
                    self.tabs[d] = None;
                    self.stroke[d] = false;
                    freed = true;
                    if alive.len() == 1 || self.rng.below(3) != 0 {
                        self.tabs[d] = Some(new_tab(&mut self.rng));
                    }
                }
            }
            _ => unreachable!(),
        }

        // Invariant: no other document changed.
        for (i, s) in others {
            assert!(
                snap(self.p(i)) == s,
                "doc {i} changed by an operation on doc {d} (log: {:?})",
                &self.log[self.log.len().saturating_sub(8)..]
            );
        }
        if freed {
            return;
        }
        let after = snap(p);
        assert_eq!(
            (*p).history_drops(),
            0,
            "history discarded by op {op} on doc {d} (log: {:?})",
            &self.log[self.log.len().saturating_sub(10)..]
        );
        if !touches {
            assert!(
                after.sprite == before.sprite,
                "op {op} must not change doc {d}'s sprite (log: {:?})",
                &self.log[self.log.len().saturating_sub(8)..]
            );
        }
        // Per-document dirty tracking: a clean document equals its last saved state.
        let tab = self.tabs[d].as_mut().unwrap();
        if !after.dirty {
            assert!(
                after.sprite == tab.saved,
                "doc {d} is clean but differs from its saved state (op {op}, undo {} redo {}, log: {:?})",
                after.undo,
                after.redo,
                &self.log[self.log.len().saturating_sub(10)..]
            );
        }
        if undoable && tab.txn == 0 {
            assert!(after.dirty || after.sprite == tab.saved);
            tab.pending_undo = Some(before.sprite.clone());
        } else if !keep_pending {
            tab.pending_undo = None;
        }
    }

    unsafe fn finish(&mut self) {
        for d in self.alive() {
            let p = self.p(d);
            while self.tabs[d].as_ref().unwrap().txn > 0 {
                history::pixmith_end_transaction(p);
                self.tabs[d].as_mut().unwrap().txn -= 1;
            }
            self.settle(d);
            // Walk the whole history of every document back and forth.
            let s = snap(p);
            assert!(history::pixmith_history_goto(p, 0));
            assert!(history::pixmith_history_goto(p, s.undo + s.redo));
            assert_eq!((*p).history_drops(), 0, "history was discarded: an edit bypassed checkpoint");
            pixmith_free(p);
        }
        shapes::pixmith_stroke_end();
    }
}

unsafe fn run(seed: u64, docs: usize, steps: usize) {
    let mut w = World::new(seed, docs);
    for _ in 0..steps {
        w.step();
    }
    w.finish();
}

#[test]
fn two_documents_stay_independent() {
    for seed in 1..=40 {
        unsafe { run(seed, 2, 400) };
    }
}

#[test]
fn three_and_four_documents_stay_independent() {
    for seed in 100..=120 {
        unsafe { run(seed, 3, 400) };
    }
    for seed in 200..=215 {
        unsafe { run(seed, 4, 500) };
    }
}

/// Regression: a freehand stroke left running when its document is freed must not be continued
/// into a new document that happens to be allocated at the same address.
#[test]
fn freeing_a_document_drops_its_running_stroke() {
    unsafe {
        let mut hits = 0;
        for _ in 0..8 {
            let a = pixmith_new(8, 8);
            pixmith_begin_stroke(a);
            shapes::pixmith_stroke_begin(a, 0, 0, 1, 1, 0xFF0000FF, 1, 0);
            pixmith_free(a);
            let b = pixmith_new(8, 8);
            if b == a {
                hits += 1;
            }
            let before = (*b).sprite.clone();
            shapes::pixmith_stroke_to(b, 5, 5);
            assert_eq!((*b).sprite, before, "a stale stroke drew into a new document");
            shapes::pixmith_stroke_end();
            pixmith_free(b);
        }
        // Not every allocator hands the freed block straight back (Windows' does not), so only insist
        // on reuse where it is known to happen.
        assert!(hits > 0 || cfg!(miri) || !cfg!(unix), "address reuse never happened, so the test proved nothing");
    }
}

/// A stroke started in one document is not continued by calls for another one, and the first
/// document can continue it afterwards.
#[test]
fn interleaved_strokes_never_cross_documents() {
    unsafe {
        let (a, b) = (pixmith_new(8, 8), pixmith_new(8, 8));
        pixmith_begin_stroke(a);
        shapes::pixmith_stroke_begin(a, 0, 0, 0, 0, 0xFF0000FF, 1, 0);
        let b0 = (*b).sprite.clone();
        shapes::pixmith_stroke_to(b, 7, 7);
        assert_eq!((*b).sprite, b0);
        shapes::pixmith_stroke_to(a, 3, 0);
        assert_ne!((*a).sprite.render(0)[3 * 4..3 * 4 + 4], [0, 0, 0, 0]);
        shapes::pixmith_stroke_end();
        pixmith_free(a);
        pixmith_free(b);
    }
}

#[test]
fn cel_clipboard_pastes_into_documents_of_other_sizes() {
    unsafe {
        let (a, b, c) = (pixmith_new(16, 16), pixmith_new(8, 8), pixmith_new(16, 16));
        pixmith_begin_stroke(a);
        shapes::pixmith_shape(a, 1, 0, 0, 2, 2, 12, 12, 0x00FF00FF, 1, 8, false);
        assert!(layers::pixmith_cels_copy(a, 0, 0, 0, 0));
        let (sa, sb) = ((*a).sprite.clone(), (*b).sprite.clone());
        // A clipboard from a canvas of another size is refused and changes nothing.
        assert!(!layers::pixmith_cels_paste(b, 0, 0));
        assert_eq!((*a).sprite, sa, "pasting must not change the source document");
        assert_eq!((*b).sprite, sb);
        assert!(layers::pixmith_cels_paste(c, 0, 0));
        assert_eq!((*c).sprite.render(0), sa.render(0), "same-size paste is exact");
        pixmith_free(a);
        pixmith_free(b);
        pixmith_free(c);
    }
}

#[test]
fn dirty_flags_and_undo_are_per_document() {
    unsafe {
        let (a, b) = (pixmith_new(8, 8), pixmith_new(8, 8));
        history::pixmith_mark_saved(a);
        history::pixmith_mark_saved(b);
        pixmith_begin_stroke(a);
        pixmith_fill(a, 0, 0, 1, 1, 0xFF0000FF);
        assert!(history::pixmith_is_dirty(a) && !history::pixmith_is_dirty(b));
        pixmith_begin_stroke(b);
        pixmith_fill(b, 0, 0, 2, 2, 0x0000FFFF);
        history::pixmith_mark_saved(b);
        assert!(history::pixmith_is_dirty(a) && !history::pixmith_is_dirty(b));
        assert!(pixmith_undo(a));
        assert!(!history::pixmith_is_dirty(a), "undoing back to the saved state is clean");
        assert!(!history::pixmith_is_dirty(b));
        assert!(pixmith_undo(b) && history::pixmith_is_dirty(b));
        pixmith_free(a);
        assert!(pixmith_redo(b) && !history::pixmith_is_dirty(b));
        pixmith_free(b);
    }
}

#[test]
fn transactions_are_per_document() {
    unsafe {
        let (a, b) = (pixmith_new(8, 8), pixmith_new(8, 8));
        history::pixmith_begin_transaction(a);
        for i in 0..3 {
            pixmith_begin_stroke(a);
            pixmith_fill(a, 0, 0, i, 0, 0xFF0000FF);
            // Edits to b in the middle of a's transaction are undoable on their own.
            pixmith_begin_stroke(b);
            pixmith_fill(b, 0, 0, i, 1, 0x0000FFFF);
        }
        let (mut ua, mut ra, mut ub, mut rb) = (0, 0, 0, 0);
        history::pixmith_history_counts(a, &mut ua, &mut ra);
        history::pixmith_history_counts(b, &mut ub, &mut rb);
        assert_eq!((ua, ub), (1, 3));
        history::pixmith_end_transaction(a);
        assert!(pixmith_undo(a) && !pixmith_undo(a));
        assert!(pixmith_undo(b) && pixmith_undo(b) && pixmith_undo(b) && !pixmith_undo(b));
        pixmith_free(a);
        pixmith_free(b);
    }
}

/// Regression: saving while a transaction is open, then editing, must leave the document dirty
/// even though the transaction records no further undo step.
#[test]
fn editing_after_a_mid_transaction_save_is_dirty() {
    unsafe {
        let a = pixmith_new(8, 8);
        history::pixmith_mark_saved(a);
        history::pixmith_begin_transaction(a);
        pixmith_begin_stroke(a);
        pixmith_fill(a, 0, 0, 1, 1, 0xFF0000FF);
        history::pixmith_mark_saved(a);
        assert!(!history::pixmith_is_dirty(a));
        pixmith_begin_stroke(a);
        pixmith_fill(a, 0, 0, 5, 5, 0x00FF00FF);
        assert!(history::pixmith_is_dirty(a));
        history::pixmith_end_transaction(a);
        assert!(pixmith_undo(a) && !pixmith_undo(a));
        assert!(history::pixmith_is_dirty(a), "the saved state had the red fill");
        pixmith_free(a);
    }
}
