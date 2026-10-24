//! ZSoft PCX. Reads 1, 2, 4 and 8 bit images (packed or in planes, with the 16 color header palette or the 256
//! color palette at the end) and 24 bit RGB in three planes; writes 8 bit with a 256 color palette and 24 bit.
//! PCX has no alpha: transparent pixels are flattened onto the background color.

use super::{check_dims, color_count, flatten, indexed, le16, Decoded, ImageOptions};

pub fn decode(b: &[u8]) -> Result<Decoded, String> {
    let short = || "the PCX is truncated".to_string();
    if b.len() < 129 || b[0] != 0x0A {
        return Err("not a PCX file".into());
    }
    let (rle, bpp, planes) = (b[2], b[3] as usize, b[65] as usize);
    let (x0, y0, x1, y1) =
        (le16(b, 4).unwrap_or(0), le16(b, 6).unwrap_or(0), le16(b, 8).unwrap_or(0), le16(b, 10).unwrap_or(0));
    let bpl = le16(b, 66).unwrap_or(0) as usize;
    if x1 < x0 || y1 < y0 {
        return Err("the PCX has an invalid size".into());
    }
    let (w, h) = ((x1 - x0) as usize + 1, (y1 - y0) as usize + 1);
    check_dims(w, h)?;
    if !matches!(bpp, 1 | 2 | 4 | 8) || !(1..=4).contains(&planes) || rle > 1 {
        return Err("the PCX uses a format that is not supported".into());
    }
    let min_bpl = (w * bpp).div_ceil(8);
    if bpl < min_bpl || bpl > 2 * min_bpl + 2 {
        return Err("the PCX has an invalid scan line length".into());
    }
    let total = h * planes * bpl;
    let has_vga = bpp == 8 && planes == 1 && b.len() >= 129 + 768 && b[b.len() - 769] == 0x0C;
    let end = if has_vga { b.len() - 769 } else { b.len() };
    let data = &b[128..end];

    // Scan lines, each `planes * bpl` bytes.
    let mut raw = Vec::with_capacity(total);
    if rle == 0 {
        raw.extend_from_slice(data.get(..total).ok_or_else(short)?);
    } else {
        let mut i = 0;
        while raw.len() < total {
            let v = *data.get(i).ok_or_else(short)?;
            i += 1;
            if v & 0xC0 == 0xC0 {
                let n = (v & 0x3F) as usize;
                let r = *data.get(i).ok_or_else(short)?;
                i += 1;
                let room = (total - raw.len()).min(n);
                raw.resize(raw.len() + room, r);
            } else {
                raw.push(v);
            }
        }
    }

    // Palettes.
    let header_pal: Vec<[u8; 4]> = (0..16).map(|i| [b[16 + 3 * i], b[17 + 3 * i], b[18 + 3 * i], 255]).collect();
    let palette: Vec<[u8; 4]> = if has_vga {
        b[b.len() - 768..].chunks_exact(3).map(|c| [c[0], c[1], c[2], 255]).collect()
    } else if bpp == 8 && planes == 1 {
        (0..=255u8).map(|i| [i, i, i, 255]).collect()
    } else if bpp == 1 && planes == 1 && header_pal[0] == header_pal[1] {
        vec![[0, 0, 0, 255], [255, 255, 255, 255]]
    } else {
        header_pal
    };
    let lookup = |i: usize| palette.get(i).copied().unwrap_or([0, 0, 0, 255]);

    let mut rgba = vec![255u8; w * h * 4];
    for y in 0..h {
        let line = &raw[y * planes * bpl..(y + 1) * planes * bpl];
        for x in 0..w {
            let c = if bpp == 8 && planes >= 3 {
                [line[x], line[bpl + x], line[2 * bpl + x], 255]
            } else if planes == 1 {
                let per = 8 / bpp;
                let shift = (per - 1 - x % per) * bpp;
                lookup(((line[x / per] >> shift) as usize) & ((1 << bpp) - 1))
            } else {
                // Planar: one bit (or more) per plane builds the index.
                let per = 8 / bpp;
                let shift = (per - 1 - x % per) * bpp;
                let mut index = 0;
                for p in 0..planes {
                    index |= (((line[p * bpl + x / per] >> shift) as usize) & ((1 << bpp) - 1)) << (p * bpp);
                }
                lookup(index)
            };
            rgba[(y * w + x) * 4..][..4].copy_from_slice(&c);
        }
    }
    let file_palette = (bpp <= 8 && planes == 1 && bpp * planes <= 8).then(|| palette.clone());
    Ok(Decoded { width: w, height: h, frames: vec![rgba], durations: vec![100], palette: file_palette })
}

fn rle_line(line: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < line.len() {
        let v = line[i];
        let mut run = 1;
        while i + run < line.len() && line[i + run] == v && run < 63 {
            run += 1;
        }
        if run > 1 || v >= 0xC0 {
            out.push(0xC0 | run as u8);
        }
        out.push(v);
        i += run;
    }
}

