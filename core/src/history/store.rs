//! Compact storage for undo and redo history.
//!
//! A history chain is a list of snapshots ordered from the oldest to the one nearest the live
//! document. Only the nearest snapshot is kept whole (`Entry::Full`, sharing its cel buffers
//! with the live sprite). When a newer snapshot arrives, the previous nearest one is turned into
//! a [`Compact`]: everything small is kept as is, but each cel buffer that differs from the one
//! at the same place in the newer state becomes a *delta*, the bounding rectangle of the changed
//! pixels with their old values run-length encoded. Cels that did not change are not stored at
//! all (they are taken from the newer state when needed). Memory therefore grows with what
//! actually changed, not with the size of the cels.
//!
//! A compact entry is relative to the content of the entry right after it in the chain, so it
//! is turned back into a whole sprite ([`Compact::materialize`]) only when that neighbor is
//! known, which is the case exactly when the neighbor has just become the live document.

use crate::document::slices::UserData;
use crate::document::tilemap::TileGrid;
use crate::document::{Cel, OffCanvas, Sprite};
use crate::paint::selection::Selection;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, OnceLock};

// MARK: Region deltas

/// The old values of the rectangle in which two equally sized 2D arrays of `es`-byte elements
/// differ. Applying it to the newer array restores the older one.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RegionDelta {
    x0: u32,
    y0: u32,
    w: u32,
    h: u32,
    es: u8,
    payload: Payload,
}

#[derive(Clone, Debug, PartialEq)]
enum Payload {
    Raw(Vec<u8>),
    /// (run length, element value) pairs over the rectangle in row-major order.
    Rle(Vec<(u32, u32)>),
}

fn element(chunk: &[u8]) -> u32 {
    let mut b = [0u8; 4];
    b[..chunk.len()].copy_from_slice(chunk);
    u32::from_le_bytes(b)
}

impl RegionDelta {
    /// A delta that changes nothing.
    fn empty(es: usize) -> Self {
        RegionDelta { x0: 0, y0: 0, w: 0, h: 0, es: es as u8, payload: Payload::Raw(Vec::new()) }
    }

    /// The region where `old` differs from `new` (both `es`-byte elements, rows `width` elements
    /// wide), or `None` if they are identical. Arrays that are not a whole number of rows are
    /// treated as a single row.
    fn diff(old: &[u8], new: &[u8], es: usize, width: usize) -> Option<RegionDelta> {
        if old.len() != new.len() || old.is_empty() || !old.len().is_multiple_of(es) {
            return None;
        }
        let total = old.len() / es;
        let width = if width == 0 || !total.is_multiple_of(width) { total } else { width };
        let rows = total / width;
        let row_bytes = width * es;
        let differs = |r: usize| old[r * row_bytes..(r + 1) * row_bytes] != new[r * row_bytes..(r + 1) * row_bytes];
        let y0 = (0..rows).find(|&r| differs(r))?;
        let y1 = (y0..rows).rev().find(|&r| differs(r))?;
        let (mut x0, mut x1) = (width, 0);
        for r in y0..=y1 {
            let (a, b) = (&old[r * row_bytes..(r + 1) * row_bytes], &new[r * row_bytes..(r + 1) * row_bytes]);
            if a == b {
                continue;
            }
            let first = a.iter().zip(b).position(|(p, q)| p != q).unwrap_or(0) / es;
            let last = a.iter().zip(b).rposition(|(p, q)| p != q).unwrap_or(0) / es;
            x0 = x0.min(first);
            x1 = x1.max(last + 1);
        }
        let (w, h) = (x1 - x0, y1 - y0 + 1);
        Some(Self::capture(old, es, width, x0, y0, w, h))
    }

    /// The values currently in this delta's rectangle of `buf` (rows `width` elements wide), as a
    /// delta over the same rectangle.
    fn capture_from(&self, buf: &[u8], width: usize) -> RegionDelta {
        let es = self.es as usize;
        if self.w == 0 || self.h == 0 || es == 0 {
            return RegionDelta::empty(es.max(1));
        }
        let total = buf.len() / es;
        let width = if width == 0 || !total.is_multiple_of(width) { total } else { width };
        Self::capture(buf, es, width, self.x0 as usize, self.y0 as usize, self.w as usize, self.h as usize)
    }

    /// Copies a rectangle of `buf` and run-length encodes it when that is smaller.
    fn capture(buf: &[u8], es: usize, width: usize, x0: usize, y0: usize, w: usize, h: usize) -> RegionDelta {
        let mut raw = Vec::with_capacity(w * h * es);
        for r in y0..y0 + h {
            let start = (r * width + x0) * es;
            raw.extend_from_slice(buf.get(start..start + w * es).unwrap_or(&[]));
        }
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for c in raw.chunks_exact(es) {
            let v = element(c);
            match runs.last_mut() {
                Some((n, last)) if *last == v && *n < u32::MAX => *n += 1,
                _ => runs.push((1, v)),
            }
        }
        let payload = if runs.len() * 8 < raw.len() { Payload::Rle(runs) } else { Payload::Raw(raw) };
        RegionDelta { x0: x0 as u32, y0: y0 as u32, w: w as u32, h: h as u32, es: es as u8, payload }
    }

    /// Writes the stored old values into `dst` (the newer array), whose rows are `width` elements.
    fn apply(&self, dst: &mut [u8], width: usize) {
        let es = self.es as usize;
        if self.w == 0 || self.h == 0 || es == 0 {
            return;
        }
        let total = dst.len() / es;
        let width = if width == 0 || !total.is_multiple_of(width) { total } else { width };
        let raw: std::borrow::Cow<[u8]> = match &self.payload {
            Payload::Raw(r) => std::borrow::Cow::Borrowed(r),
            Payload::Rle(runs) => {
                let mut out = Vec::with_capacity(self.w as usize * self.h as usize * es);
                for &(n, v) in runs {
                    let bytes = v.to_le_bytes();
                    for _ in 0..n {
                        out.extend_from_slice(&bytes[..es]);
                    }
                }
                std::borrow::Cow::Owned(out)
            }
        };
        let (w, h) = (self.w as usize, self.h as usize);
        for r in 0..h {
            let d = ((self.y0 as usize + r) * width + self.x0 as usize) * es;
            let s = r * w * es;
            if let (Some(dst_row), Some(src_row)) = (dst.get_mut(d..d + w * es), raw.get(s..s + w * es)) {
                dst_row.copy_from_slice(src_row);
            }
        }
    }

