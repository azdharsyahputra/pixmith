//! Game-development features: the Tween panel, the Godot exporters, Lua scripts, and autosave with crash
//! recovery plus Open Recent. One module per Swift file: `tween` and `tween_canvas` (`TweenView.swift`), `godot`
//! (`GodotExport.swift`), `tileset` (`GodotTilesetExport.swift`), `scripts` (`Scripts.swift`), `autosave`
//! (`Autosaver` in `FileActions.swift`), `jobs` (`ProgressSheet.swift`), `prefs` (`UserDefaults`) and `paths`
//! (`AppSupport`).
//!
//! The menu code calls the public methods of [`PixmithApp`] at the end of this file: `toggle_tween_panel`,
//! `open_godot_export`, `reexport_godot`, `open_tileset_export`, `reexport_tileset`, `run_script_dialog`,
//! `run_script_at`, `scripts`, `install_example_scripts`, `open_scripts_folder`, `recent_files`, `open_recent`
//! and `clear_recent`.

pub mod autosave;
pub mod form;
pub mod godot;
pub mod jobs;
pub mod paths;
pub mod prefs;
pub mod scripts;
pub mod tileset;
pub mod tween;
pub mod tween_canvas;

use crate::app::PixmithApp;
use eframe::egui;
use pixmith_core::formats::json::{self, Value};
use pixmith_core::formats::sheet::json_str;
use std::path::PathBuf;

pub use godot::GodotSheet;
pub use tileset::TilesetSheet;

// MARK: Tween types

/// Properties of a tween track, in the order of the core's codes (`TweenProp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TweenProp {
    PosX,
    PosY,
    Rotation,
    ScaleX,
    ScaleY,
    Opacity,
}

impl TweenProp {
    pub const ALL: [TweenProp; 6] = [
        TweenProp::PosX,
        TweenProp::PosY,
        TweenProp::Rotation,
        TweenProp::ScaleX,
        TweenProp::ScaleY,
        TweenProp::Opacity,
    ];

    pub fn code(self) -> u32 {
        self as u32
    }

    pub fn title(self) -> &'static str {
        match self {
            TweenProp::PosX => "Position X",
            TweenProp::PosY => "Position Y",
            TweenProp::Rotation => "Rotation (deg)",
            TweenProp::ScaleX => "Scale X",
            TweenProp::ScaleY => "Scale Y",
            TweenProp::Opacity => "Opacity (0-1)",
        }
    }

    /// The row label of the dope sheet.
    pub fn short(self) -> &'static str {
        match self {
            TweenProp::PosX => "X",
            TweenProp::PosY => "Y",
            TweenProp::Rotation => "Rot",
            TweenProp::ScaleX => "SX",
            TweenProp::ScaleY => "SY",
            TweenProp::Opacity => "Op",
        }
    }
}

/// How a key moves to the next one, in the order of the core's codes (`TweenEase`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TweenEase {
    Linear,
    Hold,
    InQuad,
    OutQuad,
    InOutQuad,
    InOutCubic,
    OutBack,
    OutBounce,
    Bezier,
}

impl TweenEase {
    pub const ALL: [TweenEase; 9] = [
        TweenEase::Linear,
        TweenEase::Hold,
        TweenEase::InQuad,
        TweenEase::OutQuad,
        TweenEase::InOutQuad,
        TweenEase::InOutCubic,
        TweenEase::OutBack,
        TweenEase::OutBounce,
        TweenEase::Bezier,
    ];

    pub fn code(self) -> u32 {
        self as u32
    }

    pub fn from_code(code: u32) -> TweenEase {
        TweenEase::ALL.get(code as usize).copied().unwrap_or(TweenEase::Linear)
    }

    pub fn title(self) -> &'static str {
        match self {
            TweenEase::Linear => "Linear",
            TweenEase::Hold => "Hold",
            TweenEase::InQuad => "In Quad",
            TweenEase::OutQuad => "Out Quad",
            TweenEase::InOutQuad => "In-Out Quad",
            TweenEase::InOutCubic => "In-Out Cubic",
            TweenEase::OutBack => "Out Back",
            TweenEase::OutBounce => "Out Bounce",
            TweenEase::Bezier => "Bezier",
        }
    }
}

/// One key of a track (`TweenKeyInfo`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TweenKey {
    pub frame: u32,
    pub value: f32,
    pub ease: TweenEase,
    pub bezier: [f32; 4],
}

/// The transform of a layer at a frame: x, y, rotation, scale x, scale y, opacity, pivot x, pivot y.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TweenTransform(pub [f32; 8]);

