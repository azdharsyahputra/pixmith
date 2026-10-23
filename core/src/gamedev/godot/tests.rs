//! Tests for the Godot SpriteFrames export: frame expansion, atlas packing and `.tres` output.

use super::sprite_frames::*;
use super::*;
use crate::document::{Sprite, Tag};

pub(super) fn tag(name: &str, from: u16, to: u16, direction: u8, repeat: u16) -> Tag {
    Tag { name: name.into(), from, to, direction, repeat, color: [0; 3], user_data: Default::default() }
}

/// 4x4 sprite; frame `i` has one red pixel at (i % 4, 1).
pub(super) fn sprite(frames: usize, durations: &[u16]) -> Sprite {
    let mut s = Sprite::new(4, 4);
    for _ in 1..frames {
        s.add_frame(s.frame_count() - 1, false);
    }
    for f in 0..frames {
        let c = s.cel_mut(0, f).unwrap();
        let i = (4 + f % 4) * 4;
        c.pixels[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
        s.durations[f] = durations[f];
    }
    s
}

pub(super) fn target<'a>(prev: Option<&'a str>) -> Target<'a> {
    Target { name: "player", res_dir: "res://art", previous: prev }
}

pub(super) fn two_tag() -> Sprite {
    let mut s = sprite(4, &[100, 150, 50, 100]);
    s.tags = vec![tag("idle", 0, 1, 0, 0), tag("walk", 0, 3, 2, 2)];
    s
}

#[test]
fn gcd_and_durations() {
    assert_eq!(gcd(100, 150), 50);
    assert_eq!(gcd(50, gcd(100, 150)), 50);
    let (speed, d) = speed_and_durations(&[100, 150, 50]);
    assert_eq!(speed, 20.0);
    assert_eq!(d, vec![2.0, 3.0, 1.0]);
    let (speed, d) = speed_and_durations(&[80]);
    assert_eq!((speed, d), (12.5, vec![1.0]));
    assert_eq!(fmt_float(12.5), "12.5");
    assert_eq!(fmt_float(1.0), "1.0");
    assert_eq!(fmt_float(20.0), "20.0");
}

#[test]
fn direction_expansion() {
    assert_eq!(expand_frames(0, 3, 0), [0, 1, 2, 3]);
    assert_eq!(expand_frames(0, 3, 1), [3, 2, 1, 0]);
    assert_eq!(expand_frames(0, 3, 2), [0, 1, 2, 3, 2, 1]);
    assert_eq!(expand_frames(0, 3, 3), [3, 2, 1, 0, 1, 2]);
    assert_eq!(expand_frames(2, 3, 2), [2, 3]);
    assert_eq!(expand_frames(5, 5, 2), [5]);
    assert_eq!(expand_frames(5, 5, 3), [5]);
}

#[test]
fn golden_two_tags() {
    let b = build(&two_tag(), &target(None), &Options::default()).unwrap();
    let expected = r#"[gd_resource type="SpriteFrames" format=3]

[ext_resource type="Texture2D" path="res://art/player.png" id="1_tex"]

[sub_resource type="AtlasTexture" id="at_idle_0"]
atlas = ExtResource("1_tex")
region = Rect2(0, 0, 4, 4)

[sub_resource type="AtlasTexture" id="at_idle_1"]
atlas = ExtResource("1_tex")
region = Rect2(4, 0, 4, 4)

[sub_resource type="AtlasTexture" id="at_walk_0"]
atlas = ExtResource("1_tex")
region = Rect2(0, 0, 4, 4)

[sub_resource type="AtlasTexture" id="at_walk_1"]
atlas = ExtResource("1_tex")
region = Rect2(4, 0, 4, 4)

[sub_resource type="AtlasTexture" id="at_walk_2"]
atlas = ExtResource("1_tex")
region = Rect2(0, 4, 4, 4)

[sub_resource type="AtlasTexture" id="at_walk_3"]
atlas = ExtResource("1_tex")
region = Rect2(4, 4, 4, 4)

[resource]
animations = [{
"frames": [{
"duration": 2.0,
"texture": SubResource("at_idle_0")
}, {
"duration": 3.0,
"texture": SubResource("at_idle_1")
}],
"loop": true,
"name": &"idle",
"speed": 20.0
}, {
"frames": [{
"duration": 2.0,
"texture": SubResource("at_walk_0")
}, {
"duration": 3.0,
"texture": SubResource("at_walk_1")
}, {
"duration": 1.0,
"texture": SubResource("at_walk_2")
}, {
"duration": 2.0,
"texture": SubResource("at_walk_3")
}, {
"duration": 1.0,
"texture": SubResource("at_walk_2")
}, {
"duration": 3.0,
"texture": SubResource("at_walk_1")
}],
"loop": false,
"name": &"walk",
"speed": 20.0
}]
"#;
    assert_eq!(b.tres, expected);
    assert_eq!(b.pages.len(), 1);
    assert_eq!(b.pages[0].0, "player.png");
    assert!(b.warnings[0].contains("Nearest"));
}

