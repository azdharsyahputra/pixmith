//! Reader and writer for the Aseprite (.ase/.aseprite) file format.
//! Spec: https://github.com/aseprite/aseprite/blob/main/docs/ase-file-specs.md

use crate::document::color_mode::{ColorMode, Encoder};
use crate::document::spritedata::ColorProfile;
use crate::document::tilemap;
use crate::document::{default_palette, Cel, Layer, OffCanvas, Sprite, Tag, MAX_CANVAS_PIXELS};
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;

pub type Result<T> = std::result::Result<T, String>;

const FILE_MAGIC: u16 = 0xA5E0;
const FRAME_MAGIC: u16 = 0xF1FA;
const CHUNK_OLD_PALETTE: u16 = 0x0004;
const CHUNK_OLD_PALETTE_6BIT: u16 = 0x0011;
const CHUNK_LAYER: u16 = 0x2004;
const CHUNK_CEL: u16 = 0x2005;
const CHUNK_COLOR_PROFILE: u16 = 0x2007;
const CHUNK_TAGS: u16 = 0x2018;
const CHUNK_PALETTE: u16 = 0x2019;

/// Limits that keep a corrupt or hostile file from exhausting memory.
const MAX_LAYERS: usize = 4096;
/// Total number of (frame, layer) slots.
const MAX_CEL_SLOTS: usize = 1 << 22;
/// Decoded-but-not-yet-placed pixel data (zlib bombs are rejected by this).
const MAX_PENDING_BYTES: usize = 1 << 29;
/// Memory the finished cels of one file may take.
const MAX_CEL_BYTES: usize = 1 << 31;

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.buf.len())
            .ok_or_else(|| "unexpected end of file".to_string())?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }

    fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    fn string(&mut self) -> Result<String> {
        let n = self.u16()? as usize;
        Ok(String::from_utf8_lossy(self.bytes(n)?).into_owned())
    }

    fn rest(&mut self) -> &'a [u8] {
        let s = &self.buf[self.pos..];
        self.pos = self.buf.len();
        s
    }
}

enum CelData {
    Pixels { x: i32, y: i32, w: usize, h: usize, data: Vec<u8> },
    Linked(usize),
    Tilemap(tilemap::TileGrid),
}

struct PendingCel {
    frame: usize,
    layer: usize,
    opacity: u8,
    z_index: i16,
    data: CelData,
}

