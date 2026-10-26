//! The FX filters (`Dialogs/FiltersView.swift`) and Color Curves (`Dialogs/CurvesEditor.swift`), with the same
//! live preview: while a sheet is open the canvas shows what the filter would do, recomputed by the core a
//! moment after a control stops moving.

use super::controls::{bar_button, color_row, divider, sheet, value_slider};
use crate::app::{DocTab, PixmithApp};
use crate::calls::FilterRegion;
use crate::doc::Doc;
use crate::theme::{Colors, font, radius, ui_font};
use crate::ui::timeline::playback;
use crate::ui::widgets::{self, DialogAction, SegGlyph};
use eframe::egui::{self, Align, Color32, Layout, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use egui_phosphor::regular as icon;

/// The filters offered in the FX menu. The ids match `pixmith_filters.h`; `Scale` is not a pixel filter and
/// has its own sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    ReplaceColor,
    Invert,
    Desaturate,
    Hsl,
    BrightnessContrast,
    ColorBalance,
    Outline,
    DropShadow,
    Despeckle,
    Blur,
    Sharpen,
    Curves,
    CleanEdges,
    LintSprite,
    Scale,
}

impl FilterKind {
    pub fn title(self) -> &'static str {
        match self {
            FilterKind::ReplaceColor => "Replace Color",
            FilterKind::Invert => "Invert",
            FilterKind::Desaturate => "Desaturate",
            FilterKind::Hsl => "Hue/Saturation/Lightness",
            FilterKind::BrightnessContrast => "Brightness/Contrast",
            FilterKind::ColorBalance => "Color Balance",
            FilterKind::Curves => "Color Curves",
            FilterKind::Outline => "Outline",
            FilterKind::DropShadow => "Drop Shadow",
            FilterKind::Despeckle => "Remove Isolated Pixels",
            FilterKind::Blur => "Blur",
            FilterKind::Sharpen => "Sharpen",
            FilterKind::CleanEdges => "Clean Edges",
            FilterKind::LintSprite => "Lint Sprite",
            FilterKind::Scale => "Scale Sprite",
        }
    }

    /// The Rust kind id of the core's filter.
    pub fn core_kind(self) -> u32 {
        match self {
            FilterKind::ReplaceColor => 0,
            FilterKind::Invert => 1,
            FilterKind::Desaturate => 2,
            FilterKind::Hsl => 3,
            FilterKind::BrightnessContrast => 4,
            FilterKind::ColorBalance => 5,
            FilterKind::Outline => 6,
            FilterKind::DropShadow => 7,
            FilterKind::Despeckle => 8,
            FilterKind::Blur => 9,
            FilterKind::Sharpen => 10,
            FilterKind::Curves => 11,
            FilterKind::CleanEdges => 12,
            FilterKind::Scale => 1000,
            FilterKind::LintSprite => 1001,
        }
    }
}

/// Which cels a filter touches (`FilterTarget`).
pub const TARGETS: [&str; 3] = ["Current cel", "All frames of this layer", "All layers of this frame"];

/// The five editable curves of Color Curves: RGB (master), R, G, B and A.
pub const CURVE_NAMES: [&str; 5] = ["RGB", "R", "G", "B", "A"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurveSet {
    pub channels: [Vec<(i32, i32)>; 5],
}

impl Default for CurveSet {
    fn default() -> CurveSet {
        CurveSet { channels: std::array::from_fn(|_| identity()) }
    }
}

fn identity() -> Vec<(i32, i32)> {
    vec![(0, 0), (255, 255)]
}

impl CurveSet {
    /// Parameter block for the curves filter: per curve a point count, then x, y pairs.
    pub fn encoded(&self) -> Vec<i32> {
        self.channels
            .iter()
            .flat_map(|pts| std::iter::once(pts.len() as i32).chain(pts.iter().flat_map(|&(x, y)| [x, y])))
            .collect()
    }
}

/// Everything the sheets can edit. Only the values relevant to the open filter are sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterParams {
    pub target: usize,
    pub from_color: u32,
    pub to_color: u32,
    pub tolerance: i32,
    pub hue: i32,
    pub saturation: i32,
    pub lightness: i32,
    pub brightness: i32,
    pub contrast: i32,
    pub red: i32,
    pub green: i32,
    pub blue: i32,
    pub outline_color: u32,
    pub inside: bool,
    pub eight: bool,
    pub shadow_x: i32,
    pub shadow_y: i32,
    pub shadow_color: u32,
    pub curves: CurveSet,
    pub clean: CleanParams,
}

