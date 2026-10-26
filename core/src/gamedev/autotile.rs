//! Auto-tiling for tilemap layers (docs/SPEC-game.md section 2).
//!
//! * The user's intent (a terrain grid per frame) and manual overrides are the source of truth;
//!   the cel's tile grid is a cache that `rebuild` can recreate at any time.
//! * All state lives in the layer's user data as one `pixmith.autotile.v1:` line (after the
//!   user's own text), so it survives `.aseprite` save/load and undo snapshots.
//! * Grids use cell coordinates (canvas pixel / tile size).
//!
//! Directions (the "ring"): index 0..8 = N, NE, E, SE, S, SW, W, NW. Every tile carries the
//! terrain id of each ring direction (`peer`, 0 = empty) plus its own center terrain, like
//! Godot's peering bits. Modes only look at some directions: Sides = N E S W, Blob = all
//! eight (corners only count when both adjacent sides match), Corners (dual grid) = the four
//! diagonals.
//!
//! Dual grid: intent lives on the world cells, which act as *vertices* of the tile grid. The
//! tile grid is offset by half a tile (grid origin `-tile/2`, size `(W+1) x (H+1)`); display
//! cell (i, j) has world cells NW=(i-1,j-1) NE=(i,j-1) SE=(i,j) SW=(i-1,j) as its corners, so a
//! painted cell updates a 2x2 block.

use crate::document::iso::Iso;
use crate::document::tilemap::{render_cel, TileGrid, FLIP_D, FLIP_X, FLIP_Y, KIND_TILEMAP, TILE_ID_MASK};
use crate::document::{Editor, Sprite};
use std::collections::BTreeMap;
use std::ffi::{c_char, CStr, CString};
use std::ptr::null_mut;

pub type TerrainId = u8;
/// A tile value: index in the low 29 bits plus the flip flags of `tilemap`.
pub type TileRef = u32;

pub const PREFIX: &str = "pixmith.autotile.v1:";
const SEPARATOR: &str = "--";
pub const MAX_TERRAINS: usize = 15;

// Blob bit layout (spec): NW=128 N=1 NE=2 / W=64 E=4 / SW=32 S=16 SE=8.
const N: u8 = 1;
const NE: u8 = 2;
const E: u8 = 4;
const SE: u8 = 8;
const S: u8 = 16;
const SW: u8 = 32;
const W: u8 = 64;
const NW: u8 = 128;

/// Ring directions as (dx, dy): N, NE, E, SE, S, SW, W, NW.
pub const DIRS: [(i32, i32); 8] = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];

/// Corner bits of a blob mask only count when both adjacent sides are set.
pub fn reduce(mut m: u8) -> u8 {
    if m & N == 0 || m & E == 0 {
        m &= !NE;
    }
    if m & S == 0 || m & E == 0 {
        m &= !SE;
    }
    if m & S == 0 || m & W == 0 {
        m &= !SW;
    }
    if m & N == 0 || m & W == 0 {
        m &= !NW;
    }
    m
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// 4-bit: N=1 E=2 S=4 W=8.
    Sides,
    /// 8-bit reduced to 47 values (spec layout, bit i = ring direction i).
    Blob,
    /// Dual grid, 4 corner bits: TL=1 TR=2 BR=4 BL=8.
    Corners,
}

impl Mode {
    pub fn from_i32(v: i32) -> Option<Mode> {
        match v {
            0 => Some(Mode::Sides),
            1 => Some(Mode::Blob),
            2 => Some(Mode::Corners),
            _ => None,
        }
    }
    pub fn to_i32(self) -> i32 {
        self as i32
    }
    /// Ring directions this mode looks at.
    pub fn dirs(self) -> &'static [usize] {
        match self {
            Mode::Sides => &[0, 2, 4, 6],
            Mode::Blob => &[0, 1, 2, 3, 4, 5, 6, 7],
            Mode::Corners => &[7, 1, 3, 5],
        }
    }
    /// Mask bit of a ring direction in this mode.
    pub fn bit(self, dir: usize) -> Option<u8> {
        self.dirs().iter().position(|&d| d == dir).map(|i| match self {
            Mode::Blob => 1u8 << dir,
            _ => 1u8 << i,
        })
    }
    /// Every mask value this mode can produce, ascending.
    pub fn masks(self) -> Vec<u8> {
        match self {
            Mode::Sides | Mode::Corners => (0..16).collect(),
            Mode::Blob => {
                let mut v: Vec<u8> = (0..=255u8).map(reduce).collect();
                v.sort_unstable();
                v.dedup();
                v
            }
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Mode::Sides => "sides",
            Mode::Blob => "blob",
            Mode::Corners => "corners",
        }
    }
}

// MARK: Transforms

/// Where a ring direction points after a tile is drawn with transform `t`
/// (bit 0 = flip X, bit 1 = flip Y, bit 2 = diagonal; see `tilemap::source_pixel`).
pub fn transform_dir(dir: usize, t: u8) -> usize {
    let (mut dx, mut dy) = DIRS[dir];
    if t & 4 != 0 {
        std::mem::swap(&mut dx, &mut dy);
    }
    if t & 1 != 0 {
        dx = -dx;
    }
    if t & 2 != 0 {
        dy = -dy;
    }
    DIRS.iter().position(|&d| d == (dx, dy)).unwrap()
}

/// Like [`transform_dir`] for isometric tiles. A flip is a flip of the picture on screen: flipping X mirrors the
/// diamond left to right, which swaps the two cell axes (`i` runs down-right, `j` down-left), so the neighbour
/// at (dx, dy) in cell space moves to (dy, dx); flipping Y mirrors it top to bottom, which sends (dx, dy) to
/// (-dy, -dx). The diagonal flip does not exist for isometric tiles.
pub fn transform_dir_iso(dir: usize, t: u8) -> usize {
    let (mut dx, mut dy) = DIRS[dir];
    if t & 1 != 0 {
        (dx, dy) = (dy, dx);
    }
    if t & 2 != 0 {
        (dx, dy) = (-dy, -dx);
    }
    DIRS.iter().position(|&d| d == (dx, dy)).unwrap()
}

/// The tile value flags of a transform.
pub fn transform_flags(t: u8) -> u32 {
    ((t & 1) as u32 * FLIP_X) | (((t >> 1) & 1) as u32 * FLIP_Y) | (((t >> 2) & 1) as u32 * FLIP_D)
}

fn transform_peers(p: &[u8; 8], t: u8, iso: bool) -> [u8; 8] {
    let mut o = [0u8; 8];
    for (d, &v) in p.iter().enumerate() {
        o[if iso { transform_dir_iso(d, t) } else { transform_dir(d, t) }] = v;
    }
    o
}

// MARK: Model

#[derive(Clone, Debug, PartialEq)]
pub struct Terrain {
    pub id: TerrainId,
    pub name: String,
    pub color: [u8; 4],
}

/// Auto-tile data of one tile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileMeta {
    /// Center terrain (none for dual-grid tiles, which are described by their corners).
    pub terrain: Option<TerrainId>,
    /// Terrain id per ring direction (0 = empty).
    pub peer: [u8; 8],
    pub weight: f32,
    /// Transforms this tile may be drawn with to match other masks: bit 0 flip X, bit 1 flip Y,
    /// bit 2 diagonal (square tiles only).
    pub sym: u8,
}

impl TileMeta {
    /// The single-terrain mask in a mode's bit layout (a bit is set when that peer is non-empty).
    pub fn mask(&self, mode: Mode) -> u8 {
        mode.dirs().iter().filter(|&&d| self.peer[d] != 0).fold(0, |m, &d| m | mode.bit(d).unwrap())
    }
    pub fn peers(&self) -> [Option<TerrainId>; 8] {
        self.peer.map(|p| (p != 0).then_some(p))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grid<T> {
    pub w: u32,
    pub h: u32,
    pub cells: Vec<T>,
}

impl<T: Clone + Default> Grid<T> {
    pub fn new(w: u32, h: u32) -> Self {
        Grid { w, h, cells: vec![T::default(); w as usize * h as usize] }
    }
    pub fn get(&self, x: i32, y: i32) -> Option<&T> {
        (x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h)
            .then(|| &self.cells[y as usize * self.w as usize + x as usize])
    }
    pub fn set(&mut self, x: i32, y: i32, v: T) -> bool {
        if x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h {
            self.cells[y as usize * self.w as usize + x as usize] = v;
            true
        } else {
            false
        }
    }
    fn resized(&self, w: u32, h: u32) -> Grid<T> {
        let mut g = Grid::new(w, h);
        for y in 0..h.min(self.h) {
            for x in 0..w.min(self.w) {
                g.cells[y as usize * w as usize + x as usize] =
                    self.cells[y as usize * self.w as usize + x as usize].clone();
            }
        }
        g
    }
}

/// The auto-tile state of one tilemap layer (a "terrain set").
#[derive(Clone, Debug, PartialEq)]
pub struct Autotile {
    pub mode: Mode,
    /// Terrains in priority order (first = highest priority).
    pub terrains: Vec<Terrain>,
    /// Tile index -> metadata (only tiles that were given terrain data).
    pub tiles: BTreeMap<u32, TileMeta>,
    pub seed: u32,
    /// Treat cells outside the map as the same terrain (default: empty).
    pub edge_same: bool,
    /// The user's intent per frame (world cells; keyed by the first frame of a linked group).
    pub intent: BTreeMap<u32, Grid<Option<TerrainId>>>,
    /// Manual tiles per frame (display cells); they always win.
    pub overrides: BTreeMap<u32, Grid<Option<TileRef>>>,
    /// The layer is isometric: cells are (i, j) diamonds and flips move neighbours as `transform_dir_iso` says.
    /// Not stored; `load` sets it from the layer's tileset.
    pub iso: bool,
}

impl Default for Autotile {
    fn default() -> Self {
        Autotile {
            mode: Mode::Blob,
            terrains: vec![Terrain { id: 1, name: "Terrain".into(), color: [90, 170, 70, 255] }],
            tiles: BTreeMap::new(),
            seed: 0,
            edge_same: false,
            intent: BTreeMap::new(),
            overrides: BTreeMap::new(),
            iso: false,
        }
    }
}

/// What `Autotile::choose` decided for a cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Choice {
    pub value: TileRef,
    /// False when no tile has exactly the wanted terrain/peers (a fallback was used).
    pub exact: bool,
}

impl Autotile {
    /// The first (default) terrain.
    pub fn terrain(&self) -> &Terrain {
        &self.terrains[0]
    }
    pub fn mode(&self) -> Mode {
        self.mode
    }
    pub fn terrain_by_id(&self, id: TerrainId) -> Option<&Terrain> {
        self.terrains.iter().find(|t| t.id == id)
    }
    pub fn tile_meta(&self, tile: u32) -> Option<&TileMeta> {
        self.tiles.get(&tile)
    }
    /// Priority rank of a terrain (0 = highest).
    fn rank(&self, id: TerrainId) -> usize {
        self.terrains.iter().position(|t| t.id == id).unwrap_or(usize::MAX)
    }
    pub fn intent_at(&self, frame: u32, x: i32, y: i32) -> Option<TerrainId> {
        self.intent.get(&frame).and_then(|g| g.get(x, y)).copied().flatten()
    }
    pub fn override_at(&self, frame: u32, x: i32, y: i32) -> Option<TileRef> {
        self.overrides.get(&frame).and_then(|g| g.get(x, y)).copied().flatten()
    }

    /// Terrain at a world cell for neighbour lookups (outside the map: `edge`).
    fn world(&self, frame: u32, x: i32, y: i32, dims: (u32, u32), edge: u8) -> u8 {
        if x < 0 || y < 0 || x as u32 >= dims.0 || y as u32 >= dims.1 {
            return edge;
        }
        self.intent_at(frame, x, y).unwrap_or(0)
    }

    /// Center terrain and desired ring of a display cell: for Sides/Blob the cell's own terrain
    /// and its neighbours, for Corners the four world cells around display cell (x, y).
    pub fn wanted(&self, frame: u32, x: i32, y: i32, dims: (u32, u32)) -> Option<(Option<TerrainId>, [u8; 8])> {
        let mut want = [0u8; 8];
        match self.mode {
            Mode::Corners => {
                let edge = if self.edge_same { self.edge_fill(frame) } else { 0 };
                let g = |dx: i32, dy: i32| self.world(frame, x + dx, y + dy, dims, edge);
                want[7] = g(-1, -1);
                want[1] = g(0, -1);
                want[3] = g(0, 0);
                want[5] = g(-1, 0);
                want.iter().any(|&v| v != 0).then_some((None, want))
            }
            _ => {
                let c = self.intent_at(frame, x, y)?;
                let edge = if self.edge_same { c } else { 0 };
                for &d in self.mode.dirs() {
                    want[d] = self.world(frame, x + DIRS[d].0, y + DIRS[d].1, dims, edge);
                }
                if self.mode == Mode::Blob {
                    // A corner only counts when both adjacent sides show the same terrain.
                    for d in [1, 3, 5, 7] {
                        let (a, b) = (want[(d + 7) % 8], want[(d + 1) % 8]);
                        if want[d] != a || want[d] != b {
                            want[d] = 0;
                        }
                    }
                }
                Some((Some(c), want))
            }
        }
    }

    /// Dual-grid edge fill: the first painted terrain of the frame (or the first terrain).
    fn edge_fill(&self, frame: u32) -> u8 {
        self.intent.get(&frame).and_then(|g| g.cells.iter().flatten().next().copied()).unwrap_or(self.terrains[0].id)
    }

