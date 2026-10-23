//! Export settings: JSON (versioned) used by the FFI and stored per document.

use super::sprite_frames::{LayerMode, Options, Packing};
use crate::document::Sprite;
use crate::formats::json::{self, Value};
use crate::formats::sheet::json_str;

pub const VERSION: u32 = 1;
/// Line prefix inside the sprite's user data text that holds the saved settings.
const MARKER: &str = "pixmith-godot-export ";

#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub root: String,
    pub tres: String,
    pub options: Options,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn list(v: &[String]) -> String {
    format!("[{}]", v.iter().map(|s| json_str(s)).collect::<Vec<_>>().join(","))
}

impl Settings {
    pub fn to_json(&self) -> String {
        let o = &self.options;
        let layers = match &o.layers {
            LayerMode::Visible => json_str("visible"),
            LayerMode::Each => json_str("each"),
            LayerMode::Selected(n) => list(n),
        };
        format!(
            "{{\"v\":{VERSION},\"root\":{},\"tres\":{},\"padding\":{},\"extrude\":{},\"trim\":{},\"max_texture\":{},\"scale\":{},\"packing\":{},\"per_animation_png\":{},\"name_template\":{},\"layers\":{},\"tags_include\":{},\"tags_exclude\":{},\"tscn\":{},\"overwrite_tscn\":{}}}",
            json_str(&self.root),
            json_str(&self.tres),
            o.padding,
            o.extrude,
            o.trim,
            o.max_texture,
            o.scale,
            json_str(if o.packing == Packing::Grid { "grid" } else { "tight" }),
            o.per_animation_png,
            json_str(&o.name_template),
            layers,
            list(&o.tags_include),
            list(&o.tags_exclude),
            o.emit_tscn,
            o.overwrite_tscn
        )
    }

    /// Missing fields take their defaults; a newer `v` is refused.
    pub fn from_json(text: &str) -> Result<Settings, String> {
        let v = json::parse(text)?;
        if v.as_object().is_none() {
            return Err("settings must be a JSON object".into());
        }
        if let Some(ver) = v.get("v").and_then(Value::as_f64) {
            if ver as u32 > VERSION {
                return Err(format!("settings version {ver} is newer than this app understands"));
            }
        }
        let num = |k: &str| v.get(k).and_then(Value::as_f64);
        let flag = |k: &str, d: bool| match v.get(k) {
            Some(Value::Bool(b)) => *b,
            _ => d,
        };
        let d = Options::default();
        let layers = match v.get("layers") {
            Some(Value::Str(s)) if s == "each" => LayerMode::Each,
            Some(Value::Arr(_)) => LayerMode::Selected(strings(v.get("layers"))),
            _ => LayerMode::Visible,
        };
        let packing =
            if v.get("packing").and_then(Value::as_str) == Some("grid") { Packing::Grid } else { Packing::Tight };
        Ok(Settings {
            root: v.get("root").and_then(Value::as_str).unwrap_or("").to_string(),
            tres: v.get("tres").and_then(Value::as_str).unwrap_or("").to_string(),
            options: Options {
                padding: num("padding").map_or(d.padding, |n| n.clamp(0.0, 64.0) as u32),
                extrude: flag("extrude", d.extrude),
                trim: flag("trim", d.trim),
                max_texture: num("max_texture").map_or(d.max_texture, |n| n.max(0.0) as u32),
                scale: num("scale").map_or(d.scale, |n| n.clamp(1.0, 16.0) as u32),
                packing,
                per_animation_png: flag("per_animation_png", d.per_animation_png),
                name_template: v
                    .get("name_template")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&d.name_template)
                    .to_string(),
                layers,
                tags_include: strings(v.get("tags_include")),
                tags_exclude: strings(v.get("tags_exclude")),
                emit_tscn: flag("tscn", d.emit_tscn),
                overwrite_tscn: flag("overwrite_tscn", d.overwrite_tscn),
            },
        })
    }
}

/// Settings saved in the document's user data, if any.
pub fn load(s: &Sprite) -> Option<String> {
    let text = s.extra.sprite.text.as_deref()?;
    text.lines().find_map(|l| l.strip_prefix(MARKER)).map(str::to_string)
}

/// Saves (or replaces) the settings line, leaving the rest of the user data text alone.
pub fn store(s: &mut Sprite, json: &str) {
    let line = format!("{MARKER}{}", json.replace('\n', " "));
    let old = s.extra.sprite.text.take().unwrap_or_default();
    let mut lines: Vec<&str> = old.lines().filter(|l| !l.starts_with(MARKER)).collect();
    lines.push(&line);
    s.extra.sprite.text = Some(lines.join("\n"));
}
