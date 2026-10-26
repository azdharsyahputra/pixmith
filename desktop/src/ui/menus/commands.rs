//! Every menu command: its label, shortcut, enabled and checked state, and what it does. The menu bar draws
//! them (`bar.rs`), the shortcut handler runs them, and the shortcuts window lists them, so one table serves all.
//!
//! The entries and their order follow the commands of `PixmithApp.swift` and every `CommandMenu` /
//! `CommandGroup` in `app/Sources/Pixmith`.

use super::filters::FilterKind;
use super::hooks;
use super::spritedata;
use crate::app::{PixmithApp, Tool};
use crate::doc::Doc;
use crate::ui::canvas::{INKS, fit_zoom};
use crate::ui::dialogs::ExportKind;
use crate::ui::timeline::{header, layers, playback};
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};

/// One entry of a menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
    // File
    Home,
    New,
    Open,
    OpenRecent(usize),
    ClearRecent,
    CloseTab,
    Save,
    SaveAs,
    ExportAseprite,
    ExportPng,
    ExportOptions,
    ExportSheet,
    ExportGif,
    ExportApng,
    ExportSequence,
    /// WebP, JPEG, BMP, TGA, ICO, PCX or FLC (index into `image_formats::EXPORT_KINDS`).
    ExportImage(usize),
    ExportToGodot,
    ReexportToGodot,
    ExportTileset,
    ReexportTileset,
    ImportPng,
    ImportSheetFrames,
    ImportSheetJson,
    ImportGif,
    ImportPngLayer,
    ImportImage,
    Script(usize),
    RunScript,
    OpenScriptsFolder,
    InstallExamples,
    RescanScripts,
    /// A command a Lua plugin added to File > Scripts (index into `LuaState::commands`).
    PluginCommand(usize),
    Quit,
    // Edit
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    FreeTransform,
    Transform(u32),
    Preferences,
    // Select
    SelectAll,
    Deselect,
    InvertSelection,
    ModifySelection(u32),
    SelectByColor,
    SelectIsoTile,
    // Sprite
    SpriteProperties,
    CanvasSize,
    CropToSelection,
    SliceProperties,
    DeleteSlice,
    UserData(u32),
    ColorMode(u32),
    IndexedDithered,
    SnapPixels,
    PalettePreset(usize),
    PaletteFromSprite(u32),
    // Layer
    NewLayer,
    DeleteLayer,
    LayerProperties,
    MoveLayerUp,
    MoveLayerDown,
    DuplicateLayer,
    MergeLayerDown,
    NewGroup,
    NewReferenceLayer,
    ToggleReferenceLayer,
    // Frame
    NewFrame,
    DuplicateFrame,
    DeleteFrame,
    MoveFrameLeft,
    MoveFrameRight,
    CelProperties,
    NewTag,
    // Drawing
    FilledShapes,
    PixelPerfect,
    ContiguousFill,
    MirrorX,
    MirrorY,
    SwapColors,
    InkMode(usize),
    RoundBrush,
    BrushFromSelection,
    ResetBrush,
    GradientTool,
    // Palette
    LoadPalette,
    SavePalette,
    RemovePrimary,
    SortHue,
    SortLuminance,
    // Tilemap
    NewTilemapLayer,
    ConvertToTilemap,
    ConvertToNormal,
    TileStamp,
    // FX
    Fx(FilterKind),
    // View
    ZoomIn,
    ZoomOut,
    ZoomToFit,
    ActualSize,
    OnionSkin,
    FrameDiff,
    Grid,
    IsoGrid,
    Preview,
    TiledMode(usize),
    ShowReference,
    LoadReference,
    ReferenceDisplay(usize),
    History,
    ViewOptions,
    Tween,
    NextTab,
    PreviousTab,
    // Help
    KeyboardShortcuts,
    About,
    CommandLineTool,
}