pub fn read(bytes: &[u8]) -> Result<Sprite> {
    let mut r = Reader::new(bytes);
    r.skip(4)?;
    if r.u16()? != FILE_MAGIC {
        return Err("not an Aseprite file".into());
    }
    let frame_count = r.u16()? as usize;
    let width = r.u16()? as u32;
    let height = r.u16()? as u32;
    let depth = r.u16()?;
    let flags = r.u32()?;
    let speed = r.u16()?; // deprecated, but frames with a duration of 0 use it
    r.skip(4 + 4)?;
    let transparent = r.u8()?;
    r.skip(3 + 2)?; // reserved, color count
    let pixel_ratio = (r.u8()?, r.u8()?);
    let grid = (r.i16()?, r.i16()?, r.u16()?, r.u16()?);
    let rest = 128 - r.pos;
    r.skip(rest)?;
    if width == 0 || height == 0 || frame_count == 0 {
        return Err("sprite has no pixels or frames".into());
    }
    if width as usize * height as usize > MAX_CANVAS_PIXELS {
        return Err(format!("a {width}x{height} canvas is too large"));
    }
    let bpp = match depth {
        32 => 4,
        16 => 2,
        8 => 1,
        d => return Err(format!("unsupported color depth {d}")),
    };
    let opacity_valid = flags & 1 != 0;
    // Groups carry meaningful opacity and blend mode only when this flag is set (Aseprite 1.3 and later).
    let group_values_valid = flags & 2 != 0;

    let mut layers = Vec::new();
    let mut durations = Vec::with_capacity(frame_count);
    let mut tags = Vec::new();
    let mut palette = Vec::new();
    let mut has_new_palette = false;
    let mut pending = Vec::new();
    let mut raw_tilesets = Vec::new();
    let mut color_profile = None;
    let mut pending_bytes = 0usize;
    let mut cel_bytes = 0usize;
    let mut tileset_bytes = 0usize;
    let canvas_bytes = width as usize * height as usize * 4;

    for frame in 0..frame_count {
        let start = r.pos;
        let size = r.u32()? as usize;
        if r.u16()? != FRAME_MAGIC {
            return Err(format!("frame {} is corrupt", frame + 1));
        }
        let old_chunks = r.u16()?;
        durations.push(match r.u16()? {
            0 if speed > 0 => speed,
            d => d,
        });
        r.skip(2)?;
        let new_chunks = r.u32()?;
        let chunks = if new_chunks == 0 { old_chunks as u32 } else { new_chunks };
        for _ in 0..chunks {
            let size = r.u32()? as usize;
            let kind = r.u16()?;
            let body = size.checked_sub(6).ok_or_else(|| "corrupt chunk".to_string())?;
            let mut c = Reader::new(r.bytes(body)?);
            match kind {
                CHUNK_LAYER => {
                    let lflags = c.u16()?;
                    let kind = c.u16()?;
                    let child_level = c.u16()?;
                    c.skip(4)?;
                    let blend_mode = c.u16()?;
                    let opacity = c.u8()?;
                    c.skip(3)?;
                    if layers.len() >= MAX_LAYERS {
                        return Err("too many layers".into());
                    }
                    let name = c.string()?;
                    let tileset = if kind == tilemap::KIND_TILEMAP { c.u32()? } else { 0 };
                    layers.push(Layer {
                        name,
                        visible: lflags & 1 != 0,
                        opacity: if opacity_valid && (kind != 1 || group_values_valid) { opacity } else { 255 },
                        kind,
                        child_level,
                        blend_mode: if kind == 1 && !group_values_valid { 0 } else { blend_mode },
                        flags: lflags & !1,
                        user_data: Default::default(),
                        tileset,
                    });
                }
                CHUNK_CEL => {
                    if let Some(cel) = read_cel(&mut c, frame, bpp, MAX_PENDING_BYTES - pending_bytes)? {
                        pending_bytes += match &cel.data {
                            CelData::Pixels { data, .. } => data.len(),
                            CelData::Tilemap(g) => g.tiles.len() * 4,
                            CelData::Linked(_) => 0,
                        };
                        if !matches!(cel.data, CelData::Linked(_)) {
                            cel_bytes += canvas_bytes;
                        }
                        if pending_bytes > MAX_PENDING_BYTES || cel_bytes > MAX_CEL_BYTES {
                            return Err("file needs too much memory".into());
                        }
                        pending.push(cel);
                    }
                }
                tilemap::CHUNK_TILESET => {
                    let t = tilemap::parse_tileset_chunk(c.rest(), bpp)?;
                    tileset_bytes += t.pixel_bytes();
                    if tileset_bytes > MAX_PENDING_BYTES {
                        return Err("tilesets need too much memory".into());
                    }
                    raw_tilesets.push(t);
                }
                CHUNK_COLOR_PROFILE => {
                    color_profile.get_or_insert_with(|| ColorProfile::from_chunk(c.rest()));
                }
                CHUNK_TAGS => tags = read_tags(&mut c)?,
                CHUNK_PALETTE => {
                    if !has_new_palette {
                        palette.clear();
                        has_new_palette = true;
                    }
                    read_palette(&mut c, &mut palette)?;
                }
                CHUNK_OLD_PALETTE if !has_new_palette => read_old_palette(&mut c, &mut palette, false)?,
                CHUNK_OLD_PALETTE_6BIT if !has_new_palette => read_old_palette(&mut c, &mut palette, true)?,
                _ => {}
            }
        }
        r.pos = start
            .checked_add(size)
            .filter(|&e| e > start && e <= bytes.len())
            .ok_or_else(|| format!("frame {} has an invalid size", frame + 1))?;
    }

    if !layers.iter().any(|l| !l.is_group()) {
        layers.push(Layer::new("Layer 1"));
    }
    // Frame ranges of tags must refer to existing frames, in order.
    for t in &mut tags {
        let last = (frame_count - 1).min(u16::MAX as usize) as u16;
        let (a, b) = (t.from.min(last), t.to.min(last));
        (t.from, t.to) = (a.min(b), a.max(b));
    }
    if frame_count * layers.len() > MAX_CEL_SLOTS {
        return Err("too many frames and layers".into());
    }
    if palette.is_empty() {
        palette = default_palette();
    }
    if depth == 8 {
        // Pixels may use indices beyond the stored palette; give them (black) entries so they survive saving.
        let used = pending.iter().filter_map(|p| match &p.data {
            CelData::Pixels { data, .. } => data.iter().filter(|&&v| v != transparent).max(),
            _ => None,
        });
        if let Some(&max) = used.max() {
            if max as usize >= palette.len() {
                palette.resize(max as usize + 1, [0, 0, 0, 255]);
            }
        }
    }
    let (w, h) = (width as usize, height as usize);
    crate::document::layers::normalize_levels(&mut layers);
    let (tilesets, tileset_ids) =
        tilemap::finish_tilesets(raw_tilesets, |d| to_rgba(d, depth, &palette, transparent, false));
    tilemap::resolve_layers(&mut layers, &tileset_ids);
    let mut cels = vec![vec![None; layers.len()]; frame_count];
    let mut links = Vec::new();
    for p in pending {
        if p.layer >= layers.len() || layers[p.layer].is_group() {
            continue;
        }
        match p.data {
            CelData::Pixels { x, y, w: cw, h: ch, data } => {
                let background = layers[p.layer].flags & 8 != 0;
                let src = to_rgba(&data, depth, &palette, transparent, background);
                let mut pixels = vec![0; w * h * 4];
                blit(&mut pixels, w, h, &src, x, y, cw, ch);
                // What lies beyond the canvas is kept (a part too far away for Pixmith is dropped).
                let outside = if x < 0 || y < 0 || x as i64 + cw as i64 > w as i64 || y as i64 + ch as i64 > h as i64 {
                    OffCanvas::extract(w, h, (x, y, cw, ch), &src).ok().flatten().map(Arc::new)
                } else {
                    None
                };
                let indices = (depth == 8).then(|| {
                    Arc::new(crate::document::color_mode::index_canvas(&data, x, y, cw, ch, w, h, transparent))
                });
                cels[p.frame][p.layer] = Some(Arc::new(Cel {
                    opacity: p.opacity,
                    pixels,
                    indices,
                    z_index: p.z_index,
                    outside,
                    ..Default::default()
                }));
            }
            CelData::Tilemap(grid) => {
                if let Some(mut cel) = tilemap::cel_from_grid(&tilesets, &layers[p.layer], p.opacity, grid, w, h) {
                    cel.z_index = p.z_index;
                    cels[p.frame][p.layer] = Some(Arc::new(cel));
                }
            }
            CelData::Linked(src) => links.push((p.frame, p.layer, src)),
        }
    }
    for (frame, layer, src) in links {
        if src < frame_count {
            cels[frame][layer] = cels[src][layer].clone();
        }
    }
    let mut sprite = Sprite {
        width,
        height,
        layers,
        durations,
        cels,
        tags,
        palette,
        links: Default::default(),
        tilesets,
        color_mode: ColorMode::from_depth(depth),
        transparent_index: transparent,
        extra: Default::default(),
        color_profile: color_profile.unwrap_or_default(),
    };
    crate::document::slices::attach(bytes, &mut sprite);
    // The defaults (1:1 pixels, 16x16 grid at the origin) are stored as zeros so new sprites compare equal.
    sprite.extra.pixel_ratio =
        if pixel_ratio == (1, 1) || pixel_ratio.0 == 0 || pixel_ratio.1 == 0 { (0, 0) } else { pixel_ratio };
    sprite.extra.grid = if grid == (0, 0, 16, 16) { (0, 0, 0, 0) } else { grid };
    Ok(sprite)
}

