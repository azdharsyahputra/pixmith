//! Trace-to-grid and background removal on synthetic "AI pseudo-pixel" art: a small sprite is upscaled by a
//! non-integer factor with a phase offset, then given soft edges, JPEG-ish noise, a white paper background and a
//! grey drop shadow under the feet. Every fixture is generated here; nothing touches the disk, HOME or the app's
//! support directory.
#![allow(clippy::chunks_exact_to_as_chunks)]

use pixmith_core::paint::background::{detect_background, remove_background, BgOptions};
use pixmith_core::paint::trace::{close_outline, detect_grid, trace_grid, Colors, Pitch, TraceOptions};

// MARK: Fixture

const OUTLINE: [u8; 4] = [30, 28, 40, 255];
const HAIR: [u8; 4] = [60, 90, 200, 255];
const SKIN: [u8; 4] = [235, 175, 125, 255];
const SHIRT: [u8; 4] = [200, 50, 60, 255];
const PANTS: [u8; 4] = [60, 150, 80, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const GREY: [u8; 4] = [190, 190, 196, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];
const PAPER: [f64; 3] = [255.0, 255.0, 255.0];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Uniform in -1..1.
    fn signed(&mut self) -> f64 {
        (self.next() % 20001) as f64 / 10000.0 - 1.0
    }
}

/// The original pixel sprite: an outlined round figure that touches all four edges of its `nx` x `ny` canvas, with
/// white eyes and a white and a grey highlight completely inside the outline.
struct Sprite {
    nx: usize,
    ny: usize,
    px: Vec<[u8; 4]>,
}

impl Sprite {
    fn new(nx: usize, ny: usize) -> Sprite {
        let (cx, cy) = (nx as f64 / 2.0, ny as f64 / 2.0);
        let inside = |x: i64, y: i64| -> bool {
            if x < 0 || y < 0 || x >= nx as i64 || y >= ny as i64 {
                return false;
            }
            let dx = (x as f64 + 0.5 - cx) / cx;
            let dy = (y as f64 + 0.5 - cy) / cy;
            dx * dx + dy * dy <= 1.0
        };
        let mut px = vec![CLEAR; nx * ny];
        for y in 0..ny {
            for x in 0..nx {
                if !inside(x as i64, y as i64) {
                    continue;
                }
                let edge =
                    [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dy)| !inside(x as i64 + dx, y as i64 + dy));
                px[y * nx + x] = if edge {
                    OUTLINE
                } else if y < ny / 4 {
                    HAIR
                } else if y < ny / 2 {
                    SKIN
                } else if y < ny * 3 / 4 {
                    SHIRT
                } else {
                    PANTS
                };
            }
        }
        let mut set = |x: usize, y: usize, c: [u8; 4]| px[y * nx + x] = c;
        // Eyes: white with a dark pupil, on the skin rows.
        let ey = ny / 4 + 2;
        for (ex, pupil) in [(nx / 2 - 6, 1usize), (nx / 2 + 4, 0usize)] {
            for dy in 0..2 {
                for dx in 0..2 {
                    set(ex + dx, ey + dy, WHITE);
                }
            }
            set(ex + pupil, ey + 1, OUTLINE);
        }
        // A white highlight on the hair and a light grey patch on the shirt.
        set(nx / 2 - 3, 4, WHITE);
        set(nx / 2 - 2, 4, WHITE);
        for dy in 0..2 {
            for dx in 0..3 {
                set(nx / 2 - 1 + dx, ny / 2 + 3 + dy, GREY);
            }
        }
        Sprite { nx, ny, px }
    }

    fn at(&self, x: usize, y: usize) -> [u8; 4] {
        self.px[y * self.nx + x]
    }
}

struct Render {
    w: usize,
    h: usize,
    rgba: Vec<u8>,
}

struct Look {
    pitch: (f64, f64),
    /// Image coordinates of the sprite's left / top edge.
    origin: (f64, f64),
    sigma: f64,
    noise: f64,
    shadow: bool,
    /// 1 keeps the colours; less pulls them toward grey (AI art with washed-out edges).
    contrast: f64,
}

