//! Undo history stores what changed, not whole cels.

use pixmith_core::document::{Editor, Sprite};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;

const MIB: usize = 1 << 20;

/// CPU time of this thread in milliseconds, which a busy machine does not inflate like wall time.
fn thread_cpu_ms() -> f64 {
    #[repr(C)]
    struct Timespec {
        sec: i64,
        nsec: i64,
    }
    extern "C" {
        fn clock_gettime(clock: i32, ts: *mut Timespec) -> i32;
    }
    #[cfg(target_os = "macos")]
    const THREAD_CPU: i32 = 16;
    #[cfg(not(target_os = "macos"))]
    const THREAD_CPU: i32 = 3;
    let mut ts = Timespec { sec: 0, nsec: 0 };
    unsafe { clock_gettime(THREAD_CPU, &mut ts) };
    ts.sec as f64 * 1000.0 + ts.nsec as f64 / 1e6
}

fn fingerprint(e: &Editor) -> u64 {
    let mut h = DefaultHasher::new();
    e.sprite.render(0).hash(&mut h);
    h.finish()
}

/// One single-pixel stroke: a checkpoint, then the pixel.
fn dot(e: &mut Editor, n: usize, size: usize) {
    e.checkpoint();
    let (x, y) = ((n * 37) % size, (n * 91) % size);
    e.sprite.draw_line(0, 0, x as i32, y as i32, x as i32, y as i32, [(n % 250) as u8 + 1, 40, 80, 255], 1);
}

/// Returns (bytes of older history, total bytes, (median, slowest) undo in ms, slowest checkpoint in ms).
fn run(size: usize, strokes: usize) -> (usize, usize, (f64, f64), f64) {
    let mut s = Sprite::new(size as u32, size as u32);
    s.draw_line(0, 0, 0, 0, 3, 3, [9, 9, 9, 255], 1);
    let mut e = Editor::new(s);
    let mut prints = vec![fingerprint(&e)];
    let mut slowest_checkpoint = 0.0f64;
    for n in 0..strokes {
        let t = thread_cpu_ms();
        dot(&mut e, n, size);
        slowest_checkpoint = slowest_checkpoint.max(thread_cpu_ms() - t);
        prints.push(fingerprint(&e));
    }
    let (beyond, total) = (e.history_bytes_beyond_newest(), e.history_bytes());
    // Undo is exact all the way back, and one step is quick.
    let mut times = Vec::new();
    for k in (0..strokes).rev() {
        let t = thread_cpu_ms();
        assert!(e.undo());
        times.push(thread_cpu_ms() - t);
        assert_eq!(fingerprint(&e), prints[k], "state {k}");
    }
    times.sort_by(|a, b| a.total_cmp(b));
    // (median, slowest): the median is what a user feels; the slowest depends on machine load.
    let slowest = (times[times.len() / 2], *times.last().unwrap());
    for (k, print) in prints.iter().enumerate().skip(1).take(strokes) {
        assert!(e.redo());
        assert_eq!(&fingerprint(&e), print, "redo to state {k}");
    }
    (beyond, total, slowest, slowest_checkpoint)
}

#[test]
fn single_pixel_strokes_cost_bytes_not_cels() {
    let (beyond, total, _, _) = run(512, 30);
    assert!(beyond < 64 * 1024, "older steps hold {beyond} bytes");
    // Plus the newest step, which still holds the 1 MiB cel from before the last stroke.
    assert!(total < MIB + 64 * 1024, "{total}");
}

/// Undo and redo patch the live cel in place instead of copying it: its allocation never changes
/// after the first undo (which swaps in the newest snapshot).
#[test]
fn consecutive_undo_and_redo_patch_the_live_cel_in_place() {
    let mut s = Sprite::new(512, 512);
    s.draw_line(0, 0, 0, 0, 3, 3, [9, 9, 9, 255], 1);
    let mut e = Editor::new(s);
    for n in 0..12 {
        dot(&mut e, n, 512);
    }
    let cel = |e: &Editor| Arc::as_ptr(e.sprite.cels[0][0].as_ref().unwrap());
    // Compaction runs on the history thread; once it has finished, steps patch in place.
    e.wait_for_history();
    assert!(e.undo());
    e.wait_for_history();
    let live = cel(&e);
    for _ in 0..11 {
        assert!(e.undo());
        assert_eq!(cel(&e), live, "undo copied the cel");
    }
    for _ in 0..12 {
        assert!(e.redo());
        assert_eq!(cel(&e), live, "redo copied the cel");
    }
    assert_eq!(e.history_drops(), 0);
}

/// 100 single-pixel strokes on a 4096 x 4096 cel (64 MiB). Run with
/// `cargo test --release --test undo_compaction -- --ignored --nocapture`.
#[test]
#[ignore]
fn benchmark_100_strokes_on_4096_squared() {
    let t = Instant::now();
    let (beyond, total, (median_undo_ms, slowest_undo_ms), slowest_checkpoint_ms) =
        run(4096, std::env::var("PIXMITH_BENCH_STROKES").ok().and_then(|v| v.parse().ok()).unwrap_or(100));
    println!(
        "older history: {:.1} KiB, with the newest step: {:.1} MiB, median undo (CPU): {:.1} ms, slowest undo (CPU): {:.1} ms, slowest checkpoint+stroke (CPU): {:.1} ms, whole run: {:.1} s",
        beyond as f64 / 1024.0,
        total as f64 / MIB as f64,
        median_undo_ms,
        slowest_undo_ms,
        slowest_checkpoint_ms,
        t.elapsed().as_secs_f64()
    );
    assert!(beyond < 5 * MIB);
    assert!(total < 70 * MIB);
    assert!(median_undo_ms < 5.0, "median undo took {median_undo_ms} ms");
}
