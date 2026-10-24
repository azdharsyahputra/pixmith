//! Theme files: a JSON document that overrides the design tokens of both front ends.
//!
//! ```json
//! {
//!   "name": "Midnight",
//!   "accent": "#FF6AC1",
//!   "dark":  { "panel": "#14141F", "text": "#EDEDF7" },
//!   "light": { "panel": "#F4F1FA" }
//! }
//! ```
//!
//! `dark` and `light` map token names to colors (`#RGB`, `#RRGGBB` or `#RRGGBBAA`). A theme may give only one of the
//! two; the other appearance keeps the default look. The top-level `accent` is used by both appearances unless a
//! block sets its own `accent`. Tokens not mentioned keep their default values.

use crate::formats::json::{self, Value};

/// The overridable tokens, in the order the front ends list them. These are the semantic colors of
/// `DSColor` (Swift) and `Colors` (egui); the accent's tinted variants (selected fills, outlines) follow `accent`.
pub const TOKENS: [&str; 18] = [
    "workspace",
    "panel",
    "panel2",
    "recessed",
    "elevated",
    "separator",
    "separatorSoft",
    "field",
    "hover",
    "pressed",
    "accent",
    "destructive",
    "guide",
    "overlay",
    "canvasShadow",
    "text",
    "textSecondary",
    "textTertiary",
];

const MAX_THEME_BYTES: usize = 64 * 1024;

/// A parsed theme: `0xRRGGBBAA` per token, for each appearance that the theme styles.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Theme {
    pub name: String,
    pub dark: Vec<(String, u32)>,
    pub light: Vec<(String, u32)>,
}

impl Theme {
    pub fn styles_dark(&self) -> bool {
        !self.dark.is_empty()
    }

    pub fn styles_light(&self) -> bool {
        !self.light.is_empty()
    }

    /// The value of `token` for an appearance.
    pub fn get(&self, token: &str, dark: bool) -> Option<u32> {
        let list = if dark { &self.dark } else { &self.light };
        list.iter().find(|(k, _)| k == token).map(|(_, v)| *v)
    }

    /// JSON for the front ends: colors as `"RRGGBBAA"` hex strings.
    pub fn to_json(&self) -> Value {
        let block = |list: &[(String, u32)]| {
            Value::Obj(list.iter().map(|(k, v)| (k.clone(), Value::Str(format!("{v:08X}")))).collect())
        };
        Value::Obj(vec![
            ("name".into(), Value::Str(self.name.clone())),
            ("dark".into(), block(&self.dark)),
            ("light".into(), block(&self.light)),
        ])
    }
}

/// Parses `#RGB`, `#RRGGBB` or `#RRGGBBAA` (the `#` is optional) into `0xRRGGBBAA`.
pub fn parse_color(text: &str) -> Result<u32, String> {
    let t = text.trim().trim_start_matches('#');
    let bad = || format!("{text:?} is not a color (use #RRGGBB or #RRGGBBAA)");
    if !t.is_ascii() || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let v = |s: &str| u32::from_str_radix(s, 16).map_err(|_| bad());
    match t.len() {
        3 => {
            let (r, g, b) = (v(&t[0..1])?, v(&t[1..2])?, v(&t[2..3])?);
            Ok((r * 17) << 24 | (g * 17) << 16 | (b * 17) << 8 | 0xFF)
        }
        6 => Ok(v(t)? << 8 | 0xFF),
        8 => v(t),
        _ => Err(bad()),
    }
}

fn block(value: Option<&Value>, which: &str, accent: Option<u32>) -> Result<Vec<(String, u32)>, String> {
    let mut out: Vec<(String, u32)> = Vec::new();
    if let Some(v) = value {
        let members = v.as_object().ok_or_else(|| format!("\"{which}\" must be an object of colors"))?;
        for (key, c) in members {
            if !TOKENS.contains(&key.as_str()) {
                return Err(format!("\"{which}\": unknown token {key:?} (known: {})", TOKENS.join(", ")));
            }
            let text = c.as_str().ok_or_else(|| format!("\"{which}.{key}\" must be a color string"))?;
            let color = parse_color(text).map_err(|e| format!("\"{which}.{key}\": {e}"))?;
            out.retain(|(k, _)| k != key);
            out.push((key.clone(), color));
        }
    }
    // The shared accent only goes into an appearance the theme styles.
    if let Some(a) = accent {
        if !out.is_empty() && !out.iter().any(|(k, _)| k == "accent") {
            out.push(("accent".into(), a));
        }
    }
    Ok(out)
}

