//! Tilemap layers and tilesets.
//!
//! A tilemap layer shows a grid of tiles taken from a [`Tileset`]. The tile grid lives in
//! `Cel::tiles`; `Cel::pixels` always holds the rendered result so the compositor and the
//! pixel tools keep working unchanged. Pixel edits made on a tilemap layer are written back
//! into the shared tile images by [`sync_cel`], so they affect every instance of a tile.

use crate::document::color_mode::Encoder;
use crate::document::iso::{self, Iso};
use crate::document::{Cel, Editor, Sprite};
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::ffi::{c_char, CString};
use std::io::{Read, Write};
use std::ptr::null_mut;
use std::sync::Arc;

/// `Layer::kind` of a tilemap layer.
pub const KIND_TILEMAP: u16 = 2;
/// Chunk type of a Tileset chunk in .aseprite files.
pub const CHUNK_TILESET: u16 = 0x2023;

/// Tile value layout: the low bits are the tile index, the top three bits are flip flags.
pub const TILE_ID_MASK: u32 = 0x1FFF_FFFF;
pub const FLIP_X: u32 = 0x8000_0000;
pub const FLIP_Y: u32 = 0x4000_0000;
/// Diagonal flip (swap the X and Y axes); only applies to square tiles.
pub const FLIP_D: u32 = 0x2000_0000;
const FLIP_MASK: u32 = FLIP_X | FLIP_Y | FLIP_D;

/// Tileset flag: tile index 0 is the empty tile.
const TS_ZERO_EMPTY: u32 = 4;
const TS_EMBEDDED: u32 = 2;
const TS_EXTERNAL: u32 = 1;

const MAX_TILE_SIZE: u32 = 1024;
const MAX_CELLS: usize = 1 << 24;
const MAX_TILES: usize = 1 << 20;

pub type Result<T> = std::result::Result<T, String>;

/// How the cells of an isometric grid are laid out. Godot calls this `TileSet.tile_layout`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsoLayout {
    /// Cell (i, j) has its box at `(x + (i - j) * w/2, y + (i + j) * h/2)`: `i` runs down-right, `j` down-left.
    DiamondDown,
}

impl IsoLayout {
    /// Value of Godot's `TileSet.tile_layout`.
    pub fn godot_value(self) -> u8 {
        match self {
            IsoLayout::DiamondDown => 5,
        }
    }
}

/// The isometric shape of a tileset: the diamond cell (`cell_w` x `cell_h`, 2:1 for the usual look) that
/// tiles are placed on, and where each tile image sits on it. Square tilesets have none.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoSpec {
    pub cell_w: u32,
    pub cell_h: u32,
    pub layout: IsoLayout,
    /// Pivot of tiles that deviate from the default, by tile index: the point of the tile image (in pixels from
    /// its top-left corner, edge coordinates) that sits on the center of the cell's diamond.
    pub pivots: BTreeMap<u32, (i32, i32)>,
}

impl IsoSpec {
    /// A spec with sanitized (even, in range) cell sides.
    pub fn new(cell_w: u32, cell_h: u32) -> IsoSpec {
        IsoSpec {
            cell_w: iso::clamp_side(cell_w.min(iso::MAX_SIDE as u32) as i32) as u32,
            cell_h: iso::clamp_side(cell_h.min(iso::MAX_SIDE as u32) as i32) as u32,
            layout: IsoLayout::DiamondDown,
            pivots: BTreeMap::new(),
        }
    }
}

/// A set of equally sized RGBA tiles. Tile 0 is the empty tile.
#[derive(Clone, Debug, PartialEq)]
pub struct Tileset {
    pub name: String,
    /// Size of the tile images. For isometric tilesets the diamond cell is `iso.cell_w` x `iso.cell_h`; images
    /// taller than the diamond (cliffs, tufts) stick out above it.
    pub tile_w: u32,
    pub tile_h: u32,
    /// `Some` for isometric tilesets; `None` is the ordinary square grid.
    pub iso: Option<IsoSpec>,
    /// Number shown in the UI for tile 1 (the file format default is 1).
    pub base_index: i16,
    /// Raw Aseprite tileset flags except the storage bits.
    pub flags: u32,
    pub tiles: Vec<Arc<Vec<u8>>>,
}

impl Tileset {
    pub fn new(name: impl Into<String>, tile_w: u32, tile_h: u32) -> Self {
        let mut t = Tileset {
            name: name.into(),
            tile_w: tile_w.clamp(1, MAX_TILE_SIZE),
            tile_h: tile_h.clamp(1, MAX_TILE_SIZE),
            iso: None,
            base_index: 1,
            flags: TS_ZERO_EMPTY,
            tiles: Vec::new(),
        };
        t.add_blank();
        t
    }

    /// An isometric tileset: `tile_w` x `tile_h` images on `cell_w` x `cell_h` diamonds.
    pub fn new_iso(name: impl Into<String>, tile_w: u32, tile_h: u32, cell_w: u32, cell_h: u32) -> Self {
        let mut t = Tileset::new(name, tile_w, tile_h);
        t.iso = Some(IsoSpec::new(cell_w, cell_h));
        t
    }

    pub fn is_iso(&self) -> bool {
        self.iso.is_some()
    }

    /// The pivot of a tile of an isometric tileset: the image point that sits on the diamond's center. Unless
    /// set, tiles are centered across and, when taller than the diamond, bottom-aligned (the diamond is the
    /// bottom `cell_h` rows of the image); otherwise the image is centered. Square tilesets: the image center.
    pub fn pivot(&self, tile: u32) -> (i32, i32) {
        let (tw, th) = (self.tile_w as i32, self.tile_h as i32);
        let Some(spec) = &self.iso else { return (tw / 2, th / 2) };
        if let Some(&p) = spec.pivots.get(&tile) {
            return p;
        }
        (tw / 2, if th > spec.cell_h as i32 { th - spec.cell_h as i32 / 2 } else { th / 2 })
    }

    /// The pivot as a Godot `texture_origin` (the tile's drawn position is `center - size/2 - origin`).
    pub fn texture_origin(&self, tile: u32) -> (i32, i32) {
        let (px, py) = self.pivot(tile);
        (px - self.tile_w as i32 / 2, py - self.tile_h as i32 / 2)
    }

    /// The grid used to place this tileset's cells with the layer offset `offset` (isometric tilesets only).
    pub fn iso_grid(&self, offset: (i32, i32)) -> Option<Iso> {
        self.iso.as_ref().map(|i| Iso::new(i.cell_w as i32, i.cell_h as i32, offset.0, offset.1))
    }

    fn tile_len(&self) -> usize {
        self.tile_w as usize * self.tile_h as usize * 4
    }

    pub fn zero_is_empty(&self) -> bool {
        self.flags & TS_ZERO_EMPTY != 0
    }

    /// Appends a transparent tile and returns its index.
    pub fn add_blank(&mut self) -> u32 {
        self.tiles.push(Arc::new(vec![0; self.tile_len()]));
        self.tiles.len() as u32 - 1
    }

    /// Whether a tile value refers to a tile that should be drawn.
    fn usable(&self, id: usize) -> bool {
        id < self.tiles.len() && !(id == 0 && self.zero_is_empty())
    }
}

/// A rectangle of tile values placed at pixel position (`x`, `y`) of the canvas.
#[derive(Clone, Debug, PartialEq)]
pub struct TileGrid {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// Row-major tile values (index | flip flags).
    pub tiles: Vec<u32>,
}

impl TileGrid {
    fn empty() -> Self {
        TileGrid { x: 0, y: 0, w: 0, h: 0, tiles: Vec::new() }
    }

    /// A grid at the origin large enough to cover a canvas, filled with empty tiles.
    pub fn covering(width: u32, height: u32, tile_w: u32, tile_h: u32) -> Self {
        let (w, h) = (width.div_ceil(tile_w.max(1)), height.div_ceil(tile_h.max(1)));
        TileGrid { x: 0, y: 0, w, h, tiles: vec![0; w as usize * h as usize] }
    }

    fn index(&self, tx: i32, ty: i32) -> Option<usize> {
        (tx >= 0 && ty >= 0 && (tx as u32) < self.w && (ty as u32) < self.h)
            .then(|| ty as usize * self.w as usize + tx as usize)
    }

    /// Cell column and row (possibly outside the grid) containing a canvas pixel.
    fn cell_of(&self, px: i32, py: i32, tw: u32, th: u32) -> (i32, i32) {
        ((px - self.x).div_euclid(tw as i32), (py - self.y).div_euclid(th as i32))
    }

    /// Grows the grid so the cell containing the canvas pixel exists. Returns whether it grew.
    fn grow_to_include(&mut self, px: i32, py: i32, tw: u32, th: u32) -> bool {
        if self.w == 0 || self.h == 0 {
            *self = TileGrid {
                x: px.div_euclid(tw as i32) * tw as i32,
                y: py.div_euclid(th as i32) * th as i32,
                w: 1,
                h: 1,
                tiles: vec![0],
            };
            return true;
        }
        let (tx, ty) = self.cell_of(px, py, tw, th);
        self.grow_rel(tx, ty, tw as i32, th as i32)
    }

    /// Grows the grid by whole rows and columns so that the cell at column `tx` and row `ty` (relative to the
    /// grid, possibly outside it) exists; the origin moves by `sx` / `sy` per column / row.
    fn grow_rel(&mut self, tx: i32, ty: i32, sx: i32, sy: i32) -> bool {
        if self.index(tx, ty).is_some() {
            return false;
        }
        let (min_x, min_y) = (tx.min(0), ty.min(0));
        let (max_x, max_y) = (tx.max(self.w as i32 - 1), ty.max(self.h as i32 - 1));
        let (nw, nh) = ((max_x - min_x + 1) as usize, (max_y - min_y + 1) as usize);
        if nw * nh > MAX_CELLS {
            return false;
        }
        let mut tiles = vec![0; nw * nh];
        for row in 0..self.h as usize {
            let dst = (row as i32 - min_y) as usize * nw + (0 - min_x) as usize;
            let src = row * self.w as usize;
            tiles[dst..dst + self.w as usize].copy_from_slice(&self.tiles[src..src + self.w as usize]);
        }
        self.x += min_x * sx;
        self.y += min_y * sy;
        self.w = nw as u32;
        self.h = nh as u32;
        self.tiles = tiles;
        true
    }

    // Isometric grids: `x` and `y` are the cell coordinates (i, j) of the first column and row.

    /// Index of isometric cell `(i, j)`, if the grid holds it.
    pub fn index_cell(&self, i: i64, j: i64) -> Option<usize> {
        let (tx, ty) = (i - self.x as i64, j - self.y as i64);
        (tx >= 0 && ty >= 0 && tx < self.w as i64 && ty < self.h as i64)
            .then(|| ty as usize * self.w as usize + tx as usize)
    }

    /// The tile value in isometric cell `(i, j)` (0 outside the grid).
    pub fn cell_value(&self, i: i64, j: i64) -> u32 {
        self.index_cell(i, j).map_or(0, |k| self.tiles[k])
    }

    /// Grows an isometric grid so cell `(i, j)` exists. Returns whether it grew.
    fn grow_to_include_cell(&mut self, i: i64, j: i64) -> bool {
        if i.abs() > iso::MAX_CELL || j.abs() > iso::MAX_CELL {
            return false;
        }
        if self.w == 0 || self.h == 0 {
            *self = TileGrid { x: i as i32, y: j as i32, w: 1, h: 1, tiles: vec![0] };
            return true;
        }
        self.grow_rel((i - self.x as i64) as i32, (j - self.y as i64) as i32, 1, 1)
    }

    /// Calls `f(i, j, value)` for every non-empty cell of an isometric grid in draw order (back to front by
    /// `i + j`, then left to right by `i`).
    pub fn for_each_cell_in_draw_order(&self, mut f: impl FnMut(i64, i64, u32)) {
        if self.w == 0 || self.h == 0 {
            return;
        }
        let (x0, y0) = (self.x as i64, self.y as i64);
        let (x1, y1) = (x0 + self.w as i64 - 1, y0 + self.h as i64 - 1);
        for sum in x0 + y0..=x1 + y1 {
            for i in x0.max(sum - y1)..=x1.min(sum - y0) {
                let j = sum - i;
                let v = self.tiles[(j - y0) as usize * self.w as usize + (i - x0) as usize];
                if v != 0 {
                    f(i, j, v);
                }
            }
        }
    }
}

/// Maps a pixel of a displayed cell to the pixel of the tile image it shows.
///
/// The file format spec only names the three flip bit masks ("X flip", "Y flip", "diagonal
/// flip (swap X/Y axis)") and says nothing about the order they combine in. Pixmith uses the
/// convention of the Tiled map format, whose flag bits Aseprite shares: the tile image is
/// first transposed (diagonal), then flipped on X, then flipped on Y. As a consequence
/// diagonal+X shows the image rotated 90 degrees clockwise and diagonal+Y counter-clockwise.
/// A diagonal flip only applies to square tiles.
fn source_pixel(cx: usize, cy: usize, flags: u32, tw: usize, th: usize) -> (usize, usize) {
    let (mut a, mut b) = (cx, cy);
    if flags & FLIP_Y != 0 {
        b = th - 1 - b;
    }
    if flags & FLIP_X != 0 {
        a = tw - 1 - a;
    }
    if flags & FLIP_D != 0 && tw == th {
        std::mem::swap(&mut a, &mut b);
    }
    (a, b)
}

fn same_pixel(a: &[u8], b: &[u8]) -> bool {
    a == b || (a[3] == 0 && b[3] == 0)
}

/// Composites a tile grid into a full-canvas RGBA buffer.
pub fn render_grid(grid: &TileGrid, ts: &Tileset, width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 4];
    let (tw, th) = (ts.tile_w as usize, ts.tile_h as usize);
    for ty in 0..grid.h as usize {
        for tx in 0..grid.w as usize {
            let value = grid.tiles[ty * grid.w as usize + tx];
            let id = (value & TILE_ID_MASK) as usize;
            if !ts.usable(id) {
                continue;
            }
            let tile = &ts.tiles[id];
            for cy in 0..th {
                let py = grid.y as i64 + (ty * th + cy) as i64;
                if py < 0 || py >= height as i64 {
                    continue;
                }
                for cx in 0..tw {
                    let px = grid.x as i64 + (tx * tw + cx) as i64;
                    if px < 0 || px >= width as i64 {
                        continue;
                    }
                    let (u, v) = source_pixel(cx, cy, value, tw, th);
                    let d = (py as usize * width + px as usize) * 4;
                    let s = (v * tw + u) * 4;
                    out[d..d + 4].copy_from_slice(&tile[s..s + 4]);
                }
            }
        }
    }
    out
}

/// Source-over compositing of straight-alpha RGBA pixels: `src` drawn onto `dst`.
fn blend_over(dst: &mut [u8], src: &[u8]) {
    let sa = src[3] as u32;
    if sa == 0 {
        return;
    }
    let da = dst[3] as u32;
    if sa == 255 || da == 0 {
        dst.copy_from_slice(&src[..4]);
        return;
    }
    // Alpha scaled by 255: the result is sa + da * (1 - sa) and each channel the alpha-weighted mix.
    let out_a255 = sa * 255 + da * (255 - sa);
    for c in 0..3 {
        let v = (src[c] as u32 * sa * 255 + dst[c] as u32 * da * (255 - sa) + out_a255 / 2) / out_a255;
        dst[c] = v.min(255) as u8;
    }
    dst[3] = ((out_a255 + 127) / 255).min(255) as u8;
}

/// The pivot of tile `id` as drawn with `flags`: the image point that sits on the diamond's center.
fn drawn_pivot(ts: &Tileset, id: u32, flags: u32) -> (i32, i32) {
    let (tw, th) = (ts.tile_w as i32, ts.tile_h as i32);
    let (mut px, mut py) = ts.pivot(id);
    if flags & FLIP_D != 0 && tw == th {
        std::mem::swap(&mut px, &mut py);
    }
    if flags & FLIP_X != 0 {
        px = tw - px;
    }
    if flags & FLIP_Y != 0 {
        py = th - py;
    }
    (px, py)
}

/// Canvas position of the top-left pixel of the tile image drawn in isometric cell `(i, j)`.
fn iso_image_origin(grid: &Iso, ts: &Tileset, cell: (i64, i64), value: u32) -> (i64, i64) {
    let (cx, cy) = grid.cell_center(cell);
    let (px, py) = drawn_pivot(ts, value & TILE_ID_MASK, value & FLIP_MASK);
    (cx - px as i64, cy - py as i64)
}

