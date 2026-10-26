//! Slices (chunk 0x2022) and user data (chunk 0x2020) for sprites, layers, cels, tags and slices.
//!
//! Both are stored in `Sprite::extra`, keyed by index. The .aseprite reader and writer call
//! `parse` and `inject`, which scan the chunk stream themselves, so `formats::ase` needs no chunk logic.

use crate::document::{Editor, Sprite};
use crate::formats::sheet::json_str;
use std::collections::BTreeMap;
use std::ffi::{c_char, CStr, CString};
use std::fmt::Write;
use std::ops::Range;
use std::ptr::null_mut;
use std::sync::Arc;

const CHUNK_OLD_PALETTE_A: u16 = 0x0004;
const CHUNK_OLD_PALETTE_B: u16 = 0x0011;
const CHUNK_LAYER: u16 = 0x2004;
const CHUNK_CEL: u16 = 0x2005;
const CHUNK_COLOR_PROFILE: u16 = 0x2007;
const CHUNK_EXTERNAL_FILES: u16 = 0x2008;
const CHUNK_TAGS: u16 = 0x2018;
const CHUNK_PALETTE: u16 = 0x2019;
const CHUNK_USER_DATA: u16 = 0x2020;
const CHUNK_SLICE: u16 = 0x2022;

const SLICE_NINE_PATCH: u32 = 1;
const SLICE_PIVOT: u32 = 2;

/// Free-form data attached to an object. `props` holds the raw properties-map bytes (from the
/// size field on), preserved verbatim but not interpreted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UserData {
    pub text: Option<String>,
    pub color: Option<[u8; 4]>,
    pub props: Option<Vec<u8>>,
}

impl UserData {
    pub fn is_empty(&self) -> bool {
        self.text.is_none() && self.color.is_none() && self.props.is_none()
    }

    fn body(&self) -> Vec<u8> {
        let flags =
            self.text.is_some() as u32 | (self.color.is_some() as u32) << 1 | (self.props.is_some() as u32) << 2;
        let mut b = flags.to_le_bytes().to_vec();
        if let Some(t) = &self.text {
            put_str(&mut b, t);
        }
        if let Some(c) = self.color {
            b.extend(c);
        }
        if let Some(p) = &self.props {
            b.extend(p);
        }
        b
    }

