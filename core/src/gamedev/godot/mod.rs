//! Godot 4 export: SpriteFrames `.tres` plus atlas PNG(s) and an optional `.tscn`.
//! See docs/SPEC-game.md section 1A. Godot 3 (`format=2`) is not supported.

pub mod scene;
pub mod settings;
pub mod sprite_frames;
pub mod tileset;

use crate::document::{Editor, Sprite};
use crate::ffi::{path_arg, set_error};
use settings::Settings;
use std::collections::HashSet;
use std::ffi::{c_char, CString};
use std::path::{Component, Path, PathBuf};

pub use sprite_frames::{LayerMode, Options, Packing};

/// Lexically resolves `.` and `..` (the path may not exist yet).
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Absolute path with symlinks of the existing part resolved.
fn resolve(p: &Path) -> Result<PathBuf, String> {
    let abs =
        if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().map_err(|e| e.to_string())?.join(p) };
    let abs = normalize(&abs);
    let mut tail = Vec::new();
    let mut cur = abs.as_path();
    loop {
        if let Ok(c) = cur.canonicalize() {
            let mut out = c;
            out.extend(tail.iter().rev());
            return Ok(out);
        }
        match (cur.file_name(), cur.parent()) {
            (Some(n), Some(parent)) => {
                tail.push(n.to_os_string());
                cur = parent;
            }
            _ => return Ok(abs),
        }
    }
}

