//! Godot 4 SpriteFrames (`.tres`) generation: animations, atlas packing and text output.

use crate::document::Sprite;
use crate::formats::export::{encode_png, upscale};
use std::collections::{HashMap, HashSet};

pub const DEFAULT_MAX_TEXTURE: u32 = 4096;
pub const HARD_MAX_TEXTURE: u32 = 16384;
pub const MAX_SCALE: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Packing {
    /// Shelf packing of the (trimmed, deduplicated) frames.
    Tight,
    /// Uniform cells in rows and columns.
    Grid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerMode {
    /// Whatever is visible in the sprite.
    Visible,
    /// Only these layers (and the children of selected groups), shown even if hidden.
    Selected(Vec<String>),
    /// One SpriteFrames file per visible layer.
    Each,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub padding: u32,
    /// Copy the edge pixels of every frame 1px into the padding area.
    pub extrude: bool,
    pub trim: bool,
    pub max_texture: u32,
    /// Integer upscale of every frame (nearest neighbour).
    pub scale: u32,
    pub packing: Packing,
    /// One PNG (set of pages) per animation instead of one shared atlas.
    pub per_animation_png: bool,
    /// File stem template: `{name}` is the `.tres` stem, `{layer}` the layer name.
    pub name_template: String,
    pub layers: LayerMode,
    /// Only these tags (empty: all).
    pub tags_include: Vec<String>,
    pub tags_exclude: Vec<String>,
    /// Also write a ready-to-use `.tscn` with an AnimatedSprite2D.
    pub emit_tscn: bool,
    pub overwrite_tscn: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            padding: 0,
            extrude: false,
            trim: false,
            max_texture: DEFAULT_MAX_TEXTURE,
            scale: 1,
            packing: Packing::Tight,
            per_animation_png: false,
            name_template: "{name}".into(),
            layers: LayerMode::Visible,
            tags_include: Vec::new(),
            tags_exclude: Vec::new(),
            emit_tscn: false,
            overwrite_tscn: false,
        }
    }
}

/// Where the `.tres` should point, plus ids to keep from an earlier export.
pub struct Target<'a> {
    /// File name of the sprite without extension (`player`).
    pub name: &'a str,
    /// Godot directory of the output, e.g. `res://art` (no trailing slash).
    pub res_dir: &'a str,
    /// Previous `.tres` text, if any, to read uids from.
    pub previous: Option<&'a str>,
}

pub struct Built {
    /// `(file name, png bytes)` per atlas page.
    pub pages: Vec<(String, Vec<u8>)>,
    pub tres: String,
    pub warnings: Vec<String>,
    /// Name of the first animation (the default one in a scene).
    pub first_animation: String,
}

pub fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Plain playback order of `from..=to` (inclusive) for an Aseprite direction.
/// 0 forward, 1 reverse, 2 ping-pong, 3 ping-pong reverse. Ends are never doubled.
pub fn expand_frames(from: usize, to: usize, direction: u8) -> Vec<usize> {
    let fwd: Vec<usize> = (from..=to).collect();
    let rev: Vec<usize> = fwd.iter().rev().copied().collect();
    let bounce = |v: &[usize]| {
        let mut out = v.to_vec();
        if v.len() > 2 {
            out.extend(v[1..v.len() - 1].iter().rev());
        }
        out
    };
    match direction {
        1 => rev,
        2 => bounce(&fwd),
        3 => bounce(&rev),
        _ => fwd,
    }
}

/// Deterministic float text: `1.0`, `2.0`, `12.5`.
pub fn fmt_float(x: f64) -> String {
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("NaN") {
        s
    } else {
        format!("{s}.0")
    }
}

/// `(speed, durations)` for frames with the given millisecond durations.
/// `speed = 1000 / gcd(ms)` and `duration = ms / gcd`, so `duration / speed` is the frame time in seconds.
pub fn speed_and_durations(ms: &[u32]) -> (f64, Vec<f64>) {
    let ms: Vec<u32> = ms.iter().map(|&m| m.max(1)).collect();
    let g = ms.iter().copied().fold(0, gcd).max(1);
    (1000.0 / g as f64, ms.iter().map(|&m| (m / g) as f64).collect())
}