/// Parses theme JSON. `fallback_name` names themes that do not give a `name`.
pub fn parse(text: &str, fallback_name: &str) -> Result<Theme, String> {
    if text.len() > MAX_THEME_BYTES {
        return Err("theme file is too large".into());
    }
    let root = json::parse(text.trim_start_matches('\u{feff}'))?;
    let Some(members) = root.as_object() else {
        return Err("a theme must be a JSON object".into());
    };
    let name = match root.get("name") {
        Some(Value::Str(s)) if !s.trim().is_empty() => s.trim().to_string(),
        Some(_) => return Err("\"name\" must be a non-empty string".into()),
        None => fallback_name.to_string(),
    };
    if name.chars().count() > 60 || name.chars().any(char::is_control) {
        return Err("theme name is too long or has control characters".into());
    }
    let accent = match root.get("accent") {
        Some(Value::Str(s)) => Some(parse_color(s).map_err(|e| format!("\"accent\": {e}"))?),
        Some(_) => return Err("\"accent\" must be a color string".into()),
        None => None,
    };
    for (key, _) in members {
        if !["name", "accent", "dark", "light", "description", "author"].contains(&key.as_str()) {
            return Err(format!("unknown member {key:?} (a theme has name, accent, dark and light)"));
        }
    }
    let dark = block(root.get("dark"), "dark", accent)?;
    let light = block(root.get("light"), "light", accent)?;
    if dark.is_empty() && light.is_empty() {
        return Err("a theme must style at least one token in \"dark\" or \"light\"".into());
    }
    Ok(Theme { name, dark, light })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_parse_in_all_forms() {
        assert_eq!(parse_color("#FF8000").unwrap(), 0xFF8000FF);
        assert_eq!(parse_color("ff800080").unwrap(), 0xFF800080);
        assert_eq!(parse_color("#f80").unwrap(), 0xFF8800FF);
        for bad in ["", "#12", "#GGGGGG", "#1234567", "red", "#ÿÿÿ"] {
            assert!(parse_color(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_theme_with_both_appearances() {
        let t = parse(
            r##"{"name":"Midnight","accent":"#FF6AC1","dark":{"panel":"#14141F","text":"#EDEDF7"},"light":{"panel":"#F4F1FA","accent":"#00AA00"}}"##,
            "x",
        )
        .unwrap();
        assert_eq!(t.name, "Midnight");
        assert_eq!(t.get("panel", true), Some(0x14141FFF));
        assert_eq!(t.get("accent", true), Some(0xFF6AC1FF), "top-level accent fills in");
        assert_eq!(t.get("accent", false), Some(0x00AA00FF), "a block's own accent wins");
        assert!(t.styles_dark() && t.styles_light());
        assert_eq!(t.get("hover", true), None);
    }

    #[test]
    fn a_dark_only_theme_leaves_light_alone() {
        let t = parse(r##"{"dark":{"panel":"#000"}}"##, "Fallback").unwrap();
        assert_eq!(t.name, "Fallback");
        assert!(t.styles_dark() && !t.styles_light());
        let t = parse(r##"{"accent":"#fff","dark":{"panel":"#000"}}"##, "F").unwrap();
        assert!(!t.styles_light(), "an accent alone does not make a light theme");
    }

    #[test]
    fn malformed_themes_are_errors() {
        for bad in [
            "",
            "[]",
            "{}",
            r#"{"dark":{}}"#,
            r##"{"dark":{"panel":"nope"}}"##,
            r##"{"dark":{"paneel":"#000"}}"##,
            r##"{"dark":{"panel":12}}"##,
            r##"{"dark":[1]}"##,
            r##"{"name":3,"dark":{"panel":"#000"}}"##,
            r##"{"name":"","dark":{"panel":"#000"}}"##,
            r##"{"colors":{},"dark":{"panel":"#000"}}"##,
            r##"{"accent":5,"dark":{"panel":"#000"}}"##,
        ] {
            assert!(parse(bad, "x").is_err(), "{bad}");
        }
        assert!(parse(&" ".repeat(70_000), "x").is_err());
    }

    #[test]
    fn json_form_has_hex_strings() {
        let t = parse(r##"{"name":"A","dark":{"panel":"#0A0B0C"}}"##, "x").unwrap();
        let j = json::to_string(&t.to_json());
        assert!(j.contains(r#""panel":"0A0B0CFF""#), "{j}");
    }
}
