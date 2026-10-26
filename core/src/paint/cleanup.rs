//! Clean Edges and Sprite Lint: tidy a sprite that was traced from reference art, and report what is wrong
//! with it without changing anything.
//!
//! The cleanup passes work on one straight-alpha RGBA cel buffer and share these ideas:
//!
//! * A pixel is *solid* when its alpha is above zero. Components are 8-connected; transparent regions are
//!   4-connected (so a diagonal run of solid pixels closes a region, as it does on screen).
//! * *Outside* is the transparent region that touches the canvas border. Transparent areas that do not are
//!   *holes*. The canvas border itself is not "transparency": a pixel at the edge of the canvas is never
//!   treated as lying on the outer ring because of what is beyond the canvas.
//! * A solid pixel is only ever removed when it is a *simple point*: removing it neither splits a component,
//!   nor merges or creates a hole (see `simple`). This is the guard that keeps thin strands, 1 px fingers
//!   and outlines intact.
//!
//! Details inside the body (eyes, highlights, dark marks) are never candidates: halo removal and outline
//! thinning only look at pixels that are reachable from the outside, and despeckling only removes whole
//! components that touch the outside. Anything that could eat interior detail (filling holes, removing lone
//! light pixels) is opt-in.
//!
//! `Filter::CleanEdges` (see `paint::filters`) wraps `clean_edges`, so previews, selections, "all frames" and
//! undo work like every other filter.

use crate::document::Sprite;
use crate::paint::filters::Target;

// ---- Options -----------------------------------------------------------------------------

/// Paper-halo removal: light, grayish pixels on the outermost rings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HaloOptions {
    /// How many rings to peel at most (1 = only pixels touching the outside).
    pub rings: u32,
    /// A halo pixel must be at least this much lighter (luma, 0..=255) than every neighbour behind it.
    pub min_luma_gain: u32,
    /// A halo pixel must have at least this luma (0..=255).
    pub min_luma: u32,
    /// A halo pixel must have at most this HSV saturation (percent).
    pub max_saturation: u32,
    /// ... and not be more saturated than a neighbour behind it by more than this (percent).
    pub saturation_slack: u32,
    /// Light pixels are only removed as part of a connected group of at least this many (a lone light
    /// pixel on the rim is a highlight, not a halo).
    pub min_group: u32,
}

impl Default for HaloOptions {
    fn default() -> Self {
        HaloOptions {
            rings: 2,
            min_luma_gain: 40,
            min_luma: 110,
            max_saturation: 30,
            saturation_slack: 10,
            min_group: 3,
        }
    }
}

/// Removal of detached specks (and, optionally, tiny enclosed holes and lone light pixels).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DespeckleOptions {
    /// Loose components smaller than this many pixels are removed. 0 disables.
    pub min_size: u32,
    /// Enclosed transparent areas smaller than this many pixels are filled. 0 (the default) keeps every
    /// hole, because a transparent eye or a window is a hole too.
    pub hole_size: u32,
    /// Also replace a light, grayish pixel whose eight neighbours are all solid and much darker with their
    /// most common colour. Off by default: a 1 px highlight inside a dark eye looks exactly like this.
    pub lone_light: bool,
}

impl Default for DespeckleOptions {
    fn default() -> Self {
        DespeckleOptions { min_size: 4, hole_size: 0, lone_light: false }
    }
}

/// Thinning of the outer outline to one pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThinOptions {
    /// The outline colour (RGB). `None` treats every pixel whose luma is at most `max_luma` as outline.
    pub color: Option<[u8; 3]>,
    /// Largest per-channel difference from `color` that still counts as outline.
    pub tolerance: u32,
    /// Darkness limit of an automatically detected outline (luma, 0..=255).
    pub max_luma: u32,
    /// Outlines thicker than this are left alone (so a big dark mass such as hair is not eroded).
    pub max_thickness: u32,
}

impl Default for ThinOptions {
    fn default() -> Self {
        ThinOptions { color: None, tolerance: 0, max_luma: 70, max_thickness: 3 }
    }
}

/// The passes of Clean Edges; `None` skips a pass. They run in the order halo, despeckle, thin outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CleanEdgesOptions {
    pub halo: Option<HaloOptions>,
    pub despeckle: Option<DespeckleOptions>,
    pub thin_outline: Option<ThinOptions>,
}

impl CleanEdgesOptions {
    /// Every pass with its default settings.
    pub fn all() -> Self {
        CleanEdgesOptions {
            halo: Some(HaloOptions::default()),
            despeckle: Some(DespeckleOptions::default()),
            thin_outline: Some(ThinOptions::default()),
        }
    }
}

// ---- Pixel helpers -----------------------------------------------------------------------

const N8: [(i32, i32); 8] = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];
const N4: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

fn luma(p: &[u8]) -> i32 {
    ((299 * p[0] as u32 + 587 * p[1] as u32 + 114 * p[2] as u32) / 1000) as i32
}

/// HSV saturation in percent.
fn saturation(p: &[u8]) -> i32 {
    let max = p[0].max(p[1]).max(p[2]) as i32;
    let min = p[0].min(p[1]).min(p[2]) as i32;
    if max == 0 {
        0
    } else {
        100 * (max - min) / max
    }
}

fn solid_mask(px: &[u8]) -> Vec<bool> {
    px.chunks_exact(4).map(|p| p[3] > 0).collect()
}

fn at(px: &[u8], i: usize) -> [u8; 4] {
    [px[i * 4], px[i * 4 + 1], px[i * 4 + 2], px[i * 4 + 3]]
}

/// Index of the neighbour at (dx, dy), if it is on the canvas.
fn nb(w: usize, h: usize, x: usize, y: usize, d: (i32, i32)) -> Option<usize> {
    let (nx, ny) = (x as i32 + d.0, y as i32 + d.1);
    (nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h).then(|| ny as usize * w + nx as usize)
}

/// The transparent pixels connected (4-connected) to the canvas border.
fn outside_mask(solid: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut out = vec![false; w * h];
    let mut stack = Vec::new();
    let push = |i: usize, out: &mut Vec<bool>, stack: &mut Vec<usize>| {
        if !solid[i] && !out[i] {
            out[i] = true;
            stack.push(i);
        }
    };
    for x in 0..w {
        push(x, &mut out, &mut stack);
        push((h - 1) * w + x, &mut out, &mut stack);
    }
    for y in 0..h {
        push(y * w, &mut out, &mut stack);
        push(y * w + w - 1, &mut out, &mut stack);
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        for d in N4 {
            if let Some(j) = nb(w, h, x, y, d) {
                push(j, &mut out, &mut stack);
            }
        }
    }
    out
}

/// Labels the 8-connected components of `set`: (label per pixel, 0 = none; size per label, index 0 unused).
fn components8(set: &[bool], w: usize, h: usize) -> (Vec<u32>, Vec<usize>) {
    let mut label = vec![0u32; w * h];
    let mut sizes = vec![0usize];
    let mut stack = Vec::new();
    for s in 0..w * h {
        if !set[s] || label[s] != 0 {
            continue;
        }
        let id = sizes.len() as u32;
        sizes.push(0);
        label[s] = id;
        stack.push(s);
        while let Some(i) = stack.pop() {
            sizes[id as usize] += 1;
            let (x, y) = (i % w, i / w);
            for d in N8 {
                if let Some(j) = nb(w, h, x, y, d) {
                    if set[j] && label[j] == 0 {
                        label[j] = id;
                        stack.push(j);
                    }
                }
            }
        }
    }
    (label, sizes)
}

/// The enclosed transparent areas: (first pixel in scan order, pixels) of each 4-connected transparent region
/// that does not touch the border, plus the label of every pixel (0 = not in a hole).
fn holes_of(solid: &[bool], w: usize, h: usize) -> (Vec<u32>, Vec<Vec<usize>>) {
    let outside = outside_mask(solid, w, h);
    let mut label = vec![0u32; w * h];
    let mut holes: Vec<Vec<usize>> = Vec::new();
    for s in 0..w * h {
        if solid[s] || outside[s] || label[s] != 0 {
            continue;
        }
        let id = holes.len() as u32 + 1;
        let mut members = Vec::new();
        label[s] = id;
        let mut stack = vec![s];
        while let Some(i) = stack.pop() {
            members.push(i);
            let (x, y) = (i % w, i / w);
            for d in N4 {
                if let Some(j) = nb(w, h, x, y, d) {
                    if !solid[j] && !outside[j] && label[j] == 0 {
                        label[j] = id;
                        stack.push(j);
                    }
                }
            }
        }
        holes.push(members);
    }
    (label, holes)
}