/// Identifier-safe text (ASCII letters, digits and underscore).
pub fn sanitize(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if t.is_empty() {
        "_".into()
    } else {
        t
    }
}

/// Godot string literal: backslash, quote and control characters are escaped.
pub fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push('_'),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

struct Anim {
    name: String,
    key: String,
    looped: bool,
    /// Frame indices in playback order.
    seq: Vec<usize>,
    /// First frame of the tag; sub-resource index is `frame - base`.
    base: usize,
}

fn animations(s: &Sprite, o: &Options, warnings: &mut Vec<String>) -> Result<Vec<Anim>, String> {
    let n = s.frame_count();
    let mut anims: Vec<Anim> = Vec::new();
    let (mut names, mut keys) = (HashSet::new(), HashSet::new());
    for t in &s.tags {
        if (!o.tags_include.is_empty() && !o.tags_include.contains(&t.name)) || o.tags_exclude.contains(&t.name) {
            continue;
        }
        let (from, to) = (t.from as usize, (t.to as usize).min(n.saturating_sub(1)));
        if from >= n || from > to {
            warnings.push(format!("tag \"{}\" is outside the sprite's frames and was skipped", t.name));
            continue;
        }
        let mut name = t.name.clone();
        let mut i = 2;
        while !names.insert(name.clone()) {
            name = format!("{}_{i}", t.name);
            i += 1;
        }
        let mut key = sanitize(&name);
        let mut i = 2;
        while !keys.insert(key.clone()) {
            key = format!("{}_{i}", sanitize(&name));
            i += 1;
        }
        anims.push(Anim { name, key, looped: t.repeat == 0, seq: expand_frames(from, to, t.direction), base: from });
    }
    if s.tags.is_empty() {
        anims.push(Anim {
            name: "default".into(),
            key: "default".into(),
            looped: true,
            seq: (0..n).collect(),
            base: 0,
        });
    }
    if anims.is_empty() {
        return Err("no tag is selected for export".into());
    }
    Ok(anims)
}

struct Image {
    w: usize,
    h: usize,
    rgba: Vec<u8>,
}

/// A frame's atlas image and the offset of its trimmed box in the full canvas.
struct Cropped {
    img: usize,
    ox: usize,
    oy: usize,
}

fn crop(full: &[u8], w: usize, h: usize, trim: bool) -> (Image, usize, usize) {
    if !trim {
        return (Image { w, h, rgba: full.to_vec() }, 0, 0);
    }
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if full[(y * w + x) * 4 + 3] != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 == 0 {
        // Fully transparent: keep a 1x1 placeholder so the region is valid.
        return (Image { w: 1, h: 1, rgba: vec![0; 4] }, 0, 0);
    }
    let (cw, ch) = (x1 - x0, y1 - y0);
    let mut rgba = Vec::with_capacity(cw * ch * 4);
    for y in y0..y1 {
        rgba.extend_from_slice(&full[(y * w + x0) * 4..(y * w + x1) * 4]);
    }
    (Image { w: cw, h: ch, rgba }, x0, y0)
}

struct Placed {
    page: usize,
    x: usize,
    y: usize,
}

struct Page {
    w: usize,
    h: usize,
    rgba: Vec<u8>,
}

fn blit(page: &mut Page, img: &Image, x: usize, y: usize, extrude: bool) {
    for row in 0..img.h {
        let d = ((y + row) * page.w + x) * 4;
        page.rgba[d..d + img.w * 4].copy_from_slice(&img.rgba[row * img.w * 4..(row + 1) * img.w * 4]);
    }
    if !extrude {
        return;
    }
    // Border pixels (corners included) copy the nearest edge pixel.
    let (x, y) = (x as isize, y as isize);
    for dy in -1..=img.h as isize {
        for dx in -1..=img.w as isize {
            if dx >= 0 && dx < img.w as isize && dy >= 0 && dy < img.h as isize {
                continue;
            }
            let sx = dx.clamp(0, img.w as isize - 1) as usize;
            let sy = dy.clamp(0, img.h as isize - 1) as usize;
            let s = (sy * img.w + sx) * 4;
            let d = (((y + dy) as usize) * page.w + (x + dx) as usize) * 4;
            page.rgba[d..d + 4].copy_from_slice(&img.rgba[s..s + 4]);
        }
    }
}

