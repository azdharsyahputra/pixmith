//! Godot 4 `TileSet` (`.tres`, format 3) export, plus an optional `.tscn` that paints the
//! sprite's tilemap layers into `TileMapLayer` nodes. See docs/SPEC-game.md section 1B.
//!
//! # Terrain model
//! One terrain set (`terrain_set_0`) holds any number of terrains. Every tile has an optional
//! center terrain and, per direction, an optional neighbour terrain id (`peering`). Directions
//! are indexed like the blob mask bits: N=0 (bit 1), NE=1 (2), E=2 (4), SE=3 (8), S=4 (16),
//! SW=5 (32), W=6 (64), NW=7 (128). Even indexes are sides, odd ones are corners.
//!
//! | index | mask bit | Godot `terrains_peering_bit/…` | Sides | Corners | Blob |
//! |-------|----------|--------------------------------|-------|---------|------|
//! | 2 E   | 4        | `right_side`                   | yes   |         | yes  |
//! | 3 SE  | 8        | `bottom_right_corner`          |       | yes     | yes  |
//! | 4 S   | 16       | `bottom_side`                  | yes   |         | yes  |
//! | 5 SW  | 32       | `bottom_left_corner`           |       | yes     | yes  |
//! | 6 W   | 64       | `left_side`                    | yes   |         | yes  |
//! | 7 NW  | 128      | `top_left_corner`              |       | yes     | yes  |
//! | 0 N   | 1        | `top_side`                     | yes   |         | yes  |
//! | 1 NE  | 2        | `top_right_corner`             |       | yes     | yes  |
//!
//! Terrain set mode: Blob = 0 (Match Corners and Sides), Corners = 1, Sides = 2.
//!
//! # Alternative tiles
//! `Alternative { flip_h, flip_v, transpose }` becomes Godot alternative tile `x:y/N` (N from 1).
//! Its peering bits are the base tile's bits moved by the same transform (transpose first, then
//! flips), its collision polygons are the base ones (Godot applies the flags itself).
//!
//! Verified against Godot 4.7.2: terrain painting for all three modes and the `tile_map_data` bytes.
//! Unverified: the transform order of alternative tiles.

use crate::document::tilemap::{self, Tileset};
use crate::document::Sprite;
use std::path::{Component, Path, PathBuf};