/// `budget` is how many more bytes of decoded pixel data the file may still declare.
fn read_cel(c: &mut Reader, frame: usize, bpp: usize, budget: usize) -> Result<Option<PendingCel>> {
    let layer = c.u16()? as usize;
    let x = c.i16()? as i32;
    let y = c.i16()? as i32;
    let opacity = c.u8()?;
    let kind = c.u16()?;
    let z_index = c.i16()?;
    c.skip(5)?; // reserved
    let data = match kind {
        0 | 2 => {
            let w = c.u16()? as usize;
            let h = c.u16()? as usize;
            let len = w * h * bpp;
            if len > budget {
                return Err("cel is too large".into());
            }
            let data = if kind == 0 {
                c.bytes(len)?.to_vec()
            } else {
                let mut out = Vec::with_capacity(len.min(1 << 20));
                ZlibDecoder::new(c.rest())
                    .take(len as u64)
                    .read_to_end(&mut out)
                    .map_err(|e| format!("corrupt cel data: {e}"))?;
                out
            };
            if data.len() < len {
                return Err("truncated cel data".into());
            }
            CelData::Pixels { x, y, w, h, data }
        }
        1 => CelData::Linked(c.u16()? as usize),
        3 => CelData::Tilemap(tilemap::parse_tilemap_cel(c.rest(), x, y)?),
        _ => return Ok(None),
    };
    Ok(Some(PendingCel { frame, layer, opacity, z_index, data }))
}

fn read_tags(c: &mut Reader) -> Result<Vec<Tag>> {
    let n = c.u16()?;
    c.skip(8)?;
    (0..n)
        .map(|_| {
            let from = c.u16()?;
            let to = c.u16()?;
            let direction = c.u8()?;
            let repeat = c.u16()?;
            c.skip(6)?;
            let color = c.bytes(3)?.try_into().unwrap();
            c.skip(1)?;
            Ok(Tag { name: c.string()?, from, to, direction, repeat, color, user_data: Default::default() })
        })
        .collect()
}

fn read_palette(c: &mut Reader, palette: &mut Vec<[u8; 4]>) -> Result<()> {
    let size = c.u32()? as usize;
    let first = c.u32()? as usize;
    let last = c.u32()? as usize;
    c.skip(8)?;
    if size > 65536 {
        return Err("palette too large".into());
    }
    palette.resize(size, [0, 0, 0, 255]);
    for i in first..=last {
        let flags = c.u16()?;
        let rgba = c.bytes(4)?.try_into().unwrap();
        if flags & 1 != 0 {
            c.string()?;
        }
        if let Some(entry) = palette.get_mut(i) {
            *entry = rgba;
        }
    }
    Ok(())
}

