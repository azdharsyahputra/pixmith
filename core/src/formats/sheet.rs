//! Sprite sheet export with Aseprite-style JSON metadata.

use crate::document::{Editor, Sprite, Tag};
use crate::ffi::path_arg;
use crate::formats::export::{encode_png, upscale};
use crate::formats::export_plan::Job;
use crate::formats::gif::{report, write_file};
use std::ffi::c_char;
use std::fmt::Write;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Layout {
    #[default]
    Horizontal,
    Vertical,
    /// Number of columns.
    Grid(usize),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    pub layout: Layout,
    /// Transparent pixels between frames (and around the sheet edge unless `border` is set), in output pixels.
    pub padding: usize,
    pub scale: usize,
    /// Transparent margin around the whole sheet; `None` uses `padding`.
    pub border: Option<usize>,
    /// Copies of each frame's edge pixels added around it (against texture bleeding), in output pixels.
    pub extrude: usize,
}

/// Where one frame sits in the sheet, in output pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetFrame {
    /// The frame image itself (without extrusion).
    pub rect: (usize, usize, usize, usize),
    /// `spriteSourceSize`: where the (possibly trimmed) image sits in the untrimmed frame.
    pub source: (usize, usize, usize, usize),
    pub trimmed: bool,
    pub duration: u16,
}

pub struct Sheet {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    /// Top-left corner of each frame in output pixels.
    pub origins: Vec<(usize, usize)>,
    /// Size of the largest frame in output pixels.
    pub cell: (usize, usize),
    pub scale: usize,
    pub frames: Vec<SheetFrame>,
    /// Untrimmed frame size in output pixels (`sourceSize`).
    pub source: (usize, usize),
}

pub fn build(s: &Sprite, opts: Options) -> Result<Sheet, String> {
    build_job(&Job::from_sprite(s), opts)
}

/// Packs the frames of a job, which may be trimmed to different sizes. Columns are as wide and rows
/// as tall as their largest frame.
pub fn build_job(job: &Job, opts: Options) -> Result<Sheet, String> {
    let scale = opts.scale.clamp(1, 64);
    let pad = opts.padding.min(1024);
    let border = opts.border.unwrap_or(pad).min(1024);
    let ex = opts.extrude.min(64);
    let n = job.frames.len();
    if n == 0 {
        return Err("there are no frames to export".into());
    }
    let cols = match opts.layout {
        Layout::Horizontal => n,
        Layout::Vertical => 1,
        Layout::Grid(c) => c.clamp(1, n),
    };
    let rows = n.div_ceil(cols);
    // Sizes first: nothing is upscaled until the sheet is known to be a sane size.
    let sizes: Vec<(usize, usize)> = job.frames.iter().map(|f| (f.w * scale, f.h * scale)).collect();
    let mut col_w = vec![0; cols];
    let mut row_h = vec![0; rows];
    for (i, (w, h)) in sizes.iter().enumerate() {
        col_w[i % cols] = col_w[i % cols].max(w + 2 * ex);
        row_h[i / cols] = row_h[i / cols].max(h + 2 * ex);
    }
    let starts = |sizes: &[usize]| -> Vec<usize> {
        let mut at = border;
        sizes
            .iter()
            .map(|&s| {
                let v = at;
                at += s + pad;
                v
            })
            .collect()
    };
    let (xs, ys) = (starts(&col_w), starts(&row_h));
    let width = 2 * border + col_w.iter().sum::<usize>() + (cols - 1) * pad;
    let height = 2 * border + row_h.iter().sum::<usize>() + (rows - 1) * pad;
    if width.saturating_mul(height) > 1 << 27 {
        return Err("sprite sheet is too large".into());
    }
    let mut rgba = vec![0u8; width * height * 4];
    let mut origins = Vec::with_capacity(n);
    let mut frames = Vec::with_capacity(n);
    for (i, ((fw, fh), f)) in sizes.iter().zip(&job.frames).enumerate() {
        let img = upscale(&f.rgba, f.w, f.h, scale);
        let (fw, fh) = (*fw, *fh);
        let (x0, y0) = (xs[i % cols] + ex, ys[i / cols] + ex);
        // Rows and columns of the cell that show the frame; the extruded ring samples the nearest edge pixel.
        for y in 0..fh + 2 * ex {
            let sy = y.saturating_sub(ex).min(fh - 1);
            for x in 0..fw + 2 * ex {
                let sx = x.saturating_sub(ex).min(fw - 1);
                let d = ((y0 - ex + y) * width + x0 - ex + x) * 4;
                rgba[d..d + 4].copy_from_slice(&img[(sy * fw + sx) * 4..][..4]);
            }
        }
        origins.push((x0, y0));
        frames.push(SheetFrame {
            rect: (x0, y0, fw, fh),
            source: (f.off.0 * scale, f.off.1 * scale, fw, fh),
            trimmed: f.trimmed,
            duration: f.duration,
        });
    }
    let cell = (sizes.iter().map(|i| i.0).max().unwrap_or(0), sizes.iter().map(|i| i.1).max().unwrap_or(0));
    Ok(Sheet {
        width,
        height,
        rgba,
        origins,
        cell,
        scale,
        frames,
        source: (job.canvas.0 * scale, job.canvas.1 * scale),
    })
}

