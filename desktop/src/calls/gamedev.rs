//! Core calls for the game-development features: tweening, the Godot exporters, Lua scripts and the
//! background snapshots behind autosave. One-to-one with the `SpriteModel` extensions in `TweenView.swift`,
//! `GodotExport.swift`, `GodotTilesetExport.swift` and `Scripts.swift`. All `unsafe` stays here.
//!
//! Every mutating method names its undo step like the Swift app and marks the document changed
//! (`pixmith_touch`); the caller then calls `DocTab::changed`.

use crate::doc::{Doc, last_error, take_string};
use crate::ui::gamedev::{
    GodotPreview, ScriptOutcome, TilesetSettings, TweenEase, TweenKey, TweenProp, TweenTransform,
};
use pixmith_core::Editor;
use pixmith_core::document::tilemap as tilemap_ffi;
use pixmith_core::ffi::document as core;
use pixmith_core::gamedev::godot as godot_ffi;
use pixmith_core::gamedev::godot::tileset as tileset_ffi;
use pixmith_core::gamedev::tween as tween_ffi;
use pixmith_core::history;
use pixmith_core::{jobs, scripting};
use std::ffi::{CString, c_char};
use std::path::Path;
use std::ptr::null_mut;

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

/// A snapshot document (`pixmith_snapshot`): an independent copy of a sprite that one worker thread may use
/// while the UI keeps the real document. Freed on drop.
pub struct Snapshot(*mut Editor);

// SAFETY: a snapshot shares nothing mutable with the document it was copied from, and whoever holds it is its
// only user (docs/ARCHITECTURE.md, Threading). It moves to one worker thread and is dropped there or back here.
unsafe impl Send for Snapshot {}

impl Snapshot {
    /// Writes the snapshot as `.aseprite` (crash safe: through a temporary file).
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let c = cstr(&path.to_string_lossy());
        // SAFETY: `self.0` is a live snapshot; `c` outlives the call.
        if unsafe { core::pixmith_save(self.0, c.as_ptr()) } { Ok(()) } else { Err(last_error()) }
    }

    /// Runs a Lua script on the snapshot (`pixmith_run_script_at`), with 0-based active layer and frame. Call on
    /// the worker thread: the error text is read from that thread's last error.
    pub fn run_script(&self, source: &str, layer: u32, frame: u32) -> ScriptOutcome {
        let src = cstr(source);
        let mut out: *mut c_char = null_mut();
        // SAFETY: `self.0` is live, `src` is NUL terminated and `out` is writable.
        let ok = unsafe { scripting::pixmith_run_script_at(self.0, src.as_ptr(), layer, frame, &mut out) };
        // SAFETY: `out` is null or a string the core allocated.
        let messages = unsafe { take_string(out) };
        ScriptOutcome { ok, messages, failure: if ok { String::new() } else { last_error() } }
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `pixmith_snapshot` and is freed exactly once.
        unsafe { core::pixmith_free(self.0) }
    }
}

/// Asks the running script to stop (`true`) or clears the request before a new run (`false`).
pub fn script_cancel(cancel: bool) {
    scripting::pixmith_script_cancel(cancel);
}

/// The example scripts that ship inside the core: `(file name, source)`.
pub fn example_scripts() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for i in 0..scripting::pixmith_example_script_count() {
        // SAFETY: both strings are null or allocated by the core.
        let (name, source) = unsafe {
            (
                take_string(scripting::pixmith_example_script_name(i)),
                take_string(scripting::pixmith_example_script_source(i)),
            )
        };
        if !name.is_empty() {
            out.push((name, source));
        }
    }
    out
}

impl Doc {
    // SAFETY note for this file: `self.raw()` is a live document owned by `self`, every string passed is a
    // NUL-terminated `CString` that outlives the call and out pointers are valid for the sizes documented.

    // MARK: Snapshots and jobs

    /// A copy of the sprite and selection for background work, or `None` when the core refuses.
    pub fn snapshot(&self) -> Option<Snapshot> {
        // SAFETY: `self.raw()` is live.
        let p = unsafe { jobs::pixmith_snapshot(self.raw()) };
        (!p.is_null()).then_some(Snapshot(p))
    }

    /// Takes the result of a job as one undo step named like the job's own step; false when nothing changed.
    pub fn apply_job(&mut self, job: &Snapshot) -> bool {
        // SAFETY: both documents are live.
        unsafe { jobs::pixmith_job_apply(self.raw(), job.0) }
    }