/// Would removing the pixel at (x, y) from `set` keep its topology? True when the pixel's 8-neighbourhood
/// holds exactly one 8-connected group of `set` pixels and exactly one 4-connected group of non-`set` pixels
/// that touches the pixel along an edge. Then removal cannot split a component, open or merge a hole, or
/// delete a whole component. Pixels beyond the canvas count as not in `set`.
fn simple(set: &[bool], w: usize, h: usize, x: usize, y: usize) -> bool {
    let mut cell = [false; 8];
    for (k, d) in N8.iter().enumerate() {
        cell[k] = nb(w, h, x, y, *d).is_some_and(|j| set[j]);
    }
    // Groups by flood fill over the eight ring cells; `adjacent` decides which cells touch.
    let groups = |want: bool, adjacent: fn((i32, i32), (i32, i32)) -> bool, only_edge_cells: bool| -> usize {
        let mut seen = [false; 8];
        let mut count = 0;
        for s in 0..8 {
            if cell[s] != want || seen[s] {
                continue;
            }
            let mut touches_edge = s % 2 == 0;
            seen[s] = true;
            let mut stack = vec![s];
            while let Some(a) = stack.pop() {
                for b in 0..8 {
                    if !seen[b] && cell[b] == want && adjacent(N8[a], N8[b]) {
                        seen[b] = true;
                        touches_edge |= b % 2 == 0;
                        stack.push(b);
                    }
                }
            }
            if touches_edge || !only_edge_cells {
                count += 1;
            }
        }
        count
    };
    let king = |a: (i32, i32), b: (i32, i32)| (a.0 - b.0).abs().max((a.1 - b.1).abs()) <= 1;
    let rook = |a: (i32, i32), b: (i32, i32)| (a.0 - b.0).abs() + (a.1 - b.1).abs() == 1;
    groups(true, king, false) == 1 && groups(false, rook, true) == 1
}

/// Most common colour among `colors` (first seen wins ties).
fn mode_color(colors: impl Iterator<Item = [u8; 4]>) -> Option<[u8; 4]> {
    let mut counts: Vec<([u8; 4], u32)> = Vec::new();
    for c in colors {
        match counts.iter_mut().find(|(k, _)| *k == c) {
            Some((_, n)) => *n += 1,
            None => counts.push((c, 1)),
        }
    }
    let mut best: Option<([u8; 4], u32)> = None;
    for (c, n) in counts {
        if best.is_none_or(|(_, bn)| n > bn) {
            best = Some((c, n));
        }
    }
    best.map(|(c, _)| c)
}

fn clear(px: &mut [u8], i: usize) {
    px[i * 4..i * 4 + 4].fill(0);
}

/// Whether a pixel passes the "light and grayish" test of the halo.
fn halo_like(p: &[u8], o: &HaloOptions) -> bool {
    luma(p) >= o.min_luma as i32 && saturation(p) <= o.max_saturation as i32
}

// ---- Halo --------------------------------------------------------------------------------

/// Distance (in rings) of every solid pixel from the outside: 1 for a pixel 4-adjacent to the outside,
/// `u32::MAX` for pixels farther than `limit` rings or not reached.
fn ring_depth(solid: &[bool], outside: &[bool], w: usize, h: usize, limit: u32) -> Vec<u32> {
    let mut depth = vec![u32::MAX; w * h];
    let mut queue = std::collections::VecDeque::new();
    for i in 0..w * h {
        if solid[i] && N4.iter().any(|&d| nb(w, h, i % w, i / w, d).is_some_and(|j| outside[j])) {
            depth[i] = 1;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        if depth[i] >= limit {
            continue;
        }
        for d in N4 {
            if let Some(j) = nb(w, h, i % w, i / w, d) {
                if solid[j] && depth[j] == u32::MAX {
                    depth[j] = depth[i] + 1;
                    queue.push_back(j);
                }
            }
        }
    }
    depth
}

/// Removes light, grayish "paper halo" pixels from the outer rings. A pixel is a candidate when it is light
/// and grayish, has pixels behind it (a 1 px strand has none), and is much lighter and not more colourful than
/// every pixel behind it (or sits in front of another candidate). Candidates must form a group, and each one
/// is removed only while it touches the outside and is a simple point. Returns whether anything changed.
pub fn remove_halo(px: &mut [u8], w: usize, h: usize, o: &HaloOptions) -> bool {
    if o.rings == 0 || px.len() != w * h * 4 {
        return false;
    }
    let mut solid = solid_mask(px);
    let mut outside = outside_mask(&solid, w, h);
    let depth = ring_depth(&solid, &outside, w, h, o.rings);

    let mut cand = vec![false; w * h];
    for d in (1..=o.rings).rev() {
        for i in 0..w * h {
            if depth[i] != d || !halo_like(&px[i * 4..], o) {
                continue;
            }
            let (x, y) = (i % w, i / w);
            let (lp, sp) = (luma(&px[i * 4..]), saturation(&px[i * 4..]));
            let mut behind = 0;
            let mut ok = true;
            for dd in N8 {
                let Some(j) = nb(w, h, x, y, dd) else { continue };
                if !solid[j] || depth[j] <= d {
                    continue; // outside the sprite, or level with / in front of this pixel
                }
                behind += 1;
                let contrast = lp - luma(&px[j * 4..]) >= o.min_luma_gain as i32
                    && sp <= saturation(&px[j * 4..]) + o.saturation_slack as i32;
                if !(contrast || (depth[j] <= o.rings && cand[j])) {
                    ok = false;
                    break;
                }
            }
            cand[i] = ok && behind > 0;
        }
    }
    // A halo is a band, not a lone light pixel.
    let (label, sizes) = components8(&cand, w, h);
    for i in 0..w * h {
        if cand[i] && sizes[label[i] as usize] < o.min_group as usize {
            cand[i] = false;
        }
    }

    let mut changed = false;
    for _ in 0..o.rings {
        let ring: Vec<usize> = (0..w * h)
            .filter(|&i| {
                cand[i] && solid[i] && N4.iter().any(|&d| nb(w, h, i % w, i / w, d).is_some_and(|j| outside[j]))
            })
            .collect();
        let mut any = false;
        for i in ring {
            if simple(&solid, w, h, i % w, i / w) {
                solid[i] = false;
                outside[i] = true;
                clear(px, i);
                any = true;
            }
        }
        changed |= any;
        if !any {
            break;
        }
    }
    changed
}

// ---- Despeckle ---------------------------------------------------------------------------

/// Removes loose fragments, fills tiny holes and (optionally) replaces lone light pixels; see
/// `DespeckleOptions`. `light` supplies the thresholds of "light and grayish". Returns whether anything changed.
pub fn despeckle(px: &mut [u8], w: usize, h: usize, o: &DespeckleOptions, light: &HaloOptions) -> bool {
    if px.len() != w * h * 4 {
        return false;
    }
    let mut changed = false;
    if o.min_size > 0 {
        let solid = solid_mask(px);
        let doomed = loose_components(&solid, w, h, o.min_size as usize);
        for comp in doomed {
            for i in comp {
                clear(px, i);
                changed = true;
            }
        }
    }
    if o.hole_size > 0 {
        let solid = solid_mask(px);
        let (_, holes) = holes_of(&solid, w, h);
        for hole in holes.iter().filter(|m| m.len() < o.hole_size as usize) {
            let mut around = Vec::new();
            for &i in hole {
                for d in N4 {
                    if let Some(j) = nb(w, h, i % w, i / w, d) {
                        if solid[j] {
                            around.push(at(px, j));
                        }
                    }
                }
            }
            if let Some(c) = mode_color(around.into_iter()) {
                for &i in hole {
                    px[i * 4..i * 4 + 4].copy_from_slice(&c);
                }
                changed = true;
            }
        }
    }
    if o.lone_light {
        let solid = solid_mask(px);
        let src = px.to_vec();
        for i in 0..w * h {
            if !solid[i] || !halo_like(&src[i * 4..], light) {
                continue;
            }
            let (x, y) = (i % w, i / w);
            let lp = luma(&src[i * 4..]);
            let mut around = Vec::with_capacity(8);
            let mut ok = true;
            for d in N8 {
                match nb(w, h, x, y, d) {
                    Some(j) if solid[j] && lp - luma(&src[j * 4..]) >= light.min_luma_gain as i32 => {
                        around.push(at(&src, j))
                    }
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                if let Some(c) = mode_color(around.into_iter()) {
                    px[i * 4..i * 4 + 4].copy_from_slice(&c);
                    changed = true;
                }
            }
        }
    }
    changed
}

/// Loose fragments: 8-connected components smaller than `min_size` that touch the outside, provided some
/// component is at least that big (a sprite made only of small pieces is left alone). A small component
/// sealed inside a hole of the body, such as a pupil in an eye socket, is part of the body and never listed.
/// Returns the pixels of each fragment.
fn loose_components(solid: &[bool], w: usize, h: usize, min_size: usize) -> Vec<Vec<usize>> {
    let (label, sizes) = components8(solid, w, h);
    if !sizes.iter().skip(1).any(|&s| s >= min_size) {
        return Vec::new();
    }
    let outside = outside_mask(solid, w, h);
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); sizes.len()];
    for (i, &l) in label.iter().enumerate() {
        if l != 0 {
            members[l as usize].push(i);
        }
    }
    members
        .into_iter()
        .enumerate()
        .skip(1)
        .filter(|(l, m)| {
            sizes[*l] < min_size
                && m.iter().any(|&i| {
                    let (x, y) = (i % w, i / w);
                    x == 0
                        || y == 0
                        || x == w - 1
                        || y == h - 1
                        || N4.iter().any(|&d| nb(w, h, x, y, d).is_some_and(|j| outside[j]))
                })
        })
        .map(|(_, m)| m)
        .collect()
}