/// The Clean Edges sheet: which passes run and their thresholds (`pixmith_filters.h`, `PIXMITH_FILTER_CLEAN_EDGES`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanParams {
    pub halo: bool,
    pub rings: i32,
    pub gain: i32,
    pub specks: bool,
    pub min_size: i32,
    pub holes: bool,
    pub hole_size: i32,
    pub light: bool,
    pub thin: bool,
    pub thickness: i32,
    pub luma: i32,
    pub fixed_color: bool,
    pub color: u32,
}

impl Default for CleanParams {
    fn default() -> CleanParams {
        CleanParams {
            halo: true,
            rings: 2,
            gain: 40,
            specks: true,
            min_size: 4,
            holes: false,
            hole_size: 2,
            light: false,
            thin: true,
            thickness: 3,
            luma: 70,
            fixed_color: false,
            color: 0x000000FF,
        }
    }
}

impl CleanParams {
    /// The parameter block of the core call.
    pub fn values(&self) -> Vec<i32> {
        let flags = self.halo as i32 | (self.specks as i32) << 1 | (self.thin as i32) << 2 | (self.light as i32) << 3;
        vec![
            flags,
            self.rings,
            self.gain,
            30,
            self.min_size,
            if self.holes { self.hole_size } else { 0 },
            self.thickness,
            self.luma,
            self.fixed_color as i32,
            self.color as i32,
            0,
        ]
    }
}

impl Default for FilterParams {
    fn default() -> FilterParams {
        FilterParams {
            target: 0,
            from_color: 0x000000FF,
            to_color: 0xFF0000FF,
            tolerance: 0,
            hue: 0,
            saturation: 0,
            lightness: 0,
            brightness: 0,
            contrast: 0,
            red: 0,
            green: 0,
            blue: 0,
            outline_color: 0x000000FF,
            inside: false,
            eight: false,
            shadow_x: 1,
            shadow_y: 1,
            shadow_color: 0x00000099,
            curves: CurveSet::default(),
            clean: CleanParams::default(),
        }
    }
}

impl FilterParams {
    /// The parameter block of the core call (`FilterParams.values`).
    pub fn values(&self, kind: FilterKind) -> Vec<i32> {
        let c = |v: u32| v as i32;
        match kind {
            FilterKind::ReplaceColor => vec![c(self.from_color), c(self.to_color), self.tolerance],
            FilterKind::Hsl => vec![self.hue, self.saturation, self.lightness],
            FilterKind::BrightnessContrast => vec![self.brightness, self.contrast],
            FilterKind::ColorBalance => vec![self.red, self.green, self.blue],
            FilterKind::Curves => self.curves.encoded(),
            FilterKind::Outline => vec![c(self.outline_color), self.inside as i32, self.eight as i32],
            FilterKind::DropShadow => vec![self.shadow_x, self.shadow_y, c(self.shadow_color)],
            FilterKind::Despeckle => vec![self.eight as i32],
            FilterKind::CleanEdges => self.clean.values(),
            FilterKind::LintSprite
            | FilterKind::Invert
            | FilterKind::Desaturate
            | FilterKind::Blur
            | FilterKind::Sharpen
            | FilterKind::Scale => Vec::new(),
        }
    }
}

/// An open filter sheet.
pub struct FilterSession {
    pub kind: FilterKind,
    pub p: FilterParams,
    region: FilterRegion,
    /// The parameters the canvas preview was last computed for.
    shown: Option<FilterParams>,
    /// When the pending preview is due (`input.time`); previews wait 60 ms for the controls to settle.
    due: Option<f64>,
    pixels: Vec<u8>,
    channel: usize,
    selected: Option<usize>,
    dragging: Option<i32>,
}

/// The Scale Sprite sheet (`ScaleSheet`).
pub struct ScaleSession {
    by_percent: bool,
    percent: u32,
    width: u32,
    height: u32,
    keep_aspect: bool,
}

impl ScaleSession {
    pub fn new(doc: &Doc) -> ScaleSession {
        ScaleSession { by_percent: true, percent: 200, width: doc.width(), height: doc.height(), keep_aspect: true }
    }