struct Shelf {
    used_w: usize,
    used_h: usize,
    x: usize,
    y: usize,
    h: usize,
}

fn check_fit(images: &[Image], o: &Options, max: usize) -> Result<(), String> {
    let (pad, e) = (o.padding as usize, o.extrude as usize);
    for i in images {
        if i.w + 2 * e + 2 * pad > max || i.h + 2 * e + 2 * pad > max {
            return Err(format!("a {}x{} frame does not fit in a {max}px texture", i.w, i.h));
        }
    }
    Ok(())
}

/// Shelf-packs `images` into pages no larger than `max` in either direction.
fn pack_tight(images: &[Image], o: &Options, max: usize) -> Result<(Vec<Page>, Vec<Placed>), String> {
    check_fit(images, o, max)?;
    let pad = o.padding as usize;
    let e = o.extrude as usize;
    let cell = |i: &Image| (i.w + 2 * e, i.h + 2 * e);
    let area: usize = images.iter().map(|i| (cell(i).0 + pad) * (cell(i).1 + pad)).sum();
    let widest = images.iter().map(|i| cell(i).0 + 2 * pad).max().unwrap_or(1);
    let limit = ((area as f64).sqrt().ceil() as usize).max(widest).next_power_of_two().min(max);
    let mut order: Vec<usize> = (0..images.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(images[i].h), i));

    let mut shelves: Vec<Shelf> = Vec::new();
    let mut placed: Vec<Option<Placed>> = (0..images.len()).map(|_| None).collect();
    for &i in &order {
        let (cw, ch) = cell(&images[i]);
        let mut done = false;
        for (pi, l) in shelves.iter_mut().enumerate() {
            let (mut x, mut y, mut sh) = (l.x, l.y, l.h);
            if x + cw + pad > limit {
                x = pad;
                y += sh + pad;
                sh = 0;
            }
            if y + ch + pad > max {
                continue;
            }
            placed[i] = Some(Placed { page: pi, x: x + e, y: y + e });
            l.x = x + cw + pad;
            l.y = y;
            l.h = sh.max(ch);
            l.used_w = l.used_w.max(x + cw + pad);
            l.used_h = l.used_h.max(y + ch + pad);
            done = true;
            break;
        }
        if !done {
            placed[i] = Some(Placed { page: shelves.len(), x: pad + e, y: pad + e });
            shelves.push(Shelf { used_w: cw + 2 * pad, used_h: ch + 2 * pad, x: pad + cw + pad, y: pad, h: ch });
        }
    }
    let dims: Vec<(usize, usize)> = shelves.iter().map(|l| (l.used_w, l.used_h)).collect();
    Ok(finish(images, o, dims, placed.into_iter().map(Option::unwrap).collect()))
}

/// Uniform cells (as large as the largest frame) in rows and columns.
fn pack_grid(images: &[Image], o: &Options, max: usize) -> Result<(Vec<Page>, Vec<Placed>), String> {
    check_fit(images, o, max)?;
    let (pad, e) = (o.padding as usize, o.extrude as usize);
    let cw = images.iter().map(|i| i.w).max().unwrap_or(1) + 2 * e;
    let ch = images.iter().map(|i| i.h).max().unwrap_or(1) + 2 * e;
    let (px, py) = (cw + pad, ch + pad);
    let (cols_fit, rows_fit) = (((max - pad) / px).max(1), ((max - pad) / py).max(1));
    let n = images.len();
    let cols = cols_fit.min((n as f64).sqrt().ceil() as usize).max(1);
    let per_page = cols * rows_fit;
    let mut dims = Vec::new();
    let mut placed = Vec::new();
    for i in 0..n {
        let (page, k) = (i / per_page, i % per_page);
        placed.push(Placed { page, x: pad + (k % cols) * px + e, y: pad + (k / cols) * py + e });
    }
    for page in 0..n.div_ceil(per_page) {
        let count = (n - page * per_page).min(per_page);
        dims.push((pad + cols.min(count) * px, pad + count.div_ceil(cols) * py));
    }
    Ok(finish(images, o, dims, placed))
}