/// An entry of a menu or submenu.
pub enum Entry {
    Item(Cmd),
    Sep,
    Sub(String, Vec<Entry>),
    /// A greyed-out line ("No scripts found").
    Note(&'static str),
}

use Entry::{Item, Sep};

fn sub(title: &str, entries: Vec<Entry>) -> Entry {
    Entry::Sub(title.to_string(), entries)
}

pub const TILED_MODES: [&str; 4] = ["None", "Tiled in X Axis", "Tiled in Y Axis", "Tiled in Both Axes"];
const COLOR_MODES: [&str; 3] = ["RGB Color", "Grayscale", "Indexed"];
const MODIFY: [&str; 3] = ["Expand\u{2026}", "Contract\u{2026}", "Border\u{2026}"];
const USER_DATA: [&str; 4] =
    ["Sprite User Data\u{2026}", "Layer User Data\u{2026}", "Cel User Data\u{2026}", "Tag User Data\u{2026}"];

/// The top-level menus, in order.
pub const MENUS: [&str; 12] =
    ["File", "Edit", "Select", "Sprite", "Layer", "Frame", "Drawing", "Palette", "Tilemap", "FX", "View", "Help"];

fn presets(app: &PixmithApp) -> Entry {
    let _ = app;
    sub(
        "Load Palette Preset",
        Doc::palette_preset_names().iter().enumerate().map(|(i, _)| Item(Cmd::PalettePreset(i))).collect(),
    )
}

/// The entries of the menu called `name`.
pub fn entries(name: &str, app: &PixmithApp) -> Vec<Entry> {
    match name {
        "File" => {
            let recents: Vec<Entry> = app
                .menus
                .recents_existing()
                .iter()
                .enumerate()
                .map(|(i, _)| Item(Cmd::OpenRecent(i)))
                .chain([Sep, Item(Cmd::ClearRecent)])
                .collect();
            let scripts = hooks::script_names(app);
            let mut script_entries: Vec<Entry> =
                scripts.iter().enumerate().map(|(i, _)| Item(Cmd::Script(i))).collect();
            if script_entries.is_empty() {
                script_entries.push(Entry::Note("No scripts found"));
            }
            let plugin_commands = app.gamedev.lua.commands().len();
            if plugin_commands > 0 {
                script_entries.push(Sep);
                script_entries.extend((0..plugin_commands).map(|i| Item(Cmd::PluginCommand(i))));
            }
            script_entries.extend([
                Sep,
                Item(Cmd::RunScript),
                Item(Cmd::OpenScriptsFolder),
                Item(Cmd::InstallExamples),
                Item(Cmd::RescanScripts),
            ]);
            vec![
                Item(Cmd::Home),
                Sep,
                Item(Cmd::New),
                Item(Cmd::Open),
                sub("Open Recent", recents),
                Item(Cmd::CloseTab),
                Sep,
                Item(Cmd::Save),
                Item(Cmd::SaveAs),
                Item(Cmd::ExportAseprite),
                Sep,
                Item(Cmd::ExportPng),
                Item(Cmd::ExportOptions),
                Item(Cmd::ExportSheet),
                Item(Cmd::ExportGif),
                Item(Cmd::ExportApng),
                Item(Cmd::ExportSequence),
                sub(
                    "Export As",
                    (0..crate::ui::image_formats::EXPORT_KINDS.len()).map(|i| Item(Cmd::ExportImage(i))).collect(),
                ),
                Sep,
                Item(Cmd::ExportToGodot),
                Item(Cmd::ReexportToGodot),
                Item(Cmd::ExportTileset),
                Item(Cmd::ReexportTileset),
                Sep,
                sub(
                    "Import",
                    vec![
                        Item(Cmd::ImportPng),
                        Item(Cmd::ImportSheetFrames),
                        Item(Cmd::ImportSheetJson),
                        Item(Cmd::ImportGif),
                        Item(Cmd::ImportPngLayer),
                        Item(Cmd::ImportImage),
                    ],
                ),
                sub("Scripts", script_entries),
                Sep,
                Item(Cmd::Quit),
            ]
        }
        "Edit" => vec![
            Item(Cmd::Undo),
            Item(Cmd::Redo),
            Sep,
            Item(Cmd::Cut),
            Item(Cmd::Copy),
            Item(Cmd::Paste),
            Item(Cmd::Delete),
            Sep,
            Item(Cmd::FreeTransform),
            Item(Cmd::Transform(0)),
            Item(Cmd::Transform(1)),
            Item(Cmd::Transform(2)),
            Item(Cmd::Transform(3)),
            Sep,
            Item(Cmd::Preferences),
        ],
        "Select" => vec![
            Item(Cmd::SelectAll),
            Item(Cmd::Deselect),
            Item(Cmd::InvertSelection),
            Sep,
            sub("Modify", (0..3).map(|k| Item(Cmd::ModifySelection(k))).collect()),
            Item(Cmd::SelectByColor),
            Item(Cmd::SelectIsoTile),
        ],
        "Sprite" => vec![
            Item(Cmd::SpriteProperties),
            Sep,
            Item(Cmd::CanvasSize),
            Item(Cmd::CropToSelection),
            Sep,
            Item(Cmd::SliceProperties),
            Item(Cmd::DeleteSlice),
            Sep,
            Item(Cmd::UserData(0)),
            Item(Cmd::UserData(1)),
            Item(Cmd::UserData(2)),
            Item(Cmd::UserData(3)),
            Sep,
            sub(
                "Color Mode",
                vec![
                    Item(Cmd::ColorMode(0)),
                    Item(Cmd::ColorMode(1)),
                    Item(Cmd::ColorMode(2)),
                    Sep,
                    Item(Cmd::IndexedDithered),
                    Sep,
                    Item(Cmd::SnapPixels),
                ],
            ),
            presets(app),
            sub(
                "Palette from Sprite",
                [Item(Cmd::PaletteFromSprite(0)), Sep]
                    .into_iter()
                    .chain([8, 16, 32, 64, 128, 256].map(|n| Item(Cmd::PaletteFromSprite(n))))
                    .collect(),
            ),
        ],
        "Layer" => vec![
            Item(Cmd::NewLayer),
            Item(Cmd::DeleteLayer),
            Sep,
            Item(Cmd::LayerProperties),
            Sep,
            Item(Cmd::MoveLayerUp),
            Item(Cmd::MoveLayerDown),
            Item(Cmd::DuplicateLayer),
            Item(Cmd::MergeLayerDown),
            Item(Cmd::NewGroup),
            Item(Cmd::NewReferenceLayer),
            Item(Cmd::ToggleReferenceLayer),
        ],
        "Frame" => vec![
            Item(Cmd::NewFrame),
            Item(Cmd::DuplicateFrame),
            Item(Cmd::DeleteFrame),
            Sep,
            Item(Cmd::MoveFrameLeft),
            Item(Cmd::MoveFrameRight),
            Item(Cmd::CelProperties),
            Item(Cmd::NewTag),
        ],
        "Drawing" => vec![
            Item(Cmd::FilledShapes),
            Item(Cmd::PixelPerfect),
            Item(Cmd::ContiguousFill),
            Sep,
            Item(Cmd::MirrorX),
            Item(Cmd::MirrorY),
            Sep,
            Item(Cmd::SwapColors),
            Sep,
            sub("Ink Mode", (0..INKS.len()).map(|i| Item(Cmd::InkMode(i))).collect()),
            Item(Cmd::RoundBrush),
            Item(Cmd::BrushFromSelection),
            Item(Cmd::ResetBrush),
            Sep,
            Item(Cmd::GradientTool),
        ],
        "Palette" => vec![
            Item(Cmd::LoadPalette),
            Item(Cmd::SavePalette),
            Sep,
            Item(Cmd::RemovePrimary),
            Item(Cmd::SortHue),
            Item(Cmd::SortLuminance),
            Sep,
            presets(app),
        ],
        "Tilemap" => vec![
            Item(Cmd::NewTilemapLayer),
            Item(Cmd::ConvertToTilemap),
            Item(Cmd::ConvertToNormal),
            Sep,
            Item(Cmd::TileStamp),
        ],
        "FX" => {
            use FilterKind::*;
            let f = |k| Item(Cmd::Fx(k));
            vec![
                f(ReplaceColor),
                f(Invert),
                f(Desaturate),
                Sep,
                f(Hsl),
                f(BrightnessContrast),
                f(ColorBalance),
                f(Curves),
                Sep,
                f(Outline),
                f(DropShadow),
                f(Despeckle),
                f(CleanEdges),
                Sep,
                f(LintSprite),
                Sep,
                f(Blur),
                f(Sharpen),
                Sep,
                f(Scale),
            ]
        }
        "View" => vec![
            Item(Cmd::ZoomIn),
            Item(Cmd::ZoomOut),
            Item(Cmd::ZoomToFit),
            Item(Cmd::ActualSize),
            Sep,
            Item(Cmd::OnionSkin),
            Item(Cmd::FrameDiff),
            Item(Cmd::Grid),
            Item(Cmd::IsoGrid),
            Sep,
            Item(Cmd::Preview),
            sub("Tiled Mode", (0..4).map(|i| Item(Cmd::TiledMode(i))).collect()),
            Item(Cmd::ShowReference),
            Item(Cmd::LoadReference),
            sub(
                "Reference Layers",
                (0..spritedata::REFERENCE_DISPLAY.len()).map(|i| Item(Cmd::ReferenceDisplay(i))).collect(),
            ),
            Item(Cmd::History),
            Item(Cmd::ViewOptions),
            Item(Cmd::Tween),
            Sep,
            Item(Cmd::NextTab),
            Item(Cmd::PreviousTab),
        ],
        "Help" => vec![Item(Cmd::KeyboardShortcuts), Item(Cmd::CommandLineTool), Item(Cmd::About)],
        _ => Vec::new(),
    }
}

/// Every command that is listed in a menu, with the menu it is in (for the shortcuts window).
pub fn walk(entries: &[Entry], out: &mut Vec<Cmd>) {
    for e in entries {
        match e {
            Item(c) => out.push(*c),
            Entry::Sub(_, inner) => walk(inner, out),
            _ => {}
        }
    }
}

fn ks(mods: Modifiers, key: Key) -> Option<KeyboardShortcut> {
    Some(KeyboardShortcut::new(mods, key))
}

impl Cmd {
    /// The text of the entry. Names of presets, recent files and scripts come from the app.
    pub fn label(self, app: &PixmithApp) -> String {
        let t = |s: &str| s.to_string();
        match self {
            Cmd::Home => t("Home"),
            Cmd::New => t("New Sprite\u{2026}"),
            Cmd::Open => t("Open\u{2026}"),
            Cmd::OpenRecent(i) => app
                .menus
                .recents_existing()
                .get(i)
                .and_then(|p| p.file_name())
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            Cmd::ClearRecent => t("Clear Menu"),
            Cmd::CloseTab => t("Close Tab"),
            Cmd::Save => t("Save"),
            Cmd::SaveAs => t("Save As\u{2026}"),
            Cmd::ExportAseprite => t("Export As Aseprite\u{2026}"),
            Cmd::ExportPng => t("Export PNG\u{2026}"),
            Cmd::ExportOptions => t("Export\u{2026}"),
            Cmd::ExportSheet => t("Export Sprite Sheet\u{2026}"),
            Cmd::ExportGif => t("Export Animated GIF\u{2026}"),
            Cmd::ExportApng => t("Export Animated PNG\u{2026}"),
            Cmd::ExportSequence => t("Export PNG Sequence\u{2026}"),
            Cmd::ExportImage(i) => t(crate::ui::image_formats::menu_label(crate::ui::image_formats::EXPORT_KINDS[i])),
            Cmd::ExportToGodot => t("Export to Godot\u{2026}"),
            Cmd::ReexportToGodot => t("Re-export to Godot"),
            Cmd::ExportTileset => t("Export Tileset to Godot\u{2026}"),
            Cmd::ReexportTileset => t("Re-export Tileset to Godot"),
            Cmd::ImportPng => t("PNG as New Sprite\u{2026}"),
            Cmd::ImportSheetFrames => t("Sprite Sheet as Frames\u{2026}"),
            Cmd::ImportSheetJson => t("Sprite Sheet with JSON\u{2026}"),
            Cmd::ImportGif => t("Animated GIF as New Sprite\u{2026}"),
            Cmd::ImportPngLayer => t("PNG as New Layer\u{2026}"),
            Cmd::ImportImage => t("Image (WebP, JPEG, BMP, TGA, ICO, PCX, FLC) as New Sprite\u{2026}"),
            Cmd::Script(i) => hooks::script_names(app).get(i).cloned().unwrap_or_default(),
            Cmd::RunScript => t("Run Script\u{2026}"),
            Cmd::OpenScriptsFolder => t("Open Scripts Folder"),
            Cmd::InstallExamples => t("Install Example Scripts"),
            Cmd::RescanScripts => t("Rescan"),
            Cmd::PluginCommand(i) => app.lua_command(i).map_or_else(String::new, |c| c.title.clone()),
            Cmd::Quit => t("Quit Pixmith"),
            Cmd::Undo => t("Undo"),
            Cmd::Redo => t("Redo"),
            Cmd::Cut => t("Cut"),
            Cmd::Copy => t("Copy"),
            Cmd::Paste => t("Paste"),
            Cmd::Delete => t("Delete"),
            Cmd::FreeTransform => t("Free Transform"),
            Cmd::Transform(op) => {
                t(["Flip Horizontal", "Flip Vertical", "Rotate 90\u{b0} Clockwise", "Rotate 90\u{b0} Counterclockwise"]
                    [op.min(3) as usize])
            }
            Cmd::Preferences => t("Preferences\u{2026}"),
            Cmd::SelectAll => t("Select All"),
            Cmd::Deselect => t("Deselect"),
            Cmd::InvertSelection => t("Invert Selection"),
            Cmd::ModifySelection(k) => t(MODIFY[k.min(2) as usize]),
            Cmd::SelectByColor => t("Select by Color"),
            Cmd::SelectIsoTile => t("Isometric Tile"),
            Cmd::SpriteProperties => t("Sprite Properties\u{2026}"),
            Cmd::CanvasSize => t("Canvas Size\u{2026}"),
            Cmd::CropToSelection => t("Crop to Selection"),
            Cmd::SliceProperties => t("Slice Properties\u{2026}"),
            Cmd::DeleteSlice => t("Delete Slice"),
            Cmd::UserData(k) => t(USER_DATA[k.min(3) as usize]),
            Cmd::ColorMode(m) => t(COLOR_MODES[m.min(2) as usize]),
            Cmd::IndexedDithered => t("Indexed with Ordered Dithering"),
            Cmd::SnapPixels => t("Snap Pixels to Palette"),
            Cmd::PalettePreset(i) => Doc::palette_preset_names().get(i).cloned().unwrap_or_default(),
            Cmd::PaletteFromSprite(0) => t("Exact Colors"),
            Cmd::PaletteFromSprite(n) => format!("Quantize to {n} Colors"),
            Cmd::NewLayer => t("New Layer"),
            Cmd::DeleteLayer => t("Delete Layer"),
            Cmd::LayerProperties => t("Layer Properties\u{2026}"),
            Cmd::MoveLayerUp => t("Move Layer Up"),
            Cmd::MoveLayerDown => t("Move Layer Down"),
            Cmd::DuplicateLayer => t("Duplicate Layer"),
            Cmd::MergeLayerDown => t("Merge Layer Down"),
            Cmd::NewGroup => t("New Group"),
            Cmd::NewReferenceLayer => t("New Reference Layer from File\u{2026}"),
            Cmd::ToggleReferenceLayer => t("Reference Layer"),
            Cmd::NewFrame => t("New Frame"),
            Cmd::DuplicateFrame => t("Duplicate Frame"),
            Cmd::DeleteFrame => t("Delete Frame"),
            Cmd::MoveFrameLeft => t("Move Frame Left"),
            Cmd::MoveFrameRight => t("Move Frame Right"),
            Cmd::CelProperties => t("Cel Properties & Links\u{2026}"),
            Cmd::NewTag => t("New Tag\u{2026}"),
            Cmd::FilledShapes => t("Filled Shapes"),
            Cmd::PixelPerfect => t("Pixel-perfect Pencil"),
            Cmd::ContiguousFill => t("Contiguous Fill"),
            Cmd::MirrorX => t("Mirror Horizontally"),
            Cmd::MirrorY => t("Mirror Vertically"),
            Cmd::SwapColors => t("Swap Colors"),
            Cmd::InkMode(i) => t(INKS[i.min(INKS.len() - 1)]),
            Cmd::RoundBrush => t("Round Brush"),
            Cmd::BrushFromSelection => t("Brush from Selection"),
            Cmd::ResetBrush => t("Reset Brush"),
            Cmd::GradientTool => t("Gradient Tool"),
            Cmd::LoadPalette => t("Load Palette\u{2026}"),
            Cmd::SavePalette => t("Save Palette\u{2026}"),
            Cmd::RemovePrimary => t("Remove Primary Color from Palette"),
            Cmd::SortHue => t("Sort by Hue"),
            Cmd::SortLuminance => t("Sort by Luminance"),
            Cmd::NewTilemapLayer => t("New Tilemap Layer\u{2026}"),
            Cmd::ConvertToTilemap => t("Convert Layer to Tilemap\u{2026}"),
            Cmd::ConvertToNormal => t("Convert Tilemap to Normal Layer"),
            Cmd::TileStamp => t("Tile Stamp Mode"),
            Cmd::Fx(k) => format!("{}\u{2026}", k.title()),
            Cmd::ZoomIn => t("Zoom In"),
            Cmd::ZoomOut => t("Zoom Out"),
            Cmd::ZoomToFit => t("Zoom to Fit"),
            Cmd::ActualSize => t("Actual Size"),
            Cmd::OnionSkin => t("Onion Skin"),
            Cmd::FrameDiff => t("Frame Difference"),
            Cmd::Grid => t("Grid"),
            Cmd::IsoGrid => t("Isometric Grid"),
            Cmd::Preview => t("Preview"),
            Cmd::TiledMode(i) => t(TILED_MODES[i.min(3)]),
            Cmd::ShowReference => t("Show Reference Image"),
            Cmd::LoadReference => t("Load Reference Image\u{2026}"),
            Cmd::ReferenceDisplay(i) => {
                t(spritedata::REFERENCE_DISPLAY[i.min(spritedata::REFERENCE_DISPLAY.len() - 1)].0)
            }
            Cmd::History => t("History"),
            Cmd::ViewOptions => t("View Options\u{2026}"),
            Cmd::Tween => t("Tween"),
            Cmd::NextTab => t("Next Tab"),
            Cmd::PreviousTab => t("Previous Tab"),
            Cmd::KeyboardShortcuts => t("Keyboard Shortcuts"),
            Cmd::About => t("About Pixmith"),
            Cmd::CommandLineTool => t("Command Line Tool\u{2026}"),
        }
    }

