//! PNG import: whole images, sprite sheets cut into frames, and images added as layers.

use crate::document::{Cel, Editor, Sprite};
use crate::ffi::{path_arg, set_error};
use std::ffi::c_char;
use std::path::Path;
use std::ptr::null_mut;
use std::sync::Arc;

/// Largest PNG (in pixels) that is decoded: 8192 x 8192.
const MAX_IMPORT_PIXELS: u64 = 1 << 26;

/// Decodes any PNG to straight-alpha RGBA8. Returns `(width, height, pixels)`.
pub fn decode_png(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    let mut dec = png::Decoder::new(bytes);
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let (iw, ih) = reader.info().size();
    if iw > u16::MAX as u32
        || ih > u16::MAX as u32
        || iw as u64 * ih as u64 > MAX_IMPORT_PIXELS
        || iw as usize * ih as usize > crate::document::MAX_CANVAS_PIXELS
    {
        return Err("image is too large".into());
    }
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width as usize, info.height as usize);
    if w > u16::MAX as usize || h > u16::MAX as usize {
        return Err("image is too large".into());
    }
    let data = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("unsupported PNG color type".into()),
    };
    Ok((w, h, rgba))
}

pub(crate) fn cel(pixels: Vec<u8>) -> Option<Arc<Cel>> {
    Some(Arc::new(Cel { opacity: 255, pixels, ..Default::default() }))
}

/// The image's own opaque colors when there are few enough to make a palette.
pub(crate) fn image_palette(frames: &[&[u8]]) -> Option<Vec<[u8; 4]>> {
    let mut colors: Vec<[u8; 4]> = Vec::new();
    for px in frames.iter().flat_map(|f| f.chunks_exact(4)).filter(|px| px[3] == 255) {
        let c = [px[0], px[1], px[2], 255];
        if !colors.contains(&c) {
            if colors.len() == 256 {
                return None;
            }
            colors.push(c);
        }
    }
    (!colors.is_empty()).then_some(colors)
}

pub fn sprite_from_png(bytes: &[u8]) -> Result<Sprite, String> {
    let (w, h, rgba) = decode_png(bytes)?;
    let mut s = Sprite::new(w as u32, h as u32);
    s.palette = image_palette(&[&rgba]).unwrap_or(s.palette);
    s.cels[0][0] = cel(rgba);
    Ok(s)
}

/// Cuts a sheet into `cell_w` x `cell_h` frames, row by row. Partial cells at the edges are ignored.
pub fn sprite_from_sheet(bytes: &[u8], cell_w: usize, cell_h: usize) -> Result<Sprite, String> {
    let (w, h, rgba) = decode_png(bytes)?;
    if cell_w == 0 || cell_h == 0 || cell_w > w || cell_h > h {
        return Err(format!("cell size {cell_w}x{cell_h} does not fit in a {w}x{h} image"));
    }
    let (cols, rows) = (w / cell_w, h / cell_h);
    if cols * rows > u16::MAX as usize {
        return Err(format!("the sheet would have {} frames; at most {} are supported", cols * rows, u16::MAX));
    }
    let mut s = Sprite::new(cell_w as u32, cell_h as u32);
    s.durations.clear();
    s.cels.clear();
    for i in 0..cols * rows {
        let (x0, y0) = ((i % cols) * cell_w, (i / cols) * cell_h);
        let mut px = Vec::with_capacity(cell_w * cell_h * 4);
        for y in y0..y0 + cell_h {
            px.extend_from_slice(&rgba[(y * w + x0) * 4..(y * w + x0 + cell_w) * 4]);
        }
        s.durations.push(100);
        s.cels.push(vec![cel(px)]);
    }
    let all: Vec<&[u8]> = s.cels.iter().filter_map(|r| r[0].as_ref()).map(|c| c.pixels.as_slice()).collect();
    s.palette = image_palette(&all).unwrap_or(s.palette);
    Ok(s)
}

/// Adds the image as a new layer above `above`, placed at the top-left of `frame` and clipped to
/// the canvas. Returns the new layer's index.
pub fn add_png_layer(e: &mut Editor, above: usize, frame: usize, name: &str, bytes: &[u8]) -> Result<usize, String> {
    let (w, h, rgba) = decode_png(bytes)?;
    if frame >= e.sprite.frame_count() {
        return Err("no such frame".into());
    }
    e.checkpoint();
    let s = &mut e.sprite;
    let at = s.add_layer(above);
    s.layers[at].name = name.to_string();
    let (cw, ch) = (s.width as usize, s.height as usize);
    let mut px = vec![0u8; cw * ch * 4];
    for y in 0..h.min(ch) {
        let n = w.min(cw) * 4;
        px[y * cw * 4..y * cw * 4 + n].copy_from_slice(&rgba[y * w * 4..y * w * 4 + n]);
    }
    s.cels[frame][at] = cel(px);
    crate::document::color_mode::conform_cel(s, at, frame);
    Ok(at)
}

fn read(path: *const c_char) -> Result<(String, Vec<u8>), String> {
    let path = unsafe { path_arg(path) }.ok_or("invalid path")?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    Ok((path, bytes))
}

fn into_editor(result: Result<Sprite, String>) -> *mut Editor {
    match result {
        Ok(s) => Box::into_raw(Box::new(Editor::new(s))),
        Err(e) => {
            set_error(e);
            null_mut()
        }
    }
}