// MARK: Godot types

/// What `pixmith_godot_preview` returns.
#[derive(Clone, Debug, Default)]
pub struct GodotPreview {
    /// The first `.tres` text.
    pub tres: String,
    /// Absolute paths that would be written.
    pub files: Vec<String>,
    pub warnings: Vec<String>,
}

/// Which layers the SpriteFrames export includes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerMode {
    Visible,
    Each,
    Selected,
}

/// Mirrors the JSON understood by `core/src/gamedev/godot/settings.rs` (`GodotSettings`).
#[derive(Clone, Debug, PartialEq)]
pub struct GodotSettings {
    pub root: String,
    pub tres: String,
    pub padding: u32,
    pub extrude: bool,
    pub trim: bool,
    pub max_texture: u32,
    pub scale: u32,
    pub grid: bool,
    pub per_animation_png: bool,
    pub name_template: String,
    pub layer_mode: LayerMode,
    pub selected_layers: Vec<String>,
    pub tags_include: Vec<String>,
    pub tags_exclude: Vec<String>,
    pub tscn: bool,
    pub overwrite_tscn: bool,
}

impl Default for GodotSettings {
    fn default() -> Self {
        GodotSettings {
            root: String::new(),
            tres: String::new(),
            padding: 1,
            extrude: true,
            trim: false,
            max_texture: 4096,
            scale: 1,
            grid: false,
            per_animation_png: false,
            name_template: "{name}".into(),
            layer_mode: LayerMode::Visible,
            selected_layers: Vec::new(),
            tags_include: Vec::new(),
            tags_exclude: Vec::new(),
            tscn: false,
            overwrite_tscn: false,
        }
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn string_list(v: &[String]) -> String {
    format!("[{}]", v.iter().map(|s| json_str(s)).collect::<Vec<_>>().join(","))
}

impl GodotSettings {
    pub fn from_json(text: &str) -> Option<GodotSettings> {
        let v = json::parse(text).ok()?;
        v.as_object()?;
        let d = GodotSettings::default();
        let num = |k: &str, d: u32| v.get(k).and_then(Value::as_f64).map_or(d, |n| n.max(0.0) as u32);
        let flag = |k: &str, d: bool| match v.get(k) {
            Some(Value::Bool(b)) => *b,
            _ => d,
        };
        let text_of = |k: &str, d: &str| v.get(k).and_then(Value::as_str).unwrap_or(d).to_string();
        let (layer_mode, selected_layers) = match v.get("layers") {
            Some(Value::Arr(_)) => (LayerMode::Selected, strings(v.get("layers"))),
            Some(Value::Str(s)) if s == "each" => (LayerMode::Each, Vec::new()),
            _ => (LayerMode::Visible, Vec::new()),
        };
        Some(GodotSettings {
            root: text_of("root", ""),
            tres: text_of("tres", ""),
            padding: num("padding", d.padding),
            extrude: flag("extrude", d.extrude),
            trim: flag("trim", d.trim),
            max_texture: num("max_texture", d.max_texture),
            scale: num("scale", d.scale),
            grid: v.get("packing").and_then(Value::as_str) == Some("grid"),
            per_animation_png: flag("per_animation_png", false),
            name_template: text_of("name_template", &d.name_template),
            layer_mode,
            selected_layers,
            tags_include: strings(v.get("tags_include")),
            tags_exclude: strings(v.get("tags_exclude")),
            tscn: flag("tscn", false),
            overwrite_tscn: flag("overwrite_tscn", false),
        })
    }

    pub fn json(&self) -> String {
        let layers = match self.layer_mode {
            LayerMode::Visible => json_str("visible"),
            LayerMode::Each => json_str("each"),
            LayerMode::Selected => string_list(&self.selected_layers),
        };
        format!(
            "{{\"v\":1,\"root\":{},\"tres\":{},\"padding\":{},\"extrude\":{},\"trim\":{},\"max_texture\":{},\"scale\":{},\"packing\":{},\"per_animation_png\":{},\"name_template\":{},\"layers\":{layers},\"tags_include\":{},\"tags_exclude\":{},\"tscn\":{},\"overwrite_tscn\":{}}}",
            json_str(&self.root),
            json_str(&self.tres),
            self.padding,
            self.extrude,
            self.trim,
            self.max_texture,
            self.scale,
            json_str(if self.grid { "grid" } else { "tight" }),
            self.per_animation_png,
            json_str(&self.name_template),
            string_list(&self.tags_include),
            string_list(&self.tags_exclude),
            self.tscn,
            self.overwrite_tscn
        )
    }
}

/// Export settings of the TileSet exporter, remembered per document (`GodotExportSettings`).
#[derive(Clone, Debug, PartialEq)]
pub struct TilesetSettings {
    pub project_root: String,
    pub output_path: String,
    pub tileset: u32,
    pub mode: u32,
    pub collision: u32,
    pub margin: u32,
    pub separation: u32,
    pub extrude: u32,
    pub max_texture: u32,
    pub write_scene: bool,
    pub terrain_name: String,
    pub color: [f64; 4],
}

impl Default for TilesetSettings {
    fn default() -> Self {
        TilesetSettings {
            project_root: String::new(),
            output_path: String::new(),
            tileset: 0,
            mode: 0,
            collision: 0,
            margin: 0,
            separation: 0,
            extrude: 0,
            max_texture: 4096,
            write_scene: false,
            terrain_name: String::new(),
            color: [0.3, 0.7, 0.3, 1.0],
        }
    }
}

impl TilesetSettings {
    pub fn json(&self) -> String {
        let c = self.color.iter().map(|v| format!("{v}")).collect::<Vec<_>>().join(",");
        format!(
            "{{\"projectRoot\":{},\"outputPath\":{},\"tileset\":{},\"mode\":{},\"collision\":{},\"margin\":{},\"separation\":{},\"extrude\":{},\"maxTexture\":{},\"writeScene\":{},\"terrainName\":{},\"color\":[{c}]}}",
            json_str(&self.project_root),
            json_str(&self.output_path),
            self.tileset,
            self.mode,
            self.collision,
            self.margin,
            self.separation,
            self.extrude,
            self.max_texture,
            self.write_scene,
            json_str(&self.terrain_name)
        )
    }

    pub fn from_json(text: &str) -> Option<TilesetSettings> {
        let v = json::parse(text).ok()?;
        v.as_object()?;
        let d = TilesetSettings::default();
        let num = |k: &str, d: u32| v.get(k).and_then(Value::as_f64).map_or(d, |n| n.max(0.0) as u32);
        let text_of = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let mut color = d.color;
        if let Some(a) = v.get("color").and_then(Value::as_array) {
            for (slot, x) in color.iter_mut().zip(a) {
                if let Some(n) = x.as_f64() {
                    *slot = n;
                }
            }
        }
        Some(TilesetSettings {
            project_root: text_of("projectRoot"),
            output_path: text_of("outputPath"),
            tileset: num("tileset", 0),
            mode: num("mode", 0),
            collision: num("collision", 0),
            margin: num("margin", 0),
            separation: num("separation", 0),
            extrude: num("extrude", 0),
            max_texture: num("maxTexture", d.max_texture),
            write_scene: matches!(v.get("writeScene"), Some(Value::Bool(true))),
            terrain_name: text_of("terrainName"),
            color,
        })
    }
}

// MARK: Scripts

/// What one script run produced (`pixmith_run_script_at`).
#[derive(Clone, Debug, Default)]
pub struct ScriptOutcome {
    pub ok: bool,
    /// `print` and `app.alert` output, newline separated; may end with the selection line.
    pub messages: String,
    /// The error when `ok` is false.
    pub failure: String,
}

// MARK: Shared state

/// A modal notice with an OK button (Swift's `NSAlert` for scripts and re-exports).
#[derive(Clone, Debug)]
pub struct Notice {
    pub title: String,
    pub text: String,
    pub warning: bool,
}

/// State of this area that outlives a frame; one field of [`PixmithApp`].
#[derive(Default)]
pub struct GamedevState {
    pub tween: tween::TweenState,
    pub godot: Option<GodotSheet>,
    pub tileset: Option<TilesetSheet>,
    pub scripts: scripts::ScriptLibrary,
    /// Lua sessions, dialogs and plugin commands (`ui::lua_dialog`).
    pub lua: crate::ui::lua_dialog::LuaState,
    pub job: Option<jobs::Job>,
    pub notice: Option<Notice>,
    pub autosave: autosave::Autosaver,
    pub prefs: prefs::Prefs,
    /// Each tab's file path as last seen, to notice opens and saves for Open Recent.
    seen_paths: Vec<(usize, Option<PathBuf>)>,
}

impl GamedevState {
    /// Turns on what only the real app does: preferences on disk and autosave with its recovery offer.
    pub fn start(&mut self) {
        self.prefs = prefs::Prefs::at(paths::prefs_file());
        self.autosave.start();
        self.scripts.rescan();
        self.lua.start();
    }
}

/// Everything this area draws or runs each frame; call once per frame after the other dialogs.
pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    track_recent(app);
    autosave::tick(app, ctx);
    jobs::poll(app, ctx);
    tween::show(app, ctx);
    if app.error.is_some() {
        return;
    }
    jobs::show(app, ctx);
    if app.gamedev.job.is_some() {
        return;
    }
    let blocked = app.files.prompt.is_some() || app.files.new_sheet.is_some() || app.files.export.is_some();
    if blocked {
        return;
    }
    if autosave::show_offer(app, ctx) {
        return;
    }
    scripts::show_notice(app, ctx);
    if app.gamedev.notice.is_some() {
        return;
    }
    godot::show(app, ctx);
    tileset::show(app, ctx);
    shortcuts(app, ctx);
}

/// Cmd/Ctrl+Alt+T toggles the Tween panel and Cmd/Ctrl+Shift+G re-exports to Godot (`TweenCommands`,
/// `GodotExportCommands`).
fn shortcuts(app: &mut PixmithApp, ctx: &egui::Context) {
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::ALT, egui::Key::T)) {
        app.toggle_tween_panel();
    }
    if app.gamedev.godot.is_some() || app.gamedev.tileset.is_some() {
        return;
    }
    let again = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::G));
    if again {
        app.reexport_godot();
    }
}

