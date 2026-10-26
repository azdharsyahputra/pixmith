//! Tween rig: parented layers (thigh -> shin -> boot). Composition goldens, cycles, bake, persistence.
//! Everything runs in memory; nothing here touches HOME or the support directory.

use pixmith_core::document::{Editor, Sprite};
use pixmith_core::formats::{ase, pxmt};
use pixmith_core::gamedev::rig;
use pixmith_core::gamedev::tween::{self, pixmith_tween_options, Ease, Prop};

const SIZE: u32 = 96;
const FRAMES: usize = 5;
const LIN: Ease = Ease::Linear;

// Rest pose (corner coordinates): hip, knee and ankle on one vertical line.
const HIP: (f32, f32) = (48.0, 30.0);
const KNEE: (f32, f32) = (48.0, 42.0);
const ANKLE: (f32, f32) = (48.0, 54.0);

// Keyed values per frame 0..=4: (rotation, x, y).
const THIGH: [(f32, f32, f32); FRAMES] =
    [(0.0, 0.0, 0.0), (90.0, 1.0, 0.0), (180.0, 2.0, 0.0), (90.0, 3.0, 0.0), (0.0, 4.0, 0.0)];
const SHIN: [(f32, f32, f32); FRAMES] =
    [(0.0, 0.0, 0.0), (0.0, 0.0, 0.0), (-90.0, 0.0, 0.0), (90.0, 0.0, 0.0), (0.0, 0.0, 0.0)];
const BOOT: [(f32, f32, f32); FRAMES] =
    [(0.0, 0.0, 0.0), (90.0, 0.0, 2.0), (0.0, 2.0, 3.0), (180.0, 0.0, 2.0), (0.0, 0.0, 0.0)];

const RED: [u8; 4] = [255, 0, 0, 255];
const PINK: [u8; 4] = [255, 128, 128, 255];
const GREEN: [u8; 4] = [0, 200, 0, 255];
const LIME: [u8; 4] = [160, 255, 160, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const SKY: [u8; 4] = [128, 192, 255, 255];

/// Marker pixels (main, tip) of the three limb layers in the rest pose.
const MARKERS: [((i32, i32), (i32, i32)); 3] = [((48, 34), (49, 34)), ((48, 46), (49, 46)), ((48, 58), (49, 58))];
const COLORS: [([u8; 4], [u8; 4]); 3] = [(RED, PINK), (GREEN, LIME), (BLUE, SKY)];

/// Golden pixel positions [thigh, shin, boot] x [main, tip] per frame, computed by an independent
/// script (rotate about each pivot, apply the parent chain, floor the pixel center).
#[rustfmt::skip]
const GOLDEN: [[[(i32, i32); 2]; 3]; FRAMES] = [
    [[(48, 34), (49, 34)], [(48, 46), (49, 46)], [(48, 58), (49, 58)]],
    [[(44, 30), (44, 31)], [(32, 30), (32, 31)], [(22, 25), (21, 25)]],
    [[(49, 25), (48, 25)], [(45, 18), (45, 19)], [(30, 20), (30, 21)]],
    [[(46, 30), (46, 31)], [(38, 25), (37, 25)], [(39, 20), (40, 20)]],
    [[(52, 34), (53, 34)], [(52, 46), (53, 46)], [(52, 58), (53, 58)]],
];

fn blank(frames: usize) -> Sprite {
    let mut s = Sprite::new(SIZE, SIZE);
    while s.frame_count() < frames {
        s.add_frame(s.frame_count() - 1, false);
    }
    s
}

fn key_all(e: &mut Editor, layer: usize, vals: &[(f32, f32, f32); FRAMES], pivot: (f32, f32)) {
    for (f, &(rot, x, y)) in vals.iter().enumerate() {
        assert!(tween::set_key(e, layer, Prop::Rotation, f as u32, rot, LIN));
        assert!(tween::set_key(e, layer, Prop::PosX, f as u32, x, LIN));
        assert!(tween::set_key(e, layer, Prop::PosY, f as u32, y, LIN));
    }
    assert!(tween::set_pivot(e, layer, Some(pivot)));
}

/// Three layers (0 thigh, 1 shin, 2 boot), each with two marker pixels, keyed as above. Not parented yet.
fn marker_rig() -> Editor {
    let mut s = blank(FRAMES);
    s.add_layer(0);
    s.add_layer(1);
    for (l, (m, c)) in MARKERS.iter().zip(COLORS).enumerate() {
        s.draw_line(l, 0, m.0 .0, m.0 .1, m.0 .0, m.0 .1, c.0, 1);
        s.draw_line(l, 0, m.1 .0, m.1 .1, m.1 .0, m.1 .1, c.1, 1);
    }
    let mut e = Editor::new(s);
    key_all(&mut e, 0, &THIGH, HIP);
    key_all(&mut e, 1, &SHIN, KNEE);
    key_all(&mut e, 2, &BOOT, ANKLE);
    e
}

fn link_chain(e: &mut Editor) {
    assert!(rig::set_parent(e, 1, Some(0)));
    assert!(rig::set_parent(e, 2, Some(1)));
}

fn positions(rgba: &[u8], color: [u8; 4]) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    for (i, p) in rgba.as_chunks::<4>().0.iter().enumerate() {
        if *p == color {
            v.push(((i % SIZE as usize) as i32, (i / SIZE as usize) as i32));
        }
    }
    v
}

