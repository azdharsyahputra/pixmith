//! Extensions: `.pixmith-extension` packages that add scripts, palettes and themes.
//!
//! A package is a zip file with a `package.json` at its root (name, displayName, version, author, description and
//! `contributes`: `scripts` (Lua), `palettes` (`.gpl` / `.hex`) and `themes` (see [`theme`])). Installing unpacks
//! it into `<app data>/Extensions/<name>/` after validating every name and size ([`zip`]), the manifest
//! ([`manifest`]) and the contributed files themselves. [`Store`] works on any folder (tests); the free functions
//! use the app's own, which honors `PIXMITH_SUPPORT_DIR` (see `docs/LIVE_PROTOCOL.md`, "Discovery"), and keep the
//! parsed result cached so menus built every frame cost nothing.
//!
//! The C ABI is in [`ffi`]; the format is described in `docs/EXTENSIONS.md`.

pub mod ffi;
pub mod manifest;
pub mod theme;
pub mod zip;

use crate::formats::json::{self, Value};
use crate::palette::io;
pub use manifest::Manifest;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
pub use theme::Theme;

/// File extension of a package, without the dot.
pub const PACKAGE_EXTENSION: &str = "pixmith-extension";

#[derive(Clone, Debug, PartialEq)]
pub struct PaletteEntry {
    pub name: String,
    pub colors: Vec<[u8; 4]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeEntry {
    /// `<extension>/<path in the package>`: stable, unique, stored in the preferences.
    pub id: String,
    pub theme: Theme,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScriptEntry {
    pub name: String,
    pub path: PathBuf,
    /// The extension's folder (`plugin.path` of the plugin).
    pub dir: PathBuf,
    /// Where the plugin's preferences are kept: outside the extension folder, so an update does not lose them.
    pub prefs: PathBuf,
}

/// One installed extension.
#[derive(Clone, Debug)]
pub struct Extension {
    pub manifest: Manifest,
    pub dir: PathBuf,
    pub enabled: bool,
    /// Why the extension could not be loaded or why a contribution was skipped. An extension with an error
    /// contributes nothing but can still be uninstalled.
    pub error: Option<String>,
    pub palettes: Vec<PaletteEntry>,
    pub themes: Vec<ThemeEntry>,
    pub scripts: Vec<ScriptEntry>,
}

impl Extension {
    /// Whether the extension takes part (enabled and loaded without errors).
    pub fn active(&self) -> bool {
        self.enabled && self.error.is_none()
    }

    pub fn to_json(&self) -> Value {
        let s = |t: &str| Value::Str(t.to_string());
        Value::Obj(vec![
            ("name".into(), s(&self.manifest.name)),
            ("displayName".into(), s(&self.manifest.display_name)),
            ("version".into(), s(&self.manifest.version)),
            ("author".into(), s(&self.manifest.author)),
            ("description".into(), s(&self.manifest.description)),
            ("enabled".into(), Value::Bool(self.enabled)),
            ("dir".into(), s(&self.dir.to_string_lossy())),
            ("error".into(), self.error.as_deref().map_or(Value::Null, s)),
            (
                "scripts".into(),
                Value::Arr(
                    self.scripts
                        .iter()
                        .map(|e| {
                            Value::Obj(vec![("name".into(), s(&e.name)), ("path".into(), s(&e.path.to_string_lossy()))])
                        })
                        .collect(),
                ),
            ),
            (
                "palettes".into(),
                Value::Arr(
                    self.palettes
                        .iter()
                        .map(|p| {
                            Value::Obj(vec![
                                ("name".into(), s(&p.name)),
                                ("colors".into(), Value::Num(p.colors.len() as f64)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "themes".into(),
                Value::Arr(
                    self.themes
                        .iter()
                        .map(|t| {
                            Value::Obj(vec![
                                ("id".into(), s(&t.id)),
                                ("name".into(), s(&t.theme.name)),
                                ("dark".into(), Value::Bool(t.theme.styles_dark())),
                                ("light".into(), Value::Bool(t.theme.styles_light())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

// MARK: Locations

/// The platform's application data folder, or `PIXMITH_SUPPORT_DIR` when it is set.
fn support_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("PIXMITH_SUPPORT_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(p);
    }
    let home = || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData/Roaming"))
    } else {
        match std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
            Some(p) => p,
            None => home().join(".local/share"),
        }
    }
}

/// `<app data>`: `Pixmith` inside the support folder (`pixmith` on Linux).
pub fn app_dir() -> PathBuf {
    support_dir().join(if cfg!(any(target_os = "macos", windows)) { "Pixmith" } else { "pixmith" })
}

/// `<app data>/Extensions`.
pub fn extensions_dir() -> PathBuf {
    app_dir().join("Extensions")
}

// MARK: Store

static STAGING_COUNTER: AtomicU32 = AtomicU32::new(0);

/// A folder of installed extensions.
#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
}

fn read_limited(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    file.take(max + 1).read_to_end(&mut out).map_err(|e| format!("{}: {e}", path.display()))?;
    if out.len() as u64 > max {
        return Err(format!("{}: larger than {} MB", path.display(), max >> 20));
    }
    Ok(out)
}

/// A contribution's text, checked for the right kind.
fn check_contents(kind: &str, path: &str, bytes: &[u8], fallback_name: &str) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| format!("{path}: not UTF-8 text"))?;
    match kind {
        "palettes" => io::parse(text).map(drop).map_err(|e| format!("{path}: {e}")),
        "themes" => theme::parse(text, fallback_name).map(drop).map_err(|e| format!("{path}: {e}")),
        _ => Ok(()),
    }
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Store {
        Store { root: root.into() }
    }

    fn state_file(&self) -> PathBuf {
        self.root.join("state.json")
    }

    fn disabled(&self) -> Vec<String> {
        let Ok(text) = fs::read_to_string(self.state_file()) else { return Vec::new() };
        json::parse(&text)
            .ok()
            .and_then(|v| v.get("disabled").and_then(Value::as_array).map(<[Value]>::to_vec))
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }

    fn write_disabled(&self, names: &[String]) -> Result<(), String> {
        let v =
            Value::Obj(vec![("disabled".into(), Value::Arr(names.iter().map(|n| Value::Str(n.clone())).collect()))]);
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        crate::formats::durable::write_file(&self.state_file().to_string_lossy(), json::to_string(&v).as_bytes())
    }

    /// Reads one extension folder: manifest, then every contribution. Problems are recorded, not returned.
    fn load_dir(&self, dir: &Path, folder: &str, disabled: &[String]) -> Extension {
        let mut ext = Extension {
            manifest: Manifest { name: folder.to_string(), display_name: folder.to_string(), ..Default::default() },
            dir: dir.to_path_buf(),
            enabled: !disabled.iter().any(|d| d == folder),
            error: None,
            palettes: Vec::new(),
            themes: Vec::new(),
            scripts: Vec::new(),
        };
        let manifest = read_limited(&dir.join("package.json"), 64 * 1024).and_then(|b| Manifest::parse(&b));
        match manifest {
            Ok(m) if m.name == folder => ext.manifest = m,
            Ok(m) => {
                ext.error =
                    Some(format!("package.json names the extension {:?}, but the folder is {folder:?}", m.name));
                return ext;
            }
            Err(e) => {
                ext.error = Some(e);
                return ext;
            }
        }
        let mut problems: Vec<String> = Vec::new();
        let m = ext.manifest.clone();
        let read = |rel: &str| -> Result<Vec<u8>, String> {
            // Installed files are plain files we created; a link planted later must not lead outside.
            let path = dir.join(rel);
            let meta = fs::symlink_metadata(&path).map_err(|_| format!("{rel}: missing"))?;
            if !meta.is_file() {
                return Err(format!("{rel}: not a regular file"));
            }
            read_limited(&path, 8 << 20)
        };
        for c in &m.scripts {
            match read(&c.path).and_then(|b| check_contents("scripts", &c.path, &b, &c.name)) {
                Ok(()) => ext.scripts.push(ScriptEntry {
                    name: c.name.clone(),
                    path: dir.join(&c.path),
                    dir: dir.to_path_buf(),
                    prefs: self.root.join(".prefs").join(folder).join(format!("{}.json", c.path.replace('/', "_"))),
                }),
                Err(e) => problems.push(e),
            }
        }
        for c in &m.palettes {
            let parsed = read(&c.path).and_then(|b| {
                let text = String::from_utf8(b).map_err(|_| format!("{}: not UTF-8 text", c.path))?;
                io::parse(&text).map_err(|e| format!("{}: {e}", c.path))
            });
            match parsed {
                Ok(colors) => ext.palettes.push(PaletteEntry { name: c.name.clone(), colors }),
                Err(e) => problems.push(e),
            }
        }
        for c in &m.themes {
            let parsed = read(&c.path).and_then(|b| {
                let text = String::from_utf8(b).map_err(|_| format!("{}: not UTF-8 text", c.path))?;
                theme::parse(&text, &c.name).map_err(|e| format!("{}: {e}", c.path))
            });
            match parsed {
                Ok(mut t) => {
                    // The manifest's label wins when the theme file has no name of its own.
                    if t.name.is_empty() {
                        t.name = c.name.clone();
                    }
                    ext.themes.push(ThemeEntry { id: format!("{}/{}", m.name, c.path), theme: t });
                }
                Err(e) => problems.push(e),
            }
        }
        if !problems.is_empty() {
            ext.error = Some(problems.join("; "));
        }
        ext
    }

    /// Every installed extension, sorted by display name. Folders starting with `.` are ignored.
    pub fn list(&self) -> Vec<Extension> {
        let disabled = self.disabled();
        let mut out: Vec<Extension> = fs::read_dir(&self.root)
            .map(|d| {
                d.filter_map(Result::ok)
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let is_dir = e.file_type().ok()?.is_dir();
                        (is_dir && !name.starts_with('.')).then(|| self.load_dir(&e.path(), &name, &disabled))
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| {
            a.manifest.display_name.to_lowercase().cmp(&b.manifest.display_name.to_lowercase()).then(a.dir.cmp(&b.dir))
        });
        out
    }

    pub fn get(&self, name: &str) -> Option<Extension> {
        manifest::valid_name(name).then(|| self.list().into_iter().find(|e| e.manifest.name == name)).flatten()
    }

    /// Installs (or updates) the package at `path`.
    pub fn install_file(&self, path: &Path) -> Result<Extension, String> {
        let bytes = read_limited(path, zip::Limits::default().max_package)
            .map_err(|e| format!("could not read the package: {e}"))?;
        self.install_bytes(&bytes)
    }

    /// Validates a package completely, then unpacks it. Nothing is written to the extension folder until
    /// every check has passed; the new version replaces an older one in one rename.
    pub fn install_bytes(&self, bytes: &[u8]) -> Result<Extension, String> {
        let limits = zip::Limits::default();
        let archive = zip::Archive::open(bytes, &limits)?;
        let manifest_entry = archive.find("package.json").ok_or("the package has no package.json at its root")?;
        let manifest = Manifest::parse(&archive.read(manifest_entry)?)?;
        for (kind, list) in
            [("scripts", &manifest.scripts), ("palettes", &manifest.palettes), ("themes", &manifest.themes)]
        {
            for c in list {
                let entry = archive
                    .find(&c.path)
                    .ok_or_else(|| format!("contributes.{kind}: {} is not in the package", c.path))?;
                check_contents(kind, &c.path, &archive.read(entry)?, &c.name)?;
            }
        }
        fs::create_dir_all(&self.root).map_err(|e| format!("could not create {}: {e}", self.root.display()))?;
        let staging = self.root.join(format!(
            ".staging-{}-{}",
            std::process::id(),
            STAGING_COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&staging);
        let unpack = || -> Result<(), String> {
            fs::create_dir(&staging).map_err(|e| e.to_string())?;
            for entry in &archive.entries {
                let target = staging.join(&entry.name);
                if entry.is_dir {
                    fs::create_dir_all(&target).map_err(|e| e.to_string())?;
                    continue;
                }
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let data = archive.read(entry)?;
                // `create_new` never follows or replaces anything that is already there.
                use std::io::Write;
                let mut f = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                    .map_err(|e| format!("{}: {e}", entry.name))?;
                f.write_all(&data).map_err(|e| format!("{}: {e}", entry.name))?;
            }
            Ok(())
        };
        if let Err(e) = unpack() {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
        let dest = self.root.join(&manifest.name);
        let backup = self.root.join(format!(".old-{}-{}", std::process::id(), manifest.name));
        let _ = fs::remove_dir_all(&backup);
        let had_old = fs::symlink_metadata(&dest).is_ok();
        if had_old {
            if let Err(e) = fs::rename(&dest, &backup) {
                let _ = fs::remove_dir_all(&staging);
                return Err(format!("could not replace the installed version: {e}"));
            }
        }
        if let Err(e) = fs::rename(&staging, &dest) {
            if had_old {
                let _ = fs::rename(&backup, &dest);
            }
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("could not install into {}: {e}", dest.display()));
        }
        if had_old {
            let _ = fs::remove_dir_all(&backup);
        }
        self.get(&manifest.name).ok_or_else(|| "the extension was installed but cannot be read back".into())
    }

    pub fn set_enabled(&self, name: &str, enabled: bool) -> Result<(), String> {
        if !manifest::valid_name(name) || fs::symlink_metadata(self.root.join(name)).is_err() {
            return Err(format!("no extension named {name:?}"));
        }
        let mut disabled = self.disabled();
        disabled.retain(|d| d != name);
        if !enabled {
            disabled.push(name.to_string());
        }
        self.write_disabled(&disabled)
    }

    /// Removes the extension's folder. A folder that is a link is unlinked, never followed.
    pub fn uninstall(&self, name: &str) -> Result<(), String> {
        if !manifest::valid_name(name) {
            return Err(format!("{name:?} is not an extension name"));
        }
        let dir = self.root.join(name);
        let meta = fs::symlink_metadata(&dir).map_err(|_| format!("no extension named {name:?}"))?;
        if meta.file_type().is_symlink() {
            fs::remove_file(&dir).or_else(|_| fs::remove_dir(&dir)).map_err(|e| e.to_string())?;
        } else {
            fs::remove_dir_all(&dir).map_err(|e| format!("could not remove {}: {e}", dir.display()))?;
        }
        let _ = fs::remove_dir_all(self.root.join(".prefs").join(name));
        let mut disabled = self.disabled();
        if disabled.iter().any(|d| d == name) {
            disabled.retain(|d| d != name);
            self.write_disabled(&disabled)?;
        }
        Ok(())
    }
}

// MARK: The app's own store

struct Cache {
    root: PathBuf,
    list: Arc<Vec<Extension>>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

pub fn store() -> Store {
    Store::new(extensions_dir())
}

static GENERATION: AtomicU64 = AtomicU64::new(1);

/// Forgets the cached list; the next query reads the folder again (after a change on disk).
pub fn refresh() {
    *CACHE.lock().unwrap_or_else(|p| p.into_inner()) = None;
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// Changes whenever the installed set may have changed (install, uninstall, enable, disable, [`refresh`]).
/// A front end that keeps something built from [`scripts`] compares it every frame and rebuilds on a change.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

/// The installed extensions (cached; see [`refresh`]).
pub fn installed() -> Arc<Vec<Extension>> {
    let root = extensions_dir();
    let mut cache = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(c) = cache.as_ref().filter(|c| c.root == root) {
        return c.list.clone();
    }
    let list = Arc::new(Store::new(&root).list());
    *cache = Some(Cache { root, list: list.clone() });
    list
}

pub fn install(path: &Path) -> Result<Extension, String> {
    let r = store().install_file(path);
    refresh();
    r
}

pub fn set_enabled(name: &str, enabled: bool) -> Result<(), String> {
    let r = store().set_enabled(name, enabled);
    refresh();
    r
}

pub fn uninstall(name: &str) -> Result<(), String> {
    let r = store().uninstall(name);
    refresh();
    r
}

/// Contributions of the active extensions.
pub fn palettes() -> Vec<(String, PaletteEntry)> {
    installed()
        .iter()
        .filter(|e| e.active())
        .flat_map(|e| e.palettes.iter().map(|p| (e.manifest.display_name.clone(), p.clone())))
        .collect()
}

pub fn themes() -> Vec<ThemeEntry> {
    installed().iter().filter(|e| e.active()).flat_map(|e| e.themes.clone()).collect()
}

pub fn theme(id: &str) -> Option<ThemeEntry> {
    themes().into_iter().find(|t| t.id == id)
}

pub fn scripts() -> Vec<(String, ScriptEntry)> {
    installed()
        .iter()
        .filter(|e| e.active())
        .flat_map(|e| e.scripts.iter().map(|s| (e.manifest.name.clone(), s.clone())))
        .collect()
}

/// Whether `path` names an extension package (by its file extension).
pub fn is_package(path: &Path) -> bool {
    path.extension().is_some_and(|x| x.eq_ignore_ascii_case(PACKAGE_EXTENSION))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    /// Held by tests that change `PIXMITH_SUPPORT_DIR`.
    pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

    static N: AtomicU32 = AtomicU32::new(0);

    fn temp_store() -> (Store, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "pixmith-ext-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&base);
        (Store::new(base.join("Extensions")), base)
    }

    pub(crate) const MANIFEST: &str = r#"{"name":"sample","displayName":"Sample Pack","version":"1.0.0","author":"A",
      "description":"d","contributes":{"scripts":[{"name":"Hello","path":"scripts/hello.lua"}],
      "palettes":[{"name":"Sunset","path":"palettes/sunset.gpl"},"palettes/two.hex"],
      "themes":[{"name":"Midnight","path":"themes/midnight.json"}]}}"#;
    const GPL: &str = "GIMP Palette\nName: Sunset\n#\n255 0 0 red\n255 128 0 orange\n255 255 0 yellow\n";
    const THEME: &str =
        r##"{"name":"Midnight","accent":"#FF6AC1","dark":{"panel":"#14141F"},"light":{"panel":"#F4F1FA"}}"##;

    pub(crate) fn sample_package() -> Vec<u8> {
        zip::build(&[
            ("package.json", MANIFEST.as_bytes()),
            ("scripts/hello.lua", b"print('hi')"),
            ("palettes/sunset.gpl", GPL.as_bytes()),
            ("palettes/two.hex", b"000000\nffffff\n"),
            ("themes/midnight.json", THEME.as_bytes()),
            ("README.md", b"# Sample"),
        ])
    }

    #[test]
    fn install_list_toggle_uninstall() {
        let (store, base) = temp_store();
        assert!(store.list().is_empty());
        let ext = store.install_bytes(&sample_package()).unwrap();
        assert_eq!(ext.manifest.name, "sample");
        assert!(ext.active() && ext.error.is_none(), "{:?}", ext.error);
        assert_eq!(store.root.join("sample"), ext.dir);
        assert!(ext.dir.join("README.md").is_file());
        assert_eq!(ext.scripts.len(), 1);
        assert_eq!(ext.scripts[0].name, "Hello");
        assert!(ext.scripts[0].path.is_file());
        assert_eq!(ext.palettes.len(), 2);
        assert_eq!(ext.palettes[0].colors.len(), 3);
        assert_eq!(ext.palettes[1].name, "two");
        assert_eq!(ext.themes[0].id, "sample/themes/midnight.json");
        assert_eq!(ext.themes[0].theme.get("panel", true), Some(0x14141FFF));

        let listed = store.list();
        assert_eq!(listed.len(), 1);
        store.set_enabled("sample", false).unwrap();
        assert!(!store.get("sample").unwrap().enabled);
        assert!(!store.get("sample").unwrap().active());
        store.set_enabled("sample", true).unwrap();
        assert!(store.get("sample").unwrap().enabled);
        assert!(store.set_enabled("nope", false).is_err());
        assert!(store.set_enabled("../x", false).is_err());

        store.set_enabled("sample", false).unwrap();
        store.uninstall("sample").unwrap();
        assert!(store.list().is_empty());
        assert!(!store.root.join("sample").exists());
        assert!(store.uninstall("sample").is_err());
        assert!(store.uninstall("../Extensions").is_err());
        // The disabled mark went with it: a fresh install is enabled.
        assert!(store.install_bytes(&sample_package()).unwrap().enabled);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn reinstalling_replaces_the_old_version_completely() {
        let (store, base) = temp_store();
        store.install_bytes(&sample_package()).unwrap();
        store.set_enabled("sample", false).unwrap();
        let v2 = zip::build(&[("package.json", br#"{"name":"sample","version":"2.0.0"}"#)]);
        let ext = store.install_bytes(&v2).unwrap();
        assert_eq!(ext.manifest.version, "2.0.0");
        assert!(!ext.dir.join("README.md").exists(), "old files are gone");
        assert!(!ext.enabled, "the enabled state is kept across an update");
        let leftovers: Vec<_> = fs::read_dir(&store.root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn malicious_and_broken_packages_install_nothing() {
        let (store, base) = temp_store();
        let evil_manifest = br#"{"name":"evil","version":"1","contributes":{"scripts":["a.lua"]}}"#;
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("not a zip", b"hello, this is not a zip".to_vec()),
            (
                "path traversal",
                zip::build(&[("package.json", br#"{"name":"x","version":"1"}"#), ("../../evil.lua", b"x")]),
            ),
            ("absolute", zip::build(&[("package.json", br#"{"name":"x","version":"1"}"#), ("/tmp/evil", b"x")])),
            ("backslash", zip::build(&[("package.json", br#"{"name":"x","version":"1"}"#), ("..\\evil", b"x")])),
            ("no manifest", zip::build(&[("a.lua", b"x")])),
            ("manifest not at root", zip::build(&[("sub/package.json", br#"{"name":"x","version":"1"}"#)])),
            ("bad name", zip::build(&[("package.json", br#"{"name":"../x","version":"1"}"#)])),
            ("missing contribution", zip::build(&[("package.json", evil_manifest)])),
            (
                "contribution traversal",
                zip::build(&[(
                    "package.json",
                    br#"{"name":"x","version":"1","contributes":{"scripts":["../a.lua"]}}"#,
                )]),
            ),
            (
                "broken palette",
                zip::build(&[
                    ("package.json", br#"{"name":"x","version":"1","contributes":{"palettes":["p.gpl"]}}"#),
                    ("p.gpl", b"GIMP Palette\n1 2\n"),
                ]),
            ),
            (
                "broken theme",
                zip::build(&[
                    ("package.json", br#"{"name":"x","version":"1","contributes":{"themes":["t.json"]}}"#),
                    ("t.json", b"{\"dark\":{\"panel\":\"red\"}}"),
                ]),
            ),
            (
                "script not utf-8",
                zip::build(&[
                    ("package.json", br#"{"name":"x","version":"1","contributes":{"scripts":["a.lua"]}}"#),
                    ("a.lua", &[0xFF, 0xFE, 0x00]),
                ]),
            ),
            ("duplicate", zip::build(&[("package.json", br#"{"name":"x","version":"1"}"#), ("PACKAGE.json", b"{}")])),
        ];
        for (label, bytes) in cases {
            let r = store.install_bytes(&bytes);
            assert!(r.is_err(), "{label} was installed");
        }
        // Nothing was created: not an extension, not staging leftovers, and nothing escaped.
        let entries: Vec<_> = fs::read_dir(&store.root).map(|d| d.filter_map(Result::ok).collect()).unwrap_or_default();
        assert!(entries.is_empty(), "{:?}", entries.iter().map(|e| e.file_name()).collect::<Vec<_>>());
        assert!(!base.join("evil.lua").exists());
        assert!(!base.parent().unwrap().join("evil.lua").exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn a_package_that_is_too_big_is_refused() {
        let (store, base) = temp_store();
        let big = vec![b'a'; 9 << 20];
        let z = zip::build(&[("package.json", br#"{"name":"x","version":"1"}"#), ("big.bin", &big)]);
        assert!(store.install_bytes(&z).unwrap_err().contains("MB"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn a_broken_install_on_disk_is_listed_with_its_error() {
        let (store, base) = temp_store();
        fs::create_dir_all(store.root.join("broken")).unwrap();
        fs::write(store.root.join("broken/package.json"), "{ nope").unwrap();
        fs::create_dir_all(store.root.join("renamed")).unwrap();
        fs::write(store.root.join("renamed/package.json"), br#"{"name":"other","version":"1"}"#).unwrap();
        fs::create_dir_all(store.root.join(".hidden")).unwrap();
        let list = store.list();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|e| e.error.is_some() && !e.active()));
        store.uninstall("broken").unwrap();
        assert_eq!(store.list().len(), 1);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn a_contribution_deleted_after_install_is_reported() {
        let (store, base) = temp_store();
        let ext = store.install_bytes(&sample_package()).unwrap();
        fs::remove_file(ext.dir.join("palettes/two.hex")).unwrap();
        let ext = store.get("sample").unwrap();
        assert!(ext.error.as_deref().unwrap().contains("two.hex"));
        assert!(!ext.active());
        let _ = fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn links_inside_an_installed_folder_are_not_followed() {
        let (store, base) = temp_store();
        let ext = store.install_bytes(&sample_package()).unwrap();
        let secret = base.join("secret.lua");
        fs::write(&secret, "print('x')").unwrap();
        fs::remove_file(ext.dir.join("scripts/hello.lua")).unwrap();
        std::os::unix::fs::symlink(&secret, ext.dir.join("scripts/hello.lua")).unwrap();
        let ext = store.get("sample").unwrap();
        assert!(ext.error.as_deref().unwrap().contains("not a regular file"));
        assert!(ext.scripts.is_empty());
        // Uninstalling a linked folder removes the link only.
        let target = base.join("target-dir");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("keep.txt"), "x").unwrap();
        std::os::unix::fs::symlink(&target, store.root.join("linked")).unwrap();
        store.uninstall("linked").unwrap();
        assert!(target.join("keep.txt").exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn install_from_a_file_and_package_names() {
        let (store, base) = temp_store();
        fs::create_dir_all(&base).unwrap();
        let file = base.join("sample.pixmith-extension");
        fs::write(&file, sample_package()).unwrap();
        assert!(is_package(&file));
        assert!(is_package(Path::new("X.PIXMITH-EXTENSION")));
        assert!(!is_package(Path::new("x.zip")));
        store.install_file(&file).unwrap();
        assert!(store.install_file(&base.join("missing.pixmith-extension")).is_err());
        let _ = fs::remove_dir_all(base);
    }
}
