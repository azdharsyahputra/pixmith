//! Live Link: the atlas and frame layout of the document, as the running Godot game needs it to swap the sprite in
//! (`godot.atlas` in docs/LIVE_PROTOCOL.md). It follows the document's saved Godot export settings, so the regions
//! are the ones the exported `.tres` holds.

use super::settings::{self, Settings};
use super::sprite_frames;
use super::{resolve_paths, variants};
use crate::document::Sprite;
use crate::formats::sheet::json_str;
use std::path::Path;

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

fn num(x: f64) -> String {
    sprite_frames::fmt_float(x)
}

fn rect(r: (usize, usize, usize, usize)) -> String {
    format!("[{},{},{},{}]", r.0, r.1, r.2, r.3)
}

/// The Live Link JSON for `s`:
///
/// ```json
/// {"linked": true, "scale": 1, "warnings": [...],
///  "resources": [{"resource": "res://chars/hero.tres", "pages": [{"w", "h", "png": "<base64>"}],
///                 "animations": [{"name", "loop", "speed", "frames": [{"page", "region": [x,y,w,h],
///                                 "margin": [x,y,w,h] or null, "duration"}]}]}]}
/// ```
///
/// `linked` is false when the document has no Godot export settings yet (then `resource` is null and the layout
/// uses the default options); a game can only match a resource it knows by its `res://` path.
pub fn json(s: &Sprite) -> Result<String, String> {
    let saved = settings::load(s).and_then(|j| Settings::from_json(&j).ok());
    let o = saved.as_ref().map(|st| st.options.clone()).unwrap_or_default();
    // The res:// directory and file stem of the export, when the saved paths still point into a Godot project.
    let place = saved.as_ref().and_then(|st| {
        let (tres, res_dir) = resolve_paths(Path::new(&st.root), Path::new(&st.tres)).ok()?;
        Some((res_dir, tres.file_stem()?.to_str()?.to_string()))
    });
    let name = place.as_ref().map_or("sprite", |p| p.1.as_str());
    let mut warnings: Vec<String> = Vec::new();
    let mut resources = Vec::new();
    for (stem, sprite) in variants(s, name, &o)? {
        let linked = sprite_frames::link(&sprite, &o)?;
        for w in linked.warnings {
            if !warnings.contains(&w) {
                warnings.push(w);
            }
        }
        let resource = place.as_ref().map_or("null".to_string(), |(dir, _)| json_str(&format!("{dir}/{stem}.tres")));
        let pages: Vec<String> =
            linked.pages.iter().map(|(w, h, png)| format!(r#"{{"w":{w},"h":{h},"png":"{}"}}"#, base64(png))).collect();
        let animations: Vec<String> = linked
            .animations
            .iter()
            .map(|a| {
                let frames: Vec<String> = a
                    .frames
                    .iter()
                    .map(|f| {
                        format!(
                            r#"{{"page":{},"region":{},"margin":{},"duration":{}}}"#,
                            f.page,
                            rect(f.region),
                            f.margin.map_or("null".into(), rect),
                            num(f.duration)
                        )
                    })
                    .collect();
                format!(
                    r#"{{"name":{},"loop":{},"speed":{},"frames":[{}]}}"#,
                    json_str(&a.name),
                    a.looped,
                    num(a.speed),
                    frames.join(",")
                )
            })
            .collect();
        resources.push(format!(
            r#"{{"resource":{resource},"pages":[{}],"animations":[{}]}}"#,
            pages.join(","),
            animations.join(",")
        ));
    }
    let warnings: Vec<String> = warnings.iter().map(|w| json_str(w)).collect();
    Ok(format!(
        r#"{{"linked":{},"scale":{},"warnings":[{}],"resources":[{}]}}"#,
        place.is_some(),
        o.scale.max(1),
        warnings.join(","),
        resources.join(",")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::json;

    #[test]
    fn base64_matches_the_rfc_vectors() {
        for (i, o) in
            [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE=")]
        {
            assert_eq!(base64(i.as_bytes()), o);
        }
    }

    #[test]
    fn an_unexported_document_links_with_default_options() {
        let s = crate::gamedev::godot::tests::sprite(2, &[100, 100]);
        let v = json::parse(&json(&s).unwrap()).unwrap();
        assert!(matches!(v.get("linked"), Some(json::Value::Bool(false))));
        let r = &v.get("resources").unwrap().as_array().unwrap()[0];
        assert!(matches!(r.get("resource"), Some(json::Value::Null)));
        let pages = r.get("pages").unwrap().as_array().unwrap();
        assert_eq!(pages.len(), 1);
        assert!(pages[0].get("png").unwrap().as_str().unwrap().starts_with("iVBORw0KGgo"), "a PNG");
        let anims = r.get("animations").unwrap().as_array().unwrap();
        let frames = anims[0].get("frames").unwrap().as_array().unwrap();
        assert_eq!(frames.len(), 2);
        for f in frames {
            assert_eq!(f.get("region").unwrap().as_array().unwrap().len(), 4);
            assert_eq!(f.get("page").unwrap().as_f64(), Some(0.0));
        }
    }

    #[test]
    fn regions_match_the_exported_tres() {
        // The same sprite and options give the same regions as the .tres the export writes.
        let s = crate::gamedev::godot::tests::two_tag();
        let o = sprite_frames::Options::default();
        let built = sprite_frames::build(&s, &crate::gamedev::godot::tests::target(None), &o).unwrap();
        let linked = sprite_frames::link(&s, &o).unwrap();
        for a in &linked.animations {
            for f in &a.frames {
                let (x, y, w, h) = f.region;
                assert!(built.tres.contains(&format!("region = Rect2({x}, {y}, {w}, {h})")), "{x},{y},{w},{h}");
            }
        }
        assert_eq!(linked.pages.len(), built.pages.len());
        assert_eq!(linked.pages[0].2, built.pages[0].1, "the same atlas PNG");
    }
}