    // MARK: Tween

    /// Keys of one property track, in frame order (`SpriteModel.tweenKeys`).
    pub fn tween_keys(&self, layer: u32, prop: TweenProp) -> Vec<TweenKey> {
        // SAFETY: as above.
        let n = unsafe { tween_ffi::pixmith_tween_key_count(self.raw(), layer, prop.code()) };
        let mut out = Vec::new();
        let mut buf = [0f32; 7];
        for i in 0..n {
            // SAFETY: `buf` holds the 7 floats the call writes.
            if unsafe { tween_ffi::pixmith_tween_key_at(self.raw(), layer, prop.code(), i, buf.as_mut_ptr()) } {
                out.push(TweenKey {
                    frame: buf[0] as u32,
                    value: buf[1],
                    ease: TweenEase::from_code(buf[2] as u32),
                    bezier: [buf[3], buf[4], buf[5], buf[6]],
                });
            }
        }
        out
    }

    pub fn tween_value(&self, layer: u32, prop: TweenProp, frame: u32) -> f32 {
        unsafe { tween_ffi::pixmith_tween_value_at(self.raw(), layer, prop.code(), frame) }
    }

    pub fn tween_set_key(
        &mut self,
        layer: u32,
        prop: TweenProp,
        frame: u32,
        value: f32,
        ease: TweenEase,
        bezier: [f32; 4],
    ) -> bool {
        let ok = unsafe {
            tween_ffi::pixmith_tween_set_key(
                self.raw(),
                layer,
                prop.code(),
                frame,
                value,
                ease.code(),
                bezier[0],
                bezier[1],
                bezier[2],
                bezier[3],
            )
        };
        self.after_tween(ok, "Set Tween Key")
    }

    pub fn tween_set_ease(
        &mut self,
        layer: u32,
        prop: TweenProp,
        frame: u32,
        ease: TweenEase,
        bezier: [f32; 4],
    ) -> bool {
        let ok = unsafe {
            tween_ffi::pixmith_tween_set_ease(
                self.raw(),
                layer,
                prop.code(),
                frame,
                ease.code(),
                bezier[0],
                bezier[1],
                bezier[2],
                bezier[3],
            )
        };
        self.after_tween(ok, "Change Tween Curve")
    }

    pub fn tween_remove_key(&mut self, layer: u32, prop: TweenProp, frame: u32) -> bool {
        let ok = unsafe { tween_ffi::pixmith_tween_remove_key(self.raw(), layer, prop.code(), frame) };
        self.after_tween(ok, "Remove Tween Key")
    }

    pub fn tween_move_key(&mut self, layer: u32, prop: TweenProp, from: u32, to: u32) -> bool {
        let ok = unsafe { tween_ffi::pixmith_tween_move_key(self.raw(), layer, prop.code(), from, to) };
        self.after_tween(ok, "Move Tween Key")
    }

    /// Copies the keys of frames `from..=to` (all properties); returns how many.
    pub fn tween_copy_keys(&self, layer: u32, from: u32, to: u32) -> u32 {
        unsafe { tween_ffi::pixmith_tween_copy_keys(self.raw(), layer, from, to) }
    }

    pub fn tween_paste_keys(&mut self, layer: u32, at: u32) -> bool {
        let ok = unsafe { tween_ffi::pixmith_tween_paste_keys(self.raw(), layer, at) } > 0;
        self.after_tween(ok, "Paste Tween Keys")
    }

    /// `mode` 0 all keys, 1 frame range, 2 current frame (`from`).
    pub fn tween_bake(
        &mut self,
        layer: u32,
        mode: u32,
        from: u32,
        to: u32,
        new_layer: bool,
        keep_source: bool,
    ) -> bool {
        let r = unsafe { tween_ffi::pixmith_tween_bake_ex(self.raw(), layer, mode, from, to, new_layer, keep_source) };
        self.after_tween(r >= 0, if keep_source { "Bake Tween (keep source)" } else { "Bake Tween" })
    }

    pub fn tween_bake_all(&mut self, keep_source: bool, new_layer: bool) -> bool {
        let ok = unsafe { tween_ffi::pixmith_tween_bake_all(self.raw(), keep_source, new_layer) } > 0;
        self.after_tween(ok, "Bake All Tweens")
    }

