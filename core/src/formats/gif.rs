//! Animated GIF export and numbered PNG sequences.

use crate::document::{Editor, Sprite};
use crate::ffi::{path_arg, set_error};
use crate::formats::export::{encode_png, upscale};
use gif::{DisposalMethod, Encoder, Frame, Repeat};
use std::collections::HashMap;
use std::ffi::c_char;

pub use crate::formats::durable::write_file;

pub(crate) fn report(result: Result<(), String>) -> bool {
    result.map_err(set_error).is_ok()
}

/// Pixels with alpha below 128 become transparent. Exact colors are kept when a frame has at
/// most 255 of them, otherwise the frame is quantized.
fn gif_frame(rgba: &mut [u8], w: usize, h: usize) -> Frame<'static> {
    for px in rgba.chunks_exact_mut(4) {
        px[3] = if px[3] < 128 { 0 } else { 255 };
    }
    let mut index: HashMap<[u8; 3], u8> = HashMap::new();
    // Index 0 is reserved for transparency.
    let mut palette = vec![0, 0, 0];
    let mut buffer = Vec::with_capacity(w * h);
    let mut has_alpha = false;
    for px in rgba.chunks_exact(4) {
        if px[3] == 0 {
            has_alpha = true;
            buffer.push(0);
            continue;
        }
        let rgb = [px[0], px[1], px[2]];
        let i = match index.get(&rgb) {
            Some(&i) => i,
            None if index.len() == 255 => {
                let mut f = Frame::from_rgba_speed(w as u16, h as u16, rgba, 10);
                f.dispose = if f.transparent.is_some() { DisposalMethod::Background } else { DisposalMethod::Keep };
                return f;
            }
            None => {
                let i = index.len() as u8 + 1;
                index.insert(rgb, i);
                palette.extend_from_slice(&rgb);
                i
            }
        };
        buffer.push(i);
    }
    let mut f = Frame { width: w as u16, height: h as u16, ..Frame::default() };
    f.palette = Some(palette);
    f.buffer = buffer.into();
    if has_alpha {
        f.transparent = Some(0);
        f.dispose = DisposalMethod::Background;
    }
    f
}