/// Composites an isometric tile grid into a full-canvas RGBA buffer. Cells are drawn back to front (by
/// `i + j`, then `i`) with source-over blending, so tall tiles overlap the cells behind them correctly.
/// `offset` is the canvas position of cell (0, 0)'s bounding box.
pub fn render_iso(grid: &TileGrid, ts: &Tileset, offset: (i32, i32), width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 4];
    let Some(iso) = ts.iso_grid(offset) else { return out };
    let (tw, th) = (ts.tile_w as usize, ts.tile_h as usize);
    grid.for_each_cell_in_draw_order(|i, j, value| {
        let id = (value & TILE_ID_MASK) as usize;
        if !ts.usable(id) {
            return;
        }
        let (ox, oy) = iso_image_origin(&iso, ts, (i, j), value);
        if ox >= width as i64 || oy >= height as i64 || ox + tw as i64 <= 0 || oy + th as i64 <= 0 {
            return;
        }
        let tile = &ts.tiles[id];
        let (cx0, cx1) = ((-ox).max(0) as usize, (width as i64 - ox).min(tw as i64) as usize);
        for cy in (-oy).max(0) as usize..(height as i64 - oy).min(th as i64) as usize {
            let py = (oy + cy as i64) as usize;
            for cx in cx0..cx1 {
                let (u, v) = source_pixel(cx, cy, value, tw, th);
                let s = (v * tw + u) * 4;
                if tile[s + 3] == 0 {
                    continue;
                }
                let d = (py * width + (ox + cx as i64) as usize) * 4;
                blend_over(&mut out[d..d + 4], &tile[s..s + 4]);
            }
        }
    });
    out
}

/// Composites a tile grid of either shape; `offset` is the layer's isometric offset (ignored for square tilesets).
pub fn render_cel(grid: &TileGrid, ts: &Tileset, offset: (i32, i32), width: usize, height: usize) -> Vec<u8> {
    if ts.is_iso() {
        render_iso(grid, ts, offset, width, height)
    } else {
        render_grid(grid, ts, width, height)
    }
}

/// The isometric offset of a layer (0, 0 for layers that are not isometric tilemaps).
fn layer_offset(s: &Sprite, layer: usize) -> (i32, i32) {
    s.layers.get(layer).map_or((0, 0), |l| l.offset)
}

fn make_cel(opacity: u8, grid: TileGrid, ts: &Tileset, offset: (i32, i32), w: usize, h: usize) -> Cel {
    Cel { opacity, pixels: render_cel(&grid, ts, offset, w, h), tiles: Some(grid), ..Default::default() }
}

fn tileset_of(s: &Sprite, layer: usize) -> Option<usize> {
    let l = s.layers.get(layer)?;
    (l.kind == KIND_TILEMAP && (l.tileset as usize) < s.tilesets.len()).then_some(l.tileset as usize)
}

/// Re-renders one tilemap cel's pixels from its grid.
fn refresh_cel(s: &mut Sprite, frame: usize, layer: usize) {
    let (w, h) = (s.width as usize, s.height as usize);
    let Some(ti) = tileset_of(s, layer) else { return };
    let off = layer_offset(s, layer);
    let ts = &s.tilesets[ti];
    let Some(Some(arc)) = s.cels.get_mut(frame).and_then(|r| r.get_mut(layer)) else { return };
    let cel = Arc::make_mut(arc);
    let empty = TileGrid::empty();
    cel.pixels = render_cel(cel.tiles.as_ref().unwrap_or(&empty), ts, off, w, h);
}

/// Whether a tilemap cel's pixels match what its grid renders to.
fn cel_in_sync(cel: &Cel, ts: &Tileset, offset: (i32, i32), w: usize, h: usize) -> bool {
    let empty = TileGrid::empty();
    let expected = render_cel(cel.tiles.as_ref().unwrap_or(&empty), ts, offset, w, h);
    cel.pixels.len() == expected.len()
        && cel.pixels.chunks_exact(4).zip(expected.chunks_exact(4)).all(|(a, b)| same_pixel(a, b))
}

/// Bounding box (x0, y0, x1, y1 exclusive) of non-transparent pixels outside the grid's area.
fn pixels_outside_grid(
    cel: &Cel,
    grid: &TileGrid,
    tw: u32,
    th: u32,
    w: usize,
    h: usize,
) -> Option<(i32, i32, i32, i32)> {
    let (gx0, gy0) = (grid.x as i64, grid.y as i64);
    let (gx1, gy1) = (gx0 + (grid.w * tw) as i64, gy0 + (grid.h * th) as i64);
    let mut bb: Option<(i32, i32, i32, i32)> = None;
    for y in 0..h {
        for x in 0..w {
            let inside = (x as i64) >= gx0 && (x as i64) < gx1 && (y as i64) >= gy0 && (y as i64) < gy1;
            if inside || cel.pixels[(y * w + x) * 4 + 3] == 0 {
                continue;
            }
            let (x, y) = (x as i32, y as i32);
            bb = Some(match bb {
                None => (x, y, x + 1, y + 1),
                Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x + 1), d.max(y + 1)),
            });
        }
    }
    bb
}

/// Writes pixel edits on a tilemap cel back into the tile images, then re-renders every cel
/// that shows a modified tile. `bbox` (x0, y0, x1, y1 exclusive) limits the area to inspect.
/// Empty cells that received pixels get a fresh tile; the cel's grid grows to hold pixels
/// painted outside it.
pub fn sync_cel(s: &mut Sprite, layer: usize, frame: usize, bbox: Option<(i32, i32, i32, i32)>) {
    let (w, h) = (s.width as usize, s.height as usize);
    let Some(ti) = tileset_of(s, layer) else { return };
    if s.tilesets[ti].is_iso() {
        sync_cel_iso(s, layer, frame, ti, bbox);
        return;
    }
    let Some(Some(arc)) = s.cels.get_mut(frame).and_then(|r| r.get_mut(layer)) else { return };
    let ts = &mut s.tilesets[ti];
    let (tw, th) = (ts.tile_w, ts.tile_h);
    let cel = Arc::make_mut(arc);
    let mut grid = cel.tiles.take().unwrap_or_else(TileGrid::empty);
    if cel.pixels.len() != w * h * 4 {
        cel.pixels.resize(w * h * 4, 0);
    }

    if let Some((x0, y0, x1, y1)) = pixels_outside_grid(cel, &grid, tw, th, w, h) {
        grid.grow_to_include(x0, y0, tw, th);
        grid.grow_to_include(x1 - 1, y1 - 1, tw, th);
    }

    let (bx0, by0, bx1, by1) = bbox.unwrap_or((i32::MIN / 2, i32::MIN / 2, i32::MAX / 2, i32::MAX / 2));
    let (cx0, cy0) = grid.cell_of(bx0.max(grid.x), by0.max(grid.y), tw, th);
    let (cx1, cy1) = grid.cell_of(bx1.saturating_sub(1).max(grid.x), by1.saturating_sub(1).max(grid.y), tw, th);
    let (cx0, cy0) = (cx0.max(0), cy0.max(0));
    let (cx1, cy1) = (cx1.min(grid.w as i32 - 1), cy1.min(grid.h as i32 - 1));

    let (twu, thu) = (tw as usize, th as usize);
    let mut changed = Vec::new();
    for ty in cy0..=cy1 {
        for tx in cx0..=cx1 {
            let i = ty as usize * grid.w as usize + tx as usize;
            let value = grid.tiles[i];
            let id = (value & TILE_ID_MASK) as usize;
            let usable = ts.usable(id);
            let mut diffs: Vec<(usize, usize, [u8; 4])> = Vec::new();
            for cy in 0..thu {
                let py = grid.y as i64 + (ty as usize * thu + cy) as i64;
                if py < 0 || py >= h as i64 {
                    continue;
                }
                for cx in 0..twu {
                    let px = grid.x as i64 + (tx as usize * twu + cx) as i64;
                    if px < 0 || px >= w as i64 {
                        continue;
                    }
                    let a = (py as usize * w + px as usize) * 4;
                    let actual = &cel.pixels[a..a + 4];
                    let expected: [u8; 4] = if usable {
                        let (u, v) = source_pixel(cx, cy, value, twu, thu);
                        ts.tiles[id][(v * twu + u) * 4..][..4].try_into().unwrap()
                    } else {
                        [0; 4]
                    };
                    if !same_pixel(actual, &expected) {
                        diffs.push((cx, cy, actual.try_into().unwrap()));
                    }
                }
            }
            if diffs.is_empty() {
                continue;
            }
            let (id, flags) = if usable {
                (id, value & FLIP_MASK)
            } else {
                if ts.tiles.len() >= MAX_TILES {
                    continue;
                }
                let n = ts.add_blank();
                grid.tiles[i] = n;
                (n as usize, 0)
            };
            let tile = Arc::make_mut(&mut ts.tiles[id]);
            for (cx, cy, c) in diffs {
                let (u, v) = source_pixel(cx, cy, flags, twu, thu);
                tile[(v * twu + u) * 4..][..4].copy_from_slice(&c);
            }
            changed.push(id as u32);
        }
    }

    cel.pixels = render_grid(&grid, ts, w, h);
    cel.tiles = Some(grid);

    refresh_users(s, ti, (frame, layer), &changed);
}

/// Re-renders every cel (other than `except`) that shows one of the modified tiles of tileset `ti`.
fn refresh_users(s: &mut Sprite, ti: usize, except: (usize, usize), changed: &[u32]) {
    if changed.is_empty() {
        return;
    }
    let mut stale = Vec::new();
    for (f, row) in s.cels.iter().enumerate() {
        for (l, cel) in row.iter().enumerate() {
            if (f, l) == except || tileset_of(s, l) != Some(ti) {
                continue;
            }
            let uses = cel
                .as_ref()
                .and_then(|c| c.tiles.as_ref())
                .is_some_and(|g| g.tiles.iter().any(|v| changed.contains(&(v & TILE_ID_MASK))));
            if uses {
                stale.push((f, l));
            }
        }
    }
    for (f, l) in stale {
        refresh_cel(s, f, l);
    }
}

/// The tile that owns canvas pixel (`px`, `py`) of an isometric tilemap cel, for writing a painted pixel back:
/// the front-most cell whose tile image shows a visible pixel there, else the cell whose diamond contains the
/// pixel. Returns the cell and the pixel's position inside the (drawn, i.e. flipped) tile image if it lies in it.
fn iso_owner(grid: &TileGrid, ts: &Tileset, iso: &Iso, px: i32, py: i32) -> ((i64, i64), Option<(usize, usize)>) {
    let (tw, th) = (ts.tile_w as i64, ts.tile_h as i64);
    let reach = tw.max(th);
    let (lo_i, hi_i, lo_j, hi_j) = iso.cells_touching(px as i64, py as i64, px as i64 + 1, py as i64 + 1, reach);
    let mut best: Option<((i64, i64), (usize, usize))> = None;
    for j in lo_j..=hi_j {
        for i in lo_i..=hi_i {
            let value = grid.cell_value(i, j);
            let id = (value & TILE_ID_MASK) as usize;
            if !ts.usable(id) {
                continue;
            }
            let (ox, oy) = iso_image_origin(iso, ts, (i, j), value);
            let (cx, cy) = (px as i64 - ox, py as i64 - oy);
            if cx < 0 || cy < 0 || cx >= tw || cy >= th {
                continue;
            }
            let (u, v) = source_pixel(cx as usize, cy as usize, value, tw as usize, th as usize);
            if ts.tiles[id][(v * tw as usize + u) * 4 + 3] == 0 {
                continue;
            }
            if best.is_none_or(|(b, _)| iso::draw_order(b, (i, j)).is_lt()) {
                best = Some(((i, j), (cx as usize, cy as usize)));
            }
        }
    }
    if let Some((cell, at)) = best {
        return (cell, Some(at));
    }
    let cell = iso.cell_at(px, py);
    let value = grid.cell_value(cell.0, cell.1);
    let shown = if ts.usable((value & TILE_ID_MASK) as usize) { value } else { 0 };
    let (ox, oy) = iso_image_origin(iso, ts, cell, shown);
    let (cx, cy) = (px as i64 - ox, py as i64 - oy);
    let inside = cx >= 0 && cy >= 0 && cx < tw && cy < th;
    (cell, inside.then_some((cx as usize, cy as usize)))
}

/// `sync_cel` for isometric tilemaps. Tiles overlap, so a painted pixel is written to the tile that owns it
/// (see [`iso_owner`]); a pixel on an empty cell gets a fresh tile and the grid grows to hold it.
fn sync_cel_iso(s: &mut Sprite, layer: usize, frame: usize, ti: usize, bbox: Option<(i32, i32, i32, i32)>) {
    let (w, h) = (s.width as usize, s.height as usize);
    let off = layer_offset(s, layer);
    let Some(Some(arc)) = s.cels.get_mut(frame).and_then(|r| r.get_mut(layer)) else { return };
    let ts = &mut s.tilesets[ti];
    let Some(iso) = ts.iso_grid(off) else { return };
    let (tw, th) = (ts.tile_w as usize, ts.tile_h as usize);
    let cel = Arc::make_mut(arc);
    let mut grid = cel.tiles.take().unwrap_or_else(TileGrid::empty);
    if cel.pixels.len() != w * h * 4 {
        cel.pixels.resize(w * h * 4, 0);
    }
    let expected = render_iso(&grid, ts, off, w, h);
    let (bx0, by0, bx1, by1) = bbox.unwrap_or((0, 0, w as i32, h as i32));
    let (bx0, by0, bx1, by1) =
        (bx0.max(0) as usize, by0.max(0) as usize, (bx1.max(0) as usize).min(w), (by1.max(0) as usize).min(h));
    let mut diffs: Vec<(usize, usize, [u8; 4])> = Vec::new();
    for y in by0..by1 {
        for x in bx0..bx1 {
            let i = (y * w + x) * 4;
            if !same_pixel(&cel.pixels[i..i + 4], &expected[i..i + 4]) {
                diffs.push((x, y, cel.pixels[i..i + 4].try_into().unwrap()));
            }
        }
    }
    let mut changed: Vec<u32> = Vec::new();
    for (x, y, c) in diffs {
        let (cell, at) = iso_owner(&grid, ts, &iso, x as i32, y as i32);
        let Some((cx, cy)) = at else { continue };
        let mut value = grid.cell_value(cell.0, cell.1);
        if !ts.usable((value & TILE_ID_MASK) as usize) {
            if ts.tiles.len() >= MAX_TILES || c[3] == 0 {
                continue;
            }
            grid.grow_to_include_cell(cell.0, cell.1);
            let Some(k) = grid.index_cell(cell.0, cell.1) else { continue };
            let n = ts.add_blank();
            grid.tiles[k] = n;
            value = n;
            // The image of a fresh tile is placed with the default pivot, which may differ from where the
            // owner search looked; recompute the position inside it.
            let (ox, oy) = iso_image_origin(&iso, ts, cell, value);
            let (nx, ny) = (x as i64 - ox, y as i64 - oy);
            if nx < 0 || ny < 0 || nx >= tw as i64 || ny >= th as i64 {
                continue;
            }
            let tile = Arc::make_mut(&mut ts.tiles[n as usize]);
            tile[(ny as usize * tw + nx as usize) * 4..][..4].copy_from_slice(&c);
            changed.push(n);
            continue;
        }
        let id = (value & TILE_ID_MASK) as usize;
        let (u, v) = source_pixel(cx, cy, value, tw, th);
        let tile = Arc::make_mut(&mut ts.tiles[id]);
        tile[(v * tw + u) * 4..][..4].copy_from_slice(&c);
        if !changed.contains(&(id as u32)) {
            changed.push(id as u32);
        }
    }
    cel.pixels = render_iso(&grid, ts, off, w, h);
    cel.tiles = Some(grid);
    refresh_users(s, ti, (frame, layer), &changed);
}

/// Writes pixel edits of every tilemap cel back into its tiles, in place (see `sync_cel`).
pub fn reconcile(s: &mut Sprite) {
    let (w, h) = (s.width as usize, s.height as usize);
    let mut todo = Vec::new();
    for (f, row) in s.cels.iter().enumerate() {
        for (l, cel) in row.iter().enumerate() {
            let (Some(ti), Some(cel)) = (tileset_of(s, l), cel) else { continue };
            if cel.tiles.is_none() || !cel_in_sync(cel, &s.tilesets[ti], layer_offset(s, l), w, h) {
                todo.push((f, l));
            }
        }
    }
    for (f, l) in todo {
        sync_cel(s, l, f, None);
    }
}