    fn bytes(&self) -> usize {
        32 + match &self.payload {
            Payload::Raw(r) => r.len(),
            Payload::Rle(r) => r.len() * 8,
        }
    }
}

// MARK: Compact sprite snapshots

#[derive(Clone, Debug)]
enum GridChange {
    Same,
    Set(Option<TileGrid>),
}

#[derive(Clone, Debug)]
enum HintChange {
    /// The older cel has no hints.
    Absent,
    /// The same buffer as the newer cel.
    Same,
    Delta(RegionDelta),
    Full(Arc<Vec<u8>>),
}

/// How an older cel differs from the newer cel at the same place.
#[derive(Clone, Debug)]
struct CelDelta {
    opacity: u8,
    z_index: i16,
    user_data: UserData,
    tiles: GridChange,
    pixels: RegionDelta,
    indices: HintChange,
    /// The older cel's off-canvas part (an `Arc`, so keeping it is cheap).
    outside: Option<Arc<OffCanvas>>,
}

#[derive(Clone, Debug)]
enum Slot {
    Empty,
    /// The newer state has this very cel buffer at the same place.
    Same,
    /// The newer state has this very cel buffer at another place (frame, layer).
    At(u32, u32),
    /// The same buffer as the older state's slot at (frame, layer): linked cels stay linked.
    Alias(u32, u32),
    Delta(Box<CelDelta>),
    /// No usable counterpart (cel added, removed, resized): the buffer itself.
    Full(Arc<Cel>),
}

#[derive(Clone, Debug)]
enum TileSlot {
    Same,
    Delta(RegionDelta),
    Full(Arc<Vec<u8>>),
}

/// An older document state stored relative to a newer one (the next entry of its chain, or the
/// live document for the entry nearest to it).
#[derive(Clone, Debug)]
pub(crate) struct Compact {
    /// The older sprite without cel buffers and tile images.
    skeleton: Sprite,
    cels: Vec<Vec<Slot>>,
    tiles: Vec<Vec<TileSlot>>,
    /// Bytes of the delta data (not counting whole buffers in `Full` slots).
    bytes: usize,
}

impl Compact {
    /// Builds the compact form of `old` relative to `newer`, consuming `old` so that cel
    /// buffers nothing else references are freed.
    pub(crate) fn new(mut old: Sprite, newer: &Sprite) -> Compact {
        let width = old.width as usize;
        let cels = std::mem::take(&mut old.cels);
        let images: Vec<Vec<Arc<Vec<u8>>>> = old.tilesets.iter_mut().map(|ts| std::mem::take(&mut ts.tiles)).collect();
        let mut bytes = 0;
        let same_canvas = (old.width, old.height) == (newer.width, newer.height);
        let slots = compact_cels(cels, HashMap::new(), newer, width, same_canvas, &mut bytes);
        let tiles = compact_tiles(images, &old, newer, &mut bytes);
        Compact { skeleton: old, cels: slots, tiles, bytes }
    }

    /// Turns the live document (`live`, whose content is the "newer" state this entry was made
    /// against) into the older state this entry describes, and returns the entry that turns it
    /// back. Cel buffers that only the live document owns are patched in place, so the cost is
    /// proportional to the changed rectangles; shared buffers (linked cels, buffers some other
    /// holder still references) are copied instead.
    pub(crate) fn apply_in_place(self, live: &mut Sprite) -> Compact {
        let Compact { skeleton, cels: slots, tiles: tile_slots, bytes: _ } = self;
        let width = live.width as usize;
        // Take the live document's heavy parts; `live` becomes the older skeleton.
        let mut n_cels = std::mem::take(&mut live.cels);
        let n_images: Vec<Vec<Arc<Vec<u8>>>> =
            live.tilesets.iter_mut().map(|ts| std::mem::take(&mut ts.tiles)).collect();
        let n_skeleton = std::mem::replace(live, skeleton);

        // Positions whose newer cel another older slot refers to must not be patched in place.
        let at_targets: HashSet<(usize, usize)> = slots
            .iter()
            .flatten()
            .filter_map(|s| if let Slot::At(f, l) = s { Some((*f as usize, *l as usize)) } else { None })
            .collect();
        let mut inverse_at: HashMap<(usize, usize), Slot> = HashMap::new();
        let mut rows: Vec<Vec<Option<Arc<Cel>>>> = Vec::with_capacity(slots.len());
        for (f, row) in slots.into_iter().enumerate() {
            let mut out: Vec<Option<Arc<Cel>>> = Vec::with_capacity(row.len());
            for (l, slot) in row.into_iter().enumerate() {
                let newer_at = |cels: &Vec<Vec<Option<Arc<Cel>>>>, nf: usize, nl: usize| {
                    cels.get(nf).and_then(|r| r.get(nl)).and_then(|c| c.clone())
                };
                let cel = match slot {
                    Slot::Empty => None,
                    Slot::Same => newer_at(&n_cels, f, l),
                    Slot::At(nf, nl) => newer_at(&n_cels, nf as usize, nl as usize),
                    Slot::Alias(pf, pl) => {
                        if pf as usize == f {
                            out.get(pl as usize).cloned().flatten()
                        } else {
                            rows.get(pf as usize).and_then(|r| r.get(pl as usize)).cloned().flatten()
                        }
                    }
                    Slot::Full(arc) => Some(arc),
                    Slot::Delta(d) => {
                        let taken = if at_targets.contains(&(f, l)) {
                            None
                        } else {
                            n_cels.get_mut(f).and_then(|r| r.get_mut(l)).and_then(Option::take)
                        };
                        match taken {
                            Some(mut arc) => match Arc::get_mut(&mut arc) {
                                Some(cel) => {
                                    let inverse = Self::patch_in_place(cel, *d, width);
                                    inverse_at.insert((f, l), Slot::Delta(Box::new(inverse)));
                                    Some(arc)
                                }
                                None => {
                                    let rebuilt = Arc::new(Self::rebuild(*d, Some(&arc), width));
                                    n_cels[f][l] = Some(arc);
                                    Some(rebuilt)
                                }
                            },
                            None => Some(Arc::new(Self::rebuild(*d, newer_at(&n_cels, f, l).as_deref(), width))),
                        }
                    }
                };
                out.push(cel);
            }
            rows.push(out);
        }
        live.cels = rows;

        for (ti, slots) in tile_slots.into_iter().enumerate() {
            let tw = live.tilesets[ti].tile_w as usize;
            live.tilesets[ti].tiles = slots
                .into_iter()
                .enumerate()
                .map(|(i, slot)| {
                    let n = n_images.get(ti).and_then(|t| t.get(i));
                    match slot {
                        TileSlot::Same => n.cloned().unwrap_or_default(),
                        TileSlot::Full(a) => a,
                        TileSlot::Delta(d) => {
                            let mut v: Vec<u8> = n.map(|n| (**n).clone()).unwrap_or_default();
                            d.apply(&mut v, tw);
                            Arc::new(v)
                        }
                    }
                })
                .collect();
        }

        // The entry that undoes this: the old live content, relative to the new live document.
        let mut bytes = 0;
        let same_canvas = (n_skeleton.width, n_skeleton.height) == (live.width, live.height);
        let inverse_cels = compact_cels(n_cels, inverse_at, live, n_skeleton.width as usize, same_canvas, &mut bytes);
        let inverse_tiles = compact_tiles(n_images, &n_skeleton, live, &mut bytes);
        Compact { skeleton: n_skeleton, cels: inverse_cels, tiles: inverse_tiles, bytes }
    }

