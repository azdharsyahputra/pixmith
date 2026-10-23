//! Where the app keeps its files (`AppSupport` in `FileActions.swift`): the script folder, the crash recovery
//! folder and the small preferences file.
//!
//! `PIXMITH_SUPPORT_DIR` replaces the platform's application data folder, so tests and QA runs keep their
//! autosave copies and scripts out of the real one. Inside it the app uses `Pixmith/`.

use std::path::{Path, PathBuf};

/// The platform's application data folder: `~/Library/Application Support` on macOS, `%APPDATA%` on Windows,
/// `$XDG_DATA_HOME` (else `~/.local/share`) elsewhere.
pub fn platform_data_dir() -> PathBuf {
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

/// The base folder: `PIXMITH_SUPPORT_DIR` when set and not empty, else the platform's.
pub fn support_dir() -> PathBuf {
    match std::env::var_os("PIXMITH_SUPPORT_DIR").filter(|v| !v.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => platform_data_dir(),
    }
}

/// The app's own folder inside a base: `Pixmith` (`pixmith` on Linux, which uses lower case folders).
pub fn app_dir_in(base: &Path) -> PathBuf {
    base.join(if cfg!(any(target_os = "macos", windows)) { "Pixmith" } else { "pixmith" })
}

pub fn app_dir() -> PathBuf {
    app_dir_in(&support_dir())
}

pub fn scripts_dir() -> PathBuf {
    app_dir().join("Scripts")
}

pub fn recovery_dir() -> PathBuf {
    app_dir().join("Recovery")
}

pub fn prefs_file() -> PathBuf {
    app_dir().join("preferences.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_dir_sits_inside_the_base() {
        let base = Path::new("/tmp/base");
        let dir = app_dir_in(base);
        assert!(dir.starts_with(base));
        assert!(dir.ends_with(if cfg!(any(target_os = "macos", windows)) { "Pixmith" } else { "pixmith" }));
    }
}