#[test]
fn three_level_chain_lands_on_golden_pixels() {
    let mut e = marker_rig();
    link_chain(&mut e);
    for (f, golden) in GOLDEN.iter().enumerate() {
        let img = e.sprite.render(f);
        for (l, (g, c)) in golden.iter().zip(COLORS).enumerate() {
            assert_eq!(positions(&img, c.0), vec![g[0]], "frame {f} layer {l} main");
            assert_eq!(positions(&img, c.1), vec![g[1]], "frame {f} layer {l} tip");
        }
    }
}

#[test]
fn without_links_children_ignore_their_parents() {
    // Same keys, no parenting: the shin does not follow the thigh (pure own transform).
    let e = marker_rig();
    let img = e.sprite.render(1);
    // Shin rotation is 0 at frame 1 and nothing moves it.
    assert_eq!(positions(&img, GREEN), vec![(48, 46)]);
    assert_eq!(positions(&img, LIME), vec![(49, 46)]);
}

#[test]
fn child_pivot_is_in_its_own_space_and_follows_the_parent() {
    // Non-quarter angles: check the matrices numerically.
    let mut e = marker_rig();
    link_chain(&mut e);
    let (t, s, b) = (30.0f64, 40.0f64, 25.0f64);
    for f in 0..FRAMES as u32 {
        for (l, v) in [(0usize, t), (1, s), (2, b)] {
            tween::set_key(&mut e, l, Prop::Rotation, f, v as f32, LIN);
            tween::set_key(&mut e, l, Prop::PosX, f, 0.0, LIN);
            tween::set_key(&mut e, l, Prop::PosY, f, 0.0, LIN);
        }
    }
    let rot = |deg: f64, c: (f64, f64), p: (f64, f64)| {
        let (s, co) = deg.to_radians().sin_cos();
        let (dx, dy) = (p.0 - c.0, p.1 - c.1);
        (c.0 + co * dx - s * dy, c.1 + s * dx + co * dy)
    };
    let (h, k, a) = ((48.0, 30.0), (48.0, 42.0), (48.0, 54.0));
    // The knee is moved only by the thigh; the shin's own rotation fixes it.
    let knee = rig::map_point(&rig::world_matrix(&e.sprite, 1, 2).unwrap(), k.0, k.1);
    let want = rot(t, h, k);
    assert!((knee.0 - want.0).abs() < 1e-9 && (knee.1 - want.1).abs() < 1e-9, "{knee:?} vs {want:?}");
    // The ankle is moved by the thigh and by the shin about the (moved) knee.
    let ankle = rig::map_point(&rig::world_matrix(&e.sprite, 2, 2).unwrap(), a.0, a.1);
    let want = rot(t, h, rot(s, k, a));
    assert!((ankle.0 - want.0).abs() < 1e-9 && (ankle.1 - want.1).abs() < 1e-9, "{ankle:?} vs {want:?}");
    // The composed query reports the world position of the layer's own pivot and the total rotation.
    let c = rig::composed_transform(&e.sprite, 2, 2).unwrap();
    assert!((c.pivot_world.0 - want.0).abs() < 1e-9 && (c.pivot_world.1 - want.1).abs() < 1e-9);
    assert!((c.rotation - (t + s + b)).abs() < 1e-9, "{}", c.rotation);
    assert!((c.scale_x - 1.0).abs() < 1e-9 && (c.scale_y - 1.0).abs() < 1e-9);
    // And the local part is the layer's own transform only.
    let lr = c.local[1].atan2(c.local[0]).to_degrees();
    assert!((lr - b).abs() < 1e-9);
}

