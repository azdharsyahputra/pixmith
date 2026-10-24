//! Loads the scripts that enabled extensions contribute as plugins (`PluginHost.swift`): once per script when the
//! extension is enabled, stopped (its `exit` runs) when it is disabled, uninstalled or updated. Only starts and stops
//! sessions; their messages (commands, dialogs) are handled by the Lua dialog host like any session's.

use crate::app::PixmithApp;
use pixmith_core::extensions as core;
use std::collections::HashMap;

/// Script key (path and modification time) to its session.
#[derive(Default)]
pub struct Plugins {
    /// Off in tests and headless drivers, so they never start the user's plugins; `PixmithApp::new` turns it on.
    pub enabled: bool,
    loaded: HashMap<String, u64>,
    seen: u64,
}

fn key_of(path: &std::path::Path) -> String {
    let modified = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    format!("{}@{modified}", path.display())
}

/// Makes the loaded plugins match the enabled extensions' scripts. Cheap when nothing changed.
pub fn sync(app: &mut PixmithApp) {
    let mut state = std::mem::take(&mut app.extensions.plugins);
    if state.enabled {
        let generation = core::generation();
        if state.seen != generation {
            state.seen = generation;
            let wanted: Vec<_> = core::scripts().into_iter().map(|(ext, s)| (key_of(&s.path), ext, s)).collect();
            let stale: Vec<String> =
                state.loaded.keys().filter(|k| !wanted.iter().any(|(w, ..)| w == *k)).cloned().collect();
            for key in stale {
                if let Some(session) = state.loaded.remove(&key) {
                    app.lua_unload_plugin(session);
                }
            }
            for (key, ext, script) in wanted {
                if state.loaded.contains_key(&key) {
                    continue;
                }
                let Ok(source) = std::fs::read_to_string(&script.path) else { continue };
                if let Some(parent) = script.prefs.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let name = format!("{ext}/{}", script.name);
                let (dir, prefs) = (script.dir.to_string_lossy(), script.prefs.to_string_lossy());
                match app.lua_load_plugin(&source, &name, Some(&dir), Some(&prefs)) {
                    Ok(id) => {
                        state.loaded.insert(key, id);
                    }
                    Err(e) => app.error = Some(format!("Could not load {name}: {e}")),
                }
            }
        }
    }
    app.extensions.plugins = state;
}

impl Plugins {
    pub fn session_count(&self) -> usize {
        self.loaded.len()
    }
}
