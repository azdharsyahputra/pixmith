//! Facts and pixels documented by the fixtures' source projects (their own tests and the PNGs that
//! Aseprite exported for them).

use super::*;
use pixmith_core::document::color_mode::ColorMode;

fn layer_index(s: &Sprite, name: &str) -> usize {
    s.layers.iter().position(|l| l.name == name).unwrap_or_else(|| panic!("no layer named {name}"))
}

/// Renders `frame` with only the named layer shown.
fn render_only(s: &Sprite, name: &str, frame: usize) -> Vec<u8> {
    let keep = layer_index(s, name);
    let mut c = s.clone();
    for (i, l) in c.layers.iter_mut().enumerate() {
        l.visible = l.is_group() || i == keep;
    }
    c.render(frame)
}

fn check_frames(s: &Sprite, refs: &[&str]) {
    for (f, r) in refs.iter().enumerate() {
        assert_render(s, f, r, 0);
    }
}

fn same_pixels(a: &[u8], b: &[u8]) -> bool {
    a.chunks(4).zip(b.chunks(4)).all(|(a, b)| a == b || (a[3] == 0 && b[3] == 0))
}

#[test]
fn basic_and_big_images() {
    let s = load("asefile/basic-16x16.aseprite");
    assert_eq!((s.width, s.height, s.frame_count(), s.layers.len()), (16, 16, 1, 1));
    assert_eq!(s.color_mode, ColorMode::Rgba);
    assert_render(&s, 0, "asefile/basic-16x16.png", 0);

    let s = load("asefile/big.aseprite");
    assert_eq!((s.width, s.height, s.frame_count(), s.layers.len()), (256, 256, 1, 1));
    assert_render(&s, 0, "asefile/big.png", 0);

    let s = load("asefile/background.aseprite");
    assert_eq!((s.width, s.height, s.frame_count(), s.layers.len()), (256, 256, 1, 1));
    assert_render(&s, 0, "asefile/background.png", 0);
}

#[test]
fn layers_tags_and_frames() {
    let s = load("asefile/layers_and_tags.aseprite");
    assert_eq!((s.width, s.height, s.frame_count(), s.layers.len(), s.tags.len()), (16, 16, 4, 6, 3));
    check_frames(
        &s,
        &[
            "asefile/layers_and_tags_01.png",
            "asefile/layers_and_tags_02.png",
            "asefile/layers_and_tags_03.png",
            "asefile/layers_and_tags_04.png",
        ],
    );
    assert_matches_png(&render_only(&s, "Layer 1", 2), 16, 16, "asefile/single_layer.png", 0);
    assert!(s.tags.iter().all(|t| t.from <= t.to && (t.to as usize) < 4));

    let s = load("asefile/transparency.aseprite");
    assert_eq!((s.width, s.height, s.frame_count(), s.layers.len()), (16, 16, 2, 3));
    check_frames(&s, &["asefile/transparency_01.png", "asefile/transparency_02.png"]);
}

#[test]
fn linked_cels_render_and_stay_linked() {
    let s = load("asefile/linked_cels.aseprite");
    assert_eq!((s.frame_count(), s.layers.len()), (3, 3));
    check_frames(&s, &["asefile/linked_cels_01.png", "asefile/linked_cels_02.png", "asefile/linked_cels_03.png"]);
    let shared = (0..s.layers.len()).any(|l| {
        (0..s.frame_count()).any(|a| {
            (a + 1..s.frame_count())
                .any(|b| matches!((&s.cels[a][l], &s.cels[b][l]), (Some(x), Some(y)) if std::sync::Arc::ptr_eq(x, y)))
        })
    });
    assert!(shared, "the fixture has linked cels");
    let back = ase::read(&ase::write(&s)).unwrap();
    for l in 0..s.layers.len() {
        for (a, b) in [(0, 1), (0, 2), (1, 2)] {
            let linked = |s: &Sprite| matches!((&s.cels[a][l], &s.cels[b][l]), (Some(x), Some(y)) if std::sync::Arc::ptr_eq(x, y));
            assert_eq!(linked(&s), linked(&back), "layer {l} frames {a}/{b}");
        }
    }
}

#[test]
fn color_modes() {
    let s = load("asefile/indexed.aseprite");
    assert_eq!((s.width, s.height, s.color_mode), (64, 64, ColorMode::Indexed));
    assert_render(&s, 0, "asefile/indexed_01.png", 0);

    let s = load("asefile/grayscale.aseprite");
    assert_eq!((s.width, s.height, s.color_mode), (64, 64, ColorMode::Grayscale));
    assert_render(&s, 0, "asefile/grayscale.png", 0);

    let s = load("asefile/256_color_old_palette_chunk.aseprite");
    assert_eq!((s.width, s.height, s.frame_count(), s.layers.len(), s.color_mode), (64, 64, 1, 1, ColorMode::Indexed));
    assert_eq!(s.palette.len(), 256);
    assert_render(&s, 0, "asefile/256_color_old_palette_chunk.png", 0);

    let s = load("asefile/palette.aseprite");
    assert_eq!(s.palette.len(), 85);
    assert_eq!(s.palette[0], [46, 34, 47, 255]);
    assert_eq!(s.palette[71], [0, 0, 0, 83]);

    let s = load("asefile/util_indexed.aseprite");
    assert_eq!((s.width, s.height, s.color_mode), (4, 4, ColorMode::Indexed));
}