/// Drops the tiles at the end of the tileset that no cel refers to, but never below index `keep`. Drawing calls this
/// with the tile count from before the gesture, so tiles the gesture allocated and then emptied again do not pile up.
pub fn trim_new_unused(s: &mut Sprite, layer: usize, keep: usize) {
    let Some(ti) = tileset_of(s, layer) else { return };
    let mut used = std::collections::HashSet::new();
    for row in &s.cels {
        for (l, cel) in row.iter().enumerate() {
            if tileset_of(s, l) != Some(ti) {
                continue;
            }
            if let Some(g) = cel.as_ref().and_then(|c| c.tiles.as_ref()) {
                used.extend(g.tiles.iter().map(|v| (v & TILE_ID_MASK) as usize));
            }
        }
    }
    let ts = &mut s.tilesets[ti];
    while ts.tiles.len() > keep.max(1) && !used.contains(&(ts.tiles.len() - 1)) {
        ts.tiles.pop();
    }
}

/// Returns the sprite unchanged, or a copy whose tilemap cels are reconciled with their pixels.
/// Used before saving so edits made by tools that don't know about tilemaps are not lost.
/// Unused tilesets are kept, as Aseprite keeps them.
pub fn normalized(s: &Sprite) -> Cow<'_, Sprite> {
    let (w, h) = (s.width as usize, s.height as usize);
    let mut todo = Vec::new();
    for (f, row) in s.cels.iter().enumerate() {
        for (l, cel) in row.iter().enumerate() {
            let (Some(ti), Some(cel)) = (tileset_of(s, l), cel) else { continue };
            if cel.tiles.is_none() || !cel_in_sync(cel, &s.tilesets[ti], layer_offset(s, l), w, h) {
                todo.push((f, l));
            }
        }
    }
    let far = s.cels.iter().flatten().flatten().any(|c| c.tiles.as_ref().is_some_and(|g| !fits_i16(g)));
    if todo.is_empty() && !far {
        return Cow::Borrowed(s);
    }
    let mut copy = s.clone();
    for (f, l) in todo {
        sync_cel(&mut copy, l, f, None);
    }
    if far {
        trim_far_grids(&mut copy);
    }
    Cow::Owned(copy)
}

fn fits_i16(g: &TileGrid) -> bool {
    (i16::MIN as i32..=i16::MAX as i32).contains(&g.x) && (i16::MIN as i32..=i16::MAX as i32).contains(&g.y)
}

/// Cel positions are 16-bit in the file: drop rows and columns that lie wholly outside the
/// canvas from grids whose origin does not fit (this happens after large canvas crops).
fn trim_far_grids(s: &mut Sprite) {
    let (w, h) = (s.width as i64, s.height as i64);
    for l in 0..s.layers.len() {
        let Some(ti) = tileset_of(s, l) else { continue };
        let (tw, th) = (s.tilesets[ti].tile_w as i64, s.tilesets[ti].tile_h as i64);
        for row in s.cels.iter_mut() {
            let Some(Some(arc)) = row.get_mut(l) else { continue };
            if !arc.tiles.as_ref().is_some_and(|g| !fits_i16(g)) {
                continue;
            }
            let g = Arc::make_mut(arc).tiles.as_mut().unwrap();
            let visible =
                |t: i64, origin: i64, size: i64, extent: i64| origin + (t + 1) * size > 0 && origin + t * size < extent;
            let cols: Vec<usize> = (0..g.w as usize).filter(|&c| visible(c as i64, g.x as i64, tw, w)).collect();
            let rows: Vec<usize> = (0..g.h as usize).filter(|&r| visible(r as i64, g.y as i64, th, h)).collect();
            let (Some(&c0), Some(&r0)) = (cols.first(), rows.first()) else {
                *g = TileGrid::empty();
                continue;
            };
            let (nw, nh) = (cols.len(), rows.len());
            let mut tiles = Vec::with_capacity(nw * nh);
            for &r in &rows {
                for &c in &cols {
                    tiles.push(g.tiles[r * g.w as usize + c]);
                }
            }
            g.x += c0 as i32 * tw as i32;
            g.y += r0 as i32 * th as i32;
            g.w = nw as u32;
            g.h = nh as u32;
            g.tiles = tiles;
        }
    }
}

// MARK: Editing

/// Inserts a tilemap layer above `above`, backed by a new empty tileset. Returns its index.
pub fn add_tilemap_layer(s: &mut Sprite, above: usize, tile_w: u32, tile_h: u32) -> usize {
    let at = s.add_layer(above);
    let n = s.tilesets.len();
    s.tilesets.push(Tileset::new(format!("Tileset {}", n + 1), tile_w, tile_h));
    let layer = &mut s.layers[at];
    layer.kind = KIND_TILEMAP;
    layer.tileset = n as u32;
    layer.name = format!("Tilemap {}", at + 1);
    at
}

/// Inserts an isometric tilemap layer above `above`, backed by a new empty tileset of `tile_w` x `tile_h`
/// images on `cell_w` x `cell_h` diamonds. `offset` is the canvas position of cell (0, 0)'s bounding box.
/// Returns the layer's index.
pub fn add_iso_tilemap_layer(
    s: &mut Sprite,
    above: usize,
    tile_w: u32,
    tile_h: u32,
    cell_w: u32,
    cell_h: u32,
    offset: (i32, i32),
) -> usize {
    let at = s.add_layer(above);
    let n = s.tilesets.len();
    s.tilesets.push(Tileset::new_iso(format!("Tileset {}", n + 1), tile_w, tile_h, cell_w, cell_h));
    let layer = &mut s.layers[at];
    layer.kind = KIND_TILEMAP;
    layer.tileset = n as u32;
    layer.name = format!("Tilemap {}", at + 1);
    layer.offset = offset;
    at
}

/// Moves an isometric tilemap layer's grid: `offset` is the canvas position of cell (0, 0)'s bounding box.
/// Layers at different elevations use offsets that differ by the elevation (half a cell is `cell_h / 2`
/// pixels). Returns false for layers that are not isometric tilemaps.
pub fn set_layer_offset(s: &mut Sprite, layer: usize, offset: (i32, i32)) -> bool {
    let Some(ti) = tileset_of(s, layer) else { return false };
    if !s.tilesets[ti].is_iso() || offset.0.unsigned_abs() > 1 << 24 || offset.1.unsigned_abs() > 1 << 24 {
        return false;
    }
    if s.layers[layer].offset == offset {
        return true;
    }
    // Pixel edits made before the move belong to the old position.
    for f in 0..s.frame_count() {
        if s.cels[f][layer].is_some() {
            sync_cel(s, layer, f, None);
        }
    }
    s.layers[layer].offset = offset;
    for f in 0..s.frame_count() {
        refresh_cel(s, f, layer);
    }
    true
}

/// Sets (`Some`) or clears (`None`, back to the default) the pivot of a tile of an isometric tileset: the point
/// of the tile image, in pixels from its top-left corner, that sits on the center of the cell's diamond. Tiles
/// taller than the diamond default to bottom-aligned (see [`Tileset::pivot`]). Re-renders the layers using it.
pub fn set_tile_pivot(s: &mut Sprite, ts: usize, tile: u32, pivot: Option<(i32, i32)>) -> bool {
    let Some(t) = s.tilesets.get_mut(ts) else { return false };
    let Some(spec) = t.iso.as_mut() else { return false };
    if tile as usize >= t.tiles.len() || pivot.is_some_and(|(x, y)| x.abs() > 1 << 14 || y.abs() > 1 << 14) {
        return false;
    }
    let mut test = spec.pivots.clone();
    match pivot {
        Some(p) => test.insert(tile, p),
        None => test.remove(&tile),
    };
    if test == spec.pivots {
        return true;
    }
    for f in 0..s.frame_count() {
        for l in 0..s.layers.len() {
            if tileset_of(s, l) == Some(ts) && s.cels[f][l].is_some() {
                sync_cel(s, l, f, None);
            }
        }
    }
    if let Some(spec) = s.tilesets[ts].iso.as_mut() {
        spec.pivots = test;
    }
    refresh_tileset_users(s, ts);
    true
}

/// Whether any cel shows a tile of tileset `ts`.
fn tileset_has_tiles(s: &Sprite, ts: usize) -> bool {
    s.layers.iter().enumerate().any(|(l, _)| {
        tileset_of(s, l) == Some(ts)
            && s.cels.iter().any(|row| {
                row.get(l)
                    .and_then(|c| c.as_ref())
                    .and_then(|c| c.tiles.as_ref())
                    .is_some_and(|g| g.tiles.iter().any(|&v| v & TILE_ID_MASK != 0))
            })
    })
}

/// Turns a tileset into an isometric one (`Some((cell_w, cell_h))`) or back into a square one (`None`). Tiles keep
/// their images, but a grid of tiles means something else in the other shape, so this only works while no layer
/// shows any tile of the tileset. Returns false otherwise.
pub fn set_tileset_iso(s: &mut Sprite, ts: usize, cell: Option<(u32, u32)>) -> bool {
    if ts >= s.tilesets.len() || tileset_has_tiles(s, ts) {
        return false;
    }
    let spec = cell.map(|(w, h)| IsoSpec::new(w, h));
    if s.tilesets[ts].iso == spec {
        return true;
    }
    s.tilesets[ts].iso = spec;
    for l in 0..s.layers.len() {
        if tileset_of(s, l) == Some(ts) {
            if s.tilesets[ts].iso.is_none() {
                s.layers[l].offset = (0, 0);
            }
            for f in 0..s.frame_count() {
                if let Some(Some(arc)) = s.cels.get_mut(f).and_then(|r| r.get_mut(l)) {
                    Arc::make_mut(arc).tiles = None;
                }
            }
        }
    }
    true
}

/// Places a tile value (index | flip flags) in the cell under canvas pixel (`px`, `py`).
/// Tile index 0 clears the cell. Returns whether anything changed.
pub fn stamp(s: &mut Sprite, layer: usize, frame: usize, px: i32, py: i32, value: u32) -> bool {
    let (w, h) = (s.width as i32, s.height as i32);
    let Some(ti) = tileset_of(s, layer) else { return false };
    if frame >= s.frame_count() || px < 0 || py < 0 || px >= w || py >= h {
        return false;
    }
    let value = value & (TILE_ID_MASK | FLIP_MASK);
    if (value & TILE_ID_MASK) as usize >= s.tilesets[ti].tiles.len() {
        return false;
    }
    if s.tilesets[ti].is_iso() {
        let Some(iso) = s.tilesets[ti].iso_grid(layer_offset(s, layer)) else { return false };
        let (i, j) = iso.cell_at(px, py);
        return stamp_cell(s, layer, frame, i, j, value);
    }
    if s.cels[frame][layer].as_ref().is_some_and(|c| c.tiles.is_none()) {
        sync_cel(s, layer, frame, None);
    }
    let (tw, th) = (s.tilesets[ti].tile_w, s.tilesets[ti].tile_h);
    let (uw, uh) = (s.width as usize, s.height as usize);
    let slot = &mut s.cels[frame][layer];
    if slot.is_none() && value & TILE_ID_MASK == 0 {
        return false;
    }
    let arc =
        slot.get_or_insert_with(|| Arc::new(Cel { opacity: 255, pixels: vec![0; uw * uh * 4], ..Default::default() }));
    let cel = Arc::make_mut(arc);
    let mut grid = cel.tiles.take().unwrap_or_else(TileGrid::empty);
    let (tx, ty) = grid.cell_of(px, py, tw, th);
    let mut changed = false;
    if grid.index(tx, ty).is_none() && value & TILE_ID_MASK != 0 {
        changed = grid.grow_to_include(px, py, tw, th);
    }
    let (tx, ty) = grid.cell_of(px, py, tw, th);
    if let Some(i) = grid.index(tx, ty) {
        if grid.tiles[i] != value {
            grid.tiles[i] = value;
            changed = true;
        }
    }
    if changed {
        cel.pixels = render_grid(&grid, &s.tilesets[ti], uw, uh);
    }
    cel.tiles = Some(grid);
    changed
}

/// Places a tile value in isometric cell `(i, j)` of an isometric tilemap layer, growing the layer's grid when
/// the cell lies outside it. Tile index 0 clears the cell. Returns whether anything changed.
pub fn stamp_cell(s: &mut Sprite, layer: usize, frame: usize, i: i64, j: i64, value: u32) -> bool {
    let Some(ti) = tileset_of(s, layer) else { return false };
    if !s.tilesets[ti].is_iso() || frame >= s.frame_count() || i.abs() > iso::MAX_CELL || j.abs() > iso::MAX_CELL {
        return false;
    }
    let value = value & (TILE_ID_MASK | FLIP_MASK);
    if (value & TILE_ID_MASK) as usize >= s.tilesets[ti].tiles.len() {
        return false;
    }
    if s.cels[frame][layer].as_ref().is_some_and(|c| c.tiles.is_none()) {
        sync_cel(s, layer, frame, None);
    }
    let off = layer_offset(s, layer);
    let (uw, uh) = (s.width as usize, s.height as usize);
    let slot = &mut s.cels[frame][layer];
    if slot.is_none() && value & TILE_ID_MASK == 0 {
        return false;
    }
    let arc =
        slot.get_or_insert_with(|| Arc::new(Cel { opacity: 255, pixels: vec![0; uw * uh * 4], ..Default::default() }));
    let cel = Arc::make_mut(arc);
    let mut grid = cel.tiles.take().unwrap_or_else(TileGrid::empty);
    let mut changed = false;
    if grid.index_cell(i, j).is_none() && value & TILE_ID_MASK != 0 {
        changed = grid.grow_to_include_cell(i, j);
    }
    if let Some(k) = grid.index_cell(i, j) {
        if grid.tiles[k] != value {
            grid.tiles[k] = value;
            changed = true;
        }
    }
    if changed {
        cel.pixels = render_iso(&grid, &s.tilesets[ti], off, uw, uh);
    }
    cel.tiles = Some(grid);
    changed
}

/// Replaces references to tiles that no longer exist with the empty tile and refreshes the pixels
/// (for cels that came from a clipboard older than the tileset's current contents).
pub fn repair_cel(s: &mut Sprite, layer: usize, frame: usize) {
    let (w, h) = (s.width as usize, s.height as usize);
    let Some(ti) = tileset_of(s, layer) else { return };
    let off = layer_offset(s, layer);
    let n = s.tilesets[ti].tiles.len();
    let Some(Some(arc)) = s.cels.get_mut(frame).and_then(|r| r.get_mut(layer)) else { return };
    if arc.tiles.as_ref().is_none_or(|g| g.tiles.iter().all(|v| ((v & TILE_ID_MASK) as usize) < n)) {
        return;
    }
    let cel = Arc::make_mut(arc);
    let Some(mut grid) = cel.tiles.take() else { return };
    for v in &mut grid.tiles {
        if (*v & TILE_ID_MASK) as usize >= n {
            *v = 0;
        }
    }
    cel.pixels = render_cel(&grid, &s.tilesets[ti], off, w, h);
    cel.tiles = Some(grid);
}

/// The tile value under a canvas pixel of a tilemap cel (0 if empty or not a tilemap). For isometric tilemaps
/// this is the tile of the cell whose diamond contains the pixel; see [`tile_hit_at`] for the visible tile.
pub fn tile_at(s: &Sprite, layer: usize, frame: usize, px: i32, py: i32) -> u32 {
    let Some(ti) = tileset_of(s, layer) else { return 0 };
    let ts = &s.tilesets[ti];
    let grid = s.cels.get(frame).and_then(|r| r.get(layer)).and_then(|c| c.as_ref()).and_then(|c| c.tiles.as_ref());
    let Some(g) = grid else { return 0 };
    if let Some(iso) = ts.iso_grid(layer_offset(s, layer)) {
        let (i, j) = iso.cell_at(px, py);
        return g.cell_value(i, j);
    }
    let (tx, ty) = g.cell_of(px, py, ts.tile_w, ts.tile_h);
    g.index(tx, ty).map_or(0, |i| g.tiles[i])
}