fn render(s: &Sprite, look: &Look) -> Render {
    let (pw, ph) = look.pitch;
    let (ox, oy) = look.origin;
    let w = (ox + s.nx as f64 * pw + 3.2 * pw).ceil() as usize;
    let h = (oy + s.ny as f64 * ph + 4.4 * ph).ceil() as usize;
    // Shadow: a soft ellipse under the feet, centred below the sprite's bottom edge.
    let (scx, scy) = (ox + s.nx as f64 * pw / 2.0, oy + s.ny as f64 * ph + 0.4 * ph);
    let (srx, sry) = (s.nx as f64 * pw * 0.42, 1.8 * ph);
    let mut img = vec![[0.0f64; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
            let (cx, cy) = (((fx - ox) / pw).floor(), ((fy - oy) / ph).floor());
            let mut c = PAPER;
            if look.shadow {
                let r2 = ((fx - scx) / srx).powi(2) + ((fy - scy) / sry).powi(2);
                if r2 < 1.0 {
                    let dark = 0.42 * (1.0 - r2).powf(0.6);
                    c = [255.0 * (1.0 - dark), 252.0 * (1.0 - dark), 250.0 * (1.0 - dark)];
                }
            }
            if cx >= 0.0 && cy >= 0.0 && (cx as usize) < s.nx && (cy as usize) < s.ny {
                let p = s.at(cx as usize, cy as usize);
                if p[3] != 0 {
                    c = [p[0] as f64, p[1] as f64, p[2] as f64];
                }
            }
            img[y * w + x] = c;
        }
    }
    // Soft edges: a separable Gaussian blur.
    if look.sigma > 0.0 {
        let r = (look.sigma * 3.0).ceil() as i64;
        let k: Vec<f64> = (-r..=r).map(|i| (-(i * i) as f64 / (2.0 * look.sigma * look.sigma)).exp()).collect();
        let sum: f64 = k.iter().sum();
        let blur = |src: &[[f64; 3]], horizontal: bool| -> Vec<[f64; 3]> {
            let mut out = vec![[0.0; 3]; w * h];
            for y in 0..h as i64 {
                for x in 0..w as i64 {
                    let mut acc = [0.0; 3];
                    for (i, kv) in k.iter().enumerate() {
                        let d = i as i64 - r;
                        let (sx, sy) = if horizontal {
                            ((x + d).clamp(0, w as i64 - 1), y)
                        } else {
                            (x, (y + d).clamp(0, h as i64 - 1))
                        };
                        let p = src[sy as usize * w + sx as usize];
                        for c in 0..3 {
                            acc[c] += p[c] * kv;
                        }
                    }
                    out[y as usize * w + x as usize] = [acc[0] / sum, acc[1] / sum, acc[2] / sum];
                }
            }
            out
        };
        img = blur(&blur(&img, true), false);
    }
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for p in img {
        for c in p {
            rgba.push((128.0 + (c - 128.0) * look.contrast + rng.signed() * look.noise).round().clamp(0.0, 255.0) as u8);
        }
        rgba.push(255);
    }
    Render { w, h, rgba }
}

fn default_look() -> Look {
    Look { pitch: (7.3, 7.3), origin: (25.6, 21.1), sigma: 0.9, noise: 6.0, shadow: true, contrast: 1.0 }
}

fn dist(a: [u8; 4], b: [u8; 4]) -> f64 {
    (0..3).map(|i| (a[i] as f64 - b[i] as f64).powi(2)).sum::<f64>().sqrt()
}

/// Share of original pixels recovered (same opacity and a colour within `tol`).
fn match_share(s: &Sprite, out: &[u8], ow: usize, oh: usize, tol: f64) -> f64 {
    assert_eq!((ow, oh), (s.nx, s.ny), "the traced sprite must have the original size");
    let mut ok = 0;
    for y in 0..s.ny {
        for x in 0..s.nx {
            let want = s.at(x, y);
            let got: [u8; 4] = out[(y * ow + x) * 4..][..4].try_into().unwrap();
            if (want[3] == 0 && got[3] == 0) || (want[3] != 0 && got[3] != 0 && dist(want, got) <= tol) {
                ok += 1;
            }
        }
    }
    ok as f64 / (s.nx * s.ny) as f64
}