    /// The keyboard shortcut shown beside the entry.
    pub fn shortcut(self) -> Option<KeyboardShortcut> {
        let (c, s, a) = (Modifiers::COMMAND, Modifiers::SHIFT, Modifiers::ALT);
        match self {
            Cmd::Home => ks(c | s, Key::H),
            Cmd::New => ks(c, Key::N),
            Cmd::Open => ks(c, Key::O),
            Cmd::CloseTab => ks(c, Key::W),
            Cmd::Save => ks(c, Key::S),
            Cmd::SaveAs => ks(c | s, Key::S),
            Cmd::ExportPng => ks(c | s, Key::E),
            Cmd::ExportOptions => ks(c | a, Key::E),
            Cmd::ReexportToGodot => ks(c | s, Key::G),
            // Windows has no standard quit shortcut; the window's close button does it.
            Cmd::Quit if !cfg!(windows) => ks(c, Key::Q),
            Cmd::Undo => ks(c, Key::Z),
            Cmd::Redo => ks(c | s, Key::Z),
            Cmd::Cut => ks(c, Key::X),
            Cmd::Copy => ks(c, Key::C),
            Cmd::Paste => ks(c, Key::V),
            Cmd::FreeTransform => ks(c, Key::T),
            Cmd::Preferences => ks(c, Key::Comma),
            Cmd::SelectAll => ks(c, Key::A),
            Cmd::Deselect => ks(c, Key::D),
            Cmd::InvertSelection => ks(c | s, Key::I),
            Cmd::SelectByColor => ks(c | a, Key::B),
            Cmd::NewLayer => ks(c | s, Key::N),
            Cmd::NewFrame => ks(c | a, Key::N),
            Cmd::DuplicateFrame => ks(c | s, Key::D),
            Cmd::BrushFromSelection => ks(c, Key::B),
            Cmd::ZoomIn => ks(c, Key::Equals),
            Cmd::ZoomOut => ks(c, Key::Minus),
            Cmd::ZoomToFit => ks(c, Key::Num0),
            Cmd::ActualSize => ks(c, Key::Num1),
            Cmd::Grid => ks(c, Key::Quote),
            Cmd::Preview => ks(c | a, Key::P),
            Cmd::History => ks(c | a, Key::Y),
            Cmd::ViewOptions => ks(c | a, Key::Comma),
            Cmd::Tween => ks(c | a, Key::T),
            Cmd::NextTab => ks(Modifiers::CTRL, Key::Tab),
            Cmd::PreviousTab => ks(Modifiers::CTRL | s, Key::Tab),
            _ => None,
        }
    }

