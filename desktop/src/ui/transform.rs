//! Free Transform: handles on the canvas, and the strip under the context bar with its numeric fields
//! (`FreeTransform.swift`: `TransformBar`, the handle drag and `drawTransformOverlay`).

use crate::app::{DocTab, PixmithApp, Tool};
use crate::theme::{self, Colors, metric, radius, space};
use crate::ui::selection::{SelectionState, XfDrag};
use crate::ui::widgets::{self, NumberOpts};
use eframe::egui::{self, Color32, Modifiers, Pos2, Rect, Stroke, StrokeKind};

/// Handle directions in the order of `pixmith_xf_geometry`: TL, T, TR, L, R, BL, B, BR.
const HANDLE_ORDER: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
/// Screen distance within which a click grabs a handle (`maxHitRadius`).
const MAX_HIT_RADIUS: f64 = 8.0;
const ANCHOR_NAMES: [&str; 9] =
    ["Top left", "Top", "Top right", "Left", "Center", "Right", "Bottom left", "Bottom", "Bottom right"];

/// A drag of a transform handle (`TransformDrag`).
#[derive(Clone, Copy, Debug)]
struct HandleDrag {
    kind: XfDrag,
    handle: (i32, i32),
    start: (f64, f64),
    doc: usize,
    last: Option<((f64, f64), bool)>,
}

/// Options of the free transform and the handle drag in progress.
#[derive(Clone, Debug)]
pub struct TransformState {
    /// Pixel-art rotation (`TransformOptions.rotsprite`).
    pub rotsprite: bool,
    /// Which box point typed width and height keep fixed: 0 to 8, row by row (4 is the center).
    pub anchor: usize,
    drag: Option<HandleDrag>,
}

impl Default for TransformState {
    fn default() -> TransformState {
        TransformState { rotsprite: false, anchor: 4, drag: None }
    }
}

impl TransformState {
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn drag_doc(&self) -> Option<usize> {
        self.drag.map(|d| d.doc)
    }

    pub fn reset_transients(&mut self) {
        self.drag = None;
    }

    fn anchor_direction(&self) -> (i32, i32) {
        ((self.anchor % 3) as i32 - 1, (self.anchor / 3) as i32 - 1)
    }
}

// ---- Commands ----

/// Edit > Free Transform (`startTransform`).
pub fn start(app: &mut PixmithApp) {
    let PixmithApp { tabs, active, selection: sel, .. } = app;
    begin(&sel.xf, &mut tabs[*active]);
}

fn begin(state: &TransformState, tab: &mut DocTab) -> bool {
    tab.playing = false;
    if tab.doc.xf_active() {
        return false;
    }
    let (layer, frame) = (tab.layer, tab.frame);
    if !tab.doc.xf_begin(layer, frame, state.rotsprite) {
        return false;
    }
    tab.doc.label_last_step("Free Transform");
    let a = state.anchor_direction();
    tab.doc.xf_set_anchor(a.0, a.1);
    tab.doc.touch();
    tab.changed();
    true
}

/// Applies the transform (Return, or Apply in the strip).
pub fn commit(tab: &mut DocTab) {
    if tab.doc.xf_active() {
        tab.doc.xf_commit();
        tab.doc.touch();
        tab.changed();
    }
}

/// Discards the transform (Esc, or Cancel in the strip).
pub fn cancel(tab: &mut DocTab) {
    if tab.doc.xf_cancel() {
        tab.doc.touch();
        tab.changed();
    }
}

/// Moves the transform by `(dx, dy)` canvas pixels (arrow keys).
pub fn nudge(tab: &mut DocTab, dx: f64, dy: f64) {
    tab.doc.xf_drag_begin();
    tab.doc.xf_drag(XfDrag::Move, (0, 0), false, (0.0, 0.0), (dx, dy));
    tab.doc.touch();
    tab.changed();
}

/// Whether a transform is running in the active document (shows the strip).
pub fn active(app: &PixmithApp) -> bool {
    app.tab().doc.xf_active()
}

// ---- Pointer ----

type Pt = (f64, f64);

fn dist(a: Pt, b: Pt) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Handles of the selection's bounding box, shown with the move tool (`selectionHandles`).
fn selection_handles(tab: &DocTab, tool: Tool) -> Option<[Pt; 8]> {
    if tool != Tool::Move || tab.doc.xf_active() {
        return None;
    }
    let [x, y, w, h] = tab.doc.sel_bounds()?;
    let (x, y, w, h) = (x as f64, y as f64, w as f64, h as f64);
    Some(HANDLE_ORDER.map(|d| (x + (d.0 + 1) as f64 / 2.0 * w, y + (d.1 + 1) as f64 / 2.0 * h)))
}

