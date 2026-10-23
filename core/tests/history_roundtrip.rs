//! Randomized undo/redo/goto round trips over documents with linked cels, tilemaps, index hints
//! and selections: every state reached through history must equal the state recorded when it was
//! first created.

use pixmith_core::document::tilemap::{add_tilemap_layer, stamp};
use pixmith_core::document::{Editor, Sprite};
use pixmith_core::history;
use pixmith_core::paint::selection;
use std::sync::Arc;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Everything history is supposed to bring back: the sprite and the selection mask.
#[derive(PartialEq, Debug)]
struct State {
    sprite: Sprite,
    mask: Vec<bool>,
}

fn capture(e: &Editor) -> State {
    let (w, h) = (e.sprite.width as i32, e.sprite.height as i32);
    let mask = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| unsafe { selection::pixmith_sel_contains(e, x, y) })
        .collect();
    State { sprite: e.sprite.clone(), mask }
}

fn start() -> Editor {
    let mut s = Sprite::new(24, 16);
    s.draw_line(0, 0, 1, 1, 20, 12, [255, 0, 0, 255], 1);
    s.add_frame(0, true);
    s.add_frame(1, false);
    assert!(s.link_cels(0, 0, 2));
    let top = s.add_layer(0);
    s.draw_line(top, 1, 3, 3, 9, 3, [0, 0, 255, 255], 2);
    let tm = add_tilemap_layer(&mut s, top, 4, 4);
    let tile = s.tilesets[0].add_blank();
    Arc::make_mut(&mut s.tilesets[0].tiles[tile as usize])[0..4].copy_from_slice(&[0, 255, 0, 255]);
    stamp(&mut s, tm, 0, 4, 4, tile);
    if let Some(c) = s.cels[0][0].as_mut() {
        Arc::make_mut(c).indices = Some(Arc::new(vec![5; 24 * 16]));
    }
    Editor::new(s)
}

/// One recorded step: checkpoint, then a random edit.
fn step(e: &mut Editor, rng: &mut Rng) {
    let (layers, frames) = (e.sprite.layers.len(), e.sprite.frame_count());
    let color = [rng.below(256) as u8, rng.below(256) as u8, 7, 255];
    match rng.below(10) {
        0..=3 => {
            e.checkpoint();
            let (l, f) = (rng.below(layers), rng.below(frames));
            let (x, y) = (rng.below(24) as i32, rng.below(16) as i32);
            e.sprite.draw_line(
                l,
                f,
                x,
                y,
                x + rng.below(6) as i32,
                y + rng.below(4) as i32,
                color,
                1 + rng.below(2) as u32,
            );
        }
        4 => {
            e.checkpoint();
            e.sprite.add_layer(rng.below(layers));
        }
        5 => {
            e.checkpoint();
            e.sprite.add_frame(rng.below(frames), rng.below(2) == 0);
        }
        6 => {
            e.checkpoint();
            let l = rng.below(layers);
            let _ = e.sprite.link_cels(l, 0, frames - 1);
        }
        7 => {
            e.checkpoint();
            // A tile edit through the tilemap layer, if there is one.
            if let Some(l) =
                e.sprite.layers.iter().position(|l| l.kind == pixmith_core::document::tilemap::KIND_TILEMAP)
            {
                stamp(&mut e.sprite, l, rng.below(frames), rng.below(24) as i32, rng.below(16) as i32, 1);
                let t = e.sprite.tilesets[0].add_blank();
                Arc::make_mut(&mut e.sprite.tilesets[0].tiles[t as usize])[0..4].copy_from_slice(&color);
            }
        }
        8 => unsafe {
            // A selection change is one step; the begin call takes the checkpoint.
            selection::pixmith_sel_begin(e, 0);
            let (x, y) = (rng.below(20) as i32, rng.below(12) as i32);
            selection::pixmith_sel_rect(e, x, y, x + rng.below(8) as i32, y + rng.below(6) as i32);
            selection::pixmith_sel_end(e);
        },
        _ => {
            e.checkpoint();
            let (l, f) = (rng.below(layers), rng.below(frames));
            if let Some(Some(c)) = e.sprite.cels.get_mut(f).map(|r| &mut r[l]) {
                let cel = Arc::make_mut(c);
                cel.opacity = color[0];
                cel.indices = Some(Arc::new((0..24 * 16).map(|i| (i % 7) as u8 + color[1] % 5).collect()));
            }
        }
    }
}

fn run(seed: u64, steps: usize) {
    let mut rng = Rng(seed | 1);
    let mut e = start();
    let mut states = vec![capture(&e)];
    let mut pos = 0usize; // index of the state the editor shows
    for round in 0..steps {
        match rng.below(8) {
            0..=3 => {
                step(&mut e, &mut rng);
                states.truncate(pos + 1);
                states.push(capture(&e));
                pos += 1;
            }
            4 | 5 => {
                let available = counts(&e).0 > 0;
                let moved = e.undo();
                assert_eq!(moved, available, "undo availability");
                if moved {
                    pos -= 1;
                }
            }
            6 => {
                if e.redo() {
                    pos += 1;
                }
            }
            _ => {
                let (undo, redo) = counts(&e);
                let target = rng.below(undo + redo + 1);
                assert!(unsafe { history::pixmith_history_goto(&mut e, target as u32) });
                pos = pos + target - undo;
            }
        }
        assert!(capture(&e) == states[pos], "seed {seed} round {round}: state {pos} differs");
        assert_eq!(e.history_drops(), 0, "history was dropped (an edit bypassed checkpoint)");
    }
    // Walk to the very beginning of the kept history and back to the end.
    let (undo, redo) = counts(&e);
    let first = pos - undo;
    assert!(unsafe { history::pixmith_history_goto(&mut e, 0) });
    assert!(capture(&e) == states[first]);
    assert!(unsafe { history::pixmith_history_goto(&mut e, (undo + redo) as u32) });
    assert!(capture(&e) == states[first + undo + redo]);
}

fn counts(e: &Editor) -> (usize, usize) {
    let (mut u, mut r) = (0, 0);
    unsafe { history::pixmith_history_counts(e, &mut u, &mut r) };
    (u as usize, r as usize)
}

#[test]
fn random_histories_reproduce_every_recorded_state() {
    for seed in 1..=12 {
        run(seed, 160);
    }
}

#[test]
fn trimmed_histories_still_reproduce_the_recorded_states() {
    for seed in 50..=53 {
        let mut rng = Rng(seed | 1);
        let mut e = start();
        e.set_undo_budget(40 * 1024);
        let mut states = vec![capture(&e)];
        for _ in 0..200 {
            step(&mut e, &mut rng);
            states.push(capture(&e));
        }
        let kept = counts(&e).0;
        assert!((1..200).contains(&kept), "kept {kept}");
        let last = states.len() - 1;
        for k in 1..=kept {
            assert!(e.undo());
            assert!(capture(&e) == states[last - k], "seed {seed}: {k} steps back");
        }
        for k in (0..kept).rev() {
            assert!(e.redo());
            assert!(capture(&e) == states[last - k]);
        }
        assert_eq!(e.history_drops(), 0);
    }
}
