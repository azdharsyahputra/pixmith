//! JPEG: decoded by zune-jpeg, written by jpeg-encoder with 4:4:4 sampling so single-pixel color edges survive.
//! JPEG has no alpha: the export flattens the image onto `ImageOptions::background`.

use super::{check_dims, flatten, Decoded, ImageOptions};
use jpeg_encoder::{ColorType, Encoder, SamplingFactor};
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

pub fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGB)
        .set_max_width(u16::MAX as usize)
        .set_max_height(u16::MAX as usize);
    let mut dec = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    dec.decode_headers().map_err(|e| format!("not a readable JPEG: {e:?}"))?;
    let info = dec.info().ok_or("not a readable JPEG")?;
    let (w, h) = (info.width as usize, info.height as usize);
    check_dims(w, h)?;
    let mut rgb = vec![0u8; w * h * 3];
    let needed = dec.output_buffer_size().ok_or("not a readable JPEG")?;
    if needed != rgb.len() {
        return Err("the JPEG uses a color layout that is not supported".into());
    }
    dec.decode_into(&mut rgb).map_err(|e| format!("the JPEG is damaged: {e:?}"))?;
    let rgba = rgb.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
    Ok(Decoded::still(w, h, rgba))
}

/// Encodes `rgba` (any alpha is flattened onto the background color).
pub fn encode(rgba: &[u8], w: usize, h: usize, o: &ImageOptions) -> Result<Vec<u8>, String> {
    if w > u16::MAX as usize || h > u16::MAX as usize {
        return Err("the image is too large for a JPEG (65535 pixels at most per side)".into());
    }
    let mut px = rgba.to_vec();
    flatten(&mut px, o.background);
    let mut out = Vec::new();
    let mut enc = Encoder::new(&mut out, o.jpeg_quality.clamp(1, 100));
    enc.set_sampling_factor(SamplingFactor::R_4_4_4);
    enc.set_optimized_huffman_tables(true);
    enc.encode(&px, w as u16, h as u16, ColorType::Rgba).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::test_util::picture;
    use super::*;

    fn max_diff(a: &[u8], b: &[u8]) -> i32 {
        a.chunks_exact(4)
            .zip(b.chunks_exact(4))
            .flat_map(|(p, q)| (0..3).map(move |k| (p[k] as i32 - q[k] as i32).abs()))
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn round_trips_within_jpeg_error_and_quality_changes_size() {
        let src = picture(24, 16, false);
        let hi = encode(&src, 24, 16, &ImageOptions { jpeg_quality: 95, ..Default::default() }).unwrap();
        let lo = encode(&src, 24, 16, &ImageOptions { jpeg_quality: 10, ..Default::default() }).unwrap();
        assert!(lo.len() < hi.len());
        let d = decode(&hi).unwrap();
        assert_eq!((d.width, d.height, d.frames.len()), (24, 16, 1));
        assert!(max_diff(&d.frames[0], &src) <= 24, "diff {}", max_diff(&d.frames[0], &src));
        assert!(d.frames[0].chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn transparent_pixels_take_the_background_color() {
        let src = [0, 0, 0, 0].repeat(64);
        let o = ImageOptions { background: [255, 0, 0], jpeg_quality: 100, ..Default::default() };
        let d = decode(&encode(&src, 8, 8, &o).unwrap()).unwrap();
        for p in d.frames[0].chunks_exact(4) {
            assert!(p[0] > 240 && p[1] < 16 && p[2] < 16, "{p:?}");
        }
    }

    #[test]
    fn damaged_files_do_not_panic() {
        let good = encode(&picture(16, 16, false), 16, 16, &ImageOptions::default()).unwrap();
        let _ = decode(&good[..good.len() / 2]);
        assert!(decode(&[0xFF, 0xD8, 0xFF]).is_err());
        assert!(decode(&[]).is_err());
    }
}
