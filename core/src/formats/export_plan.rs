//! Export planning: which frames (tag range, direction), which layers, trimming and file splitting,
//! shared by the app (`pixmith_export_run`) and `pixmith-cli`.
//!
//! A plan is a list of [`Job`]s, one per output file (or sprite sheet). Each job holds rendered,
//! possibly trimmed frames; the encoders in `gif_export`, `animation` and `sheet` work on those.

use crate::document::{Editor, Sprite, Tag};
use crate::ffi::path_arg;
use crate::formats::export::{encode_png, upscale};
use crate::formats::gif::{report, write_file};
use crate::formats::sheet::{self, Layout};
use std::collections::HashMap;
use std::ffi::c_char;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Trim {
    #[default]
    None,
    /// Every frame is cropped to its own content (sheets and sequences; animations use `Uniform`).
    PerFrame,
    /// All frames of a job share the union of their content bounds.
    Uniform,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LayerMode {
    /// Layers as they are shown in the editor.
    #[default]
    Visible,
    /// Only this layer (shown even if hidden).
    Selected(usize),
    /// One output per visible layer.
    Each,
}

#[derive(Clone, Debug)]
pub struct ExportOptions {
    /// Index into `Sprite::tags`; `None` exports every frame.
    pub tag: Option<usize>,
    /// Expand the tag's direction (reverse, ping-pong) into the frame order.
    pub bake_direction: bool,
    pub layers: LayerMode,
    /// One output per tag (overrides `tag`).
    pub split_tags: bool,
    pub trim: Trim,
    /// Drop frames that have no visible pixels.
    pub ignore_empty: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            tag: None,
            bake_direction: true,
            layers: LayerMode::Visible,
            split_tags: false,
            trim: Trim::None,
            ignore_empty: false,
        }
    }
}

/// Source frames played by a tag direction: 0 forward, 1 reverse, 2 ping-pong, 3 ping-pong reverse.
/// Ping-pong turns around without repeating the end frames, so the sequence loops cleanly.
pub fn direction_order(from: usize, to: usize, direction: u8) -> Vec<usize> {
    let forward: Vec<usize> = (from..=to).collect();
    let backward: Vec<usize> = (from..=to).rev().collect();
    let inner = |v: &[usize]| v.get(1..v.len().saturating_sub(1)).map(<[usize]>::to_vec).unwrap_or_default();
    match direction {
        1 => backward,
        2 => [forward.clone(), inner(&backward)].concat(),
        3 => [backward, inner(&forward)].concat(),
        _ => forward,
    }
}

/// One rendered frame of a job. `rgba` is `w * h` straight-alpha pixels cropped from the canvas at `off`.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanFrame {
    pub rgba: Vec<u8>,
    pub w: usize,
    pub h: usize,
    pub off: (usize, usize),
    pub trimmed: bool,
    pub duration: u16,
    pub src_frame: usize,
}

/// One output: a sheet, a GIF, a PNG sequence, ...
#[derive(Clone, Debug)]
pub struct Job {
    /// Name parts appended to the output file name (layer name, tag name).
    pub parts: Vec<String>,
    /// Full canvas size before trimming.
    pub canvas: (usize, usize),
    pub frames: Vec<PlanFrame>,
    /// Tags for the sheet metadata, with frame numbers relative to `frames`.
    pub tags: Vec<Tag>,
    /// Plays of the animation (0 = forever), from the exported tag.
    pub repeat: u16,
    /// Whether any frame was cropped.
    pub trimmed: bool,
}

impl Job {
    /// Every frame of the sprite, as rendered, with its tags.
    pub fn from_sprite(s: &Sprite) -> Job {
        let (w, h) = (s.width as usize, s.height as usize);
        let frames = (0..s.frame_count())
            .map(|f| PlanFrame {
                rgba: s.render(f),
                w,
                h,
                off: (0, 0),
                trimmed: false,
                duration: s.durations[f],
                src_frame: f,
            })
            .collect();
        Job { parts: Vec::new(), canvas: (w, h), frames, tags: s.tags.clone(), repeat: 0, trimmed: false }
    }
}

/// What a job will contain, before any pixels are rendered.
#[derive(Clone, Debug)]
pub struct JobSpec {
    pub parts: Vec<String>,
    /// `None`: layers as shown. `Some(i)`: only layer `i`.
    pub layer: Option<usize>,
    /// Source frames in output order.
    pub frames: Vec<usize>,
    pub tag: Option<Tag>,
    pub repeat: u16,
    /// Frame numbers of the sprite's own tags apply (all frames, in order).
    pub keep_tags: bool,
}