    /// The effective pivot of a layer.
    pub fn tween_pivot(&self, layer: u32) -> Option<(f32, f32)> {
        let mut p = [0f32; 2];
        unsafe { tween_ffi::pixmith_tween_get_pivot(self.raw(), layer, p.as_mut_ptr()) }.then_some((p[0], p[1]))
    }

    pub fn tween_set_pivot(&mut self, layer: u32, x: f32, y: f32, custom: bool) -> bool {
        let ok = unsafe { tween_ffi::pixmith_tween_set_pivot(self.raw(), layer, x, y, custom) };
        self.after_tween(ok, "Set Tween Pivot")
    }

    /// Flag 0 RotSprite, 1 snap scale, 2 ignore opacity.
    pub fn tween_flag(&self, layer: u32, flag: u32) -> bool {
        let o = unsafe { tween_ffi::pixmith_tween_options(self.raw(), layer) };
        [o & 2 != 0, o & 8 != 0, o & 16 != 0].get(flag as usize).copied().unwrap_or(false)
    }

    pub fn tween_set_flag(&mut self, layer: u32, flag: u32, on: bool) -> bool {
        let ok = unsafe { tween_ffi::pixmith_tween_set_flag(self.raw(), layer, flag, on) };
        self.after_tween(ok, "Tween Option")
    }

    pub fn tween_warning(&self, layer: u32) -> Option<String> {
        // SAFETY: null or a string the core allocated.
        let s = unsafe { take_string(tween_ffi::pixmith_tween_warning_text(self.raw(), layer)) };
        (!s.is_empty()).then_some(s)
    }

    /// First and last key frame of the layer's tween.
    pub fn tween_range(&self, layer: u32) -> Option<(u32, u32)> {
        let mut r = [0u32; 2];
        unsafe { tween_ffi::pixmith_tween_range(self.raw(), layer, r.as_mut_ptr()) }.then_some((r[0], r[1]))
    }

    /// Transform at a frame: x, y, rotation, scale x/y, opacity and the pivot.
    pub fn tween_transform(&self, layer: u32, frame: u32) -> Option<TweenTransform> {
        let mut t = [0f32; 8];
        unsafe { tween_ffi::pixmith_tween_transform_at(self.raw(), layer, frame, t.as_mut_ptr()) }
            .then_some(TweenTransform(t))
    }

    /// Bounds of the source content: x0, y0, x1, y1.
    pub fn tween_source_bounds(&self, layer: u32) -> Option<[f32; 4]> {
        let mut b = [0f32; 4];
        unsafe { tween_ffi::pixmith_tween_source_bounds(self.raw(), layer, b.as_mut_ptr()) }.then_some(b)
    }

    /// Sets the keys of the properties in `mask` (bit i = property i) at one frame. Does not label the step: the
    /// canvas drag wraps its calls in a transaction and labels that.
    pub fn tween_set_keys_at(&mut self, layer: u32, frame: u32, mask: u32, values: [f32; 6]) -> bool {
        let ok = unsafe { tween_ffi::pixmith_tween_set_keys_at(self.raw(), layer, frame, mask, values.as_ptr(), 0) };
        if ok {
            self.touch();
        }
        ok
    }

    /// Pivot drag on the canvas: changes the pivot without labeling (the drag's transaction does that).
    pub fn tween_set_pivot_quiet(&mut self, layer: u32, x: f32, y: f32) -> bool {
        let ok = unsafe { tween_ffi::pixmith_tween_set_pivot(self.raw(), layer, x, y, true) };
        if ok {
            self.touch();
        }
        ok
    }

    pub fn begin_transaction(&mut self) {
        unsafe { history::pixmith_begin_transaction(self.raw()) }
    }

    pub fn end_transaction(&mut self) {
        unsafe { history::pixmith_end_transaction(self.raw()) }
    }

    fn after_tween(&mut self, ok: bool, label: &str) -> bool {
        if ok {
            self.label_last_step(label);
            self.touch();
        }
        ok
    }

    // MARK: Godot SpriteFrames

    /// Exports SpriteFrames from settings JSON; returns the warnings.
    pub fn godot_export(&self, json: &str) -> Result<Vec<String>, String> {
        let j = cstr(json);
        let mut w: *mut c_char = null_mut();
        // SAFETY: `w` is writable and is freed by `take_string` below.
        let ok = unsafe { godot_ffi::pixmith_export_godot(self.raw(), j.as_ptr(), &mut w) };
        let warnings = unsafe { take_string(w) };
        if ok { Ok(lines(&warnings)) } else { Err(last_error()) }
    }