/// Notices tabs whose file changed (opened or saved as) and adds that file to Open Recent.
fn track_recent(app: &mut PixmithApp) {
    let now: Vec<(usize, Option<PathBuf>)> =
        app.tabs.iter().map(|t| (t.doc.raw() as usize, t.doc.path.clone())).collect();
    let mut added = Vec::new();
    for (key, path) in &now {
        let before = app.gamedev.seen_paths.iter().find(|(k, _)| k == key).map(|(_, p)| p);
        if let Some(p) = path
            && before.is_none_or(|b| b.as_ref() != Some(p))
        {
            added.push(p.clone());
        }
    }
    app.gamedev.seen_paths = now;
    for p in added {
        app.add_recent(&p);
    }
}

/// One entry of the title row's Export menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportMenuItem {
    SpriteFrames,
    ReexportSpriteFrames,
    TileSet,
    ReexportTileSet,
}

/// The Godot items of the Export menu (`ExportButton` in `TopBar.swift`): call between separators.
pub fn export_menu(ui: &mut egui::Ui, can_reexport_tileset: bool) -> Option<ExportMenuItem> {
    let mut chosen = None;
    if ui.button("Godot SpriteFrames\u{2026}").clicked() {
        chosen = Some(ExportMenuItem::SpriteFrames);
    }
    if ui.button("Re-export Godot SpriteFrames").clicked() {
        chosen = Some(ExportMenuItem::ReexportSpriteFrames);
    }
    if ui.button("Godot TileSet\u{2026}").clicked() {
        chosen = Some(ExportMenuItem::TileSet);
    }
    if ui.add_enabled(can_reexport_tileset, egui::Button::new("Re-export Godot TileSet")).clicked() {
        chosen = Some(ExportMenuItem::ReexportTileSet);
    }
    chosen
}