    fn new_size(&self, doc: &Doc) -> (u32, u32) {
        if self.by_percent {
            let f = |v: u32| ((v as f64 * self.percent as f64 / 100.0).round() as u32).max(1);
            (f(doc.width()), f(doc.height()))
        } else {
            (self.width, self.height)
        }
    }

    fn valid(&self, doc: &Doc) -> bool {
        let (w, h) = self.new_size(doc);
        (1..=8192).contains(&w) && (1..=8192).contains(&h) && (w != doc.width() || h != doc.height())
    }

    fn set_width(&mut self, v: u32) {
        let old = self.width.max(1) as f64;
        self.width = v;
        if self.keep_aspect {
            self.height = ((self.height as f64 * v as f64 / old).round() as u32).max(1);
        }
    }

    fn set_height(&mut self, v: u32) {
        let old = self.height.max(1) as f64;
        self.height = v;
        if self.keep_aspect {
            self.width = ((self.width as f64 * v as f64 / old).round() as u32).max(1);
        }
    }
}

impl FilterSession {
    /// Opens the sheet for `kind` on `tab`: the selection limits the filter, the drawing color starts as the
    /// "from" color of Replace Color.
    pub fn new(tab: &mut DocTab, kind: FilterKind, primary: u32) -> FilterSession {
        playback::stop(tab);
        let p = FilterParams { from_color: primary, ..Default::default() };
        FilterSession {
            kind,
            p,
            region: tab.doc.filter_region(),
            shown: None,
            due: Some(0.0),
            pixels: Vec::new(),
            channel: 0,
            selected: None,
            dragging: None,
        }
    }
}

// MARK: Sheet

/// Draws the open filter or scale sheet; call every frame.
pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    if let Some(mut s) = app.menus.filter.take() {
        if filter_sheet(app, ctx, &mut s) {
            clear_preview(app);
        } else {
            app.menus.filter = Some(s);
        }
    }
    if let Some(mut s) = app.menus.scale.take()
        && !scale_sheet(app, ctx, &mut s)
    {
        app.menus.scale = Some(s);
    }
}

fn clear_preview(app: &mut PixmithApp) {
    app.canvas.preview = None;
}

/// Draws the sheet; returns true once it is closed.
fn filter_sheet(app: &mut PixmithApp, ctx: &egui::Context, s: &mut FilterSession) -> bool {
    update_preview(app, ctx, s);
    let kind = s.kind;
    let action = sheet(ctx, "filter", 360.0, kind.title(), None, false, |ui| {
        controls(ui, s);
        divider(ui);
        widgets::form_row(ui, "Apply to", |ui| widgets::popup(ui, &TARGETS, &mut s.p.target, None));
        widgets::dialog_buttons(ui, None, Some("Cancel"), "Apply", true)
    });
    match action {
        DialogAction::Primary => {
            let PixmithApp { tabs, active, .. } = app;
            let tab = &mut tabs[*active];
            let params = s.p.values(kind);
            if tab.doc.filter_apply(
                kind.title(),
                kind.core_kind(),
                s.p.target as u32,
                tab.layer,
                tab.frame,
                s.region,
                &params,
            ) {
                tab.changed();
            }
            true
        }
        DialogAction::Cancel => true,
        _ => false,
    }
}

