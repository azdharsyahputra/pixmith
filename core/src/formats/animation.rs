//! Animated image formats: GIF import and APNG export.

use crate::document::{Editor, Sprite};
use crate::ffi::{path_arg, set_error};
use crate::formats::export::upscale;
use crate::formats::gif::{report, write_file};
use crate::formats::import::{cel, image_palette};
use std::ffi::c_char;
use std::ptr::null_mut;

/// Decodes a GIF into a one-layer sprite with one frame per image. Frames are composed onto the
/// logical screen following each image's disposal method, so every sprite frame is complete.
pub fn sprite_from_gif(bytes: &[u8]) -> Result<Sprite, String> {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut dec = opts.read_info(bytes).map_err(|e| e.to_string())?;
    let (w, h) = (dec.width() as usize, dec.height() as usize);
    if w == 0 || h == 0 {
        return Err("the GIF has no pixels".into());
    }
    if (w * h) as u64 > crate::formats::export::MAX_EXPORT_PIXELS / 4 {
        return Err("the GIF is too large".into());
    }
    let mut canvas = vec![0u8; w * h * 4];
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut durations = Vec::new();
    while let Some(f) = dec.read_next_frame().map_err(|e| e.to_string())? {
        // Sprites hold at most 65535 frames, and all of them are kept in memory.
        if frames.len() >= u16::MAX as usize || (frames.len() + 1) as u64 * (w * h * 4) as u64 > 1 << 30 {
            return Err(format!(
                "the GIF has too many frames: {} frames of {w}x{h} need about {} MB (every frame is kept as a full \
                 RGBA image), the limit is 1024 MB and {} frames",
                frames.len() + 1,
                ((frames.len() + 1) * w * h * 4) >> 20,
                u16::MAX
            ));
        }
        if f.width == 0 || f.height == 0 {
            continue;
        }
        let before = (f.dispose == gif::DisposalMethod::Previous).then(|| canvas.clone());
        let (left, top) = (f.left as usize, f.top as usize);
        for (row, line) in f.buffer.chunks_exact(f.width as usize * 4).enumerate() {
            let y = top + row;
            if y >= h {
                break;
            }
            for (col, px) in line.chunks_exact(4).enumerate() {
                let x = left + col;
                if x < w && px[3] != 0 {
                    canvas[(y * w + x) * 4..][..4].copy_from_slice(px);
                }
            }
        }
        frames.push(canvas.clone());
        // A delay of 0 is conventionally shown as 100 ms.
        durations.push(if f.delay == 0 { 100 } else { (f.delay as u32 * 10).min(u16::MAX as u32) as u16 });
        match f.dispose {
            gif::DisposalMethod::Background => {
                for y in top..(top + f.height as usize).min(h) {
                    let end = (left + f.width as usize).min(w);
                    canvas[(y * w + left.min(w)) * 4..(y * w + end) * 4].fill(0);
                }
            }
            gif::DisposalMethod::Previous => canvas = before.unwrap_or(canvas),
            _ => {}
        }
    }
    if frames.is_empty() {
        return Err("the GIF has no frames".into());
    }
    let mut s = Sprite::new(w as u32, h as u32);
    let refs: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
    s.palette = image_palette(&refs).unwrap_or(s.palette);
    s.durations = durations;
    s.cels = frames.into_iter().map(|px| vec![cel(px)]).collect();
    Ok(s)
}