/// Encodes `rgba` as version 5 PCX: 8 bit with a palette when it has at most 256 colors (or `bits` says 8), else
/// 24 bit.
pub fn encode(rgba: &[u8], w: usize, h: usize, o: &ImageOptions) -> Result<Vec<u8>, String> {
    if w == 0 || h == 0 || w > u16::MAX as usize || h > u16::MAX as usize {
        return Err("the image is too large for a PCX (65535 pixels at most per side)".into());
    }
    let mut px = rgba.to_vec();
    flatten(&mut px, o.background);
    let eight = match o.bits {
        0 => color_count(&px, 256) <= 256,
        8 => true,
        24 | 32 => false,
        _ => return Err("PCX supports 8 and 24 bits per pixel".into()),
    };
    let bpl = w + (w & 1);
    let mut out = vec![0u8; 128];
    out[0] = 0x0A;
    out[1] = 5;
    out[2] = 1;
    out[3] = 8;
    out[8..10].copy_from_slice(&((w - 1) as u16).to_le_bytes());
    out[10..12].copy_from_slice(&((h - 1) as u16).to_le_bytes());
    out[12..14].copy_from_slice(&72u16.to_le_bytes());
    out[14..16].copy_from_slice(&72u16.to_le_bytes());
    out[65] = if eight { 1 } else { 3 };
    out[66..68].copy_from_slice(&(bpl as u16).to_le_bytes());
    out[68..70].copy_from_slice(&1u16.to_le_bytes());
    let mut line = vec![0u8; bpl];
    if eight {
        let (palette, planes) = indexed(&[&px], w, h, 256);
        for (i, c) in palette.iter().take(16).enumerate() {
            out[16 + 3 * i..19 + 3 * i].copy_from_slice(c);
        }
        for y in 0..h {
            line[..w].copy_from_slice(&planes[0][y * w..(y + 1) * w]);
            rle_line(&line, &mut out);
        }
        out.push(0x0C);
        for i in 0..256 {
            out.extend_from_slice(&palette.get(i).copied().unwrap_or([0, 0, 0]));
        }
    } else {
        for y in 0..h {
            for k in 0..3 {
                for x in 0..w {
                    line[x] = px[(y * w + x) * 4 + k];
                }
                rle_line(&line, &mut out);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::test_util::picture;
    use super::*;

    #[test]
    fn eight_and_twenty_four_bit_round_trip() {
        let mut few = Vec::new();
        for i in 0..35 {
            few.extend_from_slice(&[(i * 7) as u8, 0xC5, (i * 3) as u8, 255]);
        }
        let bytes = encode(&few, 7, 5, &ImageOptions::default()).unwrap();
        assert_eq!((bytes[3], bytes[65]), (8, 1));
        let d = decode(&bytes).unwrap();
        assert_eq!((d.width, d.height), (7, 5));
        assert_eq!(d.frames[0], few);
        assert!(d.palette.is_some());

        let many = picture(30, 20, false);
        let bytes = encode(&many, 30, 20, &ImageOptions::default()).unwrap();
        assert_eq!(bytes[65], 3);
        assert_eq!(decode(&bytes).unwrap().frames[0], many);
    }

    #[test]
    fn rle_handles_values_that_look_like_counts() {
        // A line of 0xC0..0xFF values needs escaping even when they are single.
        let mut px = Vec::new();
        for i in 0..8u8 {
            px.extend_from_slice(&[0xC0 + i, 0xFF, 0xC1, 255]);
        }
        let o = ImageOptions { bits: 24, ..Default::default() };
        assert_eq!(decode(&encode(&px, 8, 1, &o).unwrap()).unwrap().frames[0], px);
        assert_eq!(decode(&encode(&px, 8, 1, &ImageOptions::default()).unwrap()).unwrap().frames[0], px);
    }

    #[test]
    fn transparent_pixels_take_the_background() {
        let px = [9, 9, 9, 0].repeat(4);
        let o = ImageOptions { background: [10, 20, 30], ..Default::default() };
        let d = decode(&encode(&px, 2, 2, &o).unwrap()).unwrap();
        assert_eq!(&d.frames[0][0..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn reads_packed_four_bit_with_header_palette() {
        let mut v = vec![0u8; 128];
        v[0] = 0x0A;
        v[1] = 5;
        v[2] = 1;
        v[3] = 4;
        v[8] = 3; // 4 pixels wide
        v[10] = 0; // 1 pixel high
        v[65] = 1;
        v[66] = 2;
        v[16 + 3 * 5..16 + 3 * 5 + 3].copy_from_slice(&[1, 2, 3]);
        v.extend_from_slice(&[0x50, 0x05]); // indexes 5,0,0,5
        let d = decode(&v).unwrap();
        assert_eq!(&d.frames[0][0..4], &[1, 2, 3, 255]);
        assert_eq!(&d.frames[0][4..8], &[0, 0, 0, 255]);
        assert_eq!(&d.frames[0][12..16], &[1, 2, 3, 255]);
    }

    #[test]
    fn corrupt_files_are_errors_and_never_panic() {
        let good = encode(&picture(9, 7, false), 9, 7, &ImageOptions::default()).unwrap();
        assert!(decode(&good[..100]).is_err());
        assert!(decode(&good[..130]).is_err());
        assert!(decode(&[0x0A; 4]).is_err());
        let mut huge = good.clone();
        huge[8..12].copy_from_slice(&[255, 255, 255, 255]);
        assert!(decode(&huge).is_err());
        for i in 0..good.len().min(400) {
            let mut bad = good.clone();
            bad[i] ^= 0x77;
            let _ = decode(&bad);
        }
    }
}