pub fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => write!(out, "\\u{:04x}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Aseprite "array" style metadata. `title` names the frames (`<title> <index>.aseprite`).
pub fn json(s: &Sprite, sheet: &Sheet, image_name: &str, title: &str) -> String {
    json_job(sheet, &s.tags, &crate::document::slices::json_meta(s, sheet.scale), image_name, title)
}

/// Metadata for a built sheet. `tags` are relative to the sheet's frames; `extra_meta` is appended to
/// `meta` after `frameTags` (slices), or empty.
pub fn json_job(sheet: &Sheet, tags: &[Tag], extra_meta: &str, image_name: &str, title: &str) -> String {
    let mut out = String::from("{\n  \"frames\": [\n");
    let (sw, sh) = sheet.source;
    for (i, f) in sheet.frames.iter().enumerate() {
        let name = json_str(&format!("{title} {i}.aseprite"));
        let sep = if i + 1 < sheet.frames.len() { "," } else { "" };
        let ((x, y, w, h), (sx, sy, sww, shh)) = (f.rect, f.source);
        writeln!(
            out,
            "    {{\n      \"filename\": {name},\n      \"frame\": {{ \"x\": {x}, \"y\": {y}, \"w\": {w}, \"h\": {h} }},\n      \
             \"rotated\": false,\n      \"trimmed\": {},\n      \
             \"spriteSourceSize\": {{ \"x\": {sx}, \"y\": {sy}, \"w\": {sww}, \"h\": {shh} }},\n      \
             \"sourceSize\": {{ \"w\": {sw}, \"h\": {sh} }},\n      \"duration\": {}\n    }}{sep}",
            f.trimmed, f.duration
        )
        .unwrap();
    }
    out.push_str("  ],\n  \"meta\": {\n    \"app\": \"Pixmith\",\n");
    writeln!(out, "    \"version\": \"{}\",", env!("CARGO_PKG_VERSION")).unwrap();
    writeln!(out, "    \"image\": {},", json_str(image_name)).unwrap();
    writeln!(out, "    \"format\": \"RGBA8888\",").unwrap();
    writeln!(out, "    \"size\": {{ \"w\": {}, \"h\": {} }},", sheet.width, sheet.height).unwrap();
    writeln!(out, "    \"scale\": \"{}\",", sheet.scale).unwrap();
    out.push_str("    \"frameTags\": [");
    for (i, t) in tags.iter().enumerate() {
        let dir = ["forward", "reverse", "pingpong", "pingpong_reverse"][(t.direction as usize).min(3)];
        let [r, g, b] = t.color;
        out.push_str(if i == 0 { "\n" } else { ",\n" });
        write!(
            out,
            "      {{ \"name\": {}, \"from\": {}, \"to\": {}, \"direction\": \"{dir}\", \"color\": \"#{r:02x}{g:02x}{b:02x}ff\"",
            json_str(&t.name),
            t.from,
            t.to
        )
        .unwrap();
        if t.repeat > 0 {
            write!(out, ", \"repeat\": \"{}\"", t.repeat).unwrap();
        }
        out.push_str(" }");
    }
    out.push_str(if tags.is_empty() { "]" } else { "\n    ]" });
    out.push_str(extra_meta);
    out.push_str("\n  }\n}\n");
    out
}

/// PNG data and JSON text for a sheet. `image_name` is the PNG's file name, recorded in the JSON.
pub fn export(s: &Sprite, opts: Options, image_name: &str) -> Result<(Vec<u8>, String), String> {
    let sheet = build(s, opts)?;
    let png = encode_png(&sheet.rgba, sheet.width, sheet.height)?;
    let title = Path::new(image_name).file_stem().map_or(String::new(), |t| t.to_string_lossy().into_owned());
    Ok((png, json(s, &sheet, image_name, &title)))
}

/// `layout`: 0 horizontal, 1 vertical, 2 grid with `columns` columns. With `write_json`, the
/// metadata goes next to the PNG with a `.json` extension.
#[no_mangle]
pub unsafe extern "C" fn pixmith_export_sheet_ex(
    doc: *const Editor,
    layout: u32,
    columns: u32,
    padding: u32,
    scale: u32,
    write_json: bool,
    path: *const c_char,
) -> bool {
    crate::ffi::guarded("pixmith_export_sheet_ex", std::ptr::null_mut(), || {
        let (Some(e), Some(path)) = (crate::ffi::synced(doc), path_arg(path)) else { return false };
        let layout = match layout {
            0 => Layout::Horizontal,
            1 => Layout::Vertical,
            _ => Layout::Grid(columns as usize),
        };
        let opts = Options { layout, padding: padding as usize, scale: scale as usize, ..Default::default() };
        let image_name = Path::new(&path).file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        report(export(&e.sprite, opts, &image_name).and_then(|(png, json)| {
            write_file(&path, &png)?;
            if write_json {
                write_file(&Path::new(&path).with_extension("json").to_string_lossy(), json.as_bytes())?;
            }
            Ok(())
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Tag;

    const RED: [u8; 4] = [255, 0, 0, 255];

    fn three_frames() -> Sprite {
        let mut s = Sprite::new(2, 3);
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        s.add_frame(0, true);
        s.add_frame(1, true);
        s.durations = vec![100, 200, 300];
        s.tags.push(Tag {
            name: "run \"fast\"".into(),
            from: 1,
            to: 2,
            direction: 2,
            repeat: 0,
            color: [255, 0, 16],
            user_data: Default::default(),
        });
        s
    }

    fn opts(layout: Layout, padding: usize, scale: usize) -> Options {
        Options { layout, padding, scale, ..Default::default() }
    }

    #[test]
    fn layouts_place_frames() {
        let s = three_frames();
        let h = build(&s, opts(Layout::Horizontal, 0, 1)).unwrap();
        assert_eq!((h.width, h.height, h.origins.clone()), (6, 3, vec![(0, 0), (2, 0), (4, 0)]));
        let v = build(&s, opts(Layout::Vertical, 0, 1)).unwrap();
        assert_eq!((v.width, v.height, v.origins.clone()), (2, 9, vec![(0, 0), (0, 3), (0, 6)]));
        let g = build(&s, opts(Layout::Grid(2), 1, 2)).unwrap();
        assert_eq!((g.width, g.height), (1 + 2 * 5, 1 + 2 * 7));
        assert_eq!(g.origins, vec![(1, 1), (6, 1), (1, 8)]);
        assert_eq!(&g.rgba[(8 * g.width + 1) * 4..][..4], &RED);
        assert_eq!(&g.rgba[0..4], &[0; 4]);
    }

    #[test]
    fn grid_columns_are_clamped() {
        let s = three_frames();
        assert_eq!(build(&s, opts(Layout::Grid(0), 0, 1)).unwrap().width, 2);
        assert_eq!(build(&s, opts(Layout::Grid(99), 0, 1)).unwrap().width, 6);
    }

    #[test]
    fn json_describes_frames_and_tags() {
        let s = three_frames();
        let (png, json) = export(&s, opts(Layout::Grid(2), 1, 2), "walk.png").unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        assert_eq!(json.matches("\"filename\"").count(), 3);
        assert!(json.contains("\"filename\": \"walk 1.aseprite\""));
        assert!(json.contains("\"frame\": { \"x\": 6, \"y\": 1, \"w\": 4, \"h\": 6 }"));
        assert!(json.contains("\"duration\": 300"));
        assert!(json.contains("\"image\": \"walk.png\""));
        assert!(json.contains("\"size\": { \"w\": 11, \"h\": 15 }"));
        assert!(json.contains("\"scale\": \"2\""));
        assert!(json.contains(
            "{ \"name\": \"run \\\"fast\\\"\", \"from\": 1, \"to\": 2, \"direction\": \"pingpong\", \"color\": \"#ff0010ff\" }"
        ));
        assert_eq!(json.matches('{').count(), json.matches('}').count());
        assert_eq!(json.matches('[').count(), json.matches(']').count());
    }

    #[test]
    fn json_without_tags_has_empty_array() {
        let mut s = three_frames();
        s.tags.clear();
        let (_, json) = export(&s, opts(Layout::Horizontal, 0, 1), "a.png").unwrap();
        assert!(json.contains("\"frameTags\": []"));
    }

    fn job_of(frames: &[(usize, usize, (usize, usize), [u8; 4])], canvas: (usize, usize)) -> Job {
        Job {
            parts: vec![],
            canvas,
            frames: frames
                .iter()
                .enumerate()
                .map(|(i, &(w, h, off, c))| crate::formats::export_plan::PlanFrame {
                    rgba: c.repeat(w * h),
                    w,
                    h,
                    off,
                    trimmed: (w, h) != canvas,
                    duration: 100 + i as u16,
                    src_frame: i,
                })
                .collect(),
            tags: vec![],
            repeat: 0,
            trimmed: true,
        }
    }

    #[test]
    fn trimmed_frames_pack_by_column_and_row() {
        let blue = [0, 0, 255, 255];
        let job = job_of(&[(1, 2, (3, 1), RED), (2, 1, (0, 4), blue), (1, 1, (2, 2), RED)], (5, 6));
        let h = build_job(&job, opts(Layout::Horizontal, 1, 1)).unwrap();
        assert_eq!(h.origins, vec![(1, 1), (3, 1), (6, 1)]);
        assert_eq!((h.width, h.height), (1 + 1 + 1 + 2 + 1 + 1 + 1, 4));
        let g = build_job(&job, opts(Layout::Grid(2), 0, 1)).unwrap();
        assert_eq!(g.origins, vec![(0, 0), (1, 0), (0, 2)]);
        assert_eq!((g.width, g.height), (3, 3));
        // Pixels land at their origin and nowhere else.
        assert_eq!(&g.rgba[(0 * 3 + 1) * 4..][..4], &blue);
        assert_eq!(&g.rgba[(2 * 3 + 1) * 4..][..4], &[0; 4]);
        // The metadata locates each frame in the sheet and in the untrimmed frame.
        let json = json_job(&g, &[], "", "t.png", "t");
        assert!(json.contains("\"frame\": { \"x\": 1, \"y\": 0, \"w\": 2, \"h\": 1 }"));
        assert!(json.contains("\"spriteSourceSize\": { \"x\": 3, \"y\": 1, \"w\": 1, \"h\": 2 }"));
        assert!(json.contains("\"trimmed\": true"));
        assert!(json.contains("\"sourceSize\": { \"w\": 5, \"h\": 6 }"));
        assert!(json.contains("\"frameTags\": []"));
        assert_eq!(json.matches('{').count(), json.matches('}').count());
        // Scaling multiplies every coordinate.
        let g2 = build_job(&job, opts(Layout::Horizontal, 0, 2)).unwrap();
        let json = json_job(&g2, &[], "", "t.png", "t");
        assert!(json.contains("\"spriteSourceSize\": { \"x\": 6, \"y\": 2, \"w\": 2, \"h\": 4 }"));
        assert!(json.contains("\"sourceSize\": { \"w\": 10, \"h\": 12 }"));
    }

    #[test]
    fn extrude_and_border() {
        let blue = [0, 0, 255, 255];
        let job = job_of(&[(2, 1, (0, 0), RED), (1, 1, (0, 0), blue)], (2, 1));
        let o = Options { layout: Layout::Horizontal, padding: 1, scale: 1, border: Some(2), extrude: 1 };
        let sh = build_job(&job, o).unwrap();
        // border 2 + (2+2) + pad 1 + (1+2) + border 2, height 2 + (1+2) + 2.
        assert_eq!((sh.width, sh.height), (2 + 4 + 1 + 3 + 2, 7));
        assert_eq!(sh.origins, vec![(3, 3), (8, 3)]);
        // The frame itself is untouched and the ring repeats its edge.
        assert_eq!(&sh.rgba[(3 * 12 + 3) * 4..][..4], &RED);
        assert_eq!(&sh.rgba[(2 * 12 + 2) * 4..][..4], &RED); // corner copies the corner pixel
        assert_eq!(&sh.rgba[(3 * 12 + 9) * 4..][..4], &blue); // right of the second frame
        assert_eq!(&sh.rgba[(3 * 12 + 1) * 4..][..4], &[0; 4]); // border stays clear
        assert_eq!(&sh.rgba[(3 * 12 + 6) * 4..][..4], &[0; 4]); // padding between cells stays clear
                                                                // Metadata still describes the inner rectangle.
        assert_eq!(sh.frames[1].rect, (8, 3, 1, 1));
    }

    #[test]
    fn rejects_huge_sheets() {
        let s = Sprite::new(4000, 4000);
        assert!(build(&s, opts(Layout::Horizontal, 0, 64)).is_err());
    }
}
