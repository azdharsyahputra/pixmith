//! Tests for the Godot TileSet export.

use super::*;

fn tile(v: u8) -> Vec<u8> {
    [v, 0, 0, 255].repeat(4)
}

fn input(mode: TerrainMode, masks: Vec<Option<u8>>) -> GodotTilesetInput {
    GodotTilesetInput {
        tile_w: 2,
        tile_h: 2,
        tiles: masks.iter().enumerate().map(|(i, m)| TileSpec::from_mask(tile(i as u8 + 1), *m)).collect(),
        terrains: vec![Terrain { name: "Grass".into(), color: [0.3, 0.7, 0.3, 1.0] }],
        mode,
    }
}

fn text(inp: &GodotTilesetInput, opts: &Options) -> String {
    let layout = Layout::new(inp.tiles.len(), inp.tile_w, inp.tile_h, opts).unwrap();
    tres_text(inp, opts, &layout, &["res://art/tiles.png".to_string()], None)
}

fn project(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pixmith-godot-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("project.godot"), "").unwrap();
    d
}

#[test]
fn golden_blob() {
    let inp = input(TerrainMode::Blob, vec![Some(4 | 8 | 16), None, Some(255), Some(0)]);
    let want = "\
[gd_resource type=\"TileSet\" format=3]

[ext_resource type=\"Texture2D\" path=\"res://art/tiles.png\" id=\"1_tex\"]

[sub_resource type=\"TileSetAtlasSource\" id=\"src_0\"]
texture = ExtResource(\"1_tex\")
texture_region_size = Vector2i(2, 2)
0:0/0 = 0
0:0/0/terrain_set = 0
0:0/0/terrain = 0
0:0/0/terrains_peering_bit/right_side = 0
0:0/0/terrains_peering_bit/bottom_right_corner = 0
0:0/0/terrains_peering_bit/bottom_side = 0
1:0/0 = 0
0:1/0 = 0
0:1/0/terrain_set = 0
0:1/0/terrain = 0
0:1/0/terrains_peering_bit/right_side = 0
0:1/0/terrains_peering_bit/bottom_right_corner = 0
0:1/0/terrains_peering_bit/bottom_side = 0
0:1/0/terrains_peering_bit/bottom_left_corner = 0
0:1/0/terrains_peering_bit/left_side = 0
0:1/0/terrains_peering_bit/top_left_corner = 0
0:1/0/terrains_peering_bit/top_side = 0
0:1/0/terrains_peering_bit/top_right_corner = 0
1:1/0 = 0
1:1/0/terrain_set = 0
1:1/0/terrain = 0

[resource]
tile_size = Vector2i(2, 2)
terrain_set_0/mode = 0
terrain_set_0/terrain_0/name = \"Grass\"
terrain_set_0/terrain_0/color = Color(0.3, 0.7, 0.3, 1)
sources/0 = SubResource(\"src_0\")
";
    assert_eq!(text(&inp, &Options::default()), want);
}

#[test]
fn golden_multi_terrain() {
    let mut a = TileSpec::plain(tile(1));
    a.terrain = Some(0);
    a.peering[2] = Some(0); // E: grass
    a.peering[4] = Some(1); // S: water
    a.peering[3] = Some(1); // SE: water
    let mut b = TileSpec::plain(tile(2));
    b.terrain = Some(1);
    b.peering[6] = Some(0); // W: grass
    let inp = GodotTilesetInput {
        tile_w: 2,
        tile_h: 2,
        tiles: vec![a, b],
        terrains: vec![
            Terrain { name: "Grass".into(), color: [0.3, 0.7, 0.3, 1.0] },
            Terrain { name: "Water \"deep\"".into(), color: [0.1, 0.25, 0.9, 0.5] },
        ],
        mode: TerrainMode::Blob,
    };
    let want = "\
[gd_resource type=\"TileSet\" format=3]

[ext_resource type=\"Texture2D\" path=\"res://art/tiles.png\" id=\"1_tex\"]

[sub_resource type=\"TileSetAtlasSource\" id=\"src_0\"]
texture = ExtResource(\"1_tex\")
texture_region_size = Vector2i(2, 2)
0:0/0 = 0
0:0/0/terrain_set = 0
0:0/0/terrain = 0
0:0/0/terrains_peering_bit/right_side = 0
0:0/0/terrains_peering_bit/bottom_right_corner = 1
0:0/0/terrains_peering_bit/bottom_side = 1
1:0/0 = 0
1:0/0/terrain_set = 0
1:0/0/terrain = 1
1:0/0/terrains_peering_bit/left_side = 0

[resource]
tile_size = Vector2i(2, 2)
terrain_set_0/mode = 0
terrain_set_0/terrain_0/name = \"Grass\"
terrain_set_0/terrain_0/color = Color(0.3, 0.7, 0.3, 1)
terrain_set_0/terrain_1/name = \"Water \\\"deep\\\"\"
terrain_set_0/terrain_1/color = Color(0.1, 0.25, 0.9, 0.5)
sources/0 = SubResource(\"src_0\")
";
    assert_eq!(text(&inp, &Options::default()), want);
    // Sides mode keeps side bits only, Corners keeps corner bits only; ids stay per direction.
    let mut sides = inp.clone();
    sides.mode = TerrainMode::Sides;
    let t = text(&sides, &Options::default());
    assert!(t.contains("terrain_set_0/mode = 2") && t.contains("bottom_side = 1") && !t.contains("corner = 1"));
    let mut corners = inp.clone();
    corners.mode = TerrainMode::Corners;
    let t = text(&corners, &Options::default());
    assert!(
        t.contains("terrain_set_0/mode = 1") && t.contains("bottom_right_corner = 1") && !t.contains("bottom_side = 1")
    );
    // A tile naming a missing terrain is rejected.
    let mut bad = inp;
    bad.tiles[0].peering[0] = Some(5);
    assert!(validate(&bad).is_err());
}

#[test]
fn sides_and_corners_bits() {
    let m = vec![Some(255)];
    let sides = text(&input(TerrainMode::Sides, m.clone()), &Options::default());
    assert!(sides.contains("terrain_set_0/mode = 2"));
    assert_eq!(sides.matches("peering_bit").count(), 4);
    assert!(!sides.contains("corner = 0"));
    let corners = text(&input(TerrainMode::Corners, m), &Options::default());
    assert!(corners.contains("terrain_set_0/mode = 1"));
    assert_eq!(corners.matches("peering_bit").count(), 4);
    assert!(!corners.contains("_side = 0"));
}

#[test]
fn golden_alternatives_probability_collision() {
    let mut t = TileSpec::from_mask(tile(1), Some(4 | 16)); // E and S
    t.probability = 0.25;
    t.alternatives = vec![
        Alternative { flip_h: true, flip_v: false, transpose: false, probability: None },
        Alternative { flip_h: false, flip_v: false, transpose: true, probability: Some(2.0) },
    ];
    let inp = GodotTilesetInput {
        tile_w: 2,
        tile_h: 2,
        tiles: vec![t],
        terrains: vec![Terrain { name: "T".into(), color: [1.0, 0.0, 0.0, 1.0] }],
        mode: TerrainMode::Sides,
    };
    let opts = Options { collision: Collision::Full, ..Options::default() };
    let want = "\
[gd_resource type=\"TileSet\" format=3]

[ext_resource type=\"Texture2D\" path=\"res://art/tiles.png\" id=\"1_tex\"]

[sub_resource type=\"TileSetAtlasSource\" id=\"src_0\"]
texture = ExtResource(\"1_tex\")
texture_region_size = Vector2i(2, 2)
0:0/0 = 0
0:0/0/terrain_set = 0
0:0/0/terrain = 0
0:0/0/terrains_peering_bit/right_side = 0
0:0/0/terrains_peering_bit/bottom_side = 0
0:0/0/probability = 0.25
0:0/0/physics_layer_0/polygon_0/points = PackedVector2Array(-1, -1, 1, -1, 1, 1, -1, 1)
0:0/1 = 0
0:0/1/flip_h = true
0:0/1/terrain_set = 0
0:0/1/terrain = 0
0:0/1/terrains_peering_bit/bottom_side = 0
0:0/1/terrains_peering_bit/left_side = 0
0:0/1/probability = 0.25
0:0/1/physics_layer_0/polygon_0/points = PackedVector2Array(-1, -1, 1, -1, 1, 1, -1, 1)
0:0/2 = 0
0:0/2/transpose = true
0:0/2/terrain_set = 0
0:0/2/terrain = 0
0:0/2/terrains_peering_bit/right_side = 0
0:0/2/terrains_peering_bit/bottom_side = 0
0:0/2/probability = 2
0:0/2/physics_layer_0/polygon_0/points = PackedVector2Array(-1, -1, 1, -1, 1, 1, -1, 1)

[resource]
tile_size = Vector2i(2, 2)
terrain_set_0/mode = 2
terrain_set_0/terrain_0/name = \"T\"
terrain_set_0/terrain_0/color = Color(1, 0, 0, 1)
physics_layer_0/collision_layer = 1
sources/0 = SubResource(\"src_0\")
";
    assert_eq!(text(&inp, &opts), want);
}

#[test]
fn peering_transform() {
    let mut p = [None; 8];
    p[2] = Some(1); // E
    assert_eq!(transform_peering(&p, true, false, false)[6], Some(1)); // flip_h: W
    assert_eq!(transform_peering(&p, false, true, false)[2], Some(1)); // flip_v: still E
    assert_eq!(transform_peering(&p, false, false, true)[4], Some(1)); // transpose: S
    p[3] = Some(2); // SE
    assert_eq!(transform_peering(&p, true, true, false)[7], Some(2)); // NW
}

#[test]
fn collision_centered_and_traced() {
    // 4x2 tile: left half opaque, right half transparent.
    let mut px = vec![0u8; 4 * 2 * 4];
    for y in 0..2 {
        for x in 0..2 {
            px[(y * 4 + x) * 4 + 3] = 255;
        }
    }
    let polys = collision_polygons(&px, 4, 2, Collision::Opaque);
    assert_eq!(polys, vec![vec![[-2.0, -1.0], [0.0, -1.0], [0.0, 1.0], [-2.0, 1.0]]]);
    // L shape: two rectangles, each centered on the tile.
    let mut l = vec![0u8; 3 * 3 * 4];
    for (x, y) in [(0, 0), (0, 1), (0, 2), (1, 2), (2, 2)] {
        l[(y * 3 + x) * 4 + 3] = 255;
    }
    let polys = collision_polygons(&l, 3, 3, Collision::Opaque);
    assert_eq!(polys.len(), 2);
    assert_eq!(polys[0], vec![[-1.5, -1.5], [-0.5, -1.5], [-0.5, 0.5], [-1.5, 0.5]]);
    assert_eq!(polys[1], vec![[-1.5, 0.5], [1.5, 0.5], [1.5, 1.5], [-1.5, 1.5]]);
    let full = collision_polygons(&l, 3, 3, Collision::Full);
    assert_eq!(full, vec![vec![[-1.5, -1.5], [1.5, -1.5], [1.5, 1.5], [-1.5, 1.5]]]);
    assert!(collision_polygons(&[0u8; 16], 2, 2, Collision::Opaque).is_empty());
    let mut inp = input(TerrainMode::Blob, vec![Some(1)]);
    inp.tile_w = 3;
    inp.tile_h = 3;
    inp.tiles[0].pixels = l;
    let t = text(&inp, &Options { collision: Collision::Opaque, ..Options::default() });
    assert!(t.contains("0:0/0/physics_layer_0/polygons_count = 2"));
    assert!(t.contains("polygon_1/points = PackedVector2Array(-1.5, 0.5, 1.5, 0.5, 1.5, 1.5, -1.5, 1.5)"));
}

#[test]
fn atlas_coordinates_and_pages() {
    let o = Options::default();
    let g = |n, t, max| {
        let l = Layout::new(n, t, t, &Options { max_texture: max, ..o.clone() }).unwrap();
        (l.pages.len(), l.pages[0].cols, l.pages[0].rows)
    };
    assert_eq!(g(47, 16, 4096), (1, 7, 7));
    assert_eq!(g(1, 16, 4096), (1, 1, 1));
    assert_eq!(g(16, 16, 4096), (1, 4, 4));
    assert_eq!(g(9, 16, 48), (1, 3, 3));
    // 10 tiles, 9 fit one 48px page: split into 2 pages.
    let l = Layout::new(10, 16, 16, &Options { max_texture: 48, ..o.clone() }).unwrap();
    assert_eq!(l.pages.len(), 2);
    assert_eq!((l.pages[1].first, l.pages[1].count), (9, 1));
    assert_eq!(l.place(9), (1, 0, 0));
    assert_eq!(l.place(4), (0, 1, 1));
    // Margins and separation.
    let l = Layout::new(4, 16, 16, &Options { margin: 2, separation: 1, ..o }).unwrap();
    assert_eq!(l.page_size(&l.pages[0]), (4 + 33, 4 + 33));
}

#[test]
fn margins_separation_extrude() {
    let inp = input(TerrainMode::Blob, vec![Some(1), Some(1)]);
    let opts = Options { extrude: 1, margin: 3, ..Options::default() };
    let t = text(&inp, &opts);
    assert!(t.contains("margins = Vector2i(3, 3)"));
    assert!(t.contains("separation = Vector2i(2, 2)"));
    let layout = Layout::new(2, 2, 2, &opts).unwrap();
    let (rgba, w, _) = render_page(&inp, &layout, &layout.pages[0]);
    // Tile 0 (red=1) at x=3..5, y=3..5 is extruded by one pixel: (2,2) holds the corner pixel.
    assert_eq!(rgba[(2 * w + 2) * 4], 1);
    assert_eq!(rgba[(5 * w + 5) * 4], 1);
    // Tile 1 (red=2) starts at 3 + 2 + 2 = 7 horizontally, extruded left to x=6.
    assert_eq!(rgba[(3 * w + 6) * 4], 2);
    assert_eq!(rgba[(3 * w + 7) * 4], 2);
    // The gap between neighbours holds each side's own extrusion, nothing overlaps.
    assert_eq!(rgba[(3 * w + 5) * 4], 1);
}

#[test]
fn uid_preserved_and_deterministic() {
    let root = project("uid");
    let out = root.join("art/tiles.tres");
    let inp = input(TerrainMode::Blob, vec![Some(5), Some(7)]);
    let opts = Options::default();
    let r = export(&inp, &opts, None, &root, &out).unwrap();
    assert!(r.warnings.is_empty());
    let first = std::fs::read(&out).unwrap();
    let png1 = std::fs::read(&r.png_paths[0]).unwrap();
    assert!(!String::from_utf8_lossy(&first).contains("uid="));
    export(&inp, &opts, None, &root, &out).unwrap();
    assert_eq!(first, std::fs::read(&out).unwrap());
    assert_eq!(png1, std::fs::read(&r.png_paths[0]).unwrap());
    let text = String::from_utf8(first).unwrap().replacen("format=3", "format=3 uid=\"uid://abc123\"", 1);
    std::fs::write(&out, text).unwrap();
    export(&inp, &opts, None, &root, &out).unwrap();
    let again = std::fs::read_to_string(&out).unwrap();
    assert!(again.starts_with("[gd_resource type=\"TileSet\" format=3 uid=\"uid://abc123\"]"));
    assert!(again.contains("path=\"res://art/tiles.png\""));
    assert!(!root.join("art/tiles.tres.pixmith-tmp").exists());
}

#[test]
fn refuses_outside_root() {
    let root = project("outside");
    let inp = input(TerrainMode::Blob, vec![Some(1)]);
    let o = Options::default();
    let other = std::env::temp_dir().join(format!("pixmith-godot-{}-elsewhere.tres", std::process::id()));
    assert!(export(&inp, &o, None, &root, &other).is_err());
    assert!(export(&inp, &o, None, &root, &root.join("../escape.tres")).is_err());
    assert!(!root.join("../escape.png").exists());
    let no_proj = std::env::temp_dir().join(format!("pixmith-godot-{}-noproj", std::process::id()));
    std::fs::create_dir_all(&no_proj).unwrap();
    assert!(export(&inp, &o, None, &no_proj, &no_proj.join("a.tres")).is_err());
}

#[test]
fn no_masks_warns_and_omits_terrain() {
    let root = project("nomask");
    let inp = input(TerrainMode::Blob, vec![None, None]);
    let r = export(&inp, &Options::default(), None, &root, &root.join("t.tres")).unwrap();
    assert_eq!(r.warnings.len(), 1);
    assert!(!std::fs::read_to_string(root.join("t.tres")).unwrap().contains("terrain"));
}

#[test]
fn from_tileset_skips_empty_tile() {
    let mut ts = Tileset::new("T", 2, 2);
    ts.add_blank();
    let inp = GodotTilesetInput::from_tileset(&ts, vec![Some(9), Some(3)], "T", DEFAULT_COLOR, TerrainMode::Sides);
    assert_eq!(inp.tiles.len(), 1);
    assert_eq!(inp.tiles[0].terrain, Some(0));
    assert_eq!(inp.tiles[0].peering, [Some(0), Some(0), None, None, None, None, None, None]);
}

#[test]
fn multi_page_export_writes_all_pages() {
    let root = project("pages");
    let inp = input(TerrainMode::Blob, vec![Some(1); 10]);
    let opts = Options { max_texture: 6, ..Options::default() };
    let r = export(&inp, &opts, None, &root, &root.join("t.tres")).unwrap();
    assert_eq!(r.png_paths.len(), 2);
    assert!(root.join("t.png").exists() && root.join("t_1.png").exists());
    let t = std::fs::read_to_string(root.join("t.tres")).unwrap();
    assert!(t.contains("id=\"2_tex\"") && t.contains("sources/1 = SubResource(\"src_1\")"));
    assert!(t.contains("path=\"res://t_1.png\""));
}

#[test]
fn scene_cells_from_sprite() {
    let root = project("scene");
    let mut s = Sprite::new(4, 2);
    let layer = tilemap::add_tilemap_layer(&mut s, 0, 2, 2);
    s.layers[layer].name = "Ground.1".into();
    let ts = s.layers[layer].tileset as usize;
    s.tilesets[ts].add_blank();
    s.tilesets[ts].add_blank();
    tilemap::stamp(&mut s, layer, 0, 0, 0, 2);
    tilemap::stamp(&mut s, layer, 0, 2, 0, 1 | tilemap::FLIP_X);
    let opts = Options { write_scene: true, ..Options::default() };
    let r = export_sprite_with(&s, ts, &root, &root.join("lv/tiles.tres"), &opts, false).unwrap();
    assert_eq!(r.scene_path.as_deref(), Some(root.join("lv/tiles.tscn").canonicalize().unwrap().as_path()));
    // cell (0,0): tile 2 -> exported 1 -> atlas col 1; cell (1,0): tile 1 -> exported 0, flip_h.
    let want = "\
[gd_scene load_steps=2 format=3]

[ext_resource type=\"TileSet\" path=\"res://lv/tiles.tres\" id=\"1_ts\"]

[node name=\"tiles\" type=\"Node2D\"]

[node name=\"Ground_1\" type=\"TileMapLayer\" parent=\".\"]
tile_set = ExtResource(\"1_ts\")
tile_map_data = PackedByteArray(0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 16)
";
    assert_eq!(std::fs::read_to_string(root.join("lv/tiles.tscn")).unwrap(), want);
    // Dry run writes nothing.
    let again = export_sprite_with(&s, ts, &root, &root.join("lv2/tiles.tres"), &opts, true).unwrap();
    assert!(!root.join("lv2").exists());
    assert!(again.summary.contains("2 tiles"));
}

#[test]
fn autotile_terrain_data_reaches_the_godot_tileset() {
    use crate::gamedev::autotile;
    let mut s = Sprite::new(64, 64);
    let layer = crate::document::tilemap::add_tilemap_layer(&mut s, 0, 16, 16);
    assert!(autotile::set_mode(&mut s, layer, autotile::Mode::Blob));
    autotile::add_terrain(&mut s, layer, "Grass", [80, 180, 80, 255]).unwrap();
    assert!(autotile::generate_template(&mut s, layer, false) > 0);
    let ts = s.layers[layer].tileset as usize;
    let data = autotile_data(&s, ts).expect("autotile data for the tileset");
    let grass = data.terrains.iter().position(|t| t.name == "Grass").expect("Grass terrain");
    assert!(data.tiles.iter().any(|t| t.peering.iter().any(|p| p.is_some())));

    let dir = std::env::temp_dir().join(format!("pixmith-autotile-godot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("project.godot"), "config_version=5\n").unwrap();
    let built = export_sprite_with(&s, ts, &dir, &dir.join("tiles.tres"), &Options::default(), true).unwrap();
    let tres = String::from_utf8(
        built.files.iter().find(|(p, _)| p.extension().is_some_and(|e| e == "tres")).unwrap().1.clone(),
    )
    .unwrap();
    assert!(tres.contains(&format!("terrain_set_0/terrain_{grass}/name = \"Grass\"")), "{tres}");
    assert!(tres.contains("terrains_peering_bit/"), "{tres}");
    assert!(!built.warnings.iter().any(|w| w.contains("no terrain")), "{:?}", built.warnings);
    let _ = std::fs::remove_dir_all(&dir);
}