    fn decode(body: &[u8]) -> Option<UserData> {
        let mut c = Cursor { buf: body, pos: 0 };
        let flags = c.u32()?;
        let mut ud = UserData::default();
        if flags & 1 != 0 {
            ud.text = Some(c.string()?);
        }
        if flags & 2 != 0 {
            ud.color = Some(c.take(4)?.try_into().ok()?);
        }
        if flags & 4 != 0 {
            ud.props = Some(c.buf[c.pos..].to_vec());
        }
        Some(ud)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Slice geometry from `frame` onwards (until the next key).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SliceKey {
    pub frame: u32,
    pub bounds: Rect,
    /// 9-patch center, relative to `bounds`. Only stored when the slice has a 9-patch.
    pub center: Rect,
    pub pivot: (i32, i32),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Slice {
    pub name: String,
    pub nine_patch: bool,
    pub has_pivot: bool,
    /// First frame the pivot applies to (only meaningful with `has_pivot`). Frames before it have no pivot. The
    /// .aseprite format stores one pivot flag per slice, so a file always loads with 0.
    pub pivot_from: u32,
    /// Sorted by frame, one key per frame.
    pub keys: Vec<SliceKey>,
    pub user: UserData,
}

impl Slice {
    /// The key in effect at `frame`: the last one starting at or before it.
    pub fn key_at(&self, frame: u32) -> Option<&SliceKey> {
        self.keys.iter().rev().find(|k| k.frame <= frame)
    }

    /// The pivot at `frame`; `None` without a pivot or before the first pivot key.
    pub fn pivot_at(&self, frame: u32) -> Option<(i32, i32)> {
        if !self.has_pivot || frame < self.pivot_from {
            return None;
        }
        self.key_at(frame).or(self.keys.first()).map(|k| k.pivot)
    }

    /// Sets the pivot without naming a frame: a slice with no pivot yet gets it on every existing key, one that has
    /// one changes only its first key (later keys keep their own).
    pub fn set_pivot_all(&mut self, p: (i32, i32)) {
        if self.has_pivot {
            return self.set_pivot_at(0, p);
        }
        self.keys.iter_mut().for_each(|k| k.pivot = p);
        self.has_pivot = true;
        self.pivot_from = 0;
    }

    /// Keys a pivot at `frame` (the pivot then applies from the earliest frame it was set on).
    pub fn set_pivot_at(&mut self, frame: u32, p: (i32, i32)) {
        self.pivot_from = if self.has_pivot { self.pivot_from.min(frame) } else { frame };
        self.has_pivot = true;
        key_for_edit(self, frame).pivot = p;
    }
}

/// Sprite-level data: the sprite's own user data, its slices, and header fields Pixmith only carries along. Layers, tags and cels carry
/// their user data themselves (`Layer::user_data`, `Tag::user_data`, `Cel::user_data`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extra {
    pub sprite: UserData,
    pub slices: Vec<Slice>,
    /// Pixel aspect ratio from the file header; (0, 0) means the default 1:1.
    pub pixel_ratio: (u8, u8),
    /// Grid x, y, width, height from the file header; all zero means the default 16x16 at the origin.
    pub grid: (i16, i16, u16, u16),
}

impl Extra {
    /// Call after a frame is inserted at `at` (slice keys shift).
    pub fn frame_inserted(&mut self, at: usize) {
        let at = at as u32;
        for s in &mut self.slices {
            s.keys.iter_mut().filter(|k| k.frame >= at).for_each(|k| k.frame += 1);
            if s.pivot_from >= at {
                s.pivot_from += 1;
            }
        }
    }

    /// Call after frame `from` is moved so it ends up at index `to`: slice keys follow their frame.
    pub fn frame_moved(&mut self, from: usize, to: usize) {
        let new_index = |f: u32| -> u32 {
            let (from, to, f) = (from as u32, to as u32, f);
            if f == from {
                to
            } else if from < to && f > from && f <= to {
                f - 1
            } else if to < from && f >= to && f < from {
                f + 1
            } else {
                f
            }
        };
        for s in &mut self.slices {
            s.keys.iter_mut().for_each(|k| k.frame = new_index(k.frame));
            s.keys.sort_by_key(|k| k.frame);
            s.pivot_from = new_index(s.pivot_from);
        }
    }

    /// Call after the canvas content moved by `(dx, dy)` (canvas resize, crop).
    pub fn shifted(&mut self, dx: i32, dy: i32) {
        for k in self.slices.iter_mut().flat_map(|s| s.keys.iter_mut()) {
            k.bounds.x += dx;
            k.bounds.y += dy;
        }
    }

    /// Call after the sprite was scaled by `(sx, sy)`.
    pub fn scaled(&mut self, sx: f64, sy: f64) {
        let (px, py) = (|v: i32| (v as f64 * sx).round() as i32, |v: i32| (v as f64 * sy).round() as i32);
        let (ux, uy) =
            (|v: u32| ((v as f64 * sx).round() as u32).max(1), |v: u32| ((v as f64 * sy).round() as u32).max(1));
        for k in self.slices.iter_mut().flat_map(|s| s.keys.iter_mut()) {
            k.bounds = Rect { x: px(k.bounds.x), y: py(k.bounds.y), w: ux(k.bounds.w), h: uy(k.bounds.h) };
            // An all-zero center means "no 9-patch"; keep it that way so the file round-trips.
            if k.center != Rect::default() {
                k.center = Rect { x: px(k.center.x), y: py(k.center.y), w: ux(k.center.w), h: uy(k.center.h) };
            }
            k.pivot = (px(k.pivot.0), py(k.pivot.1));
        }
    }

    /// Drops keys that point past the last frame (the removed frame was the last). A slice that would be left
    /// without keys keeps its last one, moved onto the last frame.
    pub fn clamp_frames(&mut self, frames: usize) {
        let last = frames.saturating_sub(1) as u32;
        for s in &mut self.slices {
            let Some(&tail) = s.keys.last() else { continue };
            s.keys.retain(|k| k.frame <= last);
            s.pivot_from = s.pivot_from.min(last);
            if s.keys.is_empty() {
                s.keys.push(SliceKey { frame: last, ..tail });
            }
        }
    }

    /// Call after frame `i` is removed.
    pub fn frame_removed(&mut self, i: usize) {
        let i = i as u32;
        for s in &mut self.slices {
            s.keys.iter_mut().filter(|k| k.frame > i).for_each(|k| k.frame -= 1);
            s.pivot_from = s.pivot_from.saturating_sub((s.pivot_from > i) as u32);
            // Keys that now share a frame: the later one wins.
            let mut seen: Vec<SliceKey> = Vec::new();
            for k in s.keys.drain(..) {
                match seen.last_mut() {
                    Some(prev) if prev.frame == k.frame => *prev = k,
                    _ => seen.push(k),
                }
            }
            s.keys = seen;
        }
    }
}

fn cel_user_data(s: &Sprite, frame: usize, layer: usize) -> Option<&UserData> {
    s.cels.get(frame)?.get(layer)?.as_ref().map(|c| &c.user_data)
}

/// Whether anything in the sprite needs user-data or slice chunks.
fn has_extras(s: &Sprite) -> bool {
    !s.extra.sprite.is_empty()
        || !s.extra.slices.is_empty()
        || s.layers.iter().any(|l| !l.user_data.is_empty())
        || s.tags.iter().any(|t| !t.user_data.is_empty())
        || s.cels.iter().flatten().flatten().any(|c| !c.user_data.is_empty())
}

/// User data of a target: `kind` 0 sprite, 1 layer (`a`), 2 cel (`a` frame, `b` layer), 3 tag
/// (`a`), 4 slice (`a`).
pub fn target(s: &Sprite, kind: u32, a: u32, b: u32) -> Option<&UserData> {
    match kind {
        0 => Some(&s.extra.sprite),
        1 => s.layers.get(a as usize).map(|l| &l.user_data),
        2 => cel_user_data(s, a as usize, b as usize),
        3 => s.tags.get(a as usize).map(|t| &t.user_data),
        4 => s.extra.slices.get(a as usize).map(|sl| &sl.user),
        _ => None,
    }
}

/// Like `target`, creating an empty cel when the cel does not exist yet.
fn target_mut(s: &mut Sprite, kind: u32, a: u32, b: u32) -> Option<&mut UserData> {
    match kind {
        0 => Some(&mut s.extra.sprite),
        1 => s.layers.get_mut(a as usize).map(|l| &mut l.user_data),
        2 => {
            s.begin_cel_edit(b as usize, a as usize);
            s.cel_mut_any(b as usize, a as usize).map(|c| &mut c.user_data)
        }
        3 => s.tags.get_mut(a as usize).map(|t| &mut t.user_data),
        4 => s.extra.slices.get_mut(a as usize).map(|sl| &mut sl.user),
        _ => None,
    }
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.buf.len())?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Some(s)
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn i32(&mut self) -> Option<i32> {
        self.u32().map(|v| v as i32)
    }

    fn string(&mut self) -> Option<String> {
        let n = self.u16()? as usize;
        Some(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}

fn put_u32(b: &mut Vec<u8>, v: u32) {
    b.extend(v.to_le_bytes());
}

fn put_str(b: &mut Vec<u8>, s: &str) {
    let mut n = s.len().min(u16::MAX as usize);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    b.extend((n as u16).to_le_bytes());
    b.extend(&s.as_bytes()[..n]);
}

fn slice_body(s: &Slice) -> Vec<u8> {
    let mut b = Vec::new();
    put_u32(&mut b, s.keys.len() as u32);
    put_u32(&mut b, if s.nine_patch { SLICE_NINE_PATCH } else { 0 } | if s.has_pivot { SLICE_PIVOT } else { 0 });
    put_u32(&mut b, 0);
    put_str(&mut b, &s.name);
    for k in &s.keys {
        put_u32(&mut b, k.frame);
        put_u32(&mut b, k.bounds.x as u32);
        put_u32(&mut b, k.bounds.y as u32);
        put_u32(&mut b, k.bounds.w);
        put_u32(&mut b, k.bounds.h);
        if s.nine_patch {
            put_u32(&mut b, k.center.x as u32);
            put_u32(&mut b, k.center.y as u32);
            put_u32(&mut b, k.center.w);
            put_u32(&mut b, k.center.h);
        }
        if s.has_pivot {
            put_u32(&mut b, k.pivot.0 as u32);
            put_u32(&mut b, k.pivot.1 as u32);
        }
    }
    b
}

fn parse_slice(body: &[u8]) -> Option<Slice> {
    let mut c = Cursor { buf: body, pos: 0 };
    let n = c.u32()?;
    let flags = c.u32()?;
    c.u32()?;
    let mut s = Slice {
        name: c.string()?,
        nine_patch: flags & SLICE_NINE_PATCH != 0,
        has_pivot: flags & SLICE_PIVOT != 0,
        ..Default::default()
    };
    for _ in 0..n {
        let mut k = SliceKey { frame: c.u32()?, ..Default::default() };
        k.bounds = Rect { x: c.i32()?, y: c.i32()?, w: c.u32()?, h: c.u32()? };
        if s.nine_patch {
            k.center = Rect { x: c.i32()?, y: c.i32()?, w: c.u32()?, h: c.u32()? };
        }
        if s.has_pivot {
            k.pivot = (c.i32()?, c.i32()?);
        }
        s.keys.push(k);
    }
    s.keys.sort_by_key(|k| k.frame);
    s.keys.dedup_by_key(|k| k.frame);
    Some(s)
}

// MARK: Chunk stream

struct ChunkRef {
    kind: u16,
    /// Body bytes; the chunk header (size, kind) sits in the 6 bytes before it.
    body: Range<usize>,
}

struct FrameRef {
    start: usize,
    chunks: Vec<ChunkRef>,
}

fn frames(bytes: &[u8]) -> Option<Vec<FrameRef>> {
    let mut c = Cursor { buf: bytes, pos: 4 };
    if c.u16()? != 0xA5E0 {
        return None;
    }
    let count = c.u16()? as usize;
    let mut pos = 128;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let mut f = Cursor { buf: bytes, pos };
        let size = f.u32()? as usize;
        if f.u16()? != 0xF1FA {
            return None;
        }
        let old = f.u16()? as u32;
        f.take(4)?;
        let new = f.u32()?;
        let n = if new == 0 { old } else { new };
        let end = pos.checked_add(size).filter(|&e| e >= pos + 16 && e <= bytes.len())?;
        let mut chunks = Vec::new();
        for _ in 0..n {
            let csize = f.u32()? as usize;
            let kind = f.u16()?;
            let body = csize.checked_sub(6)?;
            let start = f.pos;
            f.take(body)?;
            chunks.push(ChunkRef { kind, body: start..start + body });
        }
        out.push(FrameRef { start: pos, chunks });
        pos = end;
    }
    Some(out)
}

enum Last {
    Nothing,
    Other,
    Layer(usize),
    Cel(u32, u32),
    Tags { next: usize },
    Slice(usize),
}

/// Everything found in a file's slice and user-data chunks.
#[derive(Default)]
struct Parsed {
    extra: Extra,
    layers: Vec<UserData>,
    tags: Vec<UserData>,
    cels: BTreeMap<(u32, u32), UserData>,
}

fn set_at(v: &mut Vec<UserData>, i: usize, u: UserData) {
    if i < 4096 {
        if v.len() <= i {
            v.resize(i + 1, UserData::default());
        }
        v[i] = u;
    }
}

fn parse(bytes: &[u8]) -> Parsed {
    let mut x = Parsed::default();
    let Some(frames) = frames(bytes) else { return x };
    let mut layer_idx = 0;
    let mut last = Last::Nothing;
    for (f, frame) in frames.iter().enumerate() {
        for c in &frame.chunks {
            let body = &bytes[c.body.clone()];
            match c.kind {
                CHUNK_LAYER => {
                    last = Last::Layer(layer_idx);
                    layer_idx += 1;
                }
                CHUNK_CEL if body.len() >= 2 => {
                    last = Last::Cel(f as u32, u16::from_le_bytes([body[0], body[1]]) as u32);
                }
                CHUNK_TAGS => last = Last::Tags { next: 0 },
                CHUNK_SLICE => match parse_slice(body) {
                    Some(s) if x.extra.slices.len() < 1 << 16 => {
                        x.extra.slices.push(s);
                        last = Last::Slice(x.extra.slices.len() - 1);
                    }
                    _ => last = Last::Other,
                },
                CHUNK_USER_DATA => {
                    let Some(ud) = UserData::decode(body) else { continue };
                    match &mut last {
                        Last::Nothing => x.extra.sprite = ud,
                        Last::Layer(i) => set_at(&mut x.layers, *i, ud),
                        Last::Cel(f, l) => {
                            x.cels.insert((*f, *l), ud);
                        }
                        Last::Tags { next } => {
                            set_at(&mut x.tags, *next, ud);
                            *next += 1;
                        }
                        Last::Slice(i) => x.extra.slices[*i].user = ud,
                        Last::Other => {}
                    }
                }
                CHUNK_OLD_PALETTE_A | CHUNK_OLD_PALETTE_B | CHUNK_PALETTE | CHUNK_COLOR_PROFILE
                | CHUNK_EXTERNAL_FILES => {}
                _ => last = Last::Other,
            }
        }
    }
    x.cels.retain(|_, u| !u.is_empty());
    x
}

/// Fills in the slices and user data of a sprite just read from `bytes`. Malformed files yield
/// whatever could be found.
pub fn attach(bytes: &[u8], s: &mut Sprite) {
    let p = parse(bytes);
    s.extra = p.extra;
    for (l, u) in s.layers.iter_mut().zip(p.layers) {
        l.user_data = u;
    }
    for (t, u) in s.tags.iter_mut().zip(p.tags) {
        t.user_data = u;
    }
    for ((f, l), u) in p.cels {
        let Some(Some(old)) = s.cels.get(f as usize).and_then(|r| r.get(l as usize)).cloned() else { continue };
        // Linked cels share one buffer: give every sharer the new data so they stay linked.
        let mut cel = (*old).clone();
        cel.user_data = u;
        let new = Arc::new(cel);
        for slot in s.cels.iter_mut().filter_map(|r| r.get_mut(l as usize)) {
            if slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, &old)) {
                *slot = Some(new.clone());
            }
        }
    }
}

