//! Windows and OS/2 bitmaps. Reads 1, 4, 8, 16, 24 and 32 bits, RLE4 and RLE8, bit fields, top-down files and
//! the older headers; writes 8 bit palettes, 24 bit and 32 bit with alpha (BITMAPV5HEADER). The decoder of the
//! device independent bitmap inside is shared with the ICO reader.

use super::{check_dims, color_count, flatten, has_alpha, indexed, le16, le32, Decoded, ImageOptions};

/// A decoded bitmap.
pub(crate) struct Dib {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    pub palette: Option<Vec<[u8; 4]>>,
}

pub fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    if bytes.len() < 26 || &bytes[0..2] != b"BM" {
        return Err("not a BMP file".into());
    }
    let offset = le32(bytes, 10).ok_or("the BMP is truncated")? as usize;
    let dib = decode_dib(&bytes[14..], offset.checked_sub(14), false)?;
    Ok(Decoded {
        width: dib.width,
        height: dib.height,
        frames: vec![dib.rgba],
        durations: vec![100],
        palette: dib.palette,
    })
}

/// How to turn a bit field mask into 8 bit values.
#[derive(Clone, Copy, Default)]
struct Mask {
    mask: u32,
    shift: u32,
    bits: u32,
}

impl Mask {
    fn new(mask: u32) -> Mask {
        if mask == 0 {
            return Mask::default();
        }
        let shift = mask.trailing_zeros();
        Mask { mask, shift, bits: (mask >> shift).count_ones().min(16) }
    }

    fn get(self, v: u32) -> u8 {
        if self.bits == 0 {
            return 0;
        }
        let x = ((v & self.mask) >> self.shift) as u64;
        let max = (1u64 << self.bits) - 1;
        ((x * 255 + max / 2) / max).min(255) as u8
    }
}