/// The cell `(i, j)` of an isometric tilemap layer that contains a canvas pixel; for square tilemaps the
/// grid column and row (`pixel / tile size`, relative to the canvas, not the cel's grid).
pub fn cell_at(s: &Sprite, layer: usize, px: i32, py: i32) -> Option<(i64, i64)> {
    let ts = &s.tilesets[tileset_of(s, layer)?];
    Some(match ts.iso_grid(layer_offset(s, layer)) {
        Some(iso) => iso.cell_at(px, py),
        None => ((px as i64).div_euclid(ts.tile_w as i64), (py as i64).div_euclid(ts.tile_h as i64)),
    })
}

/// Canvas position of the top-left corner of cell `(i, j)`'s box (for square tilemaps `(i * tile_w, j * tile_h)`).
pub fn cell_origin(s: &Sprite, layer: usize, i: i64, j: i64) -> Option<(i64, i64)> {
    let ts = &s.tilesets[tileset_of(s, layer)?];
    Some(match ts.iso_grid(layer_offset(s, layer)) {
        Some(iso) => iso.cell_origin((i, j)),
        None => (i * ts.tile_w as i64, j * ts.tile_h as i64),
    })
}

/// The tile value in cell `(i, j)` of a tilemap layer's cel (0 if empty). Cell coordinates as in [`cell_at`], but
/// for square tilemaps relative to the canvas origin (a square grid that starts elsewhere is addressed by canvas).
pub fn tile_in_cell(s: &Sprite, layer: usize, frame: usize, i: i64, j: i64) -> u32 {
    let Some(ti) = tileset_of(s, layer) else { return 0 };
    let ts = &s.tilesets[ti];
    if ts.is_iso() {
        let grid = s.cels.get(frame).and_then(|r| r.get(layer)).and_then(|c| c.as_ref()).and_then(|c| c.tiles.as_ref());
        return grid.map_or(0, |g| g.cell_value(i, j));
    }
    match (i32::try_from(i * ts.tile_w as i64), i32::try_from(j * ts.tile_h as i64)) {
        (Ok(px), Ok(py)) => tile_at(s, layer, frame, px, py),
        _ => 0,
    }
}

/// The tile you see at a canvas pixel of an isometric tilemap: the front-most tile whose image has a visible
/// pixel there (a cliff drawn over the cell behind it), as `(cell, tile value)`. Falls back to the cell whose
/// diamond contains the pixel. `None` when that cell is empty too, or the layer is not isometric.
pub fn tile_hit_at(s: &Sprite, layer: usize, frame: usize, px: i32, py: i32) -> Option<((i64, i64), u32)> {
    let ts = &s.tilesets[tileset_of(s, layer)?];
    let iso = ts.iso_grid(layer_offset(s, layer))?;
    let grid = s.cels.get(frame)?.get(layer)?.as_ref()?.tiles.as_ref()?;
    let (cell, _) = iso_owner(grid, ts, &iso, px, py);
    let v = grid.cell_value(cell.0, cell.1);
    ts.usable((v & TILE_ID_MASK) as usize).then_some((cell, v))
}

/// Converts a normal layer into a tilemap layer with a new tileset, deduplicating identical
/// tiles across all frames. Returns false for groups and layers that are already tilemaps.
pub fn convert_to_tilemap(s: &mut Sprite, layer: usize, tile_w: u32, tile_h: u32) -> bool {
    convert_to_tilemap_with(s, layer, tile_w, tile_h, false)
}

/// Like [`convert_to_tilemap`]. With `flip_aware`, a tile that is a flipped or transposed copy
/// of an existing tile reuses it with the matching flip flags instead of getting a new tile.
pub fn convert_to_tilemap_with(s: &mut Sprite, layer: usize, tile_w: u32, tile_h: u32, flip_aware: bool) -> bool {
    match s.layers.get(layer) {
        Some(l) if !l.is_group() && l.kind != KIND_TILEMAP => {}
        _ => return false,
    }
    let n = s.tilesets.len();
    let mut ts = Tileset::new(format!("Tileset {}", n + 1), tile_w, tile_h);
    let (tw, th) = (ts.tile_w as usize, ts.tile_h as usize);
    let (w, h) = (s.width as usize, s.height as usize);
    let (cols, rows) = (w.div_ceil(tw), h.div_ceil(th));
    if cols * rows > MAX_CELLS {
        return false;
    }
    let mut seen: HashMap<Vec<u8>, u32> = HashMap::new();

    // Frames sharing one cel buffer keep sharing one converted cel.
    let mut converted: Vec<(u8, TileGrid, crate::document::slices::UserData)> = Vec::new();
    let mut by_source: HashMap<*const Cel, usize> = HashMap::new();
    let mut assign: Vec<(usize, usize)> = Vec::new();
    for f in 0..s.frame_count() {
        let Some(arc) = &s.cels[f][layer] else { continue };
        let key = Arc::as_ptr(arc);
        if let Some(&ci) = by_source.get(&key) {
            assign.push((f, ci));
            continue;
        }
        let mut grid = TileGrid::covering(s.width, s.height, tw as u32, th as u32);
        for ty in 0..rows {
            for tx in 0..cols {
                let mut tile = vec![0u8; tw * th * 4];
                let mut opaque = false;
                for cy in 0..th {
                    let y = ty * th + cy;
                    if y >= h {
                        break;
                    }
                    for cx in 0..tw {
                        let x = tx * tw + cx;
                        if x >= w {
                            break;
                        }
                        let src = &arc.pixels[(y * w + x) * 4..][..4];
                        if src[3] != 0 {
                            tile[(cy * tw + cx) * 4..][..4].copy_from_slice(src);
                            opaque = true;
                        }
                    }
                }
                if !opaque {
                    continue;
                }
                let variants: &[u32] = if !flip_aware {
                    &[0]
                } else if tw == th {
                    &[
                        0,
                        FLIP_X,
                        FLIP_Y,
                        FLIP_X | FLIP_Y,
                        FLIP_D,
                        FLIP_D | FLIP_X,
                        FLIP_D | FLIP_Y,
                        FLIP_D | FLIP_X | FLIP_Y,
                    ]
                } else {
                    &[0, FLIP_X, FLIP_Y, FLIP_X | FLIP_Y]
                };
                let found = variants.iter().find_map(|&f| {
                    let stored = if f == 0 { Cow::Borrowed(&tile) } else { Cow::Owned(untransform(&tile, f, tw, th)) };
                    seen.get(stored.as_ref()).map(|&id| id | f)
                });
                let value = match found {
                    Some(v) => v,
                    None => {
                        let id = ts.tiles.len() as u32;
                        ts.tiles.push(Arc::new(tile.clone()));
                        seen.insert(tile, id);
                        id
                    }
                };
                grid.tiles[ty * cols + tx] = value;
            }
        }
        by_source.insert(key, converted.len());
        assign.push((f, converted.len()));
        converted.push((arc.opacity, grid, arc.user_data.clone()));
    }
    let cels: Vec<Arc<Cel>> = converted
        .into_iter()
        .map(|(o, g, u)| Arc::new(Cel { user_data: u, ..make_cel(o, g, &ts, (0, 0), w, h) }))
        .collect();
    for (f, ci) in assign {
        s.cels[f][layer] = Some(cels[ci].clone());
    }
    s.tilesets.push(ts);
    s.layers[layer].kind = KIND_TILEMAP;
    s.layers[layer].tileset = n as u32;
    true
}

/// Tile image `img` as the tile that shows `img` when drawn with `flags` (inverse of the
/// flip mapping used by [`render_grid`]).
fn untransform(img: &[u8], flags: u32, tw: usize, th: usize) -> Vec<u8> {
    let mut out = vec![0u8; img.len()];
    for cy in 0..th {
        for cx in 0..tw {
            let (u, v) = source_pixel(cx, cy, flags, tw, th);
            out[(v * tw + u) * 4..][..4].copy_from_slice(&img[(cy * tw + cx) * 4..][..4]);
        }
    }
    out
}

/// Drops a layer's tile grids and makes it a normal layer (its pixels are kept).
fn flatten_layer(s: &mut Sprite, layer: usize) {
    let Some(ti) = tileset_of(s, layer) else { return };
    let (w, h) = (s.width as usize, s.height as usize);
    for f in 0..s.frame_count() {
        if s.cels[f][layer]
            .as_ref()
            .is_some_and(|c| c.tiles.is_none() || !cel_in_sync(c, &s.tilesets[ti], layer_offset(s, layer), w, h))
        {
            sync_cel(s, layer, f, None);
        }
        if let Some(arc) = &mut s.cels[f][layer] {
            Arc::make_mut(arc).tiles = None;
        }
    }
    s.layers[layer].kind = 0;
    s.layers[layer].tileset = 0;
    s.layers[layer].offset = (0, 0);
}

fn tileset_in_use(s: &Sprite, ti: usize) -> bool {
    s.layers.iter().any(|l| l.kind == KIND_TILEMAP && l.tileset as usize == ti)
}

/// Removes a tileset and renumbers the layers that use later ones.
fn remove_tileset(s: &mut Sprite, ti: usize) {
    s.tilesets.remove(ti);
    for l in &mut s.layers {
        if l.kind == KIND_TILEMAP && l.tileset as usize > ti {
            l.tileset -= 1;
        }
    }
}

/// Removes tilesets no tilemap layer uses. Returns whether any were removed.
pub fn remove_unused_tilesets(s: &mut Sprite) -> bool {
    let mut any = false;
    let mut i = 0;
    while i < s.tilesets.len() {
        if tileset_in_use(s, i) {
            i += 1;
        } else {
            remove_tileset(s, i);
            any = true;
        }
    }
    any
}

/// Turns a tilemap layer back into a normal layer, keeping its rendered pixels.
pub fn convert_to_normal(s: &mut Sprite, layer: usize) -> bool {
    let Some(ti) = tileset_of(s, layer) else { return false };
    flatten_layer(s, layer);
    if !tileset_in_use(s, ti) {
        remove_tileset(s, ti);
    }
    true
}

/// Re-renders the pixels of every tilemap cel from its tile grid.
pub fn refresh_all(s: &mut Sprite) {
    for l in 0..s.layers.len() {
        if tileset_of(s, l).is_some() {
            for f in 0..s.frame_count() {
                refresh_cel(s, f, l);
            }
        }
    }
}

/// Re-renders the cels of the layers using tileset `ti`.
pub(crate) fn refresh_tileset_users(s: &mut Sprite, ti: usize) {
    for l in 0..s.layers.len() {
        if tileset_of(s, l) == Some(ti) {
            for f in 0..s.frame_count() {
                refresh_cel(s, f, l);
            }
        }
    }
}

/// Applies a per-pixel color mapping to the tile images of tilesets used by a layer accepted
/// by `layer_ok`, then re-renders the tilemap cels. Pixel positions are tile-local.
pub fn map_tiles(
    s: &mut Sprite,
    layer_ok: &dyn Fn(usize) -> bool,
    f: &mut dyn FnMut(usize, usize, [u8; 4]) -> [u8; 4],
) {
    for ti in 0..s.tilesets.len() {
        if !(0..s.layers.len()).any(|l| tileset_of(s, l) == Some(ti) && layer_ok(l)) {
            continue;
        }
        let tw = s.tilesets[ti].tile_w as usize;
        for tile in &mut s.tilesets[ti].tiles {
            let t = Arc::make_mut(tile);
            for (i, px) in t.chunks_exact_mut(4).enumerate() {
                let out = f(i % tw, i / tw, [px[0], px[1], px[2], px[3]]);
                px.copy_from_slice(&out);
            }
        }
    }
    refresh_all(s);
}

/// Shifts a grid's origin; used when the canvas is resized or cropped.
pub fn shifted(grid: Option<&TileGrid>, dx: i32, dy: i32) -> Option<TileGrid> {
    grid.map(|g| TileGrid { x: g.x.saturating_add(dx), y: g.y.saturating_add(dy), ..g.clone() })
}

/// Per layer: whether it is an isometric tilemap (its grid is in cells, its placement in `Layer::offset`).
pub fn iso_layers(s: &Sprite) -> Vec<bool> {
    (0..s.layers.len()).map(|l| tileset_of(s, l).is_some_and(|ti| s.tilesets[ti].is_iso())).collect()
}

/// Moves the grids of all isometric tilemap layers by (`dx`, `dy`) pixels (the canvas was resized or cropped).
/// The caller re-renders the cels afterwards (`refresh_all`).
pub fn shift_iso_offsets(s: &mut Sprite, dx: i32, dy: i32) {
    for (l, iso) in iso_layers(s).into_iter().enumerate() {
        if iso {
            let o = s.layers[l].offset;
            s.layers[l].offset = (o.0.saturating_add(dx), o.1.saturating_add(dy));
        }
    }
}

/// Prepares tilemaps for `filters::scale_sprite` to `nw` x `nh`. Tilesets whose layers can be
/// scaled exactly (the canvas grows by whole factors, so each tile is enlarged by replicating
/// pixels, and the tile size stays within limits) are enlarged along with the canvas. Layers
/// using any other tileset are converted to normal layers so their pixels scale like any other
/// layer. Call before the cel pixels are scaled; the grids themselves are kept by the caller.
pub fn prepare_scale(s: &mut Sprite, nw: u32, nh: u32) {
    let (w, h) = (s.width, s.height);
    let exact = w > 0 && h > 0 && nw.is_multiple_of(w) && nh.is_multiple_of(h);
    let (fx, fy) = if exact { (nw / w, nh / h) } else { (0, 0) };
    let mut flatten = vec![false; s.tilesets.len()];
    for (ti, flag) in flatten.iter_mut().enumerate() {
        let ts = &s.tilesets[ti];
        let (tw, th) = (ts.tile_w, ts.tile_h);
        let uses_d = s.layers.iter().enumerate().any(|(l, ly)| {
            ly.kind == KIND_TILEMAP
                && ly.tileset as usize == ti
                && s.cels.iter().any(|row| {
                    row.get(l)
                        .and_then(|c| c.as_ref())
                        .and_then(|c| c.tiles.as_ref())
                        .is_some_and(|g| g.tiles.iter().any(|v| v & FLIP_D != 0))
                })
        });
        // Isometric cells keep their 2:1 look only when the canvas grows by the same factor on both axes.
        let iso_skew = ts.is_iso() && fx != fy;
        *flag = !exact
            || fx == 0
            || fy == 0
            || iso_skew
            || tw * fx > MAX_TILE_SIZE
            || th * fy > MAX_TILE_SIZE
            || (uses_d && ((tw == th) != (tw * fx == th * fy)));
    }
    for l in 0..s.layers.len() {
        if s.layers[l].kind == KIND_TILEMAP && flatten.get(s.layers[l].tileset as usize).copied().unwrap_or(true) {
            flatten_layer(s, l);
        }
    }
    for (ti, &flat) in flatten.iter().enumerate() {
        if flat || !tileset_in_use(s, ti) {
            continue;
        }
        let (fx, fy) = (fx as usize, fy as usize);
        let ts = &mut s.tilesets[ti];
        let (tw, th) = (ts.tile_w as usize, ts.tile_h as usize);
        for tile in &mut ts.tiles {
            let mut big = vec![0u8; tw * fx * th * fy * 4];
            for y in 0..th * fy {
                for x in 0..tw * fx {
                    let src = ((y / fy) * tw + x / fx) * 4;
                    big[(y * tw * fx + x) * 4..][..4].copy_from_slice(&tile[src..src + 4]);
                }
            }
            *tile = Arc::new(big);
        }
        ts.tile_w = (tw * fx) as u32;
        ts.tile_h = (th * fy) as u32;
        let is_iso = ts.iso.is_some();
        if let Some(spec) = ts.iso.as_mut() {
            spec.cell_w *= fx as u32;
            spec.cell_h *= fy as u32;
            for p in spec.pivots.values_mut() {
                *p = (p.0 * fx as i32, p.1 * fy as i32);
            }
        }
        for l in 0..s.layers.len() {
            if tileset_of(s, l) != Some(ti) {
                continue;
            }
            if is_iso {
                // Isometric grids are addressed in cells; only the pixel offset of the layer scales.
                let o = s.layers[l].offset;
                s.layers[l].offset = (o.0.saturating_mul(fx as i32), o.1.saturating_mul(fy as i32));
                continue;
            }
            for row in s.cels.iter_mut() {
                if let Some(Some(arc)) = row.get_mut(l) {
                    if let Some(g) = Arc::make_mut(arc).tiles.as_mut() {
                        g.x *= fx as i32;
                        g.y *= fy as i32;
                    }
                }
            }
        }
    }
    remove_unused_tilesets(s);
}

// MARK: Tileset management

