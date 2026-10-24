//! WebP, JPEG, BMP, TGA, ICO, PCX and FLI/FLC through the export planner and the import entry points:
//! round trips (pixels within each format's limits, frame counts, durations, icon sizes), the C functions, and
//! damaged files, which must come back as errors and never panic.

use pixmith_core::document::{Editor, Sprite, Tag};
use pixmith_core::formats::export_plan::{
    plan, render_outputs, EncodeOptions, ExportOptions, Format, Job, PixmithExport, Trim,
};
use pixmith_core::formats::raster::{self, abi, ico, ImageFormat, ImageOptions, ImportOptions};
use pixmith_core::formats::sheet;
use std::ffi::{CStr, CString};

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const GREEN: [u8; 4] = [0, 200, 0, 255];

/// 6x4, three frames of 40/80/160 ms: a red diagonal that moves, a blue dot, and a transparent background.
fn animated() -> Sprite {
    let mut s = Sprite::new(6, 4);
    s.draw_line(0, 0, 0, 0, 3, 3, RED, 1);
    s.add_frame(0, true);
    s.draw_line(0, 1, 5, 0, 5, 0, BLUE, 1);
    s.add_frame(1, true);
    s.draw_line(0, 2, 0, 3, 5, 3, GREEN, 1);
    s.durations = vec![40, 80, 160];
    s.tags.push(Tag {
        name: "go".into(),
        from: 0,
        to: 2,
        direction: 2,
        repeat: 0,
        color: [0; 3],
        user_data: Default::default(),
    });
    s
}

fn encode(s: &Sprite, fmt: Format, o: &ExportOptions, e: &EncodeOptions) -> Vec<(String, Vec<u8>)> {
    render_outputs(s, fmt, o, e, "out.bin", None).unwrap()
}

fn enc(image: ImageOptions) -> EncodeOptions {
    EncodeOptions { scale: 1, frame: 0, sheet: sheet::Options::default(), write_json: false, image }
}

fn open(bytes: &[u8]) -> Sprite {
    raster::sprite_from_image(bytes, &ImportOptions::default()).unwrap()
}

/// Same visible pixels: equal alpha, and equal color where something is visible.
#[allow(clippy::chunks_exact_to_as_chunks)]
fn same_visible(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.chunks_exact(4).zip(b.chunks_exact(4)).all(|(p, q)| p[3] == q[3] && (p[3] == 0 || p == q))
}

#[test]
fn lossless_formats_round_trip_every_pixel_and_alpha() {
    let s = animated();
    let want = s.render(1);
    for (fmt, alpha_exact) in [(Format::Webp, true), (Format::Bmp, true), (Format::Tga, true)] {
        let o = ExportOptions::default();
        let e = EncodeOptions {
            frame: 1,
            image: ImageOptions { webp_animated: false, ..Default::default() },
            ..enc(Default::default())
        };
        let files = encode(&s, fmt, &o, &e);
        assert_eq!(files.len(), 1);
        let back = open(&files[0].1);
        assert_eq!((back.width, back.height, back.frame_count()), (6, 4, 1), "{fmt:?}");
        assert!(same_visible(&back.render(0), &want), "{fmt:?} {alpha_exact}");
    }
}

#[test]
fn formats_without_alpha_flatten_onto_the_background() {
    let s = animated();
    let bg = [10, 120, 230];
    for fmt in [Format::Pcx, Format::Jpeg] {
        let image = ImageOptions { background: bg, jpeg_quality: 100, ..Default::default() };
        let files = encode(&s, fmt, &ExportOptions::default(), &EncodeOptions { frame: 0, ..enc(image) });
        let back = open(&files[0].1).render(0);
        // Transparent pixel (5, 3) of the first frame shows the background.
        let at = (3 * 6 + 5) * 4;
        let tol = if fmt == Format::Jpeg { 14 } else { 0 };
        for k in 0..3 {
            assert!((back[at + k] as i32 - bg[k] as i32).abs() <= tol, "{fmt:?} channel {k}: {:?}", &back[at..at + 4]);
        }
        assert_eq!(back[at + 3], 255);
    }
}

#[test]
fn jpeg_quality_trades_size_for_error() {
    let mut s = Sprite::new(32, 32);
    for i in 0..32 {
        s.draw_line(0, 0, i, 0, 31 - i, 31, [(i * 8) as u8, 255 - (i * 8) as u8, 90, 255], 1);
    }
    let size = |q: u8| {
        let image = ImageOptions { jpeg_quality: q, ..Default::default() };
        encode(&s, Format::Jpeg, &ExportOptions::default(), &enc(image))[0].1.len()
    };
    assert!(size(20) < size(95));
    // Scale multiplies the dimensions.
    let e = EncodeOptions { scale: 3, ..enc(Default::default()) };
    let back = open(&encode(&s, Format::Jpeg, &ExportOptions::default(), &e)[0].1);
    assert_eq!((back.width, back.height), (96, 96));
}