fn px(rgba: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
    rgba[(y * w + x) * 4..][..4].try_into().unwrap()
}

fn circ_diff(a: f64, b: f64, period: f64) -> f64 {
    let d = (a - b).rem_euclid(period);
    d.min(period - d)
}

fn cleaning() -> TraceOptions {
    TraceOptions {
        pitch: Pitch::Auto { min: 5.0, max: 14.0 },
        colors: Colors::Auto { min: 2, max: 16 },
        remove_background: Some(BgOptions { shadow: true, ..BgOptions::default() }),
        ..TraceOptions::default()
    }
}

// MARK: trace_grid

#[test]
fn recovers_pitch_phase_size_and_pixels() {
    let s = Sprite::new(30, 32);
    let look = default_look();
    let r = render(&s, &look);
    let t = trace_grid(&r.rgba, r.w, r.h, &cleaning()).unwrap();
    assert!((t.pitch.0 - 7.3).abs() <= 0.1 && (t.pitch.1 - 7.3).abs() <= 0.1, "pitch {:?}", t.pitch);
    // The first full cell starts at origin mod pitch.
    let want = (look.origin.0.rem_euclid(7.3), look.origin.1.rem_euclid(7.3));
    assert!(circ_diff(t.phase.0, want.0, 7.3) <= 0.5, "x phase {} vs {}", t.phase.0, want.0);
    assert!(circ_diff(t.phase.1, want.1, 7.3) <= 0.5, "y phase {} vs {}", t.phase.1, want.1);
    assert_eq!((t.width, t.height), (30, 32), "cropped to the original sprite size");
    let share = match_share(&s, &t.rgba, t.width, t.height, 30.0);
    assert!(share >= 0.95, "only {:.1}% of the original pixels were recovered", share * 100.0);
    assert!(t.quality > 0.7, "quality {}", t.quality);
    assert!(t.palette.len() >= 7 && t.palette.len() <= 12, "palette of {}", t.palette.len());
}

#[test]
fn recovers_other_pitches_and_phases() {
    for (pitch, origin) in [((9.7, 9.7), (3.3, 40.8)), ((6.5, 6.5), (11.9, 2.2)), ((11.4, 11.4), (0.4, 5.5))] {
        let s = Sprite::new(26, 24);
        let r = render(&s, &Look { pitch, origin, ..default_look() });
        let opts = TraceOptions { pitch: Pitch::Auto { min: 5.0, max: 14.0 }, ..cleaning() };
        let t = trace_grid(&r.rgba, r.w, r.h, &opts).unwrap();
        assert!((t.pitch.0 - pitch.0).abs() <= 0.1, "pitch {:?} for {pitch:?}", t.pitch);
        assert_eq!((t.width, t.height), (26, 24), "size for pitch {pitch:?}");
        let share = match_share(&s, &t.rgba, t.width, t.height, 30.0);
        assert!(share >= 0.95, "pitch {pitch:?}: {:.1}%", share * 100.0);
    }
}

#[test]
fn survives_heavier_blur_and_noise() {
    let s = Sprite::new(30, 32);
    let r = render(&s, &Look { sigma: 1.3, noise: 14.0, pitch: (8.4, 8.4), origin: (7.7, 13.3), ..default_look() });
    let t = trace_grid(&r.rgba, r.w, r.h, &cleaning()).unwrap();
    assert!((t.pitch.0 - 8.4).abs() <= 0.1, "pitch {:?}", t.pitch);
    assert_eq!((t.width, t.height), (30, 32));
    let share = match_share(&s, &t.rgba, t.width, t.height, 40.0);
    assert!(share >= 0.95, "{:.1}%", share * 100.0);
}