impl PixmithApp {
    /// Runs an Export menu choice.
    pub fn run_export_menu_item(&mut self, item: ExportMenuItem) {
        match item {
            ExportMenuItem::SpriteFrames => self.open_godot_export(),
            ExportMenuItem::ReexportSpriteFrames => self.reexport_godot(),
            ExportMenuItem::TileSet => self.open_tileset_export(),
            ExportMenuItem::ReexportTileSet => self.reexport_tileset(),
        }
    }

    /// View > Tween (Cmd/Ctrl+Alt+T): opens or closes the floating panel.
    pub fn toggle_tween_panel(&mut self) {
        self.gamedev.tween.toggle();
    }

    pub fn tween_panel_open(&self) -> bool {
        self.gamedev.tween.open
    }

    /// File > Export to Godot... (Export menu: Godot SpriteFrames...).
    pub fn open_godot_export(&mut self) {
        godot::open(self);
    }

    /// Cmd/Ctrl+Shift+G: repeats the last SpriteFrames export, or opens the sheet when there is none.
    pub fn reexport_godot(&mut self) {
        godot::reexport(self);
    }

    pub fn open_tileset_export(&mut self) {
        tileset::open(self);
    }

    pub fn reexport_tileset(&mut self) {
        tileset::reexport(self);
    }

    /// Whether Re-export Godot TileSet is available (`hasGodotExportSettings`).
    pub fn has_tileset_export_settings(&self) -> bool {
        tileset::has_settings(self)
    }
}