pub const DEFAULT_MAX_TEXTURE: u32 = 4096;
pub const DEFAULT_COLOR: [f32; 4] = [0.3, 0.7, 0.3, 1.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainMode {
    Blob,
    Corners,
    Sides,
}

impl TerrainMode {
    /// Value of Godot's `terrain_set_N/mode`.
    pub fn godot_value(self) -> u8 {
        match self {
            TerrainMode::Blob => 0,
            TerrainMode::Corners => 1,
            TerrainMode::Sides => 2,
        }
    }

    pub fn from_index(i: u32) -> Self {
        match i {
            1 => TerrainMode::Corners,
            2 => TerrainMode::Sides,
            _ => TerrainMode::Blob,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collision {
    None,
    /// The whole tile square.
    Full,
    /// Rectangles covering the opaque pixels (alpha >= 128).
    Opaque,
}

impl Collision {
    pub fn from_index(i: u32) -> Self {
        match i {
            1 => Collision::Full,
            2 => Collision::Opaque,
            _ => Collision::None,
        }
    }
}

/// Godot peering name per direction index.
const NAMES: [&str; 8] = [
    "top_side",
    "top_right_corner",
    "right_side",
    "bottom_right_corner",
    "bottom_side",
    "bottom_left_corner",
    "left_side",
    "top_left_corner",
];
/// Direction indexes in the order Godot writes them.
const GODOT_ORDER: [usize; 8] = [2, 3, 4, 5, 6, 7, 0, 1];

/// Godot peering names of an isometric (`DIAMOND_DOWN`) tile per ring direction. The ring walks the neighbours in
/// cell space, N = (0, -1), E = (1, 0), S = (0, 1), W = (-1, 0), and the cell axes run down-right (`i`) and
/// down-left (`j`) on screen, so the ring is turned by 45 degrees: N is the upper right edge, NE the right
/// corner, E the lower right edge, SE the bottom corner, S the lower left edge, SW the left corner, W the upper
/// left edge and NW the top corner. (Verified against `TileMapLayer.get_neighbor_cell` in Godot 4.7.2.)
const NAMES_ISO: [&str; 8] = [
    "top_right_side",
    "right_corner",
    "bottom_right_side",
    "bottom_corner",
    "bottom_left_side",
    "left_corner",
    "top_left_side",
    "top_corner",
];
/// Direction indexes of an isometric tile in the order Godot writes them (the order of its `CellNeighbor` enum).
const GODOT_ORDER_ISO: [usize; 8] = [1, 2, 3, 4, 5, 6, 7, 0];
const DIRS: [(i8, i8); 8] = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];

#[derive(Clone, Debug, PartialEq)]
pub struct Terrain {
    pub name: String,
    pub color: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Alternative {
    pub flip_h: bool,
    pub flip_v: bool,
    pub transpose: bool,
    /// `None` = same as the base tile.
    pub probability: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct TileSpec {
    /// Straight-alpha RGBA, `tile_w * tile_h * 4` bytes.
    pub pixels: Vec<u8>,
    /// Godot `texture_origin`: where the drawn picture sits relative to the cell, in pixels. The picture is
    /// drawn with its top-left corner at `center - size / 2 - origin`. Zero for square tiles.
    pub origin: (i32, i32),
    /// Center terrain; `None` = the tile carries no terrain data.
    pub terrain: Option<u8>,
    /// Neighbour terrain per direction index (see module docs).
    pub peering: [Option<u8>; 8],
    pub probability: f32,
    pub alternatives: Vec<Alternative>,
}

impl TileSpec {
    pub fn plain(pixels: Vec<u8>) -> Self {
        TileSpec {
            pixels,
            origin: (0, 0),
            terrain: None,
            peering: [None; 8],
            probability: 1.0,
            alternatives: Vec::new(),
        }
    }

    /// Single-terrain tile from a blob mask: every set bit points at terrain 0.
    pub fn from_mask(pixels: Vec<u8>, mask: Option<u8>) -> Self {
        let mut t = TileSpec::plain(pixels);
        if let Some(m) = mask {
            t.terrain = Some(0);
            for (i, p) in t.peering.iter_mut().enumerate() {
                *p = (m & (1 << i) != 0).then_some(0);
            }
        }
        t
    }
}

/// Everything about the tileset itself. `tiles` excludes the empty tile 0.
#[derive(Clone, Debug)]
pub struct GodotTilesetInput {
    /// Size of the tile images (the atlas `texture_region_size`).
    pub tile_w: u32,
    pub tile_h: u32,
    pub tiles: Vec<TileSpec>,
    pub terrains: Vec<Terrain>,
    pub mode: TerrainMode,
    /// `Some((cell_w, cell_h))` for an isometric tileset: `tile_shape = ISOMETRIC`, `tile_layout = DIAMOND_DOWN`
    /// and `tile_size` is the diamond cell instead of the image size.
    pub iso: Option<(u32, u32)>,
}

impl GodotTilesetInput {
    /// Single-terrain convenience. `masks` is indexed like `ts.tiles` (index 0 included).
    pub fn from_tileset(ts: &Tileset, masks: Vec<Option<u8>>, name: &str, color: [f32; 4], mode: TerrainMode) -> Self {
        let skip = usize::from(ts.zero_is_empty());
        GodotTilesetInput {
            tile_w: ts.tile_w,
            tile_h: ts.tile_h,
            tiles: (skip..ts.tiles.len())
                .map(|i| TileSpec::from_mask(ts.tiles[i].to_vec(), masks.get(i).copied().flatten()))
                .collect(),
            terrains: vec![Terrain { name: name.to_string(), color }],
            mode,
            iso: ts.iso.as_ref().map(|i| (i.cell_w, i.cell_h)),
        }
        .with_origins(ts, skip)
    }

    /// Fills the tiles' `texture_origin` from the tileset's pivots (tile `k` is tileset tile `k + skip`).
    fn with_origins(mut self, ts: &Tileset, skip: usize) -> Self {
        if ts.is_iso() {
            for (k, t) in self.tiles.iter_mut().enumerate() {
                t.origin = ts.texture_origin((k + skip) as u32);
            }
        }
        self
    }

    fn has_terrain(&self) -> bool {
        self.tiles.iter().any(|t| t.terrain.is_some())
    }
}

/// Atlas, collision and scene options.
#[derive(Clone, Debug)]
pub struct Options {
    pub max_texture: u32,
    /// Empty space around the whole atlas (top/left, mirrored right/bottom).
    pub margin: u32,
    /// Empty space between tiles.
    pub separation: u32,
    /// Pixels of edge replication around each tile (against texture bleeding). Raises
    /// `margin` / `separation` to at least `extrude` / `2 * extrude`.
    pub extrude: u32,
    pub collision: Collision,
    pub write_scene: bool,
    /// Terrain used when the autotile data has none (mask-only path).
    pub fallback_name: String,
    pub fallback_color: [f32; 4],
    pub fallback_mode: TerrainMode,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            max_texture: DEFAULT_MAX_TEXTURE,
            margin: 0,
            separation: 0,
            extrude: 0,
            collision: Collision::None,
            write_scene: false,
            fallback_name: "Terrain".into(),
            fallback_color: DEFAULT_COLOR,
            fallback_mode: TerrainMode::Blob,
        }
    }
}

/// One `TileMapLayer` node: cells reference exported tile indexes (tile 0 excluded).
#[derive(Clone, Debug)]
pub struct SceneLayer {
    pub name: String,
    pub opacity: u8,
    /// (cell x, cell y, exported tile index, flip_h, flip_v, transpose)
    pub cells: Vec<(i32, i32, usize, bool, bool, bool)>,
    /// Isometric layers: the node's `position` (the canvas pixel of cell (0, 0)'s top-left corner). Such nodes
    /// also get `y_sort_enabled`, so tall tiles overlap the cells behind them like Pixmith draws them.
    pub iso_position: Option<(i32, i32)>,
}

#[derive(Clone, Debug)]
pub struct SceneInput {
    pub root_name: String,
    pub layers: Vec<SceneLayer>,
}

/// Autotile information for a tileset; tiles are indexed like `Tileset::tiles` (0 included).
#[derive(Clone, Debug)]
pub struct AutotileData {
    pub terrains: Vec<Terrain>,
    pub mode: TerrainMode,
    pub tiles: Vec<TileTerrain>,
}

#[derive(Clone, Debug, Default)]
pub struct TileTerrain {
    pub terrain: Option<u8>,
    pub peering: [Option<u8>; 8],
    pub weight: Option<f32>,
    pub alternatives: Vec<Alternative>,
}

/// Per-tile blob masks of a tileset from the auto-tiling data (blob layout only; the other modes
/// go through `autotile_data`, which carries per-direction terrains).
pub fn tile_masks(sprite: &Sprite, tileset: usize) -> Vec<Option<u8>> {
    let n = sprite.tilesets.get(tileset).map_or(0, |t| t.tiles.len());
    match crate::gamedev::autotile::tileset_autotile(sprite, tileset) {
        Some(at) if at.mode == crate::gamedev::autotile::Mode::Blob => {
            crate::gamedev::autotile::tile_masks(sprite, tileset)
        }
        _ => vec![None; n],
    }
}

/// Multi-terrain data from the auto-tiling module: terrains in priority order (Godot terrain
/// index = position), the mode, and per tile its terrain, per-direction peering terrains (ring
/// order N, NE, E, SE, S, SW, W, NW, the same as `TileTerrain::peering`), weight, and the
/// flip/transpose alternatives the auto-tiler may draw it with.
pub fn autotile_data(sprite: &Sprite, tileset: usize) -> Option<AutotileData> {
    use crate::gamedev::autotile::Mode;
    let at = crate::gamedev::autotile::tileset_autotile(sprite, tileset)?;
    if at.terrains.is_empty() || at.tiles.is_empty() {
        return None;
    }
    let mode = match at.mode {
        Mode::Sides => TerrainMode::Sides,
        Mode::Blob => TerrainMode::Blob,
        Mode::Corners => TerrainMode::Corners,
    };
    let index_of =
        |id: crate::gamedev::autotile::TerrainId| at.terrains.iter().position(|t| t.id == id).map(|i| i as u8);
    let terrains = at
        .terrains
        .iter()
        .map(|t| Terrain { name: t.name.clone(), color: t.color.map(|c| c as f32 / 255.0) })
        .collect();
    let n = sprite.tilesets.get(tileset)?.tiles.len();
    let mut tiles = vec![TileTerrain::default(); n];
    for (&i, m) in &at.tiles {
        let Some(t) = tiles.get_mut(i as usize) else { continue };
        t.peering = m.peers().map(|p| p.and_then(index_of));
        // Dual-grid tiles have no center terrain; Godot needs one to place the tile in the set.
        t.terrain = m.terrain.and_then(index_of).or_else(|| t.peering.iter().flatten().next().copied());
        t.weight = Some(m.weight);
        let (fx, fy, tr) = (m.sym & 1 != 0, m.sym & 2 != 0, m.sym & 4 != 0);
        let mut alt = |flip_h, flip_v, transpose| {
            t.alternatives.push(Alternative { flip_h, flip_v, transpose, probability: None });
        };
        if fx {
            alt(true, false, false);
        }
        if fy {
            alt(false, true, false);
        }
        if fx && fy {
            alt(true, true, false);
        }
        if tr {
            alt(false, false, true);
            if fx {
                alt(true, false, true);
            }
            if fy {
                alt(false, true, true);
            }
            if fx && fy {
                alt(true, true, true);
            }
        }
    }
    Some(AutotileData { terrains, mode, tiles })
}

// ---------------------------------------------------------------------------------------------
// Formatting helpers

fn fmt_num(v: f32) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.to_string()
    }
}

fn fmt_unit(v: f32) -> String {
    fmt_num(v.clamp(0.0, 1.0))
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace(['\n', '\r'], " ")
}

/// Moves peering ids with an alternative's transform (transpose first, then flips).
pub fn transform_peering(p: &[Option<u8>; 8], flip_h: bool, flip_v: bool, transpose: bool) -> [Option<u8>; 8] {
    let mut out = [None; 8];
    for (i, &(x, y)) in DIRS.iter().enumerate() {
        let (mut x, mut y) = (x, y);
        if transpose {
            std::mem::swap(&mut x, &mut y);
        }
        if flip_h {
            x = -x;
        }
        if flip_v {
            y = -y;
        }
        let j = DIRS.iter().position(|&d| d == (x, y)).unwrap_or(i);
        out[j] = p[i];
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Atlas layout

#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub first: usize,
    pub count: usize,
    pub cols: u32,
    pub rows: u32,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub pages: Vec<Page>,
    pub margin: u32,
    pub separation: u32,
    pub extrude: u32,
    pub tile_w: u32,
    pub tile_h: u32,
}

impl Layout {
    pub fn new(n: usize, tile_w: u32, tile_h: u32, o: &Options) -> Result<Layout, String> {
        let extrude = o.extrude;
        let margin = o.margin.max(extrude);
        let separation = o.separation.max(extrude * 2);
        let max = o.max_texture.max(1);
        let (cw, ch) = (tile_w + separation, tile_h + separation);
        let avail = |cell: u32| (max.saturating_sub(2 * margin) + separation) / cell;
        let (max_cols, max_rows) = (avail(cw), avail(ch));
        if max_cols == 0 || max_rows == 0 {
            return Err(format!("a {tile_w}x{tile_h} tile does not fit in a {max}px texture"));
        }
        let capacity = max_cols as usize * max_rows as usize;
        let mut pages = Vec::new();
        let mut first = 0;
        while first < n.max(1) {
            let count = (n - first).min(capacity);
            let (cols, rows) = if count == capacity {
                (max_cols, max_rows)
            } else {
                let c = count.max(1) as u32;
                let mut cols = ((c as f64).sqrt().ceil() as u32).clamp(1, max_cols);
                while cols > 1 && cols * cw > c.div_ceil(cols) * ch * 2 {
                    cols -= 1;
                }
                (cols, c.div_ceil(cols))
            };
            pages.push(Page { first, count, cols, rows });
            first += count.max(1);
        }
        Ok(Layout { pages, margin, separation, extrude, tile_w, tile_h })
    }

    pub fn page_size(&self, p: &Page) -> (usize, usize) {
        let w = 2 * self.margin + p.cols * self.tile_w + (p.cols - 1) * self.separation;
        let h = 2 * self.margin + p.rows * self.tile_h + (p.rows - 1) * self.separation;
        (w as usize, h as usize)
    }

    /// (page, column, row) of exported tile `i`.
    pub fn place(&self, i: usize) -> (usize, u32, u32) {
        let pi = self.pages.iter().position(|p| i < p.first + p.count).unwrap_or(0);
        let p = &self.pages[pi];
        let k = (i - p.first) as u32;
        (pi, k % p.cols, k / p.cols)
    }
}

fn render_page(inp: &GodotTilesetInput, l: &Layout, p: &Page) -> (Vec<u8>, usize, usize) {
    let (tw, th) = (l.tile_w as i64, l.tile_h as i64);
    let (w, h) = l.page_size(p);
    let mut out = vec![0u8; w * h * 4];
    let e = l.extrude as i64;
    for k in 0..p.count {
        let t = &inp.tiles[p.first + k];
        let (col, row) = (k as u32 % p.cols, k as u32 / p.cols);
        let ox = (l.margin + col * (l.tile_w + l.separation)) as i64;
        let oy = (l.margin + row * (l.tile_h + l.separation)) as i64;
        for y in -e..th + e {
            for x in -e..tw + e {
                let (sx, sy) = (x.clamp(0, tw - 1) as usize, y.clamp(0, th - 1) as usize);
                let src = (sy * tw as usize + sx) * 4;
                let dst = (((oy + y) as usize) * w + (ox + x) as usize) * 4;
                out[dst..dst + 4].copy_from_slice(&t.pixels[src..src + 4]);
            }
        }
    }
    (out, w, h)
}

// ---------------------------------------------------------------------------------------------
// Collision

type Poly = Vec<[f32; 2]>;

/// Collision polygons in tile-center coordinates.
pub fn collision_polygons(pixels: &[u8], tw: u32, th: u32, mode: Collision) -> Vec<Poly> {
    let (hw, hh) = (tw as f32 / 2.0, th as f32 / 2.0);
    let rect = |x0: u32, y0: u32, x1: u32, y1: u32| -> Poly {
        let (x0, y0, x1, y1) = (x0 as f32 - hw, y0 as f32 - hh, x1 as f32 - hw, y1 as f32 - hh);
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    };
    match mode {
        Collision::None => Vec::new(),
        Collision::Full => vec![rect(0, 0, tw, th)],
        Collision::Opaque => {
            // Horizontal runs of opaque pixels; identical runs on consecutive rows merge.
            let mut done: Vec<(u32, u32, u32, u32)> = Vec::new();
            let mut active: Vec<(u32, u32, u32)> = Vec::new(); // (x0, x1, y0)
            for y in 0..=th {
                let mut runs = Vec::new();
                if y < th {
                    let mut x = 0;
                    while x < tw {
                        if pixels[((y * tw + x) * 4 + 3) as usize] >= 128 {
                            let s = x;
                            while x < tw && pixels[((y * tw + x) * 4 + 3) as usize] >= 128 {
                                x += 1;
                            }
                            runs.push((s, x));
                        } else {
                            x += 1;
                        }
                    }
                }
                let mut next = Vec::new();
                for a in active.drain(..) {
                    if runs.contains(&(a.0, a.1)) {
                        next.push(a);
                    } else {
                        done.push((a.0, a.2, a.1, y));
                    }
                }
                for r in runs {
                    if !next.iter().any(|a| (a.0, a.1) == r) {
                        next.push((r.0, r.1, y));
                    }
                }
                active = next;
            }
            done.sort_by_key(|d| (d.1, d.0));
            done.into_iter().map(|(x0, y0, x1, y1)| rect(x0, y0, x1, y1)).collect()
        }
    }
}

/// Collision polygons of one tile in Godot's tile space (origin = the cell center). Isometric tiles: the full
/// polygon is the diamond cell, and rectangles of opaque pixels follow the picture's `texture_origin`.
fn tile_polygons(inp: &GodotTilesetInput, t: &TileSpec, mode: Collision) -> Vec<Poly> {
    let Some((cw, ch)) = inp.iso else { return collision_polygons(&t.pixels, inp.tile_w, inp.tile_h, mode) };
    match mode {
        Collision::None => Vec::new(),
        Collision::Full => {
            let (a, b) = (cw as f32 / 2.0, ch as f32 / 2.0);
            vec![vec![[0.0, -b], [a, 0.0], [0.0, b], [-a, 0.0]]]
        }
        Collision::Opaque => {
            // Picture-centered rectangles move by the origin: the picture's center sits at `-origin`.
            let (ox, oy) = (-(t.origin.0 as f32), -(t.origin.1 as f32));
            collision_polygons(&t.pixels, inp.tile_w, inp.tile_h, mode)
                .into_iter()
                .map(|p| p.into_iter().map(|[x, y]| [x + ox, y + oy]).collect())
                .collect()
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Text generation

fn write_tile(s: &mut String, inp: &GodotTilesetInput, opts: &Options, polys: &[Poly], t: &TileSpec, x: u32, y: u32) {
    use std::fmt::Write;
    let variant = |s: &mut String, id: usize, alt: Option<&Alternative>| {
        let _ = writeln!(s, "{x}:{y}/{id} = 0");
        let mut peering = t.peering;
        if let Some(a) = alt {
            if a.flip_h {
                let _ = writeln!(s, "{x}:{y}/{id}/flip_h = true");
            }
            if a.flip_v {
                let _ = writeln!(s, "{x}:{y}/{id}/flip_v = true");
            }
            if a.transpose {
                let _ = writeln!(s, "{x}:{y}/{id}/transpose = true");
            }
            peering = transform_peering(&t.peering, a.flip_h, a.flip_v, a.transpose);
        }
        if t.origin != (0, 0) {
            let _ = writeln!(s, "{x}:{y}/{id}/texture_origin = Vector2i({}, {})", t.origin.0, t.origin.1);
        }
        if let Some(c) = t.terrain {
            let _ = writeln!(s, "{x}:{y}/{id}/terrain_set = 0\n{x}:{y}/{id}/terrain = {c}");
            let (order, names) = if inp.iso.is_some() { (GODOT_ORDER_ISO, NAMES_ISO) } else { (GODOT_ORDER, NAMES) };
            for d in order {
                let corner = d % 2 == 1;
                let wanted = match inp.mode {
                    TerrainMode::Blob => true,
                    TerrainMode::Corners => corner,
                    TerrainMode::Sides => !corner,
                };
                if let (true, Some(v)) = (wanted, peering[d]) {
                    let _ = writeln!(s, "{x}:{y}/{id}/terrains_peering_bit/{} = {v}", names[d]);
                }
            }
        }
        let prob = alt.and_then(|a| a.probability).unwrap_or(t.probability);
        if (prob - 1.0).abs() > 1e-6 {
            let _ = writeln!(s, "{x}:{y}/{id}/probability = {}", fmt_num(prob.max(0.0)));
        }
        if polys.len() > 1 {
            let _ = writeln!(s, "{x}:{y}/{id}/physics_layer_0/polygons_count = {}", polys.len());
        }
        for (k, p) in polys.iter().enumerate() {
            let pts: Vec<String> = p.iter().flat_map(|q| [fmt_num(q[0]), fmt_num(q[1])]).collect();
            let _ =
                writeln!(s, "{x}:{y}/{id}/physics_layer_0/polygon_{k}/points = PackedVector2Array({})", pts.join(", "));
        }
    };
    let _ = opts;
    variant(s, 0, None);
    for (i, a) in t.alternatives.iter().enumerate() {
        variant(s, i + 1, Some(a));
    }
}

/// Builds the `.tres` text. `res_pngs` is the `res://` path of every atlas page.
pub fn tres_text(
    inp: &GodotTilesetInput,
    opts: &Options,
    layout: &Layout,
    res_pngs: &[String],
    uid: Option<&str>,
) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    s.push_str("[gd_resource type=\"TileSet\" format=3");
    if let Some(u) = uid {
        let _ = write!(s, " uid=\"{u}\"");
    }
    s.push_str("]\n\n");
    for (i, p) in res_pngs.iter().enumerate() {
        let _ = writeln!(s, "[ext_resource type=\"Texture2D\" path=\"{}\" id=\"{}_tex\"]", escape(p), i + 1);
    }
    s.push('\n');
    for (pi, page) in layout.pages.iter().enumerate() {
        let _ = writeln!(s, "[sub_resource type=\"TileSetAtlasSource\" id=\"src_{pi}\"]");
        let _ = writeln!(s, "texture = ExtResource(\"{}_tex\")", pi + 1);
        if layout.margin > 0 {
            let _ = writeln!(s, "margins = Vector2i({0}, {0})", layout.margin);
        }
        if layout.separation > 0 {
            let _ = writeln!(s, "separation = Vector2i({0}, {0})", layout.separation);
        }
        let _ = writeln!(s, "texture_region_size = Vector2i({}, {})", inp.tile_w, inp.tile_h);
        for k in 0..page.count {
            let t = &inp.tiles[page.first + k];
            let polys = tile_polygons(inp, t, opts.collision);
            write_tile(&mut s, inp, opts, &polys, t, k as u32 % page.cols, k as u32 / page.cols);
        }
        s.push('\n');
    }
    s.push_str("[resource]\n");
    match inp.iso {
        Some((cw, ch)) => {
            // TileSet.TILE_SHAPE_ISOMETRIC and TILE_LAYOUT_DIAMOND_DOWN; tile_size is the diamond cell.
            let _ = writeln!(
                s,
                "tile_shape = 1\ntile_layout = {}",
                crate::document::tilemap::IsoLayout::DiamondDown.godot_value()
            );
            let _ = writeln!(s, "tile_size = Vector2i({cw}, {ch})");
        }
        None => {
            let _ = writeln!(s, "tile_size = Vector2i({}, {})", inp.tile_w, inp.tile_h);
        }
    }
    if inp.has_terrain() {
        let _ = writeln!(s, "terrain_set_0/mode = {}", inp.mode.godot_value());
        for (i, t) in inp.terrains.iter().enumerate() {
            let [r, g, b, a] = t.color.map(fmt_unit);
            let _ = writeln!(s, "terrain_set_0/terrain_{i}/name = \"{}\"", escape(&t.name));
            let _ = writeln!(s, "terrain_set_0/terrain_{i}/color = Color({r}, {g}, {b}, {a})");
        }
    }
    if opts.collision != Collision::None {
        s.push_str("physics_layer_0/collision_layer = 1\n");
    }
    for pi in 0..layout.pages.len() {
        let _ = writeln!(s, "sources/{pi} = SubResource(\"src_{pi}\")");
    }
    s
}

fn node_name(s: &str) -> String {
    let n: String = s
        .chars()
        .map(|c| if matches!(c, '.' | ':' | '@' | '/' | '"' | '%' | '\\') || c.is_control() { '_' } else { c })
        .collect();
    if n.trim().is_empty() {
        "Layer".into()
    } else {
        n.trim().to_string()
    }
}

/// Builds the `.tscn` text: one `TileMapLayer` per scene layer.
pub fn tscn_text(
    scene: &SceneInput,
    layout: &Layout,
    res_tres: &str,
    tres_uid: Option<&str>,
    uid: Option<&str>,
) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    s.push_str("[gd_scene load_steps=2 format=3");
    if let Some(u) = uid {
        let _ = write!(s, " uid=\"{u}\"");
    }
    s.push_str("]\n\n");
    let _ = write!(s, "[ext_resource type=\"TileSet\" path=\"{}\"", escape(res_tres));
    if let Some(u) = tres_uid {
        let _ = write!(s, " uid=\"{u}\"");
    }
    s.push_str(" id=\"1_ts\"]\n\n");
    let _ = writeln!(s, "[node name=\"{}\" type=\"Node2D\"]", escape(&node_name(&scene.root_name)));
    let mut used: Vec<String> = Vec::new();
    for layer in &scene.layers {
        let base = node_name(&layer.name);
        let mut name = base.clone();
        let mut n = 2;
        while used.contains(&name) {
            name = format!("{base}_{n}");
            n += 1;
        }
        used.push(name.clone());
        let _ = writeln!(s, "\n[node name=\"{}\" type=\"TileMapLayer\" parent=\".\"]", escape(&name));
        if let Some((px, py)) = layer.iso_position {
            if (px, py) != (0, 0) {
                let _ = writeln!(s, "position = Vector2({px}, {py})");
            }
            s.push_str("y_sort_enabled = true\n");
        }
        if layer.opacity < 255 {
            let _ = writeln!(s, "modulate = Color(1, 1, 1, {})", fmt_unit(layer.opacity as f32 / 255.0));
        }
        let mut bytes: Vec<u8> = vec![0, 0];
        for &(cx, cy, tile, fh, fv, tr) in &layer.cells {
            let (page, col, row) = layout.place(tile);
            let alt: u16 = (fh as u16) << 12 | (fv as u16) << 13 | (tr as u16) << 14;
            for v in [cx as i16 as u16, cy as i16 as u16, page as u16, col as u16, row as u16, alt] {
                bytes.extend(v.to_le_bytes());
            }
        }
        s.push_str("tile_set = ExtResource(\"1_ts\")\n");
        let list: Vec<String> = bytes.iter().map(|b| b.to_string()).collect();
        let _ = writeln!(s, "tile_map_data = PackedByteArray({})", list.join(", "));
    }
    s
}

/// Extracts `uid="uid://…"` from the header line of an existing `.tres` / `.tscn`.
pub fn existing_uid(text: &str) -> Option<String> {
    let header = text.lines().next()?;
    if !header.starts_with("[gd_") {
        return None;
    }
    let rest = &header[header.find("uid=\"")? + 5..];
    let uid = &rest[..rest.find('"')?];
    uid.starts_with("uid://").then(|| uid.to_string())
}

// ---------------------------------------------------------------------------------------------
// Paths

fn normalize(p: &Path) -> Result<PathBuf, String> {
    let abs =
        if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().map_err(|e| e.to_string())?.join(p) };
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c.as_os_str()),
        }
    }
    Ok(out)
}