fn finish(images: &[Image], o: &Options, dims: Vec<(usize, usize)>, placed: Vec<Placed>) -> (Vec<Page>, Vec<Placed>) {
    let mut pages: Vec<Page> = dims.iter().map(|&(w, h)| Page { w, h, rgba: vec![0; w * h * 4] }).collect();
    for (img, p) in images.iter().zip(&placed) {
        blit(&mut pages[p.page], img, p.x, p.y, o.extrude);
    }
    (pages, placed)
}

/// A set of frames packed into one or more atlas pages.
struct Group {
    key: Option<String>,
    images: Vec<Image>,
    info: HashMap<usize, Cropped>,
    placed: Vec<Placed>,
    pages: Vec<Page>,
    /// 1-based id of this group's first page among all ext_resources.
    first_ext: usize,
}

fn make_group(s: &Sprite, frames: &[usize], key: Option<String>, o: &Options, max: usize) -> Result<Group, String> {
    let scale = o.scale.clamp(1, MAX_SCALE) as usize;
    let (w, h) = (s.width as usize * scale, s.height as usize * scale);
    let mut frames = frames.to_vec();
    frames.sort_unstable();
    frames.dedup();
    let mut images: Vec<Image> = Vec::new();
    let mut by_content: HashMap<(usize, usize, Vec<u8>), usize> = HashMap::new();
    let mut info = HashMap::new();
    for &f in &frames {
        let full = s.render(f);
        let full = if scale > 1 { upscale(&full, s.width as usize, s.height as usize, scale) } else { full };
        let (img, ox, oy) = crop(&full, w, h, o.trim);
        // Identical images share one region.
        let idx = *by_content.entry((img.w, img.h, img.rgba.clone())).or_insert_with(|| {
            images.push(img);
            images.len() - 1
        });
        info.insert(f, Cropped { img: idx, ox, oy });
    }
    let (pages, placed) = match o.packing {
        Packing::Tight => pack_tight(&images, o, max)?,
        Packing::Grid => pack_grid(&images, o, max)?,
    };
    Ok(Group { key, images, info, placed, pages, first_ext: 0 })
}

fn read_uid(line: &str) -> Option<String> {
    let i = line.find("uid=\"")? + 5;
    let j = line[i..].find('"')?;
    let uid = &line[i..i + j];
    uid.starts_with("uid://").then(|| uid.to_string())
}

/// uid of the `[gd_resource ...]` / `[gd_scene ...]` header.
pub fn header_uid(text: &str) -> Option<String> {
    read_uid(text.lines().find(|l| l.starts_with("[gd_resource") || l.starts_with("[gd_scene"))?)
}

/// uid of the `[ext_resource ...]` line that points at `path`.
pub fn ext_uid(text: &str, path: &str) -> Option<String> {
    let needle = format!("path={}", quote(path));
    read_uid(text.lines().find(|l| l.starts_with("[ext_resource") && l.contains(&needle))?)
}

fn page_file(stem: &str, g: &Group, page: usize) -> String {
    let mut n = stem.to_string();
    if let Some(k) = &g.key {
        n.push('_');
        n.push_str(k);
    }
    if g.pages.len() > 1 {
        n.push_str(&format!("_{page}"));
    }
    n.push_str(".png");
    n
}

