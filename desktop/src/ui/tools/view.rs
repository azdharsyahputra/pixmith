//! View-only state and the canvas layers it controls: onion skin, frame difference, tiled mode, pixel and custom grids, the
//! reference image (`ViewSettings.swift`, `CanvasOverlays.swift`, `RenderCache.swift`'s tinted frames).
//!
//! Nothing here is saved in the `.aseprite` file. The settings persist between runs in a small text file next to
//! the other per-user data (`ViewSettings.persist` in the macOS app uses `UserDefaults`).

use crate::app::DocTab;
use crate::ui::iso;
use eframe::egui::{self, Color32, Pos2, Rect, TextureHandle, TextureId, Vec2};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub use crate::ui::canvas::PAD;
/// Most sprite copies drawn on each side of the center one (`CanvasLayout.maxCopies`).
pub const MAX_COPIES: u32 = 64;

// MARK: Settings types

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TiledMode {
    #[default]
    None,
    X,
    Y,
    Both,
}

impl TiledMode {
    pub const ALL: [TiledMode; 4] = [TiledMode::None, TiledMode::X, TiledMode::Y, TiledMode::Both];

    pub fn title(self) -> &'static str {
        match self {
            TiledMode::None => "None",
            TiledMode::X => "Tiled in X Axis",
            TiledMode::Y => "Tiled in Y Axis",
            TiledMode::Both => "Tiled in Both Axes",
        }
    }

    pub fn tiles_x(self) -> bool {
        matches!(self, TiledMode::X | TiledMode::Both)
    }

    pub fn tiles_y(self) -> bool {
        matches!(self, TiledMode::Y | TiledMode::Both)
    }

    fn key(self) -> &'static str {
        match self {
            TiledMode::None => "none",
            TiledMode::X => "x",
            TiledMode::Y => "y",
            TiledMode::Both => "both",
        }
    }

    fn from_key(s: &str) -> Option<TiledMode> {
        TiledMode::ALL.into_iter().find(|m| m.key() == s)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReferencePlacement {
    #[default]
    Behind,
    Above,
}

impl ReferencePlacement {
    pub const ALL: [ReferencePlacement; 2] = [ReferencePlacement::Behind, ReferencePlacement::Above];

    pub fn title(self) -> &'static str {
        match self {
            ReferencePlacement::Behind => "Behind sprite",
            ReferencePlacement::Above => "Above sprite",
        }
    }
}

/// Zoom of the preview window (`PreviewZoom`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewZoom {
    X1,
    X2,
    X4,
    X8,
    Fit,
}

impl PreviewZoom {
    pub const ALL: [PreviewZoom; 5] =
        [PreviewZoom::X1, PreviewZoom::X2, PreviewZoom::X4, PreviewZoom::X8, PreviewZoom::Fit];

    pub fn title(self) -> &'static str {
        match self {
            PreviewZoom::X1 => "1x",
            PreviewZoom::X2 => "2x",
            PreviewZoom::X4 => "4x",
            PreviewZoom::X8 => "8x",
            PreviewZoom::Fit => "Fit",
        }
    }

    pub fn scale(self) -> Option<u32> {
        match self {
            PreviewZoom::X1 => Some(1),
            PreviewZoom::X2 => Some(2),
            PreviewZoom::X4 => Some(4),
            PreviewZoom::X8 => Some(8),
            PreviewZoom::Fit => None,
        }
    }
}

/// The kind of the custom grid: square cells, or isometric diamonds (`ui::iso`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridType {
    #[default]
    Rectangular,
    Isometric,
}

impl GridType {
    pub const ALL: [GridType; 2] = [GridType::Rectangular, GridType::Isometric];

    pub fn title(self) -> &'static str {
        match self {
            GridType::Rectangular => "Rectangular",
            GridType::Isometric => "Isometric",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            GridType::Rectangular => "rectangular",
            GridType::Isometric => "isometric",
        }
    }

    pub fn from_key(key: &str) -> Option<GridType> {
        GridType::ALL.into_iter().find(|t| t.key() == key)
    }
}

/// How the onion skin tints a frame (`OnionTint`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OnionTint {
    None,
    Past,
    Future,
}

