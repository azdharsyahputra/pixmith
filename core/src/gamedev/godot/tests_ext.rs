//! Tests for the extended export options (layers, scale, grid, per-animation PNGs, .tscn, settings).

use super::sprite_frames::*;
use super::tests::{sprite, tag, target, temp_project, two_tag};
use super::*;
use crate::document::Sprite;

fn two_layers() -> Sprite {
    let mut s = sprite(2, &[100, 100]);
    s.layers[0].name = "body".into();
    let top = s.add_layer(0);
    s.layers[top].name = "hat".into();
    for f in 0..2 {
        let c = s.cel_mut(top, f).unwrap();
        let i = (15 - f) * 4;
        c.pixels[i..i + 4].copy_from_slice(&[0, 0, 255, 255]);
    }
    s
}

#[test]
fn float_edge_cases() {
    let (speed, d) = speed_and_durations(&[33, 66, 33]);
    assert_eq!(fmt_float(speed), "30.303030303030305");
    assert_eq!(d, vec![1.0, 2.0, 1.0]);
    assert!((2.0 / speed * 1000.0 - 66.0).abs() < 1e-9);
    let (speed, d) = speed_and_durations(&[7, 13]);
    assert_eq!((speed, d), (1000.0, vec![7.0, 13.0]));
    let (speed, d) = speed_and_durations(&[0, 100]);
    assert_eq!(speed, 1000.0);
    assert_eq!(d, vec![1.0, 100.0]);
    assert_eq!(fmt_float(1000.0 / 3.0), "333.3333333333333");
}

#[test]
fn scale_multiplies_regions() {
    let mut s = sprite(1, &[100]);
    s.tags = vec![tag("a", 0, 0, 0, 0)];
    let o = Options { scale: 2, trim: true, ..Default::default() };
    let b = build(&s, &target(None), &o).unwrap();
    assert!(b.tres.contains("region = Rect2(0, 0, 2, 2)"), "{}", b.tres);
    assert!(b.tres.contains("margin = Rect2(0, 2, 6, 6)"), "{}", b.tres);
}

#[test]
fn grid_packing_uses_uniform_cells() {
    let mut s = sprite(4, &[100; 4]);
    s.tags = vec![tag("a", 0, 3, 0, 0)];
    let o = Options { packing: Packing::Grid, padding: 1, ..Default::default() };
    let b = build(&s, &target(None), &o).unwrap();
    let regions: Vec<&str> = b.tres.lines().filter(|l| l.starts_with("region")).collect();
    assert_eq!(
        regions,
        [
            "region = Rect2(1, 1, 4, 4)",
            "region = Rect2(6, 1, 4, 4)",
            "region = Rect2(1, 6, 4, 4)",
            "region = Rect2(6, 6, 4, 4)"
        ]
    );
}

#[test]
fn per_animation_png_and_tag_filters() {
    let s = two_tag();
    let o = Options { per_animation_png: true, ..Default::default() };
    let b = build(&s, &target(None), &o).unwrap();
    let names: Vec<&str> = b.pages.iter().map(|p| p.0.as_str()).collect();
    assert_eq!(names, ["player_idle.png", "player_walk.png"]);
    assert!(b.tres.contains("path=\"res://art/player_idle.png\" id=\"1_tex\""));
    assert!(b.tres.contains("path=\"res://art/player_walk.png\" id=\"2_tex\""));
    assert!(b.tres.contains("[sub_resource type=\"AtlasTexture\" id=\"at_walk_0\"]\natlas = ExtResource(\"2_tex\")"));

    let o = Options { tags_exclude: vec!["idle".into()], ..Default::default() };
    let b = build(&s, &target(None), &o).unwrap();
    assert!(!b.tres.contains("at_idle_0") && b.tres.contains("at_walk_0"));
    let o = Options { tags_include: vec!["idle".into()], ..Default::default() };
    let b = build(&s, &target(None), &o).unwrap();
    assert!(b.tres.contains("at_idle_0") && !b.tres.contains("at_walk_0"));
    let o = Options { tags_include: vec!["nope".into()], ..Default::default() };
    assert!(build(&s, &target(None), &o).is_err());
}

