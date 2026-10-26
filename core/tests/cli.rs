//! Integration tests for the `pixmith-cli` binary.

use pixmith_core::document::{Layer, Sprite, Tag};
use pixmith_core::formats::{ase, import};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pixmith-cli-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pixmith-cli")).args(args).output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// 2x2 sprite, 3 frames (durations 100/200/300), layers "bg" (red at 0,0) and "fg" (blue at 1,1).
fn sample() -> Sprite {
    let mut s = Sprite::new(2, 2);
    s.layers[0].name = "bg".into();
    s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
    let fg = s.add_layer(0);
    s.layers[fg].name = "fg".into();
    s.draw_line(fg, 0, 1, 1, 1, 1, BLUE, 1);
    s.add_frame(0, true);
    s.add_frame(1, true);
    s.durations = vec![100, 200, 300];
    s.tags.push(Tag {
        name: "tail".into(),
        from: 1,
        to: 2,
        direction: 2,
        repeat: 0,
        color: [1, 2, 3],
        user_data: Default::default(),
    });
    s
}

fn write_sample(dir: &Path, name: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, ase::write(&sample())).unwrap();
    path.to_string_lossy().into_owned()
}

fn px(path: &Path) -> (usize, usize, Vec<u8>) {
    import::decode_png(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn info_prints_text_and_json() {
    let dir = temp_dir("info");
    let f = write_sample(&dir, "a.aseprite");
    let out = cli(&["info", &f]);
    assert!(out.status.success());
    let t = text(&out.stdout);
    assert!(t.contains("size: 2x2"), "{t}");
    assert!(t.contains("frames: 3 (600 ms)"), "{t}");
    assert!(t.contains("layers: 2"), "{t}");
    assert!(t.contains("tail: frames 2-3 pingpong"), "{t}");

    let out = cli(&["info", "--json", &f]);
    let j = text(&out.stdout);
    assert!(j.starts_with("{\"file\":"), "{j}");
    assert!(j.contains("\"width\":2,\"height\":2,\"frames\":3,\"durations\":[100,200,300]"), "{j}");
    assert!(j.contains("\"name\":\"fg\""), "{j}");
    assert!(j.contains("\"name\":\"tail\",\"from\":2,\"to\":3,\"direction\":\"pingpong\""), "{j}");
}

#[test]
fn info_json_for_several_files_is_an_array() {
    let dir = temp_dir("info-many");
    let (a, b) = (write_sample(&dir, "a.aseprite"), write_sample(&dir, "b.aseprite"));
    let j = text(&cli(&["info", "--json", &a, &b]).stdout);
    assert!(j.starts_with("[{") && j.trim_end().ends_with("}]"), "{j}");
    assert_eq!(j.matches("\"width\":2").count(), 2);
}

#[test]
fn exports_png_with_frame_and_scale() {
    let dir = temp_dir("png");
    let f = write_sample(&dir, "a.aseprite");
    let out_png = dir.join("out.png");
    let out = cli(&["export", &f, "--png", out_png.to_str().unwrap(), "--scale", "3"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let (w, h, rgba) = px(&out_png);
    assert_eq!((w, h), (6, 6));
    assert_eq!(&rgba[0..4], &RED);
    assert_eq!(&rgba[(5 * 6 + 5) * 4..][..4], &BLUE);

    let out = cli(&["export", &f, "--png", out_png.to_str().unwrap(), "--frame", "4"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("frame 4 does not exist"));
}

#[test]
fn exports_gif_with_all_frames() {
    let dir = temp_dir("gif");
    let f = write_sample(&dir, "a.aseprite");
    let out_gif = dir.join("sub/out.gif");
    assert!(cli(&["export", &f, "--gif", out_gif.to_str().unwrap()]).status.success());
    let mut dec = gif::DecodeOptions::new().read_info(std::fs::File::open(&out_gif).unwrap()).unwrap();
    let mut delays = Vec::new();
    while let Some(fr) = dec.read_next_frame().unwrap() {
        delays.push(fr.delay);
    }
    // Identical consecutive frames are merged and their delays summed, so only the total is fixed.
    assert!(!delays.is_empty() && delays.len() <= 3, "{delays:?}");
    assert_eq!(delays.iter().map(|&d| u32::from(d)).sum::<u32>(), 60, "{delays:?}");
}

#[test]
fn exports_apng_with_all_frames() {
    let dir = temp_dir("apng");
    let f = write_sample(&dir, "a.aseprite");
    let out_png = dir.join("anim.png");
    let out = cli(&["export", &f, "--apng", out_png.to_str().unwrap(), "--scale", "2"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let reader = png::Decoder::new(std::fs::File::open(&out_png).unwrap()).read_info().unwrap();
    let ctl = reader.info().animation_control.unwrap();
    assert_eq!(ctl.num_frames, 3);
    assert_eq!((reader.info().width, reader.info().height), (4, 4));
}

#[test]
fn exports_sheet_and_json() {
    let dir = temp_dir("sheet");
    let f = write_sample(&dir, "a.aseprite");
    let (png, json) = (dir.join("s.png"), dir.join("s.json"));
    let out = cli(&[
        "export",
        &f,
        "--sheet",
        png.to_str().unwrap(),
        "--layout",
        "grid",
        "--columns",
        "2",
        "--padding",
        "1",
        "--json",
        json.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!((px(&png).0, px(&png).1), (1 + 2 * 3, 1 + 2 * 3));
    let j = std::fs::read_to_string(&json).unwrap();
    assert!(j.contains("\"image\": \"s.png\""), "{j}");
    assert!(j.contains("\"frame\": { \"x\": 4, \"y\": 1, \"w\": 2, \"h\": 2 }"), "{j}");
    assert!(j.contains("\"name\": \"tail\""));

    let out = cli(&["export", &f, "--sheet", png.to_str().unwrap(), "--layout", "vertical"]);
    assert!(out.status.success());
    assert_eq!((px(&png).0, px(&png).1), (2, 6));
}

#[test]
fn exports_png_sequence() {
    let dir = temp_dir("seq");
    let f = write_sample(&dir, "a.aseprite");
    let base = dir.join("frames/walk");
    assert!(cli(&["export", &f, "--png-seq", base.to_str().unwrap(), "-v"]).status.success());
    for n in ["001", "002", "003"] {
        assert!(dir.join(format!("frames/walk-{n}.png")).exists(), "{n}");
    }
    assert!(!dir.join("frames/walk-004.png").exists());
}

#[test]
fn layer_filter_keeps_only_the_named_layer() {
    let dir = temp_dir("layer");
    let f = write_sample(&dir, "a.aseprite");
    let out_png = dir.join("fg.png");
    assert!(cli(&["export", &f, "--png", out_png.to_str().unwrap(), "--layer", "fg"]).status.success());
    let (_, _, rgba) = px(&out_png);
    assert_eq!(&rgba[0..4], &[0, 0, 0, 0]);
    assert_eq!(&rgba[12..16], &BLUE);

    let out = cli(&["export", &f, "--png", out_png.to_str().unwrap(), "--layer", "nope"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("no layer named \"nope\""));
}

#[test]
fn hidden_layer_can_be_selected_and_groups_pull_in_children() {
    let dir = temp_dir("group");
    let mut s = Sprite::new(1, 1);
    let mut group = Layer::new("g");
    group.kind = 1;
    group.visible = false;
    let mut child = Layer::new("c");
    child.child_level = 1;
    s.layers = vec![Layer::new("other"), group, child];
    s.cels = vec![vec![None, None, None]];
    s.draw_line(2, 0, 0, 0, 0, 0, RED, 1);
    let f = dir.join("g.aseprite");
    std::fs::write(&f, ase::write(&s)).unwrap();
    let out_png = dir.join("g.png");
    assert!(cli(&["export", f.to_str().unwrap(), "--png", out_png.to_str().unwrap(), "--layer", "g"]).status.success());
    assert_eq!(&px(&out_png).2[..], &RED);
}

#[test]
fn tag_filter_exports_only_that_range() {
    let dir = temp_dir("tag");
    let f = write_sample(&dir, "a.aseprite");
    let base = dir.join("t");
    let (sheet_png, json) = (dir.join("t.png"), dir.join("t.json"));
    let out = cli(&[
        "export",
        &f,
        "--tag",
        "tail",
        "--png-seq",
        base.to_str().unwrap(),
        "--sheet",
        sheet_png.to_str().unwrap(),
        "--json",
        json.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(dir.join("t-001.png").exists() && dir.join("t-002.png").exists());
    assert!(!dir.join("t-003.png").exists());
    assert_eq!(px(&sheet_png).0, 4);
    let j = std::fs::read_to_string(&json).unwrap();
    assert!(j.contains("\"from\": 0, \"to\": 1"), "{j}");
    assert!(j.contains("\"duration\": 200") && j.contains("\"duration\": 300") && !j.contains("\"duration\": 100"));

    let out = cli(&["export", &f, "--tag", "nope", "--png", dir.join("x.png").to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("no tag named \"nope\""));
}

#[test]
fn batch_expands_name_placeholder() {
    let dir = temp_dir("batch");
    let (a, b) = (write_sample(&dir, "one.aseprite"), write_sample(&dir, "two.aseprite"));
    let template = dir.join("out/{name}.png");
    let out = cli(&["export", &a, &b, "--png", template.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(dir.join("out/one.png").exists() && dir.join("out/two.png").exists());
}

#[test]
fn batch_continues_after_a_bad_file_and_exits_1() {
    let dir = temp_dir("batch-bad");
    let good = write_sample(&dir, "good.aseprite");
    let bad = dir.join("bad.aseprite");
    std::fs::write(&bad, b"not a sprite").unwrap();
    let template = dir.join("{name}.png");
    let out = cli(&["export", bad.to_str().unwrap(), &good, "--png", template.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("bad.aseprite"));
    assert!(dir.join("good.png").exists());
    assert!(!dir.join("bad.png").exists());
}

#[test]
fn missing_file_exits_1() {
    let out = cli(&["info", "/nonexistent/file.aseprite"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).starts_with("pixmith-cli: /nonexistent/file.aseprite:"));
}

#[test]
fn usage_errors_exit_2() {
    let dir = temp_dir("usage");
    let f = write_sample(&dir, "a.aseprite");
    let g = write_sample(&dir, "b.aseprite");
    let png = dir.join("o.png");
    let png = png.to_str().unwrap();
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (vec![], "missing command"),
        (vec!["frobnicate"], "unknown command"),
        (vec!["info"], "no input files"),
        (vec!["export", &f], "nothing to do"),
        (vec!["export", &f, "--png"], "needs a value"),
        (vec!["export", &f, "--bogus", "1"], "unknown option"),
        (vec!["export", &f, "--png", png, "--scale", "0"], "between 1 and 64"),
        (vec!["export", &f, "--png", png, "--scale", "x"], "whole number"),
        (vec!["export", &f, "--png", png, "--frame", "0"], "from 1"),
        (vec!["export", &f, "--gif", png, "--frame", "1"], "--frame needs --png"),
        (vec!["export", &f, "--png", png, "--json", png], "--json needs --sheet"),
        (vec!["export", &f, "--sheet", png, "--layout", "diagonal"], "unknown layout"),
        (vec!["export", &f, "--sheet", png, "--layout", "vertical", "--columns", "2"], "only applies"),
        (vec!["export", &f, "--png", png, "--padding", "2"], "need --sheet"),
        (vec!["export", &f, &g, "--png", png], "must contain {name}"),
    ];
    for (args, needle) in cases {
        let out = cli(&args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(text(&out.stderr).contains(needle), "{args:?}: {}", text(&out.stderr));
    }
    assert!(!Path::new(png).exists());
}

/// 8x8 sprite with 4 frames; frame f has a 2x2 blob at (f + 1, f + 2) on layer "a" and a pixel at (7, 7) on layer "b".
fn walker() -> Sprite {
    let mut s = Sprite::new(8, 8);
    s.layers[0].name = "a".into();
    for f in 1..4 {
        s.add_frame(f - 1, false);
    }
    let b = s.add_layer(0);
    s.layers[b].name = "b".into();
    for f in 0..4 {
        s.cels[f][0] = Some(std::sync::Arc::new(pixmith_core::document::Cel {
            opacity: 255,
            pixels: vec![0; 8 * 8 * 4],
            ..Default::default()
        }));
        s.draw_line(0, f, f as i32 + 1, f as i32 + 2, f as i32 + 2, f as i32 + 2, RED, 1);
        s.draw_line(0, f, f as i32 + 1, f as i32 + 3, f as i32 + 2, f as i32 + 3, RED, 1);
    }
    s.draw_line(b, 0, 7, 7, 7, 7, BLUE, 1);
    s.durations = vec![10, 20, 30, 40];
    s.tags.push(Tag {
        name: "pp".into(),
        from: 0,
        to: 2,
        direction: 2,
        repeat: 2,
        color: [0; 3],
        user_data: Default::default(),
    });
    s.tags.push(Tag {
        name: "back".into(),
        from: 2,
        to: 3,
        direction: 1,
        repeat: 0,
        color: [0; 3],
        user_data: Default::default(),
    });
    s
}

fn write_walker(dir: &Path) -> String {
    let path = dir.join("w.aseprite");
    std::fs::write(&path, ase::write(&walker())).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn tag_direction_and_repeat_are_exported() {
    let dir = temp_dir("direction");
    let f = write_walker(&dir);
    let (seq, apng, sheet, json) = (dir.join("s"), dir.join("a.png"), dir.join("sh.png"), dir.join("sh.json"));
    let out = cli(&[
        "export",
        &f,
        "--tag",
        "pp",
        "--png-seq",
        seq.to_str().unwrap(),
        "--apng",
        apng.to_str().unwrap(),
        "--sheet",
        sheet.to_str().unwrap(),
        "--json",
        json.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    // Ping-pong over 3 frames: 1 2 3 2.
    assert!(dir.join("s-004.png").exists() && !dir.join("s-005.png").exists());
    assert_eq!(px(&sheet).0, 8 * 4);
    let j = std::fs::read_to_string(&json).unwrap();
    assert!(j.contains("\"from\": 0, \"to\": 3, \"direction\": \"forward\""), "{j}");
    assert_eq!(j.matches("\"duration\": 20").count(), 2);
    let data = std::fs::read(&apng).unwrap();
    let ctl = png::Decoder::new(&data[..]).read_info().unwrap().info().animation_control.unwrap();
    assert_eq!((ctl.num_frames, ctl.num_plays), (4, 2));
    // Reverse: frames 4, 3.
    let r = dir.join("r.png");
    assert!(cli(&[
        "export",
        &f,
        "--tag",
        "back",
        "--sheet",
        r.to_str().unwrap(),
        "--json",
        dir.join("r.json").to_str().unwrap()
    ])
    .status
    .success());
    let j = std::fs::read_to_string(dir.join("r.json")).unwrap();
    assert!(j.find("\"duration\": 40").unwrap() < j.find("\"duration\": 30").unwrap(), "{j}");
    // --no-direction keeps the tag as is.
    let n = dir.join("n.png");
    assert!(cli(&[
        "export",
        &f,
        "--tag",
        "pp",
        "--no-direction",
        "--sheet",
        n.to_str().unwrap(),
        "--json",
        dir.join("n.json").to_str().unwrap()
    ])
    .status
    .success());
    assert_eq!(px(&n).0, 8 * 3);
    let j = std::fs::read_to_string(dir.join("n.json")).unwrap();
    assert!(j.contains("\"direction\": \"pingpong\"") && j.contains("\"repeat\": \"2\""), "{j}");
}

#[test]
fn split_tags_and_layers_write_one_file_each() {
    let dir = temp_dir("split");
    let f = write_walker(&dir);
    let base = dir.join("o.png");
    let out = cli(&[
        "export",
        &f,
        "--split-tags",
        "--sheet",
        base.to_str().unwrap(),
        "--json",
        dir.join("o.json").to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(px(&dir.join("o-pp.png")).0, 8 * 4); // ping-pong 1 2 3 2
    assert_eq!(px(&dir.join("o-back.png")).0, 8 * 2);
    assert!(dir.join("o-pp.json").exists() && dir.join("o-back.json").exists());
    assert!(!base.exists());

    let gif = dir.join("g.gif");
    let out = cli(&["export", &f, "--split-layers", "--gif", gif.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(dir.join("g-a.gif").exists() && dir.join("g-b.gif").exists());

    // Layers and tags combine: <out>-<layer>-<tag>.
    let both = dir.join("c.png");
    let out = cli(&["export", &f, "--split-layers", "--split-tags", "--png", both.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    for name in ["c-a-pp.png", "c-a-back.png", "c-b-pp.png", "c-b-back.png"] {
        assert!(dir.join(name).exists(), "{name}");
    }
    // Layer "b" only has its pixel in frame 1: the pp tag starts with it.
    assert_eq!(px(&dir.join("c-b-pp.png")).2[(7 * 8 + 7) * 4 + 3], 255);
    assert_eq!(px(&dir.join("c-b-back.png")).2[(7 * 8 + 7) * 4 + 3], 0);
    // --layer limits which layers are split.
    let one = dir.join("l.png");
    assert!(cli(&["export", &f, "--layer", "b", "--split-layers", "--png", one.to_str().unwrap()]).status.success());
    assert!(dir.join("l-b.png").exists() && !dir.join("l-a.png").exists());
}

#[test]
fn trim_crops_frames_and_describes_them_in_the_json() {
    let dir = temp_dir("trim");
    let f = write_walker(&dir);
    let (sheet, json) = (dir.join("t.png"), dir.join("t.json"));
    let args =
        ["export", &f, "--layer", "a", "--trim", "--sheet", sheet.to_str().unwrap(), "--json", json.to_str().unwrap()];
    let out = cli(&args);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let (w, h, _) = px(&sheet);
    assert_eq!((w, h), (2 * 4, 2)); // four 2x2 blobs
    let j = std::fs::read_to_string(&json).unwrap();
    assert!(j.contains("\"trimmed\": true"));
    assert!(j.contains("\"spriteSourceSize\": { \"x\": 1, \"y\": 2, \"w\": 2, \"h\": 2 }"), "{j}");
    assert!(j.contains("\"spriteSourceSize\": { \"x\": 4, \"y\": 5, \"w\": 2, \"h\": 2 }"), "{j}");
    assert!(j.contains("\"sourceSize\": { \"w\": 8, \"h\": 8 }"));
    // --trim-sprite keeps one rectangle: x 1..6, y 2..7.
    let seq = dir.join("q");
    assert!(cli(&["export", &f, "--layer", "a", "--trim-sprite", "--png-seq", seq.to_str().unwrap()]).status.success());
    assert_eq!((px(&dir.join("q-001.png")).0, px(&dir.join("q-004.png")).1), (5, 5));
    // Animations use one rectangle even with --trim.
    let gif = dir.join("t.gif");
    assert!(cli(&["export", &f, "--layer", "a", "--trim", "--gif", gif.to_str().unwrap()]).status.success());
    let mut d = gif::DecodeOptions::new();
    d.set_color_output(gif::ColorOutput::RGBA);
    let bytes = std::fs::read(&gif).unwrap();
    let dec = d.read_info(&bytes[..]).unwrap();
    assert_eq!((dec.width(), dec.height()), (5, 5));
}

#[test]
fn sheet_extrude_and_border_padding() {
    let dir = temp_dir("extrude");
    let f = write_walker(&dir);
    let sheet = dir.join("e.png");
    let out = cli(&[
        "export",
        &f,
        "--layer",
        "b",
        "--sheet",
        sheet.to_str().unwrap(),
        "--layout",
        "vertical",
        "--padding",
        "1",
        "--border-padding",
        "3",
        "--sheet-extrude",
        "1",
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    // 4 frames of 8x8 plus a 1px ring each, 1px between them, 3px around the sheet.
    assert_eq!(px(&sheet).0, 2 * 3 + 10);
    assert_eq!(px(&sheet).1, 2 * 3 + 4 * 10 + 3);
}

#[test]
fn ignore_empty_drops_blank_frames() {
    let dir = temp_dir("empty");
    let f = write_walker(&dir);
    let seq = dir.join("b");
    // Layer "b" only paints frame 1.
    let out = cli(&["export", &f, "--layer", "b", "--ignore-empty", "--png-seq", seq.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(dir.join("b-001.png").exists() && !dir.join("b-002.png").exists());
}

#[test]
fn new_flag_usage_errors() {
    let dir = temp_dir("flags");
    let f = write_walker(&dir);
    let png = dir.join("o.png");
    let png = png.to_str().unwrap();
    for (args, needle) in [
        (vec!["export", &f, "--png", png, "--split-tags", "--tag", "pp"], "cannot be combined"),
        (vec!["export", &f, "--png", png, "--no-direction"], "needs --tag"),
        (vec!["export", &f, "--png", png, "--trim", "--trim-sprite"], "cannot be combined"),
        (vec!["export", &f, "--png", png, "--sheet-extrude", "1"], "need --sheet"),
        (vec!["export", &f, "--png", png, "--split-layers=yes"], "unknown option"),
    ] {
        let out = cli(&args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(text(&out.stderr).contains(needle), "{args:?}: {}", text(&out.stderr));
    }
}

#[test]
fn help_succeeds() {
    let out = cli(&["--help"]);
    assert!(out.status.success());
    assert!(text(&out.stdout).contains("pixmith-cli export"));
}

#[test]
fn exports_godot_tileset_inside_project_only() {
    let dir = temp_dir("godot");
    let mut s = Sprite::new(4, 4);
    s.tilesets.push(pixmith_core::document::tilemap::Tileset::new("Ground", 2, 2));
    s.tilesets[0].add_blank();
    let f = dir.join("t.aseprite");
    std::fs::write(&f, ase::write(&s)).unwrap();
    let f = f.to_string_lossy().into_owned();
    let proj = dir.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join("project.godot"), "").unwrap();
    let (root, tres) =
        (proj.to_string_lossy().into_owned(), proj.join("art/tiles.tres").to_string_lossy().into_owned());
    let out = cli(&["export", &f, "--godot-root", &root, "--godot-tileset", &tres]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("no auto-tile masks"));
    let t = std::fs::read_to_string(&tres).unwrap();
    assert!(t.contains("res://art/tiles.png") && t.contains("0:0/0 = 0"), "{t}");
    assert!(proj.join("art/tiles.png").exists());
    let bad = dir.join("outside.tres").to_string_lossy().into_owned();
    let out = cli(&["export", &f, "--godot-root", &root, "--godot-tileset", &bad]);
    assert_eq!(out.status.code(), Some(1));
    assert!(!dir.join("outside.png").exists());
}

#[test]
fn godot_options_scene_and_validation() {
    let dir = temp_dir("godot2");
    let mut s = Sprite::new(4, 4);
    let layer = pixmith_core::document::tilemap::add_tilemap_layer(&mut s, 0, 2, 2);
    s.tilesets[0].add_blank();
    pixmith_core::document::tilemap::stamp(&mut s, layer, 0, 0, 0, 1);
    let f = dir.join("t.aseprite");
    std::fs::write(&f, ase::write(&s)).unwrap();
    let f = f.to_string_lossy().into_owned();
    let proj = dir.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join("project.godot"), "").unwrap();
    let (root, tres) = (proj.to_string_lossy().into_owned(), proj.join("t.tres").to_string_lossy().into_owned());
    let out = cli(&[
        "export",
        &f,
        "--godot-root",
        &root,
        "--godot-tileset",
        &tres,
        "--godot-scene",
        "--godot-collision",
        "full",
        "--godot-extrude",
        "1",
        "--godot-mode",
        "sides",
        "--godot-color",
        "1,0,0",
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let t = std::fs::read_to_string(&tres).unwrap();
    assert!(t.contains("physics_layer_0/collision_layer = 1") && t.contains("separation = Vector2i(2, 2)"), "{t}");
    let sc = std::fs::read_to_string(proj.join("t.tscn")).unwrap();
    assert!(sc.contains("type=\"TileMapLayer\"") && sc.contains("PackedByteArray(0, 0, 0, 0, 0, 0,"), "{sc}");
    // Godot options without --godot-tileset are a usage error.
    let out = cli(&["export", &f, "--png", "x.png", "--godot-scene"]);
    assert_eq!(out.status.code(), Some(2));
    let out = cli(&["export", &f, "--godot-root", &root, "--godot-tileset", &tres, "--godot-mode", "weird"]);
    assert_eq!(out.status.code(), Some(2));
}