/// Everything the View Options window edits (`ViewSettings`). The on/off switches of the onion skin, the frame
/// difference and the pixel grid live in the timeline state, like `SpriteModel.onionSkin` / `showGrid`.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewSettings {
    pub onion_past: u32,
    pub onion_future: u32,
    pub onion_opacity: f64,
    pub onion_falloff: f64,
    pub onion_tint: bool,
    /// Opacity of the frame-difference tint.
    pub diff_opacity: f64,
    /// Compare only the active layer instead of the composite of the visible layers.
    pub diff_active_layer: bool,
    /// The first frame of a tag is compared with the tag's last one (a looping animation; frames outside tags wrap
    /// over the document); otherwise it has nothing to compare with.
    pub diff_wrap: bool,
    pub tiled: TiledMode,
    pub grid_color: [u8; 3],
    pub grid_opacity: f64,
    pub grid_min_zoom: u32,
    pub custom_grid: bool,
    pub custom_grid_w: i32,
    pub custom_grid_h: i32,
    pub custom_grid_x: i32,
    pub custom_grid_y: i32,
    /// Rectangular cells (the size and offset above) or isometric diamonds (the iso fields below).
    pub custom_grid_type: GridType,
    /// Isometric tile width and height (even, 4 or more) and the offset of the tile at the origin.
    pub iso_grid_w: i32,
    pub iso_grid_h: i32,
    pub iso_grid_x: i32,
    pub iso_grid_y: i32,
    pub custom_grid_color: [u8; 3],
    pub custom_grid_opacity: f64,
    pub show_reference: bool,
    pub reference_opacity: f64,
    pub reference_placement: ReferencePlacement,
    pub reference_x: i32,
    pub reference_y: i32,
    pub reference_scale: f64,
    pub preview_zoom: PreviewZoom,
}

impl Default for ViewSettings {
    fn default() -> ViewSettings {
        ViewSettings {
            onion_past: 1,
            onion_future: 0,
            onion_opacity: 0.3,
            onion_falloff: 0.6,
            onion_tint: false,
            diff_opacity: 0.65,
            diff_active_layer: false,
            diff_wrap: true,
            tiled: TiledMode::None,
            // `Prefs.Default.gridColor` is 0x0000001F: black, and the 12% opacity below.
            grid_color: [0, 0, 0],
            grid_opacity: 0.12,
            grid_min_zoom: 6,
            custom_grid: false,
            custom_grid_w: 16,
            custom_grid_h: 16,
            custom_grid_x: 0,
            custom_grid_y: 0,
            custom_grid_type: GridType::Rectangular,
            iso_grid_w: iso::DEFAULT_W,
            iso_grid_h: iso::DEFAULT_H,
            iso_grid_x: 0,
            iso_grid_y: 0,
            custom_grid_color: [26, 128, 255],
            custom_grid_opacity: 0.6,
            show_reference: true,
            reference_opacity: 0.5,
            reference_placement: ReferencePlacement::Behind,
            reference_x: 0,
            reference_y: 0,
            reference_scale: 100.0,
            preview_zoom: PreviewZoom::X2,
        }
    }
}