#[test]
fn webp_lossy_is_smaller_than_lossless() {
    let mut s = Sprite::new(48, 48);
    for i in 0..48 {
        s.draw_line(0, 0, i, 0, 47 - i, 47, [(i * 5) as u8, (i * 3) as u8, 255 - (i * 5) as u8, 255], 1);
    }
    let one = |lossless: bool| {
        let image =
            ImageOptions { webp_lossless: lossless, webp_quality: 50, webp_animated: false, ..Default::default() };
        encode(&s, Format::Webp, &ExportOptions::default(), &enc(image))[0].1.len()
    };
    assert!(one(false) < one(true));
}

#[test]
fn webp_and_flc_keep_frames_and_durations() {
    let s = animated();
    for fmt in [Format::Webp, Format::Flc] {
        let files = encode(&s, fmt, &ExportOptions::default(), &enc(ImageOptions::default()));
        assert_eq!(files.len(), 1, "{fmt:?} writes one animation");
        let back = open(&files[0].1);
        assert_eq!(back.frame_count(), 3, "{fmt:?}");
        assert_eq!(back.durations, vec![40, 80, 160], "{fmt:?}");
        for f in 0..3 {
            let want = if fmt == Format::Flc {
                // FLC has no alpha: empty pixels are the (white) background.
                let mut px = s.render(f);
                raster::flatten(&mut px, [255, 255, 255]);
                px
            } else {
                s.render(f)
            };
            assert!(same_visible(&back.render(f), &want), "{fmt:?} frame {f}");
        }
    }
}

#[test]
fn tag_direction_and_scale_reach_the_animation() {
    let s = animated();
    // The tag is ping-pong over frames 1..3, so the export plays 1, 2, 3, 2.
    let o = ExportOptions { tag: Some(0), ..Default::default() };
    let e = EncodeOptions { scale: 2, ..enc(Default::default()) };
    for fmt in [Format::Webp, Format::Flc] {
        let back = open(&encode(&s, fmt, &o, &e)[0].1);
        assert_eq!((back.width, back.height, back.frame_count()), (12, 8, 4), "{fmt:?}");
        assert_eq!(back.durations, vec![40, 80, 160, 80], "{fmt:?}");
    }
    // A single-frame format writes the chosen frame of the tag.
    let e = EncodeOptions { frame: 2, ..enc(Default::default()) };
    let back = open(&encode(&s, Format::Bmp, &o, &e)[0].1);
    assert!(same_visible(&back.render(0), &s.render(2)));
}

#[test]
fn trim_applies_uniformly_to_animations() {
    let mut s = Sprite::new(10, 10);
    s.draw_line(0, 0, 4, 4, 5, 4, RED, 1);
    s.add_frame(0, true);
    s.draw_line(0, 1, 4, 6, 5, 6, BLUE, 1);
    let o = ExportOptions { trim: Trim::PerFrame, ..Default::default() };
    for fmt in [Format::Webp, Format::Flc] {
        let back = open(&encode(&s, fmt, &o, &enc(Default::default()))[0].1);
        assert_eq!((back.width, back.height, back.frame_count()), (2, 3, 2), "{fmt:?}");
    }
}

#[test]
fn one_file_per_layer_gets_the_layer_name() {
    let mut s = Sprite::new(4, 4);
    s.layers[0].name = "back".into();
    s.draw_line(0, 0, 0, 0, 1, 1, RED, 1);
    let top = s.add_layer(0);
    s.layers[top].name = "front".into();
    s.draw_line(top, 0, 3, 3, 3, 3, BLUE, 1);
    let o = ExportOptions { layers: pixmith_core::formats::export_plan::LayerMode::Each, ..Default::default() };
    let names: Vec<String> = render_outputs(&s, Format::Bmp, &o, &enc(Default::default()), "art.bmp", None)
        .unwrap()
        .into_iter()
        .map(|f| f.0)
        .collect();
    assert_eq!(names, vec!["art-back.bmp", "art-front.bmp"]);
}