/// Resolves symlinks of the deepest existing ancestor, then appends the remaining names.
fn resolve(p: &Path) -> Result<PathBuf, String> {
    let p = normalize(p)?;
    let mut tail = Vec::new();
    let mut cur = p.as_path();
    loop {
        if let Ok(c) = cur.canonicalize() {
            let mut out = c;
            out.extend(tail.iter().rev());
            return Ok(out);
        }
        let (Some(name), Some(parent)) = (cur.file_name(), cur.parent()) else { return Ok(p) };
        tail.push(name.to_os_string());
        cur = parent;
    }
}

/// Resolved output locations, all inside the project.
#[derive(Debug, Clone)]
pub struct Paths {
    pub tres: PathBuf,
    pub scene: PathBuf,
    /// `res://` path of the `.tres` and of the `.tscn`.
    pub res_tres: String,
    root: PathBuf,
}

impl Paths {
    pub fn png(&self, page: usize) -> PathBuf {
        if page == 0 {
            self.tres.with_extension("png")
        } else {
            let stem = self.tres.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            self.tres.with_file_name(format!("{stem}_{page}.png"))
        }
    }

    pub fn res(&self, p: &Path) -> String {
        let rel = p.strip_prefix(&self.root).unwrap_or(p);
        let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        format!("res://{}", parts.join("/"))
    }
}

