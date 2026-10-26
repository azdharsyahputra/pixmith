//! Raster image formats beyond PNG and GIF: WebP, JPEG, BMP, TGA, ICO, PCX and FLI/FLC.
//!
//! Every format has a module with `decode` (bytes to RGBA frames) and an encoder. This file holds what they share:
//! format detection from the content, the options of an export (`ImageOptions`), flattening onto a background
//! color, palette building for the indexed formats, and the two entry points the rest of the core uses:
//!
//! - import: [`sprite_from_image`] opens any supported file (PNG and GIF included) as a new sprite;
//! - export: [`encode_still`] and [`encode_animation`], called by `export_plan` for each planned job.
//!
//! The C ABI (`pixmith_import_image`, `pixmith_export_image`, ...) is in [`abi`] and declared in `pixmith_formats.h`.

pub mod abi;
pub mod bmp;
pub mod flic;
pub mod ico;
pub mod jpeg;
pub mod pcx;
pub mod tga;
pub mod webp;

use crate::document::{Sprite, MAX_CANVAS_PIXELS};
use crate::formats::export::{check_export_size, upscale};
use crate::formats::export_plan::{Job, PlanFrame};
use crate::formats::json::{self, Value};
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// A file format that can be opened (and, except for the cursor variants of ICO, written).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    Png,
    Gif,
    Webp,
    Jpeg,
    Bmp,
    Tga,
    Ico,
    Pcx,
    /// Autodesk Animator FLI and FLC.
    Flc,
}

impl ImageFormat {
    pub const ALL: [ImageFormat; 9] = [
        ImageFormat::Png,
        ImageFormat::Gif,
        ImageFormat::Webp,
        ImageFormat::Jpeg,
        ImageFormat::Bmp,
        ImageFormat::Tga,
        ImageFormat::Ico,
        ImageFormat::Pcx,
        ImageFormat::Flc,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ImageFormat::Png => "PNG",
            ImageFormat::Gif => "GIF",
            ImageFormat::Webp => "WebP",
            ImageFormat::Jpeg => "JPEG",
            ImageFormat::Bmp => "BMP",
            ImageFormat::Tga => "TGA",
            ImageFormat::Ico => "ICO",
            ImageFormat::Pcx => "PCX",
            ImageFormat::Flc => "FLI/FLC",
        }
    }

    /// The extension written by default.
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Gif => "gif",
            ImageFormat::Webp => "webp",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Bmp => "bmp",
            ImageFormat::Tga => "tga",
            ImageFormat::Ico => "ico",
            ImageFormat::Pcx => "pcx",
            ImageFormat::Flc => "flc",
        }
    }

    /// Every extension that names the format, lowercase.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            ImageFormat::Png => &["png"],
            ImageFormat::Gif => &["gif"],
            ImageFormat::Webp => &["webp"],
            ImageFormat::Jpeg => &["jpg", "jpeg", "jpe", "jfif"],
            ImageFormat::Bmp => &["bmp", "dib"],
            ImageFormat::Tga => &["tga", "icb", "vda", "vst"],
            ImageFormat::Ico => &["ico", "cur"],
            ImageFormat::Pcx => &["pcx"],
            ImageFormat::Flc => &["flc", "fli"],
        }
    }

    pub fn from_extension(ext: &str) -> Option<ImageFormat> {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        ImageFormat::ALL.into_iter().find(|f| f.extensions().contains(&ext.as_str()))
    }

    pub fn from_path(path: &str) -> Option<ImageFormat> {
        std::path::Path::new(path).extension().and_then(|e| e.to_str()).and_then(ImageFormat::from_extension)
    }

    /// Formats whose files hold several frames with durations.
    pub fn is_animated(self) -> bool {
        matches!(self, ImageFormat::Gif | ImageFormat::Webp | ImageFormat::Flc)
    }

    /// The names used by the options JSON and the command line.
    pub fn from_name(name: &str) -> Option<ImageFormat> {
        let n = name.trim().to_ascii_lowercase();
        match n.as_str() {
            "fli" | "flc" | "flic" => Some(ImageFormat::Flc),
            "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
            _ => ImageFormat::from_extension(&n),
        }
    }
}