fn chunk(kind: u16, body: Vec<u8>) -> Vec<u8> {
    let mut c = Vec::with_capacity(body.len() + 6);
    put_u32(&mut c, body.len() as u32 + 6);
    c.extend(kind.to_le_bytes());
    c.extend(body);
    c
}

fn user_chunk(u: &UserData) -> Vec<u8> {
    chunk(CHUNK_USER_DATA, u.body())
}

/// Adds slice and user-data chunks to a file written without them. Association follows the
/// spec: user data comes right after its layer, cel, tags or slice chunk; the sprite's goes after
/// the palette. Files without any such data are returned unchanged.
pub fn inject(s: &Sprite, bytes: Vec<u8>) -> Vec<u8> {
    let x = &s.extra;
    if !has_extras(s) {
        return bytes;
    }
    let Some(frames) = frames(&bytes) else { return bytes };
    let mut out = bytes[..128].to_vec();
    for (f, fr) in frames.iter().enumerate() {
        let end = frames.get(f + 1).map_or(bytes.len(), |n| n.start);
        let mut chunks: Vec<Vec<u8>> = Vec::new();
        let mut sprite_at = None;
        let mut seen_object = false;
        let mut layer_idx = 0;
        for c in &fr.chunks {
            chunks.push(bytes[c.body.start - 6..c.body.end].to_vec());
            let body = &bytes[c.body.clone()];
            match c.kind {
                CHUNK_PALETTE if f == 0 && !seen_object && sprite_at.is_none() => sprite_at = Some(chunks.len()),
                CHUNK_LAYER => {
                    seen_object = true;
                    if let Some(u) = s.layers.get(layer_idx).map(|l| &l.user_data).filter(|u| !u.is_empty()) {
                        chunks.push(user_chunk(u));
                    }
                    layer_idx += 1;
                }
                CHUNK_CEL if body.len() >= 2 => {
                    seen_object = true;
                    let l = u16::from_le_bytes([body[0], body[1]]) as u32;
                    if let Some(u) = cel_user_data(s, f, l as usize).filter(|u| !u.is_empty()) {
                        chunks.push(user_chunk(u));
                    }
                }
                CHUNK_TAGS => {
                    seen_object = true;
                    if s.tags.iter().any(|t| !t.user_data.is_empty()) {
                        chunks.extend(s.tags.iter().map(|t| user_chunk(&t.user_data)));
                    }
                }
                _ => {}
            }
        }
        if f == 0 {
            if !x.sprite.is_empty() {
                let at = sprite_at.unwrap_or(0);
                chunks.insert(at, user_chunk(&x.sprite));
            }
            for sl in &x.slices {
                chunks.push(chunk(CHUNK_SLICE, slice_body(sl)));
                if !sl.user.is_empty() {
                    chunks.push(user_chunk(&sl.user));
                }
            }
        }
        let mut header = bytes[fr.start..fr.start + 16].to_vec();
        let size = 16 + chunks.iter().map(Vec::len).sum::<usize>();
        header[0..4].copy_from_slice(&(size as u32).to_le_bytes());
        header[6..8].copy_from_slice(&(chunks.len().min(0xFFFF) as u16).to_le_bytes());
        header[12..16].copy_from_slice(&(chunks.len() as u32).to_le_bytes());
        out.extend(header);
        out.extend(chunks.concat());
        debug_assert!(end >= fr.start);
    }
    let len = out.len() as u32;
    out[..4].copy_from_slice(&len.to_le_bytes());
    out
}