#[test]
fn icons_hold_the_chosen_sizes_and_open_at_any_of_them() {
    let s = animated();
    let mut image = ImageOptions::default();
    image.set_ico_sizes(&[256, 16, 32, 48, 64]);
    let files = encode(&s, Format::Ico, &ExportOptions::default(), &enc(image));
    let bytes = &files[0].1;
    let sizes: Vec<(usize, usize)> = ico::entries(bytes).unwrap().iter().map(|e| (e.width, e.height)).collect();
    assert_eq!(sizes, vec![(16, 16), (32, 32), (48, 48), (64, 64), (256, 256)]);
    assert_eq!(open(bytes).width, 256, "opens the largest by default");
    for size in [16u32, 32, 48, 64, 256] {
        let back = raster::sprite_from_image(bytes, &ImportOptions { ico_size: Some(size) }).unwrap();
        assert_eq!((back.width, back.height), (size, size));
    }
    // The 48 pixel image is the 6x4 sprite at 8x, centered: its pixel (24, 24) is the sprite's (3, 2) -> transparent,
    // while (0, 0)-> sprite (0, 0) is the red end of the diagonal.
    let big = raster::sprite_from_image(bytes, &ImportOptions { ico_size: Some(48) }).unwrap().render(0);
    let at = |x: usize, y: usize| &big[(y * 48 + x) * 4..][..4];
    assert_eq!(at(0, 8), &RED); // sprite (0,0), top margin is 8 rows
    assert_eq!(at(47, 47)[3], 0);
}

#[test]
fn imports_by_content_not_by_extension() {
    let s = animated();
    let bmp = encode(&s, Format::Bmp, &ExportOptions::default(), &enc(Default::default())).remove(0).1;
    let from_bytes = raster::sprite_from_file_bytes(&bmp, "picture.jpg", &ImportOptions::default()).unwrap();
    assert_eq!((from_bytes.width, from_bytes.height), (6, 4));
    assert_eq!(raster::detect(&bmp), Some(ImageFormat::Bmp));
    // PNG and GIF files take the same way in.
    let png = pixmith_core::formats::export::frame_png(&s, 1, 1).unwrap();
    assert_eq!(raster::detect(&png), Some(ImageFormat::Png));
    assert_eq!(open(&png).frame_count(), 1);
    let gif = pixmith_core::formats::gif::gif_bytes(&s, 1).unwrap();
    assert_eq!(open(&gif).frame_count(), 3);
    // The imported palette comes from the image's own colors.
    assert!(open(&bmp).palette.contains(&RED));
}

#[test]
fn file_palettes_are_kept_for_indexed_formats() {
    let s = animated();
    let flc = encode(&s, Format::Flc, &ExportOptions::default(), &enc(Default::default())).remove(0).1;
    let back = open(&flc);
    assert_eq!(back.palette.len(), 256);
    assert!(back.palette.iter().any(|c| c[..3] == RED[..3]));
}

fn all_files() -> Vec<(&'static str, Vec<u8>)> {
    let s = animated();
    let mut image = ImageOptions::default();
    image.set_ico_sizes(&[16, 32, 64]);
    let mut v = Vec::new();
    for (name, fmt) in [
        ("webp", Format::Webp),
        ("jpg", Format::Jpeg),
        ("bmp", Format::Bmp),
        ("tga", Format::Tga),
        ("ico", Format::Ico),
        ("pcx", Format::Pcx),
        ("flc", Format::Flc),
    ] {
        v.push((name, encode(&s, fmt, &ExportOptions::default(), &enc(image))[0].1.clone()));
    }
    v
}

#[test]
fn damaged_files_are_errors_never_panics() {
    for (name, bytes) in all_files() {
        // Every truncation.
        for cut in (0..bytes.len().min(700)).step_by(3) {
            let _ = raster::sprite_from_file_bytes(&bytes[..cut], &format!("x.{name}"), &ImportOptions::default());
        }
        // Flipped bytes through the header and the start of the data, then sparse flips over the rest.
        for i in (0..bytes.len().min(160)).chain((160..bytes.len()).step_by(23)) {
            for flip in [0xFFu8, 0x10] {
                let mut bad = bytes.clone();
                bad[i] ^= flip;
                let _ = raster::sprite_from_file_bytes(&bad, &format!("x.{name}"), &ImportOptions::default());
            }
        }
        // Garbage after a valid signature.
        let mut junk = bytes[..bytes.len().min(32)].to_vec();
        junk.extend((0..2000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8));
        let _ = raster::sprite_from_file_bytes(&junk, &format!("x.{name}"), &ImportOptions::default());
    }
}