/// Recognizes a file by its content. TGA has no signature, so it is accepted when the header is plausible.
pub fn detect(b: &[u8]) -> Option<ImageFormat> {
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some(ImageFormat::Png);
    }
    if b.starts_with(b"GIF8") {
        return Some(ImageFormat::Gif);
    }
    if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return Some(ImageFormat::Webp);
    }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(ImageFormat::Jpeg);
    }
    if b.len() >= 26 && b.starts_with(b"BM") {
        return Some(ImageFormat::Bmp);
    }
    if b.len() >= 22 && b[0] == 0 && b[1] == 0 && (b[2] == 1 || b[2] == 2) && b[3] == 0 && (b[4] != 0 || b[5] != 0) {
        return Some(ImageFormat::Ico);
    }
    if b.len() >= 128 && b[0] == 0x0A && matches!(b[1], 0 | 2 | 3 | 4 | 5) && b[2] == 1 && matches!(b[3], 1 | 2 | 4 | 8)
    {
        return Some(ImageFormat::Pcx);
    }
    if b.len() >= 128 && matches!(u16::from_le_bytes([b[4], b[5]]), 0xAF11 | 0xAF12) {
        return Some(ImageFormat::Flc);
    }
    tga::looks_like_tga(b).then_some(ImageFormat::Tga)
}

// ---- Decoding ----------------------------------------------------------------------------------

/// Largest memory an imported animation may use for its frames.
const MAX_ANIMATION_BYTES: u64 = 1 << 30;

/// Frames decoded from a file, all `width` x `height` straight-alpha RGBA.
#[derive(Clone, Debug)]
pub struct Decoded {
    pub width: usize,
    pub height: usize,
    pub frames: Vec<Vec<u8>>,
    /// Milliseconds per frame (same length as `frames`).
    pub durations: Vec<u16>,
    /// The file's own palette, when it has one.
    pub palette: Option<Vec<[u8; 4]>>,
}

impl Decoded {
    pub fn still(width: usize, height: usize, rgba: Vec<u8>) -> Decoded {
        Decoded { width, height, frames: vec![rgba], durations: vec![100], palette: None }
    }
}

/// Fails when an image of this size cannot be opened as a sprite.
pub(crate) fn check_dims(w: usize, h: usize) -> Result<(), String> {
    if w == 0 || h == 0 {
        return Err("the image has no pixels".into());
    }
    if w > u16::MAX as usize || h > u16::MAX as usize || w.saturating_mul(h) > MAX_CANVAS_PIXELS {
        return Err(format!("the image is too large ({w} x {h} pixels)"));
    }
    Ok(())
}

/// Fails when one more frame of `w` x `h` would exceed the limits on frame count or memory.
pub(crate) fn check_frame_budget(count: usize, w: usize, h: usize) -> Result<(), String> {
    if count >= u16::MAX as usize || (count as u64 + 1) * (w as u64 * h as u64 * 4) > MAX_ANIMATION_BYTES {
        return Err("the animation has too many frames".into());
    }
    Ok(())
}

/// Reads a little-endian integer, `None` past the end.
pub(crate) fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
pub(crate) fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Choices for opening a file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportOptions {
    /// ICO files hold several sizes; this picks the entry whose larger side matches (default: the largest).
    pub ico_size: Option<u32>,
    /// `"mode": "trace_grid"`: open the first image as pixel art traced onto its grid (see `paint::trace`).
    pub trace: Option<crate::paint::trace::TraceOptions>,
}

