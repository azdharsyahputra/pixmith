//! Windows icons (and cursors, read only). An ICO file holds several sizes of one icon.
//!
//! Import opens one size: the largest unless the caller names one (`entries` lists what the file has, so a front
//! end can ask). Export writes the sizes the user picked, each one a square image that holds the sprite scaled by a
//! whole number to fit (or reduced when it is too large) and centered; sizes below 256 are 32 bit bitmaps with an
//! AND mask for old readers, 256 is a PNG as Windows does it. With no sizes picked, the icon has the sprite's size.

use super::bmp::decode_dib;
use super::{le16, le32, Decoded, ImageOptions};
use crate::formats::export::{check_export_size, encode_png, upscale};
use crate::formats::export_plan::PlanFrame;

/// One image of an icon file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IcoEntry {
    pub width: usize,
    pub height: usize,
    pub bits: u16,
    pub offset: usize,
    pub size: usize,
    pub png: bool,
}

/// Lists the images of an icon file (those whose data lies inside the file).
pub fn entries(b: &[u8]) -> Result<Vec<IcoEntry>, String> {
    if b.len() < 22 || b[0] != 0 || b[1] != 0 || !matches!(b[2], 1 | 2) {
        return Err("not an ICO file".into());
    }
    let count = le16(b, 4).unwrap_or(0) as usize;
    let mut out = Vec::new();
    for i in 0..count {
        let at = 6 + 16 * i;
        let Some(dir) = b.get(at..at + 16) else { break };
        let (size, offset) = (le32(dir, 8).unwrap_or(0) as usize, le32(dir, 12).unwrap_or(0) as usize);
        let Some(data) = offset.checked_add(size).and_then(|end| b.get(offset..end)).filter(|d| d.len() >= 16) else {
            continue;
        };
        let png = data.starts_with(&[0x89, b'P', b'N', b'G']);
        let (width, height) = if png {
            match (data.get(16..20), data.get(20..24)) {
                (Some(w), Some(h)) => (
                    u32::from_be_bytes([w[0], w[1], w[2], w[3]]) as usize,
                    u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as usize,
                ),
                _ => continue,
            }
        } else {
            (
                le32(data, 4).unwrap_or(0) as i32 as usize,
                (le32(data, 8).unwrap_or(0) as i32).unsigned_abs() as usize / 2,
            )
        };
        if width == 0 || height == 0 || width > 65535 || height > 65535 {
            continue;
        }
        let bits = if png { 32 } else { le16(data, 14).unwrap_or(0) };
        out.push(IcoEntry { width, height, bits, offset, size, png });
    }
    if out.is_empty() {
        return Err("the icon has no readable images".into());
    }
    Ok(out)
}

pub fn decode(b: &[u8], want: Option<u32>) -> Result<Decoded, String> {
    let list = entries(b)?;
    let pick = match want {
        Some(s) => list
            .iter()
            .filter(|e| e.width.max(e.height) == s as usize)
            .max_by_key(|e| e.bits)
            .ok_or(format!("the icon has no {s} pixel image"))?,
        None => list.iter().max_by_key(|e| (e.width * e.height, e.bits)).ok_or("the icon has no images")?,
    };
    let data = &b[pick.offset..pick.offset + pick.size];
    let (w, h, rgba) = if pick.png {
        crate::formats::import::decode_png(data)?
    } else {
        let d = decode_dib(data, None, true)?;
        (d.width, d.height, d.rgba)
    };
    Ok(Decoded::still(w, h, rgba))
}

/// `image` placed on a transparent `s` x `s` square: scaled up by a whole number, or reduced to fit.
fn fit(rgba: &[u8], w: usize, h: usize, s: usize) -> Vec<u8> {
    let longest = w.max(h);
    let (img, iw, ih) = if longest <= s {
        let k = s / longest;
        (upscale(rgba, w, h, k), w * k, h * k)
    } else {
        let (tw, th) = ((w * s).div_ceil(longest).max(1), (h * s).div_ceil(longest).max(1));
        let mut v = Vec::with_capacity(tw * th * 4);
        for y in 0..th {
            let sy = ((2 * y + 1) * h / (2 * th)).min(h - 1);
            for x in 0..tw {
                let sx = ((2 * x + 1) * w / (2 * tw)).min(w - 1);
                v.extend_from_slice(&rgba[(sy * w + sx) * 4..][..4]);
            }
        }
        (v, tw, th)
    };
    let mut out = vec![0u8; s * s * 4];
    let (ox, oy) = ((s - iw) / 2, (s - ih) / 2);
    for y in 0..ih {
        out[((y + oy) * s + ox) * 4..][..iw * 4].copy_from_slice(&img[y * iw * 4..(y + 1) * iw * 4]);
    }
    out
}

