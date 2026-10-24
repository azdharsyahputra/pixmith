//! `package.json` of an extension.

use crate::formats::json::{self, Value};

const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_PER_KIND: usize = 64;

/// A file an extension contributes: a label and a path inside the package (`/`-separated).
#[derive(Clone, Debug, PartialEq)]
pub struct Contribution {
    pub name: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Manifest {
    /// Folder name and identity: lower case letters, digits, `.`, `-` and `_`.
    pub name: String,
    pub display_name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub scripts: Vec<Contribution>,
    pub palettes: Vec<Contribution>,
    pub themes: Vec<Contribution>,
}

/// Whether `name` is usable as an extension's identity and folder name.
pub fn valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'-' | b'_'))
        && !name.ends_with('.')
        && !name.contains("..")
}

fn text(v: &Value, key: &str, max: usize, required: bool) -> Result<String, String> {
    match v.get(key) {
        Some(Value::Str(s)) => {
            let s = s.trim();
            if s.chars().count() > max || s.chars().any(|c| c.is_control() && c != '\n') {
                return Err(format!("\"{key}\" is too long or has control characters"));
            }
            if required && s.is_empty() {
                return Err(format!("\"{key}\" must not be empty"));
            }
            Ok(s.to_string())
        }
        Some(Value::Null) | None if !required => Ok(String::new()),
        Some(_) => Err(format!("\"{key}\" must be a string")),
        None => Err(format!("package.json needs \"{key}\"")),
    }
}