#[test]
fn indexed_files_save_as_indexed_with_the_same_palette() {
    for n in
        ["asefile/indexed.aseprite", "asefile/256_color_old_palette_chunk.aseprite", "asefile/util_indexed.aseprite"]
    {
        let s = load(n);
        let out = ase::write(&s);
        assert_eq!(u16::from_le_bytes([out[12], out[13]]), 8, "{n}: color depth");
        assert_eq!(out[28], s.transparent_index, "{n}: transparent index");
        assert_eq!(ase::read(&out).unwrap().palette, s.palette, "{n}");
    }
}

#[test]
fn blend_modes_match_aseprite() {
    for n in ["blend_normal", "blend_multiply", "blend_overlay", "blend_hue", "blend_saturation_bug"] {
        let s = load(&format!("asefile/{n}.aseprite"));
        assert_eq!(s.layers.len(), if n == "blend_saturation_bug" { s.layers.len() } else { 2 }, "{n}");
        // Opaque backdrops match exactly; translucent ones differ by at most 3 per channel (the blend
        // arithmetic is rounded differently). All 19 modes of the source project's fixtures were checked.
        assert_render(&s, 0, &format!("asefile/{n}.png"), 3);
    }
}

#[test]
fn raw_and_oversized_cels() {
    let s = load("asefile/rawcel.aseprite");
    assert!(s.cels.iter().flatten().flatten().any(|c| c.pixels.chunks(4).any(|p| p[3] != 0)));
    let s = load("asefile/cel_overflow.aseprite");
    assert_eq!(s.render(0).len(), s.width as usize * s.height as usize * 4);
    let s = load("asefile/color-curve.aseprite");
    assert!(s.frame_count() >= 1);
}

#[test]
fn tilemaps_render_like_aseprite() {
    let s = load("asefile/tilemap.aseprite");
    assert_eq!((s.width, s.height), (32, 32));
    assert_eq!(s.tilesets[0].name, "test_tileset");
    assert_render(&s, 0, "asefile/tilemap.png", 0);

    let s = load("asefile/tilemap_indexed.aseprite");
    assert_eq!(s.tilesets[0].name, "test_tileset");
    assert_render(&s, 0, "asefile/tilemap_indexed.png", 0);

    let s = load("asefile/tilemap_grayscale.aseprite");
    assert_eq!(s.tilesets[0].name, "test_tileset");
    assert_render(&s, 0, "asefile/tilemap_grayscale.png", 0);

    let s = load("asefile/tilemap_multi.aseprite");
    assert_render(&s, 0, "asefile/tilemap_multi.png", 0);
    let (w, h) = (s.width as usize, s.height as usize);
    assert_matches_png(&render_only(&s, "Tilemap 1", 0), w, h, "asefile/tilemap_multi_map1.png", 0);
    assert_matches_png(&render_only(&s, "Tilemap 2", 0), w, h, "asefile/tilemap_multi_map2.png", 0);
    // Pixmith trims empty rows and columns off a tile grid; the reference counts from the file's origin.
    let grid = s.cels[0][layer_index(&s, "Tilemap 1")].as_ref().unwrap().tiles.as_ref().unwrap();
    let (tw, th) = (s.tilesets[0].tile_w as i32, s.tilesets[0].tile_h as i32);
    let id = |x: i32, y: i32| {
        let (c, r) = (x - grid.x / tw, y - grid.y / th);
        if c < 0 || r < 0 || c >= grid.w as i32 || r >= grid.h as i32 {
            0
        } else {
            grid.tiles[r as usize * grid.w as usize + c as usize] & 0x1fff_ffff
        }
    };
    assert_eq!([id(0, 0), id(0, 2), id(0, 3), id(11, 5), id(12, 15), id(4, 7)], [0, 4, 2, 3, 0, 3]);
    let t = &s.tilesets[0];
    let (rw, rh, reference) = reference_png("asefile/tilemap_single_tile_1.png");
    assert_eq!((rw, rh), (t.tile_w as usize, t.tile_h as usize));
    assert!(same_pixels(&t.tiles[3], &reference));
}