/// One entry's data: a PNG for 256 pixels, otherwise a 32 bit bitmap followed by the AND mask.
fn entry_data(rgba: &[u8], w: usize, h: usize) -> Result<Vec<u8>, String> {
    if w.max(h) >= 256 {
        return encode_png(rgba, w, h);
    }
    let and_stride = w.div_ceil(32) * 4;
    let mut v = Vec::with_capacity(40 + w * h * 4 + and_stride * h);
    v.extend_from_slice(&40u32.to_le_bytes());
    v.extend_from_slice(&(w as i32).to_le_bytes());
    v.extend_from_slice(&((2 * h) as i32).to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&32u16.to_le_bytes());
    v.extend_from_slice(&[0; 4]);
    v.extend_from_slice(&((w * h * 4 + and_stride * h) as u32).to_le_bytes());
    v.extend_from_slice(&[0; 16]);
    for y in (0..h).rev() {
        for p in rgba[y * w * 4..(y + 1) * w * 4].chunks_exact(4) {
            v.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    for y in (0..h).rev() {
        let mut row = vec![0u8; and_stride];
        for x in 0..w {
            if rgba[(y * w + x) * 4 + 3] == 0 {
                row[x / 8] |= 0x80 >> (x % 8);
            }
        }
        v.extend_from_slice(&row);
    }
    Ok(v)
}

/// An icon file with the sizes of `o` (or the frame's own size) made from one frame.
pub fn encode_frame(f: &PlanFrame, scale: usize, o: &ImageOptions) -> Result<Vec<u8>, String> {
    let sizes = o.ico_size_list();
    let images: Vec<(Vec<u8>, usize, usize)> = if sizes.is_empty() {
        let scale = scale.clamp(1, 64);
        let (w, h) = (f.w * scale, f.h * scale);
        check_export_size(w, h)?;
        if w > 256 || h > 256 {
            return Err(format!(
                "the image is {w} x {h} pixels, but an icon holds at most 256 x 256; pick icon sizes instead"
            ));
        }
        vec![(upscale(&f.rgba, f.w, f.h, scale), w, h)]
    } else {
        sizes.iter().map(|&s| (fit(&f.rgba, f.w, f.h, s as usize), s as usize, s as usize)).collect()
    };
    let datas: Vec<Vec<u8>> = images.iter().map(|(px, w, h)| entry_data(px, *w, *h)).collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for ((_, w, h), data) in images.iter().zip(&datas) {
        out.extend_from_slice(&[(*w % 256) as u8, (*h % 256) as u8, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += data.len();
    }
    for d in datas {
        out.extend_from_slice(&d);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{frame, picture};
    use super::*;

    fn sizes_of(bytes: &[u8]) -> Vec<(usize, usize)> {
        entries(bytes).unwrap().iter().map(|e| (e.width, e.height)).collect()
    }

    #[test]
    fn writes_the_picked_sizes_and_reads_each_back() {
        let src = picture(16, 16, true);
        let f = frame(src.clone(), 16, 16, 100);
        let mut o = ImageOptions::default();
        o.set_ico_sizes(&[32, 16, 256, 48]);
        let bytes = encode_frame(&f, 1, &o).unwrap();
        assert_eq!(sizes_of(&bytes), vec![(16, 16), (32, 32), (48, 48), (256, 256)]);
        assert!(entries(&bytes).unwrap().iter().map(|e| e.png).eq([false, false, false, true]));
        // Native size is exact, alpha included.
        let d = decode(&bytes, Some(16)).unwrap();
        assert_eq!((d.width, d.height), (16, 16));
        assert_eq!(d.frames[0], src);
        // 32 is an exact 2x upscale; 48 is 3x.
        let d = decode(&bytes, Some(32)).unwrap();
        assert_eq!(&d.frames[0][(2 * 32 + 2) * 4..][..4], &src[(16 + 1) * 4..][..4]);
        // The default is the largest.
        let d = decode(&bytes, None).unwrap();
        assert_eq!((d.width, d.height), (256, 256));
        assert!(decode(&bytes, Some(64)).is_err());
    }

    #[test]
    fn no_sizes_means_the_sprite_size_and_large_sprites_are_refused() {
        let f = frame(picture(20, 12, true), 20, 12, 100);
        let bytes = encode_frame(&f, 1, &ImageOptions::default()).unwrap();
        assert_eq!(sizes_of(&bytes), vec![(20, 12)]);
        assert_eq!(decode(&bytes, None).unwrap().frames[0], picture(20, 12, true));
        let big = frame(vec![0; 300 * 10 * 4], 300, 10, 100);
        assert!(encode_frame(&big, 1, &ImageOptions::default()).is_err());
        let mut o = ImageOptions::default();
        o.set_ico_sizes(&[64]);
        assert!(encode_frame(&big, 1, &o).is_ok());
    }

    #[test]
    fn non_square_sprites_are_centered_in_the_square() {
        let f = frame([255, 0, 0, 255].repeat(4 * 2), 4, 2, 100);
        let mut o = ImageOptions::default();
        o.set_ico_sizes(&[16]);
        let d = decode(&encode_frame(&f, 1, &o).unwrap(), None).unwrap();
        let at = |x: usize, y: usize| d.frames[0][(y * 16 + x) * 4 + 3];
        assert_eq!((at(8, 3), at(8, 12), at(8, 8)), (0, 0, 255)); // 4x2 becomes 16x8, rows 4..12
        assert_eq!((at(0, 8), at(15, 8)), (255, 255));
    }

    #[test]
    fn large_sprites_are_reduced_to_fit() {
        let f = frame(picture(64, 64, false), 64, 64, 100);
        let mut o = ImageOptions::default();
        o.set_ico_sizes(&[16]);
        let d = decode(&encode_frame(&f, 1, &o).unwrap(), None).unwrap();
        assert_eq!((d.width, d.height), (16, 16));
        assert!(d.frames[0].chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn reads_classic_and_masked_bitmaps() {
        // A 2x2, 24 bit BMP icon image with an AND mask that hides the bottom-left pixel.
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&2i32.to_le_bytes());
        dib.extend_from_slice(&4i32.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&24u16.to_le_bytes());
        dib.extend_from_slice(&[0; 24]);
        for _ in 0..2 {
            dib.extend_from_slice(&[0, 0, 255, 0, 0, 255, 0, 0]); // two red pixels + row padding
        }
        dib.extend_from_slice(&[0x80, 0, 0, 0, 0, 0, 0, 0]); // mask rows: bottom row hides x = 0
        let mut file = vec![0, 0, 1, 0, 1, 0, 2, 2, 0, 0, 1, 0, 24, 0];
        file.extend_from_slice(&(dib.len() as u32).to_le_bytes());
        file.extend_from_slice(&22u32.to_le_bytes());
        file.extend_from_slice(&dib);
        let d = decode(&file, None).unwrap();
        assert_eq!(d.frames[0][3], 255); // top row visible
        assert_eq!(d.frames[0][(2) * 4 + 3], 0); // bottom-left hidden
        assert_eq!(d.frames[0][(2 + 1) * 4 + 3], 255);
    }

    #[test]
    fn corrupt_files_are_errors_and_never_panic() {
        let f = frame(picture(16, 16, true), 16, 16, 100);
        let mut o = ImageOptions::default();
        o.set_ico_sizes(&[16, 32]);
        let good = encode_frame(&f, 1, &o).unwrap();
        assert!(decode(&good[..10], None).is_err());
        assert!(decode(&good[..30], None).is_err());
        assert!(decode(&[0, 0, 1, 0, 200, 0], None).is_err());
        for i in 0..good.len().min(600) {
            let mut bad = good.clone();
            bad[i] ^= 0xFF;
            let _ = decode(&bad, None);
            let _ = decode(&bad, Some(16));
        }
    }
}
