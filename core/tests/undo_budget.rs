//! Undo history is limited by bytes as well as by step count, and `Sprite::validate` catches
//! broken structure.

use pixmith_core::document::{Editor, Sprite};
use pixmith_core::history;
use pixmith_core::paint::selection;
use std::ffi::CString;
use std::sync::Arc;

const MIB: usize = 1 << 20;

/// A 512 x 512 sprite: every cel is 1 MiB.
fn big() -> Editor {
    let mut s = Sprite::new(512, 512);
    s.draw_line(0, 0, 0, 0, 1, 1, [1, 2, 3, 255], 1);
    Editor::new(s)
}

const KIB: usize = 1 << 10;

/// One labeled edit that overwrites a 128 x 128 block with fresh noise. Old history keeps only
/// the old block (64 KiB, incompressible), while the newest snapshot still holds the whole old
/// 1 MiB cel until the next checkpoint.
fn step(e: &mut Editor, n: usize) {
    e.checkpoint();
    let cel = Arc::make_mut(e.sprite.cels[0][0].as_mut().unwrap());
    let mut x = (n as u32).wrapping_mul(2654435761).wrapping_add(12345);
    for row in 0..128usize {
        for col in 0..128usize {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            let i = ((row + 100) * 512 + col + 100) * 4;
            cel.pixels[i..i + 4].copy_from_slice(&(x | 0x0100_0001).to_le_bytes());
        }
    }
    let label = CString::new(n.to_string()).unwrap();
    unsafe { history::pixmith_checkpoint_label(e, label.as_ptr()) };
}

fn counts(e: &Editor) -> (u32, u32) {
    let (mut u, mut r) = (0, 0);
    unsafe { history::pixmith_history_counts(e, &mut u, &mut r) };
    (u, r)
}

fn label(e: &Editor, i: u32) -> String {
    let p = unsafe { history::pixmith_history_label(e, i) };
    assert!(!p.is_null(), "no label {i}");
    let s = unsafe { std::ffi::CStr::from_ptr(p) }.to_str().unwrap().to_string();
    unsafe { pixmith_core::ffi::pixmith_string_free(p) };
    s
}

#[test]
fn history_stays_under_the_byte_budget() {
    let mut e = big();
    e.set_undo_budget(2 * MIB);
    for n in 0..120 {
        step(&mut e, n);
        // The newest snapshot also holds the old cel until the next checkpoint (1 MiB).
        assert!(e.history_bytes() <= 3 * MIB + 16 * KIB, "step {n}: {} bytes", e.history_bytes());
    }
    let (undo, _) = counts(&e);
    assert!((20..=40).contains(&undo), "kept {undo} steps of 64 KiB each");
    // The default budget of 1 GiB leaves small histories alone.
    let mut f = big();
    for n in 0..30 {
        step(&mut f, n);
    }
    assert_eq!(counts(&f).0, 30);
}

#[test]
fn undo_and_redo_stay_exact_after_trimming() {
    let mut e = big();
    e.set_undo_budget(MIB + 256 * KIB);
    let mut renders = vec![e.sprite.render(0)];
    for n in 0..40 {
        step(&mut e, n);
        renders.push(e.sprite.render(0));
    }
    let kept = counts(&e).0 as usize;
    assert!((10..40).contains(&kept), "kept {kept}");
    // Walking back reproduces each of the last `kept` states exactly.
    for k in 1..=kept {
        assert!(e.undo());
        assert!(e.sprite.render(0) == renders[40 - k], "state {} after {k} undos", 40 - k);
    }
    assert!(!e.undo(), "nothing older was kept");
    // And forward again.
    for k in (0..kept).rev() {
        assert!(e.redo());
        assert!(e.sprite.render(0) == renders[40 - k]);
    }
    assert!(!e.redo());
}

#[test]
fn labels_and_history_positions_stay_aligned() {
    let mut e = big();
    e.set_undo_budget(MIB + 512 * KIB);
    for n in 0..60 {
        step(&mut e, n);
    }
    let (undo, redo) = counts(&e);
    assert_eq!(redo, 0);
    // The kept steps are the most recent ones, labeled in order.
    for i in 0..undo {
        assert_eq!(label(&e, i), (60 - undo + i).to_string(), "label {i}");
    }
    // Go back two steps: the redo labels follow the applied ones.
    assert!(unsafe { history::pixmith_history_goto(&mut e, undo - 2) });
    assert_eq!(counts(&e), (undo - 2, 2));
    assert_eq!(label(&e, undo - 2), (60 - 2).to_string());
    assert_eq!(label(&e, undo - 1), (60 - 1).to_string());
    assert!(unsafe { history::pixmith_history_goto(&mut e, 0) });
    assert!(unsafe { history::pixmith_history_goto(&mut e, undo) });
    assert!(!unsafe { history::pixmith_history_goto(&mut e, undo + 1) });
}

#[test]
fn dropping_the_saved_state_keeps_the_document_modified() {
    // Within budget: undoing back to the saved state is clean.
    let mut e = big();
    unsafe { history::pixmith_mark_saved(&mut e) };
    for n in 0..5 {
        step(&mut e, n);
    }
    assert!(unsafe { history::pixmith_is_dirty(&e) });
    for _ in 0..5 {
        assert!(e.undo());
    }
    assert!(!unsafe { history::pixmith_is_dirty(&e) });

    // Over budget: the saved state is trimmed away, so no undo can reach "clean" again.
    let mut e = big();
    e.set_undo_budget(512 * KIB);
    unsafe { history::pixmith_mark_saved(&mut e) };
    for n in 0..30 {
        step(&mut e, n);
    }
    while e.undo() {
        assert!(unsafe { history::pixmith_is_dirty(&e) }, "must stay modified, the saved state is gone");
    }
    // Saving again at any point makes it clean; the oldest kept state is then not the saved one.
    unsafe { history::pixmith_mark_saved(&mut e) };
    assert!(!unsafe { history::pixmith_is_dirty(&e) });
    assert!(e.redo());
    assert!(unsafe { history::pixmith_is_dirty(&e) });
    assert!(e.undo());
    assert!(!unsafe { history::pixmith_is_dirty(&e) });
}