fn hex(c: [u8; 3]) -> String {
    format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

impl ViewSettings {
    /// The `key=value` text the settings are saved as. The reference image and its placement are not saved
    /// (`ViewSettings.Snapshot` leaves them out as well).
    pub fn to_text(&self) -> String {
        let zoom = PreviewZoom::ALL.iter().position(|z| *z == self.preview_zoom).unwrap_or(1);
        [
            format!("onionPast={}", self.onion_past),
            format!("onionFuture={}", self.onion_future),
            format!("onionOpacity={}", self.onion_opacity),
            format!("onionFalloff={}", self.onion_falloff),
            format!("onionTint={}", self.onion_tint),
            format!("diffOpacity={}", self.diff_opacity),
            format!("diffActiveLayer={}", self.diff_active_layer),
            format!("diffWrap={}", self.diff_wrap),
            format!("tiled={}", self.tiled.key()),
            format!("gridColor={}", hex(self.grid_color)),
            format!("gridOpacity={}", self.grid_opacity),
            format!("gridMinZoom={}", self.grid_min_zoom),
            format!("customGrid={}", self.custom_grid),
            format!("customGridW={}", self.custom_grid_w),
            format!("customGridH={}", self.custom_grid_h),
            format!("customGridX={}", self.custom_grid_x),
            format!("customGridY={}", self.custom_grid_y),
            format!("customGridType={}", self.custom_grid_type.key()),
            format!("isoGridW={}", self.iso_grid_w),
            format!("isoGridH={}", self.iso_grid_h),
            format!("isoGridX={}", self.iso_grid_x),
            format!("isoGridY={}", self.iso_grid_y),
            format!("customGridColor={}", hex(self.custom_grid_color)),
            format!("customGridOpacity={}", self.custom_grid_opacity),
            format!("previewZoom={zoom}"),
            String::new(),
        ]
        .join("\n")
    }

    /// Reads what `to_text` wrote; unknown or malformed lines keep the current value.
    pub fn apply_text(&mut self, text: &str) {
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (f, b, i) = (v.parse::<f64>().ok(), v.parse::<bool>().ok(), v.parse::<i32>().ok());
            match k {
                "onionPast" => self.onion_past = i.unwrap_or(self.onion_past as i32).clamp(0, 3) as u32,
                "onionFuture" => self.onion_future = i.unwrap_or(self.onion_future as i32).clamp(0, 3) as u32,
                "onionOpacity" => self.onion_opacity = f.unwrap_or(self.onion_opacity),
                "onionFalloff" => self.onion_falloff = f.unwrap_or(self.onion_falloff),
                "onionTint" => self.onion_tint = b.unwrap_or(self.onion_tint),
                "diffOpacity" => self.diff_opacity = f.unwrap_or(self.diff_opacity).clamp(0.1, 1.0),
                "diffActiveLayer" => self.diff_active_layer = b.unwrap_or(self.diff_active_layer),
                "diffWrap" => self.diff_wrap = b.unwrap_or(self.diff_wrap),
                "tiled" => self.tiled = TiledMode::from_key(v).unwrap_or(self.tiled),
                "gridColor" => self.grid_color = parse_hex(v).unwrap_or(self.grid_color),
                "gridOpacity" => self.grid_opacity = f.unwrap_or(self.grid_opacity),
                "gridMinZoom" => self.grid_min_zoom = i.unwrap_or(self.grid_min_zoom as i32).clamp(1, 64) as u32,
                "customGrid" => self.custom_grid = b.unwrap_or(self.custom_grid),
                "customGridW" => self.custom_grid_w = i.unwrap_or(self.custom_grid_w),
                "customGridH" => self.custom_grid_h = i.unwrap_or(self.custom_grid_h),
                "customGridX" => self.custom_grid_x = i.unwrap_or(self.custom_grid_x),
                "customGridY" => self.custom_grid_y = i.unwrap_or(self.custom_grid_y),
                "customGridType" => self.custom_grid_type = GridType::from_key(v).unwrap_or(self.custom_grid_type),
                "isoGridW" => self.iso_grid_w = i.map_or(self.iso_grid_w, iso::clamp_side),
                "isoGridH" => self.iso_grid_h = i.map_or(self.iso_grid_h, iso::clamp_side),
                "isoGridX" => self.iso_grid_x = i.unwrap_or(self.iso_grid_x),
                "isoGridY" => self.iso_grid_y = i.unwrap_or(self.iso_grid_y),
                "customGridColor" => self.custom_grid_color = parse_hex(v).unwrap_or(self.custom_grid_color),
                "customGridOpacity" => self.custom_grid_opacity = f.unwrap_or(self.custom_grid_opacity),
                "previewZoom" => {
                    self.preview_zoom =
                        i.and_then(|n| PreviewZoom::ALL.get(n as usize).copied()).unwrap_or(self.preview_zoom)
                }
                _ => {}
            }
        }
    }

    /// The isometric grid these settings describe.
    pub fn iso(&self) -> iso::Iso {
        iso::Iso::new(self.iso_grid_w, self.iso_grid_h, self.iso_grid_x, self.iso_grid_y)
    }

    /// Where the settings are kept: `PIXMITH_SUPPORT_DIR` if set, else the platform's per-user data folder.
    pub fn default_path() -> Option<PathBuf> {
        let dir = if let Some(d) = std::env::var_os("PIXMITH_SUPPORT_DIR") {
            PathBuf::from(d)
        } else if cfg!(target_os = "macos") {
            PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Pixmith")
        } else if cfg!(windows) {
            PathBuf::from(std::env::var_os("APPDATA")?).join("Pixmith")
        } else if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
            PathBuf::from(x).join("pixmith")
        } else {
            PathBuf::from(std::env::var_os("HOME")?).join(".config/pixmith")
        };
        Some(dir.join("view-settings.txt"))
    }
}

// MARK: Reference image

/// The loaded reference image (not saved with the sprite).
pub struct Reference {
    pub name: String,
    pub size: (u32, u32),
    pub texture: TextureHandle,
}

/// Decodes `path` as an image into straight-alpha RGBA.
pub fn read_image(path: &std::path::Path) -> Result<(u32, u32, Vec<u8>), String> {
    let img = image::open(path).map_err(|_| {
        format!("Could not read {} as an image.", path.file_name().unwrap_or_default().to_string_lossy())
    })?;
    let rgba = img.to_rgba8();
    Ok((rgba.width(), rgba.height(), rgba.into_raw()))
}

// MARK: Layout of tiled copies