// ---- Thin outline ------------------------------------------------------------------------

fn outline_mask(px: &[u8], o: &ThinOptions) -> Vec<bool> {
    px.chunks_exact(4)
        .map(|p| {
            p[3] > 0
                && match o.color {
                    Some(c) => (0..3).all(|k| p[k].abs_diff(c[k]) as u32 <= o.tolerance),
                    None => luma(p) <= o.max_luma as i32,
                }
        })
        .collect()
}

/// Steps from every pixel to the nearest solid non-outline pixel through solid pixels (8-connected, so a
/// corner of a square outline is as thick as its sides); `u32::MAX` where there is none.
fn body_distance(solid: &[bool], outline: &[bool], w: usize, h: usize) -> Vec<u32> {
    let mut dist = vec![u32::MAX; w * h];
    let mut queue = std::collections::VecDeque::new();
    for i in 0..w * h {
        if solid[i] && !outline[i] {
            dist[i] = 0;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        for d in N8 {
            if let Some(j) = nb(w, h, i % w, i / w, d) {
                if solid[j] && dist[j] == u32::MAX {
                    dist[j] = dist[i] + 1;
                    queue.push_back(j);
                }
            }
        }
    }
    dist
}

/// 2x 3x3 closing (two dilations, then two erosions) of `set`, computed on a copy padded by four empty pixels
/// so the canvas border neither fills nor eats anything.
fn close_twice(set: &[bool], w: usize, h: usize) -> Vec<bool> {
    const PAD: usize = 4;
    let (pw, ph) = (w + 2 * PAD, h + 2 * PAD);
    let mut cur = vec![false; pw * ph];
    for y in 0..h {
        for x in 0..w {
            cur[(y + PAD) * pw + x + PAD] = set[y * w + x];
        }
    }
    for dilating in [true, true, false, false] {
        let prev = cur.clone();
        for i in 0..pw * ph {
            let (x, y) = (i % pw, i / pw);
            let hit = |d: &(i32, i32)| nb(pw, ph, x, y, *d).is_some_and(|j| prev[j]);
            cur[i] = if dilating {
                prev[i] || N8.iter().any(hit)
            } else {
                prev[i] && N8.iter().all(|d| nb(pw, ph, x, y, *d).is_some_and(|j| prev[j]))
            };
        }
    }
    (0..w * h).map(|i| cur[(i / w + PAD) * pw + i % w + PAD]).collect()
}

/// Thins an outer outline that is two or more pixels thick to one pixel, keeping the pixels next to the body.
///
/// Only outline pixels on the outer ring are removed, one directional sweep at a time (top, bottom, left,
/// right edges), and only when
/// * every solid pixel next to them (4-connected) is outline too, so the body is never exposed, they are not
///   next to the body even diagonally (`body_distance >= 2`), and the outline is at most `max_thickness` thick,
/// * removal is a simple point of the sprite (nothing detaches, no hole opens or merges), and
/// * removal is a simple point of the outline itself (the outline stays 8-connected and closed).
///
/// Afterwards a 2x 3x3 closing of the thinned silhouette is computed and every pixel it fills that the thinning
/// removed is restored, which closes the notches that thinning leaves where two outlines nearly meet (a cuff
/// next to a fist). If the result still has more components or holes than the input, nothing is changed.
/// Returns whether anything changed.
pub fn thin_outline(px: &mut [u8], w: usize, h: usize, o: &ThinOptions) -> bool {
    if o.max_thickness < 2 || px.len() != w * h * 4 {
        return false;
    }
    let before = px.to_vec();
    let orig_solid = solid_mask(&before);
    let is_outline = outline_mask(&before, o);
    let mut solid = orig_solid.clone();
    let mut outside = outside_mask(&solid, w, h);
    let mut oset: Vec<bool> = (0..w * h).map(|i| solid[i] && is_outline[i]).collect();

    // Each sweep removes at most one layer on its side, so `max_thickness` rounds suffice.
    for _ in 0..o.max_thickness {
        let mut round = false;
        for dir in N4 {
            let dist = body_distance(&solid, &is_outline, w, h);
            for i in 0..w * h {
                if !oset[i] || !(2..=o.max_thickness).contains(&dist[i]) {
                    continue;
                }
                let (x, y) = (i % w, i / w);
                if !nb(w, h, x, y, dir).is_some_and(|j| outside[j]) {
                    continue;
                }
                // The pixels that would become the new edge must be outline too.
                if N4.iter().any(|&d| nb(w, h, x, y, d).is_some_and(|j| solid[j] && !is_outline[j])) {
                    continue;
                }
                if simple(&solid, w, h, x, y) && simple(&oset, w, h, x, y) {
                    solid[i] = false;
                    oset[i] = false;
                    outside[i] = true;
                    clear(px, i);
                    round = true;
                }
            }
        }
        if !round {
            break;
        }
    }

    // Undo notches: whatever a closing of the thinned shape fills, and thinning removed, comes back.
    let closed = close_twice(&solid, w, h);
    for i in 0..w * h {
        if closed[i] && !solid[i] && orig_solid[i] {
            px[i * 4..i * 4 + 4].copy_from_slice(&before[i * 4..i * 4 + 4]);
            solid[i] = true;
        }
    }

    let topology = |s: &[bool]| (components8(s, w, h).1.len(), holes_of(s, w, h).1.len());
    let (c0, h0) = topology(&orig_solid);
    let (c1, h1) = topology(&solid);
    if c1 > c0 || h1 > h0 {
        px.copy_from_slice(&before);
        return false;
    }
    px != before.as_slice()
}

// ---- Clean Edges -------------------------------------------------------------------------

/// Runs the enabled passes (halo, despeckle, thin outline) on a copy of the buffer and returns it.
pub fn clean_edges(src: &[u8], w: usize, h: usize, o: &CleanEdgesOptions) -> Vec<u8> {
    let mut px = src.to_vec();
    if px.len() != w * h * 4 || w == 0 || h == 0 {
        return px;
    }
    let halo = o.halo.unwrap_or_default();
    if let Some(ho) = &o.halo {
        remove_halo(&mut px, w, h, ho);
    }
    if let Some(d) = &o.despeckle {
        despeckle(&mut px, w, h, d, &halo);
    }
    if let Some(t) = &o.thin_outline {
        thin_outline(&mut px, w, h, t);
    }
    px
}

// ---- Lint --------------------------------------------------------------------------------

/// What `lint_cel` looks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LintOptions {
    /// Fragments smaller than this are "detached specks".
    pub speck_size: u32,
    /// At most this many positions are listed per category (the counts are always complete).
    pub max_list: usize,
    /// Also report colours that are not in the sprite's palette (only meaningful for palette-based art).
    pub check_palette: bool,
    /// How to recognise the outline, for "pixels with no outline neighbour".
    pub outline: ThinOptions,
    /// Thresholds of "light" pixels on the outline ring.
    pub halo: HaloOptions,
}

