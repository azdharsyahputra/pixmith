//! Imports a sprite sheet PNG together with Aseprite-style JSON metadata (array or hash format):
//! frames from `frames[].frame` and `duration`, tags from `meta.frameTags`, slices from `meta.slices`.

use crate::document::slices::{Rect, Slice, SliceKey, UserData};
use crate::document::{Editor, Sprite, Tag};
use crate::ffi::{path_arg, set_error};
use crate::formats::import::{cel, decode_png, image_palette};
use crate::formats::json::{self, Value};
use std::ffi::c_char;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

struct FrameDef {
    rect: (usize, usize, usize, usize),
    duration: u16,
    /// Size of the untrimmed frame and where the stored pixels sit inside it.
    source: (usize, usize),
    offset: (usize, usize),
}

fn uint(v: &Value, key: &str) -> Option<usize> {
    let n = v.get(key)?.as_f64()?;
    (n >= 0.0 && n.fract() == 0.0 && n <= 1e9).then_some(n as usize)
}

fn int(v: &Value, key: &str) -> Option<i32> {
    let n = v.get(key)?.as_f64()?;
    (n.fract() == 0.0 && n.abs() <= 1e9).then_some(n as i32)
}

fn rect(v: &Value) -> Option<(usize, usize, usize, usize)> {
    Some((uint(v, "x")?, uint(v, "y")?, uint(v, "w")?, uint(v, "h")?))
}

/// The file name of the sheet image recorded in `meta.image`.
pub fn image_name(json_text: &str) -> Option<String> {
    let root = json::parse(json_text).ok()?;
    root.get("meta")?.get("image")?.as_str().map(str::to_string)
}

fn frame_defs(root: &Value) -> Result<Vec<FrameDef>, String> {
    let frames = root.get("frames").ok_or("the JSON has no \"frames\"")?;
    let entries: Vec<&Value> = match frames {
        Value::Arr(a) => a.iter().collect(),
        Value::Obj(m) => m.iter().map(|(_, v)| v).collect(),
        _ => return Err("\"frames\" must be an array or an object".into()),
    };
    if entries.is_empty() {
        return Err("the JSON lists no frames".into());
    }
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let r = e.get("frame").and_then(rect).ok_or(format!("frame {} has no valid \"frame\" rectangle", i + 1))?;
            if e.get("rotated") == Some(&Value::Bool(true)) {
                return Err(format!("frame {} is rotated, which is not supported", i + 1));
            }
            let duration =
                e.get("duration").and_then(Value::as_f64).unwrap_or(100.0).round().clamp(1.0, 65535.0) as u16;
            let source = e.get("sourceSize").and_then(|s| Some((uint(s, "w")?, uint(s, "h")?))).unwrap_or((r.2, r.3));
            let offset = e.get("spriteSourceSize").and_then(|s| Some((uint(s, "x")?, uint(s, "y")?))).unwrap_or((0, 0));
            Ok(FrameDef { rect: r, duration, source, offset })
        })
        .collect()
}

/// An integer `meta.scale` of 2 or more means the sheet was exported upscaled. It is undone when
/// every size and offset is a multiple of it.
fn sheet_scale(root: &Value, defs: &[FrameDef]) -> usize {
    let s = root.get("meta").and_then(|m| m.get("scale")).and_then(Value::as_f64).unwrap_or(1.0);
    if s < 2.0 || s.fract() != 0.0 || s > 64.0 {
        return 1;
    }
    let s = s as usize;
    let ok = defs
        .iter()
        .all(|d| [d.rect.2, d.rect.3, d.source.0, d.source.1, d.offset.0, d.offset.1].iter().all(|v| v % s == 0));
    if ok {
        s
    } else {
        1
    }
}