/// Sprite copies needed on each side of the center sprite so that `viewport` is covered at any pan
/// (`CanvasLayout.copies`).
pub fn copies(zoom: f32, w: u32, h: u32, tiled: TiledMode, viewport: Vec2) -> (u32, u32) {
    let n = |on: bool, view: f32, size: u32| -> u32 {
        if on { ((view / (size as f32 * zoom).max(1.0)).ceil().max(1.0) as u32).min(MAX_COPIES) } else { 0 }
    };
    (n(tiled.tiles_x(), viewport.x, w), n(tiled.tiles_y(), viewport.y, h))
}

/// Space around the sprite that can be panned into (`CanvasLayout.margins`). Tiled axes need no padding: the
/// copies already reach past the viewport.
pub fn margins(zoom: f32, w: u32, h: u32, copies: (u32, u32)) -> Vec2 {
    Vec2::new(
        if copies.0 > 0 { copies.0 as f32 * w as f32 * zoom } else { PAD },
        if copies.1 > 0 { copies.1 as f32 * h as f32 * zoom } else { PAD },
    )
}

// MARK: Onion skin cache

/// Rendered frames of the onion skin, as textures: `(frame, tint)` of the document revision they were made for.
#[derive(Default)]
pub struct OnionCache {
    key: Option<(usize, u64, u32, u32)>,
    textures: HashMap<(u32, OnionTint), TextureHandle>,
    /// What to draw this frame: farthest first, with its opacity.
    layers: Vec<(TextureId, f32)>,
    buf: Vec<u8>,
}

/// Tint colors of past and future frames (`RenderCache.tinted`): 55% of red or blue over the frame's pixels.
fn tint_rgb(t: OnionTint) -> Option<[f32; 3]> {
    match t {
        OnionTint::None => None,
        OnionTint::Past => Some([1.0, 0.15, 0.15]),
        OnionTint::Future => Some([0.15, 0.35, 1.0]),
    }
}

fn tinted(px: &mut [u8], t: OnionTint) {
    let Some(c) = tint_rgb(t) else { return };
    for p in px.as_chunks_mut::<4>().0.iter_mut() {
        if p[3] == 0 {
            continue;
        }
        for k in 0..3 {
            p[k] = (p[k] as f32 * 0.45 + c[k] * 255.0 * 0.55).round().min(255.0) as u8;
        }
    }
}

impl OnionCache {
    /// Works out which frames to draw and makes sure their textures exist. While `busy` (a stroke is being
    /// drawn) textures that are already cached are kept even if the document moved on.
    pub fn prepare(&mut self, ctx: &egui::Context, tab: &DocTab, s: &ViewSettings, on: bool, busy: bool) {
        self.layers.clear();
        let n = tab.doc.frame_count();
        if !on || n <= 1 {
            return;
        }
        let key = (tab.doc.raw() as usize, tab.revision, tab.doc.width(), tab.doc.height());
        if self.key != Some(key) && !(busy && self.key.is_some_and(|k| k.0 == key.0 && k.2 == key.2 && k.3 == key.3)) {
            self.textures.clear();
            self.key = Some(key);
        }
        let mut seen = HashSet::from([tab.frame]);
        let mut wanted: Vec<(u32, u32, OnionTint)> = Vec::new();
        for step in 1..=3u32 {
            if step <= s.onion_past {
                let f = (tab.frame as i64 - step as i64).rem_euclid(n as i64) as u32;
                if seen.insert(f) {
                    wanted.push((f, step, if s.onion_tint { OnionTint::Past } else { OnionTint::None }));
                }
            }
            if step <= s.onion_future {
                let f = (tab.frame + step) % n;
                if seen.insert(f) {
                    wanted.push((f, step, if s.onion_tint { OnionTint::Future } else { OnionTint::None }));
                }
            }
        }
        // Farthest first so nearer frames sit on top.
        wanted.sort_by_key(|w| std::cmp::Reverse(w.1));
        let (w, h) = (tab.doc.width() as usize, tab.doc.height() as usize);
        for (frame, step, tint) in wanted {
            if !self.textures.contains_key(&(frame, tint)) {
                if !tab.doc.render(frame, &mut self.buf) {
                    continue;
                }
                tinted(&mut self.buf, tint);
                let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &self.buf);
                let name = format!("onion-{frame}-{tint:?}");
                self.textures.insert((frame, tint), ctx.load_texture(name, image, egui::TextureOptions::NEAREST));
            }
            if let Some(t) = self.textures.get(&(frame, tint)) {
                let alpha = s.onion_opacity * s.onion_falloff.powi(step as i32 - 1);
                self.layers.push((t.id(), alpha as f32));
            }
        }
    }
}

// MARK: Frame difference cache