/// Validates the output path against the project root.
pub fn check_paths(project_root: &Path, out_tres: &Path) -> Result<Paths, String> {
    if !project_root.join("project.godot").is_file() {
        return Err(format!("{} is not a Godot project folder (no project.godot)", project_root.display()));
    }
    let root = resolve(project_root)?;
    let tres = resolve(out_tres)?;
    if tres.extension().and_then(|e| e.to_str()) != Some("tres") {
        return Err("the TileSet file must have the extension .tres".into());
    }
    if tres.strip_prefix(&root).is_err() {
        return Err(format!("{} is outside the Godot project", out_tres.display()));
    }
    let mut p = Paths { scene: tres.with_extension("tscn"), res_tres: String::new(), tres, root };
    p.res_tres = p.res(&p.tres);
    Ok(p)
}

// ---------------------------------------------------------------------------------------------
// Build / write

#[derive(Debug, Default)]
pub struct Built {
    pub files: Vec<(PathBuf, Vec<u8>)>,
    pub tres_path: PathBuf,
    pub png_paths: Vec<PathBuf>,
    pub scene_path: Option<PathBuf>,
    pub warnings: Vec<String>,
    pub summary: String,
}

pub type Report = Built;

fn validate(inp: &GodotTilesetInput) -> Result<(), String> {
    if inp.tiles.is_empty() {
        return Err("the tileset has no tiles to export".into());
    }
    let tlen = inp.tile_w as usize * inp.tile_h as usize * 4;
    if inp.tile_w == 0 || inp.tile_h == 0 || inp.tiles.iter().any(|t| t.pixels.len() != tlen) {
        return Err("tile pixel data does not match the tile size".into());
    }
    let n = inp.terrains.len();
    for t in &inp.tiles {
        let bad = t.terrain.into_iter().chain(t.peering.iter().flatten().copied()).any(|v| v as usize >= n);
        if bad {
            return Err("a tile refers to a terrain that does not exist".into());
        }
    }
    Ok(())
}