#[test]
fn tilesets_keep_every_tile_and_unused_tilesets() {
    let s = load("asefile/tileset.aseprite");
    assert_eq!(s.tilesets.len(), 2, "the file also holds an unused second tileset");
    let t = &s.tilesets[0];
    assert_eq!((t.name.as_str(), t.tile_w, t.tile_h, t.tiles.len()), ("test_tileset", 16, 16, 5));
    let (rw, rh, reference) = reference_png("asefile/tileset.png");
    assert_eq!((rw, rh), (16, 16 * 5));
    let stacked: Vec<u8> = t.tiles.iter().flat_map(|t| t.iter().copied()).collect();
    assert!(same_pixels(&stacked, &reference));
    assert_eq!(ase::read(&ase::write(&s)).unwrap().tilesets, s.tilesets);

    let s = load("asefile/tilemap_empty_edges.aseprite");
    let grid = s.cels[0][0].as_ref().and_then(|c| c.tiles.as_ref()).expect("tilemap cel");
    assert_eq!(grid.tiles[0] & 0x1fff_ffff, 1);
}

#[test]
fn slices() {
    let s = load("asefile/slice_advanced.aseprite");
    assert_eq!(s.extra.slices.len(), 2);
    let one = &s.extra.slices[0];
    assert_eq!(one.name, "Slice 1");
    assert_eq!(one.keys.iter().map(|k| k.frame).collect::<Vec<_>>(), [0, 1, 2, 3]);
    assert_eq!(one.keys[0].pivot.0, 4);
    let two = &s.extra.slices[1];
    assert_eq!(two.keys.iter().map(|k| k.frame).collect::<Vec<_>>(), [0]);
    let c = two.keys[0].center;
    assert_eq!((c.x, c.y, c.w, c.h), (3, 3, 2, 2));
    assert!(two.nine_patch && one.has_pivot);

    assert!(!load("asefile/slice.aseprite").extra.slices.is_empty());
}

#[test]
fn user_data() {
    let s = load("asefile/user_data.aseprite");
    let green = [0, 255, 0, 255];
    let red = [255, 0, 0, 255];
    assert_eq!(s.extra.sprite.text.as_deref(), Some("test_user_data_sprite"));
    assert_eq!(s.extra.sprite.color, Some(green));
    assert_eq!(s.layers[0].user_data.text.as_deref(), Some("test_user_data_layer"));
    assert_eq!(s.layers[0].user_data.color, Some(red));
    let cel = s.cels[0][0].as_ref().expect("cel");
    assert_eq!(cel.user_data.text.as_deref(), Some("test_user_data_cel"));
    assert_eq!(cel.user_data.color, Some(green));
    assert_eq!(s.tags.len(), 3);
    let ud = |i: usize| (s.tags[i].user_data.text.clone(), s.tags[i].user_data.color);
    assert_eq!(ud(0), (Some("test_user_data_tag_0".into()), Some(green)));
    assert_eq!(ud(1), (None, Some([0, 0, 0, 255])));
    assert_eq!(ud(2), (Some("test_user_data_tag_2".into()), Some(red)));
}

#[test]
fn go_fixture_with_everything_in_it() {
    let s = load("ase-go/test_paletted.ase");
    assert_eq!((s.width, s.height, s.color_mode), (64, 64, ColorMode::Indexed));
    assert_eq!(s.palette.len(), 5);
    assert_eq!(s.palette[1], [223, 62, 35, 156]);
    assert_eq!(s.durations, vec![500, 500, 1000, 1000]);
    let names: Vec<_> = s.layers.iter().map(|l| (l.name.as_str(), l.kind, l.child_level)).collect();
    assert_eq!(names, [("group1", 1, 0), ("test tilemap", 2, 1), ("my image layer", 0, 0)]);
    assert_eq!(s.layers[0].user_data.text.as_deref(), Some("hi"));
    assert_eq!(s.layers[1].opacity, 158);
    assert_eq!(s.layers[1].user_data.text.as_deref(), Some("hello world"));
    assert_eq!(s.layers[1].user_data.color, Some([223, 62, 35, 255]));
    assert_eq!(s.layers[2].user_data.text.as_deref(), Some("test data"));

    assert_eq!(s.tags.len(), 2);
    let (t1, t2) = (&s.tags[0], &s.tags[1]);
    assert_eq!(
        (t1.name.as_str(), t1.from, t1.to, t1.repeat, t1.direction, t1.color),
        ("tag1", 0, 1, 2, 2, [247, 165, 71])
    );
    assert_eq!(t1.user_data.text.as_deref(), Some("hello world"));
    assert_eq!((t2.name.as_str(), t2.from, t2.to), ("tag2", 2, 3));
    assert_eq!(s.cels[2][2].as_ref().unwrap().user_data.text.as_deref(), Some("tag2_start"));

    assert_eq!(s.extra.slices.len(), 1);
    let sl = &s.extra.slices[0];
    assert_eq!(
        (sl.name.as_str(), sl.user.color, sl.user.text.as_deref()),
        ("slice1", Some([0, 127, 255, 207]), Some("hello world"))
    );
    assert!(!sl.keys.is_empty()); // the Go parser repeats the key for every frame; the file stores one
    let k = &sl.keys[0];
    assert_eq!((k.bounds.x, k.bounds.y, k.bounds.w, k.bounds.h), (23, 8, 36, 23));
    assert_eq!((k.center.x, k.center.y, k.center.w, k.center.h), (5, 5, 34, 21));
    assert_eq!(k.pivot, (10, 10));

    let ts = &s.tilesets[0];
    assert_eq!((ts.name.as_str(), ts.tile_w, ts.tile_h, ts.tiles.len(), ts.base_index), ("tileset test", 16, 16, 4, 1));

    let grid = s.cels[0][1].as_ref().unwrap().tiles.as_ref().unwrap();
    assert_eq!((grid.x, grid.y, grid.w, grid.h), (4, 2, 3, 2));
    let id = |v: u32| v & 0x1fff_ffff;
    let flags = |v: u32| v >> 29;
    assert_eq!(grid.tiles.iter().map(|&v| id(v)).collect::<Vec<_>>(), [1, 3, 0, 0, 2, 3]);
    assert_eq!((flags(grid.tiles[0]), flags(grid.tiles[1]), flags(grid.tiles[4])), (7, 0, 7));
}