/// The controls of each filter (`FilterSheet.controls`).
fn controls(ui: &mut Ui, s: &mut FilterSession) {
    let kind = s.kind;
    let p = &mut s.p;
    match kind {
        FilterKind::ReplaceColor => {
            color_row(ui, "From", &mut p.from_color);
            color_row(ui, "To", &mut p.to_color);
            value_slider(ui, "tolerance", "Tolerance", &mut p.tolerance, 0..=255);
        }
        FilterKind::Hsl => {
            value_slider(ui, "hue", "Hue", &mut p.hue, -180..=180);
            value_slider(ui, "sat", "Saturation", &mut p.saturation, -100..=100);
            value_slider(ui, "light", "Lightness", &mut p.lightness, -100..=100);
        }
        FilterKind::BrightnessContrast => {
            value_slider(ui, "bright", "Brightness", &mut p.brightness, -100..=100);
            value_slider(ui, "contrast", "Contrast", &mut p.contrast, -100..=100);
        }
        FilterKind::ColorBalance => {
            value_slider(ui, "red", "Red", &mut p.red, -255..=255);
            value_slider(ui, "green", "Green", &mut p.green, -255..=255);
            value_slider(ui, "blue", "Blue", &mut p.blue, -255..=255);
        }
        FilterKind::Curves => curves_editor(ui, s),
        FilterKind::Outline => {
            color_row(ui, "Color", &mut p.outline_color);
            widgets::form_row(ui, "Position", |ui| {
                ui.push_id("position", |ui| {
                    if let Some(i) = widgets::bar_segmented(
                        ui,
                        p.inside as usize,
                        &[(SegGlyph::Text("Outside"), "Outside"), (SegGlyph::Text("Inside"), "Inside")],
                    ) {
                        p.inside = i == 1;
                    }
                });
            });
            connectivity(ui, &mut p.eight);
        }
        FilterKind::DropShadow => {
            color_row(ui, "Color", &mut p.shadow_color);
            value_slider(ui, "sx", "Offset X", &mut p.shadow_x, -16..=16);
            value_slider(ui, "sy", "Offset Y", &mut p.shadow_y, -16..=16);
        }
        FilterKind::Despeckle => {
            connectivity(ui, &mut p.eight);
            widgets::note(ui, "Clears pixels that have no opaque neighbor.");
        }
        FilterKind::Invert => widgets::note(ui, "Inverts the RGB channels, keeping alpha."),
        FilterKind::Desaturate => widgets::note(ui, "Converts colors to grayscale."),
        FilterKind::Blur => widgets::note(ui, "3x3 blur that ignores transparent pixels' color."),
        FilterKind::Sharpen => widgets::note(ui, "3x3 sharpen that ignores transparent pixels' color."),
        FilterKind::CleanEdges => clean_controls(ui, &mut p.clean),
        FilterKind::Scale | FilterKind::LintSprite => {}
    }
}

/// The Clean Edges controls: a checkbox per pass, with its thresholds below while it is on.
fn clean_controls(ui: &mut Ui, c: &mut CleanParams) {
    ui.checkbox(&mut c.halo, "Remove paper halo (light, grayish outer pixels)");
    if c.halo {
        value_slider(ui, "ce_rings", "Rings", &mut c.rings, 1..=4);
        value_slider(ui, "ce_gain", "Lighter than inside by", &mut c.gain, 10..=120);
    }
    ui.checkbox(&mut c.specks, "Remove detached specks");
    if c.specks {
        value_slider(ui, "ce_min", "Smaller than (px)", &mut c.min_size, 2..=32);
    }
    ui.checkbox(&mut c.holes, "Fill tiny enclosed holes");
    if c.holes {
        value_slider(ui, "ce_hole", "Smaller than (px)", &mut c.hole_size, 2..=16);
    }
    ui.checkbox(&mut c.light, "Replace lone light pixels inside dark areas");
    ui.checkbox(&mut c.thin, "Thin the outer outline to 1 px");
    if c.thin {
        value_slider(ui, "ce_thick", "Thinner than (px)", &mut c.thickness, 2..=8);
        ui.checkbox(&mut c.fixed_color, "Use a fixed outline color");
        if c.fixed_color {
            color_row(ui, "Outline color", &mut c.color);
        } else {
            value_slider(ui, "ce_luma", "Outline darkness", &mut c.luma, 0..=255);
        }
    }
    widgets::note(
        ui,
        "Small interior details (eyes, highlights, thin fingers) are kept. Holes and lone light pixels are off by default because they can be details.",
    );
}

fn connectivity(ui: &mut Ui, eight: &mut bool) {
    widgets::form_row(ui, "Connectivity", |ui| {
        ui.push_id("connectivity", |ui| {
            let options =
                [(SegGlyph::Text("4-connected"), "4-connected"), (SegGlyph::Text("8-connected"), "8-connected")];
            if let Some(i) = widgets::bar_segmented(ui, *eight as usize, &options) {
                *eight = i == 1;
            }
        });
    });
}