    /// Writes the older values of a delta into `cel` (a cel only the caller owns) and returns the
    /// delta that restores the values it had.
    fn patch_in_place(cel: &mut Cel, d: CelDelta, width: usize) -> CelDelta {
        let CelDelta { opacity, z_index, user_data, tiles, pixels, indices, outside } = d;
        let inverse_outside = std::mem::replace(&mut cel.outside, outside);
        let inverse_pixels = pixels.capture_from(&cel.pixels, width);
        pixels.apply(&mut cel.pixels, width);
        let inverse_opacity = std::mem::replace(&mut cel.opacity, opacity);
        let inverse_z = std::mem::replace(&mut cel.z_index, z_index);
        let inverse_user = std::mem::replace(&mut cel.user_data, user_data);
        let inverse_tiles = match tiles {
            GridChange::Same => GridChange::Same,
            GridChange::Set(g) => GridChange::Set(std::mem::replace(&mut cel.tiles, g)),
        };
        let inverse_hints = match indices {
            HintChange::Same => HintChange::Same,
            HintChange::Absent => cel.indices.take().map_or(HintChange::Absent, HintChange::Full),
            HintChange::Full(a) => cel.indices.replace(a).map_or(HintChange::Absent, HintChange::Full),
            HintChange::Delta(rd) => match cel.indices.as_mut() {
                Some(h) => {
                    let buf = Arc::make_mut(h);
                    let inverse = rd.capture_from(buf, width);
                    rd.apply(buf, width);
                    HintChange::Delta(inverse)
                }
                None => HintChange::Absent,
            },
        };
        CelDelta {
            opacity: inverse_opacity,
            z_index: inverse_z,
            user_data: inverse_user,
            tiles: inverse_tiles,
            pixels: inverse_pixels,
            indices: inverse_hints,
            outside: inverse_outside,
        }
    }

    /// The older cel at (frame, layer), built from the newer document it is stored against.
    /// Number of tiles tileset `ts` had in the state this entry describes.
    pub(crate) fn tile_count(&self, ts: usize) -> Option<usize> {
        self.tiles.get(ts).map(Vec::len)
    }

    pub(crate) fn cel_at(&self, newer: &Sprite, f: usize, l: usize) -> Option<Arc<Cel>> {
        let newer_at = |nf: usize, nl: usize| newer.cels.get(nf).and_then(|r| r.get(nl)).and_then(|c| c.clone());
        match self.cels.get(f)?.get(l)? {
            Slot::Empty => None,
            Slot::Same => newer_at(f, l),
            Slot::At(nf, nl) => newer_at(*nf as usize, *nl as usize),
            Slot::Alias(pf, pl) => self.cel_at(newer, *pf as usize, *pl as usize),
            Slot::Full(a) => Some(a.clone()),
            Slot::Delta(d) => {
                Some(Arc::new(Self::rebuild((**d).clone(), newer_at(f, l).as_deref(), self.skeleton.width as usize)))
            }
        }
    }

    /// Builds the older cel from a delta by copying the newer cel.
    fn rebuild(d: CelDelta, newer: Option<&Cel>, width: usize) -> Cel {
        let CelDelta { opacity, z_index, user_data, tiles, pixels, indices, outside } = d;
        let mut px = newer.map(|n| n.pixels.clone()).unwrap_or_default();
        pixels.apply(&mut px, width);
        let grid = match tiles {
            GridChange::Same => newer.and_then(|n| n.tiles.clone()),
            GridChange::Set(g) => g,
        };
        let hints = match indices {
            HintChange::Absent => None,
            HintChange::Same => newer.and_then(|n| n.indices.clone()),
            HintChange::Full(a) => Some(a),
            HintChange::Delta(d) => newer.and_then(|n| n.indices.as_ref()).map(|b| {
                let mut v = (**b).clone();
                d.apply(&mut v, width);
                Arc::new(v)
            }),
        };
        Cel { opacity, pixels: px, tiles: grid, user_data, indices: hints, z_index, outside }
    }