#[test]
fn no_tags_gives_default_animation() {
    let s = sprite(2, &[100, 100]);
    let b = build(&s, &target(None), &Options::default()).unwrap();
    assert!(b.tres.contains("\"name\": &\"default\""));
    assert!(b.tres.contains("\"speed\": 10.0"));
    assert!(b.tres.contains("at_default_1"));
}

#[test]
fn identical_frames_share_a_region() {
    let mut s = sprite(3, &[100, 100, 100]);
    // Make frame 2 identical to frame 0.
    let p = s.cels[0][0].clone();
    s.cels[2][0] = p;
    s.tags = vec![tag("a", 0, 2, 0, 0)];
    let b = build(&s, &target(None), &Options::default()).unwrap();
    let regions: Vec<&str> = b.tres.lines().filter(|l| l.starts_with("region")).collect();
    assert_eq!(regions.len(), 3);
    assert_eq!(regions[0], regions[2]);
    assert_ne!(regions[0], regions[1]);
}

#[test]
fn trim_sets_margin() {
    let mut s = sprite(1, &[100]);
    s.tags = vec![tag("a", 0, 0, 0, 0)];
    let o = Options { trim: true, ..Default::default() };
    let b = build(&s, &target(None), &o).unwrap();
    // The red pixel is at (0, 1) of 4x4: region 1x1, margin offset (0,1), extra size (3,3).
    assert!(b.tres.contains("region = Rect2(0, 0, 1, 1)"), "{}", b.tres);
    assert!(b.tres.contains("margin = Rect2(0, 1, 3, 3)"), "{}", b.tres);
}

#[test]
fn extrude_and_padding_fill_border() {
    let mut s = sprite(1, &[100]);
    s.tags = vec![tag("a", 0, 0, 0, 0)];
    let o = Options { trim: true, extrude: true, padding: 1, ..Default::default() };
    let b = build(&s, &target(None), &o).unwrap();
    // cell = 1 + 2 extrude, origin at padding 1, image at 2.
    assert!(b.tres.contains("region = Rect2(2, 2, 1, 1)"), "{}", b.tres);
    let dec = png::Decoder::new(b.pages[0].1.as_slice());
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    assert_eq!((info.width, info.height), (5, 5));
    let px = |x: usize, y: usize| &buf[(y * 5 + x) * 4..(y * 5 + x) * 4 + 4];
    assert_eq!(px(2, 2), [255, 0, 0, 255]);
    assert_eq!(px(1, 1), [255, 0, 0, 255]); // extruded corner
    assert_eq!(px(3, 2), [255, 0, 0, 255]);
    assert_eq!(px(0, 0), [0, 0, 0, 0]); // padding stays clear
}