/// Soft AI-like sheets: small non-integer pitch, heavy blur, low contrast and noise. The whole-image fold is weak
/// here, the tile search has to carry the detection.
#[test]
fn finds_the_pitch_of_soft_low_contrast_edges() {
    let auto = |min: f64, max: f64| TraceOptions { pitch: Pitch::Auto { min, max }, ..cleaning() };
    for (pitch, origin, sigma, noise, contrast) in
        [(4.0, (5.3, 9.6), 1.1, 7.0, 0.5), (6.4, (13.7, 2.9), 1.5, 8.0, 0.45), (4.0, (1.1, 3.7), 1.3, 10.0, 0.4)]
    {
        let s = Sprite::new(40, 40);
        let r = render(&s, &Look { pitch: (pitch, pitch), origin, sigma, noise, contrast, shadow: false });
        let g = detect_grid(&r.rgba, r.w, r.h, &Pitch::Auto { min: 3.0, max: 32.0 }, false).unwrap();
        assert!((g.pitch.0 - pitch).abs() <= 0.1, "pitch {:?} for {pitch}", g.pitch);
        let t = trace_grid(&r.rgba, r.w, r.h, &auto(3.0, 32.0)).unwrap();
        assert!((t.pitch.0 - pitch).abs() <= 0.1, "traced pitch {:?} for {pitch}", t.pitch);
        assert!(t.quality > 0.5, "quality {} for {pitch}", t.quality);
        assert!(
            t.width.abs_diff(40) <= 3 && t.height.abs_diff(40) <= 3,
            "size {}x{} for pitch {pitch}",
            t.width,
            t.height
        );
    }
}

#[test]
fn a_noise_image_is_too_weak_not_a_grid() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let (w, h) = (200, 200);
    let mut px = Vec::with_capacity(w * h * 4);
    for _ in 0..w * h {
        let v = (128.0 + rng.signed() * 60.0) as u8;
        px.extend_from_slice(&[v, v, v, 255]);
    }
    let e = detect_grid(&px, w, h, &Pitch::Auto { min: 3.0, max: 32.0 }, false).unwrap_err();
    assert!(e.contains("too weak") && e.contains("pass pitch"), "{e}");
    let flat = vec![200u8; 64 * 64 * 4];
    let e = detect_grid(&flat, 64, 64, &Pitch::Auto { min: 3.0, max: 32.0 }, false).unwrap_err();
    assert!(e.contains("no edges"), "{e}");
}

/// Reads the real AI sheets when they exist (`cargo test -p pixmith-core --test trace real_ai -- --ignored --nocapture`).
#[test]
#[ignore]
fn real_ai_sheets_report_their_pitch() {
    let root = "/Users/csadeveloper/inn/inn-assets/reference";
    for (file, want) in
        [("mc_v4/walk_ref/ref_walk_S.png", 4.0), ("mc_v4/ref_sheet.png", 4.0), ("moosling_v3/ref_sheet.png", 6.4)]
    {
        let path = format!("{root}/{file}");
        let Ok(f) = std::fs::File::open(&path) else { continue };
        let mut dec = png::Decoder::new(f);
        dec.set_transformations(png::Transformations::EXPAND);
        let mut rd = dec.read_info().unwrap();
        let mut buf = vec![0; rd.output_buffer_size()];
        let info = rd.next_frame(&mut buf).unwrap();
        let (w, h) = (info.width as usize, info.height as usize);
        let rgba: Vec<u8> = match info.color_type {
            png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
            png::ColorType::Rgb => buf[..w * h * 3].chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
            other => panic!("{other:?}"),
        };
        let opts = TraceOptions { colors: Colors::Auto { min: 2, max: 32 }, ..TraceOptions::default() };
        let t = trace_grid(&rgba, w, h, &opts).unwrap();
        println!(
            "{file}: pitch {:.3} x {:.3} (expected {want}), quality {:.3}, warning {:?}",
            t.pitch.0, t.pitch.1, t.quality, t.warning
        );
    }
}