/// Screen distance that grabs a handle: 8 points, but never more than a third of the box's shorter side.
fn hit_radius(h: &[Pt; 8], zoom: f32) -> f64 {
    let side = dist(h[2], h[0]).min(dist(h[5], h[0])) * zoom as f64;
    (side / 3.0).clamp(1.0, MAX_HIT_RADIUS)
}

/// Whether `p` is inside the quadrilateral of the corner handles.
fn contains(h: &[Pt; 8], p: Pt) -> bool {
    let poly = [h[0], h[2], h[7], h[5]];
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// A press while a transform is active, or on a handle of the move tool's selection box (`transformMouseDown`).
/// Returns false when it is not for the transform.
pub fn press(
    sel: &mut SelectionState,
    tab: &mut DocTab,
    tool: Tool,
    c: Pt,
    mods: Modifiers,
    _ctx: &egui::Context,
) -> bool {
    let zoom = tab.zoom;
    if !tab.doc.xf_active() {
        // Grabbing a handle of the selection box starts a transform. Small boxes get small handles, and a click
        // inside such a box always moves the selection.
        let Some(b) = selection_handles(tab, tool) else { return false };
        let r = hit_radius(&b, zoom);
        let inside = c.0 > b[0].0 && c.0 < b[7].0 && c.1 > b[0].1 && c.1 < b[7].1;
        let near = b.iter().any(|h| dist(*h, c) * zoom as f64 <= r);
        if !near || (inside && r < MAX_HIT_RADIUS) {
            return false;
        }
        if !begin(&sel.xf, tab) {
            return false;
        }
    }
    let Some((handles, pivot)) = tab.doc.xf_geometry() else { return true };
    let r = hit_radius(&handles, zoom);
    let near = |q: Pt| dist(q, c) * zoom as f64 <= r;
    let mut handle = (0, 0);
    let kind = if near(pivot) {
        XfDrag::Pivot
    } else if let Some(i) = handles.iter().position(|h| near(*h)) {
        handle = HANDLE_ORDER[i];
        let edge = (handle.0 == 0) != (handle.1 == 0);
        if edge && mods.command { XfDrag::Skew } else { XfDrag::Scale }
    } else if contains(&handles, c) {
        XfDrag::Move
    } else {
        XfDrag::Rotate
    };
    tab.doc.xf_drag_begin();
    sel.xf.drag = Some(HandleDrag { kind, handle, start: c, doc: tab.doc.raw() as usize, last: None });
    true
}

/// One frame of a handle drag (`transformMouseDragged`); the drag ends with the button.
pub fn drag(sel: &mut SelectionState, tab: &mut DocTab, ctx: &egui::Context, c: Option<Pt>) {
    let Some(mut d) = sel.xf.drag else { return };
    let (down, shift) = ctx.input(|i| (i.pointer.any_down(), i.modifiers.shift));
    if !down {
        sel.xf.drag = None;
        return;
    }
    let Some(mut p) = c else { return };
    if d.kind == XfDrag::Move {
        // Whole-pixel moves keep the nearest-neighbor result crisp (scaling rounds in the core).
        p = (d.start.0 + (p.0 - d.start.0).round(), d.start.1 + (p.1 - d.start.1).round());
    }
    if d.last != Some((p, shift)) {
        tab.doc.xf_drag(d.kind, d.handle, shift, d.start, p);
        d.last = Some((p, shift));
        sel.xf.drag = Some(d);
        tab.doc.touch();
        tab.changed();
    }
}

// ---- Overlay ----

/// Box outline, handles and pivot (`drawTransformOverlay`).
pub fn paint_overlay(_state: &TransformState, tab: &DocTab, tool: Tool, painter: &egui::Painter, srect: Rect) {
    let zoom = tab.zoom as f64;
    let (handles, pivot): ([Pt; 8], Option<Pt>) = if let Some((h, p)) = tab.doc.xf_geometry() {
        (h, Some(p))
    } else if let Some(h) = selection_handles(tab, tool).filter(|_| tab.doc.sel_active()) {
        (h, None)
    } else {
        return;
    };
    let at = |p: Pt| -> Pos2 { srect.min + egui::vec2((p.0 * zoom) as f32, (p.1 * zoom) as f32) };
    let pts = handles.map(at);
    let quad = [pts[0], pts[2], pts[7], pts[5], pts[0]];
    painter.add(egui::Shape::line(quad.to_vec(), Stroke::new(1.0, Color32::WHITE)));
    painter.extend(egui::Shape::dashed_line(&quad, Stroke::new(1.0, Color32::BLACK), 4.0, 4.0));
    for p in pts {
        let r = Rect::from_center_size(p, egui::Vec2::splat(8.0));
        painter.rect_filled(r, 0.0, Color32::WHITE);
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Middle);
    }
    if let Some(pivot) = pivot {
        let orange = if painter.ctx().theme() == egui::Theme::Dark {
            Color32::from_rgb(255, 159, 10)
        } else {
            Color32::from_rgb(255, 149, 0)
        };
        painter.circle(at(pivot), 4.0, orange, Stroke::new(1.0, Color32::BLACK));
    }
}