// MARK: Editing

fn name_taken(x: &Extra, name: &str) -> bool {
    x.slices.iter().any(|s| s.name == name)
}

/// Adds a slice with one key at frame 0. Returns its index.
pub fn add_slice(e: &mut Editor, rect: Rect) -> usize {
    e.checkpoint();
    let x = &mut e.sprite.extra;
    let mut n = x.slices.len() + 1;
    while name_taken(x, &format!("Slice {n}")) {
        n += 1;
    }
    let key = SliceKey { bounds: rect, ..Default::default() };
    x.slices.push(Slice { name: format!("Slice {n}"), keys: vec![key], ..Default::default() });
    x.slices.len() - 1
}

pub fn remove_slice(e: &mut Editor, i: usize) -> bool {
    if i >= e.sprite.extra.slices.len() {
        return false;
    }
    e.checkpoint();
    e.sprite.extra.slices.remove(i);
    true
}

/// The key for editing at `frame`: the existing key there, or a new one copied from the key in
/// effect (the first key if the slice starts later).
pub(crate) fn key_for_edit(s: &mut Slice, frame: u32) -> &mut SliceKey {
    if let Some(i) = s.keys.iter().position(|k| k.frame == frame) {
        return &mut s.keys[i];
    }
    let mut k = s.key_at(frame).or(s.keys.first()).copied().unwrap_or_default();
    k.frame = frame;
    let at = s.keys.partition_point(|o| o.frame < frame);
    s.keys.insert(at, k);
    &mut s.keys[at]
}

/// Moves or resizes a slice at `frame` (call `checkpoint` first when starting a drag).
pub fn set_bounds(s: &mut Sprite, i: usize, frame: u32, rect: Rect) -> bool {
    let Some(sl) = s.extra.slices.get_mut(i) else { return false };
    key_for_edit(sl, frame).bounds = rect;
    true
}

/// Edits name, 9-patch center and pivot. `center` / `pivot` of `None` turn the feature off.
pub fn set_props(
    e: &mut Editor,
    i: usize,
    frame: u32,
    name: &str,
    center: Option<Rect>,
    pivot: Option<(i32, i32)>,
) -> bool {
    if i >= e.sprite.extra.slices.len() {
        return false;
    }
    let mut sl = e.sprite.extra.slices[i].clone();
    sl.name = name.to_string();
    sl.nine_patch = center.is_some();
    // The editor sets a pivot for the whole slice; only Lua keys one from a later frame.
    if pivot.is_none() || !sl.has_pivot {
        sl.pivot_from = 0;
    }
    sl.has_pivot = pivot.is_some();
    let k = key_for_edit(&mut sl, frame);
    if let Some(c) = center {
        k.center = c;
    }
    if let Some(p) = pivot {
        k.pivot = p;
    }
    if sl != e.sprite.extra.slices[i] {
        e.checkpoint();
        e.sprite.extra.slices[i] = sl;
    }
    true
}

pub fn set_user_data(e: &mut Editor, kind: u32, a: u32, b: u32, text: Option<String>, color: Option<[u8; 4]>) -> bool {
    let s = &e.sprite;
    let valid = match kind {
        0 => true,
        1 => (a as usize) < s.layers.len(),
        2 => (a as usize) < s.frame_count() && s.layers.get(b as usize).is_some_and(|l| !l.is_group()),
        3 => (a as usize) < s.tags.len(),
        4 => (a as usize) < s.extra.slices.len(),
        _ => false,
    };
    if !valid {
        return false;
    }
    let text = text.filter(|t| !t.is_empty());
    if target(&e.sprite, kind, a, b).is_some_and(|u| u.text == text && u.color == color) {
        return true;
    }
    e.checkpoint();
    let Some(u) = target_mut(&mut e.sprite, kind, a, b) else { return false };
    u.text = text;
    u.color = color;
    true
}

// MARK: Sheet JSON

fn hex(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
}

/// Text to append to the sheet's `meta` object (after `frameTags`): `,\n    "slices": [...]`.
pub fn json_meta(s: &Sprite, scale: usize) -> String {
    let sc = scale.max(1) as i64;
    let rect = |r: Rect| {
        format!(
            "{{ \"x\": {}, \"y\": {}, \"w\": {}, \"h\": {} }}",
            r.x as i64 * sc,
            r.y as i64 * sc,
            r.w as i64 * sc,
            r.h as i64 * sc
        )
    };
    let mut out = String::from(",\n    \"slices\": [");
    for (i, sl) in s.extra.slices.iter().enumerate() {
        out.push_str(if i == 0 { "\n" } else { ",\n" });
        let color = sl.user.color.unwrap_or([0, 0, 255, 255]);
        write!(out, "      {{ \"name\": {}, \"color\": \"{}\"", json_str(&sl.name), hex(color)).unwrap();
        if let Some(t) = &sl.user.text {
            write!(out, ", \"data\": {}", json_str(t)).unwrap();
        }
        out.push_str(", \"keys\": [");
        for (j, k) in sl.keys.iter().enumerate() {
            out.push_str(if j == 0 { "\n" } else { ",\n" });
            write!(out, "        {{ \"frame\": {}, \"bounds\": {}", k.frame, rect(k.bounds)).unwrap();
            if sl.nine_patch {
                write!(out, ", \"center\": {}", rect(k.center)).unwrap();
            }
            if sl.has_pivot && k.frame >= sl.pivot_from {
                write!(out, ", \"pivot\": {{ \"x\": {}, \"y\": {} }}", k.pivot.0 as i64 * sc, k.pivot.1 as i64 * sc)
                    .unwrap();
            }
            out.push_str(" }");
        }
        out.push_str(if sl.keys.is_empty() { "] }" } else { "\n      ] }" });
    }
    out.push_str(if s.extra.slices.is_empty() { "]" } else { "\n    ]" });
    out
}

// MARK: C ABI

unsafe fn ed<'a>(doc: *mut Editor) -> Option<&'a mut Editor> {
    let e = doc.as_mut()?;
    e.dirty.mark_all();
    Some(e)
}

unsafe fn sprite<'a>(doc: *const Editor) -> Option<&'a Sprite> {
    doc.as_ref().map(|e| &e.sprite)
}