#[test]
fn lowering_the_budget_trims_at_once_but_keeps_the_newest_step() {
    let mut e = big();
    for n in 0..20 {
        step(&mut e, n);
    }
    assert_eq!(counts(&e).0, 20);
    // After an edit the newest snapshot owns the old 1 MiB cel; the older steps are 64 KiB each.
    let budget = MIB + 3 * 64 * KIB + KIB;
    unsafe { history::pixmith_set_undo_budget(&mut e, budget as u64) };
    let (undo, _) = counts(&e);
    assert!((3..=5).contains(&undo), "kept {undo}");
    assert!(unsafe { history::pixmith_undo_bytes(&e) } <= budget as u64, "lowering the budget trims exactly to it");
    unsafe { history::pixmith_set_undo_budget(&mut e, 0) };
    assert_eq!(counts(&e).0, 1, "the last edit can always be undone");
    assert!(e.undo());
    assert!(!e.undo());
}

#[test]
fn buffers_shared_with_the_live_sprite_cost_almost_nothing() {
    let mut e = big();
    // Checkpoints without changes share every buffer with the live sprite.
    for _ in 0..50 {
        e.checkpoint();
    }
    assert!(e.history_bytes() < 64 * KIB, "{}", e.history_bytes());
    step(&mut e, 1);
    assert!(e.history_bytes() >= MIB && e.history_bytes() < 2 * MIB, "{}", e.history_bytes());
    // Undone steps (redo) are counted too: the redo entry holds the 64 KiB block of the step.
    assert!(e.undo());
    assert!(e.history_bytes() >= 32 * KIB, "{}", e.history_bytes());
}

#[test]
fn selection_masks_are_packed_in_history_and_restore_exactly() {
    let mut s = Sprite::new(1024, 1024);
    s.draw_line(0, 0, 0, 0, 1, 1, [1, 2, 3, 255], 1);
    let mut e = Editor::new(s);
    for n in 0..20 {
        unsafe { selection::pixmith_sel_rect(&mut e, 0, 0, n, n) };
    }
    // Each mask is 1 MiB live, but only the newest history entry keeps one in full.
    assert!(e.history_bytes() < MIB + 64 * KIB, "{}", e.history_bytes());
    assert_eq!(counts(&e).0, 20);
    let size = |e: &Editor| {
        let mut b = [0i32; 4];
        unsafe { selection::pixmith_sel_bounds(e, b.as_mut_ptr()) }.then_some(b[2])
    };
    assert_eq!(size(&e), Some(20));
    for want in (1..20).rev() {
        assert!(e.undo());
        assert_eq!(size(&e), Some(want));
    }
    assert!(e.undo());
    assert_eq!(size(&e), None, "before the first selection there was none");
    for want in 1..=20 {
        assert!(e.redo());
        assert_eq!(size(&e), Some(want));
    }
}

#[test]
fn validate_accepts_normal_sprites_and_names_broken_ones() {
    let mut s = Sprite::new(4, 3);
    s.draw_line(0, 0, 0, 0, 3, 2, [9, 9, 9, 255], 1);
    s.add_frame(0, true);
    s.add_layer(0);
    let g = s.add_group(0);
    assert!(s.validate().is_ok(), "{:?}", s.validate());
    assert!(g < s.layers.len());

    let broken = |f: &dyn Fn(&mut Sprite)| {
        let mut b = s.clone();
        f(&mut b);
        b.validate().unwrap_err()
    };
    assert!(broken(&|b| b.cels[1].pop().map(|_| ()).unwrap_or(())).contains("cel slots"));
    assert!(broken(&|b| b.durations.pop().map(|_| ()).unwrap_or(())).contains("durations"));
    assert!(broken(&|b| {
        b.cels[0][0] = Some(Arc::new(pixmith_core::document::Cel { pixels: vec![0; 8], ..Default::default() }));
    })
    .contains("bytes"));
    assert!(broken(&|b| b.layers[0].child_level = 3).contains("nested"));
    assert!(broken(&|b| {
        b.layers[0].kind = pixmith_core::document::tilemap::KIND_TILEMAP;
        b.layers[0].tileset = 7;
    })
    .contains("tileset"));
    assert!(broken(&|b| {
        b.cels[0][0] = Some(Arc::new(pixmith_core::document::Cel {
            pixels: vec![0; 48],
            indices: Some(Arc::new(vec![0; 3])),
            ..Default::default()
        }));
    })
    .contains("hints"));
    assert!(broken(&|b| b.layers.clear()).contains("no layers"));
}

#[test]
fn valid_files_still_open_after_the_structure_check() {
    // A sprite whose cel buffer has the wrong size would panic on the first edit.
    let mut s = Sprite::new(4, 4);
    s.draw_line(0, 0, 0, 0, 1, 1, [1, 1, 1, 255], 1);
    let good = pixmith_core::formats::ase::write(&s);
    let dir = std::env::temp_dir().join(format!("pixmith-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ok.aseprite");
    std::fs::write(&path, &good).unwrap();
    let c = CString::new(path.to_str().unwrap()).unwrap();
    let doc = unsafe { pixmith_core::ffi::document::pixmith_open(c.as_ptr()) };
    assert!(!doc.is_null());
    unsafe { pixmith_core::ffi::document::pixmith_free(doc) };
    std::fs::remove_dir_all(&dir).unwrap();
}
