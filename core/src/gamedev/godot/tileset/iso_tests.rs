//! Tests for the isometric Godot TileSet export.

use super::*;
use crate::document::tilemap::{add_iso_tilemap_layer, set_tile_pivot, stamp, stamp_cell};
use crate::gamedev::autotile;

fn project(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pixmith-godot-iso-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("project.godot"), "").unwrap();
    d
}

/// 64x32 canvas, 8x12 pictures on 8x4 diamonds, map origin (20, 4): tile 1 touches E and S, tile 2 only N.
fn iso_sprite() -> (Sprite, usize) {
    let mut s = Sprite::new(64, 32);
    let l = add_iso_tilemap_layer(&mut s, 0, 8, 12, 8, 4, (20, 4));
    assert!(autotile::set_terrain(&mut s, l, "Grass", [10, 200, 30, 255], autotile::Mode::Sides));
    for c in [[255, 0, 0, 255], [0, 0, 255, 255]] {
        let n = s.tilesets[0].add_blank() as usize;
        s.tilesets[0].tiles[n] = std::sync::Arc::new(c.repeat(8 * 12));
    }
    assert!(autotile::set_tile_mask(&mut s, l, 1, Some(2 | 4)));
    assert!(autotile::set_tile_mask(&mut s, l, 2, Some(1)));
    stamp_cell(&mut s, l, 0, 0, 0, 1);
    stamp_cell(&mut s, l, 0, 2, 1, 2 | tilemap::FLIP_X);
    (s, l)
}

fn files(b: &Built) -> Vec<(String, String)> {
    b.files
        .iter()
        .map(|(p, d)| (p.file_name().unwrap().to_string_lossy().into_owned(), String::from_utf8_lossy(d).into_owned()))
        .collect()
}

#[test]
fn isometric_peering_names_follow_the_cell_neighbours() {
    // Ring N, NE, E, SE, S, SW, W, NW (cell space) against Godot's CellNeighbor for TILE_SHAPE_ISOMETRIC /
    // DIAMOND_DOWN, checked with TileMapLayer.get_neighbor_cell in Godot 4.7.2:
    //   top_right_side (0,-1)  right_corner (1,-1)  bottom_right_side (1,0)  bottom_corner (1,1)
    //   bottom_left_side (0,1) left_corner (-1,1)   top_left_side (-1,0)     top_corner (-1,-1)
    let expect = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];
    assert_eq!(DIRS.map(|(x, y)| (x as i32, y as i32)), expect);
    assert_eq!(
        NAMES_ISO,
        [
            "top_right_side",
            "right_corner",
            "bottom_right_side",
            "bottom_corner",
            "bottom_left_side",
            "left_corner",
            "top_left_side",
            "top_corner"
        ]
    );
    // Sides are the even ring entries, corners the odd ones, and Godot writes the sixteen in enum order.
    assert_eq!(
        GODOT_ORDER_ISO.map(|d| NAMES_ISO[d]),
        [
            "right_corner",
            "bottom_right_side",
            "bottom_corner",
            "bottom_left_side",
            "left_corner",
            "top_left_side",
            "top_corner",
            "top_right_side"
        ]
    );
}