/// Generates every file without touching the disk (except reading existing uids).
pub fn build(
    inp: &GodotTilesetInput,
    opts: &Options,
    scene: Option<&SceneInput>,
    project_root: &Path,
    out_tres: &Path,
) -> Result<Built, String> {
    let paths = check_paths(project_root, out_tres)?;
    validate(inp)?;
    let layout = Layout::new(inp.tiles.len(), inp.tile_w, inp.tile_h, opts)?;
    let mut b = Built { tres_path: paths.tres.clone(), ..Default::default() };
    let mut res_pngs = Vec::new();
    for (pi, page) in layout.pages.iter().enumerate() {
        let (rgba, w, h) = render_page(inp, &layout, page);
        let path = paths.png(pi);
        res_pngs.push(paths.res(&path));
        b.files.push((path.clone(), crate::formats::export::encode_png(&rgba, w, h)?));
        b.png_paths.push(path);
    }
    let uid = std::fs::read_to_string(&paths.tres).ok().and_then(|t| existing_uid(&t));
    b.files.push((paths.tres.clone(), tres_text(inp, opts, &layout, &res_pngs, uid.as_deref()).into_bytes()));
    if let (true, Some(sc)) = (opts.write_scene, scene) {
        let suid = std::fs::read_to_string(&paths.scene).ok().and_then(|t| existing_uid(&t));
        b.files.push((
            paths.scene.clone(),
            tscn_text(sc, &layout, &paths.res_tres, uid.as_deref(), suid.as_deref()).into_bytes(),
        ));
        b.scene_path = Some(paths.scene.clone());
    }
    if !inp.has_terrain() {
        b.warnings.push("no auto-tile masks: exported without terrain data".to_string());
    }
    if layout.pages.len() > 1 {
        b.warnings.push(format!(
            "{} tiles do not fit one {}px texture: split into {} atlas pages",
            inp.tiles.len(),
            opts.max_texture,
            layout.pages.len()
        ));
    }
    let (w0, h0) = layout.page_size(&layout.pages[0]);
    let alts: usize = inp.tiles.iter().map(|t| t.alternatives.len()).sum();
    b.summary = format!(
        "{} tiles ({} alternatives), {} atlas page(s), first {}x{} px\nterrains: {} ({} mode)\ncollision: {:?}\nfiles: {}",
        inp.tiles.len(),
        alts,
        layout.pages.len(),
        w0,
        h0,
        if inp.has_terrain() { inp.terrains.len() } else { 0 },
        ["Match Corners and Sides", "Match Corners", "Match Sides"][inp.mode.godot_value() as usize],
        opts.collision,
        b.files.iter().map(|(p, _)| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).collect::<Vec<_>>().join(", ")
    );
    Ok(b)
}