    /// Calls `f(address, bytes)` for the memory this entry holds: its own delta data and every
    /// whole buffer it keeps.
    pub(crate) fn for_each_buffer(&self, mut f: impl FnMut(usize, usize)) {
        f(self as *const Compact as usize, self.bytes);
        for slot in self.cels.iter().flatten() {
            match slot {
                Slot::Full(a) => {
                    f(Arc::as_ptr(a) as usize, a.pixels.len() + a.outside.as_ref().map_or(0, |o| o.rgba.len()))
                }
                Slot::Delta(d) => {
                    if let HintChange::Full(a) = &d.indices {
                        f(Arc::as_ptr(a) as usize, a.len());
                    }
                    if let Some(o) = &d.outside {
                        f(Arc::as_ptr(o) as usize, o.rgba.len());
                    }
                }
                _ => {}
            }
        }
        for slot in self.tiles.iter().flatten() {
            if let TileSlot::Full(a) = slot {
                f(Arc::as_ptr(a) as usize, a.len());
            }
        }
    }
}

/// Slots describing `cels` relative to `newer`. `overrides` supplies ready-made slots for
/// positions whose cel was already consumed (patched in place into the other state).
fn compact_cels(
    cels: Vec<Vec<Option<Arc<Cel>>>>,
    mut overrides: HashMap<(usize, usize), Slot>,
    newer: &Sprite,
    width: usize,
    same_canvas: bool,
    bytes: &mut usize,
) -> Vec<Vec<Slot>> {
    let mut in_newer: HashMap<*const Cel, (u32, u32)> = HashMap::new();
    for (f, row) in newer.cels.iter().enumerate() {
        for (l, c) in row.iter().enumerate() {
            if let Some(c) = c {
                in_newer.entry(Arc::as_ptr(c)).or_insert((f as u32, l as u32));
            }
        }
    }
    let mut seen: HashMap<*const Cel, (u32, u32)> = HashMap::new();
    let mut slots: Vec<Vec<Slot>> = Vec::with_capacity(cels.len());
    for (f, row) in cels.into_iter().enumerate() {
        let mut out_row = Vec::with_capacity(row.len());
        for (l, cel) in row.into_iter().enumerate() {
            if let Some(slot) = overrides.remove(&(f, l)) {
                if let Slot::Delta(d) = &slot {
                    *bytes += d.pixels.bytes() + std::mem::size_of::<CelDelta>();
                }
                out_row.push(slot);
                continue;
            }
            let slot = match cel {
                None => Slot::Empty,
                Some(arc) => {
                    let key = Arc::as_ptr(&arc);
                    if let Some(&(pf, pl)) = seen.get(&key) {
                        Slot::Alias(pf, pl)
                    } else {
                        seen.insert(key, (f as u32, l as u32));
                        slot_for(arc, newer, &in_newer, (f, l), width, same_canvas, bytes)
                    }
                }
            };
            out_row.push(slot);
        }
        *bytes += out_row.len() * 24;
        slots.push(out_row);
    }
    slots
}

fn slot_for(
    arc: Arc<Cel>,
    newer: &Sprite,
    in_newer: &HashMap<*const Cel, (u32, u32)>,
    (f, l): (usize, usize),
    width: usize,
    same_canvas: bool,
    bytes: &mut usize,
) -> Slot {
    let n = newer.cels.get(f).and_then(|r| r.get(l)).and_then(|c| c.as_ref());
    if n.is_some_and(|n| Arc::ptr_eq(n, &arc)) {
        return Slot::Same;
    }
    if let Some(&(nf, nl)) = in_newer.get(&Arc::as_ptr(&arc)) {
        return Slot::At(nf, nl);
    }
    // Deltas are rectangles on one canvas: after a resize (even to another size with the same
    // pixel count) the buffer is kept whole.
    let Some(n) = n.filter(|n| same_canvas && n.pixels.len() == arc.pixels.len()) else { return Slot::Full(arc) };
    // Exhaustive on purpose: a new `Cel` field must be handled here before this compiles.
    let Cel { opacity, pixels, tiles, user_data, indices, z_index, outside } = &*arc;
    let pixel_delta = RegionDelta::diff(pixels, &n.pixels, 4, width).unwrap_or_else(|| RegionDelta::empty(4));
    *bytes += pixel_delta.bytes() + std::mem::size_of::<CelDelta>();
    let grid = if *tiles == n.tiles {
        GridChange::Same
    } else {
        *bytes += tiles.as_ref().map_or(0, |g| g.tiles.len() * 4 + 32);
        GridChange::Set(tiles.clone())
    };
    *bytes += user_data.text.as_ref().map_or(0, String::len) + user_data.props.as_ref().map_or(0, Vec::len);
    let hints = match (indices, &n.indices) {
        (None, _) => HintChange::Absent,
        (Some(a), Some(b)) if Arc::ptr_eq(a, b) => HintChange::Same,
        (Some(a), Some(b)) if a.len() == b.len() => {
            let d = RegionDelta::diff(a, b, 1, width).unwrap_or_else(|| RegionDelta::empty(1));
            *bytes += d.bytes();
            HintChange::Delta(d)
        }
        (Some(a), _) => HintChange::Full(a.clone()),
    };
    let slot = Slot::Delta(Box::new(CelDelta {
        opacity: *opacity,
        z_index: *z_index,
        user_data: user_data.clone(),
        tiles: grid,
        pixels: pixel_delta,
        indices: hints,
        outside: outside.clone(),
    }));
    // The old buffer is now described by the delta; if nothing else holds it, its memory serves
    // the next copy-on-write of a cel this size.
    if let Ok(cel) = Arc::try_unwrap(arc) {
        crate::history::buffer_pool::recycle(cel.pixels);
    }
    slot
}

