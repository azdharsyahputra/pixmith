//! PNG export of single frames and horizontal sprite sheets.

use crate::document::Sprite;

/// Largest image (in pixels) the exporters build in memory: 2^28 pixels, 1 GiB of RGBA.
pub(crate) const MAX_EXPORT_PIXELS: u64 = 1 << 28;

/// Fails if a `w` x `h` image is too large to build.
pub(crate) fn check_export_size(w: usize, h: usize) -> Result<(), String> {
    if (w as u64).saturating_mul(h as u64) > MAX_EXPORT_PIXELS {
        return Err("the image is too large to export; use a smaller scale".into());
    }
    Ok(())
}

pub(crate) fn upscale(rgba: &[u8], w: usize, h: usize, s: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len() * s * s);
    for y in 0..h * s {
        let row = &rgba[(y / s) * w * 4..(y / s + 1) * w * 4];
        for px in row.chunks_exact(4) {
            for _ in 0..s {
                out.extend_from_slice(px);
            }
        }
    }
    out
}

pub(crate) fn encode_png(rgba: &[u8], w: usize, h: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn frame_png(s: &Sprite, frame: usize, scale: usize) -> Result<Vec<u8>, String> {
    let (w, h, scale) = (s.width as usize, s.height as usize, scale.clamp(1, 64));
    check_export_size(w * scale, h * scale)?;
    encode_png(&upscale(&s.render(frame), w, h, scale), w * scale, h * scale)
}

pub fn sheet_png(s: &Sprite, scale: usize) -> Result<Vec<u8>, String> {
    let (w, h, scale) = (s.width as usize * scale.clamp(1, 64), s.height as usize, scale.clamp(1, 64));
    let frames = s.frame_count();
    check_export_size(w.saturating_mul(frames), h * scale)?;
    let row_len = w * 4;
    let mut sheet = vec![0u8; row_len * frames * h * scale];
    for f in 0..frames {
        let img = upscale(&s.render(f), s.width as usize, h, scale);
        for y in 0..h * scale {
            let d = (y * frames + f) * row_len;
            sheet[d..d + row_len].copy_from_slice(&img[y * row_len..(y + 1) * row_len]);
        }
    }
    encode_png(&sheet, w * frames, h * scale)
}
