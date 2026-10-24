//! Truevision TGA. Reads color-mapped, true-color and gray images, plain or run-length encoded, 8 to 32 bits, any
//! origin; writes 8 bit color-mapped, 24 bit and 32 bit with alpha, with optional RLE, top-down, with a TGA 2.0
//! footer.

use super::{check_dims, color_count, flatten, has_alpha, indexed, le16, Decoded, ImageOptions};

const FOOTER: &[u8; 18] = b"TRUEVISION-XFILE.\0";

/// TGA has no signature; a file is taken for one when the header values make sense (or the footer says so).
pub fn looks_like_tga(b: &[u8]) -> bool {
    if b.len() < 18 {
        return false;
    }
    let (cmap_type, img_type, depth) = (b[1], b[2], b[16]);
    let (w, h) = (le16(b, 12).unwrap_or(0), le16(b, 14).unwrap_or(0));
    if cmap_type > 1 || w == 0 || h == 0 {
        return false;
    }
    let depth_ok = match img_type {
        1 | 9 => cmap_type == 1 && matches!(depth, 8 | 16),
        2 | 10 => matches!(depth, 15 | 16 | 24 | 32),
        3 | 11 => matches!(depth, 8 | 16),
        _ => false,
    };
    if !depth_ok {
        return false;
    }
    if cmap_type == 1 && !matches!(b[7], 15 | 16 | 24 | 32) {
        return false;
    }
    b.ends_with(FOOTER) || b[17] & 0xC0 == 0
}

pub fn decode(b: &[u8]) -> Result<Decoded, String> {
    let short = || "the TGA is truncated".to_string();
    if !looks_like_tga(b) {
        return Err("not a TGA file".into());
    }
    let (id_len, cmap_type, img_type) = (b[0] as usize, b[1], b[2]);
    let (cmap_first, cmap_len, cmap_depth) = (le16(b, 3).unwrap_or(0) as usize, le16(b, 5).unwrap_or(0) as usize, b[7]);
    let (w, h) = (le16(b, 12).unwrap_or(0) as usize, le16(b, 14).unwrap_or(0) as usize);
    let (depth, desc) = (b[16] as usize, b[17]);
    check_dims(w, h)?;
    let pb = depth.div_ceil(8);
    let mut pos = 18 + id_len;

    let mut palette: Vec<[u8; 4]> = Vec::new();
    if cmap_type == 1 {
        let eb = (cmap_depth as usize).div_ceil(8);
        let data = b.get(pos..pos + cmap_len * eb).ok_or_else(short)?;
        palette = data.chunks_exact(eb).map(|e| pixel(e, cmap_depth as usize, 0, true)).collect();
        pos += cmap_len * eb;
    }
    let total = w * h * pb;
    let raw: Vec<u8> = if img_type >= 9 {
        let mut out = Vec::with_capacity(total);
        while out.len() < total {
            let head = *b.get(pos).ok_or_else(short)?;
            pos += 1;
            let count = (head & 0x7F) as usize + 1;
            if head & 0x80 != 0 {
                let p = b.get(pos..pos + pb).ok_or_else(short)?;
                pos += pb;
                for _ in 0..count {
                    if out.len() < total {
                        out.extend_from_slice(p);
                    }
                }
            } else {
                let p = b.get(pos..pos + count * pb).ok_or_else(short)?;
                pos += count * pb;
                let room = (total - out.len()).min(p.len());
                out.extend_from_slice(&p[..room]);
            }
        }
        out
    } else {
        b.get(pos..pos + total).ok_or_else(short)?.to_vec()
    };

    let alpha_bits = (desc & 0x0F) as usize;
    let mut rgba = vec![0u8; w * h * 4];
    for i in 0..w * h {
        let src = &raw[i * pb..(i + 1) * pb];
        let c = match img_type & 7 {
            1 => {
                let index = if pb == 2 { u16::from_le_bytes([src[0], src[1]]) as usize } else { src[0] as usize };
                index.checked_sub(cmap_first).and_then(|k| palette.get(k)).copied().unwrap_or([0, 0, 0, 255])
            }
            3 => {
                if pb == 2 {
                    [src[0], src[0], src[0], src[1]]
                } else {
                    [src[0], src[0], src[0], 255]
                }
            }
            _ => pixel(src, depth, alpha_bits, false),
        };
        // Image origin: bit 5 set is top-left, bit 4 set is right-to-left.
        let (x, y) = (i % w, i / w);
        let (x, y) = (if desc & 0x10 != 0 { w - 1 - x } else { x }, if desc & 0x20 != 0 { y } else { h - 1 - y });
        rgba[(y * w + x) * 4..][..4].copy_from_slice(&c);
    }
    // 32 bit files whose attribute bits are zero often leave the alpha byte empty.
    if depth == 32 && rgba.chunks_exact(4).all(|p| p[3] == 0) {
        rgba.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    }
    Ok(Decoded {
        width: w,
        height: h,
        frames: vec![rgba],
        durations: vec![100],
        palette: (!palette.is_empty() && palette.len() <= 256)
            .then(|| palette.iter().map(|c| [c[0], c[1], c[2], 255]).collect()),
    })
}