#[test]
fn scale_composes_through_the_chain() {
    let mut e = marker_rig();
    link_chain(&mut e);
    for f in 0..FRAMES as u32 {
        for l in 0..3 {
            tween::set_key(&mut e, l, Prop::Rotation, f, 0.0, LIN);
            tween::set_key(&mut e, l, Prop::PosX, f, 0.0, LIN);
            tween::set_key(&mut e, l, Prop::PosY, f, 0.0, LIN);
        }
        tween::set_key(&mut e, 0, Prop::ScaleX, f, 2.0, LIN);
        tween::set_key(&mut e, 1, Prop::ScaleY, f, 3.0, LIN);
    }
    // Ankle (48, 54): shin scales y by 3 about knee y=42 -> 42 + 36 = 78; thigh scales x by 2 about hip x=48 (no change at x=48).
    let p = rig::map_point(&rig::world_matrix(&e.sprite, 2, 0).unwrap(), 48.0, 54.0);
    assert!((p.0 - 48.0).abs() < 1e-9 && (p.1 - 78.0).abs() < 1e-9, "{p:?}");
    let q = rig::map_point(&rig::world_matrix(&e.sprite, 2, 0).unwrap(), 50.0, 42.0);
    assert!((q.0 - 52.0).abs() < 1e-9 && (q.1 - 42.0).abs() < 1e-9, "{q:?}");
}

#[test]
fn a_child_without_keys_follows_its_parent() {
    let mut s = blank(FRAMES);
    s.add_layer(0);
    s.draw_line(0, 0, 48, 34, 48, 34, RED, 1);
    s.draw_line(1, 0, 60, 34, 60, 34, GREEN, 1);
    let mut e = Editor::new(s);
    for f in 0..FRAMES as u32 {
        tween::set_key(&mut e, 0, Prop::PosX, f, f as f32 * 2.0, LIN);
    }
    assert!(rig::set_parent(&mut e, 1, Some(0)));
    assert_eq!(rig::effective_range(&e.sprite, 1), Some((0, 4)));
    for f in 0..FRAMES {
        let img = e.sprite.render(f);
        assert_eq!(positions(&img, GREEN), vec![(60 + 2 * f as i32, 34)], "frame {f}");
    }
    // Unlinking returns the child to its own cels.
    assert!(rig::set_parent(&mut e, 1, None));
    assert_eq!(positions(&e.sprite.render(0), GREEN), vec![(60, 34)]);
    assert!(positions(&e.sprite.render(3), GREEN).is_empty(), "only frame 0 has a cel");
    // Neither layer keeps tween state the other does not need.
    assert!(tween::load(&e.sprite, 1).is_none());
}

#[test]
fn cycles_are_refused() {
    let mut e = marker_rig();
    assert!(!rig::set_parent(&mut e, 0, Some(0)), "self");
    link_chain(&mut e);
    assert!(!rig::set_parent(&mut e, 0, Some(1)), "2-cycle");
    assert!(!rig::set_parent(&mut e, 0, Some(2)), "3-cycle");
    assert!(!rig::set_parent(&mut e, 1, Some(2)), "2-cycle below");
    assert!(!rig::can_parent(&e.sprite, 0, 2));
    assert!(rig::can_parent(&e.sprite, 2, 0), "re-parenting a leaf to the root is fine");
    assert!(!rig::set_parent(&mut e, 1, Some(9)), "no such layer");
    // Nothing changed.
    assert_eq!(rig::parent_of(&e.sprite, 0), None);
    assert_eq!(rig::parent_of(&e.sprite, 1), Some(0));
    assert_eq!(rig::parent_of(&e.sprite, 2), Some(1));
    assert_eq!(rig::chain(&e.sprite, 2), vec![2, 1, 0]);
    assert_eq!(rig::children_of(&e.sprite, 0), vec![1]);
    // Re-parenting works and replaces the old link.
    assert!(rig::set_parent(&mut e, 2, Some(0)));
    assert_eq!(rig::chain(&e.sprite, 2), vec![2, 0]);
    assert!(rig::children_of(&e.sprite, 1).is_empty());
}

