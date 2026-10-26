//! Command-line companion to Pixmith: inspect, convert and export .pxmt and .aseprite files (and open WebP, JPEG,
//! BMP, TGA, ICO, PCX and FLI/FLC images) without the app.

use pixmith_core::document::{Sprite, Tag};
use pixmith_core::formats::export_plan::{render_outputs, render_outputs_warn, EncodeOptions, ExportOptions, Format, LayerMode, Trim};
use pixmith_core::formats::raster::{self, ImageFormat, ImageOptions};
use pixmith_core::formats::sheet::{json_str, Layout, Options};
use pixmith_core::formats::{gif, pxmt};
use std::collections::HashSet;
use std::fmt::Write;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
Usage:
  pixmith-cli info <file>... [--json]
  pixmith-cli export <file>... [outputs] [options]
  pixmith-cli convert <in> <out> [image options]

Every <file> may be a .pxmt (Pixmith) or .aseprite / .ase document, or an image (PNG, GIF, WebP, JPEG, BMP, TGA,
ICO, PCX, FLI/FLC; an animation gives one frame per image); the format is detected from the content.
`convert` writes <out> as .pxmt or .aseprite (.ase), or as .webp, .jpg, .bmp, .tga, .ico, .pcx or .flc, according
to its extension (the first frame, except for animated .webp and .flc).