#[test]
fn unicode_and_special_names_are_quoted() {
    let mut s = sprite(2, &[100, 100]);
    s.tags = vec![tag("走る \"fast\"\\1", 0, 1, 0, 0), tag("走る \"fast\"\\1", 0, 1, 0, 0)];
    let b = build(&s, &Target { name: "Spieler Ü", res_dir: "res://art work/ñ", previous: None }, &Options::default())
        .unwrap();
    assert!(b.tres.contains("path=\"res://art work/ñ/Spieler Ü.png\""), "{}", b.tres);
    assert!(b.tres.contains("\"name\": &\"走る \\\"fast\\\"\\\\1\""), "{}", b.tres);
    assert!(b.tres.contains("\"name\": &\"走る \\\"fast\\\"\\\\1_2\""), "{}", b.tres);
    assert!(b.tres.contains("id=\"at_____fast__1_0\""), "{}", b.tres);
    assert!(b.tres.contains("id=\"at_____fast__1_2_0\""), "{}", b.tres);
    assert_eq!(b.pages[0].0, "Spieler Ü.png");
}

#[test]
fn unicode_project_paths_work() {
    let root = temp_project("uni ü");
    let tres = root.join("art ñ/Spieler 走.tres");
    export_sprite_frames(&two_tag(), &root, &tres, &Options::default()).unwrap();
    let text = std::fs::read_to_string(&tres).unwrap();
    assert!(text.contains("path=\"res://art ñ/Spieler 走.png\""), "{text}");
    assert!(root.join("art ñ/Spieler 走.png").exists());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn layer_modes() {
    let root = temp_project("layers");
    let s = two_layers();
    let tres = root.join("art/hero.tres");

    let p = plan(&s, &root, &tres, &Options::default()).unwrap();
    assert_eq!(p.files.len(), 2);

    // Hidden layers are not exported.
    let mut hidden = s.clone();
    hidden.layers[1].visible = false;
    let a = plan(&hidden, &root, &tres, &Options::default()).unwrap();
    let b = plan(&s, &root, &tres, &Options::default()).unwrap();
    assert_ne!(a.files[0].data, b.files[0].data);

    // Each layer: own files, named by template.
    let o = Options { layers: LayerMode::Each, ..Default::default() };
    let p = plan(&s, &root, &tres, &o).unwrap();
    let names: Vec<String> =
        p.files.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["hero_body.png", "hero_body.tres", "hero_hat.png", "hero_hat.tres"]);
    let tres_text = String::from_utf8(p.files[3].data.clone()).unwrap();
    assert!(tres_text.contains("res://art/hero_hat.png"));

    let o = Options { layers: LayerMode::Each, name_template: "{layer}-{name}".into(), ..Default::default() };
    let p = plan(&s, &root, &tres, &o).unwrap();
    assert!(p.files[0].path.ends_with("art/body-hero.png"));

    // Selected layers show hidden ones too, and only those.
    let o = Options { layers: LayerMode::Selected(vec!["hat".into()]), ..Default::default() };
    let sel = plan(&hidden, &root, &tres, &o).unwrap();
    let each = plan(&s, &root, &tres, &Options { layers: LayerMode::Each, ..Default::default() }).unwrap();
    assert_eq!(sel.files[0].data, each.files[2].data);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn tscn_variants() {
    let root = temp_project("tscn");
    let s = two_tag();
    let tres = root.join("art/player.tres");
    let o = Options { emit_tscn: true, ..Default::default() };
    export_sprite_frames(&s, &root, &tres, &o).unwrap();
    let scene = std::fs::read_to_string(root.join("art/player.tscn")).unwrap();
    assert_eq!(
        scene,
        "[gd_scene format=3]\n\n\
         [ext_resource type=\"SpriteFrames\" path=\"res://art/player.tres\" id=\"1_frames\"]\n\n\
         [node name=\"Player\" type=\"AnimatedSprite2D\"]\n\
         texture_filter = 1\n\
         sprite_frames = ExtResource(\"1_frames\")\n\
         animation = &\"idle\"\n\
         autoplay = \"idle\"\n"
    );
    // An existing scene is left alone unless overwrite is requested.
    std::fs::write(
        root.join("art/player.tscn"),
        "[gd_scene format=3 uid=\"uid://scene1\"]\n[node name=\"Mine\" type=\"Node2D\"]\n",
    )
    .unwrap();
    let w = export_sprite_frames(&s, &root, &tres, &o).unwrap();
    assert!(w.iter().any(|w| w.contains("player.tscn already exists")));
    assert!(std::fs::read_to_string(root.join("art/player.tscn")).unwrap().contains("Mine"));
    let o2 = Options { overwrite_tscn: true, ..o.clone() };
    export_sprite_frames(&s, &root, &tres, &o2).unwrap();
    let scene = std::fs::read_to_string(root.join("art/player.tscn")).unwrap();
    assert!(scene.starts_with("[gd_scene format=3 uid=\"uid://scene1\"]"));
    // The SpriteFrames uid is passed on to the scene.
    let t = std::fs::read_to_string(&tres).unwrap().replacen("format=3", "format=3 uid=\"uid://frames1\"", 1);
    std::fs::write(&tres, t).unwrap();
    export_sprite_frames(&s, &root, &tres, &o2).unwrap();
    let scene = std::fs::read_to_string(root.join("art/player.tscn")).unwrap();
    assert!(scene.contains("[ext_resource type=\"SpriteFrames\" uid=\"uid://frames1\" path=\"res://art/player.tres\""));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn trim_golden_and_two_runs_match() {
    let mut s = sprite(2, &[100, 100]);
    s.tags = vec![tag("a", 0, 1, 0, 0)];
    let o = Options { trim: true, padding: 2, ..Default::default() };
    let a = build(&s, &target(None), &o).unwrap();
    let b = build(&s, &target(None), &o).unwrap();
    assert_eq!(a.tres, b.tres);
    assert_eq!(a.pages, b.pages);
    let expected = "[sub_resource type=\"AtlasTexture\" id=\"at_a_0\"]\natlas = ExtResource(\"1_tex\")\nregion = Rect2(2, 2, 1, 1)\nmargin = Rect2(0, 1, 3, 3)\n\n[sub_resource type=\"AtlasTexture\" id=\"at_a_1\"]\natlas = ExtResource(\"1_tex\")\nregion = Rect2(2, 2, 1, 1)\nmargin = Rect2(1, 1, 3, 3)\n";
    assert!(a.tres.contains(expected), "{}", a.tres);
}

#[test]
fn reimport_and_stale_warnings() {
    let root = temp_project("reimport");
    let s = two_tag();
    let tres = root.join("art/player.tres");
    let w = export_sprite_frames(&s, &root, &tres, &Options::default()).unwrap();
    assert!(w.iter().any(|w| w.contains("not been opened in Godot")));
    std::fs::create_dir_all(root.join(".godot")).unwrap();
    std::fs::write(root.join("art/player.png.import"), "[remap]\n").unwrap();
    let o = Options { trim: true, ..Default::default() };
    let w = export_sprite_frames(&s, &root, &tres, &o).unwrap();
    assert!(w.iter().any(|w| w.contains("player.png changed") && w.contains("reimports")), "{w:?}");
    // Unrelated files are never touched or deleted.
    std::fs::write(root.join("art/other.png"), b"x").unwrap();
    let o = Options { max_texture: 4, ..Default::default() };
    let w = export_sprite_frames(&s, &root, &tres, &o).unwrap();
    assert!(w.iter().any(|w| w.contains("player.png is left over")), "{w:?}");
    assert_eq!(std::fs::read(root.join("art/other.png")).unwrap(), b"x");
    assert!(root.join("art/player.png").exists());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn mobile_warning_and_cap() {
    let s = two_tag();
    let b = build(&s, &target(None), &Options { max_texture: 8192, ..Default::default() }).unwrap();
    assert!(b.warnings.iter().any(|w| w.contains("mobile")));
    let b = build(&s, &target(None), &Options { max_texture: 99999, ..Default::default() }).unwrap();
    assert!(b.warnings.iter().any(|w| w.contains("limited to 16384")));
    let b = build(&s, &target(None), &Options { max_texture: 2048, ..Default::default() }).unwrap();
    assert!(!b.warnings.iter().any(|w| w.contains("mobile")));
}

#[test]
fn settings_roundtrip_and_storage() {
    use super::settings::{self, Settings};
    let st = Settings {
        root: "/p/game".into(),
        tres: "/p/game/art/p.tres".into(),
        options: Options {
            padding: 3,
            extrude: true,
            trim: true,
            max_texture: 8192,
            scale: 2,
            packing: Packing::Grid,
            per_animation_png: true,
            name_template: "{name}_x".into(),
            layers: LayerMode::Selected(vec!["a\"b".into(), "c".into()]),
            tags_include: vec!["x".into()],
            tags_exclude: vec!["y".into()],
            emit_tscn: true,
            overwrite_tscn: true,
        },
    };
    let j = st.to_json();
    assert_eq!(Settings::from_json(&j).unwrap().to_json(), j);
    assert!(Settings::from_json("{\"v\":99}").is_err());
    assert_eq!(Settings::from_json("{}").unwrap().options.max_texture, 4096);

    let mut s = sprite(1, &[100]);
    s.extra.sprite.text = Some("my note".into());
    settings::store(&mut s, &j);
    settings::store(&mut s, &j);
    assert_eq!(settings::load(&s).as_deref(), Some(j.as_str()));
    let text = s.extra.sprite.text.clone().unwrap();
    assert!(text.starts_with("my note\n") && text.matches("pixmith-godot-export").count() == 1);
    // Survives saving as .aseprite.
    let back = crate::formats::ase::read(&crate::formats::ase::write(&s)).unwrap();
    assert_eq!(settings::load(&back).as_deref(), Some(j.as_str()));
}

#[test]
fn ffi_export_preview_and_settings() {
    use std::ffi::{CStr, CString};
    let root = temp_project("ffi");
    let mut ed = crate::document::Editor::new(two_tag());
    let json = format!(
        "{{\"root\":{},\"tres\":{},\"tscn\":true}}",
        crate::formats::sheet::json_str(root.to_str().unwrap()),
        crate::formats::sheet::json_str(root.join("art/ffi.tres").to_str().unwrap())
    );
    let c = CString::new(json).unwrap();
    let png = CString::new(root.join("preview.png").to_str().unwrap()).unwrap();
    unsafe {
        let (mut text, mut files, mut warn) = (std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
        assert!(pixmith_godot_preview(&ed, c.as_ptr(), png.as_ptr(), &mut text, &mut files, &mut warn));
        assert!(CStr::from_ptr(text).to_str().unwrap().starts_with("[gd_resource"));
        assert_eq!(CStr::from_ptr(files).to_str().unwrap().lines().count(), 3);
        assert!(root.join("preview.png").exists());
        assert!(!root.join("art/ffi.tres").exists());
        for p in [text, files, warn] {
            crate::ffi::pixmith_string_free(p);
        }
        let mut warn = std::ptr::null_mut();
        assert!(pixmith_export_godot(&ed, c.as_ptr(), &mut warn));
        crate::ffi::pixmith_string_free(warn);
        assert!(root.join("art/ffi.tres").exists() && root.join("art/ffi.tscn").exists());
        assert!(pixmith_godot_settings_get(&ed).is_null());
        assert!(pixmith_godot_settings_set(&mut ed, c.as_ptr()));
        let got = pixmith_godot_settings_get(&ed);
        assert!(CStr::from_ptr(got).to_str().unwrap().contains("\"tscn\":true"));
        crate::ffi::pixmith_string_free(got);
        let bad = CString::new("{\"v\":9}").unwrap();
        assert!(!pixmith_godot_settings_set(&mut ed, bad.as_ptr()));
    }
    std::fs::remove_dir_all(&root).unwrap();
}