/// Old palette chunks: 8-bit components (0x0004) or 6-bit ones (0x0011).
fn read_old_palette(c: &mut Reader, palette: &mut Vec<[u8; 4]>, six_bit: bool) -> Result<()> {
    let scale = |v: u8| if six_bit { (v << 2) | (v >> 4) } else { v };
    let packets = c.u16()?;
    let mut index = 0;
    for _ in 0..packets {
        index += c.u8()? as usize;
        let count = match c.u8()? {
            0 => 256,
            n => n as usize,
        };
        for _ in 0..count {
            let rgb = c.bytes(3)?;
            if palette.len() <= index {
                palette.resize(index + 1, [0, 0, 0, 255]);
            }
            palette[index] = [scale(rgb[0]), scale(rgb[1]), scale(rgb[2]), 255];
            index += 1;
        }
    }
    Ok(())
}

fn to_rgba(data: &[u8], depth: u16, palette: &[[u8; 4]], transparent: u8, background: bool) -> Vec<u8> {
    match depth {
        32 => data.to_vec(),
        16 => data.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        _ => data
            .iter()
            .flat_map(|&i| {
                if i == transparent && !background {
                    [0; 4]
                } else {
                    palette.get(i as usize).copied().unwrap_or([0, 0, 0, 255])
                }
            })
            .collect(),
    }
}

/// Copies a `w`x`h` image at (`x`, `y`) into a canvas, clipping to its bounds.
#[allow(clippy::too_many_arguments)]
fn blit(dst: &mut [u8], dw: usize, dh: usize, src: &[u8], x: i32, y: i32, w: usize, h: usize) {
    let x0 = x.max(0);
    let x1 = (x + w as i32).min(dw as i32);
    if x0 >= x1 {
        return;
    }
    let n = (x1 - x0) as usize * 4;
    for row in 0..h {
        let ty = y + row as i32;
        if ty < 0 || ty >= dh as i32 {
            continue;
        }
        let s = (row * w + (x0 - x) as usize) * 4;
        let d = (ty as usize * dw + x0 as usize) * 4;
        dst[d..d + n].copy_from_slice(&src[s..s + n]);
    }
}

fn put_u16(b: &mut Vec<u8>, v: u16) {
    b.extend(v.to_le_bytes());
}

fn put_u32(b: &mut Vec<u8>, v: u32) {
    b.extend(v.to_le_bytes());
}

fn put_str(b: &mut Vec<u8>, s: &str) {
    let bytes = &s.as_bytes()[..s.len().min(u16::MAX as usize)];
    put_u16(b, bytes.len() as u16);
    b.extend(bytes);
}

fn chunk(kind: u16, body: Vec<u8>) -> Vec<u8> {
    let mut c = Vec::with_capacity(body.len() + 6);
    put_u32(&mut c, body.len() as u32 + 6);
    put_u16(&mut c, kind);
    c.extend(body);
    c
}

fn header(frames: usize, width: u32, height: u32, depth: u16, colors: usize) -> Vec<u8> {
    let mut h = Vec::with_capacity(128);
    put_u32(&mut h, 0); // file size, patched by the caller
    put_u16(&mut h, FILE_MAGIC);
    put_u16(&mut h, frames as u16);
    put_u16(&mut h, width as u16);
    put_u16(&mut h, height as u16);
    put_u16(&mut h, depth);
    put_u32(&mut h, 1 | 2); // layer opacity is valid, also for groups (opacity and blend mode)
    put_u16(&mut h, 100); // deprecated speed
    put_u32(&mut h, 0);
    put_u32(&mut h, 0);
    h.extend([0; 4]); // transparent index + reserved
    put_u16(&mut h, colors.min(u16::MAX as usize) as u16);
    h.extend([1, 1]); // pixel ratio
    put_u16(&mut h, 0);
    put_u16(&mut h, 0);
    put_u16(&mut h, 16);
    put_u16(&mut h, 16);
    h.resize(128, 0);
    h
}

fn frame(chunks: Vec<Vec<u8>>, duration: u16) -> Vec<u8> {
    let mut f = Vec::new();
    put_u32(&mut f, 16 + chunks.iter().map(Vec::len).sum::<usize>() as u32);
    put_u16(&mut f, FRAME_MAGIC);
    put_u16(&mut f, chunks.len().min(0xFFFF) as u16);
    put_u16(&mut f, duration);
    f.extend([0; 2]);
    put_u32(&mut f, chunks.len() as u32);
    f.extend(chunks.concat());
    f
}

