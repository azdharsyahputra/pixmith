//! Application state and the main window layout.
//!
//! Layout, one-to-one with `ContentView.swift`: a full-width title row (document tabs, zoom, export) above
//! three columns: tool rail | canvas column | inspector. The canvas column stacks the tool's context bar,
//! the canvas and the timeline.

use crate::doc::Doc;
use crate::theme::{self, Colors, metric};
use crate::ui;
use eframe::egui;

/// Drawing and selection tools, in the order of `Tool` in `SpriteModel.swift`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Marquee,
    Lasso,
    Polygon,
    Wand,
    Move,
    Pencil,
    Eraser,
    Spray,
    Fill,
    Gradient,
    Line,
    Rectangle,
    Ellipse,
    Curve,
    ShapePolygon,
    Contour,
    Text,
    Picker,
    Slice,
    Blur,
    Jumble,
}

impl Tool {
    /// Rail groups, in order: select, paint, shapes, other (`Tool.groups` in `ToolsPanel.swift`).
    pub const GROUPS: [&'static [Tool]; 4] = [
        &[Tool::Marquee, Tool::Lasso, Tool::Polygon, Tool::Wand, Tool::Move],
        &[Tool::Pencil, Tool::Eraser, Tool::Spray, Tool::Blur, Tool::Jumble, Tool::Fill, Tool::Gradient],
        &[Tool::Line, Tool::Rectangle, Tool::Ellipse, Tool::Curve, Tool::ShapePolygon, Tool::Contour],
        &[Tool::Text, Tool::Picker, Tool::Slice],
    ];

    /// Name shown in tooltips and the context bar (`dsTitle`).
    pub fn title(self) -> &'static str {
        match self {
            Tool::Marquee => "Rectangular Marquee",
            Tool::Lasso => "Lasso",
            Tool::Polygon => "Polygonal Lasso",
            Tool::Wand => "Magic Wand",
            Tool::Move => "Move",
            Tool::Pencil => "Pencil",
            Tool::Eraser => "Eraser",
            Tool::Spray => "Spray",
            Tool::Fill => "Paint Bucket",
            Tool::Gradient => "Gradient",
            Tool::Line => "Line",
            Tool::Rectangle => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Curve => "Curve",
            Tool::ShapePolygon => "Polygon",
            Tool::Contour => "Contour",
            Tool::Text => "Text",
            Tool::Picker => "Eyedropper",
            Tool::Slice => "Slice",
            Tool::Blur => "Blur",
            Tool::Jumble => "Jumble",
        }
    }

    /// Phosphor glyph closest to the SF Symbol the macOS rail uses (`dsIcon`).
    pub fn icon(self) -> &'static str {
        use egui_phosphor::regular as i;
        match self {
            Tool::Marquee => i::SELECTION,
            Tool::Lasso => i::LASSO,
            Tool::Polygon => i::POLYGON,
            Tool::Wand => i::MAGIC_WAND,
            Tool::Move => i::ARROWS_OUT_CARDINAL,
            Tool::Pencil => i::PENCIL_SIMPLE,
            Tool::Eraser => i::ERASER,
            Tool::Spray => i::SPRAY_BOTTLE,
            Tool::Fill => i::DROP,
            Tool::Gradient => i::GRADIENT,
            Tool::Line => i::LINE_SEGMENT,
            Tool::Rectangle => i::RECTANGLE,
            Tool::Ellipse => i::CIRCLE,
            Tool::Curve => i::BEZIER_CURVE,
            Tool::ShapePolygon => i::HEXAGON,
            Tool::Contour => i::SCRIBBLE,
            Tool::Text => i::TEXT_AA,
            Tool::Picker => i::EYEDROPPER,
            Tool::Slice => i::CROP,
            Tool::Blur => i::DROP_HALF,
            Tool::Jumble => i::SHUFFLE,
        }
    }

    /// Default single-key shortcut (README / `Shortcuts.swift`).
    pub fn key(self) -> &'static str {
        match self {
            Tool::Marquee => "M",
            Tool::Lasso => "Q",
            Tool::Polygon => "P",
            Tool::Wand => "W",
            Tool::Move => "V",
            Tool::Pencil => "B",
            Tool::Eraser => "E",
            Tool::Spray => "A",
            Tool::Fill => "G",
            Tool::Gradient => "⇧G",
            Tool::Line => "L",
            Tool::Rectangle => "U",
            Tool::Ellipse => "O",
            Tool::Curve => "C",
            Tool::ShapePolygon => "⇧P",
            Tool::Contour => "D",
            Tool::Text => "T",
            Tool::Picker => "I",
            Tool::Slice => "S",
            Tool::Blur => "R",
            Tool::Jumble => "⇧R",
        }
    }
}