unsafe fn text_arg(p: *const c_char) -> Option<String> {
    (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
}

fn c_string(s: &str) -> *mut c_char {
    CString::new(s.replace('\0', "")).map_or(null_mut(), CString::into_raw)
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_slice_count(doc: *const Editor) -> u32 {
    crate::ffi::guarded("pixmith_slice_count", std::ptr::null_mut(), || {
        sprite(doc).map_or(0, |s| s.extra.slices.len() as u32)
    })
}

/// Release with `pixmith_string_free`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_slice_name(doc: *const Editor, i: u32) -> *mut c_char {
    crate::ffi::guarded("pixmith_slice_name", std::ptr::null_mut(), || {
        sprite(doc).and_then(|s| s.extra.slices.get(i as usize)).map_or(null_mut(), |sl| c_string(&sl.name))
    })
}

/// Writes 13 ints for the slice as it is at `frame`: x, y, w, h, has_center, cx, cy, cw, ch,
/// has_pivot, px, py, key_frame. False when the slice does not exist yet at that frame.
#[no_mangle]
pub unsafe extern "C" fn pixmith_slice_get(doc: *const Editor, i: u32, frame: u32, out: *mut i32) -> bool {
    crate::ffi::guarded("pixmith_slice_get", std::ptr::null_mut(), || {
        let Some(sl) = sprite(doc).and_then(|s| s.extra.slices.get(i as usize)) else { return false };
        let Some(k) = sl.key_at(frame) else { return false };
        if out.is_null() {
            return true;
        }
        let has_pivot = sl.pivot_at(frame).is_some();
        let v = [
            k.bounds.x,
            k.bounds.y,
            k.bounds.w as i32,
            k.bounds.h as i32,
            sl.nine_patch as i32,
            k.center.x,
            k.center.y,
            k.center.w as i32,
            k.center.h as i32,
            has_pivot as i32,
            k.pivot.0,
            k.pivot.1,
            k.frame as i32,
        ];
        std::slice::from_raw_parts_mut(out, 13).copy_from_slice(&v);
        true
    })
}

/// Creates a slice (one undo step) and returns its index, or -1.
#[no_mangle]
pub unsafe extern "C" fn pixmith_slice_add(doc: *mut Editor, x: i32, y: i32, w: u32, h: u32) -> i32 {
    crate::ffi::guarded("pixmith_slice_add", doc, || ed(doc).map_or(-1, |e| add_slice(e, Rect { x, y, w, h }) as i32))
}

/// Moves or resizes a slice at `frame` without recording undo; call `pixmith_begin_stroke` first.
#[no_mangle]
pub unsafe extern "C" fn pixmith_slice_set_bounds(
    doc: *mut Editor,
    i: u32,
    frame: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
) -> bool {
    crate::ffi::guarded("pixmith_slice_set_bounds", doc, || {
        ed(doc).is_some_and(|e| set_bounds(&mut e.sprite, i as usize, frame, Rect { x, y, w, h }))
    })
}

/// Sets name, 9-patch center (`center` = 4 ints x, y, w, h, or null for none) and pivot (2 ints or null).
#[no_mangle]
pub unsafe extern "C" fn pixmith_slice_set_props(
    doc: *mut Editor,
    i: u32,
    frame: u32,
    name: *const c_char,
    center: *const i32,
    pivot: *const i32,
) -> bool {
    crate::ffi::guarded("pixmith_slice_set_props", doc, || {
        let (Some(e), Some(name)) = (ed(doc), text_arg(name)) else { return false };
        let center = (!center.is_null()).then(|| {
            let c = std::slice::from_raw_parts(center, 4);
            Rect { x: c[0], y: c[1], w: c[2].max(0) as u32, h: c[3].max(0) as u32 }
        });
        let pivot = (!pivot.is_null()).then(|| {
            let p = std::slice::from_raw_parts(pivot, 2);
            (p[0], p[1])
        });
        set_props(e, i as usize, frame, &name, center, pivot)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_slice_remove(doc: *mut Editor, i: u32) -> bool {
    crate::ffi::guarded("pixmith_slice_remove", doc, || ed(doc).is_some_and(|e| remove_slice(e, i as usize)))
}

/// Text of a user-data target (`kind` 0 sprite, 1 layer, 2 cel, 3 tag, 4 slice), or null.
/// Release with `pixmith_string_free`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_userdata_text(doc: *const Editor, kind: u32, a: u32, b: u32) -> *mut c_char {
    crate::ffi::guarded("pixmith_userdata_text", std::ptr::null_mut(), || {
        sprite(doc).and_then(|s| target(s, kind, a, b)).and_then(|u| u.text.as_deref()).map_or(null_mut(), c_string)
    })
}

/// Writes the color as 0xRRGGBBAA; false when the target has none.
#[no_mangle]
pub unsafe extern "C" fn pixmith_userdata_color(doc: *const Editor, kind: u32, a: u32, b: u32, out: *mut u32) -> bool {
    crate::ffi::guarded("pixmith_userdata_color", std::ptr::null_mut(), || {
        let Some(c) = sprite(doc).and_then(|s| target(s, kind, a, b)).and_then(|u| u.color) else { return false };
        if !out.is_null() {
            *out = u32::from_be_bytes(c);
        }
        true
    })
}

/// Sets text (null or empty clears it) and color (`has_color` false clears it) as one undo step.
#[no_mangle]
pub unsafe extern "C" fn pixmith_userdata_set(
    doc: *mut Editor,
    kind: u32,
    a: u32,
    b: u32,
    text: *const c_char,
    has_color: bool,
    rgba: u32,
) -> bool {
    crate::ffi::guarded("pixmith_userdata_set", doc, || {
        ed(doc).is_some_and(|e| set_user_data(e, kind, a, b, text_arg(text), has_color.then(|| rgba.to_be_bytes())))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_ud_tag_count(doc: *const Editor) -> u32 {
    crate::ffi::guarded("pixmith_ud_tag_count", std::ptr::null_mut(), || sprite(doc).map_or(0, |s| s.tags.len() as u32))
}

/// Release with `pixmith_string_free`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_ud_tag_name(doc: *const Editor, i: u32) -> *mut c_char {
    crate::ffi::guarded("pixmith_ud_tag_name", std::ptr::null_mut(), || {
        sprite(doc).and_then(|s| s.tags.get(i as usize)).map_or(null_mut(), |t| c_string(&t.name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Tag;
    use crate::formats::ase;

    fn rect(x: i32, y: i32, w: u32, h: u32) -> Rect {
        Rect { x, y, w, h }
    }

    fn rich_sprite() -> Sprite {
        let mut s = Sprite::new(16, 16);
        s.draw_line(0, 0, 1, 1, 6, 4, [255, 0, 0, 255], 1);
        let top = s.add_layer(0);
        s.draw_line(top, 0, 2, 2, 3, 3, [0, 255, 0, 255], 1);
        s.add_frame(0, true);
        s.draw_line(0, 1, 9, 9, 9, 9, [0, 0, 255, 255], 1);
        s.tags.push(Tag {
            name: "a".into(),
            from: 0,
            to: 0,
            direction: 0,
            repeat: 0,
            color: [1, 2, 3],
            user_data: Default::default(),
        });
        s.tags.push(Tag {
            name: "b".into(),
            from: 1,
            to: 1,
            direction: 0,
            repeat: 0,
            color: [4, 5, 6],
            user_data: Default::default(),
        });
        s
    }

    fn text(t: &str) -> UserData {
        UserData { text: Some(t.into()), ..Default::default() }
    }

    fn set_cel_text(s: &mut Sprite, frame: usize, layer: usize, t: &str) {
        Arc::make_mut(s.cels[frame][layer].as_mut().unwrap()).user_data = text(t);
    }

    fn layer_texts(s: &Sprite) -> Vec<Option<String>> {
        s.layers.iter().map(|l| l.user_data.text.clone()).collect()
    }

    #[test]
    fn removing_the_last_frame_drops_slice_keys_past_the_end() {
        let mut s = Sprite::new(4, 4);
        s.add_frame(0, false);
        s.add_frame(1, false);
        let key = |frame| SliceKey { frame, bounds: rect(0, 0, 2, 2), ..Default::default() };
        s.extra.slices.push(Slice { name: "a".into(), keys: vec![key(0), key(2)], ..Default::default() });
        s.extra.slices.push(Slice { name: "b".into(), keys: vec![key(2)], ..Default::default() });
        assert!(s.remove_frame(2));
        let frames = |i: usize| s.extra.slices[i].keys.iter().map(|k| k.frame).collect::<Vec<_>>();
        assert_eq!(frames(0), vec![0]);
        assert_eq!(frames(1), vec![1]);
        // Removing a middle frame still only shifts later keys.
        s.extra.slices[0].keys.push(key(1));
        assert!(s.remove_frame(0));
        assert_eq!(s.extra.slices[0].keys.iter().map(|k| k.frame).collect::<Vec<_>>(), vec![0]);
    }

    #[test]
    fn user_data_body_round_trips_all_fields() {
        let u = UserData {
            text: Some("héllo".into()),
            color: Some([1, 2, 3, 4]),
            props: Some(vec![9, 0, 0, 0, 0, 0, 0, 0]),
        };
        assert_eq!(UserData::decode(&u.body()), Some(u));
        let only_color = UserData { color: Some([5, 6, 7, 8]), ..Default::default() };
        assert_eq!(UserData::decode(&only_color.body()), Some(only_color));
        assert_eq!(UserData::decode(&[1, 0, 0, 0, 5, 0]), None);
    }

    #[test]
    fn slice_body_round_trips_with_optional_parts() {
        let mut sl = Slice {
            name: "icon".into(),
            nine_patch: true,
            has_pivot: true,
            pivot_from: 0,
            keys: vec![
                SliceKey { frame: 0, bounds: rect(1, 2, 8, 9), center: rect(2, 2, 4, 5), pivot: (-1, 3) },
                SliceKey { frame: 3, bounds: rect(-4, 0, 3, 3), center: rect(0, 0, 1, 1), pivot: (2, 2) },
            ],
            user: UserData::default(),
        };
        assert_eq!(parse_slice(&slice_body(&sl)), Some(sl.clone()));
        sl.nine_patch = false;
        sl.has_pivot = false;
        for k in &mut sl.keys {
            k.center = Rect::default();
            k.pivot = (0, 0);
        }
        assert_eq!(parse_slice(&slice_body(&sl)), Some(sl));
        assert!(
            parse_slice(&slice_body(&Slice { keys: vec![SliceKey::default()], ..Default::default() })[..10]).is_none()
        );
    }

    #[test]
    fn file_round_trip_preserves_slices_and_user_data() {
        let mut s = rich_sprite();
        s.extra.sprite = text("sprite note");
        s.layers[1].user_data = UserData { text: Some("top".into()), color: Some([9, 8, 7, 255]), props: None };
        set_cel_text(&mut s, 1, 0, "cel");
        s.tags[0].user_data = UserData { color: Some([1, 1, 1, 255]), ..Default::default() };
        s.tags[1].user_data = text("b");
        s.extra.slices.push(Slice {
            name: "hit".into(),
            nine_patch: true,
            has_pivot: true,
            pivot_from: 0,
            keys: vec![SliceKey { frame: 0, bounds: rect(2, 3, 5, 6), center: rect(1, 1, 3, 4), pivot: (2, 2) }],
            user: UserData {
                text: Some("data".into()),
                color: Some([0, 0, 255, 255]),
                props: Some(vec![8, 0, 0, 0, 0, 0, 0, 0]),
            },
        });
        s.extra.slices.push(Slice {
            name: "plain".into(),
            keys: vec![SliceKey { frame: 1, bounds: rect(0, 0, 2, 2), ..Default::default() }],
            ..Default::default()
        });
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.cels[1][0].as_ref().unwrap().user_data.text.as_deref(), Some("cel"));
    }

    #[test]
    fn files_without_extras_are_unchanged() {
        let s = rich_sprite();
        let bytes = ase::write(&s);
        assert!(!has_extras(&s));
        assert!(parse(&bytes).cels.is_empty());
        assert_eq!(inject(&s, bytes.clone()), bytes);
    }

    #[test]
    fn user_data_is_ordered_after_its_owner_chunk() {
        let mut s = rich_sprite();
        s.extra.sprite.text = Some("S".into());
        s.layers[0].user_data = text("L0");
        set_cel_text(&mut s, 0, 1, "C");
        s.tags[1].user_data = text("T1");
        let bytes = ase::write(&s);
        let frames = frames(&bytes).unwrap();
        let seq: Vec<u16> = frames[0].chunks.iter().map(|c| c.kind).collect();
        assert_eq!(seq[0], CHUNK_PALETTE);
        assert_eq!(seq[1], CHUNK_USER_DATA, "sprite data follows the palette");
        assert_eq!(&seq[2..5], &[CHUNK_LAYER, CHUNK_USER_DATA, CHUNK_LAYER]);
        let tags_at = seq.iter().position(|k| *k == CHUNK_TAGS).unwrap();
        assert_eq!(&seq[tags_at + 1..tags_at + 3], &[CHUNK_USER_DATA, CHUNK_USER_DATA], "one per tag");
        let cel1 = frames[0].chunks.iter().position(|c| c.kind == CHUNK_CEL && bytes[c.body.start] == 1).unwrap();
        assert_eq!(seq[cel1 + 1], CHUNK_USER_DATA);
        // Frame sizes and the file size stay consistent.
        assert_eq!(u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize, bytes.len());
        assert_eq!(frames.len(), 2);
    }

    #[test]
    fn unknown_chunk_blocks_misattributed_user_data() {
        // A user data chunk after a chunk kind we do not know must not land on the previous layer.
        let s = rich_sprite();
        let mut bytes = ase::write(&s);
        let fr = &frames(&bytes).unwrap()[0];
        let layer_end = fr.chunks.iter().find(|c| c.kind == CHUNK_LAYER).unwrap().body.end;
        let unknown = chunk(0x2999, vec![1, 2, 3]);
        let ud = user_chunk(&UserData { text: Some("x".into()), ..Default::default() });
        let mut patched = bytes[..layer_end].to_vec();
        patched.extend(&unknown);
        patched.extend(&ud);
        patched.extend(&bytes[layer_end..]);
        // Fix frame 0's size and chunk count, then the file size.
        let add = (unknown.len() + ud.len()) as u32;
        let size = u32::from_le_bytes(patched[128..132].try_into().unwrap()) + add;
        patched[128..132].copy_from_slice(&size.to_le_bytes());
        let count = u32::from_le_bytes(patched[140..144].try_into().unwrap()) + 2;
        patched[140..144].copy_from_slice(&count.to_le_bytes());
        patched[134..136].copy_from_slice(&(count as u16).to_le_bytes());
        let len = patched.len() as u32;
        patched[..4].copy_from_slice(&len.to_le_bytes());
        bytes = patched;
        let p = parse(&bytes);
        assert!(p.layers.is_empty() && p.extra == Extra::default());
        assert!(ase::read(&bytes).is_ok());
    }

    /// Three painted layers a, b, c (bottom to top) with user data on layers and cels.
    fn abc() -> Sprite {
        let mut s = Sprite::new(4, 4);
        s.layers[0].name = "a".into();
        for (i, n) in ["b", "c"].iter().enumerate() {
            let at = s.add_layer(i);
            s.layers[at].name = (*n).into();
        }
        for l in 0..3 {
            s.draw_line(l, 0, l as i32, 0, l as i32, 0, [255, 0, 0, 255], 1);
            let name = s.layers[l].name.clone();
            s.layers[l].user_data = text(&format!("L{name}"));
            set_cel_text(&mut s, 0, l, &format!("C{name}"));
        }
        s
    }

    fn texts(s: &Sprite) -> Vec<String> {
        s.layers.iter().map(|l| l.user_data.text.clone().unwrap()).collect()
    }

    #[test]
    fn layer_user_data_follows_move_duplicate_merge_group_and_delete() {
        let mut s = abc();
        assert_eq!(s.move_layer(0, true), Some(1));
        assert_eq!(texts(&s), ["Lb", "La", "Lc"]);
        assert_eq!(s.cels[0][1].as_ref().unwrap().user_data.text.as_deref(), Some("Ca"));
        assert_eq!(s.move_layer(2, false), Some(1));
        assert_eq!(texts(&s), ["Lb", "Lc", "La"]);

        let dup = s.duplicate_layer(0).unwrap();
        assert_eq!(texts(&s)[dup], "Lb");
        assert_eq!(s.cels[0][dup].as_ref().unwrap().user_data.text.as_deref(), Some("Cb"));
        assert_eq!(s.layers[dup].name, "b copy");
        // The copy is independent of the original.
        s.layers[dup].user_data = text("changed");
        assert_eq!(texts(&s)[0], "Lb");

        // Merge down keeps the lower layer's data and removes the upper's.
        let mut m = abc();
        assert_eq!(m.merge_down(2), Some(1));
        assert_eq!(texts(&m), ["La", "Lb"]);
        assert_eq!(m.cels[0][1].as_ref().unwrap().user_data.text.as_deref(), Some("Cb"));

        // Grouping and deleting.
        let mut g = abc();
        let at = g.group_layer(1).unwrap();
        assert!(layer_texts(&g).contains(&Some("Lb".to_string())) && g.layers[at].user_data.is_empty());
        assert!(g.remove_layer(0));
        assert!(!texts_opt(&g).contains(&Some("La".into())));
        assert!(g.layers.iter().any(|l| l.user_data.text.as_deref() == Some("Lc")));
        let lc = g.layers.iter().position(|l| l.user_data.text.as_deref() == Some("Lc")).unwrap();
        assert_eq!(g.cels[0][lc].as_ref().unwrap().user_data.text.as_deref(), Some("Cc"));
    }

    fn texts_opt(s: &Sprite) -> Vec<Option<String>> {
        layer_texts(s)
    }

    #[test]
    fn cel_user_data_follows_frame_edits() {
        let mut s = abc();
        s.add_frame(0, true);
        assert_eq!(s.cels[1][2].as_ref().unwrap().user_data.text.as_deref(), Some("Cc"));
        s.add_frame(0, false);
        assert_eq!(s.cels[2][2].as_ref().unwrap().user_data.text.as_deref(), Some("Cc"));
        assert!(s.cels[1][2].is_none());
        assert!(s.move_frame(2, 0));
        assert_eq!(s.cels[0][2].as_ref().unwrap().user_data.text.as_deref(), Some("Cc"));
        assert!(s.remove_frame(2));
        assert!(s.remove_frame(0));
        assert_eq!(s.cels[0][2].as_ref().unwrap().user_data.text.as_deref(), Some("Cc"));
        // Editing pixels keeps the cel's user data.
        s.draw_line(2, 0, 3, 3, 3, 3, [0, 0, 255, 255], 1);
        assert_eq!(s.cels[0][2].as_ref().unwrap().user_data.text.as_deref(), Some("Cc"));
    }

    #[test]
    fn tag_user_data_follows_add_update_and_remove() {
        let mut s = Sprite::new(2, 2);
        for _ in 0..3 {
            s.add_frame(0, false);
        }
        let (a, b, c) = (s.add_tag(0, 0), s.add_tag(1, 1), s.add_tag(2, 3));
        s.tags[a].user_data = text("A");
        s.tags[b].user_data = text("B");
        s.tags[c].user_data = text("C");
        assert!(s.update_tag(b, "renamed", 1, 2, 1, 0, [1, 1, 1]));
        assert_eq!(s.tags[b].user_data.text.as_deref(), Some("B"));
        assert!(s.remove_tag(a));
        let names: Vec<_> = s.tags.iter().map(|t| (t.name.clone(), t.user_data.text.clone().unwrap())).collect();
        assert_eq!(names, [("renamed".to_string(), "B".to_string()), ("Tag 3".to_string(), "C".to_string())]);
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back.tags[0].user_data.text.as_deref(), Some("B"));
        assert_eq!(back.tags[1].user_data.text.as_deref(), Some("C"));
    }

    #[test]
    fn cel_user_data_survives_tilemap_conversion() {
        let mut s = abc();
        assert!(crate::document::tilemap::convert_to_tilemap(&mut s, 0, 2, 2));
        assert_eq!(s.cels[0][0].as_ref().unwrap().user_data.text.as_deref(), Some("Ca"));
        assert!(crate::document::tilemap::convert_to_normal(&mut s, 0));
        assert_eq!(s.cels[0][0].as_ref().unwrap().user_data.text.as_deref(), Some("Ca"));
        assert_eq!(s.layers[0].user_data.text.as_deref(), Some("La"));
    }

    #[test]
    fn user_data_survives_operations_and_a_save() {
        let mut s = abc();
        s.add_frame(0, true);
        s.tags.push(Tag {
            name: "t".into(),
            from: 0,
            to: 1,
            direction: 0,
            repeat: 0,
            color: [1, 2, 3],
            user_data: text("T"),
        });
        s.move_layer(0, true);
        s.duplicate_layer(2);
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(layer_texts(&back), layer_texts(&s));
        for f in 0..2 {
            for l in 0..s.layers.len() {
                let t = |s: &Sprite| s.cels[f][l].as_ref().map(|c| c.user_data.text.clone());
                assert_eq!(t(&back), t(&s), "frame {f} layer {l}");
            }
        }
        assert_eq!(back.tags[0].user_data.text.as_deref(), Some("T"));
    }

    #[test]
    fn slice_keys_follow_frame_edits() {
        let mut s = Sprite::new(4, 4);
        for _ in 0..3 {
            s.add_frame(0, false);
        }
        let keys =
            |frames: &[u32]| frames.iter().map(|f| SliceKey { frame: *f, ..Default::default() }).collect::<Vec<_>>();
        s.extra.slices.push(Slice { keys: keys(&[0, 2, 3]), ..Default::default() });
        s.remove_frame(2);
        let frames: Vec<u32> = s.extra.slices[0].keys.iter().map(|k| k.frame).collect();
        assert_eq!(frames, vec![0, 2]);
        s.add_frame(0, false);
        let frames: Vec<u32> = s.extra.slices[0].keys.iter().map(|k| k.frame).collect();
        assert_eq!(frames, vec![0, 3]);
    }

    #[test]
    fn slice_keys_follow_moved_frames() {
        let mut s = Sprite::new(4, 4);
        for _ in 0..3 {
            s.add_frame(0, false);
        }
        let key = |f: u32, x: i32| SliceKey { frame: f, bounds: rect(x, 0, 1, 1), ..Default::default() };
        s.extra.slices.push(Slice { keys: vec![key(0, 10), key(1, 11), key(3, 13)], ..Default::default() });
        // Frame 3 moves to the front: its key becomes frame 0 and the others shift up.
        assert!(s.move_frame(3, 0));
        let got: Vec<_> = s.extra.slices[0].keys.iter().map(|k| (k.frame, k.bounds.x)).collect();
        assert_eq!(got, vec![(0, 13), (1, 10), (2, 11)]);
        // And back: frame 0 moves to the end.
        assert!(s.move_frame(0, 3));
        let got: Vec<_> = s.extra.slices[0].keys.iter().map(|k| (k.frame, k.bounds.x)).collect();
        assert_eq!(got, vec![(0, 10), (1, 11), (3, 13)]);
        assert!(s.move_frame(1, 2));
        let got: Vec<_> = s.extra.slices[0].keys.iter().map(|k| (k.frame, k.bounds.x)).collect();
        assert_eq!(got, vec![(0, 10), (2, 11), (3, 13)]);
    }

    #[test]
    fn editing_is_undoable_and_keys_are_per_frame() {
        let mut e = Editor::new(Sprite::new(16, 16));
        e.sprite.add_frame(0, false);
        e.sprite.add_frame(1, false);
        let i = add_slice(&mut e, rect(1, 1, 4, 4));
        assert_eq!(e.sprite.extra.slices[i].name, "Slice 1");
        assert_eq!(add_slice(&mut e, rect(0, 0, 1, 1)), 1);
        assert_eq!(e.sprite.extra.slices[1].name, "Slice 2");
        e.checkpoint();
        assert!(set_bounds(&mut e.sprite, i, 2, rect(5, 5, 4, 4)));
        let sl = &e.sprite.extra.slices[i];
        assert_eq!(sl.keys.len(), 2);
        assert_eq!(sl.key_at(1).unwrap().bounds, rect(1, 1, 4, 4));
        assert_eq!(sl.key_at(2).unwrap().bounds, rect(5, 5, 4, 4));
        assert!(set_bounds(&mut e.sprite, i, 2, rect(6, 5, 4, 4)));
        assert_eq!(e.sprite.extra.slices[i].keys.len(), 2);
        assert!(e.undo());
        assert_eq!(e.sprite.extra.slices[i].keys.len(), 1);
        assert!(set_props(&mut e, i, 0, "button", Some(rect(1, 1, 2, 2)), Some((3, 4))));
        let sl = &e.sprite.extra.slices[i];
        assert_eq!((sl.name.as_str(), sl.nine_patch, sl.has_pivot), ("button", true, true));
        assert_eq!(sl.keys[0].pivot, (3, 4));
        assert!(set_props(&mut e, i, 0, "button", None, None));
        assert!(!e.sprite.extra.slices[i].nine_patch);
        assert!(remove_slice(&mut e, 0));
        assert_eq!(e.sprite.extra.slices.len(), 1);
        assert!(e.undo());
        assert_eq!(e.sprite.extra.slices.len(), 2);
        assert!(!remove_slice(&mut e, 9));
    }

    #[test]
    fn user_data_editing_by_target() {
        let mut e = Editor::new(rich_sprite());
        assert!(set_user_data(&mut e, 1, 1, 0, Some("note".into()), Some([1, 2, 3, 4])));
        assert_eq!(target(&e.sprite, 1, 1, 0).unwrap().text.as_deref(), Some("note"));
        assert!(set_user_data(&mut e, 3, 1, 0, Some("tag".into()), None));
        assert!(set_user_data(&mut e, 2, 0, 1, Some("cel".into()), None));
        assert!(!set_user_data(&mut e, 4, 7, 0, None, None));
        assert!(!set_user_data(&mut e, 9, 0, 0, None, None));
        assert!(set_user_data(&mut e, 0, 0, 0, Some(String::new()), None));
        assert!(e.sprite.extra.sprite.is_empty());
        let back = ase::read(&ase::write(&e.sprite)).unwrap();
        assert_eq!(back.layers[1].user_data.color, Some([1, 2, 3, 4]));
        assert_eq!(back.tags[1].user_data.text.as_deref(), Some("tag"));
        assert_eq!(back.cels[0][1].as_ref().unwrap().user_data.text.as_deref(), Some("cel"));
        assert!(set_user_data(&mut e, 2, 0, 1, Some("again".into()), None));
        assert!(e.undo());
        assert_eq!(e.sprite.cels[0][1].as_ref().unwrap().user_data.text.as_deref(), Some("cel"));
    }

    #[test]
    fn sheet_json_lists_slices() {
        let mut s = Sprite::new(8, 8);
        s.extra.slices.push(Slice {
            name: "a\"b".into(),
            nine_patch: true,
            has_pivot: true,
            pivot_from: 0,
            keys: vec![SliceKey { frame: 0, bounds: rect(1, 2, 3, 4), center: rect(1, 1, 1, 1), pivot: (1, 0) }],
            user: UserData { text: Some("t".into()), color: Some([255, 0, 0, 255]), props: None },
        });
        let j = json_meta(&s, 2);
        assert!(j.contains("\"name\": \"a\\\"b\", \"color\": \"#ff0000ff\", \"data\": \"t\""));
        assert!(j.contains("\"bounds\": { \"x\": 2, \"y\": 4, \"w\": 6, \"h\": 8 }"));
        assert!(j.contains("\"center\": { \"x\": 2, \"y\": 2, \"w\": 2, \"h\": 2 }"));
        assert!(j.contains("\"pivot\": { \"x\": 2, \"y\": 0 }"));
        assert_eq!(j.matches('{').count(), j.matches('}').count());
        assert_eq!(json_meta(&Sprite::new(1, 1), 1), ",\n    \"slices\": []");
    }

    #[test]
    fn pivot_is_absent_before_its_first_key() {
        let mut s = Sprite::new(8, 8);
        let k = |frame| SliceKey { frame, bounds: rect(0, 0, 2, 2), ..Default::default() };
        let mut sl = Slice { keys: vec![k(0)], ..Default::default() };
        assert_eq!(sl.pivot_at(0), None);
        let mut all = Slice { keys: vec![k(0), k(4)], ..Default::default() };
        all.set_pivot_all((7, 8));
        assert_eq!((all.pivot_at(0), all.pivot_at(4)), (Some((7, 8)), Some((7, 8))));
        sl.set_pivot_at(2, (3, 4));
        assert_eq!(
            (sl.pivot_at(0), sl.pivot_at(1), sl.pivot_at(2), sl.pivot_at(5)),
            (None, None, Some((3, 4)), Some((3, 4)))
        );
        s.extra.slices.push(sl.clone());
        let j = json_meta(&s, 1);
        assert_eq!(j.matches("\"pivot\"").count(), 1, "{j}");
        // A pivot set on an earlier frame extends the range backwards.
        sl.set_pivot_at(0, (1, 1));
        assert_eq!((sl.pivot_at(0), sl.pivot_at(2)), (Some((1, 1)), Some((3, 4))));
        // Inserting a frame before the first pivot key moves it along.
        let mut ex = Extra { slices: vec![Slice { keys: vec![k(0)], ..Default::default() }], ..Default::default() };
        ex.slices[0].set_pivot_at(2, (1, 2));
        ex.frame_inserted(1);
        assert_eq!((ex.slices[0].pivot_at(2), ex.slices[0].pivot_at(3)), (None, Some((1, 2))));
    }
}