/// Animated PNG with one full-canvas frame per sprite frame, looping forever.
pub fn apng_bytes(s: &Sprite, scale: usize) -> Result<Vec<u8>, String> {
    let scale = scale.clamp(1, 64);
    let (w, h) = (s.width as usize * scale, s.height as usize * scale);
    crate::formats::export::check_export_size(w, h)?;
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_animated(s.frame_count() as u32, 0).map_err(|e| e.to_string())?;
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    for f in 0..s.frame_count() {
        writer.set_frame_delay(s.durations[f].max(1), 1000).map_err(|e| e.to_string())?;
        let rgba = upscale(&s.render(f), s.width as usize, s.height as usize, scale);
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    }
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

/// Animated PNG of a planned job: frames may be trimmed (all the same size) and the job's repeat
/// count is the number of plays (0 = forever).
pub fn apng_bytes_job(job: &crate::formats::export_plan::Job, scale: usize) -> Result<Vec<u8>, String> {
    let first = job.frames.first().ok_or("there are no frames to export")?;
    if job.frames.iter().any(|f| (f.w, f.h) != (first.w, first.h)) {
        return Err("the frames differ in size; use a uniform trim for animations".into());
    }
    let scale = scale.clamp(1, 64);
    let (w, h) = (first.w * scale, first.h * scale);
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_animated(job.frames.len() as u32, job.repeat as u32).map_err(|e| e.to_string())?;
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    for f in &job.frames {
        writer.set_frame_delay(f.duration.max(1), 1000).map_err(|e| e.to_string())?;
        writer.write_image_data(&upscale(&f.rgba, f.w, f.h, scale)).map_err(|e| e.to_string())?;
    }
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

/// Opens an animated GIF as a new sprite. Returns null on failure.
#[no_mangle]
pub unsafe extern "C" fn pixmith_import_gif(path: *const c_char) -> *mut Editor {
    crate::ffi::guarded("pixmith_import_gif", std::ptr::null_mut(), || {
        let result = path_arg(path).ok_or("invalid path".to_string()).and_then(|p| {
            let bytes = std::fs::read(&p).map_err(|e| format!("{p}: {e}"))?;
            sprite_from_gif(&bytes).map_err(|e| format!("{p}: {e}"))
        });
        match result {
            Ok(s) => Box::into_raw(Box::new(Editor::new(s))),
            Err(e) => {
                set_error(e);
                null_mut()
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_export_apng(doc: *const Editor, scale: u32, path: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_export_apng", std::ptr::null_mut(), || {
        let (Some(e), Some(path)) = (crate::ffi::synced(doc), path_arg(path)) else { return false };
        report(apng_bytes(&e.sprite, scale as usize).and_then(|d| write_file(&path, &d)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::gif::gif_bytes;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn animated() -> Sprite {
        let mut s = Sprite::new(3, 2);
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        s.add_frame(0, true);
        s.draw_line(0, 1, 2, 1, 2, 1, BLUE, 1);
        s.durations = vec![100, 250];
        s
    }

    #[test]
    fn gif_round_trips_frames_and_delays() {
        let s = animated();
        let back = sprite_from_gif(&gif_bytes(&s, 1).unwrap()).unwrap();
        assert_eq!((back.width, back.height, back.frame_count()), (3, 2, 2));
        assert_eq!(back.durations, vec![100, 250]);
        assert_eq!(back.render(0), s.render(0));
        assert_eq!(back.render(1), s.render(1));
        assert!(back.palette.contains(&RED) && back.palette.contains(&BLUE));
    }

    #[test]
    fn gif_disposal_and_subframes_are_composed() {
        // 2x1 screen. Frame 1 paints both pixels; frame 2 is a 1x1 patch at x=1 disposed to background;
        // frame 3 is transparent-only, so it shows what frame 2's disposal left.
        let mut out = Vec::new();
        {
            let mut enc = gif::Encoder::new(&mut out, 2, 1, &[]).unwrap();
            let pal = vec![255, 0, 0, 0, 0, 255];
            let full = gif::Frame {
                width: 2,
                height: 1,
                palette: Some(pal.clone()),
                buffer: vec![0, 0].into(),
                delay: 0,
                ..Default::default()
            };
            enc.write_frame(&full).unwrap();
            let patch = gif::Frame {
                left: 1,
                width: 1,
                height: 1,
                palette: Some(pal),
                buffer: vec![1].into(),
                delay: 5,
                dispose: gif::DisposalMethod::Background,
                ..Default::default()
            };
            enc.write_frame(&patch).unwrap();
            let empty = gif::Frame {
                width: 1,
                height: 1,
                palette: Some(vec![0, 0, 0]),
                buffer: vec![0].into(),
                transparent: Some(0),
                delay: 3,
                ..Default::default()
            };
            enc.write_frame(&empty).unwrap();
        }
        let s = sprite_from_gif(&out).unwrap();
        assert_eq!(s.durations, vec![100, 50, 30]);
        assert_eq!(s.render(0), [RED, RED].concat());
        assert_eq!(s.render(1), [RED, BLUE].concat());
        assert_eq!(s.render(2), [RED, [0; 4]].concat());
    }

    #[test]
    fn gif_import_rejects_garbage() {
        assert!(sprite_from_gif(b"GIF89a nope").is_err());
        assert!(sprite_from_gif(&[]).is_err());
    }

    #[test]
    fn apng_has_frames_delays_and_scale() {
        let data = apng_bytes(&animated(), 2).unwrap();
        let dec = png::Decoder::new(&data[..]);
        let mut reader = dec.read_info().unwrap();
        let ctl = reader.info().animation_control.unwrap();
        assert_eq!((ctl.num_frames, ctl.num_plays), (2, 0));
        let mut delays = Vec::new();
        let mut buf = vec![0; reader.output_buffer_size()];
        for _ in 0..2 {
            let info = reader.next_frame(&mut buf).unwrap();
            assert_eq!((info.width, info.height), (6, 4));
            let fc = reader.info().frame_control.unwrap();
            delays.push((fc.delay_num, fc.delay_den));
        }
        assert_eq!(delays, vec![(100, 1000), (250, 1000)]);
        assert_eq!(&buf[(2 * 6 + 4) * 4..][..4], &BLUE); // frame 2, original pixel (2,1)
    }

    #[test]
    fn single_frame_apng_is_valid() {
        let s = Sprite::new(2, 2);
        let data = apng_bytes(&s, 1).unwrap();
        assert!(png::Decoder::new(&data[..]).read_info().is_ok());
    }
}