#[test]
fn a_cycle_in_a_hand_edited_file_does_not_hang() {
    let mut e = marker_rig();
    link_chain(&mut e);
    // Make layer 0 the child of layer 2 by editing the payloads directly (what a hostile file could contain).
    let mut s = e.sprite.clone();
    let mut st2 = tween::load(&s, 2).unwrap();
    st2.id = Some(77);
    tween::store(&mut s, 2, Some(&st2));
    let id2 = 77;
    let mut st0 = tween::load(&s, 0).unwrap();
    st0.parent = Some(id2);
    tween::store(&mut s, 0, Some(&st0));
    assert_eq!(rig::chain(&s, 0).len(), 3, "the walk stops when it would revisit a layer");
    for f in 0..FRAMES {
        let _ = s.render(f);
        for l in 0..3 {
            assert!(rig::world_matrix(&s, l, f as u32).is_some());
        }
    }
}

#[test]
fn set_parent_is_one_undo_step_and_unlink_cleans_up() {
    let mut e = marker_rig();
    let before = e.sprite.clone();
    let steps = e.history_len().0;
    assert!(rig::set_parent(&mut e, 1, Some(0)));
    assert_eq!(e.history_len().0, steps + 1);
    assert_eq!(rig::parent_of(&e.sprite, 1), Some(0));
    assert!(e.undo());
    assert_eq!(e.sprite, before);
    assert!(e.redo());
    assert_eq!(rig::parent_of(&e.sprite, 1), Some(0));
    // Setting the same parent again is a no-op.
    let steps = e.history_len().0;
    assert!(rig::set_parent(&mut e, 1, Some(0)));
    assert_eq!(e.history_len().0, steps);
    // Unlinking removes the parent's id again: the payloads are what they were.
    assert!(rig::set_parent(&mut e, 1, None));
    for l in 0..3 {
        assert_eq!(tween::load(&e.sprite, l), tween::load(&before, l), "layer {l}");
    }
}

#[test]
fn links_survive_moving_inserting_and_deleting_layers() {
    let mut e = marker_rig();
    link_chain(&mut e);
    let want: Vec<_> = (0..FRAMES).map(|f| e.sprite.render(f)).collect();
    // Insert a layer below everything: all indices shift by one.
    e.sprite.add_layer(0);
    // add_layer inserts above 0, i.e. at 1; the chain moved up by one for layers >= 1.
    assert_eq!(rig::parent_of(&e.sprite, 0), None);
    let (thigh, shin, boot) = (0usize, 2usize, 3usize);
    assert_eq!(rig::parent_of(&e.sprite, shin), Some(thigh));
    assert_eq!(rig::parent_of(&e.sprite, boot), Some(shin));
    // Moving the shin layer around keeps the link to its parent and from its child.
    let moved = e.sprite.move_layer(shin, true).unwrap();
    let boot = if moved == 3 { 2 } else { boot };
    assert_eq!(rig::parent_of(&e.sprite, moved), Some(thigh));
    assert_eq!(rig::parent_of(&e.sprite, boot), Some(moved));
    for (f, w) in want.iter().enumerate() {
        let img = e.sprite.render(f);
        for c in [RED, PINK, GREEN, LIME, BLUE, SKY] {
            assert_eq!(positions(&img, c), positions(w, c), "frame {f}");
        }
    }
    // Deleting the parent orphans the child: it renders as if unparented and nothing panics.
    assert!(e.sprite.remove_layer(thigh));
    let orphan = if moved > thigh { moved - 1 } else { moved };
    assert_eq!(rig::parent_of(&e.sprite, orphan), None);
    let _ = e.sprite.render(2);
    // A new parent never inherits the stale link of the deleted one.
    assert!(rig::set_parent(&mut e, 0, Some(orphan)));
    assert_eq!(rig::parent_of(&e.sprite, orphan), None, "the dangling link must not attach to the new parent's id");
}

