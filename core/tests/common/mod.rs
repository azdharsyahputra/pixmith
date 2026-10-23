//! Helpers shared by the robustness test crates: allocation tracking, a PRNG, sample sprites
//! and consistency checks.
#![allow(dead_code)]

use pixmith_core::document::color_mode::{self, ColorMode};
use pixmith_core::document::tilemap;
use pixmith_core::document::{Sprite, Tag};
use pixmith_core::formats::ase;
use std::alloc::{GlobalAlloc, Layout, System};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

// ---- allocation tracking -----------------------------------------------------------------

pub struct Tracking;

pub static LARGEST: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        LARGEST.fetch_max(l.size(), Ordering::Relaxed);
        System.alloc(l)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        LARGEST.fetch_max(l.size(), Ordering::Relaxed);
        System.alloc_zeroed(l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new_size: usize) -> *mut u8 {
        LARGEST.fetch_max(new_size, Ordering::Relaxed);
        System.realloc(p, l, new_size)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
}

#[global_allocator]
static ALLOC: Tracking = Tracking;

/// Tests share the allocation counter, so they run one at a time.
pub static LOCK: Mutex<()> = Mutex::new(());

pub fn serial() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    LARGEST.store(0, Ordering::Relaxed);
    g
}

/// No single allocation may exceed this: the biggest legitimate buffer is a canvas-sized cel
/// (at most `MAX_CANVAS_PIXELS` * 4 = 256 MiB) plus some slack.
pub const ALLOC_LIMIT: usize = 300 << 20;

pub fn assert_modest_allocations(what: &str) {
    let largest = LARGEST.load(Ordering::Relaxed);
    assert!(largest <= ALLOC_LIMIT, "{what}: allocated {} MiB at once", largest >> 20);
}

// ---- PRNG --------------------------------------------------------------------------------

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() >> 11) as usize % n
    }
}

// ---- sample files ------------------------------------------------------------------------

pub const RED: [u8; 4] = [255, 0, 0, 255];
pub const BLUE: [u8; 4] = [0, 0, 255, 255];

/// Two layers, three frames (one with a linked cel), a tag, a translucent blue blob.
pub fn rgba_sprite() -> Sprite {
    let mut s = Sprite::new(7, 5);
    s.draw_line(0, 0, 0, 0, 6, 4, RED, 1);
    let top = s.add_layer(0);
    s.layers[top].blend_mode = 1;
    s.layers[top].opacity = 160;
    s.fill(top, 0, 3, 3, [0, 0, 255, 120]);
    s.add_frame(0, true); // shares (and so links) its cels
    let f = s.add_frame(1, false);
    s.draw_line(0, f, 6, 0, 0, 4, BLUE, 2);
    s.durations[f] = 300;
    s.tags.push(Tag {
        name: "walk".into(),
        from: 0,
        to: 1,
        direction: 1,
        repeat: 2,
        color: [9, 8, 7],
        user_data: Default::default(),
    });
    s
}

pub fn with_group() -> Sprite {
    let mut s = rgba_sprite();
    let g = s.add_group(0);
    s.layers[g].name = "group".into();
    s
}

pub fn indexed_sprite() -> Sprite {
    let mut s = rgba_sprite();
    color_mode::convert(&mut s, ColorMode::Indexed, 0);
    s
}

pub fn gray_sprite() -> Sprite {
    let mut s = rgba_sprite();
    color_mode::convert(&mut s, ColorMode::Grayscale, 0);
    s
}

pub fn tilemap_sprite() -> Sprite {
    let mut s = Sprite::new(16, 16);
    let l = tilemap::add_tilemap_layer(&mut s, 0, 8, 8);
    let ts = s.layers[l].tileset as usize;
    let id = s.tilesets[ts].add_blank();
    std::sync::Arc::make_mut(&mut s.tilesets[ts].tiles[id as usize])[..8]
        .copy_from_slice(&[255, 0, 0, 255, 0, 255, 0, 255]);
    tilemap::stamp(&mut s, l, 0, 1, 1, id);
    tilemap::stamp(&mut s, l, 0, 9, 9, id | tilemap::FLIP_X);
    s
}

pub fn samples() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("rgba", ase::write(&rgba_sprite())),
        ("group", ase::write(&with_group())),
        ("indexed", ase::write(&indexed_sprite())),
        ("grayscale", ase::write(&gray_sprite())),
        ("tilemap", ase::write(&tilemap_sprite())),
    ]
}

// ---- checks ------------------------------------------------------------------------------

/// A successfully read sprite must be internally consistent so that rendering, saving and the
/// editing entry points can't index out of bounds.
pub fn check_sprite(s: &Sprite) {
    let len = s.width as usize * s.height as usize * 4;
    assert!(s.width > 0 && s.height > 0);
    assert_eq!(s.cels.len(), s.durations.len());
    assert!(!s.layers.is_empty());
    for row in &s.cels {
        assert_eq!(row.len(), s.layers.len());
        for cel in row.iter().flatten() {
            assert_eq!(cel.pixels.len(), len);
        }
    }
    if len > 4 << 20 {
        // Mutations can legitimately produce huge canvases; don't spend minutes on them.
        assert_eq!(s.render(0).len(), len);
        return;
    }
    for f in 0..s.frame_count() {
        assert_eq!(s.render(f).len(), len);
    }
    let out = ase::write(s);
    ase::read(&out).expect("a sprite we wrote must read back");
}

/// Reads `bytes`, turning a panic into an error message.
pub fn try_read(bytes: &[u8]) -> Result<Result<Sprite, String>, String> {
    catch_unwind(AssertUnwindSafe(|| ase::read(bytes))).map_err(|p| {
        p.downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default()
    })
}

pub fn read_and_check(bytes: &[u8], what: &str, failures: &mut Vec<String>) {
    match try_read(bytes) {
        Err(msg) => failures.push(format!("{what}: read panicked: {msg}")),
        Ok(Ok(s)) => {
            if let Err(p) = catch_unwind(AssertUnwindSafe(|| check_sprite(&s))) {
                let msg =
                    p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
                failures.push(format!("{what}: accepted but inconsistent: {msg:?}"));
            }
        }
        Ok(Err(_)) => {}
    }
}

pub fn report(failures: Vec<String>) {
    let mut unique = failures.clone();
    unique.sort();
    unique.dedup_by(|a, b| a.split_once(": ").map(|x| x.1) == b.split_once(": ").map(|x| x.1));
    assert!(
        failures.is_empty(),
        "{} failures, e.g.:\n{}",
        failures.len(),
        unique.iter().take(12).cloned().collect::<Vec<_>>().join("\n")
    );
}