#[test]
fn the_tileset_resource_says_isometric_diamond_down_and_keeps_origins() {
    let (s, _) = iso_sprite();
    let root = project("tres");
    let b = export_sprite_with(&s, 0, &root, &root.join("map/iso.tres"), &Options::default(), true).unwrap();
    let f = files(&b);
    let tres = &f.iter().find(|(n, _)| n == "iso.tres").unwrap().1;
    let want = "\
[gd_resource type=\"TileSet\" format=3]

[ext_resource type=\"Texture2D\" path=\"res://map/iso.png\" id=\"1_tex\"]

[sub_resource type=\"TileSetAtlasSource\" id=\"src_0\"]
texture = ExtResource(\"1_tex\")
texture_region_size = Vector2i(8, 12)
0:0/0 = 0
0:0/0/texture_origin = Vector2i(0, 4)
0:0/0/terrain_set = 0
0:0/0/terrain = 0
0:0/0/terrains_peering_bit/bottom_right_side = 0
0:0/0/terrains_peering_bit/bottom_left_side = 0
1:0/0 = 0
1:0/0/texture_origin = Vector2i(0, 4)
1:0/0/terrain_set = 0
1:0/0/terrain = 0
1:0/0/terrains_peering_bit/top_right_side = 0

[resource]
tile_shape = 1
tile_layout = 5
tile_size = Vector2i(8, 4)
terrain_set_0/mode = 2
terrain_set_0/terrain_0/name = \"Grass\"
terrain_set_0/terrain_0/color = Color(0.0392, 0.7843, 0.1176, 1)
sources/0 = SubResource(\"src_0\")
";
    assert_eq!(tres, want);
    assert!(b.warnings.is_empty() || !b.warnings.iter().any(|w| w.contains("alternative")));
}

#[test]
fn per_tile_texture_origin_comes_from_the_pivot() {
    let (mut s, _) = iso_sprite();
    // Picture 8x12, pivot (4, 7): 1 above the picture's middle row -> origin (0, 1).
    assert!(set_tile_pivot(&mut s, 0, 2, Some((4, 7))));
    // A pivot left and below the middle.
    assert!(set_tile_pivot(&mut s, 0, 1, Some((2, 11))));
    let root = project("origin");
    let b = export_sprite_with(&s, 0, &root, &root.join("iso.tres"), &Options::default(), true).unwrap();
    let tres = &files(&b).iter().find(|(n, _)| n == "iso.tres").unwrap().1.clone();
    assert!(tres.contains("0:0/0/texture_origin = Vector2i(-2, 5)\n"), "{tres}");
    assert!(tres.contains("1:0/0/texture_origin = Vector2i(0, 1)\n"), "{tres}");
    // A picture centered on its diamond needs no origin line.
    let mut s2 = s.clone();
    set_tile_pivot(&mut s2, 0, 1, Some((4, 6)));
    let b = export_sprite_with(&s2, 0, &root, &root.join("iso.tres"), &Options::default(), true).unwrap();
    let t2 = &files(&b).iter().find(|(n, _)| n == "iso.tres").unwrap().1.clone();
    assert!(!t2.contains("0:0/0/texture_origin"), "{t2}");
}

#[test]
fn collision_follows_the_diamond_and_the_picture() {
    let (s, _) = iso_sprite();
    let root = project("collision");
    let opts = Options { collision: Collision::Full, ..Options::default() };
    let b = export_sprite_with(&s, 0, &root, &root.join("iso.tres"), &opts, true).unwrap();
    let tres = &files(&b).iter().find(|(n, _)| n == "iso.tres").unwrap().1.clone();
    assert!(
        tres.contains("0:0/0/physics_layer_0/polygon_0/points = PackedVector2Array(0, -2, 4, 0, 0, 2, -4, 0)\n"),
        "{tres}"
    );
    // Opaque pixels are in cell space: the picture's center sits 4 below the cell center (origin y = 4).
    let opts = Options { collision: Collision::Opaque, ..Options::default() };
    let b = export_sprite_with(&s, 0, &root, &root.join("iso.tres"), &opts, true).unwrap();
    let tres = &files(&b).iter().find(|(n, _)| n == "iso.tres").unwrap().1.clone();
    assert!(
        tres.contains("0:0/0/physics_layer_0/polygon_0/points = PackedVector2Array(-4, -10, 4, -10, 4, 2, -4, 2)\n"),
        "{tres}"
    );
}

#[test]
fn the_scene_positions_each_layer_at_its_offset() {
    let (mut s, l) = iso_sprite();
    let opts = Options { write_scene: true, ..Options::default() };
    let root = project("scene");
    let b = export_sprite_with(&s, 0, &root, &root.join("iso.tres"), &opts, true).unwrap();
    let tscn = &files(&b).iter().find(|(n, _)| n == "iso.tscn").unwrap().1.clone();
    assert!(tscn.contains("[node name=\"Tilemap 2\" type=\"TileMapLayer\" parent=\".\"]\nposition = Vector2(20, 4)\ny_sort_enabled = true\n"), "{tscn}");
    // Cell (0, 0): tile 0 (tileset tile 1) at atlas (0, 0); cell (2, 1): tile 1 at (1, 0) with flip_h.
    let want_bytes: Vec<u8> = [0u16]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .chain([[0, 0, 0, 0, 0, 0], [2, 1, 0, 1, 0, 1 << 12]].into_iter().flatten().flat_map(u16::to_le_bytes))
        .collect();
    let list = want_bytes.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ");
    assert!(tscn.contains(&format!("tile_map_data = PackedByteArray({list})")), "{tscn}");
    // A second layer at another elevation gets its own position.
    let up = add_iso_tilemap_layer(&mut s, l, 8, 12, 8, 4, (20, -8));
    assert!(tilemap::assign_tileset(&mut s, up, 0));
    stamp_cell(&mut s, up, 0, 1, 1, 1);
    let b = export_sprite_with(&s, 0, &root, &root.join("iso.tres"), &opts, true).unwrap();
    let tscn = &files(&b).iter().find(|(n, _)| n == "iso.tscn").unwrap().1.clone();
    assert!(tscn.contains("position = Vector2(20, -8)\ny_sort_enabled = true\n"), "{tscn}");
}

#[test]
fn square_tilesets_export_exactly_as_before() {
    let mut s = Sprite::new(16, 16);
    let l = tilemap::add_tilemap_layer(&mut s, 0, 8, 8);
    assert!(autotile::set_terrain(&mut s, l, "G", [255, 0, 0, 255], autotile::Mode::Blob));
    let n = s.tilesets[0].add_blank() as usize;
    s.tilesets[0].tiles[n] = std::sync::Arc::new([9, 9, 9, 255].repeat(64));
    stamp(&mut s, l, 0, 0, 0, 1);
    let root = project("square");
    let opts = Options { write_scene: true, ..Options::default() };
    let b = export_sprite_with(&s, 0, &root, &root.join("sq.tres"), &opts, true).unwrap();
    let f = files(&b);
    let tres = &f.iter().find(|(n, _)| n == "sq.tres").unwrap().1;
    assert!(tres.contains("[resource]\ntile_size = Vector2i(8, 8)\n"), "{tres}");
    assert!(!tres.contains("tile_shape") && !tres.contains("tile_layout") && !tres.contains("texture_origin"));
    let tscn = &f.iter().find(|(n, _)| n == "sq.tscn").map(|x| x.1.clone()).unwrap_or_default();
    assert!(!tscn.contains("y_sort_enabled") && !tscn.contains("position ="));
}

#[test]
fn flips_are_cell_flags_not_alternative_tiles_on_isometric_sets() {
    let (mut s, l) = iso_sprite();
    autotile::set_tile_sym(&mut s, l, 1, 3);
    let root = project("alt");
    let b = export_sprite_with(&s, 0, &root, &root.join("iso.tres"), &Options::default(), true).unwrap();
    let tres = &files(&b).iter().find(|(n, _)| n == "iso.tres").unwrap().1.clone();
    assert!(!tres.contains("flip_h") && !tres.contains("/1 = 0"), "{tres}");
}

#[test]
fn match_corners_export_uses_the_corner_names() {
    let mut s = Sprite::new(64, 32);
    let l = add_iso_tilemap_layer(&mut s, 0, 8, 4, 8, 4, (0, 0));
    assert!(autotile::set_terrain(&mut s, l, "Rock", [1, 2, 3, 255], autotile::Mode::Blob));
    let n = s.tilesets[0].add_blank() as usize;
    s.tilesets[0].tiles[n] = std::sync::Arc::new([9, 9, 9, 255].repeat(32));
    // All eight neighbours: sides and corners, the full set of bits.
    assert!(autotile::set_tile_mask(&mut s, l, 1, Some(255)));
    let root = project("corners");
    let b = export_sprite_with(&s, 0, &root, &root.join("iso.tres"), &Options::default(), true).unwrap();
    let tres = &files(&b).iter().find(|(n, _)| n == "iso.tres").unwrap().1.clone();
    for name in NAMES_ISO {
        assert!(tres.contains(&format!("0:0/0/terrains_peering_bit/{name} = 0\n")), "{name}");
    }
    // A plain tileset with a blob mask and no iso terrain data uses the mask path.
    assert!(tres.contains("terrain_set_0/mode = 0\n"));
}

/// Writes a sample Godot project (tileset, scene, and Pixmith's own render of the same map) for checking the
/// export against a real Godot by hand: `PIXMITH_ISO_SAMPLE_DIR=/some/dir cargo test -p pixmith-core
/// write_godot_sample_project -- --ignored`, then open the project, run `godot --headless --import`, load
/// `iso.tscn` and compare the viewport with `pixmith_render.png`. Verified pixel for pixel with Godot 4.7.2.
#[test]
#[ignore]
fn write_godot_sample_project() {
    let Ok(dir) = std::env::var("PIXMITH_ISO_SAMPLE_DIR") else { return };
    let root = PathBuf::from(&dir);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("project.godot"), "config_version=5\n[application]\nconfig/name=\"sample\"\n").unwrap();
    let mut s = Sprite::new(640, 360);
    let floor = add_iso_tilemap_layer(&mut s, 0, 96, 96, 96, 48, (240, 60));
    assert!(autotile::set_terrain(&mut s, floor, "Grass", [10, 200, 30, 255], autotile::Mode::Blob));
    let diamond = |c: [u8; 4], pillar: bool| {
        let mut img = vec![0u8; 96 * 96 * 4];
        for y in 0..96usize {
            for x in 0..96usize {
                let (dx, dy) = ((x as f64 + 0.5 - 48.0).abs(), (y as f64 + 0.5 - 72.0).abs());
                let inside = dx / 48.0 + dy / 24.0 <= 1.0;
                let post = pillar && (40..56).contains(&x) && (20..72).contains(&y);
                if inside || post {
                    let col = if post && !inside { [120, 80, 40, 255] } else { c };
                    img[(y * 96 + x) * 4..][..4].copy_from_slice(&col);
                }
            }
        }
        img
    };
    for (c, p) in [
        ([40, 160, 60, 255], false),
        ([60, 90, 200, 255], true),
        ([220, 200, 80, 255], true),
        ([200, 60, 60, 255], false),
    ] {
        let n = s.tilesets[0].add_blank() as usize;
        s.tilesets[0].tiles[n] = std::sync::Arc::new(diamond(c, p));
    }
    // Tile 4 is drawn 6 pixels higher than the default.
    set_tile_pivot(&mut s, 0, 4, Some((48, 78)));
    assert!(autotile::set_tile_mask(&mut s, floor, 1, Some(2)));
    for (i, j, t) in [
        (0, 0, 1),
        (1, 0, 2),
        (0, 1, 3),
        (1, 1, 1),
        (2, 1, 4),
        (2, 2, 2 | tilemap::FLIP_X),
        (3, 1, 3 | tilemap::FLIP_Y),
    ] {
        stamp_cell(&mut s, floor, 0, i, j, t);
    }
    let ledge = add_iso_tilemap_layer(&mut s, floor, 96, 96, 96, 48, (240, 60 - 24));
    assert!(tilemap::assign_tileset(&mut s, ledge, 0));
    stamp_cell(&mut s, ledge, 0, 1, 1, 2);
    stamp_cell(&mut s, ledge, 0, 4, 3, 3);
    let opts = Options { write_scene: true, ..Options::default() };
    let b = export_sprite_with(&s, 0, &root, &root.join("iso.tres"), &opts, false).unwrap();
    println!("{}", b.summary);
    let render = s.render(0);
    let png = crate::formats::export::encode_png(&render, 640, 360).unwrap();
    std::fs::write(root.join("pixmith_render.png"), png).unwrap();
}