#[test]
fn duplicating_a_layer_does_not_steal_its_children() {
    let mut e = marker_rig();
    link_chain(&mut e);
    let copy = e.sprite.duplicate_layer(0).unwrap();
    assert_eq!(copy, 1, "the copy sits just above the original, pushing the shin and boot up");
    // The children still follow the original thigh, not the copy.
    assert_eq!(rig::parent_of(&e.sprite, 2), Some(0));
    assert_eq!(rig::parent_of(&e.sprite, 3), Some(2));
    assert_eq!(rig::parent_of(&e.sprite, copy), None);
    assert_eq!(rig::children_of(&e.sprite, copy), Vec::<usize>::new());
}

#[test]
fn bake_all_equals_manual_composition() {
    let (e0, expected) = block_rig();
    // Live render before baking (the reference for the composite).
    let live: Vec<_> = (0..FRAMES).map(|f| e0.sprite.render(f)).collect();
    let mut e = e0;
    assert_eq!(tween::bake_all(&mut e, false, false), 3);
    for (l, per_frame) in expected.iter().enumerate() {
        for (f, want) in per_frame.iter().enumerate() {
            let cel = e.sprite.cels[f][l].as_ref().unwrap_or_else(|| panic!("cel {l}/{f}"));
            assert_eq!(&cel.pixels, want, "layer {l} frame {f}");
        }
        assert!(tween::load(&e.sprite, l).is_none(), "tracks and links are gone after a full bake");
    }
    for (f, want) in live.iter().enumerate() {
        assert_eq!(&e.sprite.render(f), want, "composite frame {f}");
    }
    assert!(e.undo(), "one undo step");
    assert_eq!(rig::parent_of(&e.sprite, 2), Some(1));
}

#[test]
fn bake_all_into_new_layers_keeps_the_composite() {
    let (e0, _) = block_rig();
    let live: Vec<_> = (0..FRAMES).map(|f| e0.sprite.render(f)).collect();
    let mut e = e0;
    assert_eq!(tween::bake_all(&mut e, true, true), 3);
    assert_eq!(e.sprite.layers.len(), 6);
    for l in 0..e.sprite.layers.len() {
        assert_eq!(e.sprite.layers[l].name.ends_with("(baked)"), e.sprite.layers[l].visible, "layer {l}");
    }
    for (f, want) in live.iter().enumerate() {
        assert_eq!(&e.sprite.render(f), want, "composite frame {f}");
    }
}

#[test]
fn baking_a_child_alone_uses_the_composed_transform() {
    let (e0, expected) = block_rig();
    let live: Vec<_> = (0..FRAMES).map(|f| e0.sprite.render(f)).collect();
    let mut e = e0;
    assert!(tween::bake(&mut e, 2, false));
    for (f, want) in expected[2].iter().enumerate().take(FRAMES) {
        assert_eq!(&e.sprite.cels[f][2].as_ref().unwrap().pixels, want, "boot frame {f}");
    }
    // The boot no longer follows the shin (it was baked), the others are still live.
    assert_eq!(rig::parent_of(&e.sprite, 2), None);
    assert_eq!(rig::parent_of(&e.sprite, 1), Some(0));
    for (f, want) in live.iter().enumerate() {
        assert_eq!(&e.sprite.render(f), want, "composite frame {f}");
    }
}

#[test]
fn baking_a_parent_in_place_is_refused_while_it_has_children() {
    let (mut e, _) = block_rig();
    assert!(!tween::bake(&mut e, 0, false), "children would lose the motion");
    assert!(tween::load(&e.sprite, 0).is_some());
    // Keeping the source (the tracks stay) is fine, also into a new layer.
    assert!(tween::bake(&mut e, 0, true));
    assert!(tween::bake_with(
        &mut e,
        0,
        tween::BakeOptions { range: tween::BakeRange::All, new_layer: true, keep_source: true }
    )
    .is_some());
}

#[test]
fn bake_current_frame_of_a_child() {
    let (mut e, expected) = block_rig();
    let layer = tween::bake_with(
        &mut e,
        2,
        tween::BakeOptions { range: tween::BakeRange::Current(3), new_layer: true, keep_source: true },
    )
    .unwrap();
    assert_eq!(e.sprite.cels[3][layer].as_ref().unwrap().pixels, expected[2][3]);
    assert!(e.sprite.cels[2][layer].is_none());
}