/// Recomputes the canvas preview when the parameters changed and have been still for 60 ms.
fn update_preview(app: &mut PixmithApp, ctx: &egui::Context, s: &mut FilterSession) {
    let now = ctx.input(|i| i.time);
    if s.shown.as_ref() != Some(&s.p) && s.due.is_none() {
        s.due = Some(now + 0.06);
    }
    match s.due {
        Some(due) if due <= now => {
            s.due = None;
            let PixmithApp { tabs, active, canvas, .. } = app;
            let tab = &tabs[*active];
            let params = s.p.values(s.kind);
            let ok = tab.doc.filter_preview(
                s.kind.core_kind(),
                s.p.target as u32,
                tab.layer,
                tab.frame,
                s.region,
                &params,
                &mut s.pixels,
            );
            s.shown = Some(s.p.clone());
            if ok {
                let size = [tab.doc.width() as usize, tab.doc.height() as usize];
                let image = egui::ColorImage::from_rgba_unmultiplied(size, &s.pixels);
                match &mut canvas.preview {
                    Some(t) => t.set(image, egui::TextureOptions::NEAREST),
                    None => {
                        canvas.preview = Some(ctx.load_texture("filter-preview", image, egui::TextureOptions::NEAREST))
                    }
                }
            } else {
                canvas.preview = None;
            }
            ctx.request_repaint();
        }
        Some(due) => ctx.request_repaint_after(std::time::Duration::from_secs_f64((due - now).max(0.001))),
        None => {}
    }
}

// MARK: Curves

const SIZE: f32 = 256.0;
/// How close a press must be to a point to grab it.
const GRAB: f32 = 10.0;

fn channel_color(channel: usize, c: &Colors) -> Color32 {
    match channel {
        0 => c.text,
        1 => Color32::from_rgb(0xFF, 0x45, 0x3A),
        2 => Color32::from_rgb(0x30, 0xD1, 0x58),
        3 => Color32::from_rgb(0x0A, 0x84, 0xFF),
        _ => Color32::from_gray(0x8E),
    }
}

/// Curve editor: click empty space to add a point, drag a point to move it, Option-click a point to remove it
/// (`CurvesEditor`).
fn curves_editor(ui: &mut Ui, s: &mut FilterSession) {
    let c = Colors::of(ui.ctx());
    ui.spacing_mut().item_spacing.y = 8.0;
    ui.push_id("curve_channels", |ui| {
        let options: Vec<(SegGlyph, &str)> = CURVE_NAMES.iter().map(|n| (SegGlyph::Text(n), *n)).collect();
        if let Some(i) = widgets::bar_segmented(ui, s.channel, &options) {
            s.channel = i;
            s.selected = None;
        }
    });

    let (rect, response) = ui.allocate_exact_size(vec2(SIZE, SIZE), Sense::click_and_drag());
    let (set, channel) = (&mut s.p.curves, s.channel);

    // Pointer: a press starts a drag; the point under it moves, or a new one is added there.
    let down = response.is_pointer_button_down_on();
    if down {
        let (press, now, alt) = ui.input(|i| (i.pointer.press_origin(), i.pointer.latest_pos(), i.modifiers.alt));
        if let (Some(start), Some(now)) = (press, now) {
            if s.dragging.is_none() {
                begin_drag(set, channel, rect, start, alt, &mut s.selected, &mut s.dragging);
            }
            if let Some(i) = s.dragging.filter(|&i| i >= 0) {
                let pts = &mut set.channels[channel];
                if (i as usize) < pts.len() {
                    let i = i as usize;
                    let mut p = position(rect, now);
                    let lo = if i > 0 { pts[i - 1].0 + 1 } else { 0 };
                    let hi = if i + 1 < pts.len() { pts[i + 1].0 - 1 } else { 255 };
                    p.0 = p.0.clamp(lo, hi.max(lo));
                    pts[i] = p;
                }
            }
        }
    } else {
        s.dragging = None;
    }

    paint_curve(ui, rect, &c, &set.channels[channel], channel, s.selected);

    let pts_len = set.channels[channel].len();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let can_remove = s.selected.is_some() && pts_len > 2;
        if bar_button(ui, "Remove Point", can_remove, false).clicked()
            && let Some(i) = s.selected
        {
            set.channels[channel].remove(i);
            s.selected = None;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let fresh = CurveSet::default();
            if bar_button(ui, "Reset All", *set != fresh, false).clicked() {
                *set = fresh;
                s.selected = None;
            }
            if bar_button(ui, "Reset Channel", set.channels[channel] != identity(), false).clicked() {
                set.channels[channel] = identity();
                s.selected = None;
            }
        });
    });
}