#[test]
fn separate_axes_follow_a_stretched_grid() {
    let s = Sprite::new(28, 28);
    let r = render(&s, &Look { pitch: (7.3, 8.1), origin: (9.0, 14.0), ..default_look() });
    let opts = TraceOptions { separate_axes: true, ..cleaning() };
    let t = trace_grid(&r.rgba, r.w, r.h, &opts).unwrap();
    assert!((t.pitch.0 - 7.3).abs() <= 0.1 && (t.pitch.1 - 8.1).abs() <= 0.1, "pitch {:?}", t.pitch);
    assert_eq!((t.width, t.height), (28, 28));
    assert!(match_share(&s, &t.rgba, t.width, t.height, 30.0) >= 0.95);
}

#[test]
fn background_and_shadow_go_but_white_and_grey_inside_the_figure_stay() {
    let s = Sprite::new(30, 32);
    let r = render(&s, &default_look());
    let t = trace_grid(&r.rgba, r.w, r.h, &cleaning()).unwrap();
    assert!(t.background.removed() > 0 && t.background.shadow > 0, "{:?}", t.background);
    // Everything the original left transparent is transparent again, shadow included.
    let (mut clear_want, mut clear_ok) = (0, 0);
    for y in 0..s.ny {
        for x in 0..s.nx {
            if s.at(x, y)[3] == 0 {
                clear_want += 1;
                clear_ok += (px(&t.rgba, t.width, x, y)[3] == 0) as usize;
            }
        }
    }
    assert!(clear_ok as f64 >= clear_want as f64 * 0.98, "{clear_ok} of {clear_want} empty pixels are empty");
    // Eyes (white) and the grey patch are opaque and keep their colour.
    let mut details = 0;
    for y in 0..s.ny {
        for x in 0..s.nx {
            let want = s.at(x, y);
            if want == WHITE || want == GREY {
                let got = px(&t.rgba, t.width, x, y);
                assert!(got[3] == 255 && dist(want, got) <= 30.0, "detail at {x},{y}: {want:?} became {got:?}");
                details += 1;
            }
        }
    }
    assert!(details >= 12, "the fixture has {details} detail pixels");
}

#[test]
fn without_background_removal_the_paper_stays_a_colour() {
    let s = Sprite::new(30, 32);
    let r = render(&s, &default_look());
    let opts = TraceOptions { remove_background: None, crop: false, ..cleaning() };
    let t = trace_grid(&r.rgba, r.w, r.h, &opts).unwrap();
    assert!(t.width > 30 && t.height > 32, "no crop without a transparent background");
    assert!(t.rgba.chunks_exact(4).all(|p| p[3] == 255));
    assert_eq!(t.background.removed(), 0);
    // The top-left corner is paper-white.
    assert!(dist(px(&t.rgba, t.width, 0, 0), WHITE) < 12.0);
}

#[test]
fn trace_is_deterministic() {
    let s = Sprite::new(30, 32);
    let r = render(&s, &default_look());
    let a = trace_grid(&r.rgba, r.w, r.h, &cleaning()).unwrap();
    let b = trace_grid(&r.rgba, r.w, r.h, &cleaning()).unwrap();
    assert_eq!(a.rgba, b.rgba);
    assert_eq!(a.palette, b.palette);
    assert_eq!(a.pitch, b.pitch);
}

#[test]
fn fixed_pitch_and_phase_skip_the_search() {
    let s = Sprite::new(30, 32);
    let look = default_look();
    let r = render(&s, &look);
    let opts = TraceOptions {
        pitch: Pitch::Fixed(7.3, 7.3),
        phase: Some((look.origin.0.rem_euclid(7.3), look.origin.1.rem_euclid(7.3))),
        ..cleaning()
    };
    let t = trace_grid(&r.rgba, r.w, r.h, &opts).unwrap();
    assert_eq!(t.pitch, (7.3, 7.3));
    assert_eq!((t.width, t.height), (30, 32));
    assert!(match_share(&s, &t.rgba, t.width, t.height, 30.0) >= 0.97);
}

#[test]
fn a_wrong_grid_scores_worse() {
    let s = Sprite::new(30, 32);
    let r = render(&s, &default_look());
    let good = trace_grid(&r.rgba, r.w, r.h, &cleaning()).unwrap();
    let off = TraceOptions { pitch: Pitch::Fixed(6.7, 6.7), phase: Some((1.0, 1.0)), ..cleaning() };
    let bad = trace_grid(&r.rgba, r.w, r.h, &off).unwrap();
    assert!(good.quality > bad.quality + 0.05, "{} vs {}", good.quality, bad.quality);
}