/// Writes every generated file atomically (each file via a temporary name + rename).
pub fn write(b: &Built) -> Result<(), String> {
    for (path, data) in &b.files {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        crate::formats::gif::write_file(&path.to_string_lossy(), data)?;
    }
    Ok(())
}

pub fn export(
    inp: &GodotTilesetInput,
    opts: &Options,
    scene: Option<&SceneInput>,
    project_root: &Path,
    out_tres: &Path,
) -> Result<Built, String> {
    let b = build(inp, opts, scene, project_root, out_tres)?;
    write(&b)?;
    Ok(b)
}

/// Collects the sprite's visible tilemap layers using tileset `index` (frame 0).
pub fn scene_for(s: &Sprite, index: usize, root_name: &str) -> SceneInput {
    let ts = &s.tilesets[index];
    let skip = usize::from(ts.zero_is_empty());
    let mut layers = Vec::new();
    for (li, l) in s.layers.iter().enumerate() {
        if l.kind != tilemap::KIND_TILEMAP || l.tileset as usize != index || !s.is_layer_shown(li) {
            continue;
        }
        let Some(grid) = s.cels.first().and_then(|r| r.get(li)).and_then(|c| c.as_ref()).and_then(|c| c.tiles.as_ref())
        else {
            continue;
        };
        let mut cells = Vec::new();
        for ty in 0..grid.h {
            for tx in 0..grid.w {
                let v = grid.tiles[(ty * grid.w + tx) as usize];
                let id = (v & tilemap::TILE_ID_MASK) as usize;
                if id < skip || id >= ts.tiles.len() {
                    continue;
                }
                // Isometric grids store their first cell's (i, j); square grids their pixel origin.
                let (cx, cy) = if ts.is_iso() {
                    (grid.x + tx as i32, grid.y + ty as i32)
                } else {
                    (
                        (grid.x.div_euclid(ts.tile_w as i32)) + tx as i32,
                        (grid.y.div_euclid(ts.tile_h as i32)) + ty as i32,
                    )
                };
                cells.push((
                    cx,
                    cy,
                    id - skip,
                    v & tilemap::FLIP_X != 0,
                    v & tilemap::FLIP_Y != 0,
                    v & tilemap::FLIP_D != 0,
                ));
            }
        }
        layers.push(SceneLayer {
            name: l.name.clone(),
            opacity: l.opacity,
            cells,
            iso_position: ts.is_iso().then_some(l.offset),
        });
    }
    SceneInput { root_name: root_name.to_string(), layers }
}