    /// Commands whose shortcut is consumed elsewhere (the file actions, undo and redo, zoom keys): the menu
    /// shows it but the handler here must not take it twice.
    fn handled_elsewhere(self) -> bool {
        matches!(
            self,
            Cmd::New
                | Cmd::Open
                | Cmd::CloseTab
                | Cmd::Save
                | Cmd::SaveAs
                | Cmd::Undo
                | Cmd::Redo
                | Cmd::ZoomIn
                | Cmd::ZoomOut
        )
    }

    pub fn enabled(self, app: &PixmithApp) -> bool {
        let tab = app.tab();
        let doc = &tab.doc;
        match self {
            Cmd::OpenRecent(_) => true,
            Cmd::PluginCommand(i) => app.lua_command(i).is_some_and(|c| c.enabled) && !app.job_running(),
            Cmd::ClearRecent => !app.menus.recents.is_empty(),
            Cmd::Undo => doc.history_counts().0 > 0,
            Cmd::Redo => doc.history_counts().1 > 0,
            Cmd::CropToSelection => hooks::has_selection(app),
            Cmd::SliceProperties | Cmd::DeleteSlice => hooks::has_selected_slice(app),
            Cmd::UserData(3) => doc.tag_count() > 0,
            Cmd::SnapPixels => doc.color_mode() != 0,
            Cmd::ConvertToTilemap => !doc.layer_is_tilemap(tab.layer) && !doc.layer_is_group(tab.layer),
            Cmd::ConvertToNormal | Cmd::TileStamp => doc.layer_is_tilemap(tab.layer),
            Cmd::ShowReference => hooks::has_reference_image(app),
            Cmd::ResetBrush => hooks::has_custom_brush(app),
            Cmd::ReexportTileset => hooks::has_godot_tileset_settings(app),
            Cmd::MoveFrameLeft => tab.frame > 0,
            Cmd::MoveFrameRight => tab.frame + 1 < doc.frame_count(),
            Cmd::DeleteFrame => doc.frame_count() > 1,
            Cmd::MergeLayerDown => doc.can_merge_down(tab.layer),
            Cmd::ToggleReferenceLayer => !doc.layer_is_group(tab.layer),
            Cmd::NextTab | Cmd::PreviousTab => app.tabs.len() > 1,
            _ => true,
        }
    }