/// Per-document view state (one tab).
pub struct DocTab {
    pub doc: Doc,
    pub layer: u32,
    pub frame: u32,
    /// Screen points per sprite pixel.
    pub zoom: f32,
    /// Offset of the sprite's center from the canvas area's center, in points.
    pub pan: egui::Vec2,
    pub playing: bool,
    /// Bumped after every change so the canvas re-uploads its texture.
    pub revision: u64,
}

impl DocTab {
    pub fn new(doc: Doc) -> DocTab {
        let layer = doc.layer_count().saturating_sub(1);
        DocTab { doc, layer, frame: 0, zoom: 8.0, pan: egui::Vec2::ZERO, playing: false, revision: 0 }
    }

    /// Tab title: file name, or "Untitled".
    pub fn title(&self) -> String {
        self.doc
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into())
    }

    /// Call after any change to the document.
    pub fn changed(&mut self) {
        self.revision += 1;
    }
}

pub struct PixmithApp {
    pub tabs: Vec<DocTab>,
    pub active: usize,
    pub tool: Tool,
    /// Drawing colors as 0xRRGGBBAA, like the core.
    pub primary: u32,
    pub secondary: u32,
    pub brush_size: u32,
    pub dark: bool,
    /// A message for the error dialog.
    pub error: Option<String>,
    /// UI state owned by each region; each lives next to the code that uses it.
    pub canvas: ui::canvas::CanvasState,
    pub inspector: ui::inspector::InspectorState,
    pub timeline: ui::timeline::TimelineState,
    pub files: ui::title_row::FilesState,
    pub selection: ui::selection::SelectionState,
    pub tilemap: ui::tilemap::TilemapState,
    pub gamedev: ui::gamedev::GamedevState,
    /// The menu bar, preferences, shortcuts and the dialogs opened from the menus.
    pub menus: ui::menus::MenusState,
    /// The Home screen, project view and project switcher.
    pub projects: ui::projects::ProjectsState,
    /// The Extensions tab of Preferences.
    pub extensions: ui::extensions::ExtensionsState,
    /// Live mode: the server other tools connect to, and the ids and revisions of the open documents.
    pub live: crate::live::LiveState,
}