/// Tile image slots for `images` (the tile images of the sprite `owner`) relative to `newer`.
fn compact_tiles(
    images: Vec<Vec<Arc<Vec<u8>>>>,
    owner: &Sprite,
    newer: &Sprite,
    bytes: &mut usize,
) -> Vec<Vec<TileSlot>> {
    let mut tiles = Vec::with_capacity(images.len());
    for (ti, imgs) in images.into_iter().enumerate() {
        let tw = owner.tilesets[ti].tile_w as usize;
        let mut out = Vec::with_capacity(imgs.len());
        for (i, img) in imgs.into_iter().enumerate() {
            let n = newer.tilesets.get(ti).and_then(|t| t.tiles.get(i));
            out.push(match n {
                Some(n) if Arc::ptr_eq(n, &img) => TileSlot::Same,
                Some(n) if n.len() == img.len() => match RegionDelta::diff(&img, n, 4, tw) {
                    Some(d) => {
                        *bytes += d.bytes();
                        TileSlot::Delta(d)
                    }
                    None => TileSlot::Delta(RegionDelta::empty(4)),
                },
                _ => TileSlot::Full(img),
            });
        }
        *bytes += out.len() * 16;
        tiles.push(out);
    }
    tiles
}

/// One snapshot in a history chain.
pub(crate) enum Entry {
    /// Complete and independent of the live document: a checkpoint taken just now, sharing its
    /// cel buffers with the live sprite until the next edit copies them.
    Full(Sprite),
    /// Stored relative to the next entry of the chain (nearer the live document); the entry
    /// nearest the live document is relative to the live document itself, which is only valid
    /// while the live buffers are the ones recorded in the editor's [`Witness`].
    Compact(Compact),
    /// A whole snapshot being compacted on the history thread. Until that finishes it is used
    /// exactly like `Full` (compaction only saves memory), so nothing ever waits for it on the
    /// editing path.
    Pending(Pending),
}

impl Entry {
    /// The whole snapshot, for entries that hold one (`Full`, and `Pending` until it settles).
    pub(crate) fn whole(&self) -> Option<&Sprite> {
        match self {
            Entry::Full(s) => Some(s),
            Entry::Pending(p) => p.prev.as_ref(),
            Entry::Compact(_) => None,
        }
    }

    /// Takes the whole snapshot out of a `Full` or `Pending` entry.
    pub(crate) fn into_whole(self) -> Result<Sprite, Compact> {
        match self {
            Entry::Full(s) => Ok(s),
            Entry::Pending(mut p) => Ok(p.prev.take().expect("pending entries keep their snapshot")),
            Entry::Compact(c) => Err(c),
        }
    }

    /// Memory this entry will hold once settled. With `wait`, an unfinished compaction is waited
    /// for so the count is exact; without, it counts as nothing yet (budget checks run on every
    /// checkpoint and must not block).
    pub(crate) fn for_each_buffer(&self, f: impl FnMut(usize, usize), wait: bool) {
        match self {
            Entry::Full(s) => s.for_each_buffer(f),
            Entry::Compact(c) => c.for_each_buffer(f),
            Entry::Pending(p) => {
                let state = p.shared.wait_or_peek(wait);
                match &*state {
                    Job::Done(c) => c.for_each_buffer(f),
                    Job::Failed => {
                        if let Some(s) = &p.prev {
                            s.for_each_buffer(f)
                        }
                    }
                    Job::Waiting => {}
                }
            }
        }
    }

    /// Replaces a finished `Pending` by its compact form (waiting for it with `block`) and hands
    /// the snapshot's buffers that nothing else uses any more to the buffer pool.
    pub(crate) fn settle(&mut self, block: bool) {
        let Entry::Pending(p) = self else { return };
        let result = std::mem::replace(&mut *p.shared.wait_or_peek(block), Job::Waiting);
        match result {
            Job::Waiting => {}
            Job::Done(c) => {
                let prev = p.prev.take();
                *self = Entry::Compact(c);
                if let Some(prev) = prev {
                    recycle_unique(prev);
                }
            }
            Job::Failed => {
                let prev = p.prev.take().expect("pending entries keep their snapshot");
                *self = Entry::Full(prev);
            }
        }
    }
}

/// A snapshot whose compaction runs on the history thread.
pub(crate) struct Pending {
    prev: Option<Sprite>,
    shared: Arc<Shared>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        // Popped or trimmed before the thread got to it: skip the work.
        self.shared.cancelled.store(true, Ordering::Relaxed);
    }
}

enum Job {
    Waiting,
    Done(Compact),
    Failed,
}

struct Shared {
    state: Mutex<Job>,
    done: Condvar,
    cancelled: AtomicBool,
}

impl Shared {
    fn wait_or_peek(&self, wait: bool) -> std::sync::MutexGuard<'_, Job> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        while wait && matches!(*state, Job::Waiting) {
            state = self.done.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        state
    }

    fn finish(&self, job: Job) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = job;
        self.done.notify_all();
    }
}

struct Task {
    prev: Sprite,
    newer: Sprite,
    shared: Arc<Shared>,
}

/// The history thread: compacts snapshots in the order they were scheduled.
fn history_thread() -> Option<std::sync::MutexGuard<'static, mpsc::Sender<Task>>> {
    static SENDER: OnceLock<Option<Mutex<mpsc::Sender<Task>>>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Task>();
        let spawned = std::thread::Builder::new().name("pixmith-history".into()).spawn(move || {
            for task in rx {
                let Task { prev, newer, shared } = task;
                if shared.cancelled.load(Ordering::Relaxed) {
                    shared.finish(Job::Failed);
                    continue;
                }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Compact::new(prev, &newer)));
                drop(newer);
                shared.finish(result.map_or(Job::Failed, Job::Done));
            }
        });
        spawned.ok().map(|_| Mutex::new(tx))
    });
    sender.as_ref()?.lock().ok()
}

/// Schedules the compaction of `prev` against `newer`, or does it right away when the history
/// thread is unavailable.
fn schedule(prev: Sprite, newer: &Sprite) -> Entry {
    let shared =
        Arc::new(Shared { state: Mutex::new(Job::Waiting), done: Condvar::new(), cancelled: AtomicBool::new(false) });
    let task = Task { prev: prev.clone(), newer: newer.clone(), shared: shared.clone() };
    match history_thread().map(|tx| tx.send(task)) {
        Some(Ok(())) => Entry::Pending(Pending { prev: Some(prev), shared }),
        _ => Entry::Compact(Compact::new(prev, newer)),
    }
}