#[test]
fn color_count_is_respected_and_an_existing_palette_is_used() {
    let s = Sprite::new(30, 32);
    let r = render(&s, &default_look());
    let opts = TraceOptions { colors: Colors::Count(4), ..cleaning() };
    let t = trace_grid(&r.rgba, r.w, r.h, &opts).unwrap();
    assert!(t.palette.len() <= 4 && !t.palette.is_empty(), "{:?}", t.palette);
    let used: std::collections::BTreeSet<[u8; 4]> =
        t.rgba.chunks_exact(4).filter(|p| p[3] != 0).map(|p| [p[0], p[1], p[2], p[3]]).collect();
    assert!(used.iter().all(|c| t.palette.contains(c)), "every cell is snapped to the palette");

    let own = vec![OUTLINE, HAIR, SKIN, SHIRT, PANTS, WHITE, GREY];
    let opts = TraceOptions { colors: Colors::Palette(own.clone()), ..cleaning() };
    let t = trace_grid(&r.rgba, r.w, r.h, &opts).unwrap();
    assert_eq!(t.palette, own);
    // With the exact palette the colours come back exactly.
    assert!(match_share(&s, &t.rgba, t.width, t.height, 0.5) >= 0.95);
}

#[test]
fn a_flat_image_has_no_grid() {
    let flat = vec![200u8; 64 * 64 * 4];
    assert!(detect_grid(&flat, 64, 64, &Pitch::Auto { min: 4.0, max: 12.0 }, false).is_err());
    assert!(trace_grid(&flat, 64, 64, &TraceOptions::default()).is_err());
    assert!(trace_grid(&flat, 0, 64, &TraceOptions::default()).is_err());
    assert!(trace_grid(&flat[..10], 64, 64, &TraceOptions::default()).is_err());
    // Nonsense settings are errors, never hangs.
    let r = render(&Sprite::new(20, 20), &default_look());
    for bad in [
        TraceOptions { phase: Some((f64::NAN, 0.0)), ..TraceOptions::default() },
        TraceOptions { pitch: Pitch::Fixed(f64::NAN, 5.0), ..TraceOptions::default() },
        TraceOptions { pitch: Pitch::Fixed(f64::INFINITY, 5.0), ..TraceOptions::default() },
        TraceOptions { pitch: Pitch::Fixed(0.0, 5.0), ..TraceOptions::default() },
        TraceOptions { pitch: Pitch::Auto { min: 9.0, max: 4.0 }, ..TraceOptions::default() },
        TraceOptions { colors: Colors::Count(0), ..TraceOptions::default() },
        TraceOptions { colors: Colors::Palette(Vec::new()), ..TraceOptions::default() },
    ] {
        assert!(trace_grid(&r.rgba, r.w, r.h, &bad).is_err(), "{bad:?}");
    }
    // A pitch bigger than the image leaves fewer than two cells.
    let big = TraceOptions { pitch: Pitch::Fixed(4000.0, 4000.0), ..TraceOptions::default() };
    assert!(trace_grid(&r.rgba, r.w, r.h, &big).is_err());
}

// MARK: close_outline

fn grid(rows: &[&str]) -> (Vec<u8>, usize, usize) {
    let h = rows.len();
    let w = rows[0].len();
    let mut px = Vec::new();
    for row in rows {
        for ch in row.chars() {
            px.extend_from_slice(&match ch {
                '#' => OUTLINE,
                'o' => SKIN,
                _ => CLEAR,
            });
        }
    }
    (px, w, h)
}

fn text(px: &[u8], w: usize) -> Vec<String> {
    px.chunks_exact(w * 4)
        .map(|row| {
            row.chunks_exact(4)
                .map(|p| {
                    if p[3] == 0 {
                        '.'
                    } else if p[0] < 100 {
                        '#'
                    } else {
                        'o'
                    }
                })
                .collect()
        })
        .collect()
}