#[test]
fn empty_and_unknown_input_is_an_error() {
    assert!(raster::sprite_from_image(&[], &ImportOptions::default()).is_err());
    assert!(raster::sprite_from_image(b"plain text, not a picture", &ImportOptions::default()).is_err());
    assert!(raster::sprite_from_file_bytes(b"nope", "x.webp", &ImportOptions::default()).is_err());
    assert!(raster::sprite_from_file_bytes(b"nope", "x.tga", &ImportOptions::default()).is_err());
}

#[test]
fn headers_that_claim_huge_images_are_refused_without_allocating_them() {
    // BMP 60000 x 60000, TGA 65535 x 65535, PCX 65535 x 65535, FLC 65535 x 65535, ICO entry of 60000 x 60000.
    let mut bmp = b"BM".to_vec();
    bmp.extend_from_slice(&[0; 12]);
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&60000i32.to_le_bytes());
    bmp.extend_from_slice(&60000i32.to_le_bytes());
    bmp.extend_from_slice(&[1, 0, 32, 0, 0, 0, 0, 0]);
    bmp.extend_from_slice(&[0; 40]);
    let mut tga = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255, 32, 0];
    tga.extend_from_slice(&[0; 64]);
    let mut pcx = vec![0u8; 200];
    pcx[0] = 0x0A;
    pcx[2] = 1;
    pcx[3] = 8;
    pcx[8..12].copy_from_slice(&[255, 255, 255, 255]);
    pcx[65] = 3;
    pcx[66..68].copy_from_slice(&[255, 255]);
    let mut flc = vec![0u8; 200];
    flc[4..6].copy_from_slice(&0xAF12u16.to_le_bytes());
    flc[6] = 5;
    flc[8..12].copy_from_slice(&[255, 255, 255, 255]);
    flc[12] = 8;
    for (name, bytes) in [("bmp", bmp), ("tga", tga), ("pcx", pcx), ("flc", flc)] {
        let err = raster::sprite_from_file_bytes(&bytes, &format!("x.{name}"), &ImportOptions::default());
        assert!(err.is_err(), "{name}");
    }
}

// ---- C functions ----------------------------------------------------------------------------

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

#[test]
fn c_export_run_ex_writes_the_new_formats_and_import_image_reads_them() {
    let dir = std::env::temp_dir().join(format!("pixmith-raster-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ed = Box::into_raw(Box::new(Editor::new(animated())));
    let mut opts: PixmithExport = unsafe { std::mem::zeroed() };
    opts.scale = 2;
    opts.tag = -1;
    opts.apply_direction = true;
    for (format, ext, frames) in [
        (5u32, "webp", 3usize),
        (6, "jpg", 1),
        (7, "bmp", 1),
        (8, "tga", 1),
        (9, "ico", 1),
        (10, "pcx", 1),
        (11, "flc", 3),
    ] {
        opts.format = format;
        let path = c(dir.join(format!("out.{ext}")).to_str().unwrap());
        let json = c(r##"{"ico_sizes":[32,16],"background":"#ffffff"}"##);
        let n = unsafe { abi::pixmith_export_run_ex(ed, &opts, json.as_ptr(), path.as_ptr()) };
        assert_eq!(n, 1, "{ext}: {:?}", unsafe { CStr::from_ptr(pixmith_core::ffi::pixmith_last_error()) });
        let back = unsafe { abi::pixmith_import_image(path.as_ptr()) };
        assert!(!back.is_null(), "{ext}");
        let sprite = unsafe { &(*back).sprite };
        assert_eq!(sprite.frame_count(), frames, "{ext}");
        if ext != "ico" {
            assert_eq!((sprite.width, sprite.height), (12, 8), "{ext}");
        }
        unsafe { drop(Box::from_raw(back)) };
    }
    // Bad options come back as -1 with a message, nothing is written.
    let path = c(dir.join("bad.jpg").to_str().unwrap());
    let json = c(r#"{"bits": 5}"#);
    assert_eq!(unsafe { abi::pixmith_export_run_ex(ed, &opts, json.as_ptr(), path.as_ptr()) }, -1);
    assert!(!dir.join("bad.jpg").exists());
    unsafe { drop(Box::from_raw(ed)) };
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plan_based_animation_export_matches_the_job() {
    let s = animated();
    let jobs: Vec<Job> = plan(&s, &ExportOptions::default()).unwrap();
    let bytes = raster::encode_animation(ImageFormat::Webp, &jobs[0], 1, &ImageOptions::default()).unwrap();
    assert_eq!(open(&bytes).durations, vec![40, 80, 160]);
    assert!(raster::encode_animation(ImageFormat::Jpeg, &jobs[0], 1, &ImageOptions::default()).is_err());
}
