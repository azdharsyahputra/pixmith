//! `pixmith-cli` with WebP, JPEG, BMP, TGA, ICO, PCX and FLC: export options, convert in both directions, usage errors.

#![allow(clippy::chunks_exact_to_as_chunks)]

use pixmith_core::document::Sprite;
use pixmith_core::formats::raster::{self, ico, ImportOptions};
use pixmith_core::formats::{ase, pxmt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pixmith-cli-formats-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pixmith-cli")).args(args).output().unwrap()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// 6x4, three frames (40/80/160 ms), red and blue pixels on a transparent background.
fn sample() -> Sprite {
    let mut s = Sprite::new(6, 4);
    s.draw_line(0, 0, 0, 0, 2, 2, RED, 1);
    s.add_frame(0, true);
    s.draw_line(0, 1, 5, 3, 5, 3, BLUE, 1);
    s.add_frame(1, true);
    s.draw_line(0, 2, 4, 0, 5, 0, BLUE, 1);
    s.durations = vec![40, 80, 160];
    s
}

fn write_sample(dir: &Path) -> String {
    let p = dir.join("hero.aseprite");
    std::fs::write(&p, ase::write(&sample())).unwrap();
    p.to_string_lossy().into_owned()
}

fn open(path: &Path) -> Sprite {
    let bytes = std::fs::read(path).unwrap();
    raster::sprite_from_file_bytes(&bytes, &path.to_string_lossy(), &ImportOptions::default()).unwrap()
}

#[test]
fn exports_every_format_in_one_run() {
    let dir = temp_dir("all");
    let input = write_sample(&dir);
    let o = |n: &str| dir.join(n).to_string_lossy().into_owned();
    let out = cli(&[
        "export",
        &input,
        "--webp",
        &o("a.webp"),
        "--jpg",
        &o("a.jpg"),
        "--bmp",
        &o("a.bmp"),
        "--tga",
        &o("a.tga"),
        "--ico",
        &o("a.ico"),
        "--pcx",
        &o("a.pcx"),
        "--flc",
        &o("a.flc"),
        "-v",
    ]);
    assert!(out.status.success(), "{}", err(&out));
    for (name, frames) in
        [("a.webp", 3), ("a.jpg", 1), ("a.bmp", 1), ("a.tga", 1), ("a.ico", 1), ("a.pcx", 1), ("a.flc", 3)]
    {
        let s = open(&dir.join(name));
        assert_eq!((s.width, s.height, s.frame_count()), (6, 4, frames), "{name}");
    }
    let webp = open(&dir.join("a.webp"));
    assert_eq!(webp.durations, vec![40, 80, 160]);
    let (got, want) = (webp.render(1), sample().render(1));
    assert!(got.chunks_exact(4).zip(want.chunks_exact(4)).all(|(p, q)| p[3] == q[3] && (p[3] == 0 || p == q)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn frame_scale_and_background_apply() {
    let dir = temp_dir("opts");
    let input = write_sample(&dir);
    let bmp = dir.join("f2.bmp");
    let out = cli(&[
        "export",
        &input,
        "--bmp",
        bmp.to_str().unwrap(),
        "--frame",
        "2",
        "--scale",
        "3",
        "--bits",
        "24",
        "--background",
        "#102030",
    ]);
    assert!(out.status.success(), "{}", err(&out));
    let s = open(&bmp);
    assert_eq!((s.width, s.height), (18, 12));
    let px = s.render(0);
    assert_eq!(&px[(17 * 4)..][..4], &[16, 32, 48, 255], "transparent corner is the background");
    // Blue pixel of frame 2 at (5, 3) -> scaled block at (15..18, 9..12).
    assert_eq!(&px[(9 * 18 + 15) * 4..][..4], &BLUE);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ico_sizes_and_webp_options() {
    let dir = temp_dir("ico");
    let input = write_sample(&dir);
    let icon = dir.join("i.ico");
    let out = cli(&["export", &input, "--ico", icon.to_str().unwrap(), "--ico-sizes", "16,32,256"]);
    assert!(out.status.success(), "{}", err(&out));
    let bytes = std::fs::read(&icon).unwrap();
    let sizes: Vec<usize> = ico::entries(&bytes).unwrap().iter().map(|e| e.width).collect();
    assert_eq!(sizes, vec![16, 32, 256]);

    let lossless = dir.join("l.webp");
    let lossy = dir.join("y.webp");
    let still = dir.join("s.webp");
    assert!(cli(&["export", &input, "--webp", lossless.to_str().unwrap()]).status.success());
    assert!(cli(&["export", &input, "--webp", lossy.to_str().unwrap(), "--lossy", "--quality", "40"]).status.success());
    assert!(cli(&["export", &input, "--webp", still.to_str().unwrap(), "--no-animation", "--frame", "3"])
        .status
        .success());
    assert_eq!(open(&lossless).frame_count(), 3);
    assert_eq!(open(&lossy).frame_count(), 3);
    assert_eq!(open(&still).frame_count(), 1);
    assert!(std::fs::read(&lossy).unwrap().len() != std::fs::read(&lossless).unwrap().len());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn tag_split_and_batch_work_with_the_new_outputs() {
    let dir = temp_dir("batch");
    let mut s = sample();
    s.tags.push(pixmith_core::document::Tag {
        name: "tail".into(),
        from: 1,
        to: 2,
        direction: 0,
        repeat: 0,
        color: [0; 3],
        user_data: Default::default(),
    });
    std::fs::write(dir.join("hero.aseprite"), ase::write(&s)).unwrap();
    std::fs::write(dir.join("zed.aseprite"), ase::write(&s)).unwrap();
    let hero = dir.join("hero.aseprite");
    let out = cli(&["export", hero.to_str().unwrap(), "--flc", dir.join("t.flc").to_str().unwrap(), "--tag", "tail"]);
    assert!(out.status.success(), "{}", err(&out));
    assert_eq!(open(&dir.join("t.flc")).frame_count(), 2);
    let out = cli(&["export", hero.to_str().unwrap(), "--webp", dir.join("s.webp").to_str().unwrap(), "--split-tags"]);
    assert!(out.status.success(), "{}", err(&out));
    assert!(dir.join("s-tail.webp").exists());
    let out = cli(&[
        "export",
        hero.to_str().unwrap(),
        dir.join("zed.aseprite").to_str().unwrap(),
        "--jpg",
        &format!("{}/{{name}}.jpg", dir.display()),
    ]);
    assert!(out.status.success(), "{}", err(&out));
    assert!(dir.join("hero.jpg").exists() && dir.join("zed.jpg").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_reads_and_writes_the_new_formats() {
    let dir = temp_dir("convert");
    let input = write_sample(&dir);
    let p = |n: &str| dir.join(n).to_string_lossy().into_owned();
    // Document to image and back to a document.
    for name in ["c.webp", "c.bmp", "c.tga", "c.ico", "c.pcx", "c.flc"] {
        let out = cli(&["convert", &input, &p(name)]);
        assert!(out.status.success(), "{name}: {}", err(&out));
        let back = p(&format!("{name}.pxmt"));
        let out = cli(&["convert", &p(name), &back]);
        assert!(out.status.success(), "{name} back: {}", err(&out));
        let s = pxmt::read_document(&std::fs::read(&back).unwrap()).unwrap();
        assert_eq!((s.width, s.height), (6, 4), "{name}");
    }
    // Animated outputs keep the frames; stills keep the first.
    assert_eq!(open(&dir.join("c.webp")).frame_count(), 3);
    assert_eq!(open(&dir.join("c.flc")).frame_count(), 3);
    assert_eq!(open(&dir.join("c.bmp")).frame_count(), 1);
    // Options on convert.
    let out = cli(&["convert", &input, &p("q.jpg"), "--quality", "50", "--background", "#ff0000"]);
    assert!(out.status.success(), "{}", err(&out));
    // `info` reads images too.
    let out = cli(&["info", &p("c.flc")]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("frames: 3"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bad_usage_and_bad_files_fail_cleanly() {
    let dir = temp_dir("bad");
    let input = write_sample(&dir);
    let p = |n: &str| dir.join(n).to_string_lossy().into_owned();
    for args in [
        vec!["export", &input, "--jpg", "x.jpg", "--quality", "0"],
        vec!["export", &input, "--jpg", "x.jpg", "--quality", "abc"],
        vec!["export", &input, "--ico", "x.ico", "--ico-sizes", "300"],
        vec!["export", &input, "--ico", "x.ico", "--ico-sizes", "1,2,3,4,5,6,7,8,9"],
        vec!["export", &input, "--bmp", "x.bmp", "--bits", "7"],
        vec!["export", &input, "--jpg", "x.jpg", "--background", "red"],
        vec!["export", &input, "--webp"],
        vec!["convert", &input, "x.webp", "--quality", "900"],
        vec!["convert", &input, "x.webp", "--nonsense"],
    ] {
        let out = cli(&args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", err(&out));
        assert!(!err(&out).contains("panicked"));
    }
    // A damaged image is a read error (exit 1), not a crash; no output appears.
    assert!(cli(&["export", &input, "--bmp", &p("g.bmp")]).status.success());
    let good = std::fs::read(p("g.bmp")).unwrap();
    std::fs::write(p("half.bmp"), &good[..good.len() / 2]).unwrap();
    let out = cli(&["convert", &p("half.bmp"), &p("o.pxmt")]);
    assert_eq!(out.status.code(), Some(1), "{}", err(&out));
    assert!(!dir.join("o.pxmt").exists());
    std::fs::write(p("junk.webp"), b"RIFF....WEBPnot really").unwrap();
    let out = cli(&["export", &p("junk.webp"), "--png", &p("o.png")]);
    assert_eq!(out.status.code(), Some(1));
    assert!(!err(&out).contains("panicked"));
    // The old rule for output extensions still holds.
    assert_eq!(cli(&["convert", &input, &p("x.png")]).status.code(), Some(1));
    assert_eq!(cli(&["convert", &input, &input]).status.code(), Some(1));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn help_lists_the_formats() {
    let text = String::from_utf8_lossy(&cli(&["--help"]).stdout).into_owned();
    for needle in
        ["--webp", "--jpg", "--bmp", "--tga", "--ico", "--pcx", "--flc", "--ico-sizes", "--quality", "--background"]
    {
        assert!(text.contains(needle), "{needle}");
    }
}