fn palette_body(palette: &[[u8; 4]]) -> Vec<u8> {
    let mut b = Vec::new();
    put_u32(&mut b, palette.len() as u32);
    put_u32(&mut b, 0);
    put_u32(&mut b, palette.len() as u32 - 1);
    b.extend([0; 8]);
    for c in palette {
        put_u16(&mut b, 0);
        b.extend(c);
    }
    b
}

fn layer_body(l: &Layer) -> Vec<u8> {
    let mut b = Vec::new();
    put_u16(&mut b, l.flags | l.visible as u16);
    put_u16(
        &mut b,
        if l.is_group() {
            1
        } else if l.kind == tilemap::KIND_TILEMAP {
            2
        } else {
            0
        },
    );
    put_u16(&mut b, l.child_level);
    put_u32(&mut b, 0); // default width/height (ignored)
    put_u16(&mut b, l.blend_mode);
    b.push(l.opacity);
    b.extend([0; 3]);
    put_str(&mut b, &l.name);
    if l.kind == tilemap::KIND_TILEMAP {
        put_u32(&mut b, l.tileset);
    }
    b
}

fn tags_body(tags: &[Tag]) -> Vec<u8> {
    let mut b = Vec::new();
    put_u16(&mut b, tags.len() as u16);
    b.extend([0; 8]);
    for t in tags {
        put_u16(&mut b, t.from);
        put_u16(&mut b, t.to);
        b.push(t.direction);
        put_u16(&mut b, t.repeat);
        b.extend([0; 6]);
        b.extend(t.color);
        b.push(0);
        put_str(&mut b, &t.name);
    }
    b
}

/// Writes a cel trimmed to its non-transparent bounds, including the part beyond the canvas (one
/// transparent pixel if it is empty).
fn cel_body(layer: usize, cel: &Cel, w: usize, h: usize, enc: &mut Encoder, background: bool) -> Vec<u8> {
    // The file stores x and y as signed 16-bit values and the size as unsigned 16-bit ones.
    let fits = |(x, y, fw, fh): (i32, i32, usize, usize)| {
        fw <= u16::MAX as usize && fh <= u16::MAX as usize && x >= i16::MIN as i32 && y >= i16::MIN as i32
    };
    let (x0, y0, cw, ch) = match cel.full_bounds(w, h) {
        Some(b) if fits(b) => b,
        // Too far or too large for the format: only what is visible inside the canvas is stored.
        Some(_) => {
            let mut visible = cel.clone();
            visible.outside = None;
            visible.full_bounds(w, h).unwrap_or((0, 0, 1, 1))
        }
        // An empty cel is still a cel (a keyframe, possibly with user data): store one transparent pixel.
        None => (0, 0, 1, 1),
    };
    let hints = cel.indices.as_deref().filter(|i| i.len() == w * h);
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    if cel.outside.is_none() || (x0 >= 0 && y0 >= 0 && x0 as usize + cw <= w && y0 as usize + ch <= h) {
        let (x0, y0) = (x0 as usize, y0 as usize);
        for y in y0..y0 + ch {
            let s = (y * w + x0) * 4;
            let row_hints = hints.map(|i| &i[y * w + x0..y * w + x0 + cw]);
            z.write_all(&enc.encode_cel_row(background, &cel.pixels[s..s + cw * 4], row_hints))
                .expect("in-memory write");
        }
    } else {
        // Content beyond the canvas: encode the full image row by row (palette index hints only
        // exist for the part inside the canvas).
        let full = cel.read_region(w, h, (x0, y0, cw, ch));
        for row in 0..ch {
            let ay = y0 as i64 + row as i64;
            let hint_row: Option<Vec<u8>> = hints.map(|hi| {
                (0..cw)
                    .map(|c| {
                        let ax = x0 as i64 + c as i64;
                        if ax >= 0 && ay >= 0 && ax < w as i64 && ay < h as i64 {
                            hi[ay as usize * w + ax as usize]
                        } else {
                            0
                        }
                    })
                    .collect()
            });
            let line = &full[row * cw * 4..(row + 1) * cw * 4];
            z.write_all(&enc.encode_cel_row(background, line, hint_row.as_deref())).expect("in-memory write");
        }
    }
    let mut b = Vec::new();
    put_u16(&mut b, layer as u16);
    put_u16(&mut b, x0 as i16 as u16);
    put_u16(&mut b, y0 as i16 as u16);
    b.push(cel.opacity);
    put_u16(&mut b, 2); // compressed image
    put_u16(&mut b, cel.z_index as u16);
    b.extend([0; 5]);
    put_u16(&mut b, cw as u16);
    put_u16(&mut b, ch as u16);
    b.extend(z.finish().expect("in-memory write"));
    b
}