impl ImportOptions {
    pub fn from_json(text: &str) -> Result<ImportOptions, String> {
        if text.trim().is_empty() {
            return Ok(ImportOptions::default());
        }
        let v = json::parse(text)?;
        if v.as_object().is_none() {
            return Err("the options must be a JSON object".into());
        }
        let trace = match v.get("mode").and_then(Value::as_str) {
            None | Some("") | Some("normal") => None,
            Some("trace_grid") => Some(crate::paint::trace::TraceOptions::from_json(&v)?),
            Some(other) => return Err(format!("unknown import mode \"{other}\"")),
        };
        Ok(ImportOptions {
            ico_size: v.get("ico_size").and_then(Value::as_f64).filter(|n| *n >= 1.0).map(|n| n as u32),
            trace,
        })
    }
}

/// Runs a decoder that must never take the process down: a panic inside a codec becomes an error.
fn guard<T>(what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(format!("the {what} file could not be read (internal error)")))
}

/// Decodes the frames of a file of the given format.
pub fn decode(bytes: &[u8], fmt: ImageFormat, o: &ImportOptions) -> Result<Decoded, String> {
    guard(fmt.name(), || match fmt {
        ImageFormat::Png => {
            let (w, h, rgba) = crate::formats::import::decode_png(bytes)?;
            Ok(Decoded::still(w, h, rgba))
        }
        ImageFormat::Gif => {
            let s = crate::formats::animation::sprite_from_gif(bytes)?;
            Ok(Decoded {
                width: s.width as usize,
                height: s.height as usize,
                frames: (0..s.frame_count()).map(|f| s.render(f)).collect(),
                durations: s.durations.clone(),
                palette: None,
            })
        }
        ImageFormat::Webp => webp::decode(bytes),
        ImageFormat::Jpeg => jpeg::decode(bytes),
        ImageFormat::Bmp => bmp::decode(bytes),
        ImageFormat::Tga => tga::decode(bytes),
        ImageFormat::Ico => ico::decode(bytes, o.ico_size),
        ImageFormat::Pcx => pcx::decode(bytes),
        ImageFormat::Flc => flic::decode(bytes),
    })
}

/// Builds a one-layer sprite from decoded frames. A palette of the file is kept; otherwise the sprite gets the
/// image's own colors when there are few enough.
pub fn sprite_from_decoded(d: Decoded) -> Result<Sprite, String> {
    check_dims(d.width, d.height)?;
    if d.frames.is_empty() {
        return Err("the file has no frames".into());
    }
    let mut s = Sprite::new(d.width as u32, d.height as u32);
    let refs: Vec<&[u8]> = d.frames.iter().map(Vec::as_slice).collect();
    let file_palette = d.palette.as_ref().filter(|p| !p.is_empty() && p.len() <= 256).cloned();
    s.palette = file_palette.or_else(|| crate::formats::import::image_palette(&refs)).unwrap_or(s.palette);
    s.durations = d.durations.iter().map(|&ms| ms.max(1)).collect();
    s.cels = d.frames.into_iter().map(|px| vec![crate::formats::import::cel(px)]).collect();
    Ok(s)
}

/// Opens a file of any supported format, detected from its content. Animated formats give one frame per image.
pub fn sprite_from_image(bytes: &[u8], o: &ImportOptions) -> Result<Sprite, String> {
    let fmt = detect(bytes).ok_or("this file is not a supported image format")?;
    sprite_from_format(bytes, fmt, o)
}

pub fn sprite_from_format(bytes: &[u8], fmt: ImageFormat, o: &ImportOptions) -> Result<Sprite, String> {
    if let Some(t) = &o.trace {
        // Pseudo-pixel art is traced from the first image of the file.
        let d = decode(bytes, fmt, o)?;
        check_dims(d.width, d.height)?;
        let first = d.frames.first().ok_or("the file has no frames")?;
        return crate::paint::trace::sprite_from_trace(first, d.width, d.height, t);
    }
    match fmt {
        ImageFormat::Png => crate::formats::import::sprite_from_png(bytes),
        ImageFormat::Gif => guard("GIF", || crate::formats::animation::sprite_from_gif(bytes)),
        _ => sprite_from_decoded(decode(bytes, fmt, o)?),
    }
}

