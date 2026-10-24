//! WebP, still and animated, lossy and lossless, through libwebp (the `webp-animation` crate builds it from its
//! bundled sources, so nothing has to be installed). Animations are decoded into full frames; frame durations
//! come from the difference of the frame timestamps.

use super::{check_dims, check_frame_budget, Decoded, ImageOptions};
use webp_animation::prelude::*;
use webp_animation::AnimParams;

pub fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    let dec = Decoder::new_with_options(bytes, DecoderOptions { use_threads: false, color_mode: ColorMode::Rgba })
        .map_err(|e| format!("not a readable WebP: {e}"))?;
    let (w, h) = dec.dimensions();
    let (w, h) = (w as usize, h as usize);
    check_dims(w, h)?;
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut durations = Vec::new();
    let mut previous = 0i64;
    for f in dec {
        check_frame_budget(frames.len(), w, h)?;
        if f.data().len() != w * h * 4 {
            return Err("the WebP frame has an unexpected size".into());
        }
        let end = f.timestamp() as i64;
        // libwebp reports the end of each frame; a frame without a duration is shown for 100 ms like browsers do.
        let ms = end - previous;
        durations.push(if ms <= 0 { 100 } else { ms.min(u16::MAX as i64) as u16 });
        previous = end;
        frames.push(f.data().to_vec());
    }
    if frames.is_empty() {
        return Err("the WebP has no frames".into());
    }
    Ok(Decoded { width: w, height: h, frames, durations, palette: None })
}

/// Encodes frames `(rgba, duration in ms)` as a WebP: a still image for one frame, otherwise an animation that
/// plays `repeat` times (0 = forever).
pub fn encode(frames: &[(&[u8], u16)], w: usize, h: usize, o: &ImageOptions, repeat: u16) -> Result<Vec<u8>, String> {
    if w > 16383 || h > 16383 {
        return Err("the image is too large for a WebP (16383 pixels at most per side)".into());
    }
    let quality = o.webp_quality.min(100) as f32;
    let config = if o.webp_lossless {
        EncodingConfig { encoding_type: EncodingType::Lossless, quality, method: 4 }
    } else {
        EncodingConfig::new_lossy(quality)
    };
    let options = EncoderOptions {
        anim_params: AnimParams { loop_count: repeat as i32 },
        encoding_config: Some(config),
        ..Default::default()
    };
    let mut enc = Encoder::new_with_options((w as u32, h as u32), options).map_err(|e| format!("WebP: {e}"))?;
    let mut t = 0i32;
    for (rgba, ms) in frames {
        enc.add_frame(rgba, t).map_err(|e| format!("WebP: {e}"))?;
        t = t.saturating_add((*ms).max(1) as i32);
    }
    let data = enc.finalize(t).map_err(|e| format!("WebP: {e}"))?;
    Ok(data.to_vec())
}

#[cfg(test)]
mod tests {
    use super::super::test_util::picture;
    use super::*;

    fn same_visible(a: &[u8], b: &[u8]) -> bool {
        a.chunks_exact(4).zip(b.chunks_exact(4)).all(|(p, q)| p[3] == q[3] && (p[3] == 0 || p == q))
    }

    #[test]
    fn lossless_still_is_exact_including_alpha() {
        let src = picture(13, 9, true);
        let bytes = encode(&[(&src, 100)], 13, 9, &ImageOptions::default(), 0).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WEBP");
        let d = decode(&bytes).unwrap();
        assert_eq!((d.width, d.height, d.frames.len()), (13, 9, 1));
        assert!(same_visible(&d.frames[0], &src));
    }

    #[test]
    fn lossy_is_smaller_and_close() {
        // Photo-like content: a smooth gradient with a little noise, where lossy coding pays off.
        let src: Vec<u8> = (0..64 * 64u32)
            .flat_map(|i| {
                let (x, y) = (i % 64, i / 64);
                let n = (i.wrapping_mul(2654435761) >> 28) as i32;
                let v = |base: u32| (base as i32 + n).clamp(0, 255) as u8;
                [v(x * 4), v(y * 4), v((x + y) * 2), 255]
            })
            .collect();
        let lossless = encode(&[(&src, 100)], 64, 64, &ImageOptions::default(), 0).unwrap();
        let o = ImageOptions { webp_lossless: false, webp_quality: 60, ..Default::default() };
        let lossy = encode(&[(&src, 100)], 64, 64, &o, 0).unwrap();
        assert!(lossy.len() < lossless.len(), "lossy {} lossless {}", lossy.len(), lossless.len());
        let d = decode(&lossy).unwrap();
        let total: i64 = d.frames[0].iter().zip(&src).map(|(a, b)| (*a as i64 - *b as i64).abs()).sum();
        let mean = total / src.len() as i64;
        assert!(mean < 12, "mean channel error {mean}");
    }

    #[test]
    fn animation_keeps_frames_and_durations() {
        let a = picture(8, 8, false);
        let mut b = a.clone();
        b[0] = 255 - b[0];
        let c: Vec<u8> = a.chunks_exact(4).flat_map(|p| [255 - p[0], 255 - p[1], 255 - p[2], p[3]]).collect();
        let frames = [(&a[..], 70u16), (&b[..], 120), (&c[..], 250)];
        let bytes = encode(&frames, 8, 8, &ImageOptions::default(), 0).unwrap();
        let d = decode(&bytes).unwrap();
        assert_eq!(d.frames.len(), 3);
        assert_eq!(d.durations, vec![70, 120, 250]);
        assert_eq!(d.frames[0], a);
        assert_eq!(d.frames[1], b);
        assert_eq!(d.frames[2][4..8], c[4..8]);
    }

    #[test]
    fn damaged_files_are_errors_not_panics() {
        let src = picture(16, 16, true);
        let good = encode(&[(&src, 100)], 16, 16, &ImageOptions::default(), 0).unwrap();
        for cut in [0, 4, 12, 20, good.len() / 2] {
            let _ = decode(&good[..cut]);
        }
        let mut bad = good.clone();
        for i in (20..bad.len()).step_by(3) {
            bad[i] ^= 0x5A;
        }
        let _ = decode(&bad);
        assert!(decode(b"RIFF\0\0\0\0WEBPjunk").is_err());
    }
}