fn is_hidden_by_parent(s: &Sprite, i: usize) -> bool {
    let mut level = s.layers[i].child_level;
    for j in (0..i).rev() {
        if level == 0 {
            break;
        }
        if s.layers[j].child_level < level {
            level = s.layers[j].child_level;
            if !s.layers[j].visible {
                return true;
            }
        }
    }
    false
}

/// Which layers produce their own output, for `LayerMode::Each`: visible image layers.
fn each_layers(s: &Sprite) -> Vec<usize> {
    (0..s.layers.len())
        .filter(|&i| {
            !s.layers[i].is_group() && !s.layers[i].is_reference() && s.layers[i].visible && !is_hidden_by_parent(s, i)
        })
        .collect()
}

pub fn specs(s: &Sprite, o: &ExportOptions) -> Result<Vec<JobSpec>, String> {
    let n = s.frame_count();
    // Layer variants: (layer to isolate, name parts).
    let layer_variants: Vec<(Option<usize>, Vec<String>)> = match o.layers {
        LayerMode::Visible => vec![(None, vec![])],
        LayerMode::Selected(i) => {
            if i >= s.layers.len() || s.layers[i].is_group() || s.layers[i].is_reference() {
                return Err("the selected layer cannot be exported".into());
            }
            vec![(Some(i), vec![])]
        }
        LayerMode::Each => {
            let each = each_layers(s);
            if each.is_empty() {
                return Err("there is no visible layer to export".into());
            }
            each.into_iter().map(|i| (Some(i), vec![s.layers[i].name.clone()])).collect()
        }
    };
    // Tag variants: (tag, name parts).
    let tag_range = |t: &Tag| -> Result<(usize, usize), String> {
        let (from, to) = (t.from as usize, (t.to as usize).min(n - 1));
        if from > to {
            return Err(format!("tag \"{}\" is outside the sprite's frames", t.name));
        }
        Ok((from, to))
    };
    let tag_variants: Vec<(Option<&Tag>, Vec<String>)> = if o.split_tags && !s.tags.is_empty() {
        s.tags.iter().map(|t| (Some(t), vec![t.name.clone()])).collect()
    } else if let (false, Some(i)) = (o.split_tags, o.tag) {
        let t = s.tags.get(i).ok_or("the tag does not exist")?;
        vec![(Some(t), vec![])]
    } else {
        vec![(None, vec![])]
    };
    let mut out = Vec::new();
    for (layer, lparts) in &layer_variants {
        for (tag, tparts) in &tag_variants {
            let (frames, tag, repeat, keep_tags) = match tag {
                None => ((0..n).collect(), None, 0, true),
                Some(t) => {
                    let (from, to) = tag_range(t)?;
                    let order =
                        if o.bake_direction { direction_order(from, to, t.direction) } else { (from..=to).collect() };
                    (order, Some((*t).clone()), t.repeat, false)
                }
            };
            out.push(JobSpec {
                parts: [lparts.clone(), tparts.clone()].concat(),
                layer: *layer,
                frames,
                tag,
                repeat,
                keep_tags,
            });
        }
    }
    Ok(out)
}

/// The sprite with only layer `i` (and the groups containing it) shown.
fn isolate(s: &Sprite, i: usize) -> Sprite {
    let mut v = s.clone();
    let mut level = v.layers[i].child_level;
    let mut keep = vec![false; v.layers.len()];
    keep[i] = true;
    for j in (0..i).rev() {
        if level == 0 {
            break;
        }
        if v.layers[j].child_level < level {
            level = v.layers[j].child_level;
            keep[j] = true;
        }
    }
    for (l, k) in v.layers.iter_mut().zip(keep) {
        l.visible = k;
    }
    v
}

/// Bounds `(x0, y0, x1, y1)` (exclusive end) of the pixels that are not fully transparent.
pub fn content_bounds(rgba: &[u8], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if rgba[(y * w + x) * 4 + 3] != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > x0).then_some((x0, y0, x1, y1))
}

fn crop(rgba: &[u8], w: usize, r: (usize, usize, usize, usize)) -> Vec<u8> {
    let (x0, y0, x1, y1) = r;
    let mut out = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
    for y in y0..y1 {
        out.extend_from_slice(&rgba[(y * w + x0) * 4..(y * w + x1) * 4]);
    }
    out
}