/// Appends a copy of a tile and returns its index.
pub fn duplicate_tile(s: &mut Sprite, ts: usize, tile: u32) -> Option<u32> {
    let t = s.tilesets.get_mut(ts)?;
    let src = t.tiles.get(tile as usize)?.clone();
    if t.tiles.len() >= MAX_TILES {
        return None;
    }
    t.tiles.push(Arc::new((*src).clone()));
    let new = t.tiles.len() as u32 - 1;
    if let Some(spec) = t.iso.as_mut() {
        if let Some(&p) = spec.pivots.get(&tile) {
            spec.pivots.insert(new, p);
        }
    }
    Some(new)
}

/// Deletes a tile. Cells showing it become empty, and later tile indices shift down by one in
/// every tilemap that uses the tileset. The empty tile 0 cannot be deleted.
pub fn delete_tile(s: &mut Sprite, ts: usize, tile: u32) -> bool {
    let id = tile as usize;
    let Some(t) = s.tilesets.get_mut(ts) else { return false };
    if id >= t.tiles.len() || t.tiles.len() <= 1 || (id == 0 && t.zero_is_empty()) {
        return false;
    }
    t.tiles.remove(id);
    if let Some(spec) = t.iso.as_mut() {
        spec.pivots = std::mem::take(&mut spec.pivots)
            .into_iter()
            .filter(|&(k, _)| k as usize != id)
            .map(|(k, p)| (if k as usize > id { k - 1 } else { k }, p))
            .collect();
    }
    for l in 0..s.layers.len() {
        if tileset_of(s, l) != Some(ts) {
            continue;
        }
        for row in s.cels.iter_mut() {
            let Some(Some(arc)) = row.get_mut(l) else { continue };
            let Some(g) = Arc::make_mut(arc).tiles.as_mut() else { continue };
            for v in &mut g.tiles {
                let i = (*v & TILE_ID_MASK) as usize;
                if i == id {
                    *v = 0;
                } else if i > id {
                    *v = (*v & FLIP_MASK) | (i as u32 - 1);
                }
            }
        }
    }
    refresh_tileset_users(s, ts);
    true
}

/// Points a tilemap layer at another tileset with the same tile size. Cells that refer to
/// tiles the new tileset lacks become empty. The old tileset is removed if nothing uses it.
pub fn assign_tileset(s: &mut Sprite, layer: usize, ts: usize) -> bool {
    let Some(old) = tileset_of(s, layer) else { return false };
    if ts == old || ts >= s.tilesets.len() {
        return false;
    }
    let (a, b) = (&s.tilesets[old], &s.tilesets[ts]);
    let cell = |t: &Tileset| t.iso.as_ref().map(|i| (i.cell_w, i.cell_h));
    if (a.tile_w, a.tile_h, cell(a)) != (b.tile_w, b.tile_h, cell(b)) {
        return false;
    }
    let count = b.tiles.len();
    s.layers[layer].tileset = ts as u32;
    for row in s.cels.iter_mut() {
        if let Some(Some(arc)) = row.get_mut(layer) {
            if let Some(g) = Arc::make_mut(arc).tiles.as_mut() {
                for v in &mut g.tiles {
                    if (*v & TILE_ID_MASK) as usize >= count {
                        *v = 0;
                    }
                }
            }
        }
    }
    for f in 0..s.frame_count() {
        refresh_cel(s, f, layer);
    }
    if !tileset_in_use(s, old) {
        remove_tileset(s, old);
    }
    true
}

// MARK: .aseprite chunks

struct Cur<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.p.checked_add(n).filter(|&e| e <= self.b.len()).ok_or("unexpected end of chunk")?;
        let s = &self.b[self.p..end];
        self.p = end;
        Ok(s)
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn rest(&mut self) -> &'a [u8] {
        let s = &self.b[self.p..];
        self.p = self.b.len();
        s
    }
}

fn inflate(data: &[u8], len: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(len.min(1 << 24));
    ZlibDecoder::new(data).take(len as u64).read_to_end(&mut out).map_err(|e| format!("corrupt tile data: {e}"))?;
    if out.len() < len {
        return Err("truncated tile data".into());
    }
    Ok(out)
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(data).expect("in-memory write");
    z.finish().expect("in-memory write")
}

/// A Tileset chunk as stored in the file (pixels still in the file's color depth).
pub struct RawTileset {
    id: u32,
    flags: u32,
    count: usize,
    tile_w: u32,
    tile_h: u32,
    base_index: i16,
    name: String,
    data: Option<Vec<u8>>,
}

impl RawTileset {
    /// Memory the tileset needs once converted to RGBA.
    pub fn pixel_bytes(&self) -> usize {
        self.count.max(1) * self.tile_w as usize * self.tile_h as usize * 4
    }
}

/// Parses the body of a Tileset chunk. `bpp` is the file's bytes per pixel.
pub fn parse_tileset_chunk(body: &[u8], bpp: usize) -> Result<RawTileset> {
    let mut c = Cur { b: body, p: 0 };
    let id = c.u32()?;
    let flags = c.u32()?;
    let count = c.u32()? as usize;
    let tile_w = c.u16()? as u32;
    let tile_h = c.u16()? as u32;
    let base_index = c.u16()? as i16;
    c.take(14)?;
    let nlen = c.u16()? as usize;
    let name = String::from_utf8_lossy(c.take(nlen)?).into_owned();
    if tile_w == 0 || tile_h == 0 || count > MAX_TILES || tile_w > MAX_TILE_SIZE || tile_h > MAX_TILE_SIZE {
        return Err("unsupported tileset size".into());
    }
    if count * tile_w as usize * tile_h as usize * 4 > 1 << 29 {
        return Err("tileset too large".into());
    }
    if flags & TS_EXTERNAL != 0 {
        c.take(8)?;
    }
    let data = if flags & TS_EMBEDDED != 0 {
        let clen = c.u32()? as usize;
        let len = count * tile_w as usize * tile_h as usize * bpp;
        if len > 1 << 29 {
            return Err("tileset too large".into());
        }
        Some(inflate(c.take(clen)?, len)?)
    } else {
        None
    };
    Ok(RawTileset { id, flags, count, tile_w, tile_h, base_index, name, data })
}

/// Builds tilesets from parsed chunks. `to_rgba` converts file-depth pixels to RGBA.
/// Returns the tilesets and a map from file tileset ID to index.
pub fn finish_tilesets(raw: Vec<RawTileset>, to_rgba: impl Fn(&[u8]) -> Vec<u8>) -> (Vec<Tileset>, HashMap<u32, u32>) {
    let mut sets = Vec::new();
    let mut ids = HashMap::new();
    for r in raw {
        let mut ts = Tileset {
            name: r.name,
            tile_w: r.tile_w,
            tile_h: r.tile_h,
            iso: None,
            base_index: r.base_index,
            flags: r.flags & !(TS_EXTERNAL | TS_EMBEDDED),
            tiles: Vec::new(),
        };
        let len = ts.tile_len();
        if let Some(data) = r.data {
            let rgba = to_rgba(&data);
            ts.tiles = rgba.chunks_exact(len).take(r.count).map(|t| Arc::new(t.to_vec())).collect();
        }
        while ts.tiles.len() < r.count.max(1) {
            ts.add_blank();
        }
        ids.insert(r.id, sets.len() as u32);
        sets.push(ts);
    }
    (sets, ids)
}

/// Maps tilemap layers' file tileset IDs to indices; layers with a missing tileset become normal.
pub fn resolve_layers(layers: &mut [crate::document::Layer], ids: &HashMap<u32, u32>) {
    for l in layers.iter_mut().filter(|l| l.kind == KIND_TILEMAP) {
        match ids.get(&l.tileset) {
            Some(&i) => l.tileset = i,
            None => {
                l.kind = 0;
                l.tileset = 0;
            }
        }
    }
}

/// Parses a compressed tilemap cel (type 3) body that follows the common cel header.
pub fn parse_tilemap_cel(body: &[u8], x: i32, y: i32) -> Result<TileGrid> {
    let mut c = Cur { b: body, p: 0 };
    let w = c.u16()? as usize;
    let h = c.u16()? as usize;
    let bits = c.u16()? as usize;
    let id_mask = match c.u32()? {
        0 => TILE_ID_MASK,
        m => m,
    };
    let (xm, ym, dm) = (c.u32()?, c.u32()?, c.u32()?);
    c.take(10)?;
    if !matches!(bits, 8 | 16 | 32) || w * h > MAX_CELLS {
        return Err("unsupported tilemap cel".into());
    }
    let data = inflate(c.rest(), w * h * bits / 8)?;
    let tiles = data
        .chunks_exact(bits / 8)
        .take(w * h)
        .map(|b| {
            let raw = b.iter().rev().fold(0u32, |a, &v| a << 8 | v as u32);
            let mut v = raw & id_mask & TILE_ID_MASK;
            if raw & xm != 0 {
                v |= FLIP_X;
            }
            if raw & ym != 0 {
                v |= FLIP_Y;
            }
            if raw & dm != 0 {
                v |= FLIP_D;
            }
            v
        })
        .collect();
    Ok(TileGrid { x, y, w: w as u32, h: h as u32, tiles })
}

/// Builds a cel from a parsed grid; `None` if the layer's tileset is unknown.
pub fn cel_from_grid(
    s_tilesets: &[Tileset],
    layer: &crate::document::Layer,
    opacity: u8,
    grid: TileGrid,
    w: usize,
    h: usize,
) -> Option<Cel> {
    let ts = s_tilesets.get(layer.tileset as usize).filter(|_| layer.kind == KIND_TILEMAP)?;
    Some(make_cel(opacity, grid, ts, layer.offset, w, h))
}

/// Tileset chunk bodies for a sprite, in tileset index order (the index is the file ID).
/// Tile pixels are encoded in the sprite's color mode by `enc`.
pub fn tileset_bodies(s: &Sprite, enc: &mut Encoder) -> Vec<Vec<u8>> {
    s.tilesets
        .iter()
        .enumerate()
        .map(|(i, ts)| {
            let mut b = Vec::new();
            b.extend((i as u32).to_le_bytes());
            b.extend(((ts.flags & !TS_EXTERNAL) | TS_EMBEDDED).to_le_bytes());
            b.extend((ts.tiles.len() as u32).to_le_bytes());
            b.extend((ts.tile_w as u16).to_le_bytes());
            b.extend((ts.tile_h as u16).to_le_bytes());
            b.extend(ts.base_index.to_le_bytes());
            b.extend([0u8; 14]);
            let name = &ts.name.as_bytes()[..ts.name.len().min(u16::MAX as usize)];
            b.extend((name.len() as u16).to_le_bytes());
            b.extend(name);
            let pixels: Vec<u8> = ts.tiles.iter().flat_map(|t| t.iter().copied()).collect();
            let z = deflate(&enc.encode_row(false, &pixels));
            b.extend((z.len() as u32).to_le_bytes());
            b.extend(z);
            b
        })
        .collect()
}

/// Body of a tilemap cel chunk, or `None` if the cel shows no tiles.
pub fn cel_body(layer: usize, cel: &Cel) -> Option<Vec<u8>> {
    let g = cel.tiles.as_ref()?;
    if g.w == 0 || g.h == 0 || g.w > u16::MAX as u32 || g.h > u16::MAX as u32 || g.tiles.iter().all(|&v| v == 0) {
        return None;
    }
    let mut b = Vec::new();
    b.extend((layer as u16).to_le_bytes());
    b.extend((g.x as i16).to_le_bytes());
    b.extend((g.y as i16).to_le_bytes());
    b.push(cel.opacity);
    b.extend(3u16.to_le_bytes()); // compressed tilemap
    b.extend(0u16.to_le_bytes()); // z-index
    b.extend([0u8; 5]);
    b.extend((g.w as u16).to_le_bytes());
    b.extend((g.h as u16).to_le_bytes());
    b.extend(32u16.to_le_bytes());
    for m in [TILE_ID_MASK, FLIP_X, FLIP_Y, FLIP_D] {
        b.extend(m.to_le_bytes());
    }
    b.extend([0u8; 10]);
    let raw: Vec<u8> = g.tiles.iter().flat_map(|v| v.to_le_bytes()).collect();
    b.extend(deflate(&raw));
    Some(b)
}

// MARK: C ABI

unsafe fn ed<'a>(d: *mut Editor) -> Option<&'a mut Editor> {
    let e = d.as_mut()?;
    e.dirty.mark_all();
    Some(e)
}

unsafe fn sp<'a>(d: *const Editor) -> Option<&'a Sprite> {
    d.as_ref().map(|e| &e.sprite)
}