/// Validates the project root; returns `(canonical tres path, res:// directory)`.
pub fn resolve_paths(project_root: &Path, out_tres: &Path) -> Result<(PathBuf, String), String> {
    if !project_root.join("project.godot").is_file() {
        return Err(format!("{} is not a Godot project (no project.godot found)", project_root.display()));
    }
    let root = resolve(project_root)?;
    let tres = resolve(out_tres)?;
    if tres.extension().and_then(|e| e.to_str()) != Some("tres") {
        return Err("the output file must end in .tres".into());
    }
    let rel_dir = tres
        .parent()
        .and_then(|d| d.strip_prefix(&root).ok())
        .ok_or_else(|| format!("{} is outside the Godot project {}", out_tres.display(), project_root.display()))?;
    let mut res = String::from("res:/");
    for c in rel_dir.components() {
        res.push('/');
        res.push_str(c.as_os_str().to_str().ok_or("the output path is not valid UTF-8")?);
    }
    Ok((tres, res))
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".pixmith-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, data).and_then(|_| std::fs::rename(&tmp, path)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// File name safe text: path separators and control characters become `_`.
fn file_safe(s: &str) -> String {
    let t: String =
        s.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') || c.is_control() { '_' } else { c }).collect();
    let t = t.trim().trim_start_matches('.').to_string();
    if t.is_empty() {
        "_".into()
    } else {
        t
    }
}

/// Fills `{name}` and `{layer}` in the template. Each-layer mode always includes the layer.
fn stem_from_template(template: &str, name: &str, layer: Option<&str>, each: bool) -> String {
    let mut t = if template.trim().is_empty() { "{name}".to_string() } else { template.to_string() };
    if each && !t.contains("{layer}") {
        t.push_str("_{layer}");
    }
    file_safe(&t.replace("{name}", name).replace("{layer}", layer.unwrap_or("all")))
}

/// A copy of the sprite where only the layers accepted by `keep` (and all groups) are visible.
fn restrict_layers(s: &Sprite, keep: impl Fn(usize) -> bool) -> Sprite {
    let mut c = s.clone();
    for i in 0..c.layers.len() {
        c.layers[i].visible = c.layers[i].is_group() || keep(i);
    }
    c
}

fn with_ancestors(s: &Sprite, i: usize, f: impl Fn(usize) -> bool) -> bool {
    let mut cur = Some(i);
    while let Some(j) = cur {
        if f(j) {
            return true;
        }
        cur = s.parent_layer(j);
    }
    false
}

/// `(file stem, sprite with the right layers shown)` per SpriteFrames to write.
fn variants(s: &Sprite, name: &str, o: &Options) -> Result<Vec<(String, Sprite)>, String> {
    let mut out: Vec<(String, Sprite)> = Vec::new();
    match &o.layers {
        LayerMode::Visible => out.push((stem_from_template(&o.name_template, name, None, false), s.clone())),
        LayerMode::Selected(names) => {
            if names.is_empty() {
                return Err("no layers are selected for export".into());
            }
            let c = restrict_layers(s, |i| with_ancestors(s, i, |j| names.contains(&s.layers[j].name)));
            out.push((stem_from_template(&o.name_template, name, None, false), c));
        }
        LayerMode::Each => {
            for i in 0..s.layers.len() {
                let has_cel = s.cels.iter().any(|f| f.get(i).is_some_and(|c| c.is_some()));
                if s.layers[i].is_group() || s.layers[i].is_reference() || !s.is_layer_shown(i) || !has_cel {
                    continue;
                }
                let stem = stem_from_template(&o.name_template, name, Some(&s.layers[i].name), true);
                out.push((stem, restrict_layers(s, |j| j == i)));
            }
            if out.is_empty() {
                return Err("there are no visible layers with content to export".into());
            }
        }
    }
    // Same layer name twice (or templates that collapse): keep stems unique.
    let mut seen = HashSet::new();
    for (stem, _) in &mut out {
        let base = stem.clone();
        let mut n = 2;
        while !seen.insert(stem.clone()) {
            *stem = format!("{base}_{n}");
            n += 1;
        }
    }
    Ok(out)
}

pub struct OutFile {
    pub path: PathBuf,
    pub data: Vec<u8>,
}

pub struct Plan {
    /// PNGs first, then `.tres`, then `.tscn`, per SpriteFrames.
    pub files: Vec<OutFile>,
    pub warnings: Vec<String>,
}

/// Everything an export would write, without touching the disk.
pub fn plan(s: &Sprite, project_root: &Path, out_tres: &Path, o: &Options) -> Result<Plan, String> {
    let (tres_path, res_dir) = resolve_paths(project_root, out_tres)?;
    let name = tres_path.file_stem().and_then(|n| n.to_str()).ok_or("invalid output file name")?.to_string();
    let dir = tres_path.parent().ok_or("invalid output path")?.to_path_buf();
    let root = resolve(project_root)?;
    let mut files = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut add_warning = |w: String| {
        if !warnings.contains(&w) {
            warnings.push(w);
        }
    };
    let mut written: HashSet<PathBuf> = HashSet::new();
    let mut stale_candidates: Vec<PathBuf> = Vec::new();
    for (stem, sprite) in variants(s, &name, o)? {
        let tres_file = dir.join(format!("{stem}.tres"));
        let previous = std::fs::read_to_string(&tres_file).ok();
        let target = sprite_frames::Target { name: &stem, res_dir: &res_dir, previous: previous.as_deref() };
        let built = sprite_frames::build(&sprite, &target, o)?;
        built.warnings.into_iter().for_each(&mut add_warning);
        for (file, data) in built.pages {
            let path = dir.join(&file);
            if let Ok(old) = std::fs::read(&path) {
                if old != data && path.with_extension("png.import").exists() {
                    add_warning(format!(
                        "{file} changed: Godot reimports it when the editor window regains focus; until then its .godot/imported cache is stale."
                    ));
                }
            } else if !root.join(".godot").exists() {
                add_warning(
                    "this project has not been opened in Godot yet; Godot imports the new textures on first open."
                        .into(),
                );
            } else {
                add_warning(format!("{file} is new: Godot imports it when the editor window regains focus."));
            }
            written.insert(path.clone());
            files.push(OutFile { path, data });
        }
        stale_candidates.push(dir.join(format!("{stem}.png")));
        stale_candidates.push(dir.join(format!("{stem}_0.png")));
        let tres_res = format!("{res_dir}/{stem}.tres");
        let scene = if o.emit_tscn {
            let scene_file = dir.join(format!("{stem}.tscn"));
            if scene_file.exists() && !o.overwrite_tscn {
                add_warning(format!("{stem}.tscn already exists and was left untouched."));
                None
            } else {
                let prev = std::fs::read_to_string(&scene_file).ok();
                let text = scene::tscn(&stem, &tres_res, &built.tres, &built.first_animation, prev.as_deref());
                Some(OutFile { path: scene_file, data: text.into_bytes() })
            }
        } else {
            None
        };
        written.insert(tres_file.clone());
        files.push(OutFile { path: tres_file, data: built.tres.into_bytes() });
        files.extend(scene);
    }
    for p in stale_candidates {
        if p.exists() && !written.contains(&p) {
            add_warning(format!(
                "{} is left over from an earlier export and was not touched.",
                p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            ));
        }
    }
    Ok(Plan { files, warnings })
}

/// Writes the `.tres`, atlas PNG(s) and optional `.tscn`. Returns warnings.
pub fn export_sprite_frames(
    s: &Sprite,
    project_root: &Path,
    out_tres: &Path,
    opts: &Options,
) -> Result<Vec<String>, String> {
    let plan = plan(s, project_root, out_tres, opts)?;
    for f in &plan.files {
        if let Some(dir) = f.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        write_atomic(&f.path, &f.data)?;
    }
    Ok(plan.warnings)
}

fn to_cstring(s: String) -> *mut c_char {
    CString::new(s.replace('\0', "")).unwrap_or_default().into_raw()
}

unsafe fn out_str(p: *mut *mut c_char, s: String) {
    if !p.is_null() {
        *p = to_cstring(s);
    }
}

unsafe fn clear_out(p: *mut *mut c_char) {
    if !p.is_null() {
        *p = std::ptr::null_mut();
    }
}

/// Exports `doc` as SpriteFrames with default atlas options plus the given ones. On success
/// `*out_warnings` (if non-null) receives newline separated warnings, to release with
/// `pixmith_string_free`. On failure see `pixmith_last_error`.
///
/// # Safety
/// Pointers must be null or valid; `out_warnings` must be null or writable.
#[no_mangle]
pub unsafe extern "C" fn pixmith_export_godot_sprite_frames(
    doc: *const Editor,
    project_root: *const c_char,
    out_tres_path: *const c_char,
    padding: u32,
    extrude: bool,
    trim: bool,
    max_texture: u32,
    out_warnings: *mut *mut c_char,
) -> bool {
    crate::ffi::guarded("pixmith_export_godot_sprite_frames", std::ptr::null_mut(), || {
        clear_out(out_warnings);
        let (Some(e), Some(root), Some(tres)) = (doc.as_ref(), path_arg(project_root), path_arg(out_tres_path)) else {
            set_error("invalid argument");
            return false;
        };
        let opts = Options { padding: padding.min(64), extrude, trim, max_texture, ..Default::default() };
        match export_sprite_frames(&e.sprite, Path::new(&root), Path::new(&tres), &opts) {
            Ok(w) => {
                out_str(out_warnings, w.join("\n"));
                true
            }
            Err(msg) => {
                set_error(msg);
                false
            }
        }
    })
}

/// Full-option export from settings JSON (see `settings.rs`; includes `root` and `tres`).
///
/// # Safety
/// Pointers must be null or valid; `out_warnings` must be null or writable.
#[no_mangle]
pub unsafe extern "C" fn pixmith_export_godot(
    doc: *const Editor,
    settings_json: *const c_char,
    out_warnings: *mut *mut c_char,
) -> bool {
    crate::ffi::guarded("pixmith_export_godot", std::ptr::null_mut(), || {
        clear_out(out_warnings);
        let (Some(e), Some(json)) = (doc.as_ref(), path_arg(settings_json)) else {
            set_error("invalid argument");
            return false;
        };
        let result = Settings::from_json(&json)
            .and_then(|st| export_sprite_frames(&e.sprite, Path::new(&st.root), Path::new(&st.tres), &st.options));
        match result {
            Ok(w) => {
                out_str(out_warnings, w.join("\n"));
                true
            }
            Err(msg) => {
                set_error(msg);
                false
            }
        }
    })
}

/// Builds the export without writing it. Writes the first atlas page to `png_out_path`,
/// returns the first `.tres` text in `*out_tres_text`, the files that would be written
/// (newline separated absolute paths) in `*out_files` and the warnings in `*out_warnings`.
/// Each out pointer may be null.
///
/// # Safety
/// Pointers must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn pixmith_godot_preview(
    doc: *const Editor,
    settings_json: *const c_char,
    png_out_path: *const c_char,
    out_tres_text: *mut *mut c_char,
    out_files: *mut *mut c_char,
    out_warnings: *mut *mut c_char,
) -> bool {
    crate::ffi::guarded("pixmith_godot_preview", std::ptr::null_mut(), || {
        clear_out(out_tres_text);
        clear_out(out_files);
        clear_out(out_warnings);
        let (Some(e), Some(json)) = (doc.as_ref(), path_arg(settings_json)) else {
            set_error("invalid argument");
            return false;
        };
        let result = Settings::from_json(&json)
            .and_then(|st| plan(&e.sprite, Path::new(&st.root), Path::new(&st.tres), &st.options));
        match result {
            Ok(p) => {
                let ext = |f: &OutFile, e: &str| f.path.extension().is_some_and(|x| x == e);
                if let (Some(png), Some(path)) = (p.files.iter().find(|f| ext(f, "png")), path_arg(png_out_path)) {
                    let _ = std::fs::write(path, &png.data);
                }
                if let Some(t) = p.files.iter().find(|f| ext(f, "tres")) {
                    out_str(out_tres_text, String::from_utf8_lossy(&t.data).into_owned());
                }
                out_str(
                    out_files,
                    p.files.iter().map(|f| f.path.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n"),
                );
                out_str(out_warnings, p.warnings.join("\n"));
                true
            }
            Err(msg) => {
                set_error(msg);
                false
            }
        }
    })
}

/// Settings JSON saved in the document, or null. Release with `pixmith_string_free`.
///
/// # Safety
/// `doc` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn pixmith_godot_settings_get(doc: *const Editor) -> *mut c_char {
    crate::ffi::guarded("pixmith_godot_settings_get", std::ptr::null_mut(), || {
        doc.as_ref().and_then(|e| settings::load(&e.sprite)).map_or(std::ptr::null_mut(), to_cstring)
    })
}

/// Saves settings JSON in the document's user data (written with the next save).
///
/// # Safety
/// Pointers must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn pixmith_godot_settings_set(doc: *mut Editor, settings_json: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_godot_settings_set", doc, || {
        let (Some(e), Some(json)) = (doc.as_mut(), path_arg(settings_json)) else {
            set_error("invalid argument");
            return false;
        };
        match Settings::from_json(&json) {
            Ok(st) => {
                settings::store(&mut e.sprite, &st.to_json());
                true
            }
            Err(msg) => {
                set_error(msg);
                false
            }
        }
    })
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_ext;