/// One true-color pixel of `depth` bits; `cmap` entries have no alpha attribute unless 32 bit.
fn pixel(s: &[u8], depth: usize, alpha_bits: usize, cmap: bool) -> [u8; 4] {
    match depth {
        15 | 16 => {
            let v = u16::from_le_bytes([s[0], s[1]]) as u32;
            let e = |x: u32| ((x * 255 + 15) / 31) as u8;
            let a = if depth == 16 && alpha_bits >= 1 && !cmap && v & 0x8000 == 0 { 0 } else { 255 };
            [e((v >> 10) & 31), e((v >> 5) & 31), e(v & 31), a]
        }
        24 => [s[2], s[1], s[0], 255],
        _ => [s[2], s[1], s[0], s[3]],
    }
}

pub fn encode(rgba: &[u8], w: usize, h: usize, o: &ImageOptions) -> Result<Vec<u8>, String> {
    if w == 0 || h == 0 || w > u16::MAX as usize || h > u16::MAX as usize {
        return Err("the image is too large for a TGA (65535 pixels at most per side)".into());
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
    let (palette, planes) = if bits == 8 { indexed(&[px], w, h, 256) } else { (Vec::new(), Vec::new()) };
    let pb = match bits {
        8 => 1,
        24 => 3,
        32 => 4,
        _ => return Err("TGA supports 8, 24 and 32 bits per pixel".into()),
    };
    let mut out = Vec::new();
    out.push(0); // no image id
    out.push(u8::from(bits == 8));
    out.push(match (bits == 8, o.tga_rle) {
        (true, false) => 1,
        (true, true) => 9,
        (false, false) => 2,
        (false, true) => 10,
    });
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(palette.len() as u16).to_le_bytes());
    out.push(if bits == 8 { 24 } else { 0 });
    out.extend_from_slice(&[0, 0, 0, 0]); // origin
    out.extend_from_slice(&(w as u16).to_le_bytes());
    out.extend_from_slice(&(h as u16).to_le_bytes());
    out.push(bits);
    out.push(0x20 | if bits == 32 { 8 } else { 0 }); // top-left origin
    for c in &palette {
        out.extend_from_slice(&[c[2], c[1], c[0]]);
    }
    let mut row = Vec::with_capacity(w * pb);
    for y in 0..h {
        row.clear();
        match bits {
            8 => row.extend_from_slice(&planes[0][y * w..(y + 1) * w]),
            24 => {
                px[y * w * 4..(y + 1) * w * 4].chunks_exact(4).for_each(|p| row.extend_from_slice(&[p[2], p[1], p[0]]))
            }
            _ => px[y * w * 4..(y + 1) * w * 4]
                .chunks_exact(4)
                .for_each(|p| row.extend_from_slice(&[p[2], p[1], p[0], p[3]])),
        }
        if o.tga_rle {
            rle_row(&row, pb, &mut out);
        } else {
            out.extend_from_slice(&row);
        }
    }
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(FOOTER);
    Ok(out)
}