/// Like [`sprite_from_image`], with the file name as a hint for formats that have no signature.
pub fn sprite_from_file_bytes(bytes: &[u8], path: &str, o: &ImportOptions) -> Result<Sprite, String> {
    match detect(bytes).or_else(|| ImageFormat::from_path(path)) {
        Some(fmt) => sprite_from_format(bytes, fmt, o),
        None => Err("this file is not a supported image format".into()),
    }
}

/// Opens a Pixmith or Aseprite document, or else an image of any supported format (the command line uses this for
/// every input). The error of the document reader is reported when the file is not an image either.
pub fn read_any(bytes: &[u8], path: &str) -> Result<Sprite, String> {
    match crate::formats::pxmt::read_document(bytes) {
        Ok(s) => Ok(s),
        Err(e) => {
            if detect(bytes).is_none() && ImageFormat::from_path(path).is_none() {
                return Err(e);
            }
            sprite_from_file_bytes(bytes, path, &ImportOptions::default())
        }
    }
}

// ---- Export options ----------------------------------------------------------------------------

/// Format specific settings of an export. Every format ignores the settings that are not its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageOptions {
    /// Transparent pixels are flattened onto this color by the formats without alpha (JPEG, PCX, FLC and the
    /// 8 and 24 bit modes of BMP and TGA).
    pub background: [u8; 3],
    /// JPEG quality, 1 to 100.
    pub jpeg_quality: u8,
    /// WebP: lossless (exact pixels) or lossy.
    pub webp_lossless: bool,
    /// WebP quality 0 to 100: image quality when lossy, compression effort when lossless.
    pub webp_quality: u8,
    /// WebP: write every frame as an animation (otherwise only one frame).
    pub webp_animated: bool,
    /// BMP, TGA and PCX: bits per pixel, 0 to choose (alpha: 32, up to 256 colors: 8, else 24).
    pub bits: u8,
    /// TGA: run-length compression.
    pub tga_rle: bool,
    /// ICO: the square sizes to write (at most 8). With no sizes the icon has the sprite's own size.
    pub ico_sizes: [u16; 8],
    pub ico_count: u8,
}

impl Default for ImageOptions {
    fn default() -> Self {
        ImageOptions {
            background: [255, 255, 255],
            jpeg_quality: 90,
            webp_lossless: true,
            webp_quality: 90,
            webp_animated: true,
            bits: 0,
            tga_rle: true,
            ico_sizes: [0; 8],
            ico_count: 0,
        }
    }
}

/// Sizes the export sheets offer for ICO files.
pub const ICO_SIZES: [u16; 8] = [16, 24, 32, 48, 64, 128, 256, 512];

impl ImageOptions {
    pub fn ico_size_list(&self) -> Vec<u16> {
        self.ico_sizes[..(self.ico_count as usize).min(8)].to_vec()
    }

    pub fn set_ico_sizes(&mut self, sizes: &[u16]) {
        let mut list: Vec<u16> = sizes.iter().copied().filter(|s| (1..=256).contains(s)).collect();
        list.sort_unstable();
        list.dedup();
        list.truncate(8);
        self.ico_sizes = [0; 8];
        self.ico_sizes[..list.len()].copy_from_slice(&list);
        self.ico_count = list.len() as u8;
    }

