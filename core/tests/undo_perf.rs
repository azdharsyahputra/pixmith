//! Wall-clock latency of the editing hot path: checkpoint + edit, undo, redo.
//! Run with `cargo test --release --test undo_perf -- --ignored --nocapture`.

use pixmith_core::document::{Editor, Sprite};
use pixmith_core::paint::selection;
use std::time::Instant;

struct Stats(Vec<f64>);

impl Stats {
    fn report(mut self, what: &str) -> (f64, f64) {
        self.0.sort_by(|a, b| a.total_cmp(b));
        let at = |q: f64| self.0[((self.0.len() - 1) as f64 * q).round() as usize];
        let (median, p95, max) = (at(0.5), at(0.95), *self.0.last().unwrap());
        println!("{what:<44} median {median:>7.3} ms   p95 {p95:>7.3} ms   max {max:>7.3} ms");
        (median, p95)
    }
}

fn time(f: impl FnOnce()) -> f64 {
    let t = Instant::now();
    f();
    t.elapsed().as_secs_f64() * 1000.0
}

/// A short stroke somewhere on the canvas, like a pencil drag.
fn stroke(e: &mut Editor, n: usize, size: i32) {
    let (x, y) = ((n as i32 * 37) % (size - 40), (n as i32 * 91) % (size - 40));
    e.sprite.draw_line(0, 0, x, y, x + 30, y + 17, [(n % 250) as u8 + 1, 40, 80, 255], 2);
}

fn bench(size: u32, steps: usize) {
    println!("--- {size} x {size}, 2 layers, {steps} steps ---");
    let mut s = Sprite::new(size, size);
    s.draw_line(0, 0, 0, 0, size as i32 - 1, size as i32 - 1, [9, 9, 9, 255], 3);
    let top = s.add_layer(0);
    s.draw_line(top, 0, 0, size as i32 - 1, size as i32 - 1, 0, [200, 9, 9, 255], 3);
    let mut e = Editor::new(s);
    // Warm up allocations and caches.
    for n in 0..3 {
        e.checkpoint();
        stroke(&mut e, n, size as i32);
    }
    let mut edit = Stats(Vec::new());
    for n in 0..steps {
        edit.0.push(time(|| {
            e.checkpoint();
            stroke(&mut e, n, size as i32);
        }));
    }
    let mut undo = Stats(Vec::new());
    for _ in 0..steps {
        undo.0.push(time(|| assert!(e.undo())));
    }
    let mut redo = Stats(Vec::new());
    for _ in 0..steps {
        redo.0.push(time(|| assert!(e.redo())));
    }
    // Alternating edit / undo, the "draw, oops, undo" rhythm (first undo after an edit).
    let mut first_undo = Stats(Vec::new());
    for n in 0..steps.min(40) {
        e.checkpoint();
        stroke(&mut e, n + 1000, size as i32);
        first_undo.0.push(time(|| assert!(e.undo())));
    }
    // Selection changes: rectangle selects, then undo them.
    let mut sel = Stats(Vec::new());
    let mut sel_undo = Stats(Vec::new());
    for n in 0..steps.min(40) {
        let a = ((n * 13) as i32 % (size as i32 / 2), (n * 7) as i32 % (size as i32 / 2));
        sel.0.push(time(|| selection::select_rect(&mut e, a, (a.0 + 300, a.1 + 200))));
    }
    for _ in 0..steps.min(40) {
        sel_undo.0.push(time(|| assert!(e.undo())));
    }
    edit.report("checkpoint + stroke");
    undo.report("undo");
    redo.report("redo");
    first_undo.report("first undo right after a stroke");
    sel.report("select rect (undoable)");
    sel_undo.report("undo a selection change");
}

#[test]
#[ignore]
fn editing_latency() {
    bench(512, 60);
    bench(2048, 60);
    bench(4096, 40);
}