/// The atlas groups of an export: options as clamped, warnings so far, the animations, their groups and which group
/// each animation uses. Pages are numbered across groups by `first_ext` (1-based).
fn pack(s: &Sprite, o: &Options) -> Result<(Options, Vec<String>, Vec<Anim>, Vec<Group>, Vec<usize>), String> {
    let mut warnings = Vec::new();
    let mut o = o.clone();
    o.scale = o.scale.clamp(1, MAX_SCALE);
    if o.max_texture == 0 {
        o.max_texture = DEFAULT_MAX_TEXTURE;
    }
    if o.max_texture > HARD_MAX_TEXTURE {
        warnings.push(format!("max texture size was limited to {HARD_MAX_TEXTURE}px"));
        o.max_texture = HARD_MAX_TEXTURE;
    } else if o.max_texture > DEFAULT_MAX_TEXTURE {
        warnings.push(format!(
            "textures above {DEFAULT_MAX_TEXTURE}px may fail to load on mobile GPUs; {DEFAULT_MAX_TEXTURE}px is the safe size"
        ));
    }
    if s.frame_count() == 0 {
        return Err("the sprite has no frames".into());
    }
    let anims = animations(s, &o, &mut warnings)?;
    let max = o.max_texture as usize;

    // One shared atlas, or one per animation.
    let mut groups: Vec<Group> = Vec::new();
    let mut group_of: Vec<usize> = Vec::new();
    if o.per_animation_png {
        for (i, a) in anims.iter().enumerate() {
            groups.push(make_group(s, &a.seq, Some(a.key.clone()), &o, max)?);
            group_of.push(i);
        }
    } else {
        let all: Vec<usize> = anims.iter().flat_map(|a| a.seq.iter().copied()).collect();
        groups.push(make_group(s, &all, None, &o, max)?);
        group_of = vec![0; anims.len()];
    }
    let mut next = 1;
    for g in &mut groups {
        g.first_ext = next;
        next += g.pages.len();
    }
    for g in &groups {
        if g.pages.len() > 1 {
            warnings.push(format!(
                "the atlas{} was split into {} pages of at most {}px",
                g.key.as_ref().map(|k| format!(" of \"{k}\"")).unwrap_or_default(),
                g.pages.len(),
                o.max_texture
            ));
        }
    }
    Ok((o, warnings, anims, groups, group_of))
}

pub fn build(s: &Sprite, target: &Target, o: &Options) -> Result<Built, String> {
    let (o, mut warnings, anims, groups, group_of) = pack(s, o)?;

    let mut png_pages = Vec::new();
    for g in &groups {
        for (i, p) in g.pages.iter().enumerate() {
            png_pages.push((page_file(target.name, g, i), encode_png(&p.rgba, p.w, p.h)?));
        }
    }

    let (w, h) = (s.width as usize * o.scale as usize, s.height as usize * o.scale as usize);
    let prev = target.previous;
    let mut t = String::new();
    let uid = prev.and_then(header_uid).map(|u| format!(" uid=\"{u}\"")).unwrap_or_default();
    t.push_str(&format!("[gd_resource type=\"SpriteFrames\" format=3{uid}]\n\n"));
    for g in &groups {
        for i in 0..g.pages.len() {
            let path = format!("{}/{}", target.res_dir, page_file(target.name, g, i));
            let uid = prev.and_then(|p| ext_uid(p, &path)).map(|u| format!(" uid=\"{u}\"")).unwrap_or_default();
            t.push_str(&format!(
                "[ext_resource type=\"Texture2D\"{uid} path={} id=\"{}_tex\"]\n",
                quote(&path),
                g.first_ext + i
            ));
        }
    }
    t.push('\n');
    for (ai, a) in anims.iter().enumerate() {
        let g = &groups[group_of[ai]];
        let mut frames: Vec<usize> = a.seq.clone();
        frames.sort_unstable();
        frames.dedup();
        for f in frames {
            let c = &g.info[&f];
            let (img, p) = (&g.images[c.img], &g.placed[c.img]);
            t.push_str(&format!(
                "[sub_resource type=\"AtlasTexture\" id=\"at_{}_{}\"]\natlas = ExtResource(\"{}_tex\")\nregion = Rect2({}, {}, {}, {})\n",
                a.key,
                f - a.base,
                g.first_ext + p.page,
                p.x,
                p.y,
                img.w,
                img.h
            ));
            if o.trim && (c.ox, c.oy, img.w, img.h) != (0, 0, w, h) {
                t.push_str(&format!("margin = Rect2({}, {}, {}, {})\n", c.ox, c.oy, w - img.w, h - img.h));
            }
            t.push('\n');
        }
    }
    t.push_str("[resource]\nanimations = [");
    for (ai, a) in anims.iter().enumerate() {
        let ms: Vec<u32> = a.seq.iter().map(|&f| s.durations[f] as u32).collect();
        let (speed, durs) = speed_and_durations(&ms);
        t.push_str(if ai == 0 { "{\n" } else { ", {\n" });
        t.push_str("\"frames\": [");
        for (i, (&f, d)) in a.seq.iter().zip(&durs).enumerate() {
            // Same layout as Godot's own text saver, so re-saving in the editor changes nothing here.
            t.push_str(if i == 0 { "{\n" } else { ", {\n" });
            t.push_str(&format!(
                "\"duration\": {},\n\"texture\": SubResource(\"at_{}_{}\")\n}}",
                fmt_float(*d),
                a.key,
                f - a.base
            ));
        }
        t.push_str("],\n");
        t.push_str(&format!(
            "\"loop\": {},\n\"name\": &{},\n\"speed\": {}\n}}",
            a.looped,
            quote(&a.name),
            fmt_float(speed)
        ));
    }
    t.push_str("]\n");

    warnings.insert(0, NEAREST_WARNING.into());
    Ok(Built { pages: png_pages, tres: t, warnings, first_animation: anims[0].name.clone() })
}