/// One scan line as RLE packets (packets never cross lines).
fn rle_row(row: &[u8], pb: usize, out: &mut Vec<u8>) {
    let n = row.len() / pb;
    let same = |a: usize, b: usize| row[a * pb..(a + 1) * pb] == row[b * pb..(b + 1) * pb];
    let mut i = 0;
    while i < n {
        let mut run = 1;
        while i + run < n && run < 128 && same(i, i + run) {
            run += 1;
        }
        if run >= 2 {
            out.push(0x80 | (run - 1) as u8);
            out.extend_from_slice(&row[i * pb..(i + 1) * pb]);
            i += run;
        } else {
            let start = i;
            let mut count = 1;
            i += 1;
            while i < n && count < 128 && !(i + 1 < n && same(i, i + 1)) {
                count += 1;
                i += 1;
            }
            out.push((count - 1) as u8);
            out.extend_from_slice(&row[start * pb..(start + count) * pb]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::picture;
    use super::*;

    fn opts(bits: u8, rle: bool) -> ImageOptions {
        ImageOptions { bits, tga_rle: rle, ..Default::default() }
    }

    #[test]
    fn all_depths_round_trip_with_and_without_rle() {
        let alpha = picture(11, 6, true);
        let opaque = picture(11, 6, false);
        let mut few = Vec::new();
        for i in 0..66 {
            few.extend_from_slice(&[(i % 5 * 50) as u8, (i % 3 * 100) as u8, 7, 255]);
        }
        for rle in [false, true] {
            let a = encode(&alpha, 11, 6, &opts(0, rle)).unwrap();
            assert_eq!(a[16], 32);
            assert_eq!(decode(&a).unwrap().frames[0], alpha);
            let o = encode(&opaque, 11, 6, &opts(24, rle)).unwrap();
            assert_eq!(o[16], 24);
            assert_eq!(decode(&o).unwrap().frames[0], opaque);
            let f = encode(&few, 11, 6, &opts(0, rle)).unwrap();
            assert_eq!(f[16], 8);
            let d = decode(&f).unwrap();
            assert_eq!(d.frames[0], few);
            assert!(d.palette.is_some());
        }
    }

    #[test]
    fn rle_shrinks_flat_images() {
        let flat = [30, 60, 90, 255].repeat(200 * 50);
        let raw = encode(&flat, 200, 50, &opts(24, false)).unwrap();
        let packed = encode(&flat, 200, 50, &opts(24, true)).unwrap();
        assert!(packed.len() < raw.len() / 10);
        assert_eq!(decode(&packed).unwrap().frames[0], flat);
    }

    #[test]
    fn detection_accepts_our_files_and_rejects_noise() {
        let t = encode(&picture(3, 3, false), 3, 3, &opts(24, true)).unwrap();
        assert!(looks_like_tga(&t));
        assert_eq!(super::super::detect(&t), Some(super::super::ImageFormat::Tga));
        assert!(!looks_like_tga(&[0u8; 18]));
        assert!(!looks_like_tga(b"hello"));
    }

    #[test]
    fn reads_bottom_up_gray_and_sixteen_bit() {
        // 2x1 gray, bottom-left origin.
        let mut v = vec![0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 8, 0];
        v.extend_from_slice(&[10, 200]);
        let d = decode(&v).unwrap();
        assert_eq!(d.frames[0], vec![10, 10, 10, 255, 200, 200, 200, 255]);
        // 1x1, 16 bit 555 red with the attribute bit set.
        let mut v = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 16, 1];
        v.extend_from_slice(&(0x8000u16 | 0x7C00).to_le_bytes());
        assert_eq!(decode(&v).unwrap().frames[0], vec![255, 0, 0, 255]);
    }

    #[test]
    fn corrupt_files_are_errors_and_never_panic() {
        let good = encode(&picture(9, 7, true), 9, 7, &opts(0, true)).unwrap();
        assert!(decode(&good[..30]).is_err());
        assert!(decode(&good[..good.len() / 2]).is_err());
        let mut huge = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255, 32, 0];
        huge.extend_from_slice(&[0; 40]);
        assert!(decode(&huge).is_err());
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 0xA5;
            let _ = decode(&bad);
        }
    }
}