    /// Reads the options from JSON. Missing keys keep their defaults. Keys: `background` ("#rrggbb" or [r,g,b]),
    /// `quality` (applies to JPEG and WebP; `jpeg_quality` and `webp_quality` set one), `lossless`, `animated`,
    /// `bits`, `rle`, `ico_sizes` (list of sizes up to 256).
    pub fn from_json(text: &str) -> Result<ImageOptions, String> {
        let mut o = ImageOptions::default();
        if text.trim().is_empty() {
            return Ok(o);
        }
        let v = json::parse(text)?;
        if v.as_object().is_none() {
            return Err("the options must be a JSON object".into());
        }
        let num = |k: &str| v.get(k).and_then(Value::as_f64);
        let flag = |k: &str| match v.get(k) {
            Some(Value::Bool(b)) => Some(*b),
            _ => None,
        };
        if let Some(q) = num("quality") {
            o.jpeg_quality = q.clamp(1.0, 100.0) as u8;
            o.webp_quality = q.clamp(0.0, 100.0) as u8;
        }
        if let Some(q) = num("jpeg_quality") {
            o.jpeg_quality = q.clamp(1.0, 100.0) as u8;
        }
        if let Some(q) = num("webp_quality") {
            o.webp_quality = q.clamp(0.0, 100.0) as u8;
        }
        o.webp_lossless = flag("lossless").unwrap_or(o.webp_lossless);
        o.webp_animated = flag("animated").unwrap_or(o.webp_animated);
        o.tga_rle = flag("rle").unwrap_or(o.tga_rle);
        if let Some(b) = num("bits") {
            if ![0.0, 8.0, 24.0, 32.0].contains(&b) {
                return Err("bits must be 0 (automatic), 8, 24 or 32".into());
            }
            o.bits = b as u8;
        }
        match v.get("background") {
            Some(Value::Str(s)) => o.background = parse_hex_color(s)?,
            Some(Value::Arr(a)) if a.len() >= 3 => {
                for (i, c) in a.iter().take(3).enumerate() {
                    o.background[i] = c.as_f64().ok_or("background: expected numbers")?.clamp(0.0, 255.0) as u8;
                }
            }
            Some(_) => return Err("background: expected \"#rrggbb\" or [r, g, b]".into()),
            None => {}
        }
        if let Some(Value::Arr(a)) = v.get("ico_sizes") {
            let sizes: Vec<u16> = a.iter().filter_map(Value::as_f64).map(|n| n.clamp(0.0, 65535.0) as u16).collect();
            if sizes.iter().any(|s| !(1..=256).contains(s)) {
                return Err("ico_sizes: every size must be between 1 and 256".into());
            }
            o.set_ico_sizes(&sizes);
        }
        Ok(o)
    }
}

pub fn parse_hex_color(s: &str) -> Result<[u8; 3], String> {
    let h = s.trim().trim_start_matches('#');
    let bad = || format!("\"{s}\" is not a color; use #rrggbb");
    let full: String = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 => h.to_string(),
        _ => return Err(bad()),
    };
    let n = u32::from_str_radix(&full, 16).map_err(|_| bad())?;
    Ok([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

// ---- Encoding helpers --------------------------------------------------------------------------

/// Composites straight-alpha RGBA onto an opaque background, in place.
pub fn flatten(rgba: &mut [u8], bg: [u8; 3]) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a != 255 {
            for k in 0..3 {
                px[k] = ((px[k] as u32 * a + bg[k] as u32 * (255 - a) + 127) / 255) as u8;
            }
            px[3] = 255;
        }
    }
}

pub fn has_alpha(rgba: &[u8]) -> bool {
    rgba.chunks_exact(4).any(|p| p[3] != 255)
}

/// An opaque palette for the frames (already flattened) and the index of every pixel. The palette holds the exact
/// colors when there are at most `max`, otherwise a median-cut approximation with the nearest color per pixel.
pub fn indexed(frames: &[&[u8]], w: usize, h: usize, max: usize) -> (Vec<[u8; 3]>, Vec<Vec<u8>>) {
    let mut sprite = Sprite::new(w as u32, h as u32);
    sprite.cels = frames.iter().map(|f| vec![crate::formats::import::cel(f.to_vec())]).collect();
    sprite.durations = vec![100; frames.len().max(1)];
    let mut palette: Vec<[u8; 3]> = crate::document::color_mode::palette_from_sprite(&sprite, max.clamp(1, 256))
        .into_iter()
        .map(|c| [c[0], c[1], c[2]])
        .collect();
    if palette.is_empty() {
        palette.push([0, 0, 0]);
    }
    let mut lookup: HashMap<[u8; 3], u8> = palette.iter().enumerate().map(|(i, c)| (*c, i as u8)).rev().collect();
    let planes = frames
        .iter()
        .map(|f| {
            f.chunks_exact(4)
                .map(|p| {
                    let c = [p[0], p[1], p[2]];
                    *lookup.entry(c).or_insert_with(|| nearest(&palette, c))
                })
                .collect()
        })
        .collect();
    (palette, planes)
}