/// Opens a PNG as a new one-layer sprite. Returns null on failure.
#[no_mangle]
pub unsafe extern "C" fn pixmith_import_png(path: *const c_char) -> *mut Editor {
    crate::ffi::guarded("pixmith_import_png", std::ptr::null_mut(), || {
        into_editor(read(path).and_then(|(p, b)| sprite_from_png(&b).map_err(|e| format!("{p}: {e}"))))
    })
}

/// Opens a sprite sheet PNG as a new sprite with one frame per cell. Returns null on failure.
#[no_mangle]
pub unsafe extern "C" fn pixmith_import_sheet(path: *const c_char, cell_w: u32, cell_h: u32) -> *mut Editor {
    crate::ffi::guarded("pixmith_import_sheet", std::ptr::null_mut(), || {
        into_editor(read(path).and_then(|(p, b)| {
            sprite_from_sheet(&b, cell_w as usize, cell_h as usize).map_err(|e| format!("{p}: {e}"))
        }))
    })
}

/// Imports a PNG into a new layer above `above` on `frame` (undoable). Returns the layer index or -1.
#[no_mangle]
pub unsafe extern "C" fn pixmith_import_png_layer(
    doc: *mut Editor,
    above: u32,
    frame: u32,
    path: *const c_char,
) -> i32 {
    crate::ffi::guarded("pixmith_import_png_layer", doc, || {
        let Some(e) = doc.as_mut() else { return -1 };
        e.dirty.mark_all();
        let result = read(path).and_then(|(p, b)| {
            let name = Path::new(&p).file_stem().map_or("Layer".into(), |n| n.to_string_lossy().into_owned());
            add_png_layer(e, above as usize, frame as usize, &name, &b).map_err(|m| format!("{p}: {m}"))
        });
        match result {
            Ok(i) => i as i32,
            Err(m) => {
                set_error(m);
                -1
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::export;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 128];

    fn sample() -> Sprite {
        let mut s = Sprite::new(3, 2);
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        s.draw_line(0, 0, 2, 1, 2, 1, BLUE, 1);
        s
    }

    #[test]
    fn png_round_trips() {
        let s = sample();
        let png = export::frame_png(&s, 0, 1).unwrap();
        let back = sprite_from_png(&png).unwrap();
        assert_eq!((back.width, back.height, back.frame_count()), (3, 2, 1));
        assert_eq!(back.render(0), s.render(0));
        assert!(back.palette.contains(&RED));
    }

    #[test]
    fn decodes_grayscale_and_indexed() {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 2, 1);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&[0, 200]).unwrap();
        assert_eq!(decode_png(&out).unwrap().2, vec![0, 0, 0, 255, 200, 200, 200, 255]);

        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 2, 1);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_palette(vec![10, 20, 30, 40, 50, 60]);
        enc.set_trns(vec![0]);
        enc.write_header().unwrap().write_image_data(&[0, 1]).unwrap();
        assert_eq!(decode_png(&out).unwrap().2, vec![10, 20, 30, 0, 40, 50, 60, 255]);
    }

    #[test]
    fn sheet_import_splits_frames() {
        let mut s = Sprite::new(2, 2);
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        s.add_frame(0, false);
        s.draw_line(0, 1, 1, 1, 1, 1, RED, 1);
        s.add_frame(1, true);
        let sheet = export::sheet_png(&s, 1).unwrap();
        let back = sprite_from_sheet(&sheet, 2, 2).unwrap();
        assert_eq!((back.width, back.height, back.frame_count()), (2, 2, 3));
        for f in 0..3 {
            assert_eq!(back.render(f), s.render(f));
        }
        assert!(sprite_from_sheet(&sheet, 0, 2).is_err());
        assert!(sprite_from_sheet(&sheet, 7, 2).is_err());
    }

    #[test]
    fn sheet_import_uses_grid_order() {
        let mut s = Sprite::new(4, 4);
        s.draw_line(0, 0, 2, 0, 2, 0, RED, 1);
        s.draw_line(0, 0, 0, 2, 0, 2, BLUE, 1);
        let png = export::frame_png(&s, 0, 1).unwrap();
        let back = sprite_from_sheet(&png, 2, 2).unwrap();
        assert_eq!(back.frame_count(), 4);
        assert_eq!(&back.render(1)[0..4], &RED);
        assert_eq!(&back.render(2)[0..4], &BLUE);
    }

    #[test]
    fn png_layer_is_added_undoably_and_clipped() {
        let mut e = Editor::new(Sprite::new(2, 2));
        let big = export::frame_png(&sample(), 0, 1).unwrap();
        let at = add_png_layer(&mut e, 0, 0, "art", &big).unwrap();
        assert_eq!((at, e.sprite.layers.len(), e.sprite.layers[1].name.as_str()), (1, 2, "art"));
        assert_eq!(&e.sprite.render(0)[0..4], &RED);
        assert_eq!(e.sprite.cels[0][1].as_ref().unwrap().pixels.len(), 16);
        assert!(e.undo());
        assert_eq!(e.sprite.layers.len(), 1);
        assert!(add_png_layer(&mut e, 0, 5, "x", &big).is_err());
        assert!(add_png_layer(&mut e, 0, 0, "x", b"nope").is_err());
        assert!(!e.undo());
    }
}