#[test]
fn splits_into_pages() {
    let mut s = Sprite::new(8, 8);
    for _ in 1..6 {
        s.add_frame(s.frame_count() - 1, false);
    }
    for f in 0..6 {
        let c = s.cel_mut(0, f).unwrap();
        for p in c.pixels.chunks_exact_mut(4) {
            p.copy_from_slice(&[f as u8 * 40 + 10, 0, 0, 255]);
        }
    }
    let o = Options { max_texture: 16, ..Default::default() }; // 4 frames of 8x8 per page
    let b = build(&s, &target(None), &o).unwrap();
    assert_eq!(b.pages.len(), 2);
    assert_eq!(b.pages[0].0, "player_0.png");
    assert_eq!(b.pages[1].0, "player_1.png");
    assert!(b.tres.contains("path=\"res://art/player_0.png\" id=\"1_tex\""));
    assert!(b.tres.contains("path=\"res://art/player_1.png\" id=\"2_tex\""));
    assert!(b.tres.contains("ExtResource(\"2_tex\")"));
    // A frame bigger than the limit is an error.
    let o = Options { max_texture: 4, ..Default::default() };
    assert!(build(&s, &target(None), &o).is_err());
}

#[test]
fn uid_is_preserved() {
    let prev = "[gd_resource type=\"SpriteFrames\" format=3 uid=\"uid://abc123\"]\n\n[ext_resource type=\"Texture2D\" uid=\"uid://tex456\" path=\"res://art/player.png\" id=\"1_tex\"]\n";
    let b = build(&two_tag(), &target(Some(prev)), &Options::default()).unwrap();
    assert!(b.tres.starts_with("[gd_resource type=\"SpriteFrames\" format=3 uid=\"uid://abc123\"]"));
    assert!(b
        .tres
        .contains("[ext_resource type=\"Texture2D\" uid=\"uid://tex456\" path=\"res://art/player.png\" id=\"1_tex\"]"));
    assert_eq!(header_uid(&b.tres).as_deref(), Some("uid://abc123"));
}

pub(super) fn temp_project(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pixmith-godot-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("project.godot"), "config_version=5\n").unwrap();
    dir
}

#[test]
fn refuses_paths_outside_root_and_non_projects() {
    let root = temp_project("outside");
    let s = two_tag();
    let o = Options::default();
    assert!(export_sprite_frames(&s, &root, &root.join("../evil.tres"), &o).is_err());
    assert!(export_sprite_frames(&s, &root, &root.join("art/../../evil.tres"), &o).is_err());
    assert!(export_sprite_frames(&s, &root, Path::new("/tmp/evil.tres"), &o).is_err());
    assert!(export_sprite_frames(&s, &root, &root.join("a.png"), &o).is_err());
    let not_project = std::env::temp_dir();
    assert!(export_sprite_frames(&s, &not_project, &root.join("a.tres"), &o).is_err());
    assert!(!root.parent().unwrap().join("evil.tres").exists());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn writes_files_deterministically_and_keeps_uid() {
    let root = temp_project("write");
    let s = two_tag();
    let o = Options { padding: 1, extrude: true, trim: true, ..Default::default() };
    let tres = root.join("art/player.tres");
    let warnings = export_sprite_frames(&s, &root, &tres, &o).unwrap();
    assert!(warnings[0].contains("Nearest"));
    let (a_tres, a_png) = (std::fs::read(&tres).unwrap(), std::fs::read(root.join("art/player.png")).unwrap());
    export_sprite_frames(&s, &root, &tres, &o).unwrap();
    assert_eq!(a_tres, std::fs::read(&tres).unwrap());
    assert_eq!(a_png, std::fs::read(root.join("art/player.png")).unwrap());
    assert!(String::from_utf8(a_tres.clone()).unwrap().contains("path=\"res://art/player.png\""));

    // Godot adds a uid on first import/save; re-export must keep it.
    let with_uid = String::from_utf8(a_tres).unwrap().replacen("format=3", "format=3 uid=\"uid://keepme\"", 1);
    std::fs::write(&tres, with_uid).unwrap();
    export_sprite_frames(&s, &root, &tres, &o).unwrap();
    assert!(std::fs::read_to_string(&tres).unwrap().contains("format=3 uid=\"uid://keepme\"]"));
    std::fs::remove_dir_all(&root).unwrap();
}