#[test]
fn close_outline_fills_a_one_pixel_break_in_a_straight_run() {
    let (mut px, w, _) = grid(&[".......", ".#####.", ".#ooo#.", ".#ooo#.", ".##.##.", "......."]);
    let filled = close_outline(&mut px, w, 6);
    assert_eq!(filled, 1);
    assert_eq!(text(&px, w), vec![".......", ".#####.", ".#ooo#.", ".#ooo#.", ".#####.", "......."]);
}

#[test]
fn close_outline_leaves_real_gaps_and_open_notches_alone() {
    // A one-pixel gap between two legs is not a break: nothing but background lies on either side.
    let (mut px, w, h) = grid(&[".......", ".##.##.", ".##.##.", ".##.##.", "......."]);
    let before = px.clone();
    assert_eq!(close_outline(&mut px, w, h), 0);
    assert_eq!(px, before);
    // No dark outline at all: nothing to close.
    let (mut px, w, h) = grid(&["....", ".oo.", ".o..", "...."]);
    let before = px.clone();
    assert_eq!(close_outline(&mut px, w, h), 0);
    assert_eq!(px, before);
}

#[test]
fn close_outline_closes_a_diagonal_break() {
    // A diamond whose upper-left edge misses one pixel at (3, 2).
    let (mut px, w, h) = grid(&[
        ".........",
        "....#....",
        "...#o#...",
        "..#ooo#..",
        ".#ooooo#.",
        "..#ooo#..",
        "...#o#...",
        "....#....",
        ".........",
    ]);
    // Cut the break: the pixel at (3, 2) is the upper-left edge between (4, 1) and (2, 3).
    px[(2 * w + 3) * 4..][..4].copy_from_slice(&CLEAR);
    let filled = close_outline(&mut px, w, h);
    assert_eq!(filled, 1);
    assert_eq!(text(&px, w)[2], "...#o#...");
}

#[test]
fn trace_can_close_the_outline() {
    // Cut a one-cell notch out of the sprite's outline before rendering; the traced image closes it.
    let mut s = Sprite::new(30, 32);
    let (nx, ny) = (s.nx, s.ny);
    s.px[(ny - 1) * nx + nx / 2] = CLEAR;
    s.px[(ny - 1) * nx + nx / 2 + 1] = OUTLINE;
    let r = render(&s, &default_look());
    let open = trace_grid(&r.rgba, r.w, r.h, &TraceOptions { close_outline: false, ..cleaning() }).unwrap();
    let closed = trace_grid(&r.rgba, r.w, r.h, &TraceOptions { close_outline: true, ..cleaning() }).unwrap();
    assert_eq!(open.rgba[((ny - 1) * open.width + nx / 2) * 4 + 3], 0, "the notch is there without closing");
    let got = px(&closed.rgba, closed.width, nx / 2, ny - 1);
    assert!(got[3] == 255 && dist(got, OUTLINE) <= 30.0, "closed: {got:?}");
}

// MARK: Background removal

#[test]
fn detects_the_border_colour_through_noise() {
    let s = Sprite::new(30, 32);
    let r = render(&s, &default_look());
    let bg = detect_background(&r.rgba, r.w, r.h).unwrap();
    assert!(bg.iter().all(|&c| c >= 245), "{bg:?}");
    // Mostly transparent border: nothing to detect.
    let clear = vec![0u8; 16 * 16 * 4];
    assert_eq!(detect_background(&clear, 16, 16), None);
}

fn raw_cleaned(opts: &BgOptions) -> (Sprite, Render, Vec<u8>, pixmith_core::paint::background::BgStats) {
    let s = Sprite::new(30, 32);
    let r = render(&s, &default_look());
    let mut out = r.rgba.clone();
    let stats = remove_background(&mut out, r.w, r.h, opts);
    (s, r, out, stats)
}

/// Pixel at the centre of an original sprite pixel.
fn centre(r: &Render, out: &[u8], x: usize, y: usize) -> [u8; 4] {
    let l = default_look();
    let (px_, py_) = (l.origin.0 + (x as f64 + 0.5) * l.pitch.0, l.origin.1 + (y as f64 + 0.5) * l.pitch.1);
    px(out, r.w, px_ as usize, py_ as usize)
}