#[test]
fn pxmt_and_aseprite_round_trips_keep_the_rig() {
    let mut e = marker_rig();
    link_chain(&mut e);
    let old = e.sprite.layers[1].user_data.text.clone().unwrap();
    e.sprite.layers[1].user_data.text = Some(format!("my shin\n{old}"));
    let want: Vec<_> = (0..FRAMES).map(|f| e.sprite.render(f)).collect();
    let states: Vec<_> = (0..3).map(|l| tween::load(&e.sprite, l)).collect();

    let back = pxmt::read(&pxmt::write(&e.sprite).unwrap()).unwrap();
    let back_ase = ase::read(&ase::write(&e.sprite)).unwrap();
    for b in [&back, &back_ase] {
        assert_eq!(rig::parent_of(b, 0), None);
        assert_eq!(rig::parent_of(b, 1), Some(0));
        assert_eq!(rig::parent_of(b, 2), Some(1));
        for (l, st) in states.iter().enumerate() {
            assert_eq!(&tween::load(b, l), st);
        }
        assert!(b.layers[1].user_data.text.as_deref().unwrap().starts_with("my shin\n"));
        for (f, w) in want.iter().enumerate() {
            assert_eq!(&b.render(f), w, "frame {f}");
        }
    }
    // The extension data is plain layer user data text, which is all Aseprite itself sees.
    let text = back_ase.layers[2].user_data.text.as_deref().unwrap();
    assert!(text.contains("pixmith.tween.v1:") && text.contains("pa="), "{text:?}");
}

#[test]
fn files_without_parents_are_unchanged() {
    // A payload as written before parenting existed: it decodes without a parent and writes back byte for byte.
    let legacy = "note\n\u{1e}pixmith.tween.v1:rs=1;ss=1;pv=7.5,8;px=0,0,lin~3,4,hold;rot=1,0,lin~4,45.5,ob";
    let mut s = blank(FRAMES);
    s.layers[0].user_data.text = Some(legacy.into());
    let st = tween::load(&s, 0).unwrap();
    assert_eq!((st.id, st.parent), (None, None));
    assert_eq!(rig::parent_of(&s, 0), None);
    assert_eq!(rig::chain(&s, 0), vec![0]);
    assert_eq!(rig::effective_range(&s, 0), st.range());
    tween::store(&mut s, 0, Some(&st));
    assert_eq!(s.layers[0].user_data.text.as_deref(), Some(legacy));
    // Round-tripped through the file formats too.
    let back = pxmt::read(&pxmt::write(&s).unwrap()).unwrap();
    assert_eq!(back.layers[0].user_data.text.as_deref(), Some(legacy));
    // Editing keys of a parentless layer never introduces id or pa.
    let mut e = Editor::new(s);
    tween::set_key(&mut e, 0, Prop::PosY, 2, 3.0, LIN);
    let text = e.sprite.layers[0].user_data.text.clone().unwrap();
    assert!(!text.contains("id=") && !text.contains("pa="), "{text:?}");
}

#[test]
fn c_abi_round_trip() {
    use pixmith_core::gamedev::rig::*;
    let mut e = marker_rig();
    let d: *mut Editor = &mut e;
    unsafe {
        assert!(pixmith_tween_set_parent(d, 1, 0));
        assert!(pixmith_tween_set_parent(d, 2, 1));
        assert!(!pixmith_tween_set_parent(d, 0, 2), "cycle");
        assert!(!pixmith_tween_set_parent(d, 0, 0));
        assert!(!pixmith_tween_set_parent(d, 9, 0));
        assert_eq!(pixmith_tween_get_parent(d, 2), 1);
        assert_eq!(pixmith_tween_get_parent(d, 0), -1);
        assert!(!pixmith_tween_can_parent(d, 0, 2));
        assert!(pixmith_tween_can_parent(d, 2, 0));
        let mut kids = [0u32; 4];
        assert_eq!(pixmith_tween_children(d, 0, kids.as_mut_ptr(), 4), 1);
        assert_eq!(kids[0], 1);
        assert_eq!(pixmith_tween_children(d, 0, std::ptr::null_mut(), 0), 1);
        let mut m = [0f32; 11];
        assert!(pixmith_tween_composed_at(d, 1, 1, m.as_mut_ptr()));
        // Frame 1: the thigh turned 90 degrees, the shin adds nothing: total rotation 90.
        assert!((m[6] - 90.0).abs() < 1e-4 && (m[7] - 1.0).abs() < 1e-6, "{m:?}");
        let mut r = [0u32; 2];
        assert!(pixmith_tween_effective_range(d, 2, r.as_mut_ptr()) && r == [0, 4]);
        assert_eq!(pixmith_tween_options(d, 1) >> 5 & 1, 1);
        assert_eq!(pixmith_tween_options(d, 0) >> 5 & 1, 0);
        // Null documents and buffers are refused, not crashed on.
        assert!(!pixmith_tween_set_parent(std::ptr::null_mut(), 1, 0));
        assert_eq!(pixmith_tween_get_parent(std::ptr::null(), 1), -1);
        assert!(!pixmith_tween_composed_at(d, 1, 1, std::ptr::null_mut()));
        assert!(!pixmith_tween_composed_at(std::ptr::null(), 1, 1, m.as_mut_ptr()));
        assert_eq!(pixmith_tween_children(std::ptr::null(), 0, std::ptr::null_mut(), 0), 0);
    }
    assert_eq!(rig::parent_of(&e.sprite, 2), Some(1));
}