/// Image-space position (y up) of a pointer location in the canvas.
fn position(rect: Rect, p: Pos2) -> (i32, i32) {
    let x = ((p.x - rect.left()).round() as i32).clamp(0, 255);
    let y = ((SIZE - 1.0 - (p.y - rect.top())).round() as i32).clamp(0, 255);
    (x, y)
}

fn distance(rect: Rect, pt: (i32, i32), loc: Pos2) -> f32 {
    let (x, y) = (pt.0 as f32 - (loc.x - rect.left()), (SIZE - 1.0 - pt.1 as f32) - (loc.y - rect.top()));
    x.hypot(y)
}

/// The start of a drag (`drag(to:)` when `dragging` is nil).
fn begin_drag(
    set: &mut CurveSet,
    channel: usize,
    rect: Rect,
    start: Pos2,
    alt: bool,
    selected: &mut Option<usize>,
    dragging: &mut Option<i32>,
) {
    let pts = &mut set.channels[channel];
    let hit = (0..pts.len()).min_by(|&a, &b| distance(rect, pts[a], start).total_cmp(&distance(rect, pts[b], start)));
    if let Some(hit) = hit.filter(|&h| distance(rect, pts[h], start) <= GRAB) {
        if alt {
            if pts.len() > 2 {
                pts.remove(hit);
                *selected = None;
            }
            *dragging = Some(-1);
            return;
        }
        *dragging = Some(hit as i32);
    } else {
        let p = position(rect, start);
        if pts.iter().any(|q| q.0 == p.0) {
            *dragging = Some(-1);
            return;
        }
        let at = pts.iter().position(|q| q.0 > p.0).unwrap_or(pts.len());
        pts.insert(at, p);
        *dragging = Some(at as i32);
    }
    *selected = dragging.filter(|&d| d >= 0).map(|d| d as usize);
}

fn paint_curve(ui: &Ui, rect: Rect, c: &Colors, pts: &[(i32, i32)], channel: usize, selected: Option<usize>) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, radius::CONTROL, c.field);
    let faint = Stroke::new(1.0, c.text_secondary.gamma_multiply(0.25));
    for i in 1..4 {
        let v = i as f32 * SIZE / 4.0;
        painter.line_segment([pos2(rect.left() + v, rect.top()), pos2(rect.left() + v, rect.bottom())], faint);
        painter.line_segment([pos2(rect.left(), rect.top() + v), pos2(rect.right(), rect.top() + v)], faint);
    }
    painter.extend(egui::Shape::dashed_line(
        &[pos2(rect.left(), rect.bottom()), pos2(rect.right(), rect.top())],
        Stroke::new(1.0, c.text_secondary.gamma_multiply(0.4)),
        3.0,
        3.0,
    ));
    let color = channel_color(channel, c);
    let table = Doc::curve_lut(pts);
    let line: Vec<Pos2> =
        (0..256).map(|x| pos2(rect.left() + x as f32, rect.top() + SIZE - 1.0 - table[x] as f32)).collect();
    painter.add(egui::Shape::line(line, Stroke::new(2.0, color)));
    for (i, p) in pts.iter().enumerate() {
        let center = pos2(rect.left() + p.0 as f32, rect.top() + SIZE - 1.0 - p.1 as f32);
        let on = selected == Some(i);
        let r = if on { 5.0 } else { 4.0 };
        painter.circle(center, r, if on { c.accent } else { c.panel }, Stroke::new(1.5, color));
    }
}

// MARK: Scale