fn parse_color(text: &str) -> Option<[u8; 4]> {
    let h = text.strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

fn tags(root: &Value, frames: usize) -> Vec<Tag> {
    let Some(list) = root.get("meta").and_then(|m| m.get("frameTags")).and_then(Value::as_array) else { return vec![] };
    list.iter()
        .filter_map(|t| {
            let from = uint(t, "from")?;
            let to = uint(t, "to")?.min(frames - 1);
            if from > to {
                return None;
            }
            let direction = match t.get("direction").and_then(Value::as_str) {
                Some("reverse") => 1,
                Some("pingpong") => 2,
                Some("pingpong_reverse") => 3,
                _ => 0,
            };
            let rgba = t.get("color").and_then(Value::as_str).and_then(parse_color).unwrap_or([0, 0, 0, 255]);
            Some(Tag {
                name: t.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                from: from as u16,
                to: to as u16,
                direction,
                repeat: uint(t, "repeat").unwrap_or(0).min(u16::MAX as usize) as u16,
                color: [rgba[0], rgba[1], rgba[2]],
                user_data: Default::default(),
            })
        })
        .collect()
}

fn slices(root: &Value, frames: usize, scale: usize) -> Vec<Slice> {
    let Some(list) = root.get("meta").and_then(|m| m.get("slices")).and_then(Value::as_array) else { return vec![] };
    let div = |v: i32| (v as f64 / scale as f64).round() as i32;
    let to_rect = |v: &Value| -> Option<Rect> {
        Some(Rect {
            x: div(int(v, "x")?),
            y: div(int(v, "y")?),
            w: div(int(v, "w")?).max(0) as u32,
            h: div(int(v, "h")?).max(0) as u32,
        })
    };
    list.iter()
        .map(|s| {
            let mut keys: Vec<SliceKey> = s
                .get("keys")
                .and_then(Value::as_array)
                .unwrap_or(&[])
                .iter()
                .filter_map(|k| {
                    let frame = uint(k, "frame").unwrap_or(0);
                    let bounds = to_rect(k.get("bounds")?)?;
                    let center = k.get("center").and_then(to_rect).unwrap_or_default();
                    let pivot = k.get("pivot").and_then(|p| Some((div(int(p, "x")?), div(int(p, "y")?))));
                    (frame < frames).then_some(SliceKey {
                        frame: frame as u32,
                        bounds,
                        center,
                        pivot: pivot.unwrap_or((0, 0)),
                    })
                })
                .collect();
            keys.sort_by_key(|k| k.frame);
            keys.dedup_by_key(|k| k.frame);
            let nine_patch =
                s.get("keys").and_then(Value::as_array).is_some_and(|ks| ks.iter().any(|k| k.get("center").is_some()));
            let pivot_frames: Vec<u32> = s
                .get("keys")
                .and_then(Value::as_array)
                .map(|ks| {
                    ks.iter()
                        .filter(|k| k.get("pivot").is_some())
                        .map(|k| uint(k, "frame").unwrap_or(0) as u32)
                        .collect()
                })
                .unwrap_or_default();
            let has_pivot = !pivot_frames.is_empty();
            let pivot_from = pivot_frames.into_iter().min().unwrap_or(0);
            Slice {
                name: s.get("name").and_then(Value::as_str).unwrap_or("Slice").to_string(),
                nine_patch,
                has_pivot,
                pivot_from,
                keys,
                user: UserData {
                    color: s.get("color").and_then(Value::as_str).and_then(parse_color),
                    ..Default::default()
                },
            }
        })
        .filter(|s| !s.keys.is_empty())
        .collect()
}

pub fn sprite_from_sheet_json(png: &[u8], json_text: &str) -> Result<Sprite, String> {
    let root = json::parse(json_text)?;
    let defs = frame_defs(&root)?;
    let (sw, sh, sheet) = decode_png(png)?;
    let scale = sheet_scale(&root, &defs);
    let (cw, ch) = defs.iter().fold((1, 1), |(w, h), d| {
        (w.max(d.source.0.max(d.rect.2 + d.offset.0)), h.max(d.source.1.max(d.rect.3 + d.offset.1)))
    });
    let (cw, ch) = (cw / scale, ch / scale);
    if cw > u16::MAX as usize || ch > u16::MAX as usize {
        return Err("the sprite described by the JSON is too large".into());
    }
    if defs.len() > u16::MAX as usize || (defs.len() as u64).saturating_mul(cw as u64 * ch as u64 * 4) > 1 << 30 {
        return Err("the JSON describes too many frames".into());
    }
    let mut s = Sprite::new(cw as u32, ch as u32);
    s.durations.clear();
    s.cels.clear();
    for (i, d) in defs.iter().enumerate() {
        let (x, y, w, h) = d.rect;
        if w == 0 || h == 0 || x + w > sw || y + h > sh {
            return Err(format!("frame {} ({x},{y} {w}x{h}) lies outside the {sw}x{sh} image", i + 1));
        }
        let mut px = vec![0u8; cw * ch * 4];
        let (ox, oy) = (d.offset.0 / scale, d.offset.1 / scale);
        for j in 0..h / scale {
            for k in 0..w / scale {
                let (dx, dy) = (ox + k, oy + j);
                if dx < cw && dy < ch {
                    let from = ((y + j * scale) * sw + x + k * scale) * 4;
                    px[(dy * cw + dx) * 4..][..4].copy_from_slice(&sheet[from..from + 4]);
                }
            }
        }
        s.durations.push(d.duration);
        s.cels.push(vec![cel(px)]);
    }
    let all: Vec<&[u8]> = s.cels.iter().filter_map(|r| r[0].as_ref()).map(|c| c.pixels.as_slice()).collect();
    s.palette = image_palette(&all).unwrap_or(s.palette);
    s.tags = tags(&root, defs.len());
    s.extra.slices = slices(&root, defs.len(), scale);
    Ok(s)
}

/// Where the sheet image for `json_path` is expected: `meta.image` next to the JSON, else the
/// JSON's name with a `.png` extension.
fn image_path(json_path: &Path, json_text: &str) -> PathBuf {
    let dir = json_path.parent().unwrap_or(Path::new("."));
    match image_name(json_text).and_then(|n| Path::new(&n).file_name().map(PathBuf::from)) {
        Some(name) => dir.join(name),
        None => json_path.with_extension("png"),
    }
}

/// Opens a sprite sheet described by Aseprite JSON as a new sprite. `png_path` may be null to use
/// the image named in the JSON. Returns null on failure.
#[no_mangle]
pub unsafe extern "C" fn pixmith_import_sheet_json(json_path: *const c_char, png_path: *const c_char) -> *mut Editor {
    crate::ffi::guarded("pixmith_import_sheet_json", std::ptr::null_mut(), || {
        let result = (|| {
            let json_path = path_arg(json_path).ok_or("invalid path")?;
            let text = std::fs::read_to_string(&json_path).map_err(|e| format!("{json_path}: {e}"))?;
            let png_path =
                path_arg(png_path).map(PathBuf::from).unwrap_or_else(|| image_path(Path::new(&json_path), &text));
            let png = std::fs::read(&png_path).map_err(|e| format!("sheet image {}: {e}", png_path.display()))?;
            sprite_from_sheet_json(&png, &text).map_err(|e| format!("{json_path}: {e}"))
        })();
        match result {
            Ok(s) => Box::into_raw(Box::new(Editor::new(s))),
            Err(e) => {
                set_error(e);
                null_mut()
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::export::{encode_png, frame_png};
    use crate::formats::sheet::{export, Layout, Options};

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn sample() -> Sprite {
        let mut s = Sprite::new(3, 2);
        s.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        s.add_frame(0, true);
        s.draw_line(0, 1, 2, 1, 2, 1, BLUE, 1);
        s.add_frame(1, true);
        s.durations = vec![100, 250, 40];
        s.tags.push(Tag {
            name: "go".into(),
            from: 1,
            to: 2,
            direction: 3,
            repeat: 2,
            color: [1, 2, 3],
            user_data: Default::default(),
        });
        s
    }

    #[test]
    fn round_trips_our_own_export() {
        let s = sample();
        for (layout, pad) in [(Layout::Horizontal, 0), (Layout::Grid(2), 1), (Layout::Vertical, 2)] {
            let (png, json) =
                export(&s, Options { layout, padding: pad, scale: 1, ..Default::default() }, "s.png").unwrap();
            let back = sprite_from_sheet_json(&png, &json).unwrap();
            assert_eq!((back.width, back.height, back.frame_count()), (3, 2, 3), "{layout:?}");
            for f in 0..3 {
                assert_eq!(back.render(f), s.render(f), "{layout:?} frame {f}");
            }
            assert_eq!(back.durations, s.durations);
            assert_eq!(back.tags.len(), 1);
            let (a, b) = (&back.tags[0], &s.tags[0]);
            assert_eq!((a.name.as_str(), a.from, a.to, a.direction, a.repeat, a.color), ("go", 1, 2, 3, 2, b.color));
        }
    }

    #[test]
    fn undoes_an_export_scale() {
        let s = sample();
        let (png, json) =
            export(&s, Options { layout: Layout::Horizontal, padding: 1, scale: 3, ..Default::default() }, "s.png")
                .unwrap();
        let back = sprite_from_sheet_json(&png, &json).unwrap();
        assert_eq!((back.width, back.height), (3, 2));
        for f in 0..3 {
            assert_eq!(back.render(f), s.render(f));
        }
    }

    #[test]
    fn reads_hash_format_with_trimmed_frames() {
        let img = frame_png(&sample(), 1, 1).unwrap(); // 3x2 sheet; frame 2 has blue at (2,1)
        let json = r##"{
            "frames": {
                "a 0.ase": { "frame": {"x":0,"y":0,"w":2,"h":2}, "rotated": false, "trimmed": true,
                    "spriteSourceSize": {"x":1,"y":0,"w":2,"h":2}, "sourceSize": {"w":4,"h":3}, "duration": 70 },
                "a 1.ase": { "frame": {"x":1,"y":0,"w":2,"h":2}, "sourceSize": {"w":4,"h":3} }
            },
            "meta": { "image": "x.png", "scale": "1", "frameTags": [ {"name":"t","from":0,"to":9,"direction":"reverse","color":"#ff000080"} ] }
        }"##;
        let s = sprite_from_sheet_json(&img, json).unwrap();
        assert_eq!((s.width, s.height, s.frame_count()), (4, 3, 2));
        assert_eq!(s.durations, vec![70, 100]);
        let f0 = s.render(0);
        assert_eq!(&f0[(0 * 4 + 1) * 4..][..4], &RED); // pixel (0,0) of the cell lands at (1,0)
        assert_eq!(&f0[0..4], &[0; 4]);
        assert_eq!((s.tags[0].to, s.tags[0].direction, s.tags[0].color), (1, 1, [255, 0, 0]));
    }

    #[test]
    fn reads_slices() {
        let img = frame_png(&Sprite::new(4, 4), 0, 1).unwrap();
        let json = r##"{ "frames": [ {"frame":{"x":0,"y":0,"w":4,"h":4},"duration":100} ],
            "meta": { "slices": [
                { "name": "hit", "color": "#0000ffff", "keys": [ {"frame":0,"bounds":{"x":1,"y":1,"w":2,"h":3}} ] },
                { "name": "ui", "keys": [ {"frame":0,"bounds":{"x":0,"y":0,"w":4,"h":4},"center":{"x":1,"y":1,"w":2,"h":2},"pivot":{"x":2,"y":3}} ] },
                { "name": "none", "keys": [] }
            ] } }"##;
        let s = sprite_from_sheet_json(&img, json).unwrap();
        assert_eq!(s.extra.slices.len(), 2);
        let hit = &s.extra.slices[0];
        assert_eq!(hit.name, "hit");
        assert_eq!(hit.user.color, Some([0, 0, 255, 255]));
        assert_eq!(hit.keys[0].bounds, Rect { x: 1, y: 1, w: 2, h: 3 });
        let ui = &s.extra.slices[1];
        assert!(ui.nine_patch && ui.has_pivot);
        assert_eq!((ui.keys[0].center.w, ui.keys[0].pivot), (2, (2, 3)));
    }

    #[test]
    fn reports_bad_input() {
        let img = encode_png(&[0; 16], 2, 2).unwrap();
        let ok = r#"{"frames":[{"frame":{"x":0,"y":0,"w":2,"h":2}}]}"#;
        assert!(sprite_from_sheet_json(&img, ok).is_ok());
        let cases = [
            ("{", "invalid JSON"),
            (r#"{"meta":{}}"#, "no \"frames\""),
            (r#"{"frames":[]}"#, "no frames"),
            (r#"{"frames":[{"duration":5}]}"#, "frame 1 has no valid"),
            (r#"{"frames":[{"frame":{"x":1,"y":0,"w":2,"h":2}}]}"#, "outside"),
            (r#"{"frames":[{"frame":{"x":0,"y":0,"w":2,"h":2},"rotated":true}]}"#, "rotated"),
        ];
        for (json, needle) in cases {
            let e = sprite_from_sheet_json(&img, json).err().unwrap_or_default();
            assert!(e.contains(needle), "{json}: {e}");
        }
        assert!(sprite_from_sheet_json(b"not a png", ok).is_err());
    }

    #[test]
    fn finds_the_sheet_image() {
        assert_eq!(image_name(r#"{"meta":{"image":"dir/walk.png"}}"#).as_deref(), Some("dir/walk.png"));
        assert_eq!(
            image_path(Path::new("/a/b/s.json"), r#"{"meta":{"image":"../x/walk.png"}}"#),
            PathBuf::from("/a/b/walk.png")
        );
        assert_eq!(image_path(Path::new("/a/b/s.json"), "{}"), PathBuf::from("/a/b/s.png"));
    }
}