// MARK: Block rig for bake tests

/// Three layers with 3x6 blocks of unique colors along the leg, keyed and parented like the marker rig,
/// plus per-layer expected baked cels (computed here with exact integer math, independent of the crate).
fn block_rig() -> (Editor, Vec<Vec<Vec<u8>>>) {
    let mut s = blank(FRAMES);
    s.add_layer(0);
    s.add_layer(1);
    let mut blocks: Vec<Vec<(i32, i32, [u8; 4])>> = vec![Vec::new(); 3];
    for (l, blk) in blocks.iter_mut().enumerate() {
        let y0 = 30 + 12 * l as i32;
        for dy in 0..6 {
            for dx in 0..3 {
                let (x, y) = (47 + dx, y0 + dy);
                let c = [(10 + 60 * l + 7 * dx as usize) as u8, (20 + 25 * dy as usize) as u8, (l * 80) as u8, 255];
                s.draw_line(l, 0, x, y, x, y, c, 1);
                blk.push((x, y, c));
            }
        }
    }
    let mut e = Editor::new(s);
    key_all(&mut e, 0, &THIGH, HIP);
    key_all(&mut e, 1, &SHIN, KNEE);
    key_all(&mut e, 2, &BOOT, ANKLE);
    link_chain(&mut e);

    let local = |pv: (f32, f32), v: (f32, f32, f32), p: (f64, f64)| -> (f64, f64) {
        let (dx, dy) = (p.0 - pv.0 as f64, p.1 - pv.1 as f64);
        let (c, s) = match (v.0 as i32).rem_euclid(360) {
            0 => (1.0, 0.0),
            90 => (0.0, 1.0),
            180 => (-1.0, 0.0),
            270 => (0.0, -1.0),
            other => panic!("not a quarter turn: {other}"),
        };
        (pv.0 as f64 + v.1 as f64 + c * dx - s * dy, pv.1 as f64 + v.2 as f64 + s * dx + c * dy)
    };
    let tables = [(&THIGH, HIP), (&SHIN, KNEE), (&BOOT, ANKLE)];
    let mut expected = vec![vec![Vec::new(); FRAMES]; 3];
    for (l, per_frame) in expected.iter_mut().enumerate() {
        for (f, out) in per_frame.iter_mut().enumerate() {
            let mut buf = vec![0u8; (SIZE * SIZE * 4) as usize];
            for &(x, y, c) in &blocks[l] {
                let mut p = (x as f64 + 0.5, y as f64 + 0.5);
                for k in (0..=l).rev() {
                    p = local(tables[k].1, tables[k].0[f], p);
                }
                let (qx, qy) = (p.0.floor() as i32, p.1.floor() as i32);
                assert!((0..SIZE as i32).contains(&qx) && (0..SIZE as i32).contains(&qy), "block left the canvas");
                let i = (qy as usize * SIZE as usize + qx as usize) * 4;
                buf[i..i + 4].copy_from_slice(&c);
            }
            *out = buf;
        }
    }
    (e, expected)
}