/// Exports tileset `index` of `s`.
pub fn export_sprite_with(
    s: &Sprite,
    index: usize,
    project_root: &Path,
    out_tres: &Path,
    opts: &Options,
    dry_run: bool,
) -> Result<Built, String> {
    let ts = s.tilesets.get(index).ok_or_else(|| format!("tileset {index} does not exist"))?;
    let inp = match autotile_data(s, index) {
        Some(a) => {
            let skip = usize::from(ts.zero_is_empty());
            GodotTilesetInput {
                tile_w: ts.tile_w,
                tile_h: ts.tile_h,
                tiles: (skip..ts.tiles.len())
                    .map(|i| {
                        let mut t = TileSpec::plain(ts.tiles[i].to_vec());
                        if let Some(a) = a.tiles.get(i) {
                            t.terrain = a.terrain;
                            t.peering = a.peering;
                            t.probability = a.weight.unwrap_or(1.0);
                            // Flipped copies of isometric tiles are drawn with the cell's flip flags; Godot's
                            // alternative tiles would need their peering bits mapped through the flip.
                            if !ts.is_iso() {
                                t.alternatives = a.alternatives.clone();
                            }
                        }
                        t
                    })
                    .collect(),
                terrains: a.terrains,
                mode: a.mode,
                iso: ts.iso.as_ref().map(|i| (i.cell_w, i.cell_h)),
            }
            .with_origins(ts, skip)
        }
        None => {
            let name = if opts.fallback_name.is_empty() { &ts.name } else { &opts.fallback_name };
            GodotTilesetInput::from_tileset(ts, tile_masks(s, index), name, opts.fallback_color, opts.fallback_mode)
        }
    };
    let stem = out_tres.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Level".into());
    let scene = opts.write_scene.then(|| scene_for(s, index, &stem));
    if dry_run {
        build(&inp, opts, scene.as_ref(), project_root, out_tres)
    } else {
        export(&inp, opts, scene.as_ref(), project_root, out_tres)
    }
}