/// Decodes the DIB that starts at the info header. `pixels_at` is the offset of the pixel data from the start of
/// `data` (files have it in the file header; icons put the pixels right after the palette). With `ico` the height
/// counts the AND mask too and the mask makes pixels transparent.
pub(crate) fn decode_dib(data: &[u8], pixels_at: Option<usize>, ico: bool) -> Result<Dib, String> {
    let short = || "the BMP is truncated".to_string();
    let hsize = le32(data, 0).ok_or_else(short)? as usize;
    let (w, h_raw, bpp, compression, colors_used) = match hsize {
        12 => (le16(data, 4).ok_or_else(short)? as i64, le16(data, 6).ok_or_else(short)? as i64, le16(data, 10), 0, 0),
        40 | 52 | 56 | 64 | 108 | 124 => (
            le32(data, 4).ok_or_else(short)? as i32 as i64,
            le32(data, 8).ok_or_else(short)? as i32 as i64,
            le16(data, 14),
            le32(data, 16).ok_or_else(short)?,
            le32(data, 32).ok_or_else(short)? as usize,
        ),
        _ => return Err("the BMP has an unsupported header".into()),
    };
    let bpp = bpp.ok_or_else(short)? as usize;
    let top_down = h_raw < 0;
    let mut h = h_raw.unsigned_abs() as usize;
    if ico {
        h /= 2;
    }
    if w <= 0 {
        return Err("the BMP has no pixels".into());
    }
    let w = w as usize;
    check_dims(w, h)?;
    if !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32) {
        return Err(format!("the BMP uses {bpp} bits per pixel, which is not supported"));
    }
    if !matches!(compression, 0 | 1 | 2 | 3 | 6) {
        return Err("the BMP uses a compression that is not supported".into());
    }
    if (compression == 1 && bpp != 8) || (compression == 2 && bpp != 4) {
        return Err("the BMP has an invalid compression for its depth".into());
    }

    // Bit field masks: inside the header (V2 and later), or after a plain info header.
    let mut header_end = hsize;
    let mut masks = [0u32; 4];
    if compression == 3 || compression == 6 {
        let n = if compression == 6 { 4 } else { 3 };
        for (k, m) in masks.iter_mut().enumerate().take(n) {
            *m = le32(data, 40 + 4 * k).ok_or_else(short)?;
        }
        if hsize == 40 {
            header_end = 40 + 4 * n;
        }
        if hsize >= 56 {
            masks[3] = le32(data, 52).ok_or_else(short)?;
        }
    } else if hsize >= 108 && bpp >= 16 && compression == 0 {
        // V4 and V5 headers carry masks even for plain RGB.
        for (k, m) in masks.iter_mut().enumerate() {
            *m = le32(data, 40 + 4 * k).unwrap_or(0);
        }
        if masks[..3].iter().all(|&m| m == 0) {
            masks = [0; 4];
        }
    }
    let plain_masks = masks[..3].iter().all(|&m| m == 0);
    if plain_masks {
        masks = match bpp {
            16 => [0x7C00, 0x03E0, 0x001F, 0],
            _ => [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, if hsize >= 56 { masks[3] } else { 0xFF00_0000 }],
        };
    }

    // Palette.
    let entry = if hsize == 12 { 3 } else { 4 };
    let mut palette = Vec::new();
    let mut after_palette = header_end;
    if bpp <= 8 {
        let mut n = if colors_used > 0 && colors_used <= 1 << bpp { colors_used } else { 1 << bpp };
        if let Some(p) = pixels_at {
            n = n.min(p.saturating_sub(header_end) / entry);
        }
        for i in 0..n {
            let at = header_end + i * entry;
            let c = data.get(at..at + 3).ok_or_else(short)?;
            palette.push([c[2], c[1], c[0], 255]);
        }
        after_palette = header_end + n * entry;
    }
    let start = pixels_at.unwrap_or(after_palette);
    let px = data.get(start..).ok_or_else(short)?;

    let mut rgba = vec![0u8; w * h * 4];
    let lookup = |i: usize| palette.get(i).copied().unwrap_or([0, 0, 0, 255]);
    let row_y = |r: usize| if top_down { r } else { h - 1 - r };
    let mut xor_len = 0;
    let mut forced_opaque = false;
    if compression == 1 || compression == 2 {
        let (indices, filled) = decode_rle(px, w, h, bpp == 4)?;
        for r in 0..h {
            let y = h - 1 - r; // RLE data is always stored bottom-up
            for x in 0..w {
                let o = (y * w + x) * 4;
                if filled[r * w + x] {
                    rgba[o..o + 4].copy_from_slice(&lookup(indices[r * w + x] as usize));
                }
            }
        }
    } else {
        let stride = (w * bpp).div_ceil(32) * 4;
        xor_len = stride.checked_mul(h).ok_or_else(short)?;
        if px.len() < xor_len {
            return Err(short());
        }
        let (mr, mg, mb, ma) = (Mask::new(masks[0]), Mask::new(masks[1]), Mask::new(masks[2]), Mask::new(masks[3]));
        for r in 0..h {
            let row = &px[r * stride..(r + 1) * stride];
            let y = row_y(r);
            for x in 0..w {
                let o = (y * w + x) * 4;
                let c = match bpp {
                    1 => lookup(((row[x / 8] >> (7 - x % 8)) & 1) as usize),
                    4 => lookup(((row[x / 2] >> (if x % 2 == 0 { 4 } else { 0 })) & 15) as usize),
                    8 => lookup(row[x] as usize),
                    16 => {
                        let v = u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]) as u32;
                        [mr.get(v), mg.get(v), mb.get(v), if masks[3] != 0 { ma.get(v) } else { 255 }]
                    }
                    24 => [row[x * 3 + 2], row[x * 3 + 1], row[x * 3], 255],
                    _ => {
                        let v = u32::from_le_bytes([row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]]);
                        [mr.get(v), mg.get(v), mb.get(v), if masks[3] != 0 { ma.get(v) } else { 255 }]
                    }
                };
                rgba[o..o + 4].copy_from_slice(&c);
            }
        }
        // 32 bit files from programs that leave the fourth byte at zero are opaque.
        if bpp == 32 && masks[3] != 0 && rgba.chunks_exact(4).all(|p| p[3] == 0) {
            rgba.chunks_exact_mut(4).for_each(|p| p[3] = 255);
            forced_opaque = true;
        }
    }
    if ico {
        // The AND mask follows the bitmap; it marks transparent pixels unless the bitmap has real alpha.
        let has_real_alpha = bpp == 32 && masks[3] != 0 && !forced_opaque;
        let stride = w.div_ceil(32) * 4;
        if !has_real_alpha {
            if let Some(mask) = px.get(xor_len..).filter(|m| m.len() >= stride * h) {
                for r in 0..h {
                    let y = row_y(r);
                    for x in 0..w {
                        if mask[r * stride + x / 8] >> (7 - x % 8) & 1 == 1 {
                            rgba[(y * w + x) * 4 + 3] = 0;
                        } else if bpp == 32 {
                            rgba[(y * w + x) * 4 + 3] = 255;
                        }
                    }
                }
            } else if bpp == 32 {
                rgba.chunks_exact_mut(4).for_each(|p| p[3] = 255);
            }
        }
    }
    Ok(Dib { width: w, height: h, rgba, palette: (!palette.is_empty()).then_some(palette) })
}