unsafe fn tileset<'a>(d: *const Editor, ts: u32) -> Option<&'a Tileset> {
    sp(d)?.tilesets.get(ts as usize)
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_is_tilemap(doc: *const Editor, layer: u32) -> bool {
    crate::ffi::guarded("pixmith_layer_is_tilemap", std::ptr::null_mut(), || {
        sp(doc).is_some_and(|s| tileset_of(s, layer as usize).is_some())
    })
}

/// Index of the tileset a tilemap layer uses, or -1.
#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_tileset(doc: *const Editor, layer: u32) -> i32 {
    crate::ffi::guarded("pixmith_layer_tileset", std::ptr::null_mut(), || {
        sp(doc).and_then(|s| tileset_of(s, layer as usize)).map_or(-1, |i| i as i32)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tileset_count(doc: *const Editor) -> u32 {
    crate::ffi::guarded("pixmith_tileset_count", std::ptr::null_mut(), || {
        sp(doc).map_or(0, |s| s.tilesets.len() as u32)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tileset_tile_count(doc: *const Editor, ts: u32) -> u32 {
    crate::ffi::guarded("pixmith_tileset_tile_count", std::ptr::null_mut(), || {
        tileset(doc, ts).map_or(0, |t| t.tiles.len() as u32)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tileset_tile_width(doc: *const Editor, ts: u32) -> u32 {
    crate::ffi::guarded("pixmith_tileset_tile_width", std::ptr::null_mut(), || tileset(doc, ts).map_or(0, |t| t.tile_w))
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tileset_tile_height(doc: *const Editor, ts: u32) -> u32 {
    crate::ffi::guarded("pixmith_tileset_tile_height", std::ptr::null_mut(), || {
        tileset(doc, ts).map_or(0, |t| t.tile_h)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tileset_base_index(doc: *const Editor, ts: u32) -> i32 {
    crate::ffi::guarded("pixmith_tileset_base_index", std::ptr::null_mut(), || {
        tileset(doc, ts).map_or(1, |t| t.base_index as i32)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tileset_set_base_index(doc: *mut Editor, ts: u32, base: i32) {
    crate::ffi::guarded("pixmith_tileset_set_base_index", doc, || {
        let Some(e) = ed(doc) else { return };
        let base = base.clamp(0, i16::MAX as i32) as i16;
        if e.sprite.tilesets.get(ts as usize).is_some_and(|t| t.base_index != base) {
            e.checkpoint();
            e.sprite.tilesets[ts as usize].base_index = base;
        }
    })
}

/// Returns a string to release with `pixmith_string_free`, or null.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tileset_name(doc: *const Editor, ts: u32) -> *mut c_char {
    crate::ffi::guarded("pixmith_tileset_name", std::ptr::null_mut(), || {
        tileset(doc, ts).and_then(|t| CString::new(t.name.replace('\0', "")).ok()).map_or(null_mut(), CString::into_raw)
    })
}

/// Copies one tile as straight-alpha RGBA (tile width * height * 4 bytes) into `out`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tileset_tile_pixels(
    doc: *const Editor,
    ts: u32,
    tile: u32,
    out: *mut u8,
    len: usize,
) -> bool {
    crate::ffi::guarded("pixmith_tileset_tile_pixels", std::ptr::null_mut(), || {
        let Some(t) = tileset(doc, ts).and_then(|t| t.tiles.get(tile as usize)) else { return false };
        if out.is_null() || len < t.len() {
            return false;
        }
        std::ptr::copy_nonoverlapping(t.as_ptr(), out, t.len());
        true
    })
}

/// Appends an empty tile to a tileset (undoable). Returns its index, or 0 on failure.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tileset_add_tile(doc: *mut Editor, ts: u32) -> u32 {
    crate::ffi::guarded("pixmith_tileset_add_tile", doc, || {
        let Some(e) = ed(doc) else { return 0 };
        if (ts as usize) >= e.sprite.tilesets.len() || e.sprite.tilesets[ts as usize].tiles.len() >= MAX_TILES {
            return 0;
        }
        e.checkpoint();
        e.sprite.tilesets[ts as usize].add_blank()
    })
}

/// Inserts a tilemap layer with a new tileset (undoable). Returns the layer index.
#[no_mangle]
pub unsafe extern "C" fn pixmith_add_tilemap_layer(doc: *mut Editor, above: u32, tile_w: u32, tile_h: u32) -> u32 {
    crate::ffi::guarded("pixmith_add_tilemap_layer", doc, || {
        let Some(e) = ed(doc) else { return 0 };
        e.checkpoint();
        add_tilemap_layer(&mut e.sprite, above as usize, tile_w, tile_h) as u32
    })
}

/// Places `tile` (index | flip flags) under canvas pixel (x, y). Call `pixmith_begin_stroke` first.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tile_stamp(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    tile: u32,
) -> bool {
    crate::ffi::guarded("pixmith_tile_stamp", doc, || {
        ed(doc).is_some_and(|e| {
            let ok = stamp(&mut e.sprite, layer as usize, frame as usize, x, y, tile);
            if ok {
                crate::gamedev::autotile::note_stamp(&mut e.sprite, layer as usize, frame as usize, x, y, tile);
            }
            ok
        })
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tile_at(doc: *const Editor, layer: u32, frame: u32, x: i32, y: i32) -> u32 {
    crate::ffi::guarded("pixmith_tile_at", std::ptr::null_mut(), || {
        sp(doc).map_or(0, |s| tile_at(s, layer as usize, frame as usize, x, y))
    })
}

/// Runs an edit; if it succeeds the previous state becomes one undo step.
fn undoable(e: &mut Editor, f: impl FnOnce(&mut Sprite) -> bool) -> bool {
    let before = e.sprite.clone();
    if f(&mut e.sprite) {
        let after = std::mem::replace(&mut e.sprite, before);
        e.checkpoint();
        e.sprite = after;
        true
    } else {
        e.sprite = before;
        false
    }
}

/// Converts a normal layer to a tilemap layer (undoable). Only exact duplicate tiles are shared.
#[no_mangle]
pub unsafe extern "C" fn pixmith_convert_to_tilemap(doc: *mut Editor, layer: u32, tile_w: u32, tile_h: u32) -> bool {
    crate::ffi::guarded("pixmith_convert_to_tilemap", doc, || {
        ed(doc).is_some_and(|e| undoable(e, |s| convert_to_tilemap(s, layer as usize, tile_w, tile_h)))
    })
}

/// Like `pixmith_convert_to_tilemap`; with `flip_aware`, flipped and transposed copies of a tile
/// share it too (using flip flags).
#[no_mangle]
pub unsafe extern "C" fn pixmith_convert_to_tilemap_ex(
    doc: *mut Editor,
    layer: u32,
    tile_w: u32,
    tile_h: u32,
    flip_aware: bool,
) -> bool {
    crate::ffi::guarded("pixmith_convert_to_tilemap_ex", doc, || {
        ed(doc).is_some_and(|e| undoable(e, |s| convert_to_tilemap_with(s, layer as usize, tile_w, tile_h, flip_aware)))
    })
}

/// Converts a tilemap layer back to a normal layer (undoable).
#[no_mangle]
pub unsafe extern "C" fn pixmith_convert_to_normal(doc: *mut Editor, layer: u32) -> bool {
    crate::ffi::guarded("pixmith_convert_to_normal", doc, || {
        ed(doc).is_some_and(|e| undoable(e, |s| convert_to_normal(s, layer as usize)))
    })
}

/// Appends a copy of a tile (undoable). Returns the new index, or 0 on failure.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tileset_duplicate_tile(doc: *mut Editor, ts: u32, tile: u32) -> u32 {
    crate::ffi::guarded("pixmith_tileset_duplicate_tile", doc, || {
        let Some(e) = ed(doc) else { return 0 };
        let mut new = 0;
        undoable(e, |s| duplicate_tile(s, ts as usize, tile).inspect(|&n| new = n).is_some());
        new
    })
}

/// Deletes a tile and remaps tile indices in all tilemaps using the tileset (undoable).
#[no_mangle]
pub unsafe extern "C" fn pixmith_tileset_delete_tile(doc: *mut Editor, ts: u32, tile: u32) -> bool {
    crate::ffi::guarded("pixmith_tileset_delete_tile", doc, || {
        ed(doc).is_some_and(|e| undoable(e, |s| delete_tile(s, ts as usize, tile)))
    })
}

/// Points a tilemap layer at another tileset of the same tile size (undoable).
#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_assign_tileset(doc: *mut Editor, layer: u32, ts: u32) -> bool {
    crate::ffi::guarded("pixmith_layer_assign_tileset", doc, || {
        ed(doc).is_some_and(|e| undoable(e, |s| assign_tileset(s, layer as usize, ts as usize)))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tileset_rename(doc: *mut Editor, ts: u32, name: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_tileset_rename", doc, || {
        let Some(e) = ed(doc) else { return false };
        if name.is_null() {
            return false;
        }
        let name = std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned();
        undoable(e, |s| match s.tilesets.get_mut(ts as usize) {
            Some(t) => {
                t.name = name;
                true
            }
            None => false,
        })
    })
}

/// Offset of the tile grid inside the canvas for a tilemap cel: writes (x, y) in `[0, tile size)`
/// into `out[2]`. Returns false if the layer is not a tilemap.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tile_grid_origin(doc: *const Editor, layer: u32, frame: u32, out: *mut i32) -> bool {
    crate::ffi::guarded("pixmith_tile_grid_origin", std::ptr::null_mut(), || {
        let Some(s) = sp(doc) else { return false };
        let Some(ti) = tileset_of(s, layer as usize) else { return false };
        let ts = &s.tilesets[ti];
        if ts.is_iso() {
            return false;
        }
        let (x, y) = s
            .cels
            .get(frame as usize)
            .and_then(|r| r.get(layer as usize))
            .and_then(|c| c.as_ref())
            .and_then(|c| c.tiles.as_ref())
            .map_or((0, 0), |g| (g.x.rem_euclid(ts.tile_w as i32), g.y.rem_euclid(ts.tile_h as i32)));
        if !out.is_null() {
            std::slice::from_raw_parts_mut(out, 2).copy_from_slice(&[x, y]);
        }
        true
    })
}

// MARK: C ABI: isometric tilemaps

unsafe fn put<T: Copy, const N: usize>(out: *mut T, v: [T; N]) {
    if !out.is_null() {
        std::slice::from_raw_parts_mut(out, N).copy_from_slice(&v);
    }
}

/// Inserts an isometric tilemap layer with a new isometric tileset (undoable): `tile_w` x `tile_h` tile images on
/// `cell_w` x `cell_h` diamonds (even sizes; odd ones are rounded up), cell (0, 0)'s box at canvas pixel
/// (`offset_x`, `offset_y`). Returns the layer index.
#[no_mangle]
pub unsafe extern "C" fn pixmith_add_iso_tilemap_layer(
    doc: *mut Editor,
    above: u32,
    tile_w: u32,
    tile_h: u32,
    cell_w: u32,
    cell_h: u32,
    offset_x: i32,
    offset_y: i32,
) -> u32 {
    crate::ffi::guarded("pixmith_add_iso_tilemap_layer", doc, || {
        let Some(e) = ed(doc) else { return 0 };
        e.checkpoint();
        add_iso_tilemap_layer(&mut e.sprite, above as usize, tile_w, tile_h, cell_w, cell_h, (offset_x, offset_y))
            as u32
    })
}

/// Whether tileset `ts` is isometric.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tileset_is_iso(doc: *const Editor, ts: u32) -> bool {
    crate::ffi::guarded("pixmith_tileset_is_iso", std::ptr::null_mut(), || {
        tileset(doc, ts).is_some_and(Tileset::is_iso)
    })
}

/// Writes the diamond cell size (`cell_w`, `cell_h`) of an isometric tileset into `out[2]`. False for square tilesets.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tileset_iso_cell(doc: *const Editor, ts: u32, out: *mut u32) -> bool {
    crate::ffi::guarded("pixmith_tileset_iso_cell", std::ptr::null_mut(), || {
        let Some(spec) = tileset(doc, ts).and_then(|t| t.iso.as_ref()) else { return false };
        put(out, [spec.cell_w, spec.cell_h]);
        true
    })
}

/// Makes a tileset isometric with the given cell size, or square again with `cell_w` = 0 (undoable). Only works
/// while no layer shows a tile of the tileset.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tileset_set_iso(doc: *mut Editor, ts: u32, cell_w: u32, cell_h: u32) -> bool {
    crate::ffi::guarded("pixmith_tileset_set_iso", doc, || {
        let Some(e) = ed(doc) else { return false };
        let cell = (cell_w != 0).then_some((cell_w, cell_h.max(1)));
        undoable(e, |s| set_tileset_iso(s, ts as usize, cell))
    })
}

/// Writes the pivot of a tile of an isometric tileset (the point of the image, from its top-left corner, that sits
/// on the diamond's center; the default when none was set) into `out[2]`. False for square tilesets.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tile_pivot(doc: *const Editor, ts: u32, tile: u32, out: *mut i32) -> bool {
    crate::ffi::guarded("pixmith_tile_pivot", std::ptr::null_mut(), || {
        let Some(t) = tileset(doc, ts).filter(|t| t.is_iso() && (tile as usize) < t.tiles.len()) else { return false };
        let (x, y) = t.pivot(tile);
        put(out, [x, y]);
        true
    })
}

/// Whether the pivot of a tile was set explicitly (false: it has the default).
#[no_mangle]
pub unsafe extern "C" fn pixmith_tile_pivot_is_set(doc: *const Editor, ts: u32, tile: u32) -> bool {
    crate::ffi::guarded("pixmith_tile_pivot_is_set", std::ptr::null_mut(), || {
        tileset(doc, ts).and_then(|t| t.iso.as_ref()).is_some_and(|i| i.pivots.contains_key(&tile))
    })
}

/// Sets the pivot of a tile of an isometric tileset (undoable).
#[no_mangle]
pub unsafe extern "C" fn pixmith_tile_set_pivot(doc: *mut Editor, ts: u32, tile: u32, x: i32, y: i32) -> bool {
    crate::ffi::guarded("pixmith_tile_set_pivot", doc, || {
        ed(doc).is_some_and(|e| undoable(e, |s| set_tile_pivot(s, ts as usize, tile, Some((x, y)))))
    })
}

/// Resets the pivot of a tile to the default (undoable).
#[no_mangle]
pub unsafe extern "C" fn pixmith_tile_clear_pivot(doc: *mut Editor, ts: u32, tile: u32) -> bool {
    crate::ffi::guarded("pixmith_tile_clear_pivot", doc, || {
        ed(doc).is_some_and(|e| undoable(e, |s| set_tile_pivot(s, ts as usize, tile, None)))
    })
}

/// Writes the pivot as Godot's `texture_origin` (offset from the image center) into `out[2]`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tile_texture_origin(doc: *const Editor, ts: u32, tile: u32, out: *mut i32) -> bool {
    crate::ffi::guarded("pixmith_tile_texture_origin", std::ptr::null_mut(), || {
        let Some(t) = tileset(doc, ts).filter(|t| t.is_iso() && (tile as usize) < t.tiles.len()) else { return false };
        let (x, y) = t.texture_origin(tile);
        put(out, [x, y]);
        true
    })
}

/// Whether the layer is an isometric tilemap layer.
#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_is_iso(doc: *const Editor, layer: u32) -> bool {
    crate::ffi::guarded("pixmith_layer_is_iso", std::ptr::null_mut(), || {
        sp(doc).is_some_and(|s| tileset_of(s, layer as usize).is_some_and(|ti| s.tilesets[ti].is_iso()))
    })
}

/// Writes the canvas pixel of cell (0, 0)'s box (the layer's grid offset) into `out[2]`. False for other layers.
#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_grid_offset(doc: *const Editor, layer: u32, out: *mut i32) -> bool {
    crate::ffi::guarded("pixmith_layer_grid_offset", std::ptr::null_mut(), || {
        let Some(s) = sp(doc) else { return false };
        if !tileset_of(s, layer as usize).is_some_and(|ti| s.tilesets[ti].is_iso()) {
            return false;
        }
        let o = s.layers[layer as usize].offset;
        put(out, [o.0, o.1]);
        true
    })
}

/// Moves an isometric layer's grid (undoable): canvas pixel of cell (0, 0)'s box. Layers at other elevations use
/// offsets that differ by the elevation, e.g. half a cell is `cell_h / 2` pixels.
#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_set_grid_offset(doc: *mut Editor, layer: u32, x: i32, y: i32) -> bool {
    crate::ffi::guarded("pixmith_layer_set_grid_offset", doc, || {
        ed(doc).is_some_and(|e| undoable(e, |s| set_layer_offset(s, layer as usize, (x, y))))
    })
}

/// Writes the cell `(i, j)` containing canvas pixel (x, y) into `out[2]`: the diamond of the layer's isometric grid,
/// or for square tilemaps `(x / tile_w, y / tile_h)` (relative to the canvas). False for other layers.
#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_cell_at(doc: *const Editor, layer: u32, x: i32, y: i32, out: *mut i32) -> bool {
    crate::ffi::guarded("pixmith_layer_cell_at", std::ptr::null_mut(), || {
        let Some((i, j)) = sp(doc).and_then(|s| cell_at(s, layer as usize, x, y)) else { return false };
        put(out, [i.clamp(i32::MIN as i64, i32::MAX as i64) as i32, j.clamp(i32::MIN as i64, i32::MAX as i64) as i32]);
        true
    })
}

/// Writes the canvas pixel of the top-left corner of cell `(i, j)`'s box into `out[2]`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_cell_origin(
    doc: *const Editor,
    layer: u32,
    i: i32,
    j: i32,
    out: *mut i32,
) -> bool {
    crate::ffi::guarded("pixmith_layer_cell_origin", std::ptr::null_mut(), || {
        let Some((x, y)) = sp(doc).and_then(|s| cell_origin(s, layer as usize, i as i64, j as i64)) else {
            return false;
        };
        put(out, [x.clamp(i32::MIN as i64, i32::MAX as i64) as i32, y.clamp(i32::MIN as i64, i32::MAX as i64) as i32]);
        true
    })
}

/// The tile value in cell `(i, j)` of an isometric layer (0 if empty).
#[no_mangle]
pub unsafe extern "C" fn pixmith_tile_in_cell(doc: *const Editor, layer: u32, frame: u32, i: i32, j: i32) -> u32 {
    crate::ffi::guarded("pixmith_tile_in_cell", std::ptr::null_mut(), || {
        sp(doc).map_or(0, |s| tile_in_cell(s, layer as usize, frame as usize, i as i64, j as i64))
    })
}

/// Places `tile` (index | flip flags) in cell `(i, j)` of an isometric layer; 0 clears it. The grid grows to hold
/// the cell. Call `pixmith_begin_stroke` first, as for `pixmith_tile_stamp`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tile_stamp_cell(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    i: i32,
    j: i32,
    tile: u32,
) -> bool {
    crate::ffi::guarded("pixmith_tile_stamp_cell", doc, || {
        ed(doc).is_some_and(|e| {
            let ok = stamp_cell(&mut e.sprite, layer as usize, frame as usize, i as i64, j as i64, tile);
            if ok {
                crate::gamedev::autotile::note_stamp_cell(
                    &mut e.sprite,
                    layer as usize,
                    frame as usize,
                    i as i64,
                    j as i64,
                    tile,
                );
            }
            ok
        })
    })
}

/// The tile you see at canvas pixel (x, y) of an isometric layer (see [`tile_hit_at`]): writes `[i, j, tile value]`
/// into `out[3]`. False when the cell is empty.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tile_hit_at(
    doc: *const Editor,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    out: *mut i32,
) -> bool {
    crate::ffi::guarded("pixmith_tile_hit_at", std::ptr::null_mut(), || {
        let Some(((i, j), v)) = sp(doc).and_then(|s| tile_hit_at(s, layer as usize, frame as usize, x, y)) else {
            return false;
        };
        put(out, [i as i32, j as i32, v as i32]);
        true
    })
}

/// Grid math without a document, for overlays: the cell `(i, j)` of the isometric grid (`w` x `h` cells, cell (0, 0)'s
/// box at (`gx`, `gy`)) that contains pixel (x, y), into `out[2]`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_iso_cell_at(w: i32, h: i32, gx: i32, gy: i32, x: i32, y: i32, out: *mut i32) -> bool {
    crate::ffi::guarded("pixmith_iso_cell_at", std::ptr::null_mut(), || {
        let (i, j) = Iso::new(w, h, gx, gy).cell_at(x, y);
        put(out, [i.clamp(i32::MIN as i64, i32::MAX as i64) as i32, j.clamp(i32::MIN as i64, i32::MAX as i64) as i32]);
        true
    })
}

