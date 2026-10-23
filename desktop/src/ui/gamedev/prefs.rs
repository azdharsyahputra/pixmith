//! The few preferences this area keeps between runs (`UserDefaults` in the Swift app): Open Recent, the last
//! Godot project folder, whether the texture filter hint was shown, and the TileSet export settings
//! remembered per document.
//!
//! eframe's own storage needs a feature that is not enabled here, so they live in a small JSON file in the
//! app's folder (`paths::prefs_file`). A `Prefs` without a path (the default, used by tests) is memory only.

use super::TilesetSettings;
use crate::app::PixmithApp;
use pixmith_core::formats::json::{self, Value};
use pixmith_core::formats::sheet::json_str;
use std::path::{Path, PathBuf};

/// Recent files kept (`RecentFiles.limit`).
const RECENT_LIMIT: usize = 10;

#[derive(Default)]
pub struct Prefs {
    path: Option<PathBuf>,
    recent: Vec<PathBuf>,
    godot_root: String,
    godot_warned: bool,
    /// `godotExport.<file>` key -> settings JSON.
    tileset: Vec<(String, String)>,
}

impl Prefs {
    /// Loads the file at `path` (missing or damaged files give empty preferences) and saves back to it.
    pub fn at(path: PathBuf) -> Prefs {
        let mut p = Prefs { path: Some(path.clone()), ..Default::default() };
        let Some(Value::Obj(members)) = std::fs::read_to_string(&path).ok().and_then(|t| json::parse(&t).ok()) else {
            return p;
        };
        for (k, v) in &members {
            match (k.as_str(), v) {
                ("recentFiles", Value::Arr(a)) => {
                    p.recent = a.iter().filter_map(|x| x.as_str().map(PathBuf::from)).collect();
                }
                ("godotExport.projectRoot", Value::Str(s)) => p.godot_root = s.clone(),
                ("godotExport.filterWarningShown", Value::Bool(b)) => p.godot_warned = *b,
                ("tilesetExport", Value::Obj(m)) => {
                    p.tileset = m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
                }
                _ => {}
            }
        }
        p
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let recent = self.recent.iter().map(|p| json_str(&p.to_string_lossy())).collect::<Vec<_>>().join(",");
        let tileset = self.tileset.iter().map(|(k, v)| format!("{}:{}", json_str(k), json_str(v))).collect::<Vec<_>>();
        let text = format!(
            "{{\"recentFiles\":[{recent}],\"godotExport.projectRoot\":{},\"godotExport.filterWarningShown\":{},\"tilesetExport\":{{{}}}}}\n",
            json_str(&self.godot_root),
            self.godot_warned,
            tileset.join(",")
        );
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    // MARK: Recent files

    pub fn recent(&self) -> &[PathBuf] {
        &self.recent
    }

    /// Puts `file` first, at most ten (`RecentFiles.add`).
    pub fn add_recent(&mut self, file: &Path) {
        self.recent.retain(|p| p != file);
        self.recent.insert(0, file.to_path_buf());
        self.recent.truncate(RECENT_LIMIT);
        self.save();
    }

    pub fn clear_recent(&mut self) {
        self.recent.clear();
        self.save();
    }

    // MARK: Godot

    /// The last Godot project folder (`godotExport.projectRoot`).
    pub fn godot_root(&self) -> &str {
        &self.godot_root
    }

    pub fn set_godot_root(&mut self, root: &str) {
        if self.godot_root != root {
            self.godot_root = root.to_string();
            self.save();
        }
    }

    /// Whether the "Pixel art looks blurry" hint has been shown (`godotExport.filterWarningShown`).
    pub fn godot_warned(&self) -> bool {
        self.godot_warned
    }

    pub fn set_godot_warned(&mut self) {
        if !self.godot_warned {
            self.godot_warned = true;
            self.save();
        }
    }

    // MARK: TileSet export settings, per document

    fn tileset_key(file: &Path) -> String {
        format!("godotExport.{}", file.canonicalize().unwrap_or_else(|_| file.to_path_buf()).display())
    }

    pub fn tileset_settings(&self, file: Option<&Path>) -> Option<TilesetSettings> {
        let key = Self::tileset_key(file?);
        let json = &self.tileset.iter().find(|(k, _)| *k == key)?.1;
        TilesetSettings::from_json(json)
    }

    pub fn set_tileset_settings(&mut self, file: Option<&Path>, s: &TilesetSettings) {
        let Some(file) = file else { return };
        let key = Self::tileset_key(file);
        self.tileset.retain(|(k, _)| *k != key);
        self.tileset.push((key, s.json()));
        self.save();
    }
}

impl PixmithApp {
    /// Recent files that still exist, newest first (`RecentFiles.existing`).
    pub fn recent_files(&self) -> Vec<PathBuf> {
        self.gamedev.prefs.recent().iter().filter(|p| p.exists()).cloned().collect()
    }