/// Writes a link to the cel of the same layer at an earlier frame.
fn linked_cel_body(layer: usize, cel: &Cel, source: usize) -> Vec<u8> {
    let mut b = Vec::new();
    put_u16(&mut b, layer as u16);
    b.extend([0; 4]); // x, y
    b.push(cel.opacity);
    put_u16(&mut b, 1); // linked cel
    put_u16(&mut b, cel.z_index as u16);
    b.extend([0; 5]);
    put_u16(&mut b, source as u16);
    b
}

/// Writes an .aseprite file in the sprite's color mode. Cels of a layer that share one buffer
/// are written as links. Tileset pixels use the same color mode. Unused tilesets are dropped.
pub fn write(s: &Sprite) -> Vec<u8> {
    let s = &*tilemap::normalized(s);
    let mut enc = Encoder::new(s);
    let mut out = header(s.frame_count(), s.width, s.height, enc.depth(), s.palette.len());
    out[28] = s.transparent_index;
    let (px, py) = s.extra.pixel_ratio;
    if px != 0 && py != 0 {
        out[34] = px;
        out[35] = py;
    }
    if s.extra.grid != (0, 0, 0, 0) {
        let (gx, gy, gw, gh) = s.extra.grid;
        out[36..38].copy_from_slice(&gx.to_le_bytes());
        out[38..40].copy_from_slice(&gy.to_le_bytes());
        out[40..42].copy_from_slice(&gw.to_le_bytes());
        out[42..44].copy_from_slice(&gh.to_le_bytes());
    }
    let mut written: HashMap<(usize, *const Cel), usize> = HashMap::new();
    for f in 0..s.frame_count() {
        let mut chunks = Vec::new();
        if f == 0 {
            if let Some(body) = s.color_profile.chunk_body() {
                chunks.push(chunk(CHUNK_COLOR_PROFILE, body));
            }
            if !s.palette.is_empty() {
                chunks.push(chunk(CHUNK_PALETTE, palette_body(&s.palette)));
            }
            chunks.extend(tilemap::tileset_bodies(s, &mut enc).into_iter().map(|b| chunk(tilemap::CHUNK_TILESET, b)));
            chunks.extend(s.layers.iter().map(|l| chunk(CHUNK_LAYER, layer_body(l))));
            if !s.tags.is_empty() {
                chunks.push(chunk(CHUNK_TAGS, tags_body(&s.tags)));
            }
        }
        for (li, cel) in s.cels[f].iter().enumerate() {
            let Some(cel) = cel else { continue };
            let background = s.layers[li].flags & 8 != 0;
            if s.layers[li].kind == tilemap::KIND_TILEMAP {
                if let Some(mut body) = tilemap::cel_body(li, cel) {
                    // Offset 9 of a cel chunk: layer, x, y, opacity, type, then the z-index.
                    body[9..11].copy_from_slice(&cel.z_index.to_le_bytes());
                    chunks.push(chunk(CHUNK_CEL, body));
                }
            } else if let Some(&source) = written.get(&(li, Arc::as_ptr(cel))) {
                chunks.push(chunk(CHUNK_CEL, linked_cel_body(li, cel, source)));
            } else {
                let body = cel_body(li, cel, s.width as usize, s.height as usize, &mut enc, background);
                written.insert((li, Arc::as_ptr(cel)), f);
                chunks.push(chunk(CHUNK_CEL, body));
            }
        }
        out.extend(frame(chunks, s.durations[f]));
    }
    let len = out.len() as u32;
    out[..4].copy_from_slice(&len.to_le_bytes());
    crate::document::slices::inject(s, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_document() {
        let mut s = Sprite::new(8, 6);
        s.draw_line(0, 0, 1, 1, 6, 4, [255, 0, 0, 255], 1);
        let top = s.add_layer(0);
        s.layers[top].name = "Top".into();
        s.layers[top].opacity = 128;
        s.layers[top].visible = false;
        s.fill(top, 0, 0, 0, [0, 0, 255, 200]);
        let f = s.add_frame(0, true);
        s.durations[f] = 250;
        s.tags.push(Tag {
            name: "walk".into(),
            from: 0,
            to: 1,
            direction: 0,
            repeat: 0,
            color: [1, 2, 3],
            user_data: Default::default(),
        });
        assert_eq!(read(&write(&s)).unwrap(), s);
    }

    #[test]
    fn reads_indexed_raw_and_linked_cels() {
        let mut cel = Vec::new();
        for v in [0, 1, 0] {
            put_u16(&mut cel, v); // layer, x, y
        }
        cel.push(255);
        put_u16(&mut cel, 0); // raw
        cel.extend([0; 7]);
        put_u16(&mut cel, 2);
        put_u16(&mut cel, 1);
        cel.extend([1, 0]);
        let mut link = Vec::new();
        for v in [0, 0, 0] {
            put_u16(&mut link, v);
        }
        link.push(255);
        put_u16(&mut link, 1); // linked
        link.extend([0; 7]);
        put_u16(&mut link, 0);

        let palette = [[0, 0, 0, 255], [10, 20, 30, 255]];
        let mut file = header(2, 3, 1, 8, palette.len());
        file.extend(frame(
            vec![
                chunk(CHUNK_PALETTE, palette_body(&palette)),
                chunk(CHUNK_LAYER, layer_body(&Layer::new("L"))),
                chunk(CHUNK_CEL, cel),
            ],
            100,
        ));
        file.extend(frame(vec![chunk(CHUNK_CEL, link)], 50));

        let s = read(&file).unwrap();
        let expected = vec![0, 0, 0, 0, 10, 20, 30, 255, 0, 0, 0, 0];
        assert_eq!(s.render(0), expected);
        assert_eq!(s.render(1), expected);
        assert_eq!(s.durations, vec![100, 50]);
    }

    /// Hand-built indexed file whose palette has duplicate colors: [clear, red, red, blue].
    fn duplicate_palette_file(indices: &[u8]) -> Vec<u8> {
        let mut cel = Vec::new();
        for v in [0, 0, 0] {
            put_u16(&mut cel, v); // layer, x, y
        }
        cel.push(255);
        put_u16(&mut cel, 0); // raw
        cel.extend([0; 7]);
        put_u16(&mut cel, indices.len() as u16);
        put_u16(&mut cel, 1);
        cel.extend(indices);
        let palette = [[0, 0, 0, 0], [255, 0, 0, 255], [255, 0, 0, 255], [0, 0, 255, 255]];
        let mut file = header(1, indices.len() as u32, 1, 8, palette.len());
        file.extend(frame(
            vec![
                chunk(CHUNK_PALETTE, palette_body(&palette)),
                chunk(CHUNK_LAYER, layer_body(&Layer::new("L"))),
                chunk(CHUNK_CEL, cel),
            ],
            100,
        ));
        let len = file.len() as u32;
        file[..4].copy_from_slice(&len.to_le_bytes());
        file
    }

    fn written_indices(bytes: &[u8]) -> Vec<u8> {
        read(bytes).unwrap().cels[0][0].as_ref().unwrap().indices.as_ref().unwrap().to_vec()
    }

    #[test]
    fn indexed_files_keep_duplicate_palette_indices_on_resave() {
        let original = duplicate_palette_file(&[2, 1, 3, 2, 1]);
        let s = read(&original).unwrap();
        assert_eq!(s.color_mode, ColorMode::Indexed);
        assert_eq!(written_indices(&write(&s)), vec![2, 1, 3, 2, 1]);
        let saved = write(&s);
        assert_eq!(write(&read(&saved).unwrap()), saved, "re-saving is stable");
        assert_eq!(read(&saved).unwrap().render(0), s.render(0));
    }

    #[test]
    fn edited_pixels_drop_their_remembered_index_only() {
        let s0 = read(&duplicate_palette_file(&[2, 1, 3, 2])).unwrap();
        let mut s = s0.clone();
        // Paint blue over pixel 0 (was index 2) and transparent over pixel 3 (was index 2).
        s.draw_line(0, 0, 0, 0, 0, 0, [0, 0, 255, 255], 1);
        s.draw_line(0, 0, 3, 0, 3, 0, [0, 0, 0, 0], 1);
        assert_eq!(written_indices(&write(&s)), vec![3, 1, 3, 0]);
        // Painting red again picks the first red entry; untouched red pixels keep theirs.
        s.draw_line(0, 0, 2, 0, 2, 0, [255, 0, 0, 255], 1);
        assert_eq!(written_indices(&write(&s)), vec![3, 1, 1, 0]);
    }

    #[test]
    fn remembered_indices_survive_color_mode_round_trips_and_undo() {
        let mut e = crate::document::Editor::new(read(&duplicate_palette_file(&[2, 1, 3])).unwrap());
        e.checkpoint();
        crate::document::color_mode::convert(&mut e.sprite, ColorMode::Rgba, 0);
        assert_eq!(e.sprite.color_mode, ColorMode::Rgba);
        crate::document::color_mode::convert(&mut e.sprite, ColorMode::Indexed, 0);
        assert_eq!(written_indices(&write(&e.sprite)), vec![2, 1, 3]);
        assert!(e.undo());
        assert_eq!(written_indices(&write(&e.sprite)), vec![2, 1, 3]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(read(b"hello").is_err());
        assert!(read(&[0; 128]).is_err());
    }

    fn file(width: u32, height: u32, frames: Vec<Vec<u8>>) -> Vec<u8> {
        let mut f = header(frames.len(), width, height, 32, 0);
        f.extend(frames.concat());
        let len = f.len() as u32;
        f[..4].copy_from_slice(&len.to_le_bytes());
        f
    }

    #[test]
    fn six_bit_old_palette_is_scaled_to_eight_bits() {
        let body = vec![1, 0, 0, 2, 63, 0, 0, 0, 32, 0];
        let s = read(&file(2, 1, vec![frame(vec![chunk(CHUNK_OLD_PALETTE_6BIT, body)], 100)])).unwrap();
        assert_eq!(s.palette, vec![[255, 0, 0, 255], [0, 130, 0, 255]]);
    }

    #[test]
    fn zero_frame_duration_uses_the_header_speed() {
        let s = read(&file(2, 1, vec![frame(vec![], 0), frame(vec![], 40)])).unwrap();
        assert_eq!(s.durations, vec![100, 40]);
    }

    #[test]
    fn absurd_canvas_sizes_are_rejected() {
        let e = read(&file(60000, 60000, vec![frame(vec![], 100)])).unwrap_err();
        assert!(e.contains("too large"), "{e}");
    }

    #[test]
    fn out_of_range_palette_indices_get_entries() {
        // An 8-bit sprite whose pixels use index 3 although the palette stores one color.
        let mut cel = Vec::new();
        for v in [0, 0, 0] {
            put_u16(&mut cel, v); // layer, x, y
        }
        cel.push(255);
        put_u16(&mut cel, 0); // raw
        cel.extend([0; 7]);
        put_u16(&mut cel, 1);
        put_u16(&mut cel, 1);
        cel.push(3);
        let mut f = header(1, 1, 1, 8, 1);
        f.extend(frame(
            vec![
                chunk(CHUNK_PALETTE, palette_body(&[[9, 9, 9, 255]])),
                chunk(CHUNK_LAYER, layer_body(&Layer::new("L"))),
                chunk(CHUNK_CEL, cel),
            ],
            100,
        ));
        let s = read(&f).unwrap();
        assert_eq!(s.palette.len(), 4);
        assert_eq!(s.render(0), vec![0, 0, 0, 255]);
        assert_eq!(read(&write(&s)).unwrap().render(0), s.render(0));
    }

    #[test]
    fn group_opacity_is_only_valid_with_the_header_flag() {
        let mut group = Layer::new("g");
        group.kind = 1;
        group.opacity = 0;
        let child = Layer::new("c");
        let build = |flags: u32| {
            let mut f = file(
                2,
                2,
                vec![frame(vec![chunk(CHUNK_LAYER, layer_body(&group)), chunk(CHUNK_LAYER, layer_body(&child))], 100)],
            );
            f[14..18].copy_from_slice(&flags.to_le_bytes());
            f
        };
        assert_eq!(read(&build(1)).unwrap().layers[0].opacity, 255, "older files store an unused 0");
        assert_eq!(read(&build(3)).unwrap().layers[0].opacity, 0);
    }

    #[test]
    fn reads_a_cel_that_sticks_out_of_the_canvas_and_keeps_it() {
        // 2x1 canvas, an RGBA cel of 4x1 at x = -1: pixels A B C D cover canvas x -1..3.
        let (a, b, c, d) = ([1, 0, 0, 255], [2, 0, 0, 255], [3, 0, 0, 255], [4, 0, 0, 255]);
        let mut cel = Vec::new();
        put_u16(&mut cel, 0); // layer
        put_u16(&mut cel, (-1i16) as u16);
        put_u16(&mut cel, 0);
        cel.push(255);
        put_u16(&mut cel, 0); // raw
        cel.extend([0; 7]);
        put_u16(&mut cel, 4);
        put_u16(&mut cel, 1);
        for px in [a, b, c, d] {
            cel.extend(px);
        }
        let mut file = header(1, 2, 1, 32, 0);
        file.extend(frame(vec![chunk(CHUNK_LAYER, layer_body(&Layer::new("L"))), chunk(CHUNK_CEL, cel)], 100));
        let len = file.len() as u32;
        file[..4].copy_from_slice(&len.to_le_bytes());
        let s = read(&file).unwrap();
        s.validate().unwrap();
        let cel = s.cels[0][0].as_ref().unwrap();
        assert_eq!(&cel.pixels[..], &[b, c].concat()[..]);
        assert_eq!(cel.full_bounds(2, 1), Some((-1, 0, 4, 1)));
        assert_eq!(cel.full_image(2, 1).unwrap().rgba, [a, b, c, d].concat());
        assert_eq!(read(&write(&s)).unwrap(), s);
    }

    #[test]
    fn empty_cels_survive_a_round_trip() {
        let mut s = Sprite::new(3, 3);
        s.cels[0][0] = Some(Arc::new(Cel {
            opacity: 200,
            pixels: vec![0; 36],
            tiles: None,
            user_data: Default::default(),
            indices: None,
            z_index: 0,
            outside: None,
        }));
        let back = read(&write(&s)).unwrap();
        assert_eq!(back.cels[0][0].as_ref().map(|c| (c.opacity, c.pixels.iter().all(|&p| p == 0))), Some((200, true)));
    }
}
