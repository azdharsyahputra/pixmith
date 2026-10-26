//! A small synthetic "pseudo-pixel" image for the in-crate tests (FFI, import, Lua): a pixel figure upscaled by a
//! fractional pitch with a phase offset, softened, noisy, on white paper with a grey shadow.

pub const NX: usize = 16;
pub const NY: usize = 16;
pub const PITCH: f64 = 8.3;
pub const ORIGIN: (f64, f64) = (14.4, 9.7);

pub const OUTLINE: [u8; 4] = [30, 28, 40, 255];
pub const BODY: [u8; 4] = [200, 60, 70, 255];
pub const BELLY: [u8; 4] = [240, 190, 140, 255];
pub const EYE: [u8; 4] = [255, 255, 255, 255];

/// The original sprite, row-major; alpha 0 outside the figure. It touches all four edges.
pub fn sprite() -> Vec<[u8; 4]> {
    let c = NX as f64 / 2.0;
    let inside = |x: i64, y: i64| {
        x >= 0 && y >= 0 && x < NX as i64 && y < NY as i64 && {
            let (dx, dy) = ((x as f64 + 0.5 - c) / c, (y as f64 + 0.5 - c) / c);
            dx * dx + dy * dy <= 1.0
        }
    };
    let mut px = vec![[0u8; 4]; NX * NY];
    for y in 0..NY {
        for x in 0..NX {
            if inside(x as i64, y as i64) {
                let edge =
                    [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dy)| !inside(x as i64 + dx, y as i64 + dy));
                px[y * NX + x] = if edge {
                    OUTLINE
                } else if y > NY / 2 {
                    BELLY
                } else {
                    BODY
                };
            }
        }
    }
    for (x, y) in [(5, 5), (6, 5), (9, 5), (10, 5), (5, 6), (10, 6)] {
        px[y * NX + x] = EYE;
    }
    px
}

/// `(width, height, rgba)` of the rendered image.
pub fn image() -> (usize, usize, Vec<u8>) {
    let s = sprite();
    let w = (ORIGIN.0 + NX as f64 * PITCH + 2.5 * PITCH).ceil() as usize;
    let h = (ORIGIN.1 + NY as f64 * PITCH + 3.5 * PITCH).ceil() as usize;
    let (scx, scy) = (ORIGIN.0 + NX as f64 * PITCH / 2.0, ORIGIN.1 + NY as f64 * PITCH + 0.3 * PITCH);
    let mut img = vec![[255.0f64; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
            let r2 = ((fx - scx) / (NX as f64 * PITCH * 0.4)).powi(2) + ((fy - scy) / (1.6 * PITCH)).powi(2);
            let mut c = if r2 < 1.0 { [150.0 + 105.0 * r2; 3] } else { [255.0; 3] };
            let (cx, cy) = (((fx - ORIGIN.0) / PITCH).floor(), ((fy - ORIGIN.1) / PITCH).floor());
            if cx >= 0.0 && cy >= 0.0 && (cx as usize) < NX && (cy as usize) < NY {
                let p = s[cy as usize * NX + cx as usize];
                if p[3] != 0 {
                    c = [p[0] as f64, p[1] as f64, p[2] as f64];
                }
            }
            img[y * w + x] = c;
        }
    }
    // Soften with a [1 2 1] filter in both directions, then add deterministic noise.
    let blur = |src: &[[f64; 3]], horizontal: bool| -> Vec<[f64; 3]> {
        let mut out = src.to_vec();
        for y in 0..h {
            for x in 0..w {
                let at = |d: i64| {
                    let (sx, sy) = if horizontal {
                        ((x as i64 + d).clamp(0, w as i64 - 1), y as i64)
                    } else {
                        (x as i64, (y as i64 + d).clamp(0, h as i64 - 1))
                    };
                    src[sy as usize * w + sx as usize]
                };
                let (a, b, c) = (at(-1), at(0), at(1));
                out[y * w + x] = [0, 1, 2].map(|i| (a[i] + 2.0 * b[i] + c[i]) / 4.0);
            }
        }
        out
    };
    let img = blur(&blur(&img, true), false);
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for p in img {
        for c in p {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let n = (seed % 13) as f64 - 6.0;
            rgba.push((c + n).round().clamp(0.0, 255.0) as u8);
        }
        rgba.push(255);
    }
    (w, h, rgba)
}

/// Share of the original sprite's pixels found in a traced `NX` x `NY` image.
pub fn match_share(rgba: &[u8]) -> f64 {
    let s = sprite();
    let ok = s
        .iter()
        .zip(rgba.chunks_exact(4))
        .filter(|(want, got)| {
            if want[3] == 0 {
                got[3] == 0
            } else {
                got[3] != 0 && (0..3).map(|i| (want[i] as f64 - got[i] as f64).powi(2)).sum::<f64>().sqrt() <= 30.0
            }
        })
        .count();
    ok as f64 / s.len() as f64
}