pub const NEAREST_WARNING: &str = "Godot defaults to Linear texture filtering, which blurs pixel art. Set Project Settings > Rendering > Textures > Default Texture Filter = Nearest (or texture_filter on the node).";

/// One frame of a Live Link animation: the page it is on, its region there and the AtlasTexture margin (trimmed
/// frames), the same numbers the exported `.tres` holds.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkFrame {
    pub page: usize,
    pub region: (usize, usize, usize, usize),
    pub margin: Option<(usize, usize, usize, usize)>,
    /// Relative duration, as in the `.tres` (`speed` frames per second at 1.0).
    pub duration: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinkAnimation {
    pub name: String,
    pub looped: bool,
    pub speed: f64,
    pub frames: Vec<LinkFrame>,
}

/// What a running game needs to swap in the sprite: the atlas pages (PNG) and every animation's frames, laid out
/// exactly like `build` lays out the export (same options, so the same regions).
#[derive(Clone, Debug)]
pub struct Linked {
    /// `(width, height, png)` per page, in `.tres` ext_resource order.
    pub pages: Vec<(usize, usize, Vec<u8>)>,
    pub animations: Vec<LinkAnimation>,
    pub warnings: Vec<String>,
}

pub fn link(s: &Sprite, o: &Options) -> Result<Linked, String> {
    let (o, warnings, anims, groups, group_of) = pack(s, o)?;
    let mut pages = Vec::new();
    for g in &groups {
        for p in &g.pages {
            pages.push((p.w, p.h, encode_png(&p.rgba, p.w, p.h)?));
        }
    }
    let (w, h) = (s.width as usize * o.scale as usize, s.height as usize * o.scale as usize);
    let animations = anims
        .iter()
        .enumerate()
        .map(|(ai, a)| {
            let g = &groups[group_of[ai]];
            let ms: Vec<u32> = a.seq.iter().map(|&f| s.durations[f] as u32).collect();
            let (speed, durs) = speed_and_durations(&ms);
            let frames = a
                .seq
                .iter()
                .zip(durs)
                .map(|(f, duration)| {
                    let c = &g.info[f];
                    let (img, p) = (&g.images[c.img], &g.placed[c.img]);
                    let margin = (o.trim && (c.ox, c.oy, img.w, img.h) != (0, 0, w, h)).then_some((
                        c.ox,
                        c.oy,
                        w - img.w,
                        h - img.h,
                    ));
                    LinkFrame { page: g.first_ext - 1 + p.page, region: (p.x, p.y, img.w, img.h), margin, duration }
                })
                .collect();
            LinkAnimation { name: a.name.clone(), looped: a.looped, speed, frames }
        })
        .collect();
    Ok(Linked { pages, animations, warnings })
}