impl PixmithApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> PixmithApp {
        theme::install(&cc.egui_ctx);
        let mut app = PixmithApp {
            menus: ui::menus::MenusState::load(cc.storage),
            projects: ui::projects::ProjectsState::load(cc.storage),
            ..PixmithApp::default()
        };
        app.inspector.picker.load(cc.storage);
        // Pixmith opens on Home; opening a file straight away (command line, drop) goes to the editor instead.
        app.go_home();
        ui::menus::apply_appearance(&mut app, &cc.egui_ctx);
        app.canvas.tools.enable_persistence(ui::tools::view::ViewSettings::default_path());
        app.gamedev.start();
        app.extensions.plugins.enabled = true;
        // Pixmith Live follows the preference (on by default); the server starts with the first frame.
        app.live.managed = true;
        app
    }

    pub fn tab(&self) -> &DocTab {
        &self.tabs[self.active]
    }

    pub fn tab_mut(&mut self) -> &mut DocTab {
        &mut self.tabs[self.active]
    }

    pub fn colors(&self) -> Colors {
        if self.dark { Colors::dark() } else { Colors::light() }
    }

    /// Lays out the whole window. Panels are added outside-in, which is how egui assigns space.
    pub fn ui(&mut self, root: &mut egui::Ui) {
        crate::live::frame(self, root.ctx());
        let doc_id = self.tab().doc.raw() as usize;
        self.tilemap.sync(self.tool, doc_id);
        ui::menus::shortcuts_first(self, root.ctx());
        let c = self.colors();
        root.painter().rect_filled(root.max_rect(), 0.0, c.panel);
        let bar = |fill| egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(theme::space::MD as i8, 0));
        egui::Panel::top("title_row")
            .exact_size(metric::BAR_HEIGHT)
            .resizable(false)
            .frame(bar(c.panel))
            .show(root, |ui| ui::title_row::show(self, ui));
        ui::projects::tick(self, root.ctx());
        if self.projects.is_editor() {
            egui::Panel::left("tool_rail")
                .exact_size(metric::TOOL_RAIL)
                .resizable(false)
                .frame(egui::Frame::new().fill(c.panel))
                .show(root, |ui| ui::tool_rail::show(self, ui));
            egui::Panel::right("inspector")
                .exact_size(metric::INSPECTOR_WIDTH)
                .resizable(false)
                .frame(egui::Frame::new().fill(c.panel))
                .show(root, |ui| ui::inspector::show(self, ui));
            egui::Panel::top("context_bar")
                .exact_size(metric::BAR_HEIGHT)
                .resizable(false)
                .frame(bar(c.panel))
                .show(root, |ui| ui::context_bar::show(self, ui));
            if let Some(height) = ui::transform::bar_height(self, root.available_width(), root.ctx()) {
                egui::Panel::top("transform_bar")
                    .exact_size(height)
                    .resizable(false)
                    .frame(bar(c.panel))
                    .show(root, |ui| ui::transform::show_bar(self, ui));
            }
            egui::Panel::bottom("timeline")
                .exact_size(ui::timeline::panel_height(self))
                .resizable(false)
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(c.panel))
                .show(root, |ui| ui::timeline::show(self, ui));
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(self.menus.prefs.canvas_background().unwrap_or(c.workspace)))
                .show(root, |ui| ui::canvas::show(self, ui));
        } else {
            ui::projects::show_screen(self, root);
        }
        ui::dialogs::show(self, root.ctx());
        ui::selection::dialogs(self, root.ctx());
        ui::tilemap::show_sheets(self, root.ctx());
        ui::tools::show_windows(self, root.ctx());
        ui::gamedev::show(self, root.ctx());
        ui::lua_dialog::show(self, root.ctx());
        ui::menus::show(self, root.ctx());
        ui::extensions::show(self, root.ctx());
    }
}

impl Default for PixmithApp {
    fn default() -> PixmithApp {
        PixmithApp {
            tabs: vec![DocTab::new(Doc::new(32, 32))],
            active: 0,
            tool: Tool::Pencil,
            primary: 0x000000FF,
            secondary: 0xFFFFFFFF,
            brush_size: 1,
            dark: true,
            error: None,
            canvas: Default::default(),
            inspector: Default::default(),
            timeline: Default::default(),
            files: Default::default(),
            selection: Default::default(),
            tilemap: Default::default(),
            gamedev: Default::default(),
            menus: Default::default(),
            projects: Default::default(),
            extensions: Default::default(),
            live: Default::default(),
        }
    }
}

impl eframe::App for PixmithApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        PixmithApp::ui(self, ui);
    }

    /// A clean exit leaves no recovery copies behind.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.gamedev.lua.stop_all();
        self.live.stop();
        self.discard_recovery_copies();
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.menus.save(storage);
        self.projects.save(storage);
        self.inspector.picker.save(storage);
    }
}