    /// Builds the export without writing it; the first atlas page goes to `png_out`.
    pub fn godot_preview(&self, json: &str, png_out: &Path) -> Result<GodotPreview, String> {
        let (j, p) = (cstr(json), cstr(&png_out.to_string_lossy()));
        let (mut t, mut f, mut w): (*mut c_char, *mut c_char, *mut c_char) = (null_mut(), null_mut(), null_mut());
        // SAFETY: the three out pointers are writable and freed by `take_string`.
        let ok =
            unsafe { godot_ffi::pixmith_godot_preview(self.raw(), j.as_ptr(), p.as_ptr(), &mut t, &mut f, &mut w) };
        let (tres, files, warnings) = unsafe { (take_string(t), take_string(f), take_string(w)) };
        if ok { Ok(GodotPreview { tres, files: lines(&files), warnings: lines(&warnings) }) } else { Err(last_error()) }
    }

    /// The export settings saved in the document, as JSON.
    pub fn godot_settings(&self) -> Option<String> {
        // SAFETY: null or a string the core allocated.
        let s = unsafe { godot_ffi::pixmith_godot_settings_get(self.raw()) };
        (!s.is_null()).then(|| unsafe { take_string(s) })
    }

    /// Stores the settings in the document; marks it modified only when they changed (`GodotCore.save`).
    pub fn set_godot_settings(&mut self, json: &str) -> bool {
        let before = self.godot_settings();
        let j = cstr(json);
        if !unsafe { godot_ffi::pixmith_godot_settings_set(self.raw(), j.as_ptr()) } {
            return false;
        }
        if before != self.godot_settings() {
            self.touch();
        }
        true
    }

    // MARK: Godot TileSet

    pub fn tileset_count(&self) -> u32 {
        unsafe { tilemap_ffi::pixmith_tileset_count(self.raw()) }
    }

    pub fn tileset_name(&self, ts: u32) -> String {
        unsafe { take_string(tilemap_ffi::pixmith_tileset_name(self.raw(), ts)) }
    }

    /// Index of the tileset a tilemap layer uses.
    pub fn layer_tileset(&self, layer: u32) -> Option<u32> {
        let i = unsafe { tilemap_ffi::pixmith_layer_tileset(self.raw(), layer) };
        (i >= 0).then_some(i as u32)
    }

    fn with_tileset_options<T>(s: &TilesetSettings, f: impl FnOnce(&tileset_ffi::PixmithGodotOptions) -> T) -> T {
        let name = cstr(&s.terrain_name);
        let c = s.color;
        let o = tileset_ffi::PixmithGodotOptions {
            mode: s.mode,
            collision: s.collision,
            margin: s.margin,
            separation: s.separation,
            extrude: s.extrude,
            max_texture: s.max_texture,
            write_scene: s.write_scene,
            terrain_color: [c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32],
            terrain_name: name.as_ptr(),
        };
        f(&o)
    }

    /// Exports a tileset; returns the warnings text ("" when clean) (`runGodotExport`).
    pub fn tileset_export(&self, s: &TilesetSettings) -> Result<String, String> {
        let (r, o) = (cstr(&s.project_root), cstr(&s.output_path));
        let ok = Doc::with_tileset_options(s, |opts| unsafe {
            tileset_ffi::pixmith_export_godot_tileset_ex(self.raw(), s.tileset, r.as_ptr(), o.as_ptr(), opts)
        });
        if !ok {
            return Err(last_error());
        }
        Ok(unsafe { take_string(tileset_ffi::pixmith_godot_tileset_warning()) })
    }

    /// What an export would write, as text; an error comes back as the text, like Swift.
    pub fn tileset_preview(&self, s: &TilesetSettings) -> String {
        let (r, o) = (cstr(&s.project_root), cstr(&s.output_path));
        let p = Doc::with_tileset_options(s, |opts| unsafe {
            tileset_ffi::pixmith_godot_tileset_preview(self.raw(), s.tileset, r.as_ptr(), o.as_ptr(), opts)
        });
        if p.is_null() { last_error() } else { unsafe { take_string(p) } }
    }
}

fn lines(s: &str) -> Vec<String> {
    s.split('\n').filter(|l| !l.is_empty()).map(str::to_string).collect()
}