    /// `Some(on)` for entries that show a check mark.
    pub fn checked(self, app: &PixmithApp) -> Option<bool> {
        let o = &app.canvas.opts;
        Some(match self {
            Cmd::ColorMode(m) => app.tab().doc.color_mode() == m,
            Cmd::FilledShapes => o.filled,
            Cmd::PixelPerfect => o.pixel_perfect,
            Cmd::ContiguousFill => o.contiguous,
            Cmd::MirrorX => o.mirror_x,
            Cmd::MirrorY => o.mirror_y,
            Cmd::InkMode(i) => o.ink == i,
            Cmd::RoundBrush => o.circle_brush,
            Cmd::TileStamp => hooks::tile_stamp(app),
            Cmd::OnionSkin => app.timeline.onion_skin,
            Cmd::FrameDiff => app.timeline.frame_diff,
            Cmd::Grid => app.timeline.show_grid,
            Cmd::IsoGrid => hooks::iso_grid_shown(app),
            Cmd::PluginCommand(i) => match app.lua_command(i) {
                Some(c) if c.checked => true,
                _ => return None,
            },
            Cmd::Preview => hooks::preview_open(app),
            Cmd::TiledMode(i) => hooks::tiled_mode(app) == i,
            Cmd::ShowReference => hooks::show_reference(app),
            Cmd::ToggleReferenceLayer => app.tab().doc.layer_is_reference(app.tab().layer),
            Cmd::ReferenceDisplay(i) => spritedata::reference_display_is(app, i),
            Cmd::History => app.menus.history_open,
            _ => return None,
        })
    }