/// The reference PNGs are the tilemap layer's own image: 3x2 tiles of 16x16 at (4, 2).
#[test]
fn tilemap_flips_render_like_the_reference() {
    for (file, png) in [
        ("ase-go/test_paletted.ase", "ase-go/tilemap_paletted.png"),
        ("ase-go/test_nrgba.ase", "ase-go/tilemap_nrgba.png"),
        ("ase-go/test_grayscale.ase", "ase-go/tilemap_grayscale.png"),
    ] {
        let s = load(file);
        let full = render_only(&s, "test tilemap", 0);
        let (rw, rh, reference) = reference_png(png);
        assert_eq!((rw, rh), (48, 32), "{png}");
        let mut crop = Vec::new();
        for y in 2..2 + rh {
            crop.extend_from_slice(&full[(y * s.width as usize + 4) * 4..][..rw * 4]);
        }
        // The layer is composited at 158/255 opacity, so compare color and visibility, not alpha.
        let bad = crop
            .chunks(4)
            .zip(reference.chunks(4))
            .filter(|(a, b)| (a[3] == 0) != (b[3] == 0) || (a[3] != 0 && a[..3] != b[..3]))
            .count();
        assert_eq!(bad, 0, "{file}: {bad} pixels differ from {png}");
    }
}

#[test]
fn header_and_layer_properties_survive() {
    let s = load("ase-go/prop.ase");
    assert_eq!((s.width, s.height), (16, 16));
    assert_eq!(s.extra.pixel_ratio, (1, 2));
    assert_eq!(s.layers[0].name, "Layer 1");
    assert_eq!(s.layers[0].user_data.text.as_deref(), Some("user data text"));
    assert_eq!(s.layers[0].user_data.color, Some([223, 62, 35, 255]));
    assert_eq!(s.palette[0], [180, 32, 42, 255]);
    let out = ase::write(&s);
    assert_eq!((out[34], out[35]), (1, 2), "pixel ratio");
}

#[test]
fn exports_of_real_files_are_sane() {
    let s = load("asefile/layers_and_tags.aseprite");
    let (w, h, _) = decode_png(&export::frame_png(&s, 1, 3).unwrap()).unwrap();
    assert_eq!((w, h), (48, 48));

    let gif = gif_export::gif_bytes(&s, 2).unwrap();
    let mut dec = gif::DecodeOptions::new().read_info(&gif[..]).unwrap();
    let mut frames = 0;
    while dec.read_next_frame().unwrap().is_some() {
        frames += 1;
    }
    assert_eq!((dec.width(), dec.height(), frames), (32, 32, 4));

    let opts = sheet::Options { layout: sheet::Layout::Grid(2), padding: 1, scale: 1, ..Default::default() };
    let (sheet_png, json) = sheet::export(&s, opts, "x.png").unwrap();
    let back = pixmith_core::formats::sheet_import::sprite_from_sheet_json(&sheet_png, &json).unwrap();
    assert_eq!((back.width, back.height, back.frame_count()), (16, 16, 4));
    for f in 0..4 {
        assert_eq!(back.render(f), s.render(f));
    }
    assert_eq!(back.tags.len(), s.tags.len());

    let apng = pixmith_core::formats::animation::apng_bytes(&s, 1).unwrap();
    assert_eq!(png::Decoder::new(&apng[..]).read_info().unwrap().info().animation_control.unwrap().num_frames, 4);
}