pub fn render(s: &Sprite, spec: &JobSpec, o: &ExportOptions) -> Job {
    let (w, h) = (s.width as usize, s.height as usize);
    let base = spec.layer.map(|i| isolate(s, i));
    let src = base.as_ref().unwrap_or(s);
    let mut cache: HashMap<usize, Vec<u8>> = HashMap::new();
    let mut frames: Vec<PlanFrame> = Vec::new();
    for &f in &spec.frames {
        let rgba = cache.entry(f).or_insert_with(|| src.render(f)).clone();
        if o.ignore_empty && content_bounds(&rgba, w, h).is_none() {
            continue;
        }
        frames.push(PlanFrame { rgba, w, h, off: (0, 0), trimmed: false, duration: s.durations[f], src_frame: f });
    }
    let full = (0, 0, w, h);
    let rects: Vec<Option<(usize, usize, usize, usize)>> =
        frames.iter().map(|f| content_bounds(&f.rgba, w, h)).collect();
    let uniform = rects.iter().flatten().fold(None, |acc: Option<(usize, usize, usize, usize)>, r| {
        Some(acc.map_or(*r, |a| (a.0.min(r.0), a.1.min(r.1), a.2.max(r.2), a.3.max(r.3))))
    });
    let mut any = false;
    if o.trim != Trim::None {
        for (f, own) in frames.iter_mut().zip(&rects) {
            // Empty frames shrink to one transparent pixel, or to the shared rectangle.
            let r = match o.trim {
                Trim::PerFrame => own.unwrap_or((0, 0, 1, 1)),
                _ => uniform.unwrap_or((0, 0, 1, 1)),
            };
            if r != full {
                f.rgba = crop(&f.rgba, w, r);
                (f.w, f.h, f.off, f.trimmed) = (r.2 - r.0, r.3 - r.1, (r.0, r.1), true);
                any = true;
            }
        }
    }
    // Tags: the exported tag alone, or the sprite's tags remapped onto the surviving frames.
    let tags = match &spec.tag {
        Some(t) if !frames.is_empty() => {
            let baked = o.bake_direction;
            vec![Tag {
                from: 0,
                to: frames.len() as u16 - 1,
                direction: if baked { 0 } else { t.direction },
                repeat: if baked { 0 } else { t.repeat },
                ..t.clone()
            }]
        }
        Some(_) => vec![],
        None if spec.keep_tags => s
            .tags
            .iter()
            .filter_map(|t| {
                let hit = |f: &PlanFrame| (t.from as usize..=t.to as usize).contains(&f.src_frame);
                let from = frames.iter().position(hit)?;
                let to = frames.iter().rposition(hit)?;
                Some(Tag { from: from as u16, to: to as u16, ..t.clone() })
            })
            .collect(),
        None => vec![],
    };
    Job { parts: spec.parts.clone(), canvas: (w, h), frames, tags, repeat: spec.repeat, trimmed: any }
}

pub fn plan(s: &Sprite, o: &ExportOptions) -> Result<Vec<Job>, String> {
    Ok(specs(s, o)?.iter().map(|sp| render(s, sp, o)).collect())
}

// ---- Encoding -------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// One frame per job as PNG.
    Png,
    Gif,
    Apng,
    Sheet,
    /// Numbered PNG files; the path is a base name without extension.
    Sequence,
    // The formats below are implemented in `formats::raster`; their options are `EncodeOptions::image`.
    Webp,
    Jpeg,
    Bmp,
    Tga,
    Ico,
    Pcx,
    Flc,
}

impl Format {
    pub fn from_u32(v: u32) -> Option<Format> {
        [
            Format::Png,
            Format::Gif,
            Format::Apng,
            Format::Sheet,
            Format::Sequence,
            Format::Webp,
            Format::Jpeg,
            Format::Bmp,
            Format::Tga,
            Format::Ico,
            Format::Pcx,
            Format::Flc,
        ]
        .get(v as usize)
        .copied()
    }