    fn exact_candidates(&self, center: Option<TerrainId>, want: &[u8; 8], square: bool) -> Vec<(u32, u8, f32)> {
        let dirs = self.mode.dirs();
        let mut out = Vec::new();
        for (&i, m) in &self.tiles {
            if i == 0 || m.weight <= 0.0 || (self.mode != Mode::Corners && m.terrain != center) {
                continue;
            }
            for t in 0..8u8 {
                if t & !m.sym != 0 || (t & 4 != 0 && !square) {
                    continue;
                }
                let p = transform_peers(&m.peer, t, self.iso);
                if dirs.iter().all(|&d| p[d] == want[d]) {
                    out.push((i, t, m.weight));
                    break;
                }
            }
        }
        out
    }

    /// Picks a tile for a wanted terrain/ring: an exact match chosen by weighted deterministic
    /// hash, else (after letting higher-priority terrains cover lower ones) the closest tile.
    pub fn choose(&self, center: Option<TerrainId>, want: &[u8; 8], x: i32, y: i32, square: bool) -> Choice {
        let pick = |c: &[(u32, u8, f32)]| {
            let list: Vec<(u32, f32)> = c.iter().enumerate().map(|(k, &(_, _, w))| (k as u32, w)).collect();
            let k = pick_weighted(&list, hash(x, y, self.seed)) as usize;
            c[k].0 | transform_flags(c[k].1)
        };
        let ex = self.exact_candidates(center, want, square);
        if !ex.is_empty() {
            return Choice { value: pick(&ex), exact: true };
        }
        if let Some(c) = center {
            let rc = self.rank(c);
            let mut w2 = *want;
            for d in self.mode.dirs() {
                if w2[*d] != 0 && w2[*d] != c && self.rank(w2[*d]) > rc {
                    w2[*d] = 0;
                }
            }
            if w2 != *want {
                let ex = self.exact_candidates(center, &w2, square);
                if !ex.is_empty() {
                    return Choice { value: pick(&ex), exact: true };
                }
            }
        }
        // Closest tile: most matching directions (sides count double in blob mode).
        let dirs = self.mode.dirs();
        let mut best: Option<(i32, u32, u8)> = None;
        for (&i, m) in &self.tiles {
            if i == 0 || m.weight <= 0.0 {
                continue;
            }
            for t in 0..8u8 {
                if t & !m.sym != 0 || (t & 4 != 0 && !square) {
                    continue;
                }
                let p = transform_peers(&m.peer, t, self.iso);
                let mut score: i32 = dirs
                    .iter()
                    .map(|&d| {
                        if p[d] == want[d] {
                            if self.mode == Mode::Blob && d % 2 == 0 {
                                2
                            } else {
                                1
                            }
                        } else {
                            0
                        }
                    })
                    .sum();
                if self.mode != Mode::Corners && m.terrain == center {
                    score += 100;
                }
                if best.is_none_or(|b| score > b.0) {
                    best = Some((score, i, t));
                }
            }
        }
        Choice { value: best.map_or(0, |b| b.1 | transform_flags(b.2)), exact: false }
    }

    /// Like [`resolve`](Self::resolve), but `None` when the engine has nothing to say about the cell: no
    /// override and no terrain around it. Such a cell keeps whatever tile is in it unless that tile is an
    /// auto-tile (has terrain data), which is then stale and must go.
    fn resolve_or_keep(&self, frame: u32, x: i32, y: i32, dims: (u32, u32), square: bool) -> Option<TileRef> {
        if self.override_at(frame, x, y).is_none() && self.wanted(frame, x, y, dims).is_none() {
            return None;
        }
        Some(self.resolve(frame, x, y, dims, square).0)
    }