    pub fn add_recent(&mut self, file: &Path) {
        self.gamedev.prefs.add_recent(file);
    }

    /// File > Clear Menu.
    pub fn clear_recent(&mut self) {
        self.gamedev.prefs.clear_recent();
    }

    /// File > Open Recent: opens `file`; with `background` (Cmd-click) the tab opens without being shown.
    pub fn open_recent(&mut self, file: &Path, background: bool) {
        if !background {
            self.open_path(file);
            return;
        }
        let already = self.tabs.iter().any(|t| t.doc.path.as_deref() == Some(file));
        let before = self.tab().doc.raw() as usize;
        self.open_path(file);
        if !already && let Some(i) = self.tabs.iter().position(|t| t.doc.raw() as usize == before) {
            self.active = i;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pixmith-prefs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn recent_files_are_newest_first_unique_and_limited() {
        let mut p = Prefs::default();
        for i in 0..12 {
            p.add_recent(Path::new(&format!("/a/{i}.aseprite")));
        }
        p.add_recent(Path::new("/a/5.aseprite"));
        assert_eq!(p.recent().len(), RECENT_LIMIT);
        assert_eq!(p.recent()[0], Path::new("/a/5.aseprite"));
        assert_eq!(p.recent().iter().filter(|r| **r == Path::new("/a/5.aseprite")).count(), 1);
        p.clear_recent();
        assert!(p.recent().is_empty());
    }

    #[test]
    fn preferences_survive_a_restart() {
        let dir = scratch("restart");
        let file = dir.join("preferences.json");
        {
            let mut p = Prefs::at(file.clone());
            p.add_recent(Path::new("/x/one \"quoted\".aseprite"));
            p.add_recent(Path::new("/x/two.aseprite"));
            p.set_godot_root("/games/p");
            p.set_godot_warned();
            let s = TilesetSettings { output_path: "/games/p/a.tres".into(), mode: 2, ..Default::default() };
            p.set_tileset_settings(Some(Path::new("/x/two.aseprite")), &s);
        }
        let p = Prefs::at(file);
        assert_eq!(p.recent(), [PathBuf::from("/x/two.aseprite"), PathBuf::from("/x/one \"quoted\".aseprite")]);
        assert_eq!(p.godot_root(), "/games/p");
        assert!(p.godot_warned());
        let s = p.tileset_settings(Some(Path::new("/x/two.aseprite"))).expect("remembered");
        assert_eq!((s.output_path.as_str(), s.mode), ("/games/p/a.tres", 2));
        assert!(p.tileset_settings(Some(Path::new("/x/other.aseprite"))).is_none());
        assert!(p.tileset_settings(None).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_damaged_file_gives_empty_preferences() {
        let dir = scratch("damaged");
        let file = dir.join("preferences.json");
        std::fs::write(&file, "{not json").unwrap();
        assert!(Prefs::at(file).recent().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