    /// What the entry does, announced to Lua scripts as `beforecommand` / `aftercommand`.
    pub fn run(self, app: &mut PixmithApp, ctx: &egui::Context) {
        crate::ui::lua_dialog::before_command(app, self);
        self.run_command(app, ctx);
        crate::ui::lua_dialog::after_command(app, self);
    }

    fn run_command(self, app: &mut PixmithApp, ctx: &egui::Context) {
        match self {
            Cmd::Home => app.go_home(),
            Cmd::New => app.request_new_sprite(),
            Cmd::Open => app.open_dialog(),
            Cmd::OpenRecent(i) => {
                if let Some(path) = app.menus.recents_existing().get(i).cloned() {
                    app.open_path(&path);
                }
            }
            Cmd::ClearRecent => app.menus.clear_recents(),
            Cmd::CloseTab => app.request_close(app.active),
            Cmd::Save => {
                app.save();
            }
            Cmd::SaveAs => {
                app.save_as();
            }
            // `Export…` opens the options sheet of the common case until the combined sheet is ported.
            Cmd::ExportAseprite => app.export_as_aseprite(),
            Cmd::ExportPng | Cmd::ExportOptions => app.request_export(ExportKind::Png),
            Cmd::ExportSheet => app.request_export(ExportKind::Sheet),
            Cmd::ExportGif => app.request_export(ExportKind::Gif),
            Cmd::ExportApng => app.request_export(ExportKind::Apng),
            Cmd::ExportSequence => app.request_export(ExportKind::Sequence),
            Cmd::ExportImage(i) => app.request_export(crate::ui::image_formats::EXPORT_KINDS[i]),
            Cmd::ExportToGodot => hooks::export_to_godot(app),
            Cmd::ReexportToGodot => hooks::reexport_to_godot(app),
            Cmd::ExportTileset => hooks::export_tileset_to_godot(app),
            Cmd::ReexportTileset => hooks::reexport_tileset_to_godot(app),
            Cmd::ImportPng => {
                if let Some(path) = rfd::FileDialog::new().add_filter("PNG", &["png"]).pick_file() {
                    app.open_path(&path);
                }
            }
            Cmd::ImportSheetFrames => hooks::import_sheet_as_frames(app),
            Cmd::ImportSheetJson => hooks::import_sheet_with_json(app),
            Cmd::ImportGif => hooks::import_gif_as_sprite(app),
            Cmd::ImportPngLayer => hooks::import_png_as_layer(app),
            Cmd::ImportImage => app.import_image_dialog(),
            Cmd::Script(i) => {
                if let Some(name) = hooks::script_names(app).get(i).cloned() {
                    hooks::run_script(app, &name);
                }
            }
            Cmd::RunScript => hooks::run_script_from_file(app),
            Cmd::OpenScriptsFolder => hooks::open_scripts_folder(app),
            Cmd::InstallExamples => hooks::install_example_scripts(app),
            Cmd::RescanScripts => hooks::rescan_scripts(app),
            Cmd::PluginCommand(i) => app.lua_run_command(i),
            Cmd::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Cmd::Undo => {
                let tab = app.tab_mut();
                if tab.doc.undo() {
                    tab.changed();
                }
            }
            Cmd::Redo => {
                let tab = app.tab_mut();
                if tab.doc.redo() {
                    tab.changed();
                }
            }
            Cmd::Cut => hooks::cut(app),
            Cmd::Copy => hooks::copy(app),
            Cmd::Paste => hooks::paste(app),
            Cmd::Delete => hooks::delete_selection(app),
            Cmd::FreeTransform => hooks::free_transform(app),
            Cmd::Transform(op) => hooks::transform(app, op),
            Cmd::Preferences => app.menus.open_preferences(),
            Cmd::SelectAll => hooks::select_all(app),
            Cmd::Deselect => hooks::deselect(app),
            Cmd::InvertSelection => hooks::invert_selection(app),
            Cmd::ModifySelection(k) => hooks::modify_selection(app, k),
            Cmd::SelectByColor => hooks::select_by_color(app),
            Cmd::SelectIsoTile => hooks::select_iso_tile(app),
            Cmd::SpriteProperties => app.menus.sprite_props = true,
            Cmd::CanvasSize => app.menus.open_canvas_size(app.tab().doc.width(), app.tab().doc.height()),
            Cmd::CropToSelection => hooks::crop_to_selection(app),
            Cmd::SliceProperties => hooks::slice_properties(app),
            Cmd::DeleteSlice => hooks::delete_slice(app),
            Cmd::UserData(k) => hooks::user_data(app, k),
            Cmd::ColorMode(m) => convert_mode(app, m, 0),
            Cmd::IndexedDithered => convert_mode(app, 2, 4),
            Cmd::SnapPixels => {
                playback::stop(app.tab_mut());
                if app.tab_mut().doc.snap_pixels_to_palette() {
                    app.tab_mut().changed();
                }
            }
            Cmd::PalettePreset(i) => {
                let tab = app.tab_mut();
                match tab.doc.load_palette_preset(i as u32) {
                    Ok(()) => tab.changed(),
                    Err(e) => app.error = Some(e),
                }
            }
            Cmd::PaletteFromSprite(n) => {
                playback::stop(app.tab_mut());
                let result = app.tab_mut().doc.palette_from_sprite(n);
                match result {
                    Ok(()) => {
                        let snapped = app.tab().doc.snap_color(app.primary);
                        app.primary = snapped;
                        app.tab_mut().changed();
                    }
                    Err(e) => app.error = Some(e),
                }
            }
            Cmd::NewLayer => layers::add_layer(app.tab_mut()),
            Cmd::DeleteLayer => layers::remove_layer(app.tab_mut()),
            Cmd::LayerProperties => {
                let layer = app.tab().layer;
                let PixmithApp { tabs, active, timeline, .. } = app;
                timeline.open_layer_sheet(&tabs[*active], layer);
            }
            Cmd::MoveLayerUp => layer_edit(app, |d, l| d.move_layer(l, true)),
            Cmd::MoveLayerDown => layer_edit(app, |d, l| d.move_layer(l, false)),
            Cmd::DuplicateLayer => layer_edit(app, |d, l| d.duplicate_layer(l)),
            Cmd::MergeLayerDown => layer_edit(app, |d, l| d.merge_layer_down(l)),
            Cmd::NewGroup => layers::add_group(app.tab_mut()),
            Cmd::NewReferenceLayer => spritedata::new_reference_layer(app),
            Cmd::ToggleReferenceLayer => spritedata::toggle_reference(app),
            Cmd::NewFrame => header::add_frame(app.tab_mut(), false),
            Cmd::DuplicateFrame => header::add_frame(app.tab_mut(), true),
            Cmd::DeleteFrame => {
                let tab = app.tab_mut();
                playback::stop(tab);
                if tab.doc.remove_frame(tab.frame) {
                    (tab.layer, tab.frame) = tab.doc.clamp_selection(tab.layer, tab.frame);
                    tab.changed();
                }
            }
            Cmd::MoveFrameLeft => header::move_frame(app.tab_mut(), -1),
            Cmd::MoveFrameRight => header::move_frame(app.tab_mut(), 1),
            Cmd::CelProperties => {
                let PixmithApp { menus, tabs, active, .. } = app;
                menus.open_cel_sheet(&tabs[*active]);
            }
            Cmd::NewTag => {
                let PixmithApp { tabs, active, timeline, .. } = app;
                timeline.open_tag_sheet(&tabs[*active]);
            }
            Cmd::FilledShapes => app.canvas.opts.filled ^= true,
            Cmd::PixelPerfect => app.canvas.opts.pixel_perfect ^= true,
            Cmd::ContiguousFill => app.canvas.opts.contiguous ^= true,
            Cmd::MirrorX => app.canvas.opts.mirror_x ^= true,
            Cmd::MirrorY => app.canvas.opts.mirror_y ^= true,
            Cmd::SwapColors => std::mem::swap(&mut app.primary, &mut app.secondary),
            Cmd::InkMode(i) => app.canvas.opts.ink = i,
            Cmd::RoundBrush => app.canvas.opts.circle_brush ^= true,
            Cmd::BrushFromSelection => hooks::brush_from_selection(app),
            Cmd::ResetBrush => hooks::reset_brush(app),
            Cmd::GradientTool => app.tool = Tool::Gradient,
            Cmd::LoadPalette => {
                if let Some(path) = rfd::FileDialog::new().add_filter("Palette", &["gpl", "hex"]).pick_file() {
                    let tab = app.tab_mut();
                    match tab.doc.load_palette(&path) {
                        Ok(()) => tab.changed(),
                        Err(e) => app.error = Some(e),
                    }
                }
            }
            Cmd::SavePalette => {
                let picked = rfd::FileDialog::new()
                    .add_filter("Palette", &["gpl", "hex"])
                    .set_file_name("palette.gpl")
                    .save_file();
                if let Some(path) = picked
                    && let Err(e) = app.tab().doc.save_palette(&path)
                {
                    app.error = Some(e);
                }
            }
            Cmd::RemovePrimary => {
                let primary = app.primary;
                let tab = app.tab_mut();
                if let Some(i) = tab.doc.palette().iter().position(|&c| c == primary)
                    && tab.doc.remove_palette_entries(&[i as u32])
                {
                    tab.changed();
                }
            }
            Cmd::SortHue => {
                let tab = app.tab_mut();
                tab.doc.sort_palette(true);
                tab.changed();
            }
            Cmd::SortLuminance => {
                let tab = app.tab_mut();
                tab.doc.sort_palette(false);
                tab.changed();
            }
            Cmd::NewTilemapLayer => hooks::new_tilemap_layer(app),
            Cmd::ConvertToTilemap => hooks::convert_to_tilemap(app),
            Cmd::ConvertToNormal => hooks::convert_to_normal(app),
            Cmd::TileStamp => {
                let on = !hooks::tile_stamp(app);
                hooks::set_tile_stamp(app, on);
            }
            Cmd::Fx(k) => {
                let PixmithApp { menus, tabs, active, primary, .. } = app;
                menus.open_filter(&mut tabs[*active], k, *primary);
            }
            Cmd::ZoomIn => app.tab_mut().zoom = (app.tab().zoom * 2.0).min(64.0),
            Cmd::ZoomOut => app.tab_mut().zoom = (app.tab().zoom / 2.0).max(1.0),
            Cmd::ZoomToFit => {
                let viewport = app.canvas.viewport;
                let tab = app.tab_mut();
                tab.zoom = fit_zoom(viewport, tab.doc.width(), tab.doc.height());
                tab.pan = egui::Vec2::ZERO;
            }
            Cmd::ActualSize => {
                let tab = app.tab_mut();
                tab.zoom = 1.0;
                tab.pan = egui::Vec2::ZERO;
            }
            Cmd::OnionSkin => app.timeline.onion_skin ^= true,
            Cmd::FrameDiff => app.timeline.frame_diff ^= true,
            Cmd::Grid => app.timeline.show_grid ^= true,
            Cmd::IsoGrid => hooks::toggle_iso_grid(app),
            Cmd::Preview => hooks::toggle_preview(app),
            Cmd::TiledMode(i) => hooks::set_tiled_mode(app, i),
            Cmd::ShowReference => {
                let on = !hooks::show_reference(app);
                hooks::set_show_reference(app, on);
            }
            Cmd::LoadReference => hooks::load_reference_image(app, ctx),
            Cmd::ReferenceDisplay(i) => spritedata::set_reference_display(app, i),
            Cmd::History => app.menus.history_open ^= true,
            Cmd::ViewOptions => hooks::view_options(app),
            Cmd::Tween => hooks::toggle_tween(app),
            Cmd::NextTab => step_tab(app, 1),
            Cmd::PreviousTab => step_tab(app, -1),
            Cmd::KeyboardShortcuts => app.menus.help_open = true,
            Cmd::About => app.menus.about_open = true,
            Cmd::CommandLineTool => app.menus.cli_open = true,
        }
    }
}

fn step_tab(app: &mut PixmithApp, delta: i32) {
    let n = app.tabs.len() as i32;
    if n > 1 {
        app.active = (app.active as i32 + delta).rem_euclid(n) as usize;
    }
}

/// Converts the sprite to a color mode (`convertColorMode`): nothing happens when it is the mode already,
/// except for indexed, which re-quantizes.
fn convert_mode(app: &mut PixmithApp, mode: u32, dither: u32) {
    playback::stop(app.tab_mut());
    if mode == app.tab().doc.color_mode() && mode != 2 {
        return;
    }
    match app.tab_mut().doc.convert_color_mode_dithered(mode, dither) {
        Ok(()) => {
            let snapped = app.tab().doc.snap_color(app.primary);
            app.primary = snapped;
            app.tab_mut().changed();
        }
        Err(e) => app.error = Some(e),
    }
}

/// A structural layer edit on the selected layer; the selection follows the result like `finish` in
/// `LayerTools.swift`.
fn layer_edit(app: &mut PixmithApp, edit: impl FnOnce(&mut Doc, u32) -> Option<u32>) {
    let tab = app.tab_mut();
    let Some(r) = edit(&mut tab.doc, tab.layer) else { return };
    if let Some(l) = tab.doc.drawable_from(r) {
        tab.layer = l;
    }
    (tab.layer, tab.frame) = tab.doc.clamp_selection(tab.layer, tab.frame);
    tab.changed();
}

/// Every command that has a shortcut.
const SHORTCUT_CMDS: &[Cmd] = &[
    Cmd::Home,
    Cmd::New,
    Cmd::Open,
    Cmd::CloseTab,
    Cmd::Save,
    Cmd::SaveAs,
    Cmd::ExportPng,
    Cmd::ExportOptions,
    Cmd::ReexportToGodot,
    Cmd::Quit,
    Cmd::Undo,
    Cmd::Redo,
    Cmd::Cut,
    Cmd::Copy,
    Cmd::Paste,
    Cmd::FreeTransform,
    Cmd::Preferences,
    Cmd::SelectAll,
    Cmd::Deselect,
    Cmd::InvertSelection,
    Cmd::SelectByColor,
    Cmd::NewLayer,
    Cmd::NewFrame,
    Cmd::DuplicateFrame,
    Cmd::BrushFromSelection,
    Cmd::ZoomIn,
    Cmd::ZoomOut,
    Cmd::ZoomToFit,
    Cmd::ActualSize,
    Cmd::Grid,
    Cmd::Preview,
    Cmd::History,
    Cmd::ViewOptions,
    Cmd::Tween,
    Cmd::NextTab,
    Cmd::PreviousTab,
];

/// Consumes the menu shortcuts that nothing else handles and runs them (`keyboardShortcut` on the Swift buttons).
pub fn handle_shortcuts(app: &mut PixmithApp, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() || super::blocked(app) || app.menus.shortcuts.recording.is_some() {
        return;
    }
    let mut bound: Vec<(Cmd, KeyboardShortcut)> =
        SHORTCUT_CMDS.iter().filter(|c| !c.handled_elsewhere()).filter_map(|&c| c.shortcut().map(|s| (c, s))).collect();
    // Longer shortcuts first, so Cmd+Shift+D is not taken as Cmd+D.
    bound.sort_by_key(|(_, s)| std::cmp::Reverse(u8::from(s.modifiers.shift) + u8::from(s.modifiers.alt)));
    for (cmd, shortcut) in bound {
        if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) && cmd.enabled(app) {
            cmd.run(app, ctx);
        }
    }
}