/// `"a/b.lua"` or `{"name": "Label", "path": "a/b.lua"}`; the label defaults to the file name without extension.
fn contributions(v: Option<&Value>, kind: &str, extensions: &[&str]) -> Result<Vec<Contribution>, String> {
    let Some(v) = v else { return Ok(Vec::new()) };
    let items = v.as_array().ok_or_else(|| format!("contributes.{kind} must be a list"))?;
    if items.len() > MAX_PER_KIND {
        return Err(format!("contributes.{kind} has more than {MAX_PER_KIND} entries"));
    }
    let mut out: Vec<Contribution> = Vec::new();
    for item in items {
        let (name, path) = match item {
            Value::Str(p) => (None, p.clone()),
            Value::Obj(_) => {
                let path = item
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or(format!("a contributes.{kind} entry needs a \"path\""))?;
                (item.get("name").and_then(Value::as_str).map(str::to_string), path.to_string())
            }
            _ => return Err(format!("contributes.{kind} entries must be paths or {{name, path}}")),
        };
        if path.ends_with('/') {
            return Err(format!("contributes.{kind}: {path} is a folder"));
        }
        super::zip::safe_name(&path, &super::zip::Limits::default()).map_err(|e| format!("contributes.{kind}: {e}"))?;
        let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        if !path.contains('.') || !extensions.contains(&ext.as_str()) {
            return Err(format!("contributes.{kind}: {path} must end in {}", extensions.join(" or ")));
        }
        let stem = path.rsplit('/').next().unwrap_or(&path);
        let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
        let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| stem.to_string());
        if name.chars().count() > 60 || name.chars().any(char::is_control) {
            return Err(format!("contributes.{kind}: name of {path} is too long"));
        }
        if out.iter().any(|c| c.path == path) {
            return Err(format!("contributes.{kind}: {path} is listed twice"));
        }
        out.push(Contribution { name, path });
    }
    Ok(out)
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Manifest, String> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err("package.json is too large".into());
        }
        let s = std::str::from_utf8(bytes).map_err(|_| "package.json is not UTF-8")?;
        let v = json::parse(s.trim_start_matches('\u{feff}')).map_err(|e| format!("package.json: {e}"))?;
        if v.as_object().is_none() {
            return Err("package.json must be a JSON object".into());
        }
        let name = text(&v, "name", 64, true)?;
        if !valid_name(&name) {
            return Err(format!(
                "extension name {name:?} is not valid: use lower case letters, digits, \".\", \"-\" and \"_\" (at most 64)"
            ));
        }
        let version = text(&v, "version", 32, true)?;
        let display_name = text(&v, "displayName", 80, false)?;
        let c = v.get("contributes");
        if matches!(c, Some(x) if x.as_object().is_none()) {
            return Err("\"contributes\" must be an object".into());
        }
        let section = |key: &str, exts: &[&str]| contributions(c.and_then(|c| c.get(key)), key, exts);
        Ok(Manifest {
            display_name: if display_name.is_empty() { name.clone() } else { display_name },
            name,
            version,
            author: text(&v, "author", 120, false)?,
            description: text(&v, "description", 600, false)?,
            scripts: section("scripts", &["lua"])?,
            palettes: section("palettes", &["gpl", "hex"])?,
            themes: section("themes", &["json"])?,
        })
    }

    /// Every contributed path.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.scripts.iter().chain(&self.palettes).chain(&self.themes).map(|c| c.path.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{"name":"sample","displayName":"Sample","version":"1.2.0","author":"Me","description":"d",
        "contributes":{"scripts":["scripts/a.lua",{"name":"Beta","path":"scripts/b.lua"}],
        "palettes":[{"name":"Sunset","path":"p/sunset.gpl"}],"themes":["t/night.json"]}}"#;

    #[test]
    fn parses_a_manifest() {
        let m = Manifest::parse(GOOD.as_bytes()).unwrap();
        assert_eq!((m.name.as_str(), m.display_name.as_str(), m.version.as_str()), ("sample", "Sample", "1.2.0"));
        assert_eq!(m.scripts.len(), 2);
        assert_eq!(m.scripts[0].name, "a");
        assert_eq!(m.scripts[1].name, "Beta");
        assert_eq!(m.palettes[0], Contribution { name: "Sunset".into(), path: "p/sunset.gpl".into() });
        assert_eq!(m.themes[0].name, "night");
        assert_eq!(m.paths().count(), 4);
    }

    #[test]
    fn display_name_defaults_to_the_name() {
        let m = Manifest::parse(br#"{"name":"x","version":"1"}"#).unwrap();
        assert_eq!(m.display_name, "x");
        assert!(m.scripts.is_empty());
    }

    #[test]
    fn names() {
        for ok in ["a", "a-b", "a.b_c", "0x", "pico8-extras"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "A", "-a", ".a", "a b", "a/b", "a..b", "a.", "../x", "x\\y", &"a".repeat(65)] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn bad_manifests() {
        for bad in [
            "",
            "[]",
            r#"{"version":"1"}"#,
            r#"{"name":"x"}"#,
            r#"{"name":"../x","version":"1"}"#,
            r#"{"name":"X","version":"1"}"#,
            r#"{"name":"x","version":""}"#,
            r#"{"name":"x","version":1}"#,
            r#"{"name":"x","version":"1","contributes":[]}"#,
            r#"{"name":"x","version":"1","contributes":{"scripts":"a.lua"}}"#,
            r#"{"name":"x","version":"1","contributes":{"scripts":["../a.lua"]}}"#,
            r#"{"name":"x","version":"1","contributes":{"scripts":["/a.lua"]}}"#,
            r#"{"name":"x","version":"1","contributes":{"scripts":["a.exe"]}}"#,
            r#"{"name":"x","version":"1","contributes":{"scripts":["a"]}}"#,
            r#"{"name":"x","version":"1","contributes":{"scripts":["a.lua","a.lua"]}}"#,
            r#"{"name":"x","version":"1","contributes":{"scripts":[3]}}"#,
            r#"{"name":"x","version":"1","contributes":{"palettes":["p.lua"]}}"#,
            r#"{"name":"x","version":"1","contributes":{"themes":[{"name":"n"}]}}"#,
        ] {
            assert!(Manifest::parse(bad.as_bytes()).is_err(), "{bad}");
        }
        assert!(Manifest::parse(&vec![b' '; 70_000]).is_err());
        assert!(Manifest::parse(&[0xFF, 0xFE]).is_err());
    }
}