/// The frame-difference tint of the current frame as a texture, and what it found.
#[derive(Default)]
pub struct DiffCache {
    key: Option<DiffKey>,
    texture: Option<TextureHandle>,
    /// What the last comparison found; `None` while the view is off or the frame has nothing to compare with.
    pub stats: Option<DiffStats>,
    /// The texture to draw this frame.
    shown: Option<TextureId>,
    mask: Vec<u8>,
}

/// What a comparison was made for: `(document, revision, frame, width, height, layer, wrap)`.
type DiffKey = (usize, u64, u32, u32, u32, Option<u32>, bool);

/// Pixel counts of a frame difference (`pixmith_core::render::diff::PixmithFrameDiff`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiffStats {
    pub changed: u32,
    pub appeared: u32,
    pub disappeared: u32,
    /// The frame compared against, `None` when the frame has no previous one.
    pub reference: Option<u32>,
}

impl DiffStats {
    pub fn total(&self) -> u32 {
        self.changed + self.appeared + self.disappeared
    }

    /// Text of the canvas status chip.
    pub fn chip(&self) -> String {
        match (self.reference, self.total()) {
            (None, _) => "Diff: no previous frame".to_string(),
            (Some(_), 0) => "Diff: no change".to_string(),
            (Some(_), n) => {
                format!("Diff {n} px: {} changed, {} new, {} gone", self.changed, self.appeared, self.disappeared)
            }
        }
    }
}

impl DiffCache {
    /// Compares the current frame with the previous one when the view is `on` and keeps the tint as a texture.
    /// While `busy` (a stroke is being drawn) the last result is kept even if the document moved on.
    pub fn prepare(&mut self, ctx: &egui::Context, tab: &DocTab, s: &ViewSettings, on: bool, busy: bool) {
        self.shown = None;
        if !on {
            self.stats = None;
            self.key = None;
            return;
        }
        let layer = s.diff_active_layer.then_some(tab.layer);
        let (w, h) = (tab.doc.width(), tab.doc.height());
        let key = (tab.doc.raw() as usize, tab.revision, tab.frame, w, h, layer, s.diff_wrap);
        let same_doc = self.key.is_some_and(|k| k.0 == key.0 && k.3 == w && k.4 == h);
        if self.key != Some(key) && !(busy && same_doc && self.texture.is_some()) {
            self.key = Some(key);
            let found = tab.doc.frame_diff(tab.frame, layer, s.diff_wrap, &mut self.mask);
            self.stats = found.map(|d| DiffStats {
                changed: d.changed,
                appeared: d.appeared,
                disappeared: d.disappeared,
                reference: (d.reference_frame >= 0).then_some(d.reference_frame as u32),
            });
            if self.stats.is_some_and(|d| d.total() > 0) {
                let mut rgba = vec![0u8; self.mask.len() * 4];
                for (p, &k) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(&self.mask) {
                    if k != 0 {
                        let c = DIFF_COLORS[(k - 1).min(2) as usize];
                        p.copy_from_slice(&[c[0], c[1], c[2], 255]);
                    }
                }
                let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                match &mut self.texture {
                    Some(t) => t.set(image, egui::TextureOptions::NEAREST),
                    None => self.texture = Some(ctx.load_texture("frame-diff", image, egui::TextureOptions::NEAREST)),
                }
            }
        }
        if self.stats.is_some_and(|d| d.total() > 0) {
            self.shown = self.texture.as_ref().map(|t| t.id());
        }
    }
}

/// Magenta for changed pixels, green for pixels that appeared, red for pixels that disappeared.
pub const DIFF_COLORS: [[u8; 3]; 3] = pixmith_core::render::diff::DEFAULT_COLORS;

// MARK: Painting

/// What the canvas tells the view layers about the current frame.
pub struct Scene<'a> {
    pub settings: &'a ViewSettings,
    pub reference: Option<&'a Reference>,
    pub onion: &'a OnionCache,
    pub diff: &'a DiffCache,
    /// Texture of the current frame.
    pub sprite: Option<TextureId>,
    /// The visible canvas area and the center sprite's rectangle in it.
    pub area: Rect,
    pub srect: Rect,
    pub zoom: f32,
    pub size: (u32, u32),
    pub ppp: f32,
    pub dark: bool,
    pub show_grid: bool,
    /// Checkerboard colors from the preferences; `None` uses the theme's.
    pub checker: Option<(Color32, Color32)>,
    /// Pixel grid color from the preferences; `None` uses the view settings.
    pub grid: Option<Color32>,
    /// The grid of the active isometric tilemap layer; the isometric custom grid follows it instead of the
    /// settings' cell size and offset.
    pub iso: Option<iso::Iso>,
}