Outputs (at least one):
  --png <out.png>        one frame as PNG (see --frame)
  --gif <out.gif>        animated GIF
  --apng <out.png>       animated PNG
  --sheet <out.png>      sprite sheet (see --layout, --columns, --padding)
  --json <out.json>      sprite sheet metadata (needs --sheet)
  --godot-tres <f.tres>  Godot 4 SpriteFrames + atlas PNG (needs --godot-root, see below)
  --png-seq <dir/base>   every frame as <dir/base>-001.png, -002.png, ...
  --webp <out.webp>      WebP: animated when the sprite has several frames (lossless unless --lossy)
  --jpg <out.jpg>        JPEG of one frame, flattened onto --background (also --jpeg)
  --bmp <out.bmp>        BMP of one frame (32 bit with alpha, 8 bit for up to 256 colors, else 24 bit)
  --tga <out.tga>        TGA of one frame (RLE unless --no-rle)
  --ico <out.ico>        Windows icon with the sizes of --ico-sizes (default: the sprite's own size)
  --pcx <out.pcx>        PCX of one frame (8 bit palette, or 24 bit)
  --flc <out.flc>        Autodesk Animator FLC with all frames and their durations (also --fli)
  --godot-tileset <f.tres>  Godot 4 TileSet plus atlas PNG (needs --godot-root, see --tileset)

Image format options (export and convert):
  --quality <1-100>      JPEG quality (default 90); WebP quality when lossy (default 90)
  --lossy, --lossless    WebP: lossy with --quality, or lossless (default)
  --no-animation         WebP: write only the --frame instead of the whole animation
  --background <#rrggbb> color that transparent pixels are flattened onto for formats without alpha (default #ffffff)
  --ico-sizes <list>     icon sizes, e.g. 16,32,48,256 (each 1-256); the sprite is scaled by a whole number to fit
  --bits <8|24|32>       BMP, TGA and PCX depth (default: chosen from the image)
  --no-rle               TGA: do not run-length encode

Options:
  --godot-root <dir>     Godot project folder (holds project.godot); output must be inside it
  --tileset <n>          tileset to export with --godot-tileset (default 0)
  --godot-mode <m>       terrain mode when the file has no auto-tile data: blob (default), corners, sides
  --godot-collision <c>  none (default), full (tile square) or opaque (traced from opaque pixels)
  --godot-scene          also write <f>.tscn with TileMapLayer nodes painted from the tilemap layers
  --godot-margin <n>, --godot-separation <n>   atlas margins / spacing in pixels
  --godot-extrude <n>    replicate tile edges n pixels outward (avoids texture bleeding)
  --godot-max-texture <n>  atlas page size limit (default 4096; more pages are written beyond it)
  --godot-terrain <name>, --godot-color <r,g,b[,a]>   name and color of the terrain
  --frame <n>            frame for --png, counted from 1 like the editor (default 1)
  --scale <n>            integer upscale, 1-64 (default 1)
  --layout <l>           horizontal (default), vertical or grid
  --columns <n>          columns of a grid layout (implies --layout grid)
  --padding <n>          transparent pixels between frames and around the sheet
  --border-padding <n>   transparent margin around the whole sheet (default: --padding)
  --sheet-extrude <n>    repeat each sheet frame's edge pixels n pixels outward
  --layer <name>         only render layers with this name (repeatable)
  --tag <name>           only export the frames of this tag, played in the tag's direction
                         (reverse, ping-pong) and, for GIF/APNG, repeated as often as the tag says;
                         --frame counts within it
  --no-direction         with --tag, keep the tag's frames in order (direction stays in the JSON)
  --split-tags           one output per tag: <out>-<tag>.<ext>
  --split-layers         one output per visible layer: <out>-<layer>.<ext>
  --trim                 crop transparent borders per frame (GIF/APNG: one rectangle for all frames);
                         sheet JSON gets spriteSourceSize and trimmed
  --trim-sprite          crop all frames to the union of their content
  --ignore-empty         leave out frames without visible pixels
  -v, --verbose          print the files written

Godot 4 export (--godot-root DIR --godot-tres DIR/art/player.tres):
  --godot-root <dir>     Godot project folder (contains project.godot); output must be inside
  --padding <n>          pixels around every frame in the atlas
  --extrude              copy edge pixels 1px into the padding (avoids texture bleeding)
  --trim                 crop transparent borders (AtlasTexture margin restores the size)
  --max-texture <n>      largest atlas page: 2048, 4096 (default), 8192, 16384
  --packing <p>          tight (default) or grid
  --per-animation-png    one atlas PNG per animation
  --name-template <t>    file stem template with {name} and {layer} (default {name})
  --godot-layers <m>     visible (default) or each (one SpriteFrames per layer)
  --tag-include <name>   only this tag (repeatable)    --tag-exclude <name>   skip this tag (repeatable)
  --godot-tscn           also write a .tscn with an AnimatedSprite2D (texture_filter = nearest)
  --godot-overwrite-tscn replace an existing .tscn
  --godot-settings       take root, path and options saved in the document (flags override paths)
  --layer <name> limits the layers that are exported; --scale is an integer upscale.
  -h, --help             show this help

With several input files, every output path must contain {name}, which is replaced
by the input file's name without extension.

Exit codes: 0 success, 1 a file could not be read or exported, 2 bad usage.";

/// Errors are usage mistakes, reported with exit code 2.
type Res<T> = Result<T, String>;

fn usage<T>(msg: impl Into<String>) -> Res<T> {
    Err(msg.into())
}

#[derive(Default)]
struct ExportArgs {
    png: Option<String>,
    gif: Option<String>,
    apng: Option<String>,
    sheet: Option<String>,
    json: Option<String>,
    png_seq: Option<String>,
    godot_root: Option<String>,
    godot_tileset: Option<String>,
    tileset: Option<usize>,
    godot: pixmith_core::gamedev::godot::tileset::Options,
    godot_opts: bool,
    frame: Option<usize>,
    scale: Option<usize>,
    layout: Option<String>,
    columns: Option<usize>,
    padding: Option<usize>,
    border: Option<usize>,
    sheet_extrude: Option<usize>,
    layers: Vec<String>,
    tag: Option<String>,
    no_direction: bool,
    split_tags: bool,
    split_layers: bool,
    trim_sprite: bool,
    ignore_empty: bool,
    verbose: bool,
    godot_tres: Option<String>,
    godot_settings: bool,
    extrude: bool,
    trim: bool,
    max_texture: Option<u32>,
    packing: Option<String>,
    per_animation_png: bool,
    name_template: Option<String>,
    godot_layers: Option<String>,
    tag_include: Vec<String>,
    tag_exclude: Vec<String>,
    godot_tscn: bool,
    godot_overwrite_tscn: bool,
    images: Vec<(ImageFormat, String)>,
    image: ImageFlags,
}

/// Settings of the WebP, JPEG, BMP, TGA, ICO, PCX and FLC outputs.
#[derive(Default)]
struct ImageFlags {
    quality: Option<u8>,
    lossy: bool,
    no_animation: bool,
    background: Option<String>,
    ico_sizes: Option<Vec<u16>>,
    bits: Option<u8>,
    no_rle: bool,
}

impl ImageFlags {
    /// Reads one of the image format options; false when `flag` is not one.
    fn parse(&mut self, flag: &str, value: &mut dyn FnMut(&str) -> Res<String>) -> Res<bool> {
        match flag {
            "--quality" => {
                let q = number(flag, &value(flag)?)?;
                if !(1..=100).contains(&q) {
                    return usage("--quality: expected a number from 1 to 100");
                }
                self.quality = Some(q as u8);
            }
            "--lossy" => self.lossy = true,
            "--lossless" => self.lossy = false,
            "--no-animation" => self.no_animation = true,
            "--no-rle" => self.no_rle = true,
            "--background" => self.background = Some(value(flag)?),
            "--bits" => {
                let b = number(flag, &value(flag)?)?;
                if ![8, 24, 32].contains(&b) {
                    return usage("--bits: expected 8, 24 or 32");
                }
                self.bits = Some(b as u8);
            }
            "--ico-sizes" => {
                let v = value(flag)?;
                let mut sizes = Vec::new();
                for part in v.split(',').filter(|p| !p.trim().is_empty()) {
                    let n = number(flag, part.trim())?;
                    if !(1..=256).contains(&n) {
                        return usage("--ico-sizes: every size must be from 1 to 256");
                    }
                    sizes.push(n as u16);
                }
                if sizes.is_empty() || sizes.len() > 8 {
                    return usage("--ico-sizes: give between 1 and 8 sizes, e.g. 16,32,48,256");
                }
                self.ico_sizes = Some(sizes);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn options(&self) -> Res<ImageOptions> {
        let mut o = ImageOptions {
            webp_lossless: !self.lossy,
            webp_animated: !self.no_animation,
            tga_rle: !self.no_rle,
            ..Default::default()
        };
        if let Some(q) = self.quality {
            o.jpeg_quality = q;
            o.webp_quality = q;
        }
        if let Some(b) = &self.background {
            o.background = raster::parse_hex_color(b).or_else(|e| usage(format!("--background: {e}")))?;
        }
        if let Some(sizes) = &self.ico_sizes {
            o.set_ico_sizes(sizes);
        }
        o.bits = self.bits.unwrap_or(0);
        Ok(o)
    }
}

/// `--webp` and the other image outputs of `export`.
const IMAGE_FLAGS: [(&str, ImageFormat); 9] = [
    ("--webp", ImageFormat::Webp),
    ("--jpg", ImageFormat::Jpeg),
    ("--jpeg", ImageFormat::Jpeg),
    ("--bmp", ImageFormat::Bmp),
    ("--tga", ImageFormat::Tga),
    ("--ico", ImageFormat::Ico),
    ("--pcx", ImageFormat::Pcx),
    ("--flc", ImageFormat::Flc),
    ("--fli", ImageFormat::Flc),
];

impl ExportArgs {
    /// A SpriteFrames export was requested (`--godot-root` alone may belong to a TileSet export).
    fn godot(&self) -> bool {
        self.godot_tres.is_some() || self.godot_settings
    }
}

// Built once from the command line, so the size difference between variants does not matter.
#[allow(clippy::large_enum_variant)]
enum Command {
    Help,
    Info { files: Vec<String>, json: bool },
    Convert { input: String, output: String, image: ImageFlags },
    Export { files: Vec<String>, args: ExportArgs },
}

fn number(flag: &str, v: &str) -> Res<usize> {
    v.parse().or_else(|_| usage(format!("{flag}: expected a whole number, got \"{v}\"")))
}

fn parse(args: Vec<String>) -> Res<Command> {
    let mut it = args.into_iter();
    let Some(cmd) = it.next() else { return usage("missing command") };
    let export = match cmd.as_str() {
        "-h" | "--help" | "help" => return Ok(Command::Help),
        "info" => false,
        "export" => true,
        "convert" => {
            let mut rest: Vec<String> = it.collect();
            if rest.len() == 1 && (rest[0] == "-h" || rest[0] == "--help") {
                return Ok(Command::Help);
            }
            let (mut files, mut image) = (Vec::new(), ImageFlags::default());
            rest.reverse();
            while let Some(arg) = rest.pop() {
                if !arg.starts_with('-') {
                    files.push(arg);
                    continue;
                }
                let (flag, mut inline) = match arg.split_once('=') {
                    Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
                    _ => (arg.clone(), None),
                };
                let mut value = |flag: &str| match inline.take().or_else(|| rest.pop()) {
                    Some(v) => Ok(v),
                    None => usage(format!("{flag} needs a value")),
                };
                if !image.parse(&flag, &mut value)? {
                    return usage(format!("unknown option \"{flag}\" for convert"));
                }
            }
            return match files.as_slice() {
                [input, output] => Ok(Command::Convert { input: input.clone(), output: output.clone(), image }),
                _ => usage("convert takes exactly an input and an output file: convert <in> <out>"),
            };
        }
        other => return usage(format!("unknown command \"{other}\"")),
    };
    let (mut files, mut json_flag, mut a) = (Vec::new(), false, ExportArgs::default());
    while let Some(arg) = it.next() {
        if !arg.starts_with('-') || arg == "-" {
            files.push(arg);
            continue;
        }
        let (flag, mut inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let has_inline = inline.is_some();
        let mut value = |flag: &str| match inline.take().or_else(|| it.next()) {
            Some(v) => Ok(v),
            None => usage(format!("{flag} needs a value")),
        };
        match (flag.as_str(), export) {
            ("-h" | "--help", _) => return Ok(Command::Help),
            ("--json", false) if !has_inline => json_flag = true,
            ("--png", true) => a.png = Some(value("--png")?),
            ("--gif", true) => a.gif = Some(value("--gif")?),
            ("--apng", true) => a.apng = Some(value("--apng")?),
            ("--sheet", true) => a.sheet = Some(value("--sheet")?),
            ("--json", true) => a.json = Some(value("--json")?),
            ("--png-seq", true) => a.png_seq = Some(value("--png-seq")?),
            ("--godot-root", true) => a.godot_root = Some(value("--godot-root")?),
            ("--godot-tileset", true) => a.godot_tileset = Some(value("--godot-tileset")?),
            (f, true)
                if f.starts_with("--godot-")
                    && !SPRITE_FRAMES_FLAGS.contains(&f)
                    && f != "--godot-root"
                    && f != "--godot-tileset" =>
            {
                godot_option(&mut a, f, &mut value)?;
            }
            ("--tileset", true) => a.tileset = Some(number("--tileset", &value("--tileset")?)?),
            ("--frame", true) => a.frame = Some(number("--frame", &value("--frame")?)?),
            ("--scale", true) => a.scale = Some(number("--scale", &value("--scale")?)?),
            ("--columns", true) => a.columns = Some(number("--columns", &value("--columns")?)?),
            ("--padding", true) => a.padding = Some(number("--padding", &value("--padding")?)?),
            ("--border-padding", true) => a.border = Some(number("--border-padding", &value("--border-padding")?)?),
            ("--sheet-extrude", true) => a.sheet_extrude = Some(number("--sheet-extrude", &value("--sheet-extrude")?)?),
            ("--no-direction", true) if !has_inline => a.no_direction = true,
            ("--split-tags", true) if !has_inline => a.split_tags = true,
            ("--split-layers", true) if !has_inline => a.split_layers = true,
            ("--trim-sprite", true) if !has_inline => a.trim_sprite = true,
            ("--ignore-empty", true) if !has_inline => a.ignore_empty = true,
            ("--layout", true) => a.layout = Some(value("--layout")?),
            ("--layer", true) => a.layers.push(value("--layer")?),
            ("--tag", true) => a.tag = Some(value("--tag")?),
            ("--godot-tres", true) => a.godot_tres = Some(value("--godot-tres")?),
            ("--godot-settings", true) if !has_inline => a.godot_settings = true,
            ("--extrude", true) if !has_inline => a.extrude = true,
            ("--trim", true) if !has_inline => a.trim = true,
            ("--max-texture", true) => a.max_texture = Some(number("--max-texture", &value("--max-texture")?)? as u32),
            ("--packing", true) => a.packing = Some(value("--packing")?),
            ("--per-animation-png", true) if !has_inline => a.per_animation_png = true,
            ("--name-template", true) => a.name_template = Some(value("--name-template")?),
            ("--godot-layers", true) => a.godot_layers = Some(value("--godot-layers")?),
            ("--tag-include", true) => a.tag_include.push(value("--tag-include")?),
            ("--tag-exclude", true) => a.tag_exclude.push(value("--tag-exclude")?),
            ("--godot-tscn", true) if !has_inline => a.godot_tscn = true,
            ("--godot-overwrite-tscn", true) if !has_inline => a.godot_overwrite_tscn = true,
            ("-v" | "--verbose", true) => a.verbose = true,
            (f, true) if IMAGE_FLAGS.iter().any(|(n, _)| *n == f) => {
                let fmt = IMAGE_FLAGS.iter().find(|(n, _)| *n == f).map(|(_, k)| *k).unwrap_or(ImageFormat::Webp);
                a.images.push((fmt, value(f)?));
            }
            (f, true) if a.image.parse(f, &mut value)? => {}
            _ => return usage(format!("unknown option \"{flag}\" for {cmd}")),
        }
    }
    if files.is_empty() {
        return usage("no input files");
    }
    if !export {
        return Ok(Command::Info { files, json: json_flag });
    }
    validate(&a, &files)?;
    Ok(Command::Export { files, args: a })
}

/// `--godot-*` flags that belong to the SpriteFrames export rather than the TileSet export.
const SPRITE_FRAMES_FLAGS: &[&str] =
    &["--godot-tres", "--godot-settings", "--godot-layers", "--godot-tscn", "--godot-overwrite-tscn"];

fn godot_option(a: &mut ExportArgs, flag: &str, value: &mut dyn FnMut(&str) -> Res<String>) -> Res<()> {
    use pixmith_core::gamedev::godot::tileset::{Collision, TerrainMode};
    a.godot_opts = true;
    let o = &mut a.godot;
    match flag {
        "--godot-scene" => o.write_scene = true,
        "--godot-mode" => {
            o.fallback_mode = match value(flag)?.as_str() {
                "blob" => TerrainMode::Blob,
                "corners" => TerrainMode::Corners,
                "sides" => TerrainMode::Sides,
                v => return usage(format!("--godot-mode: unknown mode \"{v}\" (blob, corners or sides)")),
            }
        }
        "--godot-collision" => {
            o.collision = match value(flag)?.as_str() {
                "none" => Collision::None,
                "full" => Collision::Full,
                "opaque" => Collision::Opaque,
                v => return usage(format!("--godot-collision: unknown mode \"{v}\" (none, full or opaque)")),
            }
        }
        "--godot-margin" => o.margin = number(flag, &value(flag)?)?.min(256) as u32,
        "--godot-separation" => o.separation = number(flag, &value(flag)?)?.min(256) as u32,
        "--godot-extrude" => o.extrude = number(flag, &value(flag)?)?.min(64) as u32,
        "--godot-max-texture" => o.max_texture = number(flag, &value(flag)?)?.clamp(16, 16384) as u32,
        "--godot-terrain" => o.fallback_name = value(flag)?,
        "--godot-color" => {
            let v = value(flag)?;
            let parts: Result<Vec<f32>, _> = v.split(',').map(|p| p.trim().parse::<f32>()).collect();
            match parts {
                Ok(p) if p.len() == 3 || p.len() == 4 => {
                    o.fallback_color = [p[0], p[1], p[2], p.get(3).copied().unwrap_or(1.0)];
                }
                _ => return usage("--godot-color: expected r,g,b[,a] with values from 0 to 1"),
            }
        }
        _ => return usage(format!("unknown option \"{flag}\" for export")),
    }
    Ok(())
}

fn validate(a: &ExportArgs, files: &[String]) -> Res<()> {
    let outputs = [&a.png, &a.gif, &a.apng, &a.sheet, &a.png_seq, &a.godot_tileset];
    if outputs.iter().all(|o| o.is_none()) && !a.godot() && a.images.is_empty() {
        return usage("nothing to do: give at least one of --png, --gif, --apng, --webp, --jpg, --bmp, --tga, --ico, --pcx, --flc, --sheet, --png-seq, --godot-tres, --godot-tileset");
    }
    a.image.options()?;
    if a.godot_tileset.is_some() && a.godot_root.is_none() {
        return usage("--godot-tileset needs --godot-root");
    }
    if (a.tileset.is_some() || a.godot_opts) && a.godot_tileset.is_none() {
        return usage("--tileset and the --godot-* tileset options need --godot-tileset");
    }
    if a.godot() && !a.godot_settings && (a.godot_root.is_none() || a.godot_tres.is_none()) {
        return usage("Godot export needs both --godot-root and --godot-tres (or --godot-settings)");
    }
    if !a.godot()
        && (a.extrude
            || a.max_texture.is_some()
            || a.packing.is_some()
            || a.per_animation_png
            || a.name_template.is_some()
            || a.godot_layers.is_some()
            || !a.tag_include.is_empty()
            || !a.tag_exclude.is_empty()
            || a.godot_tscn
            || a.godot_overwrite_tscn)
    {
        return usage("--extrude, --trim, --max-texture, --packing, ... only apply to a Godot export (--godot-tres)");
    }
    match a.packing.as_deref() {
        None | Some("tight" | "grid") => {}
        Some(other) => return usage(format!("unknown packing \"{other}\" (tight or grid)")),
    }
    match a.godot_layers.as_deref() {
        None | Some("visible" | "each") => {}
        Some(other) => return usage(format!("unknown --godot-layers \"{other}\" (visible or each)")),
    }
    if a.json.is_some() && a.sheet.is_none() {
        return usage("--json needs --sheet");
    }
    if a.frame.is_some() && a.png.is_none() && a.images.is_empty() {
        return usage("--frame needs --png (or --jpg, --bmp, --tga, --ico, --pcx, --webp)");
    }
    if (a.layout.is_some()
        || a.columns.is_some()
        || (a.padding.is_some() && !a.godot())
        || a.border.is_some()
        || a.sheet_extrude.is_some())
        && a.sheet.is_none()
    {
        return usage(
            "--layout, --columns, --padding, --border-padding and --sheet-extrude need --sheet (or --godot-tres)",
        );
    }
    if a.split_tags && a.tag.is_some() {
        return usage("--split-tags and --tag cannot be combined");
    }
    if a.no_direction && a.tag.is_none() && !a.split_tags {
        return usage("--no-direction needs --tag or --split-tags");
    }
    if a.trim && a.trim_sprite {
        return usage("--trim and --trim-sprite cannot be combined");
    }
    if let Some(s) = a.scale {
        if !(1..=64).contains(&s) {
            return usage("--scale must be between 1 and 64");
        }
    }
    if a.frame == Some(0) {
        return usage("--frame counts from 1");
    }
    match a.layout.as_deref() {
        None | Some("horizontal" | "vertical" | "grid") => {}
        Some(other) => return usage(format!("unknown layout \"{other}\" (horizontal, vertical or grid)")),
    }
    if a.columns.is_some() && !matches!(a.layout.as_deref(), None | Some("grid")) {
        return usage("--columns only applies to --layout grid");
    }
    if files.len() > 1 {
        for out in [&a.png, &a.gif, &a.apng, &a.sheet, &a.json, &a.png_seq, &a.godot_tres, &a.godot_tileset]
            .into_iter()
            .flatten()
        {
            if !out.contains("{name}") {
                return usage(format!("with several input files, output \"{out}\" must contain {{name}}"));
            }
        }
        let mut seen = HashSet::new();
        for f in files {
            if !seen.insert(stem(f)) {
                return usage(format!("several inputs are named \"{}\"; their outputs would collide", stem(f)));
            }
        }
    }
    Ok(())
}

fn stem(path: &str) -> String {
    Path::new(path).file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned())
}

/// Reads a `.pxmt` or `.aseprite` document, or an image (detected from the content).
fn load(path: &str) -> Result<Sprite, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    raster::read_any(&bytes, path)
}

/// The image format an output name asks for (WebP, JPEG, BMP, TGA, ICO, PCX, FLC), if it is one.
fn image_output(path: &str) -> Option<ImageFormat> {
    ImageFormat::from_path(path).filter(|f| !matches!(f, ImageFormat::Png | ImageFormat::Gif))
}

fn convert(input: &str, output: &str, flags: &ImageFlags) -> Result<(), String> {
    let lower = output.to_ascii_lowercase();
    let image = image_output(output);
    if !(lower.ends_with(".pxmt") || lower.ends_with(".aseprite") || lower.ends_with(".ase") || image.is_some()) {
        return Err(format!(
            "the output \"{output}\" must end in .pxmt, .aseprite, .ase, .webp, .jpg, .bmp, .tga, .ico, .pcx or .flc"
        ));
    }
    if let (Ok(a), Ok(b)) = (std::fs::canonicalize(input), std::fs::canonicalize(output)) {
        if a == b {
            return Err("the input and the output are the same file".into());
        }
    }
    let s = load(input)?;
    match image {
        Some(fmt) => {
            let opts = flags.options().map_err(|e| e.to_string())?;
            write_image(&s, fmt, output, &ExportOptions::default(), 1, 0, &opts, false)
        }
        None => pxmt::save_file(output, &s),
    }
}

/// Writes one WebP, JPEG, BMP, TGA, ICO, PCX or FLC output (several files if `s` is split by layers or tags).
#[allow(clippy::too_many_arguments)]
fn write_image(
    s: &Sprite,
    fmt: ImageFormat,
    path: &str,
    o: &ExportOptions,
    scale: usize,
    frame: usize,
    image: &ImageOptions,
    verbose: bool,
) -> Result<(), String> {
    let format = match fmt {
        ImageFormat::Webp => Format::Webp,
        ImageFormat::Jpeg => Format::Jpeg,
        ImageFormat::Bmp => Format::Bmp,
        ImageFormat::Tga => Format::Tga,
        ImageFormat::Ico => Format::Ico,
        ImageFormat::Pcx => Format::Pcx,
        _ => Format::Flc,
    };
    let enc = EncodeOptions {
        scale,
        frame,
        sheet: Options { scale, ..Options::default() },
        write_json: false,
        image: *image,
    };
    for (file, data) in render_outputs(s, format, o, &enc, path, None)? {
        write(&file, &data, verbose)?;
    }
    Ok(())
}

const DIRECTIONS: [&str; 4] = ["forward", "reverse", "pingpong", "pingpong_reverse"];

fn direction(t: &Tag) -> &'static str {
    DIRECTIONS[(t.direction as usize).min(3)]
}

fn info_text(path: &str, s: &Sprite) -> String {
    let mut out = String::new();
    let total: u32 = s.durations.iter().map(|&d| d as u32).sum();
    writeln!(out, "{path}").unwrap();
    writeln!(out, "  size: {}x{}", s.width, s.height).unwrap();
    writeln!(out, "  frames: {} ({total} ms)", s.frame_count()).unwrap();
    writeln!(out, "  layers: {}", s.layers.len()).unwrap();
    for l in s.layers.iter().rev() {
        let kind = if l.is_group() { " [group]" } else { "" };
        let hidden = if l.visible { "" } else { " (hidden)" };
        writeln!(out, "    {}{}{kind}{hidden}", "  ".repeat(l.child_level as usize), l.name).unwrap();
    }
    writeln!(out, "  tags: {}", s.tags.len()).unwrap();
    for t in &s.tags {
        writeln!(out, "    {}: frames {}-{} {}", t.name, t.from + 1, t.to + 1, direction(t)).unwrap();
    }
    writeln!(out, "  palette: {} colors", s.palette.len()).unwrap();
    out.push_str(&pixmith_core::gamedev::autotile::info_text(s));
    out
}

/// Frame numbers in `tags` are counted from 1, like everywhere else in the CLI.
fn info_json(path: &str, s: &Sprite) -> String {
    let layers: Vec<String> = s
        .layers
        .iter()
        .map(|l| {
            format!(
                "{{\"name\":{},\"visible\":{},\"opacity\":{},\"group\":{},\"level\":{}}}",
                json_str(&l.name),
                l.visible,
                l.opacity,
                l.is_group(),
                l.child_level
            )
        })
        .collect();
    let tags: Vec<String> = s
        .tags
        .iter()
        .map(|t| {
            format!(
                "{{\"name\":{},\"from\":{},\"to\":{},\"direction\":\"{}\",\"repeat\":{}}}",
                json_str(&t.name),
                t.from + 1,
                t.to + 1,
                direction(t),
                t.repeat
            )
        })
        .collect();
    let durations: Vec<String> = s.durations.iter().map(u16::to_string).collect();
    format!(
        "{{\"file\":{},\"width\":{},\"height\":{},\"frames\":{},\"durations\":[{}],\"layers\":[{}],\"tags\":[{}],\"palette\":{}}}",
        json_str(path),
        s.width,
        s.height,
        s.frame_count(),
        durations.join(","),
        layers.join(","),
        tags.join(","),
        s.palette.len()
    )
}

/// Restricts the sprite to the named layers and/or the frames of a tag.
fn filter(mut s: Sprite, layers: &[String], tag: Option<&str>) -> Result<Sprite, String> {
    if !layers.is_empty() {
        let mut keep = vec![false; s.layers.len()];
        for name in layers {
            let found: Vec<usize> = (0..keep.len()).filter(|&i| s.layers[i].name == *name).collect();
            if found.is_empty() {
                return Err(format!("no layer named \"{name}\""));
            }
            for i in found {
                keep[i] = true;
                let level = s.layers[i].child_level;
                if s.layers[i].is_group() {
                    for (j, l) in s.layers.iter().enumerate().skip(i + 1) {
                        if l.child_level <= level {
                            break;
                        }
                        keep[j] = true;
                    }
                }
            }
        }
        for (l, keep) in s.layers.iter_mut().zip(keep) {
            l.visible = l.is_group() || keep;
        }
    }
    if let Some(name) = tag {
        let t = s.tags.iter().find(|t| t.name == name).ok_or(format!("no tag named \"{name}\""))?.clone();
        let (from, to) = (t.from as usize, (t.to as usize).min(s.frame_count() - 1));
        if from > to {
            return Err(format!("tag \"{name}\" is outside the sprite's frames"));
        }
        s.cels = s.cels[from..=to].to_vec();
        s.durations = s.durations[from..=to].to_vec();
        s.tags = vec![Tag { from: 0, to: (to - from) as u16, ..t }];
    }
    Ok(s)
}

fn write(path: &str, data: &[u8], verbose: bool) -> Result<(), String> {
    if let Some(dir) = Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    gif::write_file(path, data)?;
    if verbose {
        println!("wrote {path}");
    }
    Ok(())
}

fn export_godot(
    original: &Sprite,
    s: &Sprite,
    a: &ExportArgs,
    out: &dyn Fn(&Option<String>) -> Option<String>,
) -> Result<(), String> {
    use pixmith_core::gamedev::godot::{self, settings, LayerMode, Options, Packing};
    let mut st = settings::Settings::default();
    if a.godot_settings {
        let json = settings::load(original).ok_or("the document has no saved Godot export settings")?;
        st = settings::Settings::from_json(&json)?;
    }
    if let Some(r) = &a.godot_root {
        st.root = r.clone();
    }
    if let Some(t) = out(&a.godot_tres) {
        st.tres = t;
    }
    if st.root.is_empty() || st.tres.is_empty() {
        return Err("Godot export needs a project root and a .tres path".into());
    }
    let o: &mut Options = &mut st.options;
    if let Some(p) = a.padding {
        o.padding = p as u32;
    }
    o.extrude |= a.extrude;
    o.trim |= a.trim;
    if let Some(m) = a.max_texture {
        o.max_texture = m;
    }
    if let Some(sc) = a.scale {
        o.scale = sc as u32;
    }
    match a.packing.as_deref() {
        Some("grid") => o.packing = Packing::Grid,
        Some(_) => o.packing = Packing::Tight,
        None => {}
    }
    o.per_animation_png |= a.per_animation_png;
    if let Some(t) = &a.name_template {
        o.name_template = t.clone();
    }
    match a.godot_layers.as_deref() {
        Some("each") => o.layers = LayerMode::Each,
        Some(_) => o.layers = LayerMode::Visible,
        None => {}
    }
    o.tags_include.extend(a.tag_include.iter().cloned());
    o.tags_exclude.extend(a.tag_exclude.iter().cloned());
    o.emit_tscn |= a.godot_tscn;
    o.overwrite_tscn |= a.godot_overwrite_tscn;
    let warnings = godot::export_sprite_frames(s, std::path::Path::new(&st.root), std::path::Path::new(&st.tres), o)?;
    for w in warnings {
        eprintln!("pixmith-cli: warning: {w}");
    }
    if a.verbose {
        println!("wrote Godot SpriteFrames {}", st.tres);
    }
    Ok(())
}

fn export_file(input: &str, a: &ExportArgs) -> Result<(), String> {
    let original = load(input)?;
    let s = filter(original.clone(), &a.layers, None)?;
    let name = stem(input);
    let out = |t: &Option<String>| t.as_ref().map(|t| t.replace("{name}", &name));
    let tag = match &a.tag {
        Some(t) => Some(s.tags.iter().position(|x| x.name == *t).ok_or(format!("no tag named \"{t}\""))?),
        None => None,
    };
    let opts = ExportOptions {
        tag,
        bake_direction: !a.no_direction,
        layers: if a.split_layers { LayerMode::Each } else { LayerMode::Visible },
        split_tags: a.split_tags,
        trim: if a.trim {
            Trim::PerFrame
        } else if a.trim_sprite {
            Trim::Uniform
        } else {
            Trim::None
        },
        ignore_empty: a.ignore_empty,
    };
    let scale = a.scale.unwrap_or(1);
    let layout = match (a.layout.as_deref(), a.columns) {
        (Some("vertical"), _) => Layout::Vertical,
        (Some("grid"), Some(c)) | (None, Some(c)) => Layout::Grid(c),
        (Some("grid"), None) => Layout::Grid((s.frame_count() as f64).sqrt().ceil() as usize),
        _ => Layout::Horizontal,
    };
    let enc = EncodeOptions {
        scale,
        frame: a.frame.unwrap_or(1) - 1,
        sheet: Options {
            layout,
            padding: a.padding.unwrap_or(0),
            scale,
            border: a.border,
            extrude: a.sheet_extrude.unwrap_or(0),
        },
        write_json: a.json.is_some(),
        image: Default::default(),
    };
    for (format, path) in [(Format::Png, &a.png), (Format::Gif, &a.gif), (Format::Apng, &a.apng)] {
        if let Some(path) = out(path) {
            let rendered = render_outputs_warn(&s, format, &opts, &enc, &path, None)?;
            for w in &rendered.warnings {
                eprintln!("pixmith-cli: warning: {w}");
            }
            for (file, data) in rendered.files {
                write(&file, &data, a.verbose)?;
            }
        }
    }
    if let Some(path) = out(&a.sheet) {
        for (file, data) in render_outputs(&s, Format::Sheet, &opts, &enc, &path, out(&a.json).as_deref())? {
            write(&file, &data, a.verbose)?;
        }
    }
    if let (Some(root), Some(tres)) = (out(&a.godot_root), out(&a.godot_tileset)) {
        let index = a.tileset.unwrap_or(0);
        let r = pixmith_core::gamedev::godot::tileset::export_sprite_with(
            &filter(original.clone(), &a.layers, a.tag.as_deref())?,
            index,
            Path::new(&root),
            Path::new(&tres),
            &a.godot,
            false,
        )?;
        for w in &r.warnings {
            eprintln!("pixmith-cli: warning: {w}");
        }
        if a.verbose {
            for (path, _) in &r.files {
                println!("wrote {}", path.display());
            }
        }
    }
    if !a.images.is_empty() {
        let image = a.image.options().map_err(|e| e.to_string())?;
        for (fmt, path) in &a.images {
            let path = path.replace("{name}", &name);
            write_image(&s, *fmt, &path, &opts, scale, a.frame.unwrap_or(1) - 1, &image, a.verbose)?;
        }
    }
    if let Some(base) = out(&a.png_seq) {
        for (file, data) in render_outputs(&s, Format::Sequence, &opts, &enc, &base, None)? {
            write(&file, &data, a.verbose)?;
        }
    }
    if a.godot() {
        export_godot(&original, &filter(original.clone(), &a.layers, a.tag.as_deref())?, a, &out)?;
    }
    Ok(())
}

fn run(cmd: Command) -> bool {
    let mut ok = true;
    let mut fail = |file: &str, msg: String| {
        eprintln!("pixmith-cli: {file}: {msg}");
        ok = false;
    };
    match cmd {
        Command::Help => println!("{USAGE}"),
        Command::Info { files, json } => {
            let mut docs = Vec::new();
            for f in &files {
                match load(f) {
                    Ok(s) if json => docs.push(info_json(f, &s)),
                    Ok(s) => print!("{}", info_text(f, &s)),
                    Err(e) => fail(f, e),
                }
            }
            if json {
                match docs.as_slice() {
                    [one] if files.len() == 1 => println!("{one}"),
                    _ => println!("[{}]", docs.join(",")),
                }
            }
        }
        Command::Convert { input, output, image } => {
            if let Err(e) = convert(&input, &output, &image) {
                fail(&input, e);
            }
        }
        Command::Export { files, args } => {
            for f in &files {
                if let Err(e) = export_file(f, &args) {
                    fail(f, e);
                }
            }
        }
    }
    ok
}

fn main() -> ExitCode {
    match parse(std::env::args().skip(1).collect()) {
        Err(msg) => {
            eprintln!("pixmith-cli: {msg}\nTry \"pixmith-cli --help\".");
            ExitCode::from(2)
        }
        Ok(cmd) => {
            if run(cmd) {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
    }
}
