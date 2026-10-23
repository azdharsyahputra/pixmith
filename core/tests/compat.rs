//! Compatibility with .aseprite files written by real Aseprite (and other tools). The fixtures
//! come from permissively licensed projects; see tests/fixtures/aseprite/SOURCES.md.

use pixmith_core::document::Sprite;
use pixmith_core::formats::gif as gif_export;
use pixmith_core::formats::import::decode_png;
use pixmith_core::formats::{ase, export, sheet};
use std::path::{Path, PathBuf};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/aseprite")
}

fn all_fixtures() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if matches!(p.extension().and_then(|e| e.to_str()), Some("aseprite" | "ase")) {
                out.push(p);
            }
        }
    }
    let mut v = Vec::new();
    walk(&fixtures_dir(), &mut v);
    v.sort();
    v
}

#[path = "compat/facts.rs"]
mod facts;

fn load(rel: &str) -> Sprite {
    let path = fixtures_dir().join(rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    ase::read(&bytes).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn name(p: &Path) -> String {
    p.strip_prefix(fixtures_dir()).unwrap().to_string_lossy().into_owned()
}

/// Describes the first difference between two sprites, field by field.
fn diff(a: &Sprite, b: &Sprite) -> Option<String> {
    macro_rules! same {
        ($f:ident) => {
            if a.$f != b.$f {
                return Some(format!("{} differs", stringify!($f)));
            }
        };
    }
    same!(width);
    same!(height);
    same!(layers);
    same!(durations);
    same!(tags);
    same!(palette);
    same!(color_mode);
    same!(transparent_index);
    same!(tilesets);
    same!(extra);
    for f in 0..a.frame_count() {
        if a.render(f) != b.render(f) {
            return Some(format!("rendering of frame {} differs", f + 1));
        }
        for l in 0..a.layers.len() {
            match (&a.cels[f][l], &b.cels[f][l]) {
                (None, None) => {}
                (Some(x), Some(y)) if x == y => {}
                (Some(x), Some(y)) if x.pixels == y.pixels => {
                    return Some(format!(
                        "cel (frame {}, layer {}) metadata differs: {:?} vs {:?}",
                        f + 1,
                        l,
                        (x.opacity, &x.user_data, x.tiles.as_ref().map(|t| (t.x, t.y, t.w, t.h))),
                        (y.opacity, &y.user_data, y.tiles.as_ref().map(|t| (t.x, t.y, t.w, t.h)))
                    ));
                }
                (x, y) => {
                    return Some(format!(
                        "cel (frame {}, layer {}) differs: {} vs {}",
                        f + 1,
                        l,
                        x.is_some(),
                        y.is_some()
                    ))
                }
            }
        }
    }
    (a != b).then(|| "sprites differ in an unlisted field".to_string())
}

fn reference_png(rel: &str) -> (usize, usize, Vec<u8>) {
    let path = fixtures_dir().join(rel);
    decode_png(&std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap()
}

/// Compares a rendered frame with a PNG exported by Aseprite. Fully transparent pixels are equal
/// whatever their color; `tolerance` allows small per-channel differences (blend rounding).
fn assert_matches_png(rgba: &[u8], w: usize, h: usize, rel: &str, tolerance: u8) {
    let (rw, rh, reference) = reference_png(rel);
    assert_eq!((w, h), (rw, rh), "{rel}: size");
    let mut worst = 0u8;
    let mut bad = 0;
    for (i, (a, b)) in rgba.as_chunks::<4>().0.iter().zip(reference.as_chunks::<4>().0).enumerate() {
        if a[3] == 0 && b[3] == 0 {
            continue;
        }
        let d = (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap();
        if d > tolerance {
            if bad == 0 {
                eprintln!("{rel}: first mismatch at ({}, {}): got {a:?}, expected {b:?}", i % w, i / w);
            }
            bad += 1;
        }
        worst = worst.max(d);
    }
    assert_eq!(bad, 0, "{rel}: {bad} pixels differ by more than {tolerance} (worst {worst})");
}

fn assert_render(s: &Sprite, frame: usize, rel: &str, tolerance: u8) {
    assert_matches_png(&s.render(frame), s.width as usize, s.height as usize, rel, tolerance);
}

#[test]
fn every_fixture_reads_and_round_trips() {
    let mut failures = Vec::new();
    for path in all_fixtures() {
        let n = name(&path);
        let bytes = std::fs::read(&path).unwrap();
        let s = match ase::read(&bytes) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{n}: read failed: {e}"));
                continue;
            }
        };
        let out = ase::write(&s);
        let back = match ase::read(&out) {
            Ok(b) => b,
            Err(e) => {
                failures.push(format!("{n}: re-read failed: {e}"));
                continue;
            }
        };
        if let Some(d) = diff(&s, &back) {
            failures.push(format!("{n}: round trip: {d}"));
        }
        // Writing the re-read file must be stable.
        if let Ok(again) = ase::read(&ase::write(&back)) {
            if let Some(d) = diff(&back, &again) {
                failures.push(format!("{n}: second round trip: {d}"));
            }
        }
        for f in 0..s.frame_count() {
            if export::frame_png(&s, f, 1).is_err() {
                failures.push(format!("{n}: PNG export of frame {} failed", f + 1));
            }
        }
        if gif_export::gif_bytes(&s, 1).is_err() {
            failures.push(format!("{n}: GIF export failed"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    assert!(all_fixtures().len() >= 30, "fixtures are missing");
}

/// Damaged files must produce an error (or a sprite), never a panic or a hang: the app aborts on panic.
#[test]
fn damaged_files_never_panic() {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut panics = Vec::new();
    for path in all_fixtures() {
        let bytes = std::fs::read(&path).unwrap();
        if bytes.len() > 30_000 {
            continue;
        }
        let mut cases: Vec<Vec<u8>> = (0..30).map(|i| bytes[..bytes.len() * i / 30].to_vec()).collect();
        for _ in 0..100 {
            let mut b = bytes.clone();
            for _ in 0..1 + next() % 4 {
                let i = (next() as usize) % b.len();
                b[i] = next() as u8;
            }
            cases.push(b);
        }
        for (n, case) in cases.iter().enumerate() {
            if std::panic::catch_unwind(|| {
                // Reading must be safe whatever the size; only small results are rendered and written back.
                if let Some(s) = ase::read(case).ok().filter(|s| s.width * s.height <= 4_000_000) {
                    for f in 0..s.frame_count().min(3) {
                        let _ = s.render(f);
                    }
                    let _ = ase::write(&s);
                }
            })
            .is_err()
            {
                panics.push(format!("{} (case {n})", name(&path)));
                break;
            }
        }
    }
    assert!(panics.is_empty(), "panics on damaged input: {panics:?}");
}

#[test]
fn cli_exports_real_files() {
    let dir = std::env::temp_dir().join(format!("pixmith-compat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let input = fixtures_dir().join("asefile/layers_and_tags.aseprite");
    let run = |args: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_pixmith-cli")).args(args).output().unwrap();
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let (png, gif, apng, sheet_png, sheet_json) =
        (dir.join("f.png"), dir.join("f.gif"), dir.join("f-anim.png"), dir.join("s.png"), dir.join("s.json"));
    let p = |p: &Path| p.to_str().unwrap().to_string();
    run(&[
        "export",
        input.to_str().unwrap(),
        "--png",
        &p(&png),
        "--frame",
        "2",
        "--gif",
        &p(&gif),
        "--apng",
        &p(&apng),
        "--sheet",
        &p(&sheet_png),
        "--json",
        &p(&sheet_json),
        "--scale",
        "2",
    ]);

    let (w, h, rgba) = decode_png(&std::fs::read(&png).unwrap()).unwrap();
    assert_eq!((w, h), (32, 32));
    let reference = reference_png("asefile/layers_and_tags_02.png");
    let up = pixmith_core::formats::export::frame_png(&load("asefile/layers_and_tags.aseprite"), 1, 2)
        .map(|b| decode_png(&b).unwrap().2)
        .unwrap();
    assert_eq!(rgba, up);
    assert_eq!((reference.0, reference.1), (16, 16));

    assert_eq!(&std::fs::read(&gif).unwrap()[..6], b"GIF89a");
    let mut dec = gif::DecodeOptions::new().read_info(std::fs::File::open(&gif).unwrap()).unwrap();
    let mut n = 0;
    while dec.read_next_frame().unwrap().is_some() {
        n += 1;
    }
    assert_eq!(n, 4);
    assert_eq!(
        png::Decoder::new(std::fs::File::open(&apng).unwrap())
            .read_info()
            .unwrap()
            .info()
            .animation_control
            .unwrap()
            .num_frames,
        4
    );

    let json = std::fs::read_to_string(&sheet_json).unwrap();
    assert!(json.contains("\"frameTags\"") && json.contains("\"scale\": \"2\""));
    let back = pixmith_core::formats::sheet_import::sprite_from_sheet_json(&std::fs::read(&sheet_png).unwrap(), &json)
        .unwrap();
    let original = load("asefile/layers_and_tags.aseprite");
    assert_eq!((back.width, back.height, back.frame_count(), back.tags.len()), (16, 16, 4, original.tags.len()));

    let info = run(&["info", "--json", input.to_str().unwrap()]);
    assert!(info.contains("\"frames\":4") && info.contains("\"width\":16"), "{info}");
    let _ = std::fs::remove_dir_all(&dir);
}