// ---- The strip ----

const FIELD_WIDTH: f32 = 78.0;
const ANCHOR_SIZE: f32 = 33.0;
const GAP: f32 = space::XS;

fn text_width(ctx: &egui::Context, text: &str, size: f32) -> f32 {
    let painter = ctx.layer_painter(egui::LayerId::background());
    painter.layout_no_wrap(text.to_owned(), widgets::strong(ctx, size), Color32::WHITE).size().x
}

fn title_width(ctx: &egui::Context) -> f32 {
    12.0 + (space::XS + 2.0) + text_width(ctx, "Transform", theme::font::BODY_LARGE)
}

fn chip_width(ctx: &egui::Context) -> f32 {
    text_width(ctx, "RotSprite", theme::font::CAPTION) + 2.0 * (space::SM + 2.0)
}

fn button_width(ctx: &egui::Context, text: &str) -> f32 {
    text_width(ctx, text, theme::font::CAPTION) + 2.0 * (space::SM + 2.0)
}

/// Width of the one-row strip, padding included.
fn one_row_width(ctx: &egui::Context) -> f32 {
    let divider = 1.0 + 2.0 * space::XS;
    let items = [
        title_width(ctx),
        divider,
        FIELD_WIDTH * 4.0,
        ANCHOR_SIZE,
        divider,
        FIELD_WIDTH * 3.0,
        chip_width(ctx),
        space::SM,
        button_width(ctx, "Cancel"),
        button_width(ctx, "Apply"),
    ];
    // Between items: the title, divider, four fields, the anchor, divider, three fields, chip, spacer, buttons.
    let count = 15.0;
    items.iter().sum::<f32>() + (count - 1.0) * GAP + 2.0 * space::MD
}

/// Height of the strip under the context bar: one bar when it fits, two so nothing is clipped otherwise
/// (`ViewThatFits`). `None` when no transform is running.
pub fn bar_height(app: &PixmithApp, width: f32, ctx: &egui::Context) -> Option<f32> {
    if !active(app) {
        return None;
    }
    Some(if width >= one_row_width(ctx) { metric::BAR_HEIGHT } else { 2.0 * metric::BAR_HEIGHT })
}

/// `DSBarButtonStyle`: a caption-sized button, field-filled or prominent.
fn bar_button(ui: &mut egui::Ui, text: &str, prominent: bool, help: &str) -> egui::Response {
    let c = Colors::of(ui.ctx());
    let fg = if prominent { Color32::WHITE } else { c.text };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), widgets::strong(ui.ctx(), theme::font::CAPTION), fg);
    let size = egui::vec2(galley.size().x + 2.0 * (space::SM + 2.0), metric::CONTROL);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let (hover, down) = (response.hovered(), response.is_pointer_button_down_on());
    let fill = if prominent {
        c.accent.gamma_multiply(if down {
            0.75
        } else if hover {
            0.92
        } else {
            1.0
        })
    } else if down {
        c.pressed
    } else if hover {
        c.hover
    } else {
        c.field
    };
    ui.painter().rect_filled(rect, radius::CONTROL, fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    response.on_hover_text(help)
}

fn title(ui: &mut egui::Ui) {
    let c = Colors::of(ui.ctx());
    let galley =
        ui.painter().layout_no_wrap("Transform".to_owned(), widgets::strong(ui.ctx(), theme::font::BODY_LARGE), c.text);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(title_width(ui.ctx()), metric::CONTROL), egui::Sense::hover());
    ui.painter().text(
        egui::pos2(rect.left() + 6.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        egui_phosphor::regular::ARROWS_OUT_SIMPLE,
        egui::FontId::proportional(12.0),
        c.accent,
    );
    let x = rect.left() + 12.0 + space::XS + 2.0;
    ui.painter().galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, c.text);
}