impl Scene<'_> {
    fn copies(&self) -> (u32, u32) {
        copies(self.zoom, self.size.0, self.size.1, self.settings.tiled, self.area.size())
    }

    /// Rectangle of every sprite copy (the center one included) that touches the canvas area.
    fn tiles(&self) -> Vec<Rect> {
        let (cx, cy) = self.copies();
        let (w, h) = (self.srect.width(), self.srect.height());
        if w <= 0.0 || h <= 0.0 {
            return Vec::new();
        }
        let range = |lo: f32, hi: f32, origin: f32, size: f32, n: u32| {
            let a = (((lo - origin) / size).floor() as i64).max(-(n as i64));
            let b = (((hi - origin) / size).floor() as i64).min(n as i64);
            a..=b
        };
        let xs = range(self.area.left(), self.area.right(), self.srect.left(), w, cx);
        let ys = range(self.area.top(), self.area.bottom(), self.srect.top(), h, cy);
        let mut out = Vec::new();
        for iy in ys {
            for ix in xs.clone() {
                out.push(self.srect.translate(Vec2::new(ix as f32 * w, iy as f32 * h)));
            }
        }
        out
    }

    /// Everything the checkerboard covers: the sprite plus neighbouring copies in tiled mode.
    fn region(&self) -> Rect {
        let (cx, cy) = self.copies();
        let (w, h) = (self.srect.width(), self.srect.height());
        Rect::from_min_max(
            self.srect.min - Vec2::new(cx as f32 * w, cy as f32 * h),
            self.srect.max + Vec2::new(cx as f32 * w, cy as f32 * h),
        )
    }
}

fn unit_uv() -> Rect {
    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))
}