/// Draws the Scale Sprite sheet; returns true once it is closed.
fn scale_sheet(app: &mut PixmithApp, ctx: &egui::Context, s: &mut ScaleSession) -> bool {
    let c = Colors::of(ctx);
    let (w, h) = (app.tab().doc.width(), app.tab().doc.height());
    let (nw, nh) = s.new_size(&app.tab().doc);
    let valid = s.valid(&app.tab().doc);
    let action =
        sheet(ctx, "scale", 340.0, "Scale Sprite", Some("Nearest-neighbor, all frames and layers."), false, |ui| {
            ui.push_id("scale_mode", |ui| {
                let options = [(SegGlyph::Text("Percent"), "Percent"), (SegGlyph::Text("Size"), "Size")];
                if let Some(i) = widgets::bar_segmented(ui, if s.by_percent { 0 } else { 1 }, &options) {
                    s.by_percent = i == 0;
                }
            });
            if s.by_percent {
                widgets::form_row(ui, "Scale", |ui| {
                    widgets::number_field(ui, "%", &mut s.percent, 1..=10000, "%", 120.0);
                });
            } else {
                widgets::form_row(ui, "Width", |ui| {
                    let mut v = s.width;
                    if widgets::number_field(ui, "W", &mut v, 1..=8192, "px", 120.0) {
                        s.set_width(v);
                    }
                });
                widgets::form_row(ui, "Height", |ui| {
                    let mut v = s.height;
                    if widgets::number_field(ui, "H", &mut v, 1..=8192, "px", 120.0) {
                        s.set_height(v);
                    }
                });
                ui.checkbox(&mut s.keep_aspect, "Keep aspect ratio");
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(
                    egui::RichText::new(format!("{w} \u{d7} {h}")).font(ui_font(font::BODY)).color(c.text_secondary),
                );
                ui.label(egui::RichText::new(icon::ARROW_RIGHT).font(ui_font(10.0)).color(c.text_tertiary));
                ui.label(egui::RichText::new(format!("{nw} \u{d7} {nh}")).font(ui_font(font::BODY)).color(if valid {
                    c.text
                } else {
                    c.text_secondary
                }));
            });
            widgets::dialog_buttons(ui, None, Some("Cancel"), "Apply", valid)
        });
    match action {
        DialogAction::Primary => {
            let tab = app.tab_mut();
            playback::stop(tab);
            if tab.doc.scale_sprite(nw, nh) {
                tab.zoom = ((512.0 / nw.max(nh) as f32).floor()).clamp(1.0, 32.0);
                tab.changed();
            }
            true
        }
        DialogAction::Cancel => true,
        _ => false,
    }
}

/// Opens a sheet from the FX menu (`FXCommands.item`).
pub fn open(app_menus: &mut super::MenusState, tab: &mut DocTab, primary: u32, kind: FilterKind) {
    if kind == FilterKind::Scale {
        playback::stop(tab);
        app_menus.scale = Some(ScaleSession::new(&tab.doc));
    } else if kind == FilterKind::LintSprite {
        app_menus.lint = Some(super::lint::LintSession::new());
    } else {
        app_menus.filter = Some(FilterSession::new(tab, kind, primary));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_encode_like_the_core_expects() {
        let mut set = CurveSet::default();
        assert_eq!(set.encoded().len(), 5 * 5);
        assert_eq!(&set.encoded()[..5], &[2, 0, 0, 255, 255]);
        set.channels[1].insert(1, (128, 200));
        assert_eq!(&set.encoded()[5..12], &[3, 0, 0, 128, 200, 255, 255]);
    }

    #[test]
    fn clean_edges_block_matches_the_core_layout() {
        let p = FilterParams::default();
        assert_eq!(p.values(FilterKind::CleanEdges), vec![7, 2, 40, 30, 4, 0, 3, 70, 0, 0x000000FF, 0]);
        let mut q = FilterParams::default();
        q.clean.holes = true;
        q.clean.light = true;
        q.clean.halo = false;
        let v = q.values(FilterKind::CleanEdges);
        assert_eq!((v[0], v[5]), (2 | 4 | 8, 2));
    }

    #[test]
    fn filter_values_match_the_swift_blocks() {
        let p = FilterParams { hue: 10, saturation: -5, lightness: 3, ..Default::default() };
        assert_eq!(p.values(FilterKind::Hsl), vec![10, -5, 3]);
        assert_eq!(p.values(FilterKind::Invert), Vec::<i32>::new());
        let p = FilterParams { shadow_color: 0x00000099, ..Default::default() };
        assert_eq!(p.values(FilterKind::DropShadow), vec![1, 1, 0x99]);
        assert_eq!(
            FilterParams::default().values(FilterKind::ReplaceColor),
            vec![0x000000FFu32 as i32, 0xFF0000FFu32 as i32, 0]
        );
    }

    #[test]
    fn color_balance_uses_the_channel_offset_filter() {
        assert_eq!(FilterKind::ColorBalance.core_kind(), 5);
        assert_eq!(FilterKind::Curves.core_kind(), 11);
    }
}