fn field(ui: &mut egui::Ui, tab: &mut DocTab, label: &str, index: usize, help: &str, step: f64) {
    let mut value = tab.doc.xf_fields()[index];
    let opts =
        NumberOpts { range: (-9999.0, 9999.0), step, digits: 2, width: Some(FIELD_WIDTH), help, ..Default::default() };
    if widgets::ds_number_field(ui, ("xf-field", index), label, &mut value, &opts).changed {
        tab.doc.xf_set_field(index as u32, value);
        tab.doc.touch();
        tab.changed();
    }
}

fn box_fields(ui: &mut egui::Ui, tab: &mut DocTab, xf: &mut TransformState) {
    field(ui, tab, "X", 0, "Left edge of the box", 0.25);
    field(ui, tab, "Y", 1, "Top edge of the box", 0.25);
    field(ui, tab, "W", 2, "Width. Drag the label to scrub.", 0.25);
    field(ui, tab, "H", 3, "Height. Drag the label to scrub.", 0.25);
    anchor_picker(ui, tab, xf);
}

fn angle_fields(ui: &mut egui::Ui, tab: &mut DocTab, xf: &mut TransformState) {
    field(ui, tab, "Ang", 4, "Rotation angle in degrees", 0.5);
    field(ui, tab, "Sx", 5, "Horizontal skew", 0.5);
    field(ui, tab, "Sy", 6, "Vertical skew", 0.5);
    let help = "Pixel-art rotation: 8x edge-aware upscale, rotate, then downsample";
    if widgets::bar_text_chip(ui, "RotSprite", xf.rotsprite, help).clicked() {
        xf.rotsprite = !xf.rotsprite;
        tab.doc.xf_set_rotsprite(xf.rotsprite);
        tab.doc.touch();
        tab.changed();
    }
}

fn buttons(ui: &mut egui::Ui, tab: &mut DocTab) {
    // Right to left: Apply sits at the edge, Cancel before it.
    if bar_button(ui, "Apply", true, "Commit the transform (Return)").clicked() {
        commit(tab);
    }
    if bar_button(ui, "Cancel", false, "Discard the transform (Esc)").clicked() {
        cancel(tab);
    }
}

/// 3x3 grid of the box point that typed W and H keep in place.
fn anchor_picker(ui: &mut egui::Ui, tab: &mut DocTab, xf: &mut TransformState) {
    let c = Colors::of(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(ANCHOR_SIZE), egui::Sense::hover());
    ui.painter().rect_filled(rect, radius::CHIP, c.field);
    for (i, name) in ANCHOR_NAMES.iter().enumerate() {
        let (row, col) = ((i / 3) as f32, (i % 3) as f32);
        let cell = Rect::from_min_size(rect.min + egui::vec2(4.0 + col * 9.0, 4.0 + row * 9.0), egui::Vec2::splat(7.0));
        let response = ui
            .interact(cell, ui.id().with(("xf-anchor", i)), egui::Sense::click())
            .on_hover_text(format!("Anchor: {}. The point of the box that typed W and H keep in place.", name));
        let on = xf.anchor == i;
        let (r, color) = if on { (2.5, c.accent) } else { (1.5, c.text_tertiary) };
        ui.painter().circle_filled(cell.center(), r, color);
        if response.clicked() {
            xf.anchor = i;
            let a = xf.anchor_direction();
            tab.doc.xf_set_anchor(a.0, a.1);
        }
    }
}

/// The strip itself, in the panel the app lays out under the context bar.
pub fn show_bar(app: &mut PixmithApp, ui: &mut egui::Ui) {
    let c = Colors::of(ui.ctx());
    let rect = ui.max_rect();
    let two_rows = rect.height() > metric::BAR_HEIGHT + 1.0;
    let PixmithApp { tabs, active, selection: sel, .. } = app;
    let (tab, xf) = (&mut tabs[*active], &mut sel.xf);
    let rows = if two_rows { 2 } else { 1 };
    for row in 0..rows {
        let r = Rect::from_min_size(
            rect.min + egui::vec2(0.0, row as f32 * metric::BAR_HEIGHT),
            egui::vec2(rect.width(), metric::BAR_HEIGHT),
        );
        // The hairline under each bar (`DSToolbar` bottom edge).
        ui.painter().hline(
            egui::Rangef::new(r.left() - space::MD, r.right() + space::MD),
            r.bottom() - 0.5,
            Stroke::new(1.0, c.separator_soft),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("xf-row", row))
                .max_rect(r)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = GAP;
        let first = row == 0;
        if first {
            title(&mut child);
            widgets::group_divider(&mut child);
            box_fields(&mut child, tab, xf);
            if !two_rows {
                widgets::group_divider(&mut child);
                angle_fields(&mut child, tab, xf);
            }
            child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                buttons(ui, tab);
            });
        } else {
            angle_fields(&mut child, tab, xf);
        }
    }
}