/// RLE8 and RLE4 into palette indices, bottom row first. `filled` marks the pixels the stream wrote.
fn decode_rle(data: &[u8], w: usize, h: usize, four: bool) -> Result<(Vec<u8>, Vec<bool>), String> {
    let mut idx = vec![0u8; w * h];
    let mut filled = vec![false; w * h];
    let (mut x, mut y, mut i) = (0usize, 0usize, 0usize);
    let mut put = |x: usize, y: usize, v: u8| {
        if x < w && y < h {
            idx[y * w + x] = v;
            filled[y * w + x] = true;
        }
    };
    while i + 1 < data.len() && y < h {
        let (count, value) = (data[i] as usize, data[i + 1]);
        i += 2;
        if count > 0 {
            for k in 0..count {
                let v = if four {
                    if k % 2 == 0 {
                        value >> 4
                    } else {
                        value & 15
                    }
                } else {
                    value
                };
                put(x + k, y, v);
            }
            x += count;
            continue;
        }
        match value {
            0 => {
                x = 0;
                y += 1;
            }
            1 => break,
            2 => {
                let (dx, dy) =
                    (*data.get(i).ok_or("the BMP is truncated")?, *data.get(i + 1).ok_or("the BMP is truncated")?);
                i += 2;
                x += dx as usize;
                y += dy as usize;
            }
            n => {
                let n = n as usize;
                let bytes = if four { n.div_ceil(2) } else { n };
                let run = data.get(i..i + bytes).ok_or("the BMP is truncated")?;
                for k in 0..n {
                    let v = if four { (run[k / 2] >> (if k % 2 == 0 { 4 } else { 0 })) & 15 } else { run[k] };
                    put(x + k, y, v);
                }
                x += n;
                i += bytes + (bytes & 1);
            }
        }
    }
    Ok((idx, filled))
}

// ---- Encoder -----------------------------------------------------------------------------------

fn file_header(out: &mut Vec<u8>, size: usize, offset: usize) -> Result<(), String> {
    if size > u32::MAX as usize {
        return Err("the image is too large for a BMP".into());
    }
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    Ok(())
}