/// Backdrop and sprite: shadow, checkerboard, reference (behind), onion skin, the sprite and its copies.
pub fn paint_under(painter: &egui::Painter, sc: &Scene) {
    let s = sc.settings;
    if s.tiled == TiledMode::None {
        // Soft drop shadow so the sprite reads as an object on the workspace.
        let shadow = egui::Shadow {
            offset: [0, 8],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(if sc.dark { 128 } else { 56 }),
        };
        painter.add(shadow.as_shape(sc.srect, 0.0));
    }

    // Checkerboard: 8 pt squares anchored at the sprite's corner.
    let (light, darker) = if let Some(pair) = sc.checker {
        pair
    } else if sc.dark {
        (Color32::from_rgb(0x3A, 0x3A, 0x3D), Color32::from_rgb(0x2F, 0x2F, 0x32))
    } else {
        (Color32::from_rgb(0xF3, 0xF3, 0xF5), Color32::from_rgb(0xDA, 0xDA, 0xDF))
    };
    let region = sc.region();
    let vis = region.intersect(sc.area);
    if vis.is_positive() {
        let mut mesh = egui::Mesh::default();
        mesh.add_colored_rect(vis, light);
        let cell = 8.0;
        let (i0, i1) = (
            ((vis.left() - region.left()) / cell).floor() as i32,
            ((vis.right() - region.left()) / cell).ceil() as i32,
        );
        let (j0, j1) =
            (((vis.top() - region.top()) / cell).floor() as i32, ((vis.bottom() - region.top()) / cell).ceil() as i32);
        for j in j0..j1 {
            for i in i0..i1 {
                if (i + j) % 2 == 0 {
                    let r = Rect::from_min_size(region.min + Vec2::new(i as f32, j as f32) * cell, Vec2::splat(cell));
                    mesh.add_colored_rect(r.intersect(vis), darker);
                }
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }

    if s.reference_placement == ReferencePlacement::Behind {
        paint_reference(painter, sc);
    }

    let tiles = sc.tiles();
    for (tex, alpha) in &sc.onion.layers {
        for t in &tiles {
            painter.image(
                *tex,
                *t,
                unit_uv(),
                Color32::from_white_alpha((alpha.clamp(0.0, 1.0) * 255.0).round() as u8),
            );
        }
    }
    if let Some(tex) = sc.sprite {
        for t in &tiles {
            painter.image(tex, *t, unit_uv(), Color32::WHITE);
        }
    }
    if let Some(tex) = sc.diff.shown {
        let tint = Color32::from_white_alpha((s.diff_opacity.clamp(0.0, 1.0) * 255.0).round() as u8);
        for t in &tiles {
            painter.image(tex, *t, unit_uv(), tint);
        }
    }
    if s.tiled != TiledMode::None {
        // `systemOrange` at 80%, around the center sprite.
        let orange = if sc.dark { Color32::from_rgb(0xFF, 0x9F, 0x0A) } else { Color32::from_rgb(0xFF, 0x95, 0x00) };
        painter.rect_stroke(
            sc.srect,
            0.0,
            egui::Stroke::new(1.0, orange.gamma_multiply(0.8)),
            egui::StrokeKind::Outside,
        );
    }
}

/// Reference (above), outline, pixel grid and custom grid.
pub fn paint_over(painter: &egui::Painter, sc: &Scene) {
    let s = sc.settings;
    if s.reference_placement == ReferencePlacement::Above {
        paint_reference(painter, sc);
    }
    let hairline = 1.0 / sc.ppp;
    if s.tiled == TiledMode::None {
        // One-pixel outline just outside the sprite.
        let outline = Color32::from_black_alpha(if sc.dark { 153 } else { 46 });
        painter.rect_stroke(sc.srect, 0.0, egui::Stroke::new(hairline, outline), egui::StrokeKind::Outside);
    }
    let vis = sc.srect.intersect(sc.area);
    if !vis.is_positive() {
        return;
    }
    if sc.show_grid && sc.zoom >= s.grid_min_zoom as f32 {
        let [r, g, b] = s.grid_color;
        let color = sc.grid.unwrap_or_else(|| {
            Color32::from_rgba_unmultiplied(r, g, b, (s.grid_opacity.clamp(0.0, 1.0) * 255.0).round() as u8)
        });
        grid_lines(painter, sc, vis, (sc.zoom, sc.zoom), (0.0, 0.0), color, hairline);
    }
    if s.custom_grid && s.custom_grid_type == GridType::Isometric {
        let [r, g, b] = s.custom_grid_color;
        let color =
            Color32::from_rgba_unmultiplied(r, g, b, (s.custom_grid_opacity.clamp(0.0, 1.0) * 255.0).round() as u8);
        iso::paint(painter, &sc.iso.unwrap_or_else(|| s.iso()), sc.srect.min, sc.zoom, sc.ppp, vis, color);
    } else if s.custom_grid && s.custom_grid_w > 0 && s.custom_grid_h > 0 {
        let [r, g, b] = s.custom_grid_color;
        let color =
            Color32::from_rgba_unmultiplied(r, g, b, (s.custom_grid_opacity.clamp(0.0, 1.0) * 255.0).round() as u8);
        let step = (s.custom_grid_w as f32 * sc.zoom, s.custom_grid_h as f32 * sc.zoom);
        let offset = (s.custom_grid_x as f32 * sc.zoom, s.custom_grid_y as f32 * sc.zoom);
        grid_lines(painter, sc, vis, step, offset, color, hairline.max(1.0));
    }
}

/// Lines every `step` points from `offset` (relative to the sprite's corner) inside `vis`, on whole device pixels.
fn grid_lines(
    painter: &egui::Painter,
    sc: &Scene,
    vis: Rect,
    step: (f32, f32),
    offset: (f32, f32),
    color: Color32,
    width: f32,
) {
    if step.0 <= 0.5 || step.1 <= 0.5 {
        return;
    }
    let snap = |v: f32| (v * sc.ppp).round() / sc.ppp;
    let mut mesh = egui::Mesh::default();
    let first = |lo: f32, origin: f32, step: f32| ((lo - origin) / step).ceil().max(-1.0e6) as i64;
    let mut i = first(vis.left() - 1.0, sc.srect.left() + offset.0, step.0);
    loop {
        let x = snap(sc.srect.left() + offset.0 + i as f32 * step.0);
        if x > vis.right() + 1.0 {
            break;
        }
        if x >= vis.left() - 1.0 {
            mesh.add_colored_rect(
                Rect::from_min_max(Pos2::new(x, vis.top()), Pos2::new((x + width).min(vis.right()), vis.bottom())),
                color,
            );
        }
        i += 1;
    }
    let mut j = first(vis.top() - 1.0, sc.srect.top() + offset.1, step.1);
    loop {
        let y = snap(sc.srect.top() + offset.1 + j as f32 * step.1);
        if y > vis.bottom() + 1.0 {
            break;
        }
        if y >= vis.top() - 1.0 {
            mesh.add_colored_rect(
                Rect::from_min_max(Pos2::new(vis.left(), y), Pos2::new(vis.right(), (y + width).min(vis.bottom()))),
                color,
            );
        }
        j += 1;
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_reference(painter: &egui::Painter, sc: &Scene) {
    let s = sc.settings;
    let Some(r) = sc.reference.filter(|_| s.show_reference) else { return };
    let k = (s.reference_scale / 100.0) as f32 * sc.zoom;
    let min = sc.srect.min + Vec2::new(s.reference_x as f32, s.reference_y as f32) * sc.zoom;
    let rect = Rect::from_min_size(min, Vec2::new(r.size.0 as f32, r.size.1 as f32) * k);
    if !rect.intersects(sc.area) {
        return;
    }
    let alpha = (s.reference_opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    painter.image(r.texture.id(), rect, unit_uv(), Color32::from_white_alpha(alpha));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_through_text() {
        let mut s = ViewSettings { onion_past: 3, tiled: TiledMode::Both, custom_grid: true, ..Default::default() };
        s.grid_color = [1, 2, 3];
        s.preview_zoom = PreviewZoom::Fit;
        s.custom_grid_x = -4;
        s.custom_grid_type = GridType::Isometric;
        s.iso_grid_w = 104;
        s.iso_grid_h = 56;
        s.iso_grid_x = -3;
        s.iso_grid_y = 7;
        let mut back = ViewSettings::default();
        back.apply_text(&s.to_text());
        assert_eq!(back, s);
    }

    #[test]
    fn frame_difference_settings_round_trip_and_clamp() {
        let d = ViewSettings::default();
        assert_eq!((d.diff_opacity, d.diff_active_layer, d.diff_wrap), (0.65, false, true));
        let s = ViewSettings { diff_opacity: 0.4, diff_active_layer: true, diff_wrap: false, ..Default::default() };
        let mut back = ViewSettings::default();
        back.apply_text(&s.to_text());
        assert_eq!(back, s);
        back.apply_text("diffOpacity=7\ndiffWrap=zebra");
        assert_eq!((back.diff_opacity, back.diff_wrap), (1.0, false), "clamped; malformed keeps the value");
    }

    #[test]
    fn diff_chip_text() {
        let none = DiffStats::default();
        assert_eq!(none.chip(), "Diff: no previous frame");
        let same = DiffStats { reference: Some(0), ..Default::default() };
        assert_eq!(same.chip(), "Diff: no change");
        let some = DiffStats { changed: 3, appeared: 5, disappeared: 4, reference: Some(0) };
        assert_eq!(some.total(), 12);
        assert_eq!(some.chip(), "Diff 12 px: 3 changed, 5 new, 4 gone");
    }

    #[test]
    fn isometric_grid_defaults_and_malformed_values() {
        let d = ViewSettings::default();
        assert_eq!(
            (d.custom_grid_type, d.iso_grid_w, d.iso_grid_h, d.iso_grid_x, d.iso_grid_y),
            (GridType::Rectangular, 32, 16, 0, 0)
        );
        let mut s = ViewSettings::default();
        s.apply_text("customGridType=hexagonal\nisoGridW=1\nisoGridH=15\nisoGridX=zebra");
        assert_eq!(s.custom_grid_type, GridType::Rectangular);
        assert_eq!(
            (s.iso_grid_w, s.iso_grid_h, s.iso_grid_x),
            (4, 16, 0),
            "sizes are clamped to even values of 4 or more"
        );
        assert_eq!(s.iso(), iso::Iso::new(4, 16, 0, 0));
    }

    #[test]
    fn malformed_lines_keep_defaults() {
        let mut s = ViewSettings::default();
        s.apply_text("onionPast=zebra\nnonsense\ntiled=diagonal\nonionFuture=9");
        assert_eq!(s.onion_past, 1);
        assert_eq!(s.tiled, TiledMode::None);
        assert_eq!(s.onion_future, 3, "out-of-range counts are clamped");
    }

    #[test]
    fn tiled_copies_cover_the_viewport() {
        let v = Vec2::new(800.0, 600.0);
        assert_eq!(copies(8.0, 32, 32, TiledMode::None, v), (0, 0));
        // 32 px at 8x is 256 pt wide: 4 copies to reach past 800 pt, 3 for 600 pt.
        assert_eq!(copies(8.0, 32, 32, TiledMode::Both, v), (4, 3));
        assert_eq!(copies(8.0, 32, 32, TiledMode::X, v), (4, 0));
        assert_eq!(copies(0.1, 1, 1, TiledMode::X, v), (MAX_COPIES, 0));
        assert_eq!(margins(8.0, 32, 32, (4, 0)), Vec2::new(1024.0, PAD));
    }

    #[test]
    fn tint_blends_toward_red_and_keeps_alpha() {
        let mut px = [200u8, 100, 50, 128, 0, 0, 0, 0];
        tinted(&mut px, OnionTint::Past);
        assert_eq!(px[3], 128);
        assert!(px[0] > 200 - 1 && px[1] < 100, "red rises, green falls: {px:?}");
        assert_eq!(&px[4..], &[0, 0, 0, 0], "transparent pixels stay transparent");
    }
}