impl Default for LintOptions {
    fn default() -> Self {
        LintOptions {
            speck_size: 4,
            max_list: 64,
            check_palette: false,
            outline: ThinOptions::default(),
            halo: HaloOptions::default(),
        }
    }
}

/// One finding: the pixel (for areas, the first pixel in scan order) and how many pixels it spans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LintItem {
    pub x: u32,
    pub y: u32,
    pub size: u32,
}

/// All findings of one kind: the real count and a capped list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LintList {
    pub count: usize,
    pub items: Vec<LintItem>,
}

impl LintList {
    fn push(&mut self, i: usize, w: usize, size: usize, cap: usize) {
        self.count += 1;
        if self.items.len() < cap {
            self.items.push(LintItem { x: (i % w) as u32, y: (i / w) as u32, size: size as u32 });
        }
    }
}

/// The report of one cel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LintReport {
    /// Light, grayish pixels on the outside of the sprite (paper halo): touching the transparent outside, or
    /// one ring in behind such a pixel. Light fill inside a dark outline is not listed.
    pub light_edge: LintList,
    /// Enclosed transparent areas.
    pub holes: LintList,
    /// Detached specks: small components apart from the body.
    pub specks: LintList,
    /// Edge pixels that are not outline and have no outline pixel next to them.
    pub no_outline: LintList,
    /// Single pixels that differ from all eight neighbours (isolated pixels are specks, not strays).
    pub strays: LintList,
    /// Pixels with 0 < alpha < 255.
    pub semi_transparent: LintList,
    /// Opaque colours missing from the palette (empty unless `check_palette`).
    pub off_palette: LintList,
}

impl LintReport {
    /// The categories with their report names, in report order.
    pub fn categories(&self) -> [(&'static str, &LintList); 7] {
        [
            ("light_edge", &self.light_edge),
            ("holes", &self.holes),
            ("specks", &self.specks),
            ("no_outline", &self.no_outline),
            ("strays", &self.strays),
            ("semi_transparent", &self.semi_transparent),
            ("off_palette", &self.off_palette),
        ]
    }

    pub fn total(&self) -> usize {
        self.categories().iter().map(|(_, l)| l.count).sum()
    }
}

/// Inspects one cel without changing it. `palette` is used when `o.check_palette` is set.
pub fn lint_cel(px: &[u8], w: usize, h: usize, o: &LintOptions, palette: &[[u8; 4]]) -> LintReport {
    let mut r = LintReport::default();
    if px.len() != w * h * 4 || w == 0 || h == 0 {
        return r;
    }
    let cap = o.max_list;
    let solid = solid_mask(px);
    let outside = outside_mask(&solid, w, h);
    let depth = ring_depth(&solid, &outside, w, h, o.halo.rings.max(2));
    let outline = outline_mask(px, &o.outline);

    for i in 0..w * h {
        if !solid[i] {
            continue;
        }
        let (x, y) = (i % w, i / w);
        let p = &px[i * 4..i * 4 + 4];
        if p[3] < 255 {
            r.semi_transparent.push(i, w, 1, cap);
        }
        // Paper halo sits outside the outline: a light pixel touching the outside, or a second-ring light
        // pixel right behind one. A light fill behind a dark outline is the drawing, not a halo.
        let halo_edge = halo_like(p, &o.halo)
            && (depth[i] == 1
                || (depth[i] == 2
                    && N4.iter().any(|&d| {
                        nb(w, h, x, y, d).is_some_and(|j| depth[j] == 1 && halo_like(&px[j * 4..j * 4 + 4], &o.halo))
                    })));
        if halo_edge {
            r.light_edge.push(i, w, 1, cap);
        }
        if depth[i] == 1 && !outline[i] && !N8.iter().any(|&d| nb(w, h, x, y, d).is_some_and(|j| outline[j])) {
            r.no_outline.push(i, w, 1, cap);
        }
        let mut neighbours = 0;
        let mut unlike = true;
        for d in N8 {
            if let Some(j) = nb(w, h, x, y, d) {
                if solid[j] {
                    neighbours += 1;
                }
                if px[j * 4..j * 4 + 4] == *p {
                    unlike = false;
                }
            }
        }
        if neighbours > 0 && unlike {
            r.strays.push(i, w, 1, cap);
        }
        if o.check_palette && p[3] > 0 && !palette.iter().any(|c| c[3] > 0 && c[..3] == p[..3]) {
            r.off_palette.push(i, w, 1, cap);
        }
    }
    let (_, holes) = holes_of(&solid, w, h);
    for m in &holes {
        r.holes.push(m[0], w, m.len(), cap);
    }
    if o.speck_size > 0 {
        let mut specks = loose_components(&solid, w, h, o.speck_size as usize);
        specks.sort_by_key(|m| m.iter().copied().min());
        for m in specks {
            r.specks.push(m.iter().copied().min().unwrap_or(0), w, m.len(), cap);
        }
    }
    r
}

/// The report of one cel of a sprite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CelLint {
    pub layer: usize,
    pub frame: usize,
    pub report: LintReport,
}

/// Lints the cels chosen by `target` (the same choice as the filters). Group layers and missing cels are skipped.
pub fn lint_sprite(sprite: &Sprite, o: &LintOptions, target: Target, layer: usize, frame: usize) -> Vec<CelLint> {
    let (w, h) = (sprite.width as usize, sprite.height as usize);
    let cells: Vec<(usize, usize)> = match target {
        Target::Cel => vec![(layer, frame)],
        Target::AllFrames => (0..sprite.frame_count()).map(|f| (layer, f)).collect(),
        Target::AllLayers => (0..sprite.layers.len()).map(|l| (l, frame)).collect(),
    };
    let mut out = Vec::new();
    for (l, f) in cells {
        if sprite.layers.get(l).is_none_or(|ly| ly.is_group()) {
            continue;
        }
        let Some(Some(cel)) = sprite.cels.get(f).and_then(|row| row.get(l)) else { continue };
        if cel.pixels.len() != w * h * 4 {
            continue;
        }
        out.push(CelLint { layer: l, frame: f, report: lint_cel(&cel.pixels, w, h, o, &sprite.palette) });
    }
    out
}

/// The text form of a lint run used by the C ABI. A line `cel <layer> <frame>` starts a cel (0-based); a line
/// `<category> <count>` starts a category (see `LintReport::categories`); the lines below it are `x y size`.
pub fn lint_text(cels: &[CelLint]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for c in cels {
        let _ = writeln!(s, "cel {} {}", c.layer, c.frame);
        for (name, list) in c.report.categories() {
            let _ = writeln!(s, "{name} {}", list.count);
            for it in &list.items {
                let _ = writeln!(s, "{} {} {}", it.x, it.y, it.size);
            }
        }
    }
    s
}

// ---- C ABI -------------------------------------------------------------------------------

/// Lint options from an FFI parameter block (all optional, missing ones keep their defaults): 0 speck size,
/// 1 list cap per category, 2 check palette (0/1), 3 outline max luma, 4 fixed outline colour on/off,
/// 5 its colour (0xRRGGBBAA), 6 its tolerance.
pub fn lint_options_from_params(p: &[i32]) -> LintOptions {
    let int = |i: usize, d: u32, max: u32| p.get(i).map_or(d, |&v| v.clamp(0, max as i32) as u32);
    let mut o = LintOptions::default();
    o.speck_size = int(0, o.speck_size, 1024);
    o.max_list = int(1, o.max_list as u32, 100_000) as usize;
    o.check_palette = p.get(2).is_some_and(|&v| v != 0);
    o.outline.max_luma = int(3, o.outline.max_luma, 255);
    if p.get(4).is_some_and(|&v| v != 0) {
        let c = (p.get(5).copied().unwrap_or(0) as u32).to_be_bytes();
        o.outline.color = Some([c[0], c[1], c[2]]);
        o.outline.tolerance = int(6, 0, 255);
    }
    o
}