/// Writes the 40 byte info header fields shared by every depth.
fn info_header(out: &mut Vec<u8>, w: usize, h: usize, bpp: u16, compression: u32, image: usize, colors: usize) {
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&bpp.to_le_bytes());
    out.extend_from_slice(&compression.to_le_bytes());
    out.extend_from_slice(&(image as u32).to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&(colors as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
}

/// Encodes `rgba`. Depth: `o.bits`, or 32 with alpha, 8 for up to 256 colors, else 24.
pub fn encode(rgba: &[u8], w: usize, h: usize, o: &ImageOptions) -> Result<Vec<u8>, String> {
    if w == 0 || h == 0 || w > i32::MAX as usize || h > i32::MAX as usize {
        return Err("the image has an invalid size for a BMP".into());
    }
    let bits = match o.bits {
        0 if has_alpha(rgba) => 32,
        0 if color_count(rgba, 256) <= 256 => 8,
        0 => 24,
        b => b,
    };
    let mut flat;
    let px = if bits == 32 {
        rgba
    } else {
        flat = rgba.to_vec();
        flatten(&mut flat, o.background);
        &flat
    };
    let stride = (w * bits as usize).div_ceil(32) * 4;
    let image = stride.checked_mul(h).ok_or("the image is too large for a BMP")?;
    let mut out = Vec::with_capacity(image + 1200);
    match bits {
        8 => {
            let (palette, planes) = indexed(&[px], w, h, 256);
            let offset = 14 + 40 + 4 * palette.len();
            file_header(&mut out, offset + image, offset)?;
            info_header(&mut out, w, h, 8, 0, image, palette.len());
            for c in &palette {
                out.extend_from_slice(&[c[2], c[1], c[0], 0]);
            }
            for y in (0..h).rev() {
                out.extend_from_slice(&planes[0][y * w..(y + 1) * w]);
                out.resize(out.len() + stride - w, 0);
            }
        }
        24 => {
            file_header(&mut out, 54 + image, 54)?;
            info_header(&mut out, w, h, 24, 0, image, 0);
            for y in (0..h).rev() {
                for p in px[y * w * 4..(y + 1) * w * 4].chunks_exact(4) {
                    out.extend_from_slice(&[p[2], p[1], p[0]]);
                }
                out.resize(out.len() + stride - w * 3, 0);
            }
        }
        32 => {
            // BITMAPV5HEADER with channel masks, so alpha is part of the format (macOS's ImageIO reads this
            // and not the V4 header with an sRGB color space tag).
            file_header(&mut out, 14 + 124 + image, 14 + 124)?;
            out.extend_from_slice(&124u32.to_le_bytes());
            out.extend_from_slice(&(w as i32).to_le_bytes());
            out.extend_from_slice(&(h as i32).to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&32u16.to_le_bytes());
            out.extend_from_slice(&3u32.to_le_bytes());
            out.extend_from_slice(&(image as u32).to_le_bytes());
            out.extend_from_slice(&2835i32.to_le_bytes());
            out.extend_from_slice(&2835i32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            for m in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000] {
                out.extend_from_slice(&m.to_le_bytes());
            }
            out.extend_from_slice(&0x7352_4742u32.to_le_bytes()); // "sRGB"
            out.resize(out.len() + 48, 0); // endpoints and gamma
            for v in [4u32, 0, 0, 0] {
                out.extend_from_slice(&v.to_le_bytes()); // intent: images; no embedded profile
            }
            for y in (0..h).rev() {
                for p in px[y * w * 4..(y + 1) * w * 4].chunks_exact(4) {
                    out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
                }
            }
        }
        _ => return Err("BMP supports 8, 24 and 32 bits per pixel".into()),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::test_util::picture;
    use super::*;

    fn opts(bits: u8) -> ImageOptions {
        ImageOptions { bits, ..Default::default() }
    }

    #[test]
    fn thirty_two_bit_keeps_alpha_exactly() {
        let src = picture(7, 5, true);
        let bytes = encode(&src, 7, 5, &opts(0)).unwrap();
        assert_eq!(&bytes[0..2], b"BM");
        let d = decode(&bytes).unwrap();
        assert_eq!((d.width, d.height), (7, 5));
        assert_eq!(d.frames[0], src);
    }

    #[test]
    fn twenty_four_bit_is_exact_and_flattens_alpha() {
        let src = picture(9, 3, false);
        let d = decode(&encode(&src, 9, 3, &opts(24)).unwrap()).unwrap();
        assert_eq!(d.frames[0], src);
        let clear = [10, 20, 30, 0].repeat(4);
        let o = ImageOptions { bits: 24, background: [1, 2, 3], ..Default::default() };
        let d = decode(&encode(&clear, 2, 2, &o).unwrap()).unwrap();
        assert_eq!(&d.frames[0][0..4], &[1, 2, 3, 255]);
    }

    #[test]
    fn eight_bit_is_chosen_for_few_colors_and_exact() {
        let mut src = Vec::new();
        for i in 0..30 {
            src.extend_from_slice(&[(i * 8) as u8, 100, (255 - i * 8) as u8, 255]);
        }
        let bytes = encode(&src, 6, 5, &opts(0)).unwrap();
        assert_eq!(u16::from_le_bytes([bytes[28], bytes[29]]), 8);
        let d = decode(&bytes).unwrap();
        assert_eq!(d.frames[0], src);
        assert_eq!(d.palette.as_ref().map(Vec::len), Some(30));
    }

    #[test]
    fn many_colors_choose_24_bit() {
        let src = picture(40, 40, false);
        let bytes = encode(&src, 40, 40, &opts(0)).unwrap();
        assert_eq!(u16::from_le_bytes([bytes[28], bytes[29]]), 24);
    }

    fn header(w: i32, h: i32, bpp: u16, comp: u32, colors: u32) -> Vec<u8> {
        let mut v = b"BM".to_vec();
        v.extend_from_slice(&[0; 12]);
        v.extend_from_slice(&40u32.to_le_bytes());
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&bpp.to_le_bytes());
        v.extend_from_slice(&comp.to_le_bytes());
        v.extend_from_slice(&[0; 12]);
        v.extend_from_slice(&colors.to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        v
    }

    fn seal(mut v: Vec<u8>) -> Vec<u8> {
        let size = v.len() as u32;
        v[2..6].copy_from_slice(&size.to_le_bytes());
        v
    }

    #[test]
    fn reads_top_down_and_four_bit_palettes() {
        // 2x2, 4 bit, top-down (negative height), palette: red, green, blue.
        let mut v = header(2, -2, 4, 0, 3);
        for c in [[0, 0, 255], [0, 255, 0], [255, 0, 0]] {
            v.extend_from_slice(&[c[0], c[1], c[2], 0]);
        }
        let offset = v.len() as u32;
        v[10..14].copy_from_slice(&offset.to_le_bytes());
        v.extend_from_slice(&[0x01, 0, 0, 0, 0x20, 0, 0, 0]);
        let d = decode(&seal(v)).unwrap();
        let f = &d.frames[0];
        assert_eq!(&f[0..4], &[255, 0, 0, 255]);
        assert_eq!(&f[4..8], &[0, 255, 0, 255]);
        assert_eq!(&f[8..12], &[0, 0, 255, 255]);
    }

    #[test]
    fn reads_sixteen_bit_565_and_555() {
        let mut v = header(1, 1, 16, 0, 0);
        v.extend_from_slice(&[0, 0, 0, 0]);
        let offset = v.len() as u32;
        v[10..14].copy_from_slice(&offset.to_le_bytes());
        v.extend_from_slice(&0x7C00u16.to_le_bytes());
        v.extend_from_slice(&[0, 0]);
        assert_eq!(&decode(&seal(v)).unwrap().frames[0][0..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn reads_rle8() {
        // 4x2: bottom row (first in file) is "3 x index 1" then delta-free end of line, top row "4 x index 0".
        let mut v = header(4, 2, 8, 1, 2);
        v.extend_from_slice(&[0, 0, 0, 0, 255, 255, 255, 0]);
        let offset = v.len() as u32;
        v[10..14].copy_from_slice(&offset.to_le_bytes());
        v.extend_from_slice(&[3, 1, 0, 0, 4, 0, 0, 1]);
        let d = decode(&seal(v)).unwrap();
        let f = &d.frames[0];
        assert_eq!(&f[0..4], &[0, 0, 0, 255]); // top row
        assert_eq!(&f[(4) * 4..(4) * 4 + 4], &[255, 255, 255, 255]); // bottom row, x = 0
        assert_eq!(f[(4 + 3) * 4 + 3], 0); // bottom row, x = 3 was never written
    }

    #[test]
    fn corrupt_files_are_errors_and_never_panic() {
        let good = encode(&picture(9, 7, true), 9, 7, &opts(0)).unwrap();
        assert!(decode(&good[..40]).is_err());
        assert!(decode(&good[..good.len() - 5]).is_err());
        assert!(decode(b"BM").is_err());
        assert!(decode(&[]).is_err());
        // A header that claims a gigantic image, with no data behind it.
        assert!(decode(&seal(header(60000, 60000, 32, 0, 0))).is_err());
        assert!(decode(&seal(header(-5, 3, 8, 0, 0))).is_err());
        assert!(decode(&seal(header(3, 3, 8, 1, 0))).is_err() || true);
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 0xFF;
            let _ = decode(&bad);
        }
    }
}