    /// The tile value and whether it is a fallback for one display cell (overrides win).
    pub fn resolve(&self, frame: u32, x: i32, y: i32, dims: (u32, u32), square: bool) -> (TileRef, bool) {
        if let Some(v) = self.override_at(frame, x, y) {
            return (v, false);
        }
        match self.wanted(frame, x, y, dims) {
            None => (0, false),
            Some((c, want)) => {
                let ch = self.choose(c, &want, x, y, square);
                (ch.value, !ch.exact)
            }
        }
    }
}

/// Deterministic hash of a cell and seed (splitmix64 finalizer).
pub fn hash(x: i32, y: i32, seed: u32) -> u64 {
    let mut z = (x as u32 as u64) | (y as u32 as u64) << 32;
    z ^= (seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn pick_weighted(c: &[(u32, f32)], h: u64) -> u32 {
    let total: f32 = c.iter().map(|x| x.1).sum();
    let mut r = (h >> 11) as f64 / (1u64 << 53) as f64 * total as f64;
    for &(i, w) in c {
        if r < w as f64 {
            return i;
        }
        r -= w as f64;
    }
    c[c.len() - 1].0
}

// MARK: Persistence

fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '%' | ';' | '|' | ':' | ',' | '\n' | '\r' | '*' | '=' => o.push_str(&format!("%{:02X}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

fn unesc(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() {
            if let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn rle<T: Copy + PartialEq>(g: &[T], enc: impl Fn(T) -> u32) -> String {
    let mut parts = Vec::new();
    let mut i = 0;
    while i < g.len() {
        let mut j = i;
        while j < g.len() && g[j] == g[i] {
            j += 1;
        }
        parts.push(format!("{}*{}", enc(g[i]), j - i));
        i = j;
    }
    parts.join(",")
}

fn unrle(s: &str, n: usize) -> Option<Vec<u32>> {
    let mut v = Vec::with_capacity(n);
    for p in s.split(',').filter(|p| !p.is_empty()) {
        let (a, b) = p.split_once('*')?;
        let (a, b): (u32, usize) = (a.parse().ok()?, b.parse().ok()?);
        if v.len() + b > n {
            return None;
        }
        v.extend(std::iter::repeat_n(a, b));
    }
    (v.len() == n).then_some(v)
}

impl Autotile {
    /// Compact single-line encoding (no newlines).
    pub fn encode(&self) -> String {
        let mut f = vec![
            format!("m={}", self.mode.to_i32()),
            format!("s={}", self.seed),
            format!("e={}", self.edge_same as u8),
        ];
        let terr: Vec<String> = self
            .terrains
            .iter()
            .map(|t| {
                format!(
                    "{}:{}:{:02x}{:02x}{:02x}{:02x}",
                    t.id,
                    esc(&t.name),
                    t.color[0],
                    t.color[1],
                    t.color[2],
                    t.color[3]
                )
            })
            .collect();
        f.push(format!("t={}", terr.join(",")));
        let tiles: Vec<String> = self
            .tiles
            .iter()
            .map(|(i, m)| {
                let peers: String = m.peer.iter().map(|&p| char::from_digit(p as u32, 16).unwrap_or('0')).collect();
                format!("{}:{}:{}:{}:{}", i, m.terrain.unwrap_or(0), peers, m.weight, m.sym)
            })
            .collect();
        f.push(format!("k={}", tiles.join(",")));
        let grids = |name: &str, it: Vec<(u32, u32, u32, String)>| {
            format!(
                "{}={}",
                name,
                it.iter().map(|(fr, w, h, r)| format!("{fr}:{w}:{h}:{r}")).collect::<Vec<_>>().join("|")
            )
        };
        f.push(grids(
            "i",
            self.intent.iter().map(|(fr, g)| (*fr, g.w, g.h, rle(&g.cells, |c| c.unwrap_or(0) as u32))).collect(),
        ));
        f.push(grids(
            "o",
            self.overrides
                .iter()
                .map(|(fr, g)| (*fr, g.w, g.h, rle(&g.cells, |c| c.map_or(0, |v| v.wrapping_add(1)))))
                .collect(),
        ));
        f.join(";")
    }

    pub fn decode(s: &str) -> Option<Autotile> {
        let mut at = Autotile { terrains: Vec::new(), ..Default::default() };
        for field in s.split(';') {
            let Some((k, v)) = field.split_once('=') else { continue };
            match k {
                "m" => at.mode = Mode::from_i32(v.parse().ok()?)?,
                "s" => at.seed = v.parse().ok()?,
                "e" => at.edge_same = v == "1",
                "t" => {
                    for e in v.split(',').filter(|e| !e.is_empty()) {
                        let p: Vec<&str> = e.split(':').collect();
                        if p.len() != 3 || p[2].len() != 8 {
                            return None;
                        }
                        let mut color = [0u8; 4];
                        for (n, c) in color.iter_mut().enumerate() {
                            *c = u8::from_str_radix(&p[2][n * 2..n * 2 + 2], 16).ok()?;
                        }
                        at.terrains.push(Terrain { id: p[0].parse().ok()?, name: unesc(p[1]), color });
                    }
                }
                "k" => {
                    for e in v.split(',').filter(|e| !e.is_empty()) {
                        let p: Vec<&str> = e.split(':').collect();
                        if p.len() != 5 || p[2].len() != 8 {
                            return None;
                        }
                        let mut peer = [0u8; 8];
                        for (n, c) in p[2].chars().enumerate() {
                            peer[n] = c.to_digit(16)? as u8;
                        }
                        let t: u8 = p[1].parse().ok()?;
                        at.tiles.insert(
                            p[0].parse().ok()?,
                            TileMeta {
                                terrain: (t != 0).then_some(t),
                                peer,
                                weight: p[3].parse().ok()?,
                                sym: p[4].parse().ok()?,
                            },
                        );
                    }
                }
                "i" | "o" => {
                    for e in v.split('|').filter(|e| !e.is_empty()) {
                        let p: Vec<&str> = e.splitn(4, ':').collect();
                        if p.len() != 4 {
                            return None;
                        }
                        let (fr, w, h): (u32, u32, u32) = (p[0].parse().ok()?, p[1].parse().ok()?, p[2].parse().ok()?);
                        if w as u64 * h as u64 > 1 << 24 {
                            return None;
                        }
                        let vals = unrle(p[3], w as usize * h as usize)?;
                        if k == "i" {
                            at.intent.insert(
                                fr,
                                Grid { w, h, cells: vals.iter().map(|&c| (c != 0).then_some(c as u8)).collect() },
                            );
                        } else {
                            at.overrides.insert(
                                fr,
                                Grid {
                                    w,
                                    h,
                                    cells: vals.iter().map(|&c| (c != 0).then(|| c.wrapping_sub(1))).collect(),
                                },
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        if at.terrains.is_empty() {
            return None;
        }
        Some(at)
    }
}

/// Splits layer text into (the user's own text, the autotile payload if present).
pub fn split_text(text: &str) -> (String, Option<Autotile>) {
    let mut user: Vec<&str> = Vec::new();
    let mut payload = None;
    for line in text.split('\n') {
        if let Some(p) = line.strip_prefix(PREFIX) {
            if payload.is_none() {
                payload = Autotile::decode(p);
                if payload.is_some() {
                    continue;
                }
            }
        }
        user.push(line);
    }
    if payload.is_some() && user.last() == Some(&SEPARATOR) {
        user.pop();
    }
    (user.join("\n"), payload)
}

pub fn join_text(user: &str, at: &Autotile) -> String {
    let line = format!("{PREFIX}{}", at.encode());
    if user.is_empty() {
        line
    } else {
        format!("{user}\n{SEPARATOR}\n{line}")
    }
}

/// Loads a layer's auto-tile state (`None` if the layer has none).
pub fn load(s: &Sprite, layer: usize) -> Option<Autotile> {
    let t = s.layers.get(layer)?.user_data.text.as_deref()?;
    let mut at = split_text(t).1?;
    if is_iso_layer(s, layer) {
        at.iso = true;
        // The dual grid needs a half-cell shift of the tile grid, which isometric layers do not have.
        if at.mode == Mode::Corners {
            at.mode = Mode::Blob;
        }
    }
    Some(at)
}

/// Whether the layer is an isometric tilemap layer.
fn is_iso_layer(s: &Sprite, layer: usize) -> bool {
    s.layers
        .get(layer)
        .filter(|l| l.kind == KIND_TILEMAP)
        .and_then(|l| s.tilesets.get(l.tileset as usize))
        .is_some_and(|t| t.is_iso())
}

pub fn save(s: &mut Sprite, layer: usize, at: &Autotile) {
    if let Some(l) = s.layers.get_mut(layer) {
        let user = l.user_data.text.as_deref().map(|t| split_text(t).0).unwrap_or_default();
        l.user_data.text = Some(join_text(&user, at));
    }
}

/// The layer's own user text without the auto-tile payload.
pub fn user_text(s: &Sprite, layer: usize) -> String {
    s.layers.get(layer).and_then(|l| l.user_data.text.as_deref()).map(|t| split_text(t).0).unwrap_or_default()
}

// MARK: Geometry and writing the cache

#[derive(Clone, Copy)]
struct Ctx {
    ts: usize,
    tw: u32,
    th: u32,
    /// World (intent) size in cells.
    dims: (u32, u32),
    dual: bool,
    square: bool,
    /// The layer's isometric grid (cells are (i, j) diamonds placed at the layer's offset).
    iso: Option<Iso>,
    /// Layer offset and offset-independent render settings.
    offset: (i32, i32),
}

impl Ctx {
    /// Size of the display (tile) grid.
    fn grid_dims(&self) -> (u32, u32) {
        if self.dual {
            (self.dims.0 + 1, self.dims.1 + 1)
        } else {
            self.dims
        }
    }
    fn origin(&self) -> (i32, i32) {
        if self.iso.is_some() {
            (0, 0)
        } else if self.dual {
            (-((self.tw / 2) as i32), -((self.th / 2) as i32))
        } else {
            (0, 0)
        }
    }
    /// Display cell containing a canvas pixel.
    fn display_cell(&self, px: i32, py: i32) -> (i32, i32) {
        if let Some(iso) = &self.iso {
            let (i, j) = iso.cell_at(px, py);
            return (
                i.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                j.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            );
        }
        let (ox, oy) = self.origin();
        ((px - ox).div_euclid(self.tw as i32), (py - oy).div_euclid(self.th as i32))
    }
}

fn ctx_for(s: &Sprite, layer: usize, mode: Mode) -> Option<Ctx> {
    let l = s.layers.get(layer)?;
    if l.kind != KIND_TILEMAP || l.tileset as usize >= s.tilesets.len() {
        return None;
    }
    let t = &s.tilesets[l.tileset as usize];
    let iso = t.iso_grid(l.offset);
    let dims = match &iso {
        // Cells (i, j) with i, j >= 0 up to the last one that reaches into the canvas.
        Some(g) => {
            let (_, hi_i, _, hi_j) = g.cells_touching(0, 0, s.width as i64, s.height as i64, 0);
            ((hi_i + 1).clamp(1, ISO_MAX_DIM as i64) as u32, (hi_j + 1).clamp(1, ISO_MAX_DIM as i64) as u32)
        }
        None => (s.width.div_ceil(t.tile_w), s.height.div_ceil(t.tile_h)),
    };
    Some(Ctx {
        ts: l.tileset as usize,
        tw: t.tile_w,
        th: t.tile_h,
        dims,
        dual: mode == Mode::Corners && iso.is_none(),
        square: t.tile_w == t.tile_h && iso.is_none(),
        iso,
        offset: l.offset,
    })
}

/// Most cells along one axis of an isometric terrain map (the map grid holds at most 2^24 cells).
const ISO_MAX_DIM: u32 = 4096;

fn is_tilemap(s: &Sprite, layer: usize) -> bool {
    s.layers.get(layer).is_some_and(|l| l.kind == KIND_TILEMAP && (l.tileset as usize) < s.tilesets.len())
}

/// The frame that owns the intent of `frame`: the first frame of its linked group.
fn key_frame(s: &Sprite, layer: usize, frame: usize) -> u32 {
    s.link_siblings(layer, frame).into_iter().chain([frame]).min().unwrap_or(frame) as u32
}

fn ensure_grids(at: &mut Autotile, kf: u32, c: &Ctx) {
    let d = c.dims;
    let g = at.intent.entry(kf).or_insert_with(|| Grid::new(d.0, d.1));
    if (g.w, g.h) != d {
        *g = g.resized(d.0, d.1);
    }
    let gd = c.grid_dims();
    if let Some(g) = at.overrides.get_mut(&kf) {
        if (g.w, g.h) != gd {
            *g = g.resized(gd.0, gd.1);
        }
    }
}

/// Writes display-cell values into the cel's tile grid (normalising its geometry first) and
/// re-renders the pixels once. Returns true if existing tiles were lost while the grid geometry
/// changed (the caller then resolves everything).
/// An update value for `apply`: the engine has nothing to place here, so a plain tile stays and an auto-tile goes.
const KEEP: u32 = u32::MAX;

fn apply(s: &mut Sprite, layer: usize, frame: usize, at: &Autotile, c: &Ctx, updates: &[(i32, i32, u32)]) -> bool {
    let (w, h) = (s.width as usize, s.height as usize);
    let ts = s.tilesets[c.ts].clone();
    let (ox, oy) = c.origin();
    let (gw, gh) = c.grid_dims();
    let Some(cel) = s.cel_mut(layer, frame) else { return false };
    let old = cel.tiles.take();
    let mut lost = false;
    let mut grid = match old {
        Some(g) if (g.x, g.y, g.w, g.h) == (ox, oy, gw, gh) => g,
        Some(old) if c.iso.is_some() => {
            // Cells are addressed by (i, j): keep the tiles that lie inside the new range.
            let mut ng = TileGrid { x: 0, y: 0, w: gw, h: gh, tiles: vec![0; gw as usize * gh as usize] };
            for ty in 0..old.h as i64 {
                for tx in 0..old.w as i64 {
                    let v = old.tiles[ty as usize * old.w as usize + tx as usize];
                    let (i, j) = (old.x as i64 + tx, old.y as i64 + ty);
                    if v == 0 {
                        continue;
                    }
                    if i >= 0 && j >= 0 && i < gw as i64 && j < gh as i64 {
                        ng.tiles[j as usize * gw as usize + i as usize] = v;
                    } else {
                        lost = true;
                    }
                }
            }
            ng
        }
        old => {
            let mut ng = TileGrid { x: ox, y: oy, w: gw, h: gh, tiles: vec![0; gw as usize * gh as usize] };
            if let Some(old) = old {
                // Keep tiles whose cells line up with the new geometry.
                for ty in 0..old.h as i32 {
                    for tx in 0..old.w as i32 {
                        let v = old.tiles[ty as usize * old.w as usize + tx as usize];
                        if v == 0 {
                            continue;
                        }
                        let (px, py) = (old.x + tx * c.tw as i32, old.y + ty * c.th as i32);
                        let (nx, ny) = ((px - ox).div_euclid(c.tw as i32), (py - oy).div_euclid(c.th as i32));
                        if (px - ox).rem_euclid(c.tw as i32) == 0
                            && (py - oy).rem_euclid(c.th as i32) == 0
                            && nx >= 0
                            && ny >= 0
                            && (nx as u32) < gw
                            && (ny as u32) < gh
                        {
                            ng.tiles[ny as usize * gw as usize + nx as usize] = v;
                        } else {
                            lost = true;
                        }
                    }
                }
            }
            ng
        }
    };
    for &(x, y, v) in updates {
        if x >= 0 && y >= 0 && (x as u32) < gw && (y as u32) < gh {
            let slot = &mut grid.tiles[y as usize * gw as usize + x as usize];
            *slot = if v != KEEP {
                v
            } else if at.tiles.contains_key(&(*slot & TILE_ID_MASK)) {
                0
            } else {
                *slot
            };
        }
    }
    cel.pixels = render_cel(&grid, &ts, c.offset, w, h);
    cel.tiles = Some(grid);
    lost
}

fn resolved(at: &Autotile, kf: u32, x: i32, y: i32, c: &Ctx, keep_plain: bool) -> u32 {
    if keep_plain {
        at.resolve_or_keep(kf, x, y, c.dims, c.square).unwrap_or(KEEP)
    } else {
        at.resolve(kf, x, y, c.dims, c.square).0
    }
}

fn resolve_all_cells(c: &Ctx) -> Vec<(i32, i32)> {
    let (gw, gh) = c.grid_dims();
    (0..gh as i32).flat_map(|y| (0..gw as i32).map(move |x| (x, y))).collect()
}

/// Re-resolves display cells. With `keep_plain`, cells without terrain data (and no override) keep a plain tile.
fn resolve_cells(
    s: &mut Sprite,
    layer: usize,
    frame: usize,
    at: &Autotile,
    c: &Ctx,
    cells: &[(i32, i32)],
    keep_plain: bool,
) {
    let kf = key_frame(s, layer, frame);
    let (gw, gh) = c.grid_dims();
    let mut updates = Vec::with_capacity(cells.len());
    for &(x, y) in cells {
        if x < 0 || y < 0 || x as u32 >= gw || y as u32 >= gh {
            continue;
        }
        updates.push((x, y, resolved(at, kf, x, y, c, keep_plain)));
    }
    if apply(s, layer, frame, at, c, &updates) {
        let all: Vec<(i32, i32, u32)> =
            resolve_all_cells(c).into_iter().map(|(x, y)| (x, y, resolved(at, kf, x, y, c, keep_plain))).collect();
        apply(s, layer, frame, at, c, &all);
    }
}

/// Display cells whose result depends on world cell (x, y): 3x3, or 2x2 for dual-grid.
fn affected(mode: Mode, x: i32, y: i32) -> Vec<(i32, i32)> {
    match mode {
        Mode::Corners => (0..=1).flat_map(|dy| (0..=1).map(move |dx| (x + dx, y + dy))).collect(),
        _ => (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (x + dx, y + dy))).collect(),
    }
}

// MARK: Editing

fn payload(s: &Sprite, layer: usize) -> Option<(Autotile, Ctx)> {
    let mut at = load(s, layer).unwrap_or_default();
    let c = ctx_for(s, layer, at.mode)?;
    at.iso = c.iso.is_some();
    Some((at, c))
}

/// Defines (or updates) the layer's first terrain and the mode. Creates the payload when needed.
pub fn set_terrain(s: &mut Sprite, layer: usize, name: &str, color: [u8; 4], mode: Mode) -> bool {
    if !is_tilemap(s, layer) || (mode == Mode::Corners && is_iso_layer(s, layer)) {
        return false;
    }
    let mut at = load(s, layer).unwrap_or_default();
    at.terrains[0].name = name.to_string();
    at.terrains[0].color = color;
    save(s, layer, &at);
    set_mode(s, layer, mode)
}

/// Changes the mode; the cache is rebuilt (manual overrides are dropped when the grid geometry changes).
pub fn set_mode(s: &mut Sprite, layer: usize, mode: Mode) -> bool {
    if mode == Mode::Corners && is_iso_layer(s, layer) {
        return false;
    }
    let Some((mut at, _)) = payload(s, layer) else { return false };
    if at.mode != mode {
        if (at.mode == Mode::Corners) != (mode == Mode::Corners) {
            at.overrides.clear();
        }
        at.mode = mode;
        save(s, layer, &at);
        rebuild_all(s, layer);
    } else {
        save(s, layer, &at);
    }
    true
}

/// Adds a terrain with the lowest priority. Returns its id.
pub fn add_terrain(s: &mut Sprite, layer: usize, name: &str, color: [u8; 4]) -> Option<TerrainId> {
    if !is_tilemap(s, layer) {
        return None;
    }
    let mut at = load(s, layer).unwrap_or_default();
    if at.terrains.len() >= MAX_TERRAINS {
        return None;
    }
    let id = (1..=MAX_TERRAINS as u8).find(|i| at.terrain_by_id(*i).is_none())?;
    at.terrains.push(Terrain { id, name: name.to_string(), color });
    save(s, layer, &at);
    Some(id)
}

/// Removes a terrain (its cells become empty, tiles using it lose it). The last terrain stays.
pub fn remove_terrain(s: &mut Sprite, layer: usize, id: TerrainId) -> bool {
    let Some((mut at, _)) = payload(s, layer) else { return false };
    if at.terrains.len() <= 1 || at.terrain_by_id(id).is_none() {
        return false;
    }
    at.terrains.retain(|t| t.id != id);
    for g in at.intent.values_mut() {
        for c in &mut g.cells {
            if *c == Some(id) {
                *c = None;
            }
        }
    }
    at.tiles.retain(|_, m| m.terrain != Some(id));
    for m in at.tiles.values_mut() {
        for p in &mut m.peer {
            if *p == id {
                *p = 0;
            }
        }
    }
    save(s, layer, &at);
    rebuild_all(s, layer);
    true
}

/// Sets a terrain's name and color.
pub fn edit_terrain(s: &mut Sprite, layer: usize, id: TerrainId, name: &str, color: [u8; 4]) -> bool {
    let Some((mut at, _)) = payload(s, layer) else { return false };
    let Some(t) = at.terrains.iter_mut().find(|t| t.id == id) else { return false };
    t.name = name.to_string();
    t.color = color;
    save(s, layer, &at);
    true
}

/// Moves a terrain to a new priority position (0 = highest) and re-resolves.
pub fn move_terrain(s: &mut Sprite, layer: usize, id: TerrainId, to: usize) -> bool {
    let Some((mut at, _)) = payload(s, layer) else { return false };
    let Some(from) = at.terrains.iter().position(|t| t.id == id) else { return false };
    let t = at.terrains.remove(from);
    at.terrains.insert(to.min(at.terrains.len()), t);
    save(s, layer, &at);
    rebuild_all(s, layer);
    true
}

/// Sets the intent of many world cells at once and re-resolves only the display cells that
/// depend on them (3x3 around each, 2x2 for dual-grid). Returns the display cells re-resolved.
pub fn paint_cells(
    s: &mut Sprite,
    layer: usize,
    frame: usize,
    cells: &[(i32, i32)],
    t: Option<TerrainId>,
) -> Vec<(i32, i32)> {
    let Some((mut at, c)) = payload(s, layer) else { return Vec::new() };
    if frame >= s.frame_count() || t.is_some_and(|t| at.terrain_by_id(t).is_none()) {
        return Vec::new();
    }
    let kf = key_frame(s, layer, frame);
    ensure_grids(&mut at, kf, &c);
    let mut touched: Vec<(i32, i32)> = Vec::new();
    for &(x, y) in cells {
        if x < 0 || y < 0 || x as u32 >= c.dims.0 || y as u32 >= c.dims.1 || at.intent_at(kf, x, y) == t {
            continue;
        }
        at.intent.get_mut(&kf).unwrap().set(x, y, t);
        touched.extend(affected(at.mode, x, y));
    }
    if touched.is_empty() {
        return touched;
    }
    touched.sort_unstable();
    touched.dedup();
    resolve_cells(s, layer, frame, &at, &c, &touched, true);
    save(s, layer, &at);
    touched
}

/// Paints (or erases with `None`) a single world cell.
pub fn paint_terrain(
    s: &mut Sprite,
    layer: usize,
    frame: usize,
    x: i32,
    y: i32,
    t: Option<TerrainId>,
) -> Vec<(i32, i32)> {
    paint_cells(s, layer, frame, &[(x, y)], t)
}

/// Mirrors cells across the map center (flags: 1 = horizontally, 2 = vertically).
pub fn mirror_cells(cells: &[(i32, i32)], flags: u32, dims: (u32, u32)) -> Vec<(i32, i32)> {
    let mut out = cells.to_vec();
    if flags & 1 != 0 {
        out.extend(cells.iter().map(|&(x, y)| (dims.0 as i32 - 1 - x, y)));
    }
    if flags & 2 != 0 {
        let more: Vec<(i32, i32)> = out.iter().map(|&(x, y)| (x, dims.1 as i32 - 1 - y)).collect();
        out.extend(more);
    }
    out
}

/// Cells of a square or round brush of `size` cells centred on (x, y).
pub fn brush_cells(x: i32, y: i32, size: u32, round: bool) -> Vec<(i32, i32)> {
    let size = size.clamp(1, 64) as i32;
    let lo = -(size - 1) / 2;
    let mut v = Vec::new();
    for dy in 0..size {
        for dx in 0..size {
            if round && size > 2 {
                let (qx, qy) = (2 * dx - (size - 1), 2 * dy - (size - 1));
                if qx * qx + qy * qy > size * size {
                    continue;
                }
            }
            v.push((x + lo + dx, y + lo + dy));
        }
    }
    v
}

/// Cells of the rectangle between two corners (inclusive).
pub fn rect_cells(x0: i32, y0: i32, x1: i32, y1: i32) -> Vec<(i32, i32)> {
    let (xa, xb, ya, yb) = (x0.min(x1), x0.max(x1), y0.min(y1), y0.max(y1));
    (ya..=yb).flat_map(|y| (xa..=xb).map(move |x| (x, y))).collect()
}

/// Cells of the 4-connected region around (x, y) that share its intent.
pub fn flood_region(s: &Sprite, layer: usize, frame: usize, x: i32, y: i32) -> Vec<(i32, i32)> {
    let Some((at, c)) = payload(s, layer) else { return Vec::new() };
    let kf = key_frame(s, layer, frame);
    if x < 0 || y < 0 || x as u32 >= c.dims.0 || y as u32 >= c.dims.1 {
        return Vec::new();
    }
    let target = at.intent_at(kf, x, y);
    let mut seen = vec![false; c.dims.0 as usize * c.dims.1 as usize];
    let mut out = Vec::new();
    let mut stack = vec![(x, y)];
    while let Some((cx, cy)) = stack.pop() {
        if cx < 0 || cy < 0 || cx as u32 >= c.dims.0 || cy as u32 >= c.dims.1 {
            continue;
        }
        let i = cy as usize * c.dims.0 as usize + cx as usize;
        if seen[i] || at.intent_at(kf, cx, cy) != target {
            continue;
        }
        seen[i] = true;
        out.push((cx, cy));
        stack.extend([(cx + 1, cy), (cx - 1, cy), (cx, cy + 1), (cx, cy - 1)]);
    }
    out
}

/// The painted terrain at a world cell (eyedropper).
pub fn terrain_at(s: &Sprite, layer: usize, frame: usize, x: i32, y: i32) -> Option<TerrainId> {
    let at = load(s, layer)?;
    at.intent_at(key_frame(s, layer, frame), x, y)
}

/// Sets (or with `None` clears) a manual tile on a display cell. Clearing restores the auto result.
pub fn set_override(s: &mut Sprite, layer: usize, frame: usize, x: i32, y: i32, v: Option<TileRef>) -> bool {
    let Some((mut at, c)) = payload(s, layer) else { return false };
    let (gw, gh) = c.grid_dims();
    if frame >= s.frame_count() || x < 0 || y < 0 || x as u32 >= gw || y as u32 >= gh {
        return false;
    }
    let kf = key_frame(s, layer, frame);
    ensure_grids(&mut at, kf, &c);
    let g = at.overrides.entry(kf).or_insert_with(|| Grid::new(gw, gh));
    g.set(x, y, v);
    resolve_cells(s, layer, frame, &at, &c, &[(x, y)], false);
    save(s, layer, &at);
    true
}

/// Clears every manual override of a frame and restores the auto result.
pub fn clear_overrides(s: &mut Sprite, layer: usize, frame: usize) {
    let Some((mut at, _)) = payload(s, layer) else { return };
    at.overrides.remove(&key_frame(s, layer, frame));
    save(s, layer, &at);
    rebuild(s, layer, frame);
}

/// Hook for the manual stamp tool: a stamped tile becomes an override, erasing it restores auto.
pub fn note_stamp(s: &mut Sprite, layer: usize, frame: usize, px: i32, py: i32, value: u32) {
    if load(s, layer).is_none() {
        return;
    }
    let Some((at, c)) = payload(s, layer) else { return };
    let kf = key_frame(s, layer, frame);
    if !at.intent.contains_key(&kf) && !at.overrides.contains_key(&kf) {
        return;
    }
    let (x, y) = c.display_cell(px, py);
    set_override(s, layer, frame, x, y, (value & TILE_ID_MASK != 0).then_some(value));
}

/// Like [`note_stamp`] for a tile stamped by cell (i, j) on an isometric layer.
pub fn note_stamp_cell(s: &mut Sprite, layer: usize, frame: usize, i: i64, j: i64, value: u32) {
    if load(s, layer).is_none() || !is_iso_layer(s, layer) {
        return;
    }
    let Some((at, _)) = payload(s, layer) else { return };
    let kf = key_frame(s, layer, frame);
    if !at.intent.contains_key(&kf) && !at.overrides.contains_key(&kf) {
        return;
    }
    let (Ok(x), Ok(y)) = (i32::try_from(i), i32::try_from(j)) else { return };
    set_override(s, layer, frame, x, y, (value & TILE_ID_MASK != 0).then_some(value));
}

/// Rebuilds the tile cache of a frame from intent and overrides. Frames without any intent
/// (never painted with terrain) are left alone.
pub fn rebuild(s: &mut Sprite, layer: usize, frame: usize) {
    let Some((at, c)) = payload(s, layer) else { return };
    let kf = key_frame(s, layer, frame);
    if !at.intent.contains_key(&kf) && !at.overrides.contains_key(&kf) {
        return;
    }
    let all: Vec<(i32, i32, u32)> =
        resolve_all_cells(&c).into_iter().map(|(x, y)| (x, y, resolved(&at, kf, x, y, &c, true))).collect();
    apply(s, layer, frame, &at, &c, &all);
}

pub fn rebuild_all(s: &mut Sprite, layer: usize) {
    for f in 0..s.frame_count() {
        rebuild(s, layer, f);
    }
}

/// Display cells (cell coordinates) that use a fallback tile because nothing matches exactly.
pub fn missing_cells(s: &Sprite, layer: usize, frame: usize) -> Vec<(i32, i32)> {
    let Some((at, c)) = payload(s, layer) else { return Vec::new() };
    let kf = key_frame(s, layer, frame);
    resolve_all_cells(&c).into_iter().filter(|&(x, y)| at.resolve(kf, x, y, c.dims, c.square).1).collect()
}

/// Human-readable list of what the tileset lacks: per terrain the masks without a tile, and
/// the terrain combinations the map asks for that no tile provides.
pub fn missing_report(s: &Sprite, layer: usize, frame: usize) -> Vec<String> {
    let Some((at, c)) = payload(s, layer) else { return Vec::new() };
    let mut out = Vec::new();
    for t in &at.terrains {
        let mut lacking = Vec::new();
        for m in at.mode.masks() {
            if at.mode == Mode::Corners && m == 0 {
                continue;
            }
            let mut want = [0u8; 8];
            for &d in at.mode.dirs() {
                if m & at.mode.bit(d).unwrap() != 0 {
                    want[d] = t.id;
                }
            }
            let center = (at.mode != Mode::Corners).then_some(t.id);
            if at.exact_candidates(center, &want, c.square).is_empty() {
                lacking.push(m.to_string());
            }
        }
        if !lacking.is_empty() {
            out.push(format!("{}: missing masks {}", t.name, lacking.join(", ")));
        }
    }
    let kf = key_frame(s, layer, frame);
    let mut seen = std::collections::BTreeSet::new();
    for (x, y) in missing_cells(s, layer, frame) {
        if let Some((cen, want)) = at.wanted(kf, x, y, c.dims) {
            let name = |id: u8| at.terrain_by_id(id).map_or("?".to_string(), |t| t.name.clone());
            let names = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
            let parts: Vec<String> = at
                .mode
                .dirs()
                .iter()
                .filter(|&&d| want[d] != 0)
                .map(|&d| format!("{}={}", names[d], name(want[d])))
                .collect();
            seen.insert(format!("{} needs {}", cen.map_or("corners".into(), name), parts.join(" ")));
        }
    }
    out.extend(seen.into_iter().take(50));
    out
}

fn edit_tile(s: &mut Sprite, layer: usize, tile: u32, f: impl FnOnce(&mut Autotile, &mut TileMeta)) -> bool {
    let Some((mut at, c)) = payload(s, layer) else { return false };
    if tile == 0 || tile as usize >= s.tilesets[c.ts].tiles.len() {
        return false;
    }
    let mut m = at.tiles.get(&tile).copied().unwrap_or(TileMeta { terrain: None, peer: [0; 8], weight: 1.0, sym: 0 });
    f(&mut at, &mut m);
    at.tiles.insert(tile, m);
    save(s, layer, &at);
    rebuild_all(s, layer);
    true
}

/// Gives a tile a single-terrain mask in the layer's mode (center terrain = the first terrain
/// unless the tile already has one); `None` removes the tile's auto-tile data.
pub fn set_tile_mask(s: &mut Sprite, layer: usize, tile: u32, mask: Option<u8>) -> bool {
    let Some((mut at, c)) = payload(s, layer) else { return false };
    if tile == 0 || tile as usize >= s.tilesets[c.ts].tiles.len() {
        return false;
    }
    let Some(mask) = mask else {
        at.tiles.remove(&tile);
        save(s, layer, &at);
        rebuild_all(s, layer);
        return true;
    };
    edit_tile(s, layer, tile, |at, m| {
        let t = m.terrain.or(m.peer.iter().copied().find(|&p| p != 0)).unwrap_or(at.terrains[0].id);
        if at.mode != Mode::Corners {
            m.terrain = Some(t);
        }
        for &d in at.mode.dirs() {
            m.peer[d] = if mask & at.mode.bit(d).unwrap() != 0 { t } else { 0 };
        }
    })
}

pub fn set_tile_weight(s: &mut Sprite, layer: usize, tile: u32, weight: f32) -> bool {
    if !weight.is_finite() || weight < 0.0 {
        return false;
    }
    edit_tile(s, layer, tile, |_, m| m.weight = weight)
}

/// Sets the tile's center terrain (0 clears it).
pub fn set_tile_terrain(s: &mut Sprite, layer: usize, tile: u32, t: u8) -> bool {
    edit_tile(s, layer, tile, |at, m| m.terrain = (t != 0 && at.terrain_by_id(t).is_some()).then_some(t))
}

/// Sets the terrain id on one ring direction of a tile (0 = empty).
pub fn set_tile_peer(s: &mut Sprite, layer: usize, tile: u32, dir: usize, t: u8) -> bool {
    if dir >= 8 {
        return false;
    }
    edit_tile(s, layer, tile, |at, m| {
        if t == 0 || at.terrain_by_id(t).is_some() {
            m.peer[dir] = t;
        }
        if m.terrain.is_none() && at.mode != Mode::Corners {
            m.terrain = Some(at.terrains[0].id);
        }
    })
}

/// Sets which transforms a tile may be reused with (bit 0 flip X, bit 1 flip Y, bit 2 diagonal).
pub fn set_tile_sym(s: &mut Sprite, layer: usize, tile: u32, sym: u8) -> bool {
    edit_tile(s, layer, tile, |_, m| m.sym = sym & 7)
}

pub fn set_seed(s: &mut Sprite, layer: usize, seed: u32) -> bool {
    let Some((mut at, _)) = payload(s, layer) else { return false };
    at.seed = seed;
    save(s, layer, &at);
    rebuild_all(s, layer);
    true
}

pub fn set_edge_same(s: &mut Sprite, layer: usize, same: bool) -> bool {
    let Some((mut at, _)) = payload(s, layer) else { return false };
    at.edge_same = same;
    save(s, layer, &at);
    rebuild_all(s, layer);
    true
}

// MARK: Exporter API (Godot TileSet)

/// Per tile of a tileset: the single-terrain mask (in the mode's bit layout) of tiles that have
/// terrain data, from the first tilemap layer using the tileset. Index = tile index.
pub fn tile_masks(sprite: &Sprite, tileset_index: usize) -> Vec<Option<u8>> {
    let n = sprite.tilesets.get(tileset_index).map_or(0, |t| t.tiles.len());
    let mut out = vec![None; n];
    if let Some(at) = tileset_autotile(sprite, tileset_index) {
        for (&i, m) in &at.tiles {
            if (i as usize) < n {
                out[i as usize] = Some(m.mask(at.mode));
            }
        }
    }
    out
}

/// Peering data of one tile: center terrain and the terrain id per ring direction
/// (N, NE, E, SE, S, SW, W, NW; `None` = empty).
#[derive(Clone, Debug, PartialEq)]
pub struct TilePeering {
    pub terrain: Option<TerrainId>,
    pub peers: [Option<TerrainId>; 8],
    pub weight: f32,
    /// Transform bits this tile may be reused with (1 flip X, 2 flip Y, 4 diagonal).
    pub sym: u8,
}

pub fn tile_peering(sprite: &Sprite, tileset_index: usize, tile: u32) -> Option<TilePeering> {
    let at = tileset_autotile(sprite, tileset_index)?;
    let m = at.tiles.get(&tile)?;
    Some(TilePeering { terrain: m.terrain, peers: m.peers(), weight: m.weight, sym: m.sym })
}

/// The auto-tile payload of the first layer using a tileset: mode, terrains (name, color, in
/// priority order), seed, tile metadata.
pub fn tileset_autotile(sprite: &Sprite, tileset_index: usize) -> Option<Autotile> {
    sprite
        .layers
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind == KIND_TILEMAP && l.tileset as usize == tileset_index)
        .find_map(|(i, _)| load(sprite, i))
}

/// One text line per auto-tile layer for `pixmith-cli info`.
pub fn info_text(s: &Sprite) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for (i, l) in s.layers.iter().enumerate() {
        let Some(at) = load(s, i) else { continue };
        let painted: usize = at.intent.values().map(|g| g.cells.iter().flatten().count()).sum();
        let ov: usize = at.overrides.values().map(|g| g.cells.iter().flatten().count()).sum();
        let names: Vec<String> = at.terrains.iter().map(|t| t.name.clone()).collect();
        let _ = writeln!(
            out,
            "  autotile '{}': mode {}, terrains [{}], {} tiles with masks, {} painted cells, {} overrides, {} fallback cells",
            l.name,
            at.mode.name(),
            names.join(", "),
            at.tiles.len(),
            painted,
            ov,
            (0..s.frame_count()).map(|f| missing_cells(s, i, f).len()).sum::<usize>()
        );
    }
    out
}

// MARK: Templates

/// A mask as it looks when its tile is drawn with transform `t`.
pub fn transform_mask(mode: Mode, m: u8, t: u8) -> u8 {
    let mut out = 0u8;
    for &d in mode.dirs() {
        if m & mode.bit(d).unwrap() != 0 {
            out |= mode.bit(transform_dir(d, t)).unwrap();
        }
    }
    out
}

/// One representative (the smallest mask) per class of masks that are rotations/mirrors of each other.
pub fn reduced_masks(mode: Mode) -> Vec<u8> {
    mode.masks().into_iter().filter(|&m| (0..8u8).map(|t| transform_mask(mode, m, t)).min() == Some(m)).collect()
}

/// Assigns masks to tiles 1.. for the first terrain. A tileset of exactly `reduced_masks` tiles
/// gets the reduced layout (tiles reusable by flipping/rotating, `sym` = 7); otherwise the full
/// ascending layout (47 blob / 16 sides or corners). Returns how many tiles were assigned.
pub fn assign_masks_from_template(s: &mut Sprite, layer: usize) -> usize {
    let Some((mut at, c)) = payload(s, layer) else { return 0 };
    let id = at.terrains[0].id;
    let avail = s.tilesets[c.ts].tiles.len().saturating_sub(1);
    let reduced = reduced_masks(at.mode);
    // The reduced layouts rely on rotating tiles, which isometric tiles cannot do.
    let (masks, sym) = if c.iso.is_none() && avail == reduced.len() && avail != at.mode.masks().len() {
        (reduced, if c.square { 7u8 } else { 3 })
    } else {
        (at.mode.masks(), 0)
    };
    let n = masks.len().min(avail);
    for (k, m) in masks.iter().take(n).enumerate() {
        let mut meta = TileMeta { terrain: (at.mode != Mode::Corners).then_some(id), peer: [0; 8], weight: 1.0, sym };
        for &d in at.mode.dirs() {
            if m & at.mode.bit(d).unwrap() != 0 {
                meta.peer[d] = id;
            }
        }
        at.tiles.insert(k as u32 + 1, meta);
    }
    save(s, layer, &at);
    rebuild_all(s, layer);
    n
}

const DIGITS: [[u8; 5]; 10] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
];

/// Draws a template tile: border, dotted guide cross, filled marks for the set neighbours, and the mask number.
fn draw_template(px: &mut [u8], tw: usize, th: usize, mode: Mode, mask: u8, color: [u8; 4]) {
    let mut put = |x: usize, y: usize, c: [u8; 4]| {
        if x < tw && y < th {
            px[(y * tw + x) * 4..][..4].copy_from_slice(&c);
        }
    };
    let guide = [140, 140, 140, 255];
    let fill = [color[0], color[1], color[2], 150];
    let (qw, qh) = ((tw / 3).max(1), (th / 3).max(1));
    for &d in mode.dirs() {
        if mask & mode.bit(d).unwrap() == 0 {
            continue;
        }
        let (dx, dy) = DIRS[d];
        let (x0, x1) = match dx {
            -1 => (0, qw),
            0 => (qw, tw.saturating_sub(qw).max(qw)),
            _ => (tw.saturating_sub(qw), tw),
        };
        let (y0, y1) = match dy {
            -1 => (0, qh),
            0 => (qh, th.saturating_sub(qh).max(qh)),
            _ => (th.saturating_sub(qh), th),
        };
        for y in y0..y1 {
            for x in x0..x1 {
                put(x, y, fill);
            }
        }
    }
    for i in 0..tw {
        put(i, 0, guide);
        put(i, th - 1, guide);
        if i % 2 == 0 {
            put(i, th / 2, guide);
        }
    }
    for j in 0..th {
        put(0, j, guide);
        put(tw - 1, j, guide);
        if j % 2 == 0 {
            put(tw / 2, j, guide);
        }
    }
    for (k, ch) in mask.to_string().bytes().enumerate() {
        for (r, row) in DIGITS[(ch - b'0') as usize].iter().enumerate() {
            for b in 0..3 {
                if row >> (2 - b) & 1 == 1 {
                    put(2 + k * 4 + b, 2 + r, [255, 255, 255, 255]);
                }
            }
        }
    }
}

/// Draws an isometric template tile: the diamond outline at the tile's pivot, a filled mark where a mask bit is
/// set (on the matching edge for sides, on the matching corner for corners) and the mask number.
fn draw_template_iso(
    px: &mut [u8],
    (tw, th): (usize, usize),
    (cw, ch): (usize, usize),
    pivot: (i32, i32),
    mode: Mode,
    mask: u8,
    color: [u8; 4],
) {
    let mut put = |x: i64, y: i64, c: [u8; 4]| {
        if x >= 0 && y >= 0 && (x as usize) < tw && (y as usize) < th {
            px[(y as usize * tw + x as usize) * 4..][..4].copy_from_slice(&c);
        }
    };
    let (cx, cy) = (pivot.0 as i64, pivot.1 as i64);
    let (a, b) = (cw as i64 / 2, ch as i64 / 2);
    let guide = [140, 140, 140, 255];
    let fill = [color[0], color[1], color[2], 255];
    // Outline: walk the four edges.
    let corners = [(cx, cy - b), (cx + a, cy), (cx, cy + b), (cx - a, cy)];
    for k in 0..4 {
        let ((x0, y0), (x1, y1)) = (corners[k], corners[(k + 1) % 4]);
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
        for t in 0..=steps {
            put(x0 + (x1 - x0) * t / steps, y0 + (y1 - y0) * t / steps, guide);
        }
    }
    // Marks, by ring direction N, NE, E, SE, S, SW, W, NW (see `DIRS`) in screen terms: N is the upper right edge,
    // NE the right corner, E the lower right edge, SE the bottom corner, and so on around.
    let spots: [(i64, i64); 8] = [
        (cx + a / 2, cy - b / 2),
        (cx + a - 1, cy),
        (cx + a / 2, cy + b / 2),
        (cx, cy + b - 1),
        (cx - a / 2, cy + b / 2),
        (cx - a + 1, cy),
        (cx - a / 2, cy - b / 2),
        (cx, cy - b + 1),
    ];
    for &d in mode.dirs() {
        if mask & mode.bit(d).unwrap() == 0 {
            continue;
        }
        let (sx, sy) = spots[d];
        for dy in -1..=1 {
            for dx in -1..=1 {
                put(sx + dx, sy + dy, fill);
            }
        }
    }
    for (k, ch) in mask.to_string().bytes().enumerate() {
        for (r, row) in DIGITS[(ch - b'0') as usize].iter().enumerate() {
            for bit in 0..3 {
                if row >> (2 - bit) & 1 == 1 {
                    put(cx - 4 + (k * 4 + bit) as i64, cy - 2 + r as i64, [255, 255, 255, 255]);
                }
            }
        }
    }
}

/// Appends one labelled, guide-lined template tile per mask of the layer's mode (or only the
/// rotation/flip representatives when `reduced`) and assigns the masks. Returns tiles added.
pub fn generate_template(s: &mut Sprite, layer: usize, reduced: bool) -> usize {
    let Some((mut at, c)) = payload(s, layer) else { return 0 };
    let reduced = reduced && c.iso.is_none();
    let masks = if reduced { reduced_masks(at.mode) } else { at.mode.masks() };
    let (tw, th) = (c.tw as usize, c.th as usize);
    let first = s.tilesets[c.ts].tiles.len();
    let t0 = at.terrains[0].clone();
    for m in &masks {
        let id = s.tilesets[c.ts].add_blank() as usize;
        let cell = s.tilesets[c.ts].iso.as_ref().map(|i| (i.cell_w as usize, i.cell_h as usize));
        let pivot = s.tilesets[c.ts].pivot(id as u32);
        let t = std::sync::Arc::make_mut(&mut s.tilesets[c.ts].tiles[id]);
        match cell {
            Some(cell) => draw_template_iso(t, (tw, th), cell, pivot, at.mode, *m, t0.color),
            None => draw_template(t, tw, th, at.mode, *m, t0.color),
        }
    }
    for (k, m) in masks.iter().enumerate() {
        let mut meta = TileMeta {
            terrain: (at.mode != Mode::Corners).then_some(t0.id),
            peer: [0; 8],
            weight: 1.0,
            sym: if reduced {
                if c.square {
                    7
                } else {
                    3
                }
            } else {
                0
            },
        };
        for &d in at.mode.dirs() {
            if m & at.mode.bit(d).unwrap() != 0 {
                meta.peer[d] = t0.id;
            }
        }
        at.tiles.insert((first + k) as u32, meta);
    }
    save(s, layer, &at);
    rebuild_all(s, layer);
    masks.len()
}

// MARK: C ABI

unsafe fn ed<'a>(d: *mut Editor) -> Option<&'a mut Editor> {
    d.as_mut()
}

unsafe fn sp<'a>(d: *const Editor) -> Option<&'a Sprite> {
    d.as_ref().map(|e| &e.sprite)
}

fn cell_of_px(s: &Sprite, layer: usize, px: i32, py: i32) -> Option<(i32, i32)> {
    let c = ctx_for(s, layer, Mode::Blob)?;
    if c.iso.is_some() {
        return Some(c.display_cell(px, py));
    }
    (px >= 0 && py >= 0).then(|| (px / c.tw as i32, py / c.th as i32))
}

fn dims_of(s: &Sprite, layer: usize) -> (u32, u32) {
    ctx_for(s, layer, Mode::Blob).map_or((0, 0), |c| c.dims)
}

fn cstring(s: String) -> *mut c_char {
    CString::new(s.replace('\0', "")).map_or(null_mut(), CString::into_raw)
}

unsafe fn name_arg(name: *const c_char) -> String {
    if name.is_null() {
        "Terrain".to_string()
    } else {
        CStr::from_ptr(name).to_string_lossy().into_owned()
    }
}

/// Defines the layer's first terrain and the mode (undoable). `rgba` is 0xRRGGBBAA;
/// `mode`: 0 sides, 1 blob, 2 corners.
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_set_terrain(
    doc: *mut Editor,
    layer: u32,
    name: *const c_char,
    rgba: u32,
    mode: i32,
) -> bool {
    crate::ffi::guarded("pixmith_autotile_set_terrain", doc, || {
        let Some(e) = ed(doc) else { return false };
        let Some(m) = Mode::from_i32(mode) else { return false };
        let name = name_arg(name);
        if !is_tilemap(&e.sprite, layer as usize) {
            return false;
        }
        e.checkpoint();
        set_terrain(&mut e.sprite, layer as usize, &name, rgba.to_be_bytes(), m)
    })
}

/// Changes the mode of a layer that already has terrain data (undoable).
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_set_mode(doc: *mut Editor, layer: u32, mode: i32) -> bool {
    crate::ffi::guarded("pixmith_autotile_set_mode", doc, || {
        let Some(e) = ed(doc) else { return false };
        let Some(m) = Mode::from_i32(mode) else { return false };
        if load(&e.sprite, layer as usize).is_none() {
            return false;
        }
        e.checkpoint();
        set_mode(&mut e.sprite, layer as usize, m)
    })
}

/// Adds a terrain (lowest priority, undoable). Returns its id or 0.
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_add_terrain(
    doc: *mut Editor,
    layer: u32,
    name: *const c_char,
    rgba: u32,
) -> u32 {
    crate::ffi::guarded("pixmith_autotile_add_terrain", doc, || {
        let Some(e) = ed(doc) else { return 0 };
        let name = name_arg(name);
        if load(&e.sprite, layer as usize).is_none() {
            return 0;
        }
        e.checkpoint();
        add_terrain(&mut e.sprite, layer as usize, &name, rgba.to_be_bytes()).map_or(0, u32::from)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_remove_terrain(doc: *mut Editor, layer: u32, id: u32) -> bool {
    crate::ffi::guarded("pixmith_autotile_remove_terrain", doc, || {
        let Some(e) = ed(doc) else { return false };
        e.checkpoint();
        remove_terrain(&mut e.sprite, layer as usize, id as u8)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_edit_terrain(
    doc: *mut Editor,
    layer: u32,
    id: u32,
    name: *const c_char,
    rgba: u32,
) -> bool {
    crate::ffi::guarded("pixmith_autotile_edit_terrain", doc, || {
        let Some(e) = ed(doc) else { return false };
        let name = name_arg(name);
        e.checkpoint();
        edit_terrain(&mut e.sprite, layer as usize, id as u8, &name, rgba.to_be_bytes())
    })
}

/// Moves a terrain to priority position `to` (0 = highest, undoable).
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_move_terrain(doc: *mut Editor, layer: u32, id: u32, to: u32) -> bool {
    crate::ffi::guarded("pixmith_autotile_move_terrain", doc, || {
        let Some(e) = ed(doc) else { return false };
        e.checkpoint();
        move_terrain(&mut e.sprite, layer as usize, id as u8, to as usize)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_terrain_count(doc: *const Editor, layer: u32) -> u32 {
    crate::ffi::guarded("pixmith_autotile_terrain_count", std::ptr::null_mut(), || {
        sp(doc).and_then(|s| load(s, layer as usize)).map_or(0, |a| a.terrains.len() as u32)
    })
}

/// Terrain id at priority position `index` (0 if out of range).
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_terrain_id(doc: *const Editor, layer: u32, index: u32) -> u32 {
    crate::ffi::guarded("pixmith_autotile_terrain_id", std::ptr::null_mut(), || {
        sp(doc)
            .and_then(|s| load(s, layer as usize))
            .and_then(|a| a.terrains.get(index as usize).map(|t| t.id as u32))
            .unwrap_or(0)
    })
}

/// Terrain name; release with `pixmith_string_free`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_terrain_name(doc: *const Editor, layer: u32, id: u32) -> *mut c_char {
    crate::ffi::guarded("pixmith_autotile_terrain_name", std::ptr::null_mut(), || {
        sp(doc)
            .and_then(|s| load(s, layer as usize))
            .and_then(|a| a.terrain_by_id(id as u8).map(|t| cstring(t.name.clone())))
            .unwrap_or(null_mut())
    })
}

/// Terrain color as 0xRRGGBBAA (0 if unknown).
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_terrain_color(doc: *const Editor, layer: u32, id: u32) -> u32 {
    crate::ffi::guarded("pixmith_autotile_terrain_color", std::ptr::null_mut(), || {
        sp(doc)
            .and_then(|s| load(s, layer as usize))
            .and_then(|a| a.terrain_by_id(id as u8).map(|t| u32::from_be_bytes(t.color)))
            .unwrap_or(0)
    })
}

/// Paints terrain with a square (or round) brush of `size` cells at canvas pixel (x, y);
/// `terrain` 0 erases; `mirror` bit 0 = horizontal, bit 1 = vertical symmetry. Call
/// `pixmith_begin_stroke` first (one undo step per stroke). Returns whether anything changed.
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_paint_brush(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    terrain: u32,
    size: u32,
    round: bool,
    mirror: u32,
) -> bool {
    crate::ffi::guarded("pixmith_autotile_paint_brush", doc, || {
        let Some(e) = ed(doc) else { return false };
        let Some((cx, cy)) = cell_of_px(&e.sprite, layer as usize, x, y) else { return false };
        let cells = mirror_cells(&brush_cells(cx, cy, size, round), mirror, dims_of(&e.sprite, layer as usize));
        !paint_cells(&mut e.sprite, layer as usize, frame as usize, &cells, (terrain != 0).then_some(terrain as u8))
            .is_empty()
    })
}

/// Single-cell paint (see `pixmith_autotile_paint_brush`).
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_paint(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    terrain: u32,
) -> bool {
    crate::ffi::guarded("pixmith_autotile_paint", doc, || {
        pixmith_autotile_paint_brush(doc, layer, frame, x, y, terrain, 1, false, 0)
    })
}

/// Fills the rectangle between two canvas pixels with terrain (0 erases). Call `pixmith_begin_stroke` first.
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_rect(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    terrain: u32,
    mirror: u32,
) -> bool {
    crate::ffi::guarded("pixmith_autotile_rect", doc, || {
        let Some(e) = ed(doc) else { return false };
        let dims = dims_of(&e.sprite, layer as usize);
        let (Some(a), Some(b)) = (
            cell_of_px(&e.sprite, layer as usize, x0.max(0), y0.max(0)),
            cell_of_px(&e.sprite, layer as usize, x1.max(0), y1.max(0)),
        ) else {
            return false;
        };
        let clamp = |c: (i32, i32)| (c.0.min(dims.0 as i32 - 1), c.1.min(dims.1 as i32 - 1));
        let (a, b) = (clamp(a), clamp(b));
        let cells = mirror_cells(&rect_cells(a.0, a.1, b.0, b.1), mirror, dims);
        !paint_cells(&mut e.sprite, layer as usize, frame as usize, &cells, (terrain != 0).then_some(terrain as u8))
            .is_empty()
    })
}

/// Flood-fills the region under a canvas pixel with terrain (0 erases). Call `pixmith_begin_stroke` first.
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_flood(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    terrain: u32,
    mirror: u32,
) -> bool {
    crate::ffi::guarded("pixmith_autotile_flood", doc, || {
        let Some(e) = ed(doc) else { return false };
        let Some((cx, cy)) = cell_of_px(&e.sprite, layer as usize, x, y) else { return false };
        let dims = dims_of(&e.sprite, layer as usize);
        let t = (terrain != 0).then_some(terrain as u8);
        let mut changed = false;
        for seed in mirror_cells(&[(cx, cy)], mirror, dims) {
            let region = flood_region(&e.sprite, layer as usize, frame as usize, seed.0, seed.1);
            changed |= !paint_cells(&mut e.sprite, layer as usize, frame as usize, &region, t).is_empty();
        }
        changed
    })
}

/// The painted terrain id under a canvas pixel (0 = none); the eyedropper.
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_terrain_at(
    doc: *const Editor,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
) -> u32 {
    crate::ffi::guarded("pixmith_autotile_terrain_at", std::ptr::null_mut(), || {
        let Some(s) = sp(doc) else { return 0 };
        let Some((cx, cy)) = cell_of_px(s, layer as usize, x, y) else { return 0 };
        terrain_at(s, layer as usize, frame as usize, cx, cy).map_or(0, u32::from)
    })
}

/// Sets (`tile` != 0) or clears (`tile` == 0) a manual tile override at canvas pixel (x, y).
/// Call `pixmith_begin_stroke` first.
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_set_override(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    tile: u32,
) -> bool {
    crate::ffi::guarded("pixmith_autotile_set_override", doc, || {
        let Some(e) = ed(doc) else { return false };
        let Some(at) = load(&e.sprite, layer as usize) else { return false };
        let Some(c) = ctx_for(&e.sprite, layer as usize, at.mode) else { return false };
        let (cx, cy) = c.display_cell(x, y);
        let v = (tile & TILE_ID_MASK != 0).then_some(tile);
        set_override(&mut e.sprite, layer as usize, frame as usize, cx, cy, v)
    })
}

/// Removes all manual overrides of a frame (undoable).
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_clear_overrides(doc: *mut Editor, layer: u32, frame: u32) -> bool {
    crate::ffi::guarded("pixmith_autotile_clear_overrides", doc, || {
        let Some(e) = ed(doc) else { return false };
        if load(&e.sprite, layer as usize).is_none() {
            return false;
        }
        e.checkpoint();
        clear_overrides(&mut e.sprite, layer as usize, frame as usize);
        true
    })
}

/// Rebuilds the tile cache of every frame from the stored intent (undoable).
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_rebuild(doc: *mut Editor, layer: u32) -> bool {
    crate::ffi::guarded("pixmith_autotile_rebuild", doc, || {
        let Some(e) = ed(doc) else { return false };
        if load(&e.sprite, layer as usize).is_none() {
            return false;
        }
        e.checkpoint();
        rebuild_all(&mut e.sprite, layer as usize);
        true
    })
}

/// Gives a tile a single-terrain mask (`mask` 0..=255), or removes its data with a negative value (undoable).
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_set_tile_mask(doc: *mut Editor, layer: u32, tile: u32, mask: i32) -> bool {
    crate::ffi::guarded("pixmith_autotile_set_tile_mask", doc, || {
        let Some(e) = ed(doc) else { return false };
        e.checkpoint();
        set_tile_mask(&mut e.sprite, layer as usize, tile, (0..=255).contains(&mask).then_some(mask as u8))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_set_tile_weight(
    doc: *mut Editor,
    layer: u32,
    tile: u32,
    weight: f32,
) -> bool {
    crate::ffi::guarded("pixmith_autotile_set_tile_weight", doc, || {
        let Some(e) = ed(doc) else { return false };
        e.checkpoint();
        set_tile_weight(&mut e.sprite, layer as usize, tile, weight)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_set_tile_terrain(
    doc: *mut Editor,
    layer: u32,
    tile: u32,
    terrain: u32,
) -> bool {
    crate::ffi::guarded("pixmith_autotile_set_tile_terrain", doc, || {
        let Some(e) = ed(doc) else { return false };
        e.checkpoint();
        set_tile_terrain(&mut e.sprite, layer as usize, tile, terrain as u8)
    })
}

/// Sets the terrain id on ring direction `dir` (0 N, 1 NE, 2 E, 3 SE, 4 S, 5 SW, 6 W, 7 NW) of a tile.
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_set_tile_peer(
    doc: *mut Editor,
    layer: u32,
    tile: u32,
    dir: u32,
    terrain: u32,
) -> bool {
    crate::ffi::guarded("pixmith_autotile_set_tile_peer", doc, || {
        let Some(e) = ed(doc) else { return false };
        e.checkpoint();
        set_tile_peer(&mut e.sprite, layer as usize, tile, dir as usize, terrain as u8)
    })
}

/// Sets reuse transforms of a tile (bit 0 flip X, bit 1 flip Y, bit 2 diagonal).
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_set_tile_sym(doc: *mut Editor, layer: u32, tile: u32, sym: u32) -> bool {
    crate::ffi::guarded("pixmith_autotile_set_tile_sym", doc, || {
        let Some(e) = ed(doc) else { return false };
        e.checkpoint();
        set_tile_sym(&mut e.sprite, layer as usize, tile, sym as u8)
    })
}

/// A tile's mask in the layer's mode, or -1 when it has no terrain data.
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_tile_mask(doc: *const Editor, layer: u32, tile: u32) -> i32 {
    crate::ffi::guarded("pixmith_autotile_tile_mask", std::ptr::null_mut(), || {
        sp(doc)
            .and_then(|s| load(s, layer as usize))
            .and_then(|a| a.tiles.get(&tile).map(|m| m.mask(a.mode) as i32))
            .unwrap_or(-1)
    })
}

/// A tile's center terrain id (0 none, -1 no data).
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_tile_terrain(doc: *const Editor, layer: u32, tile: u32) -> i32 {
    crate::ffi::guarded("pixmith_autotile_tile_terrain", std::ptr::null_mut(), || {
        sp(doc)
            .and_then(|s| load(s, layer as usize))
            .and_then(|a| a.tiles.get(&tile).map(|m| m.terrain.unwrap_or(0) as i32))
            .unwrap_or(-1)
    })
}

/// Terrain id on a ring direction of a tile (0 empty, -1 no data).
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_tile_peer(doc: *const Editor, layer: u32, tile: u32, dir: u32) -> i32 {
    crate::ffi::guarded("pixmith_autotile_tile_peer", std::ptr::null_mut(), || {
        if dir >= 8 {
            return -1;
        }
        sp(doc)
            .and_then(|s| load(s, layer as usize))
            .and_then(|a| a.tiles.get(&tile).map(|m| m.peer[dir as usize] as i32))
            .unwrap_or(-1)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_tile_sym(doc: *const Editor, layer: u32, tile: u32) -> u32 {
    crate::ffi::guarded("pixmith_autotile_tile_sym", std::ptr::null_mut(), || {
        sp(doc)
            .and_then(|s| load(s, layer as usize))
            .and_then(|a| a.tiles.get(&tile).map(|m| m.sym as u32))
            .unwrap_or(0)
    })
}

/// A tile's variation weight (1 when unset).
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_tile_weight(doc: *const Editor, layer: u32, tile: u32) -> f32 {
    crate::ffi::guarded("pixmith_autotile_tile_weight", std::ptr::null_mut(), || {
        sp(doc).and_then(|s| load(s, layer as usize)).and_then(|a| a.tiles.get(&tile).map(|m| m.weight)).unwrap_or(1.0)
    })
}

/// The layer's mode (0 sides, 1 blob, 2 corners), or -1 when the layer has no terrain data.
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_mode(doc: *const Editor, layer: u32) -> i32 {
    crate::ffi::guarded("pixmith_autotile_mode", std::ptr::null_mut(), || {
        sp(doc).and_then(|s| load(s, layer as usize)).map_or(-1, |a| a.mode.to_i32())
    })
}

/// Treat cells outside the map as the same terrain (undoable).
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_set_edge_same(doc: *mut Editor, layer: u32, same: bool) -> bool {
    crate::ffi::guarded("pixmith_autotile_set_edge_same", doc, || {
        let Some(e) = ed(doc) else { return false };
        if load(&e.sprite, layer as usize).is_none() {
            return false;
        }
        e.checkpoint();
        set_edge_same(&mut e.sprite, layer as usize, same)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_edge_same(doc: *const Editor, layer: u32) -> bool {
    crate::ffi::guarded("pixmith_autotile_edge_same", std::ptr::null_mut(), || {
        sp(doc).and_then(|s| load(s, layer as usize)).is_some_and(|a| a.edge_same)
    })
}

/// Sets the variation seed of the layer (undoable).
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_set_seed(doc: *mut Editor, layer: u32, seed: u32) -> bool {
    crate::ffi::guarded("pixmith_autotile_set_seed", doc, || {
        let Some(e) = ed(doc) else { return false };
        if load(&e.sprite, layer as usize).is_none() {
            return false;
        }
        e.checkpoint();
        set_seed(&mut e.sprite, layer as usize, seed)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_seed(doc: *const Editor, layer: u32) -> u32 {
    crate::ffi::guarded("pixmith_autotile_seed", std::ptr::null_mut(), || {
        sp(doc).and_then(|s| load(s, layer as usize)).map_or(0, |a| a.seed)
    })
}

/// Writes the top-left canvas pixel (x, y pairs) of every cell using a fallback tile into `out`
/// (capacity `cap` i32 values). Returns the number of cells (may exceed what fit).
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_missing_cells(
    doc: *const Editor,
    layer: u32,
    frame: u32,
    out: *mut i32,
    cap: usize,
) -> usize {
    crate::ffi::guarded("pixmith_autotile_missing_cells", std::ptr::null_mut(), || {
        let Some(s) = sp(doc) else { return 0 };
        let Some(at) = load(s, layer as usize) else { return 0 };
        let Some(c) = ctx_for(s, layer as usize, at.mode) else { return 0 };
        let cells = missing_cells(s, layer as usize, frame as usize);
        if !out.is_null() {
            let (ox, oy) = c.origin();
            for (k, (x, y)) in cells.iter().enumerate() {
                if k * 2 + 1 < cap {
                    let (px, py) = match &c.iso {
                        Some(g) => {
                            let (a, b) = g.cell_origin((*x as i64, *y as i64));
                            (a as i32, b as i32)
                        }
                        None => (ox + x * c.tw as i32, oy + y * c.th as i32),
                    };
                    *out.add(k * 2) = px;
                    *out.add(k * 2 + 1) = py;
                }
            }
        }
        cells.len()
    })
}

/// Newline separated report of missing masks / terrain combinations; release with `pixmith_string_free`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_missing_report(doc: *const Editor, layer: u32, frame: u32) -> *mut c_char {
    crate::ffi::guarded("pixmith_autotile_missing_report", std::ptr::null_mut(), || {
        sp(doc).map_or(null_mut(), |s| cstring(missing_report(s, layer as usize, frame as usize).join("\n")))
    })
}

/// Assigns masks to tiles 1.. in template order (undoable). Returns the number assigned.
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_assign_masks_from_template(doc: *mut Editor, layer: u32) -> u32 {
    crate::ffi::guarded("pixmith_autotile_assign_masks_from_template", doc, || {
        let Some(e) = ed(doc) else { return 0 };
        if load(&e.sprite, layer as usize).is_none() {
            return 0;
        }
        e.checkpoint();
        assign_masks_from_template(&mut e.sprite, layer as usize) as u32
    })
}

/// Appends a labelled template tileset (undoable); `reduced` makes only the flip/rotation
/// representatives. Returns tiles added.
#[no_mangle]
pub unsafe extern "C" fn pixmith_autotile_generate_template(doc: *mut Editor, layer: u32, reduced: bool) -> u32 {
    crate::ffi::guarded("pixmith_autotile_generate_template", doc, || {
        let Some(e) = ed(doc) else { return 0 };
        if load(&e.sprite, layer as usize).is_none() {
            return 0;
        }
        e.checkpoint();
        generate_template(&mut e.sprite, layer as usize, reduced) as u32
    })
}

#[cfg(test)]
mod iso_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::tilemap::{add_tilemap_layer, render_grid, stamp, tile_at};
    use std::collections::HashSet;

    /// 32x32 sprite, 8x8 tiles (4x4 cells), full template tileset in the given mode.
    fn setup(mode: Mode) -> (Sprite, usize) {
        let mut s = Sprite::new(32, 32);
        let l = add_tilemap_layer(&mut s, 0, 8, 8);
        assert!(set_terrain(&mut s, l, "Grass", [1, 2, 3, 255], mode));
        generate_template(&mut s, l, false);
        (s, l)
    }

    /// The tile value in display cell (x, y) of frame 0.
    fn cell(s: &Sprite, l: usize, x: i32, y: i32) -> u32 {
        let g = s.cels[0][l].as_ref().and_then(|c| c.tiles.as_ref());
        g.and_then(|g| {
            (x >= 0 && y >= 0 && (x as u32) < g.w && (y as u32) < g.h)
                .then(|| g.tiles[y as usize * g.w as usize + x as usize])
        })
        .unwrap_or(0)
    }

    /// The mask a cell displays (the stored mask seen through the cell's flip flags).
    fn mask_of(s: &Sprite, l: usize, x: i32, y: i32) -> Option<u8> {
        let v = cell(s, l, x, y);
        let at = load(s, l).unwrap();
        let t = ((v & FLIP_X != 0) as u8) | ((v & FLIP_Y != 0) as u8) << 1 | ((v & FLIP_D != 0) as u8) << 2;
        at.tiles.get(&(v & TILE_ID_MASK)).map(|m| transform_mask(at.mode, m.mask(at.mode), t))
    }

    #[test]
    fn blob_has_47_unique() {
        let set: HashSet<u8> = (0..=255u8).map(reduce).collect();
        assert_eq!(set.len(), 47);
        assert_eq!(Mode::Blob.masks().len(), 47);
    }

    #[test]
    fn sides_table() {
        let (mut s, l) = setup(Mode::Sides);
        for (x, y) in [(1, 1), (1, 0), (2, 1), (1, 2), (0, 1)] {
            paint_terrain(&mut s, l, 0, x, y, Some(1));
        }
        assert_eq!(mask_of(&s, l, 1, 1), Some(15));
        assert_eq!(mask_of(&s, l, 1, 0), Some(4)); // S only
        assert_eq!(mask_of(&s, l, 2, 1), Some(8)); // W only
        assert_eq!(mask_of(&s, l, 1, 2), Some(1)); // N only
        assert_eq!(mask_of(&s, l, 0, 1), Some(2)); // E only
        assert!(missing_cells(&s, l, 0).is_empty());
        assert_eq!(Mode::Sides.masks().len(), 16);
    }

    #[test]
    fn blob_table_all_neighbourhoods() {
        // Every one of the 256 neighbourhoods resolves to the reduced mask.
        let (s, l) = setup(Mode::Blob);
        for m in 0..=255u8 {
            let mut s2 = s.clone();
            paint_terrain(&mut s2, l, 0, 1, 1, Some(1));
            for d in 0..8 {
                if m & (1 << d) != 0 {
                    paint_terrain(&mut s2, l, 0, 1 + DIRS[d].0, 1 + DIRS[d].1, Some(1));
                }
            }
            assert_eq!(mask_of(&s2, l, 1, 1), Some(reduce(m)), "mask {m}");
            assert!(!missing_cells(&s2, l, 0).contains(&(1, 1)));
        }
    }

    #[test]
    fn local_update_only_3x3_and_2x2() {
        let (mut s, l) = setup(Mode::Blob);
        assert!(stamp(&mut s, l, 0, 3 * 8, 3 * 8, 2)); // sentinel a full rebuild would clear
        let touched = paint_terrain(&mut s, l, 0, 0, 0, Some(1));
        assert_eq!(touched.len(), 9);
        assert_eq!(cell(&s, l, 3, 3), 2);
        let (mut s2, l2) = setup(Mode::Corners);
        let t = paint_terrain(&mut s2, l2, 0, 2, 2, Some(1));
        assert_eq!(t.len(), 4);
        assert!(t.iter().all(|&(x, y)| (2..=3).contains(&x) && (2..=3).contains(&y)));
    }

    #[test]
    fn plain_stamped_tiles_survive_painting_next_to_them() {
        for mode in [Mode::Sides, Mode::Blob] {
            let (mut s, l) = setup(mode);
            let plain: Vec<u32> = (0..9).map(|_| s.tilesets[0].add_blank()).collect();
            for (k, &t) in plain.iter().enumerate() {
                assert!(stamp(&mut s, l, 0, (k as i32 % 3) * 8, (k as i32 / 3) * 8, t));
            }
            // Paint the row under the block and one cell next to it, then erase them again.
            let row = [(0, 3), (1, 3), (2, 3), (3, 3), (3, 1)];
            for &(x, y) in &row {
                paint_terrain(&mut s, l, 0, x, y, Some(1));
            }
            for (k, &t) in plain.iter().enumerate() {
                assert_eq!(cell(&s, l, k as i32 % 3, k as i32 / 3), t, "{mode:?}: plain tile {k} survives painting");
            }
            paint_cells(&mut s, l, 0, &row, None);
            rebuild(&mut s, l, 0);
            for (k, &t) in plain.iter().enumerate() {
                assert_eq!(cell(&s, l, k as i32 % 3, k as i32 / 3), t, "{mode:?}: plain tile {k} survives erasing");
            }
            for &(x, y) in &row {
                assert_eq!(cell(&s, l, x, y), 0, "{mode:?}: erased terrain leaves no tile at ({x}, {y})");
            }
        }
    }

    #[test]
    fn dual_grid_offset_and_corners() {
        let (mut s, l) = setup(Mode::Corners);
        paint_terrain(&mut s, l, 0, 1, 1, Some(1));
        // Display cell (i,j) has world cells NW=(i-1,j-1) NE=(i,j-1) SE=(i,j) SW=(i-1,j) as corners.
        assert_eq!(mask_of(&s, l, 1, 1), Some(4)); // world (1,1) is its SE corner -> BR
        assert_eq!(mask_of(&s, l, 2, 1), Some(8)); // its SW corner
        assert_eq!(mask_of(&s, l, 1, 2), Some(2)); // its NE corner
        assert_eq!(mask_of(&s, l, 2, 2), Some(1)); // its NW corner
        assert_eq!(mask_of(&s, l, 0, 0), None);
        // The grid is offset by half a tile and one cell larger.
        let g = s.cels[0][l].as_ref().unwrap().tiles.as_ref().unwrap();
        assert_eq!((g.x, g.y, g.w, g.h), (-4, -4, 5, 5));
        // Display cell (1,1) draws at canvas 4..12: its border pixel (4,4) is set, (3,3) is not.
        let px = &s.cels[0][l].as_ref().unwrap().pixels;
        assert!(px[(4 * 32 + 4) * 4 + 3] > 0, "tile drawn at half-tile offset");
        assert_eq!(px[(3 * 32 + 3) * 4 + 3], 0);
        for (x, y) in [(2, 1), (1, 2), (2, 2)] {
            paint_terrain(&mut s, l, 0, x, y, Some(1));
        }
        assert_eq!(mask_of(&s, l, 2, 2), Some(15));
        assert!(missing_cells(&s, l, 0).is_empty());
    }

    #[test]
    fn transform_dirs_match_rendering() {
        // A marker pixel at the north edge of a tile must end up where transform_dir says.
        let mut ts = crate::document::tilemap::Tileset::new("t", 8, 8);
        let id = ts.add_blank() as usize;
        std::sync::Arc::make_mut(&mut ts.tiles[id])[4 * 4..][..4].copy_from_slice(&[255, 0, 0, 255]);
        for t in 0..8u8 {
            let g = TileGrid { x: 0, y: 0, w: 1, h: 1, tiles: vec![id as u32 | transform_flags(t)] };
            let px = render_grid(&g, &ts, 8, 8);
            let (mut fx, mut fy) = (0, 0);
            for y in 0..8 {
                for x in 0..8 {
                    if px[(y * 8 + x) * 4 + 3] != 0 {
                        (fx, fy) = (x as i32, y as i32);
                    }
                }
            }
            let (dx, dy) = DIRS[transform_dir(0, t)];
            let want = (
                if dx < 0 {
                    0
                } else if dx > 0 {
                    7
                } else {
                    fx
                },
                if dy < 0 {
                    0
                } else if dy > 0 {
                    7
                } else {
                    fy
                },
            );
            assert_eq!((fx, fy), want, "transform {t}");
        }
    }

    #[test]
    fn reduced_tilesets_reuse_by_rotation_and_flip() {
        assert_eq!(reduced_masks(Mode::Sides).len(), 6);
        assert_eq!(reduced_masks(Mode::Corners).len(), 6);
        let blob = reduced_masks(Mode::Blob).len();
        assert!((10..=20).contains(&blob), "{blob}");
        for mode in [Mode::Sides, Mode::Corners, Mode::Blob] {
            let mut s = Sprite::new(32, 32);
            let l = add_tilemap_layer(&mut s, 0, 8, 8);
            set_terrain(&mut s, l, "G", [0, 0, 0, 255], mode);
            let n = generate_template(&mut s, l, true);
            assert_eq!(n, reduced_masks(mode).len());
            for (x, y) in [(1, 1), (2, 1), (1, 2), (3, 3), (0, 3), (2, 2), (3, 0), (0, 0), (1, 3), (0, 2)] {
                paint_terrain(&mut s, l, 0, x, y, Some(1));
            }
            assert!(missing_cells(&s, l, 0).is_empty(), "{mode:?} {:?}", missing_cells(&s, l, 0));
            // The cache, seen through flips, shows the wanted masks.
            let at = load(&s, l).unwrap();
            for y in 0..4 {
                for x in 0..4 {
                    if mode != Mode::Corners {
                        if let Some((_, want)) = at.wanted(0, x, y, (4, 4)) {
                            let wm =
                                mode.dirs().iter().filter(|&&d| want[d] != 0).fold(0, |m, &d| m | mode.bit(d).unwrap());
                            assert_eq!(mask_of(&s, l, x, y), Some(wm), "{mode:?} ({x},{y})");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn override_survives_repaint() {
        let (mut s, l) = setup(Mode::Blob);
        paint_terrain(&mut s, l, 0, 1, 1, Some(1));
        assert!(set_override(&mut s, l, 0, 1, 1, Some(20)));
        assert_eq!(cell(&s, l, 1, 1), 20);
        paint_terrain(&mut s, l, 0, 2, 1, Some(1));
        paint_terrain(&mut s, l, 0, 1, 1, None);
        paint_terrain(&mut s, l, 0, 1, 1, Some(1));
        assert_eq!(cell(&s, l, 1, 1), 20);
        rebuild(&mut s, l, 0);
        assert_eq!(cell(&s, l, 1, 1), 20);
        set_override(&mut s, l, 0, 1, 1, None);
        assert_ne!(cell(&s, l, 1, 1), 20);
        assert_ne!(cell(&s, l, 1, 1), 0);
        set_override(&mut s, l, 0, 1, 1, Some(20));
        clear_overrides(&mut s, l, 0);
        assert_ne!(cell(&s, l, 1, 1), 20);
        // A manual stamp becomes an override; stamping empty restores auto.
        note_stamp(&mut s, l, 0, 8, 8, 21);
        assert_eq!(load(&s, l).unwrap().override_at(0, 1, 1), Some(21));
        note_stamp(&mut s, l, 0, 8, 8, 0);
        assert_eq!(load(&s, l).unwrap().override_at(0, 1, 1), None);
    }

    #[test]
    fn missing_tile_fallback_flag_and_report() {
        let mut s = Sprite::new(32, 32);
        let l = add_tilemap_layer(&mut s, 0, 8, 8);
        set_terrain(&mut s, l, "G", [0, 0, 0, 255], Mode::Blob);
        let a = s.tilesets[0].add_blank();
        let b = s.tilesets[0].add_blank();
        set_tile_mask(&mut s, l, a, Some(0));
        set_tile_mask(&mut s, l, b, Some(E | W));
        paint_terrain(&mut s, l, 0, 1, 1, Some(1));
        paint_terrain(&mut s, l, 0, 2, 1, Some(1));
        assert!(missing_cells(&s, l, 0).contains(&(1, 1)));
        assert_ne!(cell(&s, l, 1, 1), 0);
        assert_ne!(cell(&s, l, 2, 1), 0);
        paint_terrain(&mut s, l, 0, 0, 1, Some(1));
        assert!(!missing_cells(&s, l, 0).contains(&(1, 1)));
        let rep = missing_report(&s, l, 0);
        assert!(rep[0].starts_with("G: missing masks"), "{rep:?}");
        assert!(!rep[0].contains("masks 0,"));
        // No tiles at all: no crash, flagged.
        let mut s3 = Sprite::new(16, 16);
        let l3 = add_tilemap_layer(&mut s3, 0, 8, 8);
        set_terrain(&mut s3, l3, "G", [0, 0, 0, 255], Mode::Sides);
        paint_terrain(&mut s3, l3, 0, 0, 0, Some(1));
        assert_eq!(missing_cells(&s3, l3, 0), vec![(0, 0)]);
    }

    #[test]
    fn variation_is_deterministic_weighted_and_seeded() {
        let mut s = Sprite::new(64, 8);
        let l = add_tilemap_layer(&mut s, 0, 8, 8);
        set_terrain(&mut s, l, "G", [0, 0, 0, 255], Mode::Sides);
        let a = s.tilesets[0].add_blank();
        let b = s.tilesets[0].add_blank();
        set_tile_mask(&mut s, l, a, Some(0));
        set_tile_mask(&mut s, l, b, Some(0));
        let pick = |s: &Sprite| -> Vec<u32> {
            let at = load(s, l).unwrap();
            (0..64).map(|x| at.choose(Some(1), &[0; 8], x, 0, true).value).collect()
        };
        let p1 = pick(&s);
        assert_eq!(p1, pick(&s));
        assert!(p1.contains(&a) && p1.contains(&b));
        set_seed(&mut s, l, 7);
        assert_ne!(p1, pick(&s));
        assert_eq!(pick(&s), pick(&s));
        assert!(set_tile_weight(&mut s, l, b, 0.0));
        assert!(pick(&s).iter().all(|&v| v == a));
        set_tile_weight(&mut s, l, b, 1.0);
        for x in 0..8 {
            paint_terrain(&mut s, l, 0, x, 0, Some(1));
        }
        let before: Vec<u32> = (0..8).map(|x| cell(&s, l, x, 0)).collect();
        rebuild(&mut s, l, 0);
        assert_eq!(before, (0..8).map(|x| cell(&s, l, x, 0)).collect::<Vec<_>>());
    }

    #[test]
    fn map_edge_option() {
        let (mut s, l) = setup(Mode::Sides);
        paint_terrain(&mut s, l, 0, 0, 0, Some(1));
        assert_eq!(mask_of(&s, l, 0, 0), Some(0));
        set_edge_same(&mut s, l, true);
        assert_eq!(mask_of(&s, l, 0, 0), Some(1 | 8)); // N and W are outside the map
    }

    #[test]
    fn multi_terrain_transitions_and_priority() {
        let mut s = Sprite::new(32, 8);
        let l = add_tilemap_layer(&mut s, 0, 8, 8);
        set_terrain(&mut s, l, "Grass", [0, 255, 0, 255], Mode::Sides);
        let dirt = add_terrain(&mut s, l, "Dirt", [150, 100, 50, 255]).unwrap();
        assert_eq!(dirt, 2);
        let g = s.tilesets[0].add_blank();
        let gt = s.tilesets[0].add_blank();
        let d = s.tilesets[0].add_blank();
        set_tile_terrain(&mut s, l, g, 1);
        set_tile_terrain(&mut s, l, gt, 1);
        set_tile_peer(&mut s, l, gt, 2, 2); // grass with dirt to the east
        set_tile_terrain(&mut s, l, d, 2);
        paint_terrain(&mut s, l, 0, 0, 0, Some(1));
        paint_terrain(&mut s, l, 0, 1, 0, Some(2));
        assert_eq!(cell(&s, l, 0, 0), gt, "grass next to dirt uses the transition tile");
        assert_eq!(cell(&s, l, 1, 0), d);
        assert!(!missing_cells(&s, l, 0).contains(&(0, 0)));
        paint_terrain(&mut s, l, 0, 1, 0, None);
        assert_eq!(cell(&s, l, 0, 0), g);
        // Without the transition tile, higher-priority grass treats dirt as empty and stays exact;
        // lower-priority dirt next to grass has no tile for it and is flagged.
        let mut at = load(&s, l).unwrap();
        at.tiles.remove(&gt);
        save(&mut s, l, &at);
        paint_terrain(&mut s, l, 0, 1, 0, Some(2));
        assert_eq!(cell(&s, l, 0, 0), g);
        assert!(!missing_cells(&s, l, 0).contains(&(0, 0)));
        assert!(missing_cells(&s, l, 0).contains(&(1, 0)));
        // Moving dirt above grass swaps who is exact.
        move_terrain(&mut s, l, 2, 0);
        assert!(!missing_cells(&s, l, 0).contains(&(1, 0)));
        assert!(missing_cells(&s, l, 0).contains(&(0, 0)));
        assert!(missing_report(&s, l, 0).iter().any(|r| r.contains("needs E=Dirt")));
        assert!(remove_terrain(&mut s, l, 2));
        assert_eq!(terrain_at(&s, l, 0, 1, 0), None);
    }

    #[test]
    fn multi_terrain_dual_grid_corners() {
        let mut s = Sprite::new(32, 8);
        let l = add_tilemap_layer(&mut s, 0, 8, 8);
        set_terrain(&mut s, l, "Grass", [0, 255, 0, 255], Mode::Corners);
        add_terrain(&mut s, l, "Water", [0, 0, 255, 255]);
        let t = s.tilesets[0].add_blank();
        // A tile with grass on its west corners and water on its east corners.
        for (dir, id) in [(7, 1), (5, 1), (1, 2), (3, 2)] {
            set_tile_peer(&mut s, l, t, dir, id);
        }
        paint_terrain(&mut s, l, 0, 0, 0, Some(1));
        paint_terrain(&mut s, l, 0, 1, 0, Some(2));
        // Display cell (1,0) has grass at its west corners (world 0) and water at its east (world 1).
        assert_eq!(cell(&s, l, 1, 0), t);
    }

    #[test]
    fn tools_rect_flood_brush_mirror() {
        let (mut s, l) = setup(Mode::Blob);
        let cells = rect_cells(0, 0, 3, 1);
        assert_eq!(cells.len(), 8);
        paint_cells(&mut s, l, 0, &cells, Some(1));
        assert_eq!(terrain_at(&s, l, 0, 3, 1), Some(1));
        assert_eq!(terrain_at(&s, l, 0, 3, 2), None);
        let region = flood_region(&s, l, 0, 0, 3);
        assert_eq!(region.len(), 8);
        paint_cells(&mut s, l, 0, &region, Some(1));
        assert_eq!(terrain_at(&s, l, 0, 0, 3), Some(1));
        assert_eq!(brush_cells(2, 2, 1, false), vec![(2, 2)]);
        assert_eq!(brush_cells(2, 2, 3, false).len(), 9);
        assert!(brush_cells(2, 2, 5, true).len() < 25);
        let m = mirror_cells(&[(0, 0)], 3, (4, 4));
        assert!(m.contains(&(3, 0)) && m.contains(&(0, 3)) && m.contains(&(3, 3)));
        paint_cells(&mut s, l, 0, &cells, None);
        assert_eq!(terrain_at(&s, l, 0, 0, 0), None);
    }

    #[test]
    fn linked_cels_share_intent() {
        let (mut s, l) = setup(Mode::Sides);
        paint_terrain(&mut s, l, 0, 0, 0, Some(1));
        s.add_frame(0, true);
        assert!(s.link_cels(l, 0, 1));
        paint_terrain(&mut s, l, 1, 1, 1, Some(1));
        s.flush_links();
        assert_eq!(terrain_at(&s, l, 0, 1, 1), Some(1));
        assert!(s.is_cel_linked(l, 0));
        assert_eq!(cell(&s, l, 1, 1), tile_at(&s, l, 1, 8, 8));
    }

    #[test]
    fn tile_edits_propagate() {
        let (mut s, l) = setup(Mode::Sides);
        paint_terrain(&mut s, l, 0, 1, 1, Some(1));
        let id = (cell(&s, l, 1, 1) & TILE_ID_MASK) as usize;
        std::sync::Arc::make_mut(&mut s.tilesets[0].tiles[id])[(4 * 8 + 4) * 4..][..4].copy_from_slice(&[9, 9, 9, 255]);
        crate::document::tilemap::refresh_all(&mut s);
        assert_eq!(&s.cels[0][l].as_ref().unwrap().pixels[(12 * 32 + 12) * 4..][..4], &[9, 9, 9, 255]);
    }

    #[test]
    fn mode_change_rebuilds_from_intent() {
        let (mut s, l) = setup(Mode::Blob);
        paint_terrain(&mut s, l, 0, 1, 1, Some(1));
        set_mode(&mut s, l, Mode::Sides);
        assert_eq!(load(&s, l).unwrap().intent_at(0, 1, 1), Some(1));
        assert_ne!(cell(&s, l, 1, 1), 0);
        set_mode(&mut s, l, Mode::Corners);
        assert_eq!(load(&s, l).unwrap().mode, Mode::Corners);
        let g = s.cels[0][l].as_ref().unwrap().tiles.as_ref().unwrap();
        assert_eq!((g.w, g.h), (5, 5));
    }

    #[test]
    fn persistence_roundtrip_and_undo() {
        let (mut s, l) = setup(Mode::Blob);
        let a0 = load(&s, l).unwrap();
        s.layers[l].user_data.text = Some(join_text("my notes\nline 2", &a0));
        let dirt = add_terrain(&mut s, l, "Di;rt:|x", [9, 8, 7, 255]).unwrap();
        paint_terrain(&mut s, l, 0, 1, 1, Some(1));
        paint_terrain(&mut s, l, 0, 2, 1, Some(dirt));
        set_override(&mut s, l, 0, 3, 3, Some(5 | FLIP_X));
        set_tile_weight(&mut s, l, 4, 2.5);
        set_tile_sym(&mut s, l, 4, 5);
        set_seed(&mut s, l, 99);
        set_edge_same(&mut s, l, true);
        assert_eq!(user_text(&s, l), "my notes\nline 2");
        let before = load(&s, l).unwrap();
        let bytes = crate::formats::ase::write(&s);
        let s2 = crate::formats::ase::read(&bytes).unwrap();
        let after = load(&s2, l).unwrap();
        assert_eq!(before, after);
        assert_eq!(after.terrains[1].name, "Di;rt:|x");
        assert_eq!(user_text(&s2, l), "my notes\nline 2");
        assert_eq!(tile_masks(&s2, 0)[1], Some(0));
        let p = tile_peering(&s2, 0, 4).unwrap();
        assert_eq!((p.weight, p.sym), (2.5, 5));
        // The cache is rebuildable from the loaded intent.
        let mut s3 = s2.clone();
        let c = cell(&s3, l, 1, 1);
        rebuild(&mut s3, l, 0);
        assert_eq!(c, cell(&s3, l, 1, 1));
        // Undo restores the payload together with the cache.
        let mut e = Editor::new(s2);
        e.checkpoint();
        paint_terrain(&mut e.sprite, l, 0, 0, 0, Some(1));
        assert_eq!(terrain_at(&e.sprite, l, 0, 0, 0), Some(1));
        assert!(e.undo());
        assert_eq!(terrain_at(&e.sprite, l, 0, 0, 0), None);
        assert_eq!(user_text(&e.sprite, l), "my notes\nline 2");
        assert!(info_text(&e.sprite).contains("mode blob"));
    }

    #[test]
    fn template_assign_detects_layouts() {
        for (mode, full) in [(Mode::Blob, 47usize), (Mode::Sides, 16), (Mode::Corners, 16)] {
            let red = reduced_masks(mode).len();
            let mut s = Sprite::new(16, 16);
            let l = add_tilemap_layer(&mut s, 0, 8, 8);
            set_terrain(&mut s, l, "G", [0, 0, 0, 255], mode);
            for _ in 0..full {
                s.tilesets[0].add_blank();
            }
            assert_eq!(assign_masks_from_template(&mut s, l), full);
            let m = tile_masks(&s, 0);
            assert_eq!(m[1], Some(0));
            assert_eq!(m[full], Some(*mode.masks().last().unwrap()));
            let mut s = Sprite::new(16, 16);
            let l = add_tilemap_layer(&mut s, 0, 8, 8);
            set_terrain(&mut s, l, "G", [0, 0, 0, 255], mode);
            for _ in 0..red {
                s.tilesets[0].add_blank();
            }
            assert_eq!(assign_masks_from_template(&mut s, l), red);
            assert_eq!(tile_peering(&s, 0, 1).unwrap().sym, 7);
        }
    }
}