/// Lints the cels chosen by `target` (0 cel, 1 every frame of `layer`, 2 every layer of `frame`) without
/// touching the document. Returns the text report described at `lint_text` (release with
/// `pixmith_string_free`), or null on a bad argument.
///
/// # Safety
/// `doc` must be null or a live document; `p` must point to `n` ints (or be null with `n == 0`).
#[no_mangle]
pub unsafe extern "C" fn pixmith_sprite_lint(
    doc: *const crate::document::Editor,
    target: u32,
    layer: u32,
    frame: u32,
    p: *const i32,
    n: usize,
) -> *mut std::ffi::c_char {
    crate::ffi::guarded("pixmith_sprite_lint", std::ptr::null_mut(), || {
        let (Some(e), Some(target)) = (doc.as_ref(), Target::from_u32(target)) else { return std::ptr::null_mut() };
        let params: &[i32] = if p.is_null() || n == 0 { &[] } else { std::slice::from_raw_parts(p, n) };
        let o = lint_options_from_params(params);
        let cels = lint_sprite(&e.sprite, &o, target, layer as usize, frame as usize);
        std::ffi::CString::new(lint_text(&cels).replace('\0', "")).map_or(std::ptr::null_mut(), |c| c.into_raw())
    })
}

// ---- Tests -------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const T: [u8; 4] = [0, 0, 0, 0];
    const K: [u8; 4] = [20, 20, 30, 255]; // outline
    const R: [u8; 4] = [200, 60, 50, 255]; // body
    const S: [u8; 4] = [235, 190, 160, 255]; // skin
    const W: [u8; 4] = [250, 250, 250, 255]; // highlight / paper
    const P: [u8; 4] = [225, 225, 220, 255]; // paper halo

    /// Builds a w x h buffer from rows of characters.
    fn art(rows: &[&str], key: &[(char, [u8; 4])]) -> (Vec<u8>, usize, usize) {
        let h = rows.len();
        let w = rows[0].len();
        let mut px = Vec::with_capacity(w * h * 4);
        for r in rows {
            assert_eq!(r.len(), w);
            for c in r.chars() {
                let col = if c == '.' { T } else { key.iter().find(|(k, _)| *k == c).expect("unknown key").1 };
                px.extend_from_slice(&col);
            }
        }
        (px, w, h)
    }

    const KEY: &[(char, [u8; 4])] = &[('K', K), ('R', R), ('S', S), ('W', W), ('P', P)];

    fn get(px: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        at(px, y * w + x)
    }

    fn count_solid(px: &[u8]) -> usize {
        px.chunks_exact(4).filter(|p| p[3] > 0).count()
    }

    fn comps(px: &[u8], w: usize, h: usize) -> usize {
        components8(&solid_mask(px), w, h).1.len() - 1
    }

    fn hole_count(px: &[u8], w: usize, h: usize) -> usize {
        holes_of(&solid_mask(px), w, h).1.len()
    }

    /// A 12x12 creature: dark outline, red body, 2x1 skin hand touching the outline on the right, a 1 px
    /// white highlight, and a 2 px dark eye. All of them are small interior details.
    fn creature() -> (Vec<u8>, usize, usize) {
        art(
            &[
                "............",
                "...KKKKKK...",
                "..KRRRRRRK..",
                ".KRRRRWRRRK.",
                ".KRRKKRRRRK.",
                ".KRRRRRRRRKK",
                ".KRRRRRRRRKS",
                ".KRRRRRRRRK.",
                "..KRRRRRRK..",
                "...KKKKKK...",
                "............",
                "............",
            ],
            KEY,
        )
    }

    fn every_default(px: &[u8], w: usize, h: usize) -> Vec<Vec<u8>> {
        let mut out = vec![clean_edges(px, w, h, &CleanEdgesOptions::all())];
        for o in [
            CleanEdgesOptions { halo: Some(Default::default()), ..Default::default() },
            CleanEdgesOptions { despeckle: Some(Default::default()), ..Default::default() },
            CleanEdgesOptions { thin_outline: Some(Default::default()), ..Default::default() },
        ] {
            out.push(clean_edges(px, w, h, &o));
        }
        out
    }

    // ---- regression: small interior features survive everything ----

    #[test]
    fn eyes_highlights_and_fingers_survive_every_default_operation() {
        let (px, w, h) = creature();
        for (n, out) in every_default(&px, w, h).iter().enumerate() {
            assert_eq!(get(out, w, 4, 4), K, "eye pixel 1, op {n}");
            assert_eq!(get(out, w, 5, 4), K, "eye pixel 2, op {n}");
            assert_eq!(get(out, w, 6, 3), W, "1 px highlight, op {n}");
            assert_eq!(get(out, w, 11, 6), S, "1 px finger, op {n}");
            assert_eq!(get(out, w, 10, 5), K, "outline next to the finger, op {n}");
            assert_eq!(out, &px, "the creature is already clean, op {n}");
        }
    }

    #[test]
    fn a_thin_finger_touching_the_outline_survives_even_when_light() {
        // A 1 px light finger and a 2 px one stick out of the body and touch its outline.
        let (px, w, h) =
            art(&["..........", ".KKKKKKK..", ".KRRRRRKPP", ".KRRRRRKK.", ".KRRRRRK..", ".KKKKKKK.."], KEY);
        for (n, out) in every_default(&px, w, h).iter().enumerate() {
            assert_eq!(get(out, w, 8, 2), P, "finger, op {n}");
            assert_eq!(get(out, w, 9, 2), P, "finger tip, op {n}");
        }
    }

    #[test]
    fn small_details_survive_inside_a_dark_blob_too() {
        // A 1 px highlight and a 2 px eye in a small face next to a black border.
        let (px, w, h) = art(&["KKKKKKK", "KSSSSSK", "KSKKSWK", "KSSSSSK", "KKKKKKK"], KEY);
        for (n, out) in every_default(&px, w, h).iter().enumerate() {
            assert_eq!(out, &px, "op {n}");
        }
    }

    // ---- halo ----

    #[test]
    fn halo_two_rings_of_paper_are_removed() {
        let (mut px, w, h) = art(
            &[
                "..........",
                ".PPPPPPPP.",
                ".PWWWWWWP.",
                ".PWKKKKWP.",
                ".PWKRRKWP.",
                ".PWKRRKWP.",
                ".PWKKKKWP.",
                ".PWWWWWWP.",
                ".PPPPPPPP.",
                "..........",
            ],
            KEY,
        );
        assert!(remove_halo(&mut px, w, h, &HaloOptions::default()));
        for y in 1..=8 {
            for x in 1..=8 {
                let band = x <= 2 || x >= 7 || y <= 2 || y >= 7;
                assert_eq!(get(&px, w, x, y)[3] == 0, band, "({x},{y})");
            }
        }
        assert_eq!(get(&px, w, 3, 3), K);
        assert_eq!(get(&px, w, 4, 4), R);
    }

    #[test]
    fn halo_removes_only_the_requested_number_of_rings() {
        // Paper (P), then a lighter white ring (W), then black: the first ring alone is not "much lighter
        // than what is behind it", so one ring removes nothing, two rings remove both.
        let rows = [".......", ".PPPPP.", ".PWWWP.", ".PWKWP.", ".PWWWP.", ".PPPPP.", "......."];
        let (px, w, h) = art(&rows, KEY);
        let mut one = px.clone();
        assert!(!remove_halo(&mut one, w, h, &HaloOptions { rings: 1, min_group: 1, ..Default::default() }));
        let mut two = px.clone();
        assert!(remove_halo(&mut two, w, h, &HaloOptions { rings: 2, min_group: 1, ..Default::default() }));
        assert_eq!(get(&two, w, 1, 1)[3], 0);
        assert_eq!(get(&two, w, 2, 2)[3], 0);
        assert_eq!(get(&two, w, 3, 3), K);
        // A single ring of paper in front of black goes with rings = 1.
        let (mut ring, w, h) = art(&[".....", ".PPP.", ".PKP.", ".PPP.", "....."], KEY);
        assert!(remove_halo(&mut ring, w, h, &HaloOptions { rings: 1, min_group: 1, ..Default::default() }));
        assert_eq!(count_solid(&ring), 1);
    }

    #[test]
    fn halo_keeps_a_light_pixel_that_is_not_in_front_of_something_darker() {
        // A white body (a ghost) with no outline: nothing is darker behind its edge, so nothing is a halo.
        let (px, w, h) = art(&["......", ".WWWW.", ".WWWW.", ".WWWW.", ".WWWW.", "......"], KEY);
        let mut out = px.clone();
        assert!(!remove_halo(&mut out, w, h, &HaloOptions::default()));
        assert_eq!(out, px);
    }

    #[test]
    fn halo_keeps_a_lone_light_rim_pixel() {
        // One white highlight on the rim of a red body is a highlight, not a band of paper.
        let (px, w, h) = art(&["......", ".RRRR.", ".RWRR.", ".RRRR.", ".RRRR.", "......"], KEY);
        let mut out = px.clone();
        remove_halo(&mut out, w, h, &HaloOptions { rings: 2, ..Default::default() });
        assert_eq!(out, px);
    }

    #[test]
    fn halo_never_changes_topology_on_random_sprites() {
        // Random blobs in four colours: whatever the halo pass decides, the number of components and of
        // holes stays exactly as it was.
        let cols = [K, P, W, R];
        let mut seed = 0x2545_F491u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let (w, h) = (14usize, 12usize);
        let o = HaloOptions { min_group: 1, min_luma_gain: 10, ..Default::default() };
        let mut changed_any = false;
        for _ in 0..400 {
            let mut px = vec![0u8; w * h * 4];
            for i in 0..w * h {
                if next() % 100 < 62 {
                    px[i * 4..i * 4 + 4].copy_from_slice(&cols[(next() % 4) as usize]);
                }
            }
            let (c0, h0) = (comps(&px, w, h), hole_count(&px, w, h));
            let mut out = px.clone();
            changed_any |= remove_halo(&mut out, w, h, &o);
            assert_eq!((comps(&out, w, h), hole_count(&out, w, h)), (c0, h0));
            let mut thin = px.clone();
            thin_outline(&mut thin, w, h, &ThinOptions::default());
            assert_eq!((comps(&thin, w, h), hole_count(&thin, w, h)), (c0, h0));
        }
        assert!(changed_any, "the random sprites exercise the pass");
    }

    #[test]
    fn halo_does_not_remove_a_pixel_that_is_a_bridge() {
        // The paper pixel at (3,2) is the only link between two dark parts and has pixels behind it.
        let (mut px, w, h) = art(&["......", ".KK...", ".KKP..", ".KKKKK", "......"], KEY);
        let o = HaloOptions { min_group: 1, ..Default::default() };
        let before = comps(&px, w, h);
        remove_halo(&mut px, w, h, &o);
        assert_eq!(comps(&px, w, h), before);
    }

    #[test]
    fn halo_does_not_open_a_hole_in_a_ring() {
        // Paper completes a ring around a transparent pocket. Removing it would be fine topologically
        // only if the pocket stays closed.
        let (mut px, w, h) =
            art(&["........", ".KKKKKK.", ".KPPPPK.", ".KP..PK.", ".KPPPPK.", ".KKKKKK.", "........"], KEY);
        let o = HaloOptions { min_group: 1, ..Default::default() };
        remove_halo(&mut px, w, h, &o);
        // The pocket's paper lining touches a hole, not the outside: untouched.
        assert_eq!(get(&px, w, 2, 2), P);
        assert_eq!(hole_count(&px, w, h), 1);
    }

    // ---- despeckle ----

    #[test]
    fn despeckle_removes_small_detached_islands_but_not_the_body() {
        let (mut px, w, h) = art(
            &[
                "..........",
                ".RRRRR....",
                ".RRRRR..K.",
                ".RRRRR....",
                ".RRRRR....",
                "........KK",
                "..K.......",
                "..........",
            ],
            KEY,
        );
        let o = DespeckleOptions::default();
        assert!(despeckle(&mut px, w, h, &o, &HaloOptions::default()));
        assert_eq!(count_solid(&px), 20);
        assert_eq!(get(&px, w, 8, 2)[3], 0);
        assert_eq!(get(&px, w, 8, 5)[3], 0);
        assert_eq!(get(&px, w, 2, 6)[3], 0);
    }

    #[test]
    fn despeckle_keeps_a_pupil_attached_inside_a_socket() {
        let (px, w, h) = art(&["........", ".RRRRRR.", ".RR..RR.", ".RR.KRR.", ".RRRRRR.", "........"], KEY);
        let mut out = px.clone();
        despeckle(&mut out, w, h, &DespeckleOptions::default(), &HaloOptions::default());
        assert_eq!(out, px);
    }

    #[test]
    fn despeckle_keeps_a_pupil_that_is_not_connected_to_the_face() {
        // Enclosed by a ring of body but separated from it by transparency.
        let (px, w, h) =
            art(&[".........", ".RRRRRRR.", ".R.....R.", ".R..K..R.", ".R.....R.", ".RRRRRRR.", "........."], KEY);
        let mut out = px.clone();
        assert!(!despeckle(&mut out, w, h, &DespeckleOptions::default(), &HaloOptions::default()));
        assert_eq!(out, px);
    }

    #[test]
    fn despeckle_leaves_a_tiny_sprite_alone() {
        let (px, w, h) = art(&["....", ".K..", "....", "..K.", "...."], KEY);
        let mut out = px.clone();
        assert!(!despeckle(&mut out, w, h, &DespeckleOptions::default(), &HaloOptions::default()));
        assert_eq!(out, px);
    }

    #[test]
    fn despeckle_fills_tiny_holes_only_when_asked() {
        let (px, w, h) =
            art(&["........", ".RRRRRR.", ".RR.RRR.", ".RRRRRR.", ".RRR..R.", ".RRR..R.", ".RRRRRR.", "........"], KEY);
        let mut off = px.clone();
        assert!(!despeckle(&mut off, w, h, &DespeckleOptions::default(), &HaloOptions::default()));
        assert_eq!(hole_count(&off, w, h), 2);

        let mut on = px.clone();
        let o = DespeckleOptions { hole_size: 2, ..Default::default() };
        assert!(despeckle(&mut on, w, h, &o, &HaloOptions::default()));
        assert_eq!(get(&on, w, 3, 2), R, "the 1 px hole is filled with the surrounding colour");
        assert_eq!(hole_count(&on, w, h), 1, "the 2 px hole is not smaller than 2 and stays");
        assert_eq!(get(&on, w, 4, 4)[3], 0);
    }

    #[test]
    fn despeckle_removes_a_lone_light_pixel_inside_a_dark_area_when_asked() {
        let (px, w, h) = art(&["KKKKK", "KKKKK", "KKWKK", "KKKKK", "KKKKK"], KEY);
        let mut off = px.clone();
        assert!(!despeckle(&mut off, w, h, &DespeckleOptions::default(), &HaloOptions::default()));
        assert_eq!(off, px, "off by default: it is indistinguishable from a highlight");
        let mut on = px.clone();
        let o = DespeckleOptions { lone_light: true, ..Default::default() };
        assert!(despeckle(&mut on, w, h, &o, &HaloOptions::default()));
        assert_eq!(get(&on, w, 2, 2), K);
    }

    // ---- thin outline ----

    #[test]
    fn thin_outline_reduces_a_two_pixel_outline_to_one() {
        let (mut px, w, h) = art(
            &[
                "..........",
                ".KKKKKKKK.",
                ".KKKKKKKK.",
                ".KKRRRRKK.",
                ".KKRRRRKK.",
                ".KKRRRRKK.",
                ".KKRRRRKK.",
                ".KKKKKKKK.",
                ".KKKKKKKK.",
                "..........",
            ],
            KEY,
        );
        assert!(thin_outline(&mut px, w, h, &ThinOptions::default()));
        // The outline is now the layer next to the body; the body is untouched.
        for y in 2..=7 {
            for x in 2..=7 {
                let edge = x == 2 || x == 7 || y == 2 || y == 7;
                assert_eq!(get(&px, w, x, y), if edge { K } else { R }, "({x},{y})");
            }
        }
        for i in 0..10 {
            assert_eq!(get(&px, w, i, 0)[3], 0);
            assert_eq!(get(&px, w, 1, i)[3], 0);
            assert_eq!(get(&px, w, 8, i)[3], 0);
            assert_eq!(get(&px, w, i, 9)[3], 0);
        }
        assert_eq!(comps(&px, w, h), 1);
    }

    #[test]
    fn thin_outline_handles_three_pixel_outlines_and_leaves_thicker_ones() {
        let rows3 = [
            "...........",
            ".KKKKKKKKK.",
            ".KKKKKKKKK.",
            ".KKKKKKKKK.",
            ".KKKRRRKKK.",
            ".KKKRRRKKK.",
            ".KKKRRRKKK.",
            ".KKKKKKKKK.",
            ".KKKKKKKKK.",
            ".KKKKKKKKK.",
            "...........",
        ];
        let (mut px, w, h) = art(&rows3, KEY);
        assert!(thin_outline(&mut px, w, h, &ThinOptions::default()));
        assert_eq!(count_solid(&px), 5 * 5, "a 5x5 block: 1 px outline around the 3x3 body");
        let (mut thick, w2, h2) = art(&rows3, KEY);
        assert!(!thin_outline(&mut thick, w2, h2, &ThinOptions { max_thickness: 2, ..Default::default() }));
    }

    #[test]
    fn thin_outline_leaves_a_one_pixel_outline_and_a_pure_dark_sprite() {
        let (px, w, h) = art(&["......", ".KKKK.", ".KRRK.", ".KRRK.", ".KKKK.", "......"], KEY);
        let mut out = px.clone();
        assert!(!thin_outline(&mut out, w, h, &ThinOptions::default()));
        assert_eq!(out, px);
        let (px, w, h) = art(&["....", ".KK.", ".KK.", "...."], KEY);
        let mut out = px.clone();
        assert!(!thin_outline(&mut out, w, h, &ThinOptions::default()));
        assert_eq!(out, px);
    }

    #[test]
    fn thin_outline_keeps_eyes_that_touch_the_outline() {
        // The 2 px eye is the same dark colour as the 2 px outline and touches it.
        let (px, w, h) = art(
            &[
                "............",
                ".KKKKKKKKKK.",
                ".KKKKKKKKKK.",
                ".KKSSKKSSKK.",
                ".KKSSKKSSKK.",
                ".KKSSSSSSKK.",
                ".KKSSSSSSKK.",
                ".KKSSSSSSKK.",
                ".KKKKKKKKKK.",
                ".KKKKKKKKKK.",
                "............",
            ],
            KEY,
        );
        let mut out = px.clone();
        assert!(thin_outline(&mut out, w, h, &ThinOptions::default()));
        // Both eyes (rows 3-4, the "KK" pairs at x 5-6 inside the face) remain dark.
        for (x, y) in [(5, 3), (6, 3), (5, 4), (6, 4)] {
            assert_eq!(get(&out, w, x, y), K, "eye at ({x},{y})");
        }
        // The face is intact and the silhouette has no notch above the eyes.
        for x in 3..=8 {
            assert!(get(&out, w, x, 2)[3] > 0, "outline above the eyes at x={x}");
        }
        assert_eq!(comps(&out, w, h), 1);
        assert_eq!(hole_count(&out, w, h), 0);
    }

    #[test]
    fn thin_outline_does_not_cut_a_notch_between_a_cuff_and_a_fist() {
        // Two parts with a one pixel crevice between their 2 px outlines.
        let (px, w, h) = art(
            &[
                "..............",
                ".KKKKKK.KKKKKK",
                ".KKKKKK.KKKKKK",
                ".KSSSSKKKSSSSK",
                ".KSSSSK.KSSSSK",
                ".KSSSSK.KSSSSK",
                ".KKKKKK.KKKKKK",
                ".KKKKKK.KKKKKK",
                "..............",
            ],
            KEY,
        );
        let mut out = px.clone();
        thin_outline(&mut out, w, h, &ThinOptions::default());
        assert_eq!(comps(&out, w, h), comps(&px, w, h));
        assert_eq!(hole_count(&out, w, h), hole_count(&px, w, h));
        // The seam where the two outlines meet keeps its pixels.
        for y in 3..=3 {
            for x in 6..=8 {
                assert!(get(&out, w, x, y)[3] > 0, "joint ({x},{y})");
            }
        }
        // Everything that is not outline is untouched.
        for (i, p) in px.chunks_exact(4).enumerate() {
            if p == S {
                assert_eq!(at(&out, i), S);
            }
        }
    }

    #[test]
    fn thin_outline_never_opens_a_hole_or_detaches_a_strand() {
        // A 2 px outline ring around a window (a hole) with a 2 px dark strand hanging off it.
        let (px, w, h) = art(
            &[
                "............",
                ".KKKKKKKK...",
                ".KKKKKKKK...",
                ".KK....KK...",
                ".KK....KKKKK",
                ".KK....KK...",
                ".KKKKKKKK...",
                ".KKKKKKKK...",
                "............",
            ],
            KEY,
        );
        let mut out = px.clone();
        thin_outline(&mut out, w, h, &ThinOptions { max_thickness: 4, ..Default::default() });
        assert_eq!(comps(&out, w, h), 1);
        assert_eq!(hole_count(&out, w, h), 1);
    }

    #[test]
    fn thin_outline_respects_a_fixed_outline_colour() {
        // Dark brown hair on top of a light body: with the outline colour given, only that colour is thinned.
        let brown = [90u8, 40, 20, 255];
        let key = &[('K', K), ('S', S), ('B', brown)];
        let (mut px, w, h) =
            art(&["......", ".BBBB.", ".BBBB.", ".KKKK.", ".KKKK.", ".SSSS.", ".SSSS.", "......"], key);
        let o = ThinOptions { color: Some([20, 20, 30]), max_luma: 0, ..Default::default() };
        thin_outline(&mut px, w, h, &o);
        assert_eq!(get(&px, w, 1, 1), brown, "hair is not outline");
        assert_eq!(get(&px, w, 1, 3)[3], 255);
    }

    // ---- clean_edges as a whole ----

    #[test]
    fn clean_edges_cleans_a_traced_sprite_end_to_end() {
        // Concentric rectangles: paper, whiter paper, a 2 px outline, a red body with a dark mark; a dust
        // speck lies away from it.
        let (w, h) = (16usize, 14usize);
        let layers = [P, W, K, K];
        let mut px = vec![0u8; w * h * 4];
        for y in 1..=11 {
            for x in 1..=11 {
                let d = x.min(y).min(12 - x).min(12 - y) - 1; // 0 = outermost ring
                let c = layers.get(d).copied().unwrap_or(R);
                px[(y * w + x) * 4..][..4].copy_from_slice(&c);
            }
        }
        px[(6 * w + 6) * 4..][..4].copy_from_slice(&K); // a dark mark inside the body
        px[(7 * w + 6) * 4..][..4].copy_from_slice(&K);
        px[(2 * w + 14) * 4..][..4].copy_from_slice(&K); // dust
        let out = clean_edges(&px, w, h, &CleanEdgesOptions::all());
        assert_eq!(get(&out, w, 14, 2)[3], 0, "speck gone");
        for i in 1..=11 {
            assert_eq!(get(&out, w, i, 1)[3], 0, "paper gone");
            assert_eq!(get(&out, w, i, 2)[3], 0, "second paper ring gone");
            assert_eq!(get(&out, w, 1, i)[3], 0);
            assert_eq!(get(&out, w, 2, i)[3], 0);
        }
        // Of the 2 px outline only the layer next to the body is left; the body and its mark are intact.
        for i in 4..=8 {
            assert_eq!(get(&out, w, 4, i), K);
            assert_eq!(get(&out, w, 8, i), K);
            assert_eq!(get(&out, w, i, 4), K);
        }
        assert_eq!(get(&out, w, 3, 6)[3], 0, "outer outline layer thinned away");
        assert_eq!(get(&out, w, 5, 5), R);
        assert_eq!(get(&out, w, 6, 6), K);
        assert_eq!(get(&out, w, 6, 7), K);
        assert_eq!(comps(&out, w, h), 1);
    }

    #[test]
    fn clean_edges_with_nothing_enabled_is_a_copy() {
        let (px, w, h) = creature();
        assert_eq!(clean_edges(&px, w, h, &CleanEdgesOptions::default()), px);
    }

    // ---- through the filter pipeline: cels, frames, selection ----

    fn sprite_from(px: Vec<u8>, w: usize, h: usize) -> Sprite {
        let mut s = Sprite::new(w as u32, h as u32);
        s.cels[0][0] =
            Some(std::sync::Arc::new(crate::document::Cel { opacity: 255, pixels: px, ..Default::default() }));
        s
    }

    #[test]
    fn filter_applies_to_all_frames_and_is_one_pass_per_cel() {
        use crate::paint::filters::{apply, Filter};
        let (px, w, h) = art(&[".......", ".RRR...", ".RRR...", ".RRR...", ".......", "......K"], KEY);
        let mut s = sprite_from(px.clone(), w, h);
        s.add_frame(0, true);
        let f = Filter::CleanEdges(CleanEdgesOptions::all());
        assert!(apply(&mut s, &f, Target::AllFrames, 0, 0, None));
        for fr in 0..2 {
            let cel = s.cels[fr][0].as_ref().unwrap();
            assert_eq!(cel.pixels[(5 * w + 6) * 4 + 3], 0, "speck gone in frame {fr}");
            assert_eq!(cel.pixels[(w + 1) * 4 + 3], 255);
        }
        // Nothing left to clean: no change reported.
        assert!(!apply(&mut s, &f, Target::AllFrames, 0, 0, None));
    }

    #[test]
    fn filter_params_round_trip_and_defaults() {
        use crate::paint::filters::{Filter, KIND_CLEAN_EDGES};
        let all = Filter::from_params(KIND_CLEAN_EDGES, &[]).unwrap();
        assert_eq!(all, Filter::CleanEdges(CleanEdgesOptions::all()));
        let f = Filter::from_params(KIND_CLEAN_EDGES, &[2 | 8, 1, 50, 20, 6, 3, 4, 60, 0, 0, 0]).unwrap();
        let Filter::CleanEdges(o) = f else { panic!() };
        assert_eq!(o.halo, None);
        let d = o.despeckle.unwrap();
        assert_eq!((d.min_size, d.hole_size, d.lone_light), (6, 3, true));
        assert_eq!(o.thin_outline, None);
    }

    // ---- lint ----

    #[test]
    fn lint_reports_each_category_with_positions_and_changes_nothing() {
        let (mut px, w, h) = art(
            &[
                "...........",
                ".PPPPPPPPP.",
                ".PKKKKKKKP.",
                ".PKRRRRRKP.",
                ".PKR.RWRKP.",
                ".PKRRRRRKP.",
                ".PKKKKKKKP.",
                ".PPPPPPPPP.",
                "...........",
                "..........K",
            ],
            KEY,
        );
        // One semi-transparent pixel and one colour that is not in the palette.
        let i = 3 * w + 4;
        px[i * 4 + 3] = 128;
        let before = px.clone();
        let pal = [K, R, S, W, P];
        let o = LintOptions { check_palette: true, ..Default::default() };
        let r = lint_cel(&px, w, h, &o, &pal);
        assert_eq!(px, before);
        assert_eq!(r.holes.count, 1);
        assert_eq!(r.holes.items[0], LintItem { x: 4, y: 4, size: 1 });
        assert_eq!(r.specks.count, 1);
        assert_eq!(r.specks.items[0], LintItem { x: 10, y: 9, size: 1 });
        assert_eq!(r.semi_transparent.items, vec![LintItem { x: 4, y: 3, size: 1 }]);
        assert!(r.light_edge.count >= 20, "the paper band is listed");
        assert_eq!(r.off_palette.count, 0);
        let odd = [1u8, 2, 3, 255];
        px[(5 * w + 3) * 4..(5 * w + 3) * 4 + 4].copy_from_slice(&odd);
        let r = lint_cel(&px, w, h, &o, &pal);
        assert_eq!(r.off_palette.items, vec![LintItem { x: 3, y: 5, size: 1 }]);
        assert!(r.strays.items.iter().any(|s| (s.x, s.y) == (3, 5)), "the odd pixel is also a stray");
    }

    #[test]
    fn lint_lists_are_capped_but_counts_are_complete() {
        let (px, w, h) = art(&["......", ".PPPP.", ".PPPP.", ".PPPP.", "......"], KEY);
        let o = LintOptions { max_list: 3, ..Default::default() };
        let r = lint_cel(&px, w, h, &o, &[]);
        assert_eq!(r.light_edge.count, 12);
        assert_eq!(r.light_edge.items.len(), 3);
    }

    #[test]
    fn lint_light_edge_ignores_light_fill_inside_a_dark_outline() {
        // A cream body behind a dark outline is the drawing, not paper.
        let (px, w, h) = art(&[".......", ".KKKKK.", ".KWWWK.", ".KWWWK.", ".KWWWK.", ".KKKKK.", "......."], KEY);
        assert_eq!(lint_cel(&px, w, h, &LintOptions::default(), &[]).light_edge.count, 0);
        // The same body with paper around the outline: only the paper is listed.
        let (px, w, h) =
            art(&[".........", ".PPPPPPP.", ".PKKKKKP.", ".PKWWWKP.", ".PKKKKKP.", ".PPPPPPP.", "........."], KEY);
        let r = lint_cel(&px, w, h, &LintOptions::default(), &[]);
        assert_eq!(r.light_edge.count, 20);
        assert!(r.light_edge.items.iter().all(|i| i.x == 1 || i.x == 7 || i.y == 1 || i.y == 5));
    }

    #[test]
    fn lint_flags_edge_pixels_with_no_outline_next_to_them() {
        let (px, w, h) = art(&["........", ".KKKK...", ".KRRK...", ".KRRRRR.", ".KKKK...", "........"], KEY);
        let r = lint_cel(&px, w, h, &LintOptions::default(), &[]);
        assert_eq!(r.no_outline.count, 1);
        assert_eq!((r.no_outline.items[0].x, r.no_outline.items[0].y), (6, 3));
    }

    #[test]
    fn lint_of_a_clean_creature_finds_nothing_but_the_expected() {
        let (px, w, h) = creature();
        let r = lint_cel(&px, w, h, &LintOptions::default(), &[]);
        assert_eq!(r.holes.count, 0);
        assert_eq!(r.specks.count, 0);
        assert_eq!(r.semi_transparent.count, 0);
        assert_eq!(r.light_edge.count, 0);
    }

    #[test]
    fn lint_sprite_and_text_report() {
        let (px, w, h) = art(&[".....", ".RR..", ".RR..", ".....", "....K"], KEY);
        let mut s = sprite_from(px, w, h);
        s.add_frame(0, true);
        let cels = lint_sprite(&s, &LintOptions::default(), Target::AllFrames, 0, 0);
        assert_eq!(cels.len(), 2);
        let text = lint_text(&cels);
        assert!(text.starts_with("cel 0 0\nlight_edge 0\nholes 0\nspecks 1\n4 4 1\n"), "{text}");
        assert!(text.contains("cel 0 1\n"));
    }

    #[test]
    fn c_abi_applies_one_undo_step_and_lints() {
        use crate::paint::filters::{pixmith_filter_apply, KIND_CLEAN_EDGES};
        let (px, w, h) = art(&[".......", ".RRR...", ".RRR...", ".RRR...", ".......", "......K"], KEY);
        let mut ed = Box::new(crate::document::Editor::new(sprite_from(px.clone(), w, h)));
        let doc: *mut crate::document::Editor = &mut *ed;
        unsafe {
            let p = [2i32];
            assert!(pixmith_filter_apply(doc, KIND_CLEAN_EDGES, 0, 0, 0, 0, 0, 0, 0, p.as_ptr(), p.len()));
            assert_eq!((&*doc).sprite.cels[0][0].as_ref().unwrap().pixels[(5 * w + 6) * 4 + 3], 0);
            // Nothing left to clean: no second history step.
            assert!(!pixmith_filter_apply(doc, KIND_CLEAN_EDGES, 0, 0, 0, 0, 0, 0, 0, p.as_ptr(), p.len()));
            assert!(crate::ffi::document::pixmith_undo(doc));
            assert_eq!((&*doc).sprite.cels[0][0].as_ref().unwrap().pixels, px);
            assert!(!crate::ffi::document::pixmith_undo(doc), "one step only");

            let text = pixmith_sprite_lint(doc, 0, 0, 0, std::ptr::null(), 0);
            assert!(!text.is_null());
            let s = std::ffi::CStr::from_ptr(text).to_str().unwrap().to_string();
            crate::ffi::pixmith_string_free(text);
            assert!(s.contains("specks 1\n6 5 1\n"), "{s}");
            assert!(pixmith_sprite_lint(doc, 9, 0, 0, std::ptr::null(), 0).is_null());
        }
    }

    #[test]
    fn simple_point_basics() {
        // End of a line: simple. Middle of a line: not. Isolated pixel: not.
        let (px, w, h) = art(&[".....", ".KKK.", "....."], KEY);
        let set = solid_mask(&px);
        assert!(simple(&set, w, h, 1, 1));
        assert!(!simple(&set, w, h, 2, 1));
        let (px, w, h) = art(&[".....", "..K..", "....."], KEY);
        assert!(!simple(&solid_mask(&px), w, h, 2, 1));
        // A pixel of a 2x2 block is simple.
        let (px, w, h) = art(&["......", ".KK...", ".KK...", "......"], KEY);
        assert!(simple(&solid_mask(&px), w, h, 1, 1));
    }
}