#[test]
fn flood_removes_the_paper_and_keeps_enclosed_whites() {
    let (s, r, out, stats) = raw_cleaned(&BgOptions::default());
    assert_eq!(stats.shadow, 0);
    assert!(stats.background > r.w * r.h / 3, "{stats:?}");
    assert_eq!(out[3], 0, "the corner is gone");
    for y in 0..s.ny {
        for x in 0..s.nx {
            let want = s.at(x, y);
            if want == WHITE || want == GREY {
                assert_eq!(centre(&r, &out, x, y)[3], 255, "enclosed {want:?} at {x},{y} survived the flood");
            }
        }
    }
}

#[test]
fn global_removal_also_takes_enclosed_paper_colours() {
    let (s, r, out, stats) = raw_cleaned(&BgOptions { global: true, ..BgOptions::default() });
    assert!(stats.global > 0, "{stats:?}");
    let mut gone = 0;
    for y in 0..s.ny {
        for x in 0..s.nx {
            if s.at(x, y) == WHITE {
                gone += (centre(&r, &out, x, y)[3] == 0) as usize;
            }
            if s.at(x, y) == GREY {
                assert_eq!(centre(&r, &out, x, y)[3], 255, "grey is not paper");
            }
        }
    }
    assert!(gone >= 4, "enclosed whites are removed by the global option ({gone})");
}

#[test]
fn shadow_is_removed_only_when_asked_and_is_conservative() {
    let (s, r, plain, a) = raw_cleaned(&BgOptions::default());
    let (_, _, shadowless, b) = raw_cleaned(&BgOptions { shadow: true, ..BgOptions::default() });
    assert_eq!(a.shadow, 0);
    assert!(b.shadow > 200, "{b:?}");
    // The darkest shadow pixel (below the feet) survives without the option and is gone with it.
    let look = default_look();
    let (sx, sy) = (
        (look.origin.0 + s.nx as f64 * look.pitch.0 / 2.0) as usize,
        (look.origin.1 + s.ny as f64 * look.pitch.1 + 0.4 * look.pitch.1) as usize,
    );
    assert_eq!(px(&plain, r.w, sx, sy)[3], 255);
    assert_eq!(px(&shadowless, r.w, sx, sy)[3], 0);
    // The figure itself, and what is enclosed in it, stays.
    for y in 0..s.ny {
        for x in 0..s.nx {
            let want = s.at(x, y);
            if want[3] != 0 {
                assert_eq!(centre(&r, &shadowless, x, y)[3], 255, "figure pixel {x},{y} was removed");
            }
        }
    }
}

#[test]
fn an_image_that_is_not_on_a_background_is_left_alone() {
    // Three colours in stripes touching every border: none covers half of it.
    let mut img = vec![0u8; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let v = [20u8, 120, 230][(x / 4 + y / 4) % 3];
            img[(y * 32 + x) * 4..][..4].copy_from_slice(&[v, v / 2, 255 - v, 255]);
        }
    }
    let before = img.clone();
    let stats = remove_background(&mut img, 32, 32, &BgOptions::default());
    assert_eq!(stats.removed(), 0);
    assert_eq!(img, before);
}

#[test]
fn a_large_dark_grey_figure_is_not_mistaken_for_a_shadow() {
    // A light-grey knight without an outline on white paper: removing it as "shadow" would erase the figure, so
    // the shadow step backs off.
    let (w, h) = (48usize, 48usize);
    let mut img = vec![255u8; w * h * 4];
    for y in 8..44 {
        for x in 8..40 {
            img[(y * w + x) * 4..][..4].copy_from_slice(&[170, 170, 172, 255]);
        }
    }
    let stats = remove_background(&mut img, w, h, &BgOptions { shadow: true, ..BgOptions::default() });
    assert_eq!(img[(24 * w + 24) * 4 + 3], 255, "the figure stays");
    assert_eq!(stats.shadow, 0);
    assert!(stats.shadow_skipped);
}