/// Recycles the pixel buffers of `s` that no one else references.
fn recycle_unique(s: Sprite) {
    for cel in s.cels.into_iter().flatten().flatten() {
        if let Ok(cel) = Arc::try_unwrap(cel) {
            crate::history::buffer_pool::recycle(cel.pixels);
        }
    }
}

/// Most snapshots allowed to wait for compaction; beyond that the oldest are waited for, which
/// bounds the memory whole snapshots hold during bursts of edits.
const MAX_PENDING: usize = 2;

/// Settles the entries whose compaction has finished, and waits for the oldest ones when too
/// many are still running.
pub(crate) fn settle(chain: &mut [Entry]) {
    for e in chain.iter_mut() {
        e.settle(false);
    }
    let pending: Vec<usize> =
        chain.iter().enumerate().filter(|(_, e)| matches!(e, Entry::Pending(_))).map(|(i, _)| i).collect();
    for &i in pending.iter().take(pending.len().saturating_sub(MAX_PENDING)) {
        chain[i].settle(true);
    }
}

/// Waits for every compaction of the chain (exact memory accounting, tests).
pub(crate) fn settle_all(chain: &mut [Entry]) {
    for e in chain.iter_mut() {
        e.settle(true);
    }
}

/// If the nearest entry is whole, compacts it against `newer` (the content the entry about to be
/// pushed after it will have) on the history thread.
pub(crate) fn compact_top(chain: &mut Vec<Entry>, newer: &Sprite) {
    settle(chain);
    if matches!(chain.last(), Some(Entry::Full(_))) {
        if let Some(Entry::Full(prev)) = chain.pop() {
            chain.push(schedule(prev, newer));
        }
    }
}

/// The entry restoring `old` from the live document `live`, compacted on the history thread.
pub(crate) fn inverse_of(old: Sprite, live: &Sprite) -> Entry {
    schedule(old, live)
}

/// Appends `new` (a snapshot of the live document) as the nearest entry and compacts the entry
/// that was nearest until now against it.
pub(crate) fn push_nearest(chain: &mut Vec<Entry>, new: Sprite) {
    compact_top(chain, &new);
    chain.push(Entry::Full(new));
}

/// Remembers which buffers the live document held when a nearest `Compact` entry became valid
/// (right after an undo, redo or rollback). Holding a reference to each buffer means any later
/// in-place edit has to copy it first, so a changed pointer reveals the edit.
pub(crate) struct Witness {
    shape: Vec<usize>,
    cels: Vec<Option<Arc<Cel>>>,
    images: Vec<Arc<Vec<u8>>>,
}

impl Witness {
    pub(crate) fn of(s: &Sprite) -> Witness {
        Witness {
            shape: s.cels.iter().map(Vec::len).collect(),
            cels: s.cels.iter().flatten().cloned().collect(),
            images: s.tilesets.iter().flat_map(|t| t.tiles.iter().cloned()).collect(),
        }
    }

    /// Whether `s` still holds exactly the recorded buffers.
    pub(crate) fn matches(&self, s: &Sprite) -> bool {
        self.matches_inner(s)
    }

    fn matches_inner(&self, s: &Sprite) -> bool {
        s.cels.len() == self.shape.len()
            && s.cels.iter().zip(&self.shape).all(|(r, &n)| r.len() == n)
            && s.cels.iter().flatten().count() == self.cels.len()
            && s.cels.iter().flatten().zip(&self.cels).all(|(a, b)| match (a, b) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            })
            && s.tilesets.iter().map(|t| t.tiles.len()).sum::<usize>() == self.images.len()
            && s.tilesets.iter().flat_map(|t| t.tiles.iter()).zip(&self.images).all(|(a, b)| Arc::ptr_eq(a, b))
    }
}

// MARK: Selection masks

fn as_bytes(bits: &[bool]) -> &[u8] {
    // SAFETY: `bool` is one byte holding 0 or 1.
    unsafe { std::slice::from_raw_parts(bits.as_ptr().cast::<u8>(), bits.len()) }
}

/// Index of the first element at or after `i` that differs from `cur` (or `bits.len()`),
/// scanning eight elements per step: long uniform runs cost a fraction of a byte-wise loop.
pub(crate) fn next_change(bits: &[bool], mut i: usize, cur: bool) -> usize {
    let bytes = as_bytes(bits);
    let pattern = if cur { 0x0101_0101_0101_0101u64 } else { 0 };
    while i + 8 <= bytes.len() {
        let word = u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap()) ^ pattern;
        if word != 0 {
            return i + (word.trailing_zeros() / 8) as usize;
        }
        i += 8;
    }
    while i < bytes.len() && (bytes[i] != 0) == cur {
        i += 1;
    }
    i
}

/// A selection mask in a compact form, for history. Run lengths alternate between unselected
/// and selected pixels, starting with unselected; a bitset is used when that is smaller.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PackedMask {
    w: u32,
    h: u32,
    data: MaskData,
}

#[derive(Clone, Debug, PartialEq)]
enum MaskData {
    Runs(Vec<u32>),
    Bits(Vec<u64>),
}

impl PackedMask {
    pub(crate) fn pack(m: &Selection) -> PackedMask {
        let mut runs: Vec<u32> = Vec::new();
        let (mut i, mut cur, n) = (0usize, false, m.bits.len());
        while i < n {
            let j = next_change(&m.bits, i, cur);
            let mut len = j - i;
            // Runs longer than u32 are split with empty runs of the other value in between.
            while len > u32::MAX as usize {
                runs.push(u32::MAX);
                runs.push(0);
                len -= u32::MAX as usize;
            }
            runs.push(len as u32);
            cur = !cur;
            i = j;
        }
        let bits_len = n.div_ceil(64);
        let data = if runs.len() * 4 <= bits_len * 8 {
            MaskData::Runs(runs)
        } else {
            let bytes = as_bytes(&m.bits);
            let mut bits = vec![0u64; bits_len];
            for (word, chunk) in bits.iter_mut().zip(bytes.chunks(64)) {
                for (k, &b) in chunk.iter().enumerate() {
                    *word |= (b as u64 & 1) << k;
                }
            }
            MaskData::Bits(bits)
        };
        PackedMask { w: m.w, h: m.h, data }
    }