fn nearest(palette: &[[u8; 3]], c: [u8; 3]) -> u8 {
    let dist = |p: &[u8; 3]| (0..3).map(|k| (p[k] as i32 - c[k] as i32).pow(2)).sum::<i32>();
    palette.iter().enumerate().min_by_key(|(_, p)| dist(p)).map_or(0, |(i, _)| i as u8)
}

/// The exact number of distinct opaque colors, counted up to `limit + 1`.
pub fn color_count(rgba: &[u8], limit: usize) -> usize {
    let mut seen = std::collections::HashSet::new();
    for p in rgba.chunks_exact(4) {
        seen.insert([p[0], p[1], p[2]]);
        if seen.len() > limit {
            break;
        }
    }
    seen.len()
}

/// A frame ready for a still encoder: scaled up, with the sprite size checked.
fn scaled(f: &PlanFrame, scale: usize) -> Result<(Vec<u8>, usize, usize), String> {
    let scale = scale.clamp(1, 64);
    let (w, h) = (f.w * scale, f.h * scale);
    check_export_size(w, h)?;
    Ok((upscale(&f.rgba, f.w, f.h, scale), w, h))
}

/// The bytes of one frame in a single-image format.
pub fn encode_still(fmt: ImageFormat, f: &PlanFrame, scale: usize, o: &ImageOptions) -> Result<Vec<u8>, String> {
    if fmt == ImageFormat::Ico {
        return ico::encode_frame(f, scale, o);
    }
    let (rgba, w, h) = scaled(f, scale)?;
    match fmt {
        ImageFormat::Png => crate::formats::export::encode_png(&rgba, w, h),
        ImageFormat::Webp => webp::encode(&[(&rgba[..], 100)], w, h, o, 1),
        ImageFormat::Jpeg => jpeg::encode(&rgba, w, h, o),
        ImageFormat::Bmp => bmp::encode(&rgba, w, h, o),
        ImageFormat::Tga => tga::encode(&rgba, w, h, o),
        ImageFormat::Pcx => pcx::encode(&rgba, w, h, o),
        ImageFormat::Flc => flic::encode(&[(&rgba[..], 100)], w, h, 1, o),
        ImageFormat::Gif => Err("GIF is exported through the animation path".into()),
        ImageFormat::Ico => unreachable!(),
    }
}

/// The bytes of all frames of a job in an animated format (`repeat` plays, 0 = forever).
pub fn encode_animation(fmt: ImageFormat, job: &Job, scale: usize, o: &ImageOptions) -> Result<Vec<u8>, String> {
    let first = job.frames.first().ok_or("there are no frames to export")?;
    if job.frames.iter().any(|f| (f.w, f.h) != (first.w, first.h)) {
        return Err("the frames differ in size; use a uniform trim for animations".into());
    }
    let scale = scale.clamp(1, 64);
    let (w, h) = (first.w * scale, first.h * scale);
    check_export_size(w, h)?;
    if (job.frames.len() as u64).saturating_mul(w as u64 * h as u64 * 4) > MAX_ANIMATION_BYTES {
        return Err("the animation is too large to export; use a smaller scale".into());
    }
    let images: Vec<Vec<u8>> = job.frames.iter().map(|f| upscale(&f.rgba, f.w, f.h, scale)).collect();
    let timed: Vec<(&[u8], u16)> =
        images.iter().map(Vec::as_slice).zip(job.frames.iter().map(|f| f.duration)).collect();
    match fmt {
        ImageFormat::Webp => webp::encode(&timed, w, h, o, job.repeat),
        ImageFormat::Flc => flic::encode(&timed, w, h, job.repeat, o),
        _ => Err(format!("{} is not an animation format", fmt.name())),
    }
}