/// Grid math without a document: the pixel of the top-left corner of cell `(i, j)`'s box, into `out[2]`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_iso_cell_origin(
    w: i32,
    h: i32,
    gx: i32,
    gy: i32,
    i: i32,
    j: i32,
    out: *mut i32,
) -> bool {
    crate::ffi::guarded("pixmith_iso_cell_origin", std::ptr::null_mut(), || {
        let (x, y) = Iso::new(w, h, gx, gy).cell_origin((i as i64, j as i64));
        put(out, [x.clamp(i32::MIN as i64, i32::MAX as i64) as i32, y.clamp(i32::MIN as i64, i32::MAX as i64) as i32]);
        true
    })
}

#[cfg(test)]
mod iso_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::ase;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];

    fn px(s: &Sprite, x: usize, y: usize) -> [u8; 4] {
        let r = s.render(0);
        r[(y * s.width as usize + x) * 4..][..4].try_into().unwrap()
    }

    /// 16x16 sprite with an 8x8 tilemap layer on top of the default layer.
    fn tilemap_sprite() -> (Sprite, usize) {
        let mut s = Sprite::new(16, 16);
        let l = add_tilemap_layer(&mut s, 0, 8, 8);
        (s, l)
    }

    /// Adds an asymmetric tile: red at (0,0), blue at (7,0), green at (0,7).
    fn add_corner_tile(s: &mut Sprite, ts: usize) -> u32 {
        let id = s.tilesets[ts].add_blank();
        let t = Arc::make_mut(&mut s.tilesets[ts].tiles[id as usize]);
        t[0..4].copy_from_slice(&RED);
        t[7 * 4..][..4].copy_from_slice(&BLUE);
        t[7 * 8 * 4..][..4].copy_from_slice(&GREEN);
        id
    }

    #[test]
    fn new_tilemap_layer_is_wired_up() {
        let (s, l) = tilemap_sprite();
        assert_eq!(s.layers[l].kind, KIND_TILEMAP);
        assert_eq!(s.tilesets.len(), 1);
        assert_eq!(s.tilesets[0].tiles.len(), 1);
        assert_eq!(tileset_of(&s, l), Some(0));
    }

    #[test]
    fn stamp_places_and_flips_tiles() {
        let (mut s, l) = tilemap_sprite();
        let id = add_corner_tile(&mut s, 0);
        assert!(stamp(&mut s, l, 0, 9, 1, id));
        // Cell (1,0) covers x 8..16, y 0..8.
        assert_eq!(px(&s, 8, 0), RED);
        assert_eq!(px(&s, 15, 0), BLUE);
        assert_eq!(px(&s, 8, 7), GREEN);
        assert!(!stamp(&mut s, l, 0, 9, 1, id), "same value is a no-op");

        assert!(stamp(&mut s, l, 0, 0, 0, id | FLIP_X));
        assert_eq!(px(&s, 7, 0), RED);
        assert_eq!(px(&s, 0, 0), BLUE);
        assert!(stamp(&mut s, l, 0, 0, 0, id | FLIP_Y));
        assert_eq!(px(&s, 0, 7), RED);
        assert_eq!(px(&s, 7, 7), BLUE);
        assert_eq!(px(&s, 0, 0), GREEN);
        assert!(stamp(&mut s, l, 0, 0, 0, id | FLIP_D));
        // Transpose swaps (7,0) and (0,7).
        assert_eq!(px(&s, 0, 0), RED);
        assert_eq!(px(&s, 0, 7), BLUE);
        assert_eq!(px(&s, 7, 0), GREEN);

        assert!(stamp(&mut s, l, 0, 0, 0, 0));
        assert_eq!(px(&s, 0, 0), [0; 4]);
        assert!(!stamp(&mut s, l, 0, 99, 0, id), "outside canvas");
        assert!(!stamp(&mut s, l, 0, 0, 0, 77), "unknown tile");
    }

    #[test]
    fn drawing_edits_the_shared_tile() {
        let (mut s, l) = tilemap_sprite();
        let id = add_corner_tile(&mut s, 0);
        assert!(stamp(&mut s, l, 0, 0, 0, id));
        assert!(stamp(&mut s, l, 0, 8, 8, id));
        s.draw_line(l, 0, 3, 3, 3, 3, BLUE, 1);
        // Both instances show the new pixel and the tile image changed.
        assert_eq!(px(&s, 3, 3), BLUE);
        assert_eq!(px(&s, 11, 11), BLUE);
        assert_eq!(s.tilesets[0].tiles.len(), 2);
        assert_eq!(&s.tilesets[0].tiles[id as usize][(3 * 8 + 3) * 4..][..4], &BLUE);
        // Erasing works too.
        s.draw_line(l, 0, 0, 0, 0, 0, [0; 4], 1);
        assert_eq!(px(&s, 8, 8), [0; 4]);
    }

    #[test]
    fn drawing_through_a_flipped_instance_edits_the_unflipped_tile() {
        let (mut s, l) = tilemap_sprite();
        let id = add_corner_tile(&mut s, 0);
        assert!(stamp(&mut s, l, 0, 0, 0, id | FLIP_X));
        s.draw_line(l, 0, 1, 1, 1, 1, BLUE, 1);
        // Flipped on X, so tile pixel (6,1) is the edited one.
        assert_eq!(&s.tilesets[0].tiles[id as usize][(8 + 6) * 4..][..4], &BLUE);
        assert_eq!(px(&s, 1, 1), BLUE);
        assert_eq!(px(&s, 0, 0), BLUE); // flipped blue corner
    }

    #[test]
    fn drawing_on_an_empty_tilemap_creates_tiles() {
        let (mut s, l) = tilemap_sprite();
        s.draw_line(l, 0, 2, 2, 12, 2, RED, 1);
        // The stroke crosses cells (0,0) and (1,0), so two new tiles exist.
        assert_eq!(s.tilesets[0].tiles.len(), 3);
        assert_eq!(px(&s, 2, 2), RED);
        assert_eq!(px(&s, 12, 2), RED);
        let g = s.cels[0][l].as_ref().unwrap().tiles.as_ref().unwrap();
        assert!(g.tiles.iter().filter(|&&v| v != 0).count() == 2);
        // Filling an empty cell also works.
        s.fill(l, 0, 0, 12, GREEN);
        assert_eq!(px(&s, 15, 15), GREEN);
    }

    #[test]
    fn tile_edits_refresh_other_frames() {
        let (mut s, l) = tilemap_sprite();
        let id = add_corner_tile(&mut s, 0);
        stamp(&mut s, l, 0, 0, 0, id);
        let f = s.add_frame(0, true);
        assert_eq!(f, 1);
        s.draw_line(l, 0, 4, 4, 4, 4, BLUE, 1);
        let r1 = s.render(1);
        assert_eq!(&r1[(4 * 16 + 4) * 4..][..4], &BLUE);
    }

    #[test]
    fn grid_grows_for_negative_cells() {
        let mut g = TileGrid::covering(16, 16, 8, 8);
        g.tiles[0] = 5;
        g.x = 8;
        g.y = 8;
        assert!(g.grow_to_include(0, 0, 8, 8));
        assert_eq!((g.x, g.y, g.w, g.h), (0, 0, 3, 3));
        assert_eq!(g.tiles[1 * 3 + 1], 5);
    }

    #[test]
    fn round_trip_with_flip_flags() {
        let (mut s, l) = tilemap_sprite();
        let id = add_corner_tile(&mut s, 0);
        s.tilesets[0].name = "My tiles".into();
        s.tilesets[0].base_index = 0;
        stamp(&mut s, l, 0, 0, 0, id);
        stamp(&mut s, l, 0, 8, 0, id | FLIP_X);
        stamp(&mut s, l, 0, 0, 8, id | FLIP_Y);
        stamp(&mut s, l, 0, 8, 8, id | FLIP_X | FLIP_Y | FLIP_D);
        s.draw_line(0, 0, 1, 1, 5, 5, RED, 1); // a normal layer too
        s.layers[l].opacity = 99;
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back, s);
        let g = back.cels[0][l].as_ref().unwrap().tiles.as_ref().unwrap();
        assert_eq!(g.tiles, vec![id, id | FLIP_X, id | FLIP_Y, id | FLIP_X | FLIP_Y | FLIP_D]);
        assert_eq!(back.render(0), s.render(0));
    }

    #[test]
    fn unsynced_pixel_edits_are_saved() {
        let (mut s, l) = tilemap_sprite();
        // Simulate a tool that edits pixels directly without syncing.
        let mut cel = Cel { opacity: 255, pixels: vec![0; 16 * 16 * 4], ..Default::default() };
        cel.pixels[(9 * 16 + 9) * 4..][..4].copy_from_slice(&RED);
        s.cels[0][l] = Some(Arc::new(cel));
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(px(&back, 9, 9), RED);
        assert_eq!(back.tilesets[0].tiles.len(), 2);
    }

    #[test]
    fn convert_dedupes_and_converts_back() {
        let mut s = Sprite::new(16, 16);
        // The same 2x2 pattern in three of four 8x8 cells.
        for (ox, oy) in [(0, 0), (8, 0), (8, 8)] {
            s.draw_line(0, 0, ox + 1, oy + 1, ox + 2, oy + 1, RED, 1);
        }
        let before = s.render(0);
        assert!(convert_to_tilemap(&mut s, 0, 8, 8));
        assert_eq!(s.layers[0].kind, KIND_TILEMAP);
        assert_eq!(s.tilesets[0].tiles.len(), 2, "empty tile plus one unique tile");
        assert_eq!(s.render(0), before);
        let g = s.cels[0][0].as_ref().unwrap().tiles.as_ref().unwrap();
        assert_eq!(g.tiles, vec![1, 1, 0, 1]);
        assert!(!convert_to_tilemap(&mut s, 0, 8, 8), "already a tilemap");

        assert!(convert_to_normal(&mut s, 0));
        assert_eq!(s.layers[0].kind, 0);
        assert!(s.tilesets.is_empty());
        assert!(s.cels[0][0].as_ref().unwrap().tiles.is_none());
        assert_eq!(s.render(0), before);
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn convert_handles_partial_edge_tiles_and_shared_cels() {
        let mut s = Sprite::new(10, 10);
        s.draw_line(0, 0, 9, 9, 9, 9, RED, 1);
        s.add_frame(0, true);
        let before = (s.render(0), s.render(1));
        assert!(convert_to_tilemap(&mut s, 0, 8, 8));
        assert_eq!((s.render(0), s.render(1)), before);
        let g = s.cels[0][0].as_ref().unwrap().tiles.as_ref().unwrap();
        assert_eq!((g.w, g.h), (2, 2));
        // Duplicated frames are independent copies, so each frame gets its own tile grid.
        assert!(s.cels[1][0].as_ref().unwrap().tiles.is_some());
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back.render(1), before.1);
    }

    #[test]
    fn undo_restores_tiles() {
        let mut e = Editor::new(Sprite::new(16, 16));
        e.checkpoint();
        let l = add_tilemap_layer(&mut e.sprite, 0, 8, 8);
        e.checkpoint();
        e.sprite.draw_line(l, 0, 1, 1, 1, 1, RED, 1);
        assert_eq!(e.sprite.tilesets[0].tiles.len(), 2);
        assert!(e.undo());
        assert_eq!(e.sprite.tilesets[0].tiles.len(), 1);
        assert_eq!(e.sprite.render(0), vec![0; 16 * 16 * 4]);
        assert!(e.undo());
        assert!(e.sprite.tilesets.is_empty());
    }

    /// Hand-assembled chunks following the file format description, independent of the writer.
    #[test]
    fn parses_spec_layout() {
        let mut body = Vec::new();
        body.extend(7u32.to_le_bytes()); // tileset id
        body.extend(2u32.to_le_bytes()); // embedded only
        body.extend(2u32.to_le_bytes()); // 2 tiles
        body.extend(2u16.to_le_bytes()); // 2x2 tiles
        body.extend(2u16.to_le_bytes());
        body.extend(1i16.to_le_bytes());
        body.extend([0u8; 14]);
        body.extend(2u16.to_le_bytes());
        body.extend(b"ts");
        let mut pixels = vec![0u8; 16]; // tile 0 transparent
        pixels.extend([1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]);
        let z = deflate(&pixels);
        body.extend((z.len() as u32).to_le_bytes());
        body.extend(z);
        let raw = parse_tileset_chunk(&body, 4).unwrap();
        let (sets, ids) = finish_tilesets(vec![raw], |d| d.to_vec());
        assert_eq!(ids[&7], 0);
        assert_eq!(sets[0].name, "ts");
        assert_eq!(sets[0].tiles.len(), 2);
        assert_eq!(&sets[0].tiles[1][..4], &[1, 2, 3, 255]);

        // A 16-bit-per-tile cel using non-default masks: id in low 12 bits, flips at bits 12..14.
        let mut cel = Vec::new();
        cel.extend(2u16.to_le_bytes());
        cel.extend(1u16.to_le_bytes());
        cel.extend(16u16.to_le_bytes());
        cel.extend(0x0FFFu32.to_le_bytes());
        cel.extend(0x1000u32.to_le_bytes());
        cel.extend(0x2000u32.to_le_bytes());
        cel.extend(0x4000u32.to_le_bytes());
        cel.extend([0u8; 10]);
        let tiles: Vec<u8> = [1u16, 0x1001].iter().flat_map(|v| v.to_le_bytes()).collect();
        cel.extend(deflate(&tiles));
        let g = parse_tilemap_cel(&cel, -8, 4).unwrap();
        assert_eq!((g.x, g.y, g.w, g.h), (-8, 4, 2, 1));
        assert_eq!(g.tiles, vec![1, 1 | FLIP_X]);
    }

    fn corner_sprite() -> (Sprite, usize, u32) {
        let (mut s, l) = tilemap_sprite();
        let id = add_corner_tile(&mut s, 0);
        (s, l, id)
    }

    #[test]
    fn diagonal_flip_order_is_transpose_then_x_then_y() {
        let (mut s, l, id) = corner_sprite();
        // Red starts top-left. Diagonal+X rotates the tile 90 degrees clockwise: red goes top-right.
        stamp(&mut s, l, 0, 0, 0, id | FLIP_D | FLIP_X);
        assert_eq!(px(&s, 7, 0), RED);
        assert_eq!(px(&s, 7, 7), BLUE);
        assert_eq!(px(&s, 0, 0), GREEN);
        // Diagonal+Y rotates counter-clockwise: red goes bottom-left.
        stamp(&mut s, l, 0, 0, 0, id | FLIP_D | FLIP_Y);
        assert_eq!(px(&s, 0, 7), RED);
        assert_eq!(px(&s, 0, 0), BLUE);
        assert_eq!(px(&s, 7, 7), GREEN);
        // All three is the transpose mirrored on both axes (an anti-transpose).
        stamp(&mut s, l, 0, 0, 0, id | FLIP_D | FLIP_X | FLIP_Y);
        assert_eq!(px(&s, 7, 7), RED);
    }

    #[test]
    fn resize_canvas_moves_tile_grids() {
        let (s, l, id) = corner_sprite();
        let mut e = Editor::new(s);
        stamp(&mut e.sprite, l, 0, 0, 0, id);
        stamp(&mut e.sprite, l, 0, 8, 8, id | FLIP_X);
        let before = e.sprite.render(0);
        assert!(crate::paint::selection::resize_canvas(&mut e, 24, 20, 3, 2));
        let after = e.sprite.render(0);
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(after[((y + 2) * 24 + x + 3) * 4..][..4], before[(y * 16 + x) * 4..][..4]);
            }
        }
        let g = e.sprite.cels[0][l].as_ref().unwrap().tiles.as_ref().unwrap();
        assert_eq!((g.x, g.y), (3, 2));
        // The grid is no longer aligned to the canvas origin, but editing still works.
        e.sprite.draw_line(l, 0, 5, 4, 5, 4, BLUE, 1);
        assert_eq!(&e.sprite.tilesets[0].tiles[id as usize][(2 * 8 + 2) * 4..][..4], &BLUE);
        let back = ase::read(&ase::write(&e.sprite)).unwrap();
        assert_eq!(back.render(0), e.sprite.render(0));
        // Cropping back down keeps it a tilemap too.
        assert!(crate::paint::selection::resize_canvas(&mut e, 10, 10, -3, -2));
        assert_eq!(e.sprite.layers[l].kind, KIND_TILEMAP);
        assert_eq!(e.sprite.render(0).len(), 10 * 10 * 4);
        assert!(e.undo());
        assert_eq!(e.sprite.width, 24);
    }

    #[test]
    fn grids_far_off_canvas_are_trimmed_on_save() {
        let (mut s, l, id) = corner_sprite();
        let mut tiles = vec![0u32; 8752 * 2];
        tiles[8750] = id;
        tiles[8752 + 8751] = id | FLIP_Y;
        let grid = TileGrid { x: -70000, y: 0, w: 8752, h: 2, tiles };
        let cel = make_cel(255, grid, &s.tilesets[0], (0, 0), 16, 16);
        s.cels[0][l] = Some(Arc::new(cel));
        let before = s.render(0);
        assert_eq!(&before[..4], &RED);
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back.render(0), before);
        let g = back.cels[0][l].as_ref().unwrap().tiles.as_ref().unwrap();
        assert_eq!((g.x, g.y, g.w, g.h), (0, 0, 2, 2));
    }

    #[test]
    fn scale_by_whole_factor_scales_tiles() {
        let (mut s, l, id) = corner_sprite();
        stamp(&mut s, l, 0, 0, 0, id);
        stamp(&mut s, l, 0, 8, 0, id | FLIP_D | FLIP_X);
        s.draw_line(0, 0, 1, 1, 1, 1, GREEN, 1); // a normal layer too
        let mut expected = s.clone();
        // Reference: flatten first so everything is scaled as plain pixels.
        convert_to_normal(&mut expected, l);
        assert!(crate::paint::filters::scale_sprite(&mut expected, 32, 32));
        assert!(crate::paint::filters::scale_sprite(&mut s, 32, 32));
        assert_eq!(s.layers[l].kind, KIND_TILEMAP);
        assert_eq!((s.tilesets[0].tile_w, s.tilesets[0].tile_h), (16, 16));
        assert_eq!(s.render(0), expected.render(0));
        assert!(cel_in_sync(s.cels[0][l].as_ref().unwrap(), &s.tilesets[0], (0, 0), 32, 32));
        // Still editable and still saves as a tilemap.
        s.draw_line(l, 0, 3, 3, 3, 3, BLUE, 1);
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn scale_by_other_factors_converts_tilemaps_to_normal() {
        let (mut s, l, id) = corner_sprite();
        stamp(&mut s, l, 0, 0, 0, id);
        let mut expected = s.clone();
        convert_to_normal(&mut expected, l);
        assert!(crate::paint::filters::scale_sprite(&mut expected, 24, 24));
        assert!(crate::paint::filters::scale_sprite(&mut s, 24, 24));
        assert_eq!(s.layers[l].kind, 0);
        assert!(s.tilesets.is_empty());
        assert_eq!(s.render(0), expected.render(0));
        // Downscaling is not exact either.
        let (mut s, l, id) = corner_sprite();
        stamp(&mut s, l, 0, 0, 0, id);
        assert!(crate::paint::filters::scale_sprite(&mut s, 8, 8));
        assert_eq!(s.layers[l].kind, 0);
    }

    #[test]
    fn scaling_flattens_only_tilesets_that_cannot_scale() {
        let mut s = Sprite::new(8, 8);
        let a = add_tilemap_layer(&mut s, 0, 4, 4);
        let b = add_tilemap_layer(&mut s, a, 600, 600);
        s.draw_line(a, 0, 1, 1, 1, 1, RED, 1);
        s.draw_line(b, 0, 6, 6, 6, 6, BLUE, 1);
        assert!(crate::paint::filters::scale_sprite(&mut s, 16, 16)); // 1200 > 1024 for b
        assert_eq!(s.layers[a].kind, KIND_TILEMAP);
        assert_eq!(s.layers[b].kind, 0);
        assert_eq!(s.tilesets.len(), 1);
        assert_eq!((s.tilesets[0].tile_w, s.layers[a].tileset), (8, 0));
        assert_eq!(px(&s, 13, 13), BLUE);
        assert_eq!(px(&s, 3, 3), RED);
    }

    #[test]
    fn uneven_scale_factors_scale_tiles_unless_diagonal_flips_are_used() {
        let (mut s, l, id) = corner_sprite();
        stamp(&mut s, l, 0, 0, 0, id | FLIP_X);
        let mut expected = s.clone();
        convert_to_normal(&mut expected, l);
        assert!(crate::paint::filters::scale_sprite(&mut expected, 32, 48));
        assert!(crate::paint::filters::scale_sprite(&mut s, 32, 48));
        assert_eq!(s.layers[l].kind, KIND_TILEMAP);
        assert_eq!((s.tilesets[0].tile_w, s.tilesets[0].tile_h), (16, 24));
        assert_eq!(s.render(0), expected.render(0));
        // A diagonal flip would no longer be applicable to the now non-square tiles.
        let (mut s, l, id) = corner_sprite();
        stamp(&mut s, l, 0, 0, 0, id | FLIP_D);
        let before_px = s.render(0);
        assert!(crate::paint::filters::scale_sprite(&mut s, 32, 48));
        assert_eq!(s.layers[l].kind, 0);
        assert_eq!(s.render(0)[..4], before_px[..4]);
    }

    #[test]
    fn delete_tile_remaps_indices() {
        let (mut s, l, _) = corner_sprite(); // tile 1
        let t2 = s.tilesets[0].add_blank();
        Arc::make_mut(&mut s.tilesets[0].tiles[t2 as usize])[..4].copy_from_slice(&GREEN);
        let t3 = s.tilesets[0].add_blank();
        Arc::make_mut(&mut s.tilesets[0].tiles[t3 as usize])[..4].copy_from_slice(&BLUE);
        stamp(&mut s, l, 0, 0, 0, 1);
        stamp(&mut s, l, 0, 8, 0, t2);
        stamp(&mut s, l, 0, 0, 8, t3 | FLIP_X);
        assert!(!delete_tile(&mut s, 0, 0), "empty tile stays");
        assert!(!delete_tile(&mut s, 0, 9));
        assert!(delete_tile(&mut s, 0, t2));
        assert_eq!(s.tilesets[0].tiles.len(), 3);
        let g = s.cels[0][l].as_ref().unwrap().tiles.as_ref().unwrap();
        assert_eq!(g.tiles, vec![1, 0, 2 | FLIP_X, 0]);
        assert_eq!(px(&s, 8, 0), [0; 4]);
        assert_eq!(px(&s, 7, 8), BLUE);
        assert_eq!(&s.tilesets[0].tiles[2][..4], &BLUE);
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn duplicate_tile_copies_pixels() {
        let (mut s, _, id) = corner_sprite();
        let n = duplicate_tile(&mut s, 0, id).unwrap();
        assert_eq!(n, 2);
        assert_eq!(s.tilesets[0].tiles[1], s.tilesets[0].tiles[2]);
        assert!(
            !Arc::ptr_eq(&s.tilesets[0].tiles[1], &s.tilesets[0].tiles[2])
                || Arc::strong_count(&s.tilesets[0].tiles[1]) > 1
        );
        Arc::make_mut(&mut s.tilesets[0].tiles[2])[..4].copy_from_slice(&BLUE);
        assert_eq!(&s.tilesets[0].tiles[1][..4], &RED, "copies are independent");
        assert!(duplicate_tile(&mut s, 0, 99).is_none());
    }

    #[test]
    fn assign_tileset_switches_and_prunes() {
        let mut s = Sprite::new(16, 16);
        let a = add_tilemap_layer(&mut s, 0, 8, 8);
        let id = add_corner_tile(&mut s, 0);
        stamp(&mut s, a, 0, 0, 0, id);
        let b = add_tilemap_layer(&mut s, a, 8, 8);
        let c = add_tilemap_layer(&mut s, b, 4, 4);
        assert_eq!(s.tilesets.len(), 3);
        let b_id = s.tilesets[1].add_blank();
        Arc::make_mut(&mut s.tilesets[1].tiles[b_id as usize])[..4].copy_from_slice(&GREEN);
        stamp(&mut s, b, 0, 0, 0, b_id);
        stamp(&mut s, c, 0, 0, 0, 0);
        assert!(!assign_tileset(&mut s, b, 2), "tile size differs");
        assert!(!assign_tileset(&mut s, b, 1), "already assigned");
        assert!(!assign_tileset(&mut s, 0, 0), "not a tilemap");
        // Layer b moves to tileset 0, which has fewer tiles than index b_id? Both have 2 tiles.
        assert!(assign_tileset(&mut s, b, 0));
        assert_eq!(s.tilesets.len(), 2, "old tileset removed");
        assert_eq!(s.layers[b].tileset, 0);
        assert_eq!(s.layers[c].tileset, 1, "later tilesets renumbered");
        // b's cell showed tile 1, which now is the corner tile of tileset 0.
        assert_eq!(px(&s, 0, 0), RED);
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn unused_tilesets_are_kept_on_save() {
        // Aseprite keeps tilesets whose layers were deleted (its own files contain such orphans).
        let mut s = Sprite::new(8, 8);
        let a = add_tilemap_layer(&mut s, 0, 4, 4);
        let b = add_tilemap_layer(&mut s, a, 4, 4);
        s.draw_line(b, 0, 1, 1, 1, 1, RED, 1);
        s.tilesets[1].name = "kept".into();
        assert!(s.remove_layer(a));
        assert_eq!(s.tilesets.len(), 2);
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back.tilesets.len(), 2);
        assert_eq!(back.tilesets[1].name, "kept");
        assert_eq!(back.render(0), s.render(0));
        let layer = back.layers.iter().position(|l| l.kind == KIND_TILEMAP).unwrap();
        assert_eq!(back.layers[layer].tileset, s.layers.iter().find(|l| l.kind == KIND_TILEMAP).unwrap().tileset);
    }

    #[test]
    fn rename_tileset_ffi_is_undoable() {
        unsafe {
            let doc = Box::into_raw(Box::new(Editor::new(Sprite::new(8, 8))));
            let l = pixmith_add_tilemap_layer(doc, 0, 4, 4);
            assert_eq!(l, 1);
            let name = std::ffi::CString::new("Walls").unwrap();
            assert!(pixmith_tileset_rename(doc, 0, name.as_ptr()));
            assert!(!pixmith_tileset_rename(doc, 5, name.as_ptr()));
            let e = &mut *doc;
            assert_eq!(e.sprite.tilesets[0].name, "Walls");
            assert!(e.undo());
            assert_eq!(e.sprite.tilesets[0].name, "Tileset 1");
            let t = pixmith_tileset_duplicate_tile(doc, 0, 0);
            assert_eq!(t, 1);
            assert!(pixmith_tileset_delete_tile(doc, 0, 1));
            assert!(!pixmith_tileset_delete_tile(doc, 0, 0));
            let mut o = [0i32; 2];
            assert!(pixmith_tile_grid_origin(doc, 1, 0, o.as_mut_ptr()));
            drop(Box::from_raw(doc));
        }
    }

    #[test]
    fn flip_aware_convert_shares_flipped_tiles() {
        let mut s = Sprite::new(24, 8);
        // Cell 0: an asymmetric 8x8 pattern. Cell 1: its X mirror. Cell 2: its transpose.
        let pat = |x: usize, y: usize| x == 0 && y < 5 || y == 7 && x < 3;
        for y in 0..8 {
            for x in 0..8 {
                if pat(x, y) {
                    s.draw_line(0, 0, x as i32, y as i32, x as i32, y as i32, RED, 1);
                    s.draw_line(0, 0, 8 + 7 - x as i32, y as i32, 8 + 7 - x as i32, y as i32, RED, 1);
                    s.draw_line(0, 0, 16 + y as i32, x as i32, 16 + y as i32, x as i32, RED, 1);
                }
            }
        }
        let before = s.render(0);
        let mut exact = s.clone();
        assert!(convert_to_tilemap(&mut exact, 0, 8, 8));
        assert_eq!(exact.tilesets[0].tiles.len(), 4);

        assert!(convert_to_tilemap_with(&mut s, 0, 8, 8, true));
        assert_eq!(s.tilesets[0].tiles.len(), 2, "empty tile plus one shared tile");
        assert_eq!(s.render(0), before);
        let g = s.cels[0][0].as_ref().unwrap().tiles.as_ref().unwrap();
        assert_eq!(g.tiles[0], 1);
        assert_eq!(g.tiles[1] & TILE_ID_MASK, 1);
        assert_ne!(g.tiles[1] & FLIP_MASK, 0);
        assert_eq!(g.tiles[2] & TILE_ID_MASK, 1);
        assert_ne!(g.tiles[2] & FLIP_D, 0);
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back, s);
    }

    fn mode_sprite(mode: crate::document::color_mode::ColorMode, c1: [u8; 4], c2: [u8; 4]) -> Sprite {
        let mut s = Sprite::new(16, 16);
        s.palette = vec![[0; 4], c1, c2];
        s.transparent_index = 0;
        s.color_mode = mode;
        let l = add_tilemap_layer(&mut s, 0, 8, 8);
        let t = s.tilesets[0].add_blank();
        let tile = Arc::make_mut(&mut s.tilesets[0].tiles[t as usize]);
        tile[0..4].copy_from_slice(&c1);
        tile[4..8].copy_from_slice(&c2);
        stamp(&mut s, l, 0, 0, 0, t);
        stamp(&mut s, l, 0, 8, 8, t | FLIP_X);
        s
    }

    fn header_depth(bytes: &[u8]) -> u16 {
        u16::from_le_bytes([bytes[12], bytes[13]])
    }

    #[test]
    fn indexed_sprites_with_tilesets_round_trip_as_indexed() {
        let s = mode_sprite(crate::document::color_mode::ColorMode::Indexed, [200, 10, 10, 255], [10, 10, 200, 255]);
        let bytes = ase::write(&s);
        assert_eq!(header_depth(&bytes), 8);
        let back = ase::read(&bytes).unwrap();
        assert_eq!(back.color_mode, crate::document::color_mode::ColorMode::Indexed);
        assert_eq!(back, s);
        assert_eq!(back.render(0), s.render(0));
    }

    #[test]
    fn grayscale_sprites_with_tilesets_round_trip_as_grayscale() {
        let s = mode_sprite(crate::document::color_mode::ColorMode::Grayscale, [60, 60, 60, 255], [200, 200, 200, 128]);
        let bytes = ase::write(&s);
        assert_eq!(header_depth(&bytes), 16);
        let back = ase::read(&bytes).unwrap();
        assert_eq!(back.tilesets, s.tilesets);
        assert_eq!(back.render(0), s.render(0));
    }

    #[test]
    fn color_mode_conversion_converts_tiles() {
        use crate::document::color_mode::{convert, ColorMode};
        let mut s = mode_sprite(ColorMode::Rgba, [250, 10, 10, 255], [10, 10, 250, 255]);
        s.palette = vec![[0; 4], [255, 0, 0, 255], [0, 0, 255, 255]];
        convert(&mut s, ColorMode::Indexed, 0);
        assert_eq!(&s.tilesets[0].tiles[1][..4], &[255, 0, 0, 255]);
        assert_eq!(&s.tilesets[0].tiles[1][4..8], &[0, 0, 255, 255]);
        let l = s.layers.iter().position(|l| l.kind == KIND_TILEMAP).unwrap();
        assert!(cel_in_sync(s.cels[0][l].as_ref().unwrap(), &s.tilesets[0], (0, 0), 16, 16));
        assert_eq!(px(&s, 0, 0), [255, 0, 0, 255]);
        convert(&mut s, ColorMode::Grayscale, 0);
        let g = &s.tilesets[0].tiles[1];
        assert_eq!(g[0], g[1]);
        assert!(cel_in_sync(s.cels[0][l].as_ref().unwrap(), &s.tilesets[0], (0, 0), 16, 16));
    }

    #[test]
    fn tilemap_layer_without_tileset_degrades_to_normal() {
        let mut layers = vec![crate::document::Layer::new("a")];
        layers[0].kind = KIND_TILEMAP;
        layers[0].tileset = 3;
        resolve_layers(&mut layers, &HashMap::new());
        assert_eq!(layers[0].kind, 0);
    }
}
