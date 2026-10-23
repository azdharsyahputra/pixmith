//! Render timing benchmark. It is ignored by default; run it with
//! `cargo test --release --test render_bench -- --ignored --nocapture`.

// RGBA pixels are walked four bytes at a time with `chunks_exact(4)`, the same idiom as in the library.
#![allow(clippy::chunks_exact_to_as_chunks)]

use pixmith_core::document::layers::blend_pixel;
use pixmith_core::document::{Cel, Layer, Sprite};
use std::sync::Arc;
use std::time::Instant;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 32) as u32
    }
}

/// A cel with a random opaque, half-transparent or gradient blob on a transparent background.
fn blob_cel(rng: &mut Rng, w: usize, h: usize, kind: u32) -> Arc<Cel> {
    let mut px = vec![0u8; w * h * 4];
    let (bx, by) = (rng.next() as usize % (w / 2), rng.next() as usize % (h / 2));
    let (bw, bh) = (w / 4 + rng.next() as usize % (w / 4), h / 4 + rng.next() as usize % (h / 4));
    for y in by..(by + bh).min(h) {
        for x in bx..(bx + bw).min(w) {
            let i = (y * w + x) * 4;
            let a = match kind {
                0 => 255,
                1 => 128,
                _ => (x * 255 / w) as u8,
            };
            px[i..i + 4].copy_from_slice(&[(x * 7) as u8, (y * 5) as u8, rng.next() as u8, a]);
        }
    }
    Arc::new(Cel { opacity: 255, pixels: px, ..Default::default() })
}

/// 512x512, 20 layers (mixed blend modes and opacities), 50 frames sharing 4 cels per layer.
fn big_sprite() -> Sprite {
    let (w, h) = (512usize, 512usize);
    let mut s = Sprite::new(w as u32, h as u32);
    let mut rng = Rng(0x9E3779B97F4A7C15);
    s.layers.clear();
    let modes = [0u16, 0, 0, 1, 2, 0, 3, 4, 0, 5, 0, 6, 0, 10, 0, 12, 0, 14, 0, 18];
    for (i, &mode) in modes.iter().enumerate() {
        let mut l = Layer::new(format!("L{i}"));
        l.blend_mode = mode;
        l.opacity = if i % 3 == 1 { 180 } else { 255 };
        s.layers.push(l);
    }
    s.durations = vec![100; 50];
    let variants: Vec<Vec<Arc<Cel>>> = (0..20)
        .map(|l| (0..4).map(|v| blob_cel(&mut rng, w, h, if l == 0 { 0 } else { (l + v) as u32 % 3 })).collect())
        .collect();
    let mut full = vec![255u8; w * h * 4];
    for p in full.chunks_exact_mut(4) {
        p[0] = 40;
        p[1] = 60;
        p[2] = 90;
    }
    let background = Arc::new(Cel { opacity: 255, pixels: full, ..Default::default() });
    s.cels = (0..50)
        .map(|f| {
            (0..20)
                .map(|l| {
                    if l == 0 {
                        Some(background.clone())
                    } else if (f + l) % 7 == 3 {
                        None
                    } else {
                        Some(variants[l][f % 4].clone())
                    }
                })
                .collect()
        })
        .collect();
    s
}

/// The straightforward per-pixel compositor `Sprite::render` replaced; the optimized version
/// must produce identical bytes.
fn reference_render(s: &Sprite, frame: usize) -> Vec<u8> {
    let mut out = vec![0u8; s.width as usize * s.height as usize * 4];
    for (li, layer) in s.layers.iter().enumerate() {
        if layer.is_group() || !s.is_layer_shown(li) {
            continue;
        }
        let Some(cel) = &s.cels[frame][li] else { continue };
        let opacity = s.effective_opacity(li) * cel.opacity as u32 / 255;
        for (d, px) in out.chunks_exact_mut(4).zip(cel.pixels.chunks_exact(4)) {
            blend_pixel(d, px, opacity, layer.blend_mode);
        }
    }
    out
}

fn best_of(rounds: usize, mut f: impl FnMut()) -> std::time::Duration {
    // Best of several rounds, since other processes make single timings noisy.
    (0..rounds)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .min()
        .unwrap()
}

#[test]
#[ignore]
fn render_big_sprite() {
    let s = big_sprite();
    for f in [0, 7, 49] {
        assert!(s.render(f) == reference_render(&s, f), "frame {f} differs");
    }
    let new = best_of(7, || (0..10).for_each(|f| drop(std::hint::black_box(s.render(f))))) / 10;
    let old = best_of(7, || (0..10).for_each(|f| drop(std::hint::black_box(reference_render(&s, f))))) / 10;
    println!("render 512x512, 20 layers, mixed blend modes, per frame: reference {old:?}, optimized {new:?}");
}

#[test]
fn optimized_render_matches_reference() {
    // Small sprite with every blend mode, opacities and partial transparency.
    let mut s = Sprite::new(37, 29);
    let mut rng = Rng(12345);
    s.layers.clear();
    s.cels = vec![vec![]];
    for mode in 0..=19u16 {
        let mut l = Layer::new("l");
        l.blend_mode = mode;
        l.opacity = [255, 255, 200, 90, 0][mode as usize % 5];
        s.layers.push(l);
        let mut px = vec![0u8; 37 * 29 * 4];
        for p in px.chunks_exact_mut(4) {
            let r = rng.next();
            let a = match r % 5 {
                0 => 0,
                1 => 255,
                2 => 255,
                _ => (r >> 8) as u8,
            };
            p.copy_from_slice(&[(r >> 16) as u8, (r >> 24) as u8, (r >> 3) as u8, a]);
        }
        let opacity = if mode % 4 == 3 { 128 } else { 255 };
        s.cels[0].push(Some(Arc::new(Cel { opacity, pixels: px, ..Default::default() })));
    }
    assert_eq!(s.render(0), reference_render(&s, 0));
}
