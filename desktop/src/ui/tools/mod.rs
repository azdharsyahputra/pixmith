//! Drawing tools beyond the pencil and the canvas view features: spray, gradient, curve, polygon, contour and
//! text; ink modes and custom brushes; onion skin, tiled mode, grids and the reference image; the View Options
//! and Preview windows.
//!
//! | File | Swift counterpart |
//! | --- | --- |
//! | `draw.rs` | `DrawingTools.swift`, `InkTools.swift`, `ShapeTools.swift` (the gestures) |
//! | `text.rs` | `TextSheet` and `renderText` in `ShapeTools.swift` |
//! | `view.rs` | `ViewSettings.swift` and `CanvasOverlays.swift` |
//! | `view_options.rs` | `ViewOptionsWindow.swift` |
//! | `preview.rs` | `PreviewWindow.swift` |
//!
//! The canvas calls into `draw` from its pointer handling and into `view` when it paints. A menu bar calls the
//! `pub fn`s at the bottom of this file.

pub mod draw;
pub mod preview;
pub mod text;
pub mod view;
pub mod view_options;

use crate::app::{PixmithApp, Tool};
use crate::doc::Doc;
use eframe::egui;
use std::path::{Path, PathBuf};
use view::{ReferencePlacement, TiledMode};

/// State of these features that outlives a frame. It lives in `CanvasState::tools`.
#[derive(Default)]
pub struct ToolsState {
    pub settings: view::ViewSettings,
    pub draw: draw::DrawState,
    pub text: text::TextState,
    pub onion: view::OnionCache,
    pub reference: Option<view::Reference>,
    pub view_options_open: bool,
    pub preview: preview::PreviewState,
    settings_path: Option<PathBuf>,
    saved_settings: String,
}

impl ToolsState {
    /// Loads the saved view settings and keeps saving them when they change. Without this call (as in tests)
    /// nothing is read or written.
    pub fn enable_persistence(&mut self, path: Option<PathBuf>) {
        if let Some(p) = &path
            && let Ok(text) = std::fs::read_to_string(p)
        {
            self.settings.apply_text(&text);
        }
        self.saved_settings = self.settings.to_text();
        self.settings_path = path;
    }

    fn persist(&mut self) {
        let Some(path) = &self.settings_path else { return };
        let text = self.settings.to_text();
        if text == self.saved_settings {
            return;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Best effort, like `UserDefaults`: a settings file that cannot be written is not worth an error dialog.
        if std::fs::write(path, &text).is_ok() {
            self.saved_settings = text;
        }
    }
}

// MARK: Per-frame hooks

/// Draws the floating windows and the text sheet, and saves changed view settings. Call once per frame after
/// the main layout.
pub fn show_windows(app: &mut PixmithApp, ctx: &egui::Context) {
    app.canvas.opts.custom_brush = Doc::brush_info();
    text::show(app, ctx);
    view_options::show(app, ctx);
    preview::show(app, ctx);
    app.canvas.tools.persist();
}

/// Keyboard shortcuts of these tools: Cmd/Ctrl+B takes a brush from the selection. Call from the canvas'
/// key handling.
pub fn handle_keys(app: &mut PixmithApp, ctx: &egui::Context) {
    let brush = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::B));
    if brush {
        brush_from_selection(app);
    }
}

// MARK: Commands for a menu bar

/// View > View Options...
pub fn open_view_options(app: &mut PixmithApp) {
    app.canvas.tools.view_options_open = true;
}

pub fn toggle_view_options(app: &mut PixmithApp) {
    let t = &mut app.canvas.tools;
    t.view_options_open = !t.view_options_open;
}

pub fn is_view_options_open(app: &PixmithApp) -> bool {
    app.canvas.tools.view_options_open
}

/// View > Preview
pub fn open_preview(app: &mut PixmithApp) {
    app.canvas.tools.preview.show();
}

pub fn toggle_preview(app: &mut PixmithApp) {
    let p = &mut app.canvas.tools.preview;
    if p.open { p.close() } else { p.show() }
}

pub fn is_preview_open(app: &PixmithApp) -> bool {
    app.canvas.tools.preview.open
}

/// The pixel grid switch (also the `#` button in the timeline header).
pub fn toggle_grid(app: &mut PixmithApp) {
    app.timeline.show_grid = !app.timeline.show_grid;
}

/// The onion skin switch (also the stack button in the timeline header).
pub fn toggle_onion_skin(app: &mut PixmithApp) {
    app.timeline.onion_skin = !app.timeline.onion_skin;
}

/// View > Tiled Mode
pub fn set_tiled_mode(app: &mut PixmithApp, mode: TiledMode) {
    app.canvas.tools.settings.tiled = mode;
}

pub fn tiled_mode(app: &PixmithApp) -> TiledMode {
    app.canvas.tools.settings.tiled
}

pub fn toggle_custom_grid(app: &mut PixmithApp) {
    let s = &mut app.canvas.tools.settings;
    s.custom_grid = !s.custom_grid;
}

pub fn has_reference(app: &PixmithApp) -> bool {
    app.canvas.tools.reference.is_some()
}

pub fn toggle_reference_visible(app: &mut PixmithApp) {
    let s = &mut app.canvas.tools.settings;
    s.show_reference = !s.show_reference;
}

pub fn set_reference_placement(app: &mut PixmithApp, placement: ReferencePlacement) {
    app.canvas.tools.settings.reference_placement = placement;
}

/// View > Load Reference Image... (asks for the file).
pub fn choose_reference(app: &mut PixmithApp, ctx: &egui::Context) {
    let picked = rfd::FileDialog::new()
        .set_title("Choose a reference image (it is not saved in the sprite)")
        .add_filter("Images", &["png", "jpg", "jpeg", "gif", "bmp"])
        .pick_file();
    if let Some(path) = picked {
        load_reference(app, &path, ctx);
    }
}

/// Loads `path` as the reference image (`ViewSettings.loadReference`).
pub fn load_reference(app: &mut PixmithApp, path: &Path, ctx: &egui::Context) {
    match view::read_image(path) {
        Ok((w, h, rgba)) => {
            let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
            let options = egui::TextureOptions {
                magnification: egui::TextureFilter::Nearest,
                minification: egui::TextureFilter::Linear,
                ..Default::default()
            };
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let texture = ctx.load_texture("reference", image, options);
            let t = &mut app.canvas.tools;
            t.reference = Some(view::Reference { name, size: (w, h), texture });
            t.settings.show_reference = true;
        }
        Err(message) => app.error = Some(message),
    }
}

pub fn clear_reference(app: &mut PixmithApp) {
    app.canvas.tools.reference = None;
}

/// Drawing > Brush from Selection (Cmd+B): the selected pixels of the layer become the brush, and the pencil
/// is chosen (`brushFromSelection`).
pub fn brush_from_selection(app: &mut PixmithApp) {
    let tab = app.tab();
    if !tab.doc.brush_from_selection(tab.layer, tab.frame) {
        app.error = Some("Select some visible pixels on the current layer to use as a brush.".into());
        return;
    }
    app.canvas.opts.custom_brush = Doc::brush_info();
    app.tool = Tool::Pencil;
}

/// Drawing > Reset Brush
pub fn reset_brush(app: &mut PixmithApp) {
    Doc::brush_reset();
    app.canvas.opts.custom_brush = Doc::brush_info();
}