/// Whether the format is written as one animation per job (the others write the chosen frame).
pub fn writes_animation(fmt: ImageFormat, o: &ImageOptions) -> bool {
    match fmt {
        ImageFormat::Flc => true,
        ImageFormat::Webp => o.webp_animated,
        _ => false,
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use crate::formats::export_plan::PlanFrame;

    /// A recognizable `w` x `h` picture: a gradient with a transparent corner and a semi-transparent pixel.
    pub fn picture(w: usize, h: usize, alpha: bool) -> Vec<u8> {
        let mut v = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let mut p = [(x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, ((x + y) * 7 % 256) as u8, 255];
                if alpha && x == 0 && y == 0 {
                    p[3] = 0;
                }
                if alpha && x == 1 && y == 0 {
                    p[3] = 128;
                }
                v.extend_from_slice(&p);
            }
        }
        v
    }

    pub fn frame(rgba: Vec<u8>, w: usize, h: usize, duration: u16) -> PlanFrame {
        PlanFrame { rgba, w, h, off: (0, 0), trimmed: false, duration, src_frame: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_formats_by_content_and_extension() {
        assert_eq!(detect(b"GIF89a....."), Some(ImageFormat::Gif));
        assert_eq!(detect(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]), Some(ImageFormat::Jpeg));
        assert_eq!(detect(b"RIFF\x10\0\0\0WEBPVP8 "), Some(ImageFormat::Webp));
        assert_eq!(detect(b"hello world, not an image"), None);
        assert_eq!(detect(&[]), None);
        assert_eq!(ImageFormat::from_path("a/b/Icon.ICO"), Some(ImageFormat::Ico));
        assert_eq!(ImageFormat::from_path("x.JPEG"), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_path("x.fli"), Some(ImageFormat::Flc));
        assert_eq!(ImageFormat::from_path("x.txt"), None);
        assert_eq!(ImageFormat::from_name("FLC"), Some(ImageFormat::Flc));
    }

    #[test]
    fn options_parse_and_reject_nonsense() {
        let o = ImageOptions::from_json(
            r##"{"quality": 40, "background": "#102030", "lossless": false, "ico_sizes": [32, 16, 32]}"##,
        )
        .unwrap();
        assert_eq!((o.jpeg_quality, o.webp_quality, o.background, o.webp_lossless), (40, 40, [16, 32, 48], false));
        assert_eq!(o.ico_size_list(), vec![16, 32]);
        assert_eq!(ImageOptions::from_json("").unwrap(), ImageOptions::default());
        assert!(ImageOptions::from_json("[1]").is_err());
        assert!(ImageOptions::from_json(r#"{"bits": 7}"#).is_err());
        assert!(ImageOptions::from_json(r#"{"ico_sizes": [0]}"#).is_err());
        assert!(ImageOptions::from_json(r#"{"background": "red"}"#).is_err());
        assert_eq!(parse_hex_color("#f0a").unwrap(), [255, 0, 170]);
    }

    #[test]
    fn flatten_blends_onto_the_background() {
        let mut px = vec![255, 0, 0, 0, 0, 0, 255, 255, 100, 100, 100, 128];
        flatten(&mut px, [255, 255, 255]);
        assert_eq!(&px[0..4], &[255, 255, 255, 255]);
        assert_eq!(&px[4..8], &[0, 0, 255, 255]);
        assert_eq!(px[11], 255);
        assert!((177..=179).contains(&px[8]));
    }

    #[test]
    fn indexed_is_exact_for_few_colors_and_bounded_for_many() {
        let few = test_util::picture(4, 4, false);
        let (pal, planes) = indexed(&[&few], 4, 4, 256);
        assert_eq!(planes[0].len(), 16);
        for (i, p) in few.chunks_exact(4).enumerate() {
            assert_eq!(pal[planes[0][i] as usize], [p[0], p[1], p[2]]);
        }
        let many = test_util::picture(40, 40, false);
        let (pal, planes) = indexed(&[&many], 40, 40, 16);
        assert!(pal.len() <= 16);
        assert!(planes[0].iter().all(|&i| (i as usize) < pal.len()));
    }
}