pub fn gif_bytes(s: &Sprite, scale: usize) -> Result<Vec<u8>, String> {
    let scale = scale.clamp(1, 64);
    let (w, h) = (s.width as usize * scale, s.height as usize * scale);
    if w > u16::MAX as usize || h > u16::MAX as usize {
        return Err("image is too large for a GIF".into());
    }
    crate::formats::export::check_export_size(w, h)?;
    let mut out = Vec::new();
    {
        let mut enc = Encoder::new(&mut out, w as u16, h as u16, &[]).map_err(|e| e.to_string())?;
        enc.set_repeat(Repeat::Infinite).map_err(|e| e.to_string())?;
        for f in 0..s.frame_count() {
            let mut rgba = upscale(&s.render(f), s.width as usize, s.height as usize, scale);
            let mut frame = gif_frame(&mut rgba, w, h);
            // GIF delays are in hundredths of a second.
            frame.delay = (s.durations[f] as u32).div_ceil(10).max(1).min(u16::MAX as u32) as u16;
            enc.write_frame(&frame).map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

/// `(file name, PNG data)` for each frame, numbered from 1 as `<base>-001.png`.
pub fn png_sequence(s: &Sprite, scale: usize, base: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let scale = scale.clamp(1, 64);
    let (w, h) = (s.width as usize, s.height as usize);
    crate::formats::export::check_export_size(w * scale, h * scale)?;
    let digits = s.frame_count().to_string().len().max(3);
    (0..s.frame_count())
        .map(|f| {
            let png = encode_png(&upscale(&s.render(f), w, h, scale), w * scale, h * scale)?;
            Ok((format!("{base}-{:0digits$}.png", f + 1), png))
        })
        .collect()
}

/// An animated GIF of a planned job (trimmed or reordered frames). `job.repeat` plays: 0 loops forever.
pub fn gif_bytes_job(job: &crate::formats::export_plan::Job, scale: usize) -> Result<Vec<u8>, String> {
    let first = job.frames.first().ok_or("there are no frames to export")?;
    let (fw, fh) = (first.w, first.h);
    if job.frames.iter().any(|f| (f.w, f.h) != (fw, fh)) {
        return Err("the frames differ in size; use a uniform trim for animations".into());
    }
    let scale = scale.clamp(1, 64);
    let (w, h) = (fw * scale, fh * scale);
    if w > u16::MAX as usize || h > u16::MAX as usize {
        return Err("image is too large for a GIF".into());
    }
    let mut out = Vec::new();
    {
        let mut enc = Encoder::new(&mut out, w as u16, h as u16, &[]).map_err(|e| e.to_string())?;
        match job.repeat {
            0 => enc.set_repeat(Repeat::Infinite).map_err(|e| e.to_string())?,
            1 => {} // no loop extension: the animation plays once
            n => enc.set_repeat(Repeat::Finite(n - 1)).map_err(|e| e.to_string())?,
        }
        for f in &job.frames {
            let mut rgba = upscale(&f.rgba, fw, fh, scale);
            let mut frame = gif_frame(&mut rgba, w, h);
            frame.delay = (f.duration as u32).div_ceil(10).max(1).min(u16::MAX as u32) as u16;
            enc.write_frame(&frame).map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

/// `(file name, PNG data)` for each frame of a job, `<base>-001.png`, ... Frames keep their (trimmed) size.
pub fn png_sequence_job(
    job: &crate::formats::export_plan::Job,
    scale: usize,
    base: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let scale = scale.clamp(1, 64);
    let digits = job.frames.len().to_string().len().max(3);
    job.frames
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let png = encode_png(&upscale(&f.rgba, f.w, f.h, scale), f.w * scale, f.h * scale)?;
            Ok((format!("{base}-{:0digits$}.png", i + 1), png))
        })
        .collect()
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_export_gif(doc: *const Editor, scale: u32, path: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_export_gif", std::ptr::null_mut(), || {
        let (Some(e), Some(path)) = (crate::ffi::synced(doc), path_arg(path)) else { return false };
        report(gif_bytes(&e.sprite, scale as usize).and_then(|d| write_file(&path, &d)))
    })
}

/// `base` is a path without extension; frames are written as `<base>-001.png`, `<base>-002.png`, ...
#[no_mangle]
pub unsafe extern "C" fn pixmith_export_png_sequence(doc: *const Editor, scale: u32, base: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_export_png_sequence", std::ptr::null_mut(), || {
        let (Some(e), Some(base)) = (crate::ffi::synced(doc), path_arg(base)) else { return false };
        report(
            png_sequence(&e.sprite, scale as usize, &base)
                .and_then(|files| files.iter().try_for_each(|(name, data)| write_file(name, data))),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn animated() -> Sprite {
        let mut s = Sprite::new(2, 2);
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        s.add_frame(0, true);
        s.draw_line(0, 1, 1, 1, 1, 1, BLUE, 1);
        s.durations = vec![100, 250];
        s
    }

    fn decode(data: &[u8]) -> (u16, u16, Vec<gif::Frame<'static>>) {
        let mut opts = gif::DecodeOptions::new();
        opts.set_color_output(gif::ColorOutput::RGBA);
        let mut dec = opts.read_info(data).unwrap();
        let mut frames = Vec::new();
        while let Some(f) = dec.read_next_frame().unwrap() {
            frames.push(f.clone());
        }
        (dec.width(), dec.height(), frames)
    }

    #[test]
    fn gif_has_frames_delays_and_transparency() {
        let (w, h, frames) = decode(&gif_bytes(&animated(), 1).unwrap());
        assert_eq!((w, h), (2, 2));
        assert_eq!(frames.len(), 2);
        assert_eq!(frames.iter().map(|f| f.delay).collect::<Vec<_>>(), vec![10, 25]);
        assert_eq!(&frames[0].buffer[0..4], &RED);
        assert_eq!(frames[0].buffer[4..8][3], 0);
        assert_eq!(&frames[1].buffer[12..16], &BLUE);
    }

    #[test]
    fn gif_scales() {
        let (w, h, frames) = decode(&gif_bytes(&animated(), 3).unwrap());
        assert_eq!((w, h), (6, 6));
        assert_eq!(frames.len(), 2);
        assert_eq!(&frames[0].buffer[(2 * 6 + 2) * 4..(2 * 6 + 2) * 4 + 4], &RED);
    }

    #[test]
    fn gif_with_many_colors_still_encodes() {
        let mut s = Sprite::new(32, 32);
        for i in 0..32u8 {
            for j in 0..32u8 {
                s.draw_line(0, 0, i as i32, j as i32, i as i32, j as i32, [i * 8, j * 8, i ^ j, 255], 1);
            }
        }
        let (_, _, frames) = decode(&gif_bytes(&s, 1).unwrap());
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn png_sequence_is_numbered() {
        let files = png_sequence(&animated(), 2, "out/walk").unwrap();
        let names: Vec<_> = files.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(names, ["out/walk-001.png", "out/walk-002.png"]);
        let (w, h, _) = crate::formats::import::decode_png(&files[1].1).unwrap();
        assert_eq!((w, h), (4, 4));
    }
}