    /// The raster format behind the formats of `formats::raster`.
    pub fn image(self) -> Option<crate::formats::raster::ImageFormat> {
        use crate::formats::raster::ImageFormat as F;
        match self {
            Format::Webp => Some(F::Webp),
            Format::Jpeg => Some(F::Jpeg),
            Format::Bmp => Some(F::Bmp),
            Format::Tga => Some(F::Tga),
            Format::Ico => Some(F::Ico),
            Format::Pcx => Some(F::Pcx),
            Format::Flc => Some(F::Flc),
            _ => None,
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Format::Gif => "gif",
            _ => self.image().map_or("png", |f| f.extension()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EncodeOptions {
    pub scale: usize,
    /// Frame index for `Format::Png`, counted within each job.
    pub frame: usize,
    pub sheet: sheet::Options,
    pub write_json: bool,
    /// Settings of the formats in `formats::raster` (JPEG quality, WebP lossless, ICO sizes, ...).
    pub image: crate::formats::raster::ImageOptions,
}

fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let s = s.trim().trim_start_matches('.').to_string();
    if s.is_empty() {
        "_".into()
    } else {
        s
    }
}

/// File names for the jobs: `path` itself for a single job without name parts, otherwise
/// `<stem>-<part>-<part>.<ext>`. Identical names get a counter.
pub fn job_paths(path: &str, jobs: &[Job], fmt: Format) -> Vec<String> {
    let p = Path::new(path);
    let (stem, ext) = if fmt == Format::Sequence {
        (path.to_string(), String::new())
    } else {
        let ext = p.extension().map_or(fmt.extension().to_string(), |e| e.to_string_lossy().into_owned());
        (p.with_extension("").to_string_lossy().into_owned(), ext)
    };
    let mut seen: HashMap<String, usize> = HashMap::new();
    jobs.iter()
        .map(|j| {
            let mut name = stem.clone();
            for part in &j.parts {
                name.push('-');
                name.push_str(&sanitize(part));
            }
            let count = seen.entry(name.clone()).and_modify(|c| *c += 1).or_insert(1);
            if *count > 1 {
                name = format!("{name}-{count}");
            }
            if ext.is_empty() {
                name
            } else {
                format!("{name}.{ext}")
            }
        })
        .collect()
}

/// All files an export writes, as `(path, data)`.
pub fn render_outputs(
    s: &Sprite,
    fmt: Format,
    o: &ExportOptions,
    e: &EncodeOptions,
    path: &str,
    json_path: Option<&str>,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut o = o.clone();
    // Animated formats need equally sized frames.
    let animated = fmt.image().is_some_and(|f| crate::formats::raster::writes_animation(f, &e.image));
    if (matches!(fmt, Format::Gif | Format::Apng) || animated) && o.trim == Trim::PerFrame {
        o.trim = Trim::Uniform;
    }
    let jobs = plan(s, &o)?;
    let paths = job_paths(path, &jobs, fmt);
    let json_paths = json_path.map(|p| job_paths(p, &jobs, Format::Sheet));
    let mut files = Vec::new();
    for (k, (job, p)) in jobs.iter().zip(paths).enumerate() {
        if job.frames.is_empty() {
            return Err("there are no frames to export".into());
        }
        match fmt {
            Format::Png => {
                let f = job.frames.get(e.frame).ok_or_else(|| {
                    format!("frame {} does not exist (the export has {})", e.frame + 1, job.frames.len())
                })?;
                files.push((p, frame_png(f, e.scale)?));
            }
            Format::Webp | Format::Jpeg | Format::Bmp | Format::Tga | Format::Ico | Format::Pcx | Format::Flc => {
                let fmt = fmt.image().expect("raster format");
                if animated {
                    files.push((p, crate::formats::raster::encode_animation(fmt, job, e.scale, &e.image)?));
                } else {
                    let f = job.frames.get(e.frame).ok_or_else(|| {
                        format!("frame {} does not exist (the export has {})", e.frame + 1, job.frames.len())
                    })?;
                    files.push((p, crate::formats::raster::encode_still(fmt, f, e.scale, &e.image)?));
                }
            }
            Format::Gif => files.push((p, crate::formats::gif::gif_bytes_job(job, e.scale)?)),
            Format::Apng => files.push((p, crate::formats::animation::apng_bytes_job(job, e.scale)?)),
            Format::Sequence => files.extend(crate::formats::gif::png_sequence_job(job, e.scale, &p)?),
            Format::Sheet => {
                let mut opts = e.sheet;
                opts.scale = e.scale;
                let name = Path::new(&p).file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                let title = Path::new(&p).file_stem().map_or(String::new(), |t| t.to_string_lossy().into_owned());
                let built = sheet::build_job(job, opts)?;
                let png = encode_png(&built.rgba, built.width, built.height)?;
                if e.write_json {
                    let meta =
                        if job.trimmed { String::new() } else { crate::document::slices::json_meta(s, built.scale) };
                    let json = sheet::json_job(&built, &job.tags, &meta, &name, &title);
                    let jp = json_paths.as_ref().map_or_else(
                        || Path::new(&p).with_extension("json").to_string_lossy().into_owned(),
                        |v| v[k].clone(),
                    );
                    files.push((jp, json.into_bytes()));
                }
                files.push((p, png));
            }
        }
    }
    Ok(files)
}

fn frame_png(f: &PlanFrame, scale: usize) -> Result<Vec<u8>, String> {
    let scale = scale.clamp(1, 64);
    encode_png(&upscale(&f.rgba, f.w, f.h, scale), f.w * scale, f.h * scale)
}

// ---- C ABI ----------------------------------------------------------------------------------

/// Export settings for `pixmith_export_run`. Enumerations are explained in `pixmith_io.h`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PixmithExport {
    pub format: u32,
    pub scale: u32,
    /// Tag index, or -1 for all frames.
    pub tag: i32,
    pub apply_direction: bool,
    /// 0 visible layers, 1 only `layer`, 2 each layer separately.
    pub layers: u32,
    pub layer: u32,
    pub split_tags: bool,
    /// 0 none, 1 per frame, 2 whole animation.
    pub trim: u32,
    pub ignore_empty: bool,
    /// Frame for the PNG format, counted from 0 within the export.
    pub frame: u32,
    /// Sheet layout: 0 horizontal, 1 vertical, 2 grid.
    pub layout: u32,
    pub columns: u32,
    pub padding: u32,
    pub border: u32,
    pub extrude: u32,
    pub write_json: bool,
}

pub(crate) fn from_c(c: &PixmithExport) -> Option<(Format, ExportOptions, EncodeOptions)> {
    let layout = match c.layout {
        0 => Layout::Horizontal,
        1 => Layout::Vertical,
        _ => Layout::Grid(c.columns as usize),
    };
    let o = ExportOptions {
        tag: usize::try_from(c.tag).ok(),
        bake_direction: c.apply_direction,
        layers: match c.layers {
            1 => LayerMode::Selected(c.layer as usize),
            2 => LayerMode::Each,
            _ => LayerMode::Visible,
        },
        split_tags: c.split_tags,
        trim: match c.trim {
            1 => Trim::PerFrame,
            2 => Trim::Uniform,
            _ => Trim::None,
        },
        ignore_empty: c.ignore_empty,
    };
    let e = EncodeOptions {
        scale: c.scale as usize,
        frame: c.frame as usize,
        sheet: sheet::Options {
            layout,
            padding: c.padding as usize,
            scale: c.scale as usize,
            border: Some(c.border as usize),
            extrude: c.extrude as usize,
        },
        write_json: c.write_json,
        image: Default::default(),
    };
    Some((Format::from_u32(c.format)?, o, e))
}

/// Writes an export; returns the number of files written, or -1 (see `pixmith_last_error`).
/// `path` is the output file, or the base name (no extension) for the PNG sequence format.
#[no_mangle]
pub unsafe extern "C" fn pixmith_export_run(
    doc: *const Editor,
    opts: *const PixmithExport,
    path: *const c_char,
) -> i32 {
    crate::ffi::guarded("pixmith_export_run", std::ptr::null_mut(), || {
        let (Some(ed), Some(c), Some(path)) = (crate::ffi::synced(doc), opts.as_ref(), path_arg(path)) else {
            return -1;
        };
        let Some((fmt, o, e)) = from_c(c) else {
            crate::ffi::set_error("unknown export format".to_string());
            return -1;
        };
        let mut count = 0;
        let ok = report(render_outputs(&ed.sprite, fmt, &o, &e, &path, None).and_then(|files| {
            for (p, data) in &files {
                write_file(p, data)?;
                count += 1;
            }
            Ok(())
        }));
        if ok {
            count
        } else {
            -1
        }
    })
}

/// How many outputs (files for one frame / animation formats, sheets for sheets, frames for sequences
/// counted as one) an export with these options would create; -1 if the options are invalid.
#[no_mangle]
pub unsafe extern "C" fn pixmith_export_job_count(doc: *const Editor, opts: *const PixmithExport) -> i32 {
    crate::ffi::guarded("pixmith_export_job_count", std::ptr::null_mut(), || {
        let (Some(ed), Some(c)) = (crate::ffi::synced(doc), opts.as_ref()) else { return -1 };
        let Some((_, o, _)) = from_c(c) else { return -1 };
        specs(&ed.sprite, &o).map_or(-1, |v| v.len() as i32)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Sprite;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn tag(name: &str, from: u16, to: u16, direction: u8, repeat: u16) -> Tag {
        Tag { name: name.into(), from, to, direction, repeat, color: [0; 3], user_data: Default::default() }
    }

    /// 4x4, 4 frames; frame f has a pixel at (f, f) on layer 0.
    fn diagonal() -> Sprite {
        let mut s = Sprite::new(4, 4);
        for f in 1..4 {
            s.add_frame(f - 1, true);
        }
        for f in 0..4 {
            s.cels[f][0] = Some(std::sync::Arc::new(crate::document::Cel {
                opacity: 255,
                pixels: vec![0; 64],
                ..Default::default()
            }));
            s.draw_line(0, f, f as i32, f as i32, f as i32, f as i32, RED, 1);
        }
        s.durations = vec![10, 20, 30, 40];
        s
    }

    #[test]
    fn directions_expand_to_one_clean_cycle() {
        assert_eq!(direction_order(1, 4, 0), vec![1, 2, 3, 4]);
        assert_eq!(direction_order(1, 4, 1), vec![4, 3, 2, 1]);
        assert_eq!(direction_order(1, 4, 2), vec![1, 2, 3, 4, 3, 2]);
        assert_eq!(direction_order(1, 4, 3), vec![4, 3, 2, 1, 2, 3]);
        // Short tags never duplicate frames.
        assert_eq!(direction_order(5, 5, 2), vec![5]);
        assert_eq!(direction_order(5, 6, 2), vec![5, 6]);
        assert_eq!(direction_order(5, 6, 3), vec![6, 5]);
        assert_eq!(direction_order(2, 4, 2), vec![2, 3, 4, 3]);
    }

    #[test]
    fn tag_export_follows_direction_durations_and_repeat() {
        let mut s = diagonal();
        s.tags.push(tag("run", 0, 2, 2, 3));
        let o = ExportOptions { tag: Some(0), ..Default::default() };
        let jobs = plan(&s, &o).unwrap();
        assert_eq!(jobs.len(), 1);
        let j = &jobs[0];
        assert_eq!(j.frames.iter().map(|f| f.src_frame).collect::<Vec<_>>(), vec![0, 1, 2, 1]);
        assert_eq!(j.frames.iter().map(|f| f.duration).collect::<Vec<_>>(), vec![10, 20, 30, 20]);
        assert_eq!(j.repeat, 3);
        // Baked direction: the tag in the metadata is a plain forward range over the output.
        assert_eq!((j.tags[0].from, j.tags[0].to, j.tags[0].direction, j.tags[0].repeat), (0, 3, 0, 0));
        let o = ExportOptions { tag: Some(0), bake_direction: false, ..Default::default() };
        let j = &plan(&s, &o).unwrap()[0];
        assert_eq!(j.frames.len(), 3);
        assert_eq!((j.tags[0].direction, j.tags[0].repeat), (2, 3));
    }

    #[test]
    fn all_frames_ignores_directions_and_keeps_tags() {
        let mut s = diagonal();
        s.tags.push(tag("b", 2, 3, 1, 0));
        let j = &plan(&s, &ExportOptions::default()).unwrap()[0];
        assert_eq!(j.frames.len(), 4);
        assert_eq!((j.tags[0].from, j.tags[0].to, j.tags[0].direction), (2, 3, 1));
        assert!(plan(&s, &ExportOptions { tag: Some(5), ..Default::default() }).is_err());
    }

    #[test]
    fn split_tags_makes_one_job_per_tag() {
        let mut s = diagonal();
        s.tags.push(tag("a", 0, 1, 0, 0));
        s.tags.push(tag("b/c", 2, 3, 1, 0));
        let o = ExportOptions { split_tags: true, ..Default::default() };
        let jobs = plan(&s, &o).unwrap();
        assert_eq!(
            jobs.iter().map(|j| j.parts.clone()).collect::<Vec<_>>(),
            vec![vec!["a".to_string()], vec!["b/c".to_string()]]
        );
        assert_eq!(jobs[1].frames.iter().map(|f| f.src_frame).collect::<Vec<_>>(), vec![3, 2]);
        assert_eq!(job_paths("out/walk.png", &jobs, Format::Sheet), vec!["out/walk-a.png", "out/walk-b_c.png"]);
        // Without tags a split is the whole animation in one file.
        let plain = plan(&diagonal(), &o).unwrap();
        assert_eq!((plain.len(), plain[0].frames.len()), (1, 4));
        assert_eq!(job_paths("x.gif", &plain, Format::Gif), vec!["x.gif"]);
    }

    #[test]
    fn job_paths_handle_sequences_and_duplicates() {
        let mk = |parts: &[&str]| Job {
            parts: parts.iter().map(|p| p.to_string()).collect(),
            canvas: (1, 1),
            frames: vec![],
            tags: vec![],
            repeat: 0,
            trimmed: false,
        };
        let jobs = vec![mk(&["Body"]), mk(&["Body"]), mk(&["Head", "idle"])];
        assert_eq!(job_paths("d/a", &jobs, Format::Sequence), vec!["d/a-Body", "d/a-Body-2", "d/a-Head-idle"]);
        assert_eq!(
            job_paths("d/a.png", &jobs, Format::Png),
            vec!["d/a-Body.png", "d/a-Body-2.png", "d/a-Head-idle.png"]
        );
        assert_eq!(job_paths("a", &jobs[..1], Format::Gif), vec!["a-Body.gif"]);
    }

    fn two_layers() -> Sprite {
        let mut s = Sprite::new(2, 1);
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        let top = s.add_layer(0);
        s.layers[top].name = "top".into();
        s.draw_line(top, 0, 1, 0, 1, 0, BLUE, 1);
        let hidden = s.add_layer(top);
        s.layers[hidden].visible = false;
        s
    }

    #[test]
    fn layer_modes() {
        let s = two_layers();
        let all = &plan(&s, &ExportOptions::default()).unwrap()[0];
        assert_eq!(all.frames[0].rgba, [RED, BLUE].concat());
        let top = &plan(&s, &ExportOptions { layers: LayerMode::Selected(1), ..Default::default() }).unwrap()[0];
        assert_eq!(top.frames[0].rgba, [[0; 4], BLUE].concat());
        // A hidden layer can still be picked explicitly.
        assert!(plan(&s, &ExportOptions { layers: LayerMode::Selected(2), ..Default::default() }).is_ok());
        let each = plan(&s, &ExportOptions { layers: LayerMode::Each, ..Default::default() }).unwrap();
        assert_eq!(each.len(), 2); // the hidden layer is skipped
        assert_eq!(each[0].parts, vec![s.layers[0].name.clone()]);
        assert_eq!(each[0].frames[0].rgba, [RED, [0; 4]].concat());
        assert_eq!(each[1].parts, vec!["top".to_string()]);
    }

    #[test]
    fn each_layer_respects_groups() {
        let mut s = Sprite::new(1, 1);
        let mut g = crate::document::Layer::new("grp");
        g.kind = 1;
        let mut child = crate::document::Layer::new("child");
        child.child_level = 1;
        s.layers = vec![g, child];
        s.cels = vec![vec![None, None]];
        s.draw_line(1, 0, 0, 0, 0, 0, RED, 1);
        let each = plan(&s, &ExportOptions { layers: LayerMode::Each, ..Default::default() }).unwrap();
        assert_eq!(each.len(), 1);
        assert_eq!(each[0].frames[0].rgba, RED.to_vec());
        s.layers[0].visible = false;
        assert!(plan(&s, &ExportOptions { layers: LayerMode::Each, ..Default::default() }).is_err());
        // Selecting the child directly shows its group even though the group is hidden.
        let sel = &plan(&s, &ExportOptions { layers: LayerMode::Selected(1), ..Default::default() }).unwrap()[0];
        assert_eq!(sel.frames[0].rgba, RED.to_vec());
        assert!(plan(&s, &ExportOptions { layers: LayerMode::Selected(0), ..Default::default() }).is_err());
    }

    #[test]
    fn content_bounds_and_crop() {
        let mut px = vec![0u8; 5 * 4 * 4];
        for (x, y) in [(1, 1), (3, 2)] {
            px[(y * 5 + x) * 4 + 3] = 255;
        }
        assert_eq!(content_bounds(&px, 5, 4), Some((1, 1, 4, 3)));
        assert_eq!(content_bounds(&[0; 16], 2, 2), None);
        let c = crop(&px, 5, (1, 1, 4, 3));
        assert_eq!(c.len(), 3 * 2 * 4);
        assert_eq!(c[3], 255);
        assert_eq!(c[(1 * 3 + 2) * 4 + 3], 255);
    }

    #[test]
    fn trim_per_frame_and_uniform() {
        let s = diagonal();
        let per = &plan(&s, &ExportOptions { trim: Trim::PerFrame, ..Default::default() }).unwrap()[0];
        assert!(per.trimmed);
        for (f, fr) in per.frames.iter().enumerate() {
            assert_eq!((fr.w, fr.h, fr.off, fr.trimmed), (1, 1, (f, f), true));
            assert_eq!(fr.rgba, RED.to_vec());
        }
        let uni = &plan(&s, &ExportOptions { trim: Trim::Uniform, ..Default::default() }).unwrap()[0];
        for fr in &uni.frames {
            assert_eq!((fr.w, fr.h, fr.off), (4, 4, (0, 0)));
            assert!(!fr.trimmed); // the union is the whole canvas
        }
        assert!(!uni.trimmed);
        // A sprite whose content sits in the middle crops uniformly.
        let mut s = Sprite::new(8, 8);
        s.add_frame(0, true);
        s.draw_line(0, 0, 2, 3, 2, 3, RED, 1);
        s.draw_line(0, 1, 5, 4, 5, 4, BLUE, 1);
        let uni = &plan(&s, &ExportOptions { trim: Trim::Uniform, ..Default::default() }).unwrap()[0];
        assert!(uni.trimmed);
        assert!(uni.frames.iter().all(|f| (f.w, f.h, f.off, f.trimmed) == (4, 2, (2, 3), true)));
        assert_eq!(uni.frames[0].rgba[3], 255); // red at the crop's top-left
        assert_eq!(&uni.frames[1].rgba[(4 + 3) * 4..][..4], &BLUE);
    }

    #[test]
    fn ignore_empty_drops_blank_frames_and_remaps_tags() {
        let mut s = diagonal();
        s.cels[1][0] = None; // frame 1 becomes blank
        s.tags.push(tag("t", 1, 3, 0, 0));
        s.tags.push(tag("blank", 1, 1, 0, 0));
        let j = &plan(&s, &ExportOptions { ignore_empty: true, ..Default::default() }).unwrap()[0];
        assert_eq!(j.frames.iter().map(|f| f.src_frame).collect::<Vec<_>>(), vec![0, 2, 3]);
        assert_eq!(j.tags.len(), 1);
        assert_eq!((j.tags[0].from, j.tags[0].to), (1, 2));
        // Empty frames are kept as one pixel under trim, unless ignored.
        let j = &plan(&s, &ExportOptions { trim: Trim::PerFrame, ..Default::default() }).unwrap()[0];
        assert_eq!((j.frames[1].w, j.frames[1].h), (1, 1));
        assert_eq!(j.frames[1].rgba, vec![0; 4]);
    }

    #[test]
    fn outputs_write_split_files_with_sheet_json() {
        let mut s = diagonal();
        s.tags.push(tag("a", 0, 1, 0, 0));
        s.tags.push(tag("b", 2, 3, 0, 0));
        let o = ExportOptions { split_tags: true, trim: Trim::PerFrame, ..Default::default() };
        let e = EncodeOptions {
            scale: 1,
            frame: 0,
            sheet: sheet::Options::default(),
            write_json: true,
            image: Default::default(),
        };
        let files = render_outputs(&s, Format::Sheet, &o, &e, "d/sheet.png", None).unwrap();
        let names: Vec<_> = files.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(names, vec!["d/sheet-a.json", "d/sheet-a.png", "d/sheet-b.json", "d/sheet-b.png"]);
        let json = String::from_utf8(files[2].1.clone()).unwrap();
        assert!(json.contains("\"trimmed\": true"));
        assert!(json.contains("\"spriteSourceSize\": { \"x\": 2, \"y\": 2, \"w\": 1, \"h\": 1 }"));
        assert!(json.contains("\"sourceSize\": { \"w\": 4, \"h\": 4 }"));
        assert!(json.contains("\"image\": \"sheet-b.png\""));
        let seq = render_outputs(&s, Format::Sequence, &o, &e, "d/s", None).unwrap();
        assert_eq!(
            seq.iter().map(|f| f.0.clone()).collect::<Vec<_>>(),
            vec!["d/s-a-001.png", "d/s-a-002.png", "d/s-b-001.png", "d/s-b-002.png"]
        );
        let each = ExportOptions { layers: LayerMode::Each, ..Default::default() };
        let png = render_outputs(&two_layers(), Format::Png, &each, &e, "x.png", None).unwrap();
        assert_eq!(
            png.iter().map(|f| f.0.clone()).collect::<Vec<_>>(),
            vec![format!("x-{}.png", two_layers().layers[0].name), "x-top.png".to_string()]
        );
        let bad = EncodeOptions { frame: 9, ..e };
        assert!(render_outputs(&s, Format::Png, &ExportOptions::default(), &bad, "x.png", None).is_err());
    }

    #[test]
    fn animations_use_uniform_trim_and_tag_repeat() {
        let mut s = Sprite::new(6, 6);
        s.add_frame(0, true);
        s.draw_line(0, 0, 1, 1, 1, 1, RED, 1);
        s.draw_line(0, 1, 3, 4, 3, 4, BLUE, 1);
        s.tags.push(tag("loop", 0, 1, 0, 2));
        let o = ExportOptions { tag: Some(0), trim: Trim::PerFrame, ..Default::default() };
        let e = EncodeOptions {
            scale: 2,
            frame: 0,
            sheet: sheet::Options::default(),
            write_json: false,
            image: Default::default(),
        };
        let apng = &render_outputs(&s, Format::Apng, &o, &e, "a.png", None).unwrap()[0].1;
        let mut r = png::Decoder::new(&apng[..]).read_info().unwrap();
        let ctl = r.info().animation_control.unwrap();
        assert_eq!((ctl.num_frames, ctl.num_plays), (2, 2));
        let mut buf = vec![0; r.output_buffer_size()];
        let info = r.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (6, 8)); // union (1..4, 1..5) = 3x4, scaled by 2
        let gif = &render_outputs(&s, Format::Gif, &o, &e, "a.gif", None).unwrap()[0].1;
        let mut d = gif::DecodeOptions::new();
        d.set_color_output(gif::ColorOutput::RGBA);
        let dec = d.read_info(&gif[..]).unwrap();
        assert_eq!((dec.width(), dec.height()), (6, 8));
    }
}