    pub(crate) fn unpack(&self) -> Selection {
        let n = self.w as usize * self.h as usize;
        let mut bits = Vec::with_capacity(n);
        match &self.data {
            MaskData::Runs(runs) => {
                let mut v = false;
                for &r in runs {
                    bits.resize(bits.len() + r as usize, v);
                    v = !v;
                }
                bits.resize(n, false);
            }
            MaskData::Bits(words) => {
                for &w in words {
                    let take = (n - bits.len()).min(64);
                    if w == 0 || w == u64::MAX {
                        bits.resize(bits.len() + take, w != 0);
                    } else {
                        bits.extend((0..take).map(|k| w >> k & 1 == 1));
                    }
                }
            }
        }
        Selection { w: self.w, h: self.h, bits }
    }

    pub(crate) fn bytes(&self) -> usize {
        24 + match &self.data {
            MaskData::Runs(r) => r.len() * 4,
            MaskData::Bits(b) => b.len() * 8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::tilemap::{add_tilemap_layer, stamp};

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn xorshift(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    #[test]
    fn region_delta_round_trips_for_any_change() {
        let mut seed = 0x1234_5678_9ABC_DEF1u64;
        for case in 0..200 {
            let (w, h, es) =
                (1 + (xorshift(&mut seed) % 9) as usize, 1 + (xorshift(&mut seed) % 9) as usize, [1usize, 4][case % 2]);
            let old: Vec<u8> = (0..w * h * es).map(|_| (xorshift(&mut seed) % 3) as u8).collect();
            let mut new = old.clone();
            for _ in 0..xorshift(&mut seed) % 6 {
                let i = (xorshift(&mut seed) as usize) % new.len();
                new[i] = (xorshift(&mut seed) % 5) as u8;
            }
            match RegionDelta::diff(&old, &new, es, w) {
                None => assert_eq!(old, new),
                Some(d) => {
                    let mut restored = new.clone();
                    d.apply(&mut restored, w);
                    assert_eq!(restored, old, "case {case}");
                }
            }
        }
    }

    #[test]
    fn single_pixel_changes_are_tiny_and_runs_compress() {
        let (w, h) = (1024usize, 1024usize);
        let old = vec![0u8; w * h * 4];
        let mut new = old.clone();
        new[(500 * w + 700) * 4..][..4].copy_from_slice(&RED);
        let d = RegionDelta::diff(&old, &new, 4, w).unwrap();
        assert_eq!((d.x0, d.y0, d.w, d.h), (700, 500, 1, 1));
        assert!(d.bytes() < 64);
        // A filled rectangle of one old color is a single run.
        let mut new2 = old.clone();
        for y in 100..400 {
            for x in 100..600 {
                new2[(y * w + x) * 4..][..4].copy_from_slice(&BLUE);
            }
        }
        let d2 = RegionDelta::diff(&old, &new2, 4, w).unwrap();
        assert!(d2.bytes() < 128, "{}", d2.bytes());
        assert!(RegionDelta::diff(&old, &old, 4, w).is_none());
    }

    #[test]
    fn next_change_matches_a_byte_wise_scan() {
        let mut seed = 7u64;
        for len in [0usize, 1, 7, 8, 9, 63, 64, 65, 200] {
            for density in [0u64, 3, 50, 97, 100] {
                let bits: Vec<bool> = (0..len).map(|_| xorshift(&mut seed) % 100 < density).collect();
                for start in 0..=len {
                    for cur in [false, true] {
                        let naive = (start..len).find(|&i| bits[i] != cur).unwrap_or(len);
                        assert_eq!(next_change(&bits, start, cur), naive, "len {len} start {start} cur {cur}");
                    }
                }
            }
        }
    }

    #[test]
    fn masks_pack_and_unpack_exactly() {
        let mut seed2 = 5u64;
        for (w, h) in [(1, 1), (8, 1), (65, 3), (127, 129)] {
            for density in [0u64, 2, 50, 98, 100] {
                let mut m = Selection::new(w, h, false);
                for b in m.bits.iter_mut() {
                    *b = xorshift(&mut seed2) % 100 < density;
                }
                assert_eq!(PackedMask::pack(&m).unpack(), m, "{w}x{h} density {density}");
            }
        }
        let mut seed = 99u64;
        for density in [0u64, 1, 50, 99, 100] {
            let mut m = Selection::new(37, 23, false);
            for b in m.bits.iter_mut() {
                *b = xorshift(&mut seed) % 100 < density;
            }
            assert_eq!(PackedMask::pack(&m).unpack(), m, "density {density}");
        }
        let mut rect = Selection::new(1000, 1000, false);
        for y in 100..300 {
            for x in 100..400 {
                rect.bits[y * 1000 + x] = true;
            }
        }
        let p = PackedMask::pack(&rect);
        assert!(p.bytes() < 4096, "{}", p.bytes());
        assert_eq!(p.unpack(), rect);
        let all = Selection::new(8, 8, true);
        assert_eq!(PackedMask::pack(&all).unpack(), all);
    }

    fn rich() -> Sprite {
        let mut s = Sprite::new(20, 12);
        s.draw_line(0, 0, 1, 1, 15, 8, RED, 1);
        let top = s.add_layer(0);
        s.draw_line(top, 0, 3, 3, 5, 3, BLUE, 2);
        s.add_frame(0, true);
        s.add_frame(1, false);
        assert!(s.link_cels(0, 0, 2));
        let t = add_tilemap_layer(&mut s, 1, 4, 4);
        let id = s.tilesets[0].add_blank();
        Arc::make_mut(&mut s.tilesets[0].tiles[id as usize])[0..4].copy_from_slice(&RED);
        stamp(&mut s, t, 0, 4, 4, id);
        if let Some(c) = s.cels[0][0].as_mut() {
            Arc::make_mut(c).indices = Some(Arc::new(vec![3; 20 * 12]));
        }
        s
    }

    /// Edits the sprite in ways that exercise every kind of slot.
    fn edit(s: &mut Sprite, round: usize) {
        match round % 6 {
            0 => s.draw_line(1, 0, 10, 10, 10 + (round % 5) as i32, 10, RED, 1),
            1 => {
                s.add_layer(0);
            }
            2 => {
                s.add_frame(0, true);
            }
            3 => {
                s.draw_line(0, 1, 0, 0, 19, 0, BLUE, 1);
                Arc::make_mut(s.cels[0][0].as_mut().unwrap()).opacity = 100;
            }
            4 => {
                stamp(s, 2, 0, 8, 8, 1);
                s.tilesets[0].add_blank();
            }
            _ => {
                s.draw_line(0, 0, 2, 2, 2, 2, RED, 1);
                s.remove_frame(s.frame_count() - 1);
            }
        }
    }

    /// Undoes `chain` down to its oldest entry the way the editor does, checking every state.
    fn walk_back(chain: &mut Vec<Entry>, live: &mut Sprite, states: &[Sprite], mut at: usize) -> Vec<Compact> {
        let mut inverses = Vec::new();
        while at > 0 {
            match chain.pop().unwrap().into_whole() {
                Ok(prev) => {
                    let old = std::mem::replace(live, prev);
                    inverses.push(Compact::new(old, live));
                }
                Err(c) => inverses.push(c.apply_in_place(live)),
            }
            at -= 1;
            assert!(*live == states[at], "state {at} differs after undo");
        }
        inverses
    }

    #[test]
    fn undo_chain_reproduces_every_older_state_and_the_inverses_redo_them() {
        let mut states = vec![rich()];
        for round in 0..24 {
            let mut next = states.last().unwrap().clone();
            edit(&mut next, round);
            states.push(next);
        }
        let n = states.len() - 1;
        let mut chain: Vec<Entry> = Vec::new();
        for s in &states[..n] {
            push_nearest(&mut chain, s.clone());
        }
        let mut live = states[n].clone();
        let inverses = walk_back(&mut chain, &mut live, &states, n);
        // The inverses, applied in reverse order, walk forward again.
        let mut redo: Vec<Compact> = inverses;
        let mut at = 0;
        while let Some(c) = redo.pop() {
            c.apply_in_place(&mut live);
            at += 1;
            assert!(live == states[at], "state {at} differs after redo");
        }
        assert_eq!(at, n);
    }

    #[test]
    fn linked_cels_stay_linked_and_old_buffers_are_freed() {
        let mut a = Sprite::new(8, 8);
        a.draw_line(0, 0, 0, 0, 3, 3, RED, 1);
        a.add_frame(0, false);
        assert!(a.link_cels(0, 0, 1));
        let mut b = a.clone();
        b.draw_line(0, 0, 7, 7, 7, 7, BLUE, 1);
        b.flush_links();
        let old_arc = a.cels[0][0].clone().unwrap();
        let mut chain = Vec::new();
        push_nearest(&mut chain, a.clone());
        drop(a);
        push_nearest(&mut chain, b.clone());
        settle_all(&mut chain);
        // Only our own handle still points at the old buffer: compaction released its copy.
        assert_eq!(Arc::strong_count(&old_arc), 1);
        let mut live = b.clone();
        drop(b);
        let Some(Entry::Full(_)) = chain.pop() else { panic!("nearest entry is whole") };
        let Some(Entry::Compact(c)) = chain.pop() else { panic!("older entry is compact") };
        // Applying onto the live document (here a clone of the newest state) restores the links.
        let _ = c.apply_in_place(&mut live);
        assert!(Arc::ptr_eq(live.cels[0][0].as_ref().unwrap(), live.cels[1][0].as_ref().unwrap()), "links survive");
        assert_eq!(live.cels[0][0].as_ref().unwrap().pixels, old_arc.pixels);
    }

    #[test]
    fn unshared_cels_are_patched_in_place_and_shared_ones_are_copied() {
        let mut a = Sprite::new(64, 64);
        a.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        let mut b = a.clone();
        b.draw_line(0, 0, 40, 40, 40, 40, BLUE, 1);
        let mut chain = Vec::new();
        push_nearest(&mut chain, a.clone());
        push_nearest(&mut chain, b.clone());
        settle_all(&mut chain);
        drop(a);
        let Some(Entry::Full(_)) = chain.pop() else { panic!() };
        let Some(Entry::Compact(c)) = chain.pop() else { panic!() };
        // `b` is the only owner of its cel buffer: the patch happens in the same allocation.
        let before = Arc::as_ptr(b.cels[0][0].as_ref().unwrap());
        let inverse = c.clone().apply_in_place(&mut b);
        assert_eq!(Arc::as_ptr(b.cels[0][0].as_ref().unwrap()), before, "patched in place");
        assert!(inverse.bytes < 1024);
        // A second owner forces a copy, and the other owner keeps seeing the old content.
        let mut shared = b.clone();
        let _ = inverse.apply_in_place(&mut shared);
        assert_ne!(Arc::as_ptr(shared.cels[0][0].as_ref().unwrap()), before);
        assert_eq!(b.cels[0][0].as_ref().unwrap().pixels[(40 * 64 + 40) * 4..][..4], [0, 0, 0, 0]);
        assert_eq!(shared.cels[0][0].as_ref().unwrap().pixels[(40 * 64 + 40) * 4..][..4], BLUE);
    }

    #[test]
    fn witness_notices_edits_and_replaced_buffers() {
        let mut s = Sprite::new(8, 8);
        s.draw_line(0, 0, 1, 1, 2, 2, RED, 1);
        let w = Witness::of(&s);
        assert!(w.matches(&s));
        // An in-place edit must copy the buffer (the witness holds a reference), changing its address.
        s.draw_line(0, 0, 5, 5, 5, 5, BLUE, 1);
        assert!(!w.matches(&s));
        let w = Witness::of(&s);
        s.add_layer(0);
        assert!(!w.matches(&s), "a different shape");
    }
}