pub fn export_sprite(s: &Sprite, index: usize, project_root: &Path, out_tres: &Path) -> Result<Built, String> {
    export_sprite_with(s, index, project_root, out_tres, &Options::default(), false)
}

// ---------------------------------------------------------------------------------------------
// C ABI

use crate::document::Editor;
use crate::ffi::{path_arg, set_error};
use std::cell::RefCell;
use std::ffi::{c_char, CString};
use std::ptr::null_mut;

thread_local! {
    static WARNING: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Options for `pixmith_export_godot_tileset_ex`. Zero-initialise, then set fields.
#[repr(C)]
pub struct PixmithGodotOptions {
    /// 0 blob (Match Corners and Sides), 1 corners, 2 sides. Used when the document has no
    /// auto-tile data.
    pub mode: u32,
    /// 0 none, 1 full tile square, 2 opaque pixels.
    pub collision: u32,
    pub margin: u32,
    pub separation: u32,
    pub extrude: u32,
    /// 0 = default (4096).
    pub max_texture: u32,
    pub write_scene: bool,
    pub terrain_color: [f32; 4],
    /// Null or empty = use the tileset's name.
    pub terrain_name: *const c_char,
}

unsafe fn options_from(o: *const PixmithGodotOptions) -> Options {
    let mut r = Options::default();
    let Some(o) = o.as_ref() else { return r };
    r.mode_from(o);
    r.margin = o.margin.min(256);
    r.separation = o.separation.min(256);
    r.extrude = o.extrude.min(64);
    r.max_texture = if o.max_texture == 0 { DEFAULT_MAX_TEXTURE } else { o.max_texture.min(16384) };
    r.collision = Collision::from_index(o.collision);
    r.write_scene = o.write_scene;
    r.fallback_color = o.terrain_color;
    r.fallback_name = path_arg(o.terrain_name).unwrap_or_default();
    r
}

impl Options {
    fn mode_from(&mut self, o: &PixmithGodotOptions) {
        self.fallback_mode = TerrainMode::from_index(o.mode);
    }
}

unsafe fn run(
    doc: *const Editor,
    ts: u32,
    root: *const c_char,
    out: *const c_char,
    o: *const PixmithGodotOptions,
    dry: bool,
) -> Option<Built> {
    WARNING.with(|w| w.borrow_mut().clear());
    let (Some(e), Some(root), Some(out)) = (doc.as_ref(), path_arg(root), path_arg(out)) else {
        set_error("invalid argument");
        return None;
    };
    match export_sprite_with(&e.sprite, ts as usize, Path::new(&root), Path::new(&out), &options_from(o), dry) {
        Ok(b) => {
            WARNING.with(|w| *w.borrow_mut() = b.warnings.join("\n"));
            Some(b)
        }
        Err(m) => {
            set_error(m);
            None
        }
    }
}

/// Exports a tileset as <out_tres> plus a PNG atlas next to it, with default options.
/// `project_root` is the folder holding project.godot; paths outside it are refused.
#[no_mangle]
pub unsafe extern "C" fn pixmith_export_godot_tileset(
    doc: *const Editor,
    tileset_index: u32,
    project_root: *const c_char,
    out_tres: *const c_char,
) -> bool {
    crate::ffi::guarded("pixmith_export_godot_tileset", std::ptr::null_mut(), || {
        run(doc, tileset_index, project_root, out_tres, std::ptr::null(), false).is_some()
    })
}

/// Like `pixmith_export_godot_tileset` with options (null = defaults).
#[no_mangle]
pub unsafe extern "C" fn pixmith_export_godot_tileset_ex(
    doc: *const Editor,
    tileset_index: u32,
    project_root: *const c_char,
    out_tres: *const c_char,
    options: *const PixmithGodotOptions,
) -> bool {
    crate::ffi::guarded("pixmith_export_godot_tileset_ex", std::ptr::null_mut(), || {
        run(doc, tileset_index, project_root, out_tres, options, false).is_some()
    })
}

/// Describes what an export would write, without writing. Free with `pixmith_string_free`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_godot_tileset_preview(
    doc: *const Editor,
    tileset_index: u32,
    project_root: *const c_char,
    out_tres: *const c_char,
    options: *const PixmithGodotOptions,
) -> *mut c_char {
    crate::ffi::guarded("pixmith_godot_tileset_preview", std::ptr::null_mut(), || {
        let Some(b) = run(doc, tileset_index, project_root, out_tres, options, true) else { return null_mut() };
        let mut text = b.summary;
        for w in &b.warnings {
            text.push_str("\nwarning: ");
            text.push_str(w);
        }
        CString::new(text.replace('\0', "")).map_or(null_mut(), CString::into_raw)
    })
}

/// Warnings of the last export on this thread (empty string if none). Free with `pixmith_string_free`.
#[no_mangle]
pub extern "C" fn pixmith_godot_tileset_warning() -> *mut c_char {
    crate::ffi::guarded("pixmith_godot_tileset_warning", null_mut(), || {
        WARNING.with(|w| CString::new(w.borrow().replace('\0', "")).map_or(null_mut(), CString::into_raw))
    })
}

#[cfg(test)]
mod iso_tests;
#[cfg(test)]
mod tests;
