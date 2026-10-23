//! The canvas: the sprite on a checkerboard in the workspace, zoom and pan, and tool input (`CanvasView.swift`, `CanvasScroller.swift`, `CanvasOverlays.swift`).
//!
//! The sprite is composited by the core (`Doc::render`) and uploaded as a NEAREST-filtered texture only when
//! the tab's revision changes. Tools talk to the core exactly like `DrawingTools.swift`: `begin_edit` opens an
//! undo step, pencil and eraser run a `stroke_*` gesture, shapes redraw from the checkpoint while dragging.

use crate::app::{DocTab, PixmithApp, Tool};
use crate::doc::Doc;
use crate::theme::{Colors, space};
use crate::ui::widgets;
use eframe::egui::{self, Color32, Key, PointerButton, Pos2, Rect, Sense, Vec2};
use std::collections::HashMap;

/// Option flags understood by the drawing calls (`pixmith_shapes.h`).
pub mod flag {
    pub const MIRROR_X: u32 = 1;
    pub const MIRROR_Y: u32 = 2;
    pub const PIXEL_PERFECT: u32 = 4;
    pub const FILLED: u32 = 8;
    pub const CONSTRAIN: u32 = 16;
}

/// Shape kinds of `pixmith_shape`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Line = 0,
    Rect = 1,
    Ellipse = 2,
}

/// Breathing room around the sprite (`CanvasLayout.pad`).
pub const PAD: f32 = 64.0;
pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 64.0;
/// The pixel grid shows from this zoom on (`ViewSettings.gridMinZoom`).
const GRID_MIN_ZOOM: f32 = 6.0;

/// Ink modes of the core (`InkMode`), in order.
pub const INKS: [&str; 5] = ["Simple", "Alpha Compositing", "Lock Alpha", "Behind", "Shading"];

/// Tool options shared by the context bar and the canvas (`DrawingOptions`).
#[derive(Clone, Debug)]
pub struct DrawOptions {
    pub circle_brush: bool,
    /// Index into [`INKS`].
    pub ink: usize,
    pub pixel_perfect: bool,
    pub pressure_size: bool,
    pub pressure_alpha: bool,
    pub mirror_x: bool,
    pub mirror_y: bool,
    pub filled: bool,
    pub contiguous: bool,
    pub gradient_radial: bool,
    pub gradient_dither: usize,
    pub fill_pattern: usize,
    pub show_grid: bool,
}

impl Default for DrawOptions {
    fn default() -> DrawOptions {
        DrawOptions {
            circle_brush: false,
            ink: 0,
            pixel_perfect: false,
            pressure_size: false,
            pressure_alpha: false,
            mirror_x: false,
            mirror_y: false,
            filled: false,
            contiguous: true,
            gradient_radial: false,
            gradient_dither: 0,
            fill_pattern: 0,
            show_grid: true,
        }
    }
}

impl DrawOptions {
    /// Symmetry flags of the drawing calls (`SpriteModel.symFlags`).
    pub fn sym_flags(&self) -> u32 {
        (if self.mirror_x { flag::MIRROR_X } else { 0 }) | (if self.mirror_y { flag::MIRROR_Y } else { 0 })
    }
}

type RenderKey = (usize, u64, u32, u32, u32);

/// A pointer gesture in progress.
#[derive(Clone, Copy, Debug)]
enum Gesture {
    Pan(PointerButton),
    Tool(ToolGesture),
}

#[derive(Clone, Copy, Debug)]
struct ToolGesture {
    tool: Tool,
    button: PointerButton,
    start: (i32, i32),
    last: (i32, i32),
    color: u32,
    constrain: bool,
    /// The document the gesture started in; a tab switch ends it.
    doc: usize,
}

/// State of this region that outlives a frame.
#[derive(Default)]
pub struct CanvasState {
    pub opts: DrawOptions,
    /// Size of the canvas area in points, for zoom-to-fit (`ViewSettings.viewportSize`).
    pub viewport: Vec2,
    /// Sprite pixel under the cursor, if it is over the canvas.
    pub cursor: Option<(i32, i32)>,
    texture: Option<egui::TextureHandle>,
    /// The composited frame as straight-alpha RGBA, for the eyedropper and the texture.
    pixels: Vec<u8>,
    rendered: Option<RenderKey>,
    uploaded: Option<RenderKey>,
    /// Documents that already got their initial zoom-to-fit, with the size it was computed for.
    fitted: HashMap<usize, (u32, u32)>,
    gesture: Option<Gesture>,
}

impl CanvasState {
    /// Keeps `tab`'s current zoom and pan instead of fitting it on first display.
    pub fn keep_view(&mut self, tab: &DocTab) {
        self.fitted.insert(tab.doc.raw() as usize, (tab.doc.width(), tab.doc.height()));
    }

    /// Renders the tab's current frame and uploads it when something changed.
    fn ensure_render(&mut self, ctx: &egui::Context, tab: &DocTab) {
        let (w, h) = (tab.doc.width(), tab.doc.height());
        let key = (tab.doc.raw() as usize, tab.revision, tab.frame, w, h);
        if self.rendered != Some(key) && tab.doc.render(tab.frame, &mut self.pixels) {
            self.rendered = Some(key);
        }
        if self.rendered == Some(key) && self.uploaded != Some(key) {
            let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &self.pixels);
            match &mut self.texture {
                Some(t) => t.set(image, egui::TextureOptions::NEAREST),
                None => self.texture = Some(ctx.load_texture("sprite", image, egui::TextureOptions::NEAREST)),
            }
            self.uploaded = Some(key);
        }
    }
}

/// The zoom that fits a `w` x `h` sprite in `viewport` with the canvas padding (`ViewSettings.zoomToFit`).
pub fn fit_zoom(viewport: Vec2, w: u32, h: u32) -> f32 {
    let avail = viewport - Vec2::splat(2.0 * PAD);
    if avail.x <= 0.0 || avail.y <= 0.0 || w == 0 || h == 0 {
        return 1.0;
    }
    let mut z = (avail.x / w as f32).min(avail.y / h as f32);
    if z >= 1.0 {
        z = z.floor();
    }
    z.clamp(MIN_ZOOM, MAX_ZOOM)
}

/// Where the sprite sits in the canvas `area`: centered, shifted by the tab's pan, on whole device pixels.
pub fn sprite_rect(area: Rect, tab: &DocTab, ppp: f32) -> Rect {
    let size = Vec2::new(tab.doc.width() as f32, tab.doc.height() as f32) * tab.zoom;
    let min = area.center() + tab.pan - size / 2.0;
    let snap = |v: f32| (v * ppp).round() / ppp;
    Rect::from_min_size(Pos2::new(snap(min.x), snap(min.y)), size)
}

/// Keeps the pan inside what the Swift scroll view can reach: a sprite smaller than the view stays centered,
/// a larger one scrolls until its edge meets the 64 pt padding.
fn clamp_pan(area: Vec2, sprite: Vec2, pan: Vec2) -> Vec2 {
    let reach = |view: f32, size: f32| ((size + 2.0 * PAD - view) / 2.0).max(0.0);
    Vec2::new(
        pan.x.clamp(-reach(area.x, sprite.x), reach(area.x, sprite.x)),
        pan.y.clamp(-reach(area.y, sprite.y), reach(area.y, sprite.y)),
    )
}

fn pixel_at(rect: Rect, zoom: f32, pos: Pos2) -> (i32, i32) {
    let p = (pos - rect.min) / zoom;
    (p.x.floor() as i32, p.y.floor() as i32)
}

fn is_selection(tool: Tool) -> bool {
    matches!(tool, Tool::Marquee | Tool::Lasso | Tool::Polygon | Tool::Wand | Tool::Move)
}

pub fn show(app: &mut PixmithApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    handle_keys(app, &ctx);

    let c = Colors::of(&ctx);
    let rect = ui.max_rect();
    let response = ui.interact(rect, ui.id().with("canvas"), Sense::click_and_drag());
    let ppp = ui.pixels_per_point();
    let PixmithApp { tabs, active, tool, primary, secondary, brush_size, dark, canvas: state, .. } = app;
    let tab = &mut tabs[*active];
    state.viewport = rect.size();

    // The first time a document is shown it opens zoomed to fit.
    let doc_id = tab.doc.raw() as usize;
    let dims = (tab.doc.width(), tab.doc.height());
    if state.fitted.get(&doc_id) != Some(&dims) {
        state.fitted.insert(doc_id, dims);
        tab.zoom = fit_zoom(rect.size(), dims.0, dims.1);
        tab.pan = Vec2::ZERO;
    }
    state.ensure_render(&ctx, tab);

    navigate(state, tab, &response, rect, &ctx);
    let srect = sprite_rect(rect, tab, ppp);
    interact(state, tab, *tool, primary, *secondary, *brush_size, &response, srect, &ctx);
    state.ensure_render(&ctx, tab);

    paint(state, tab, &ui.painter_at(rect), &c, *dark, rect, srect, ppp);
    if response.hovered() {
        ctx.set_cursor_icon(if ctx.input(|i| i.key_down(Key::Space)) {
            egui::CursorIcon::Grab
        } else {
            egui::CursorIcon::Crosshair
        });
    }
}

/// Zoom (pinch, Ctrl/Cmd-scroll; the point under the cursor stays put) and pan (scroll, middle or Space-drag).
fn navigate(state: &mut CanvasState, tab: &mut DocTab, response: &egui::Response, rect: Rect, ctx: &egui::Context) {
    let (zoom_delta, scroll, hover, delta, space) = ctx.input(|i| {
        (i.zoom_delta(), i.smooth_scroll_delta(), i.pointer.hover_pos(), i.pointer.delta(), i.key_down(Key::Space))
    });
    let sprite = Vec2::new(tab.doc.width() as f32, tab.doc.height() as f32);
    let over = response.hovered();
    if over && zoom_delta != 1.0 {
        let old = tab.zoom;
        let new = (old * zoom_delta).clamp(MIN_ZOOM, MAX_ZOOM);
        if new != old {
            let anchor = hover.filter(|p| rect.contains(*p)).unwrap_or(rect.center());
            let rel = anchor - rect.center();
            tab.pan = rel - (rel - tab.pan) * (new / old);
            tab.zoom = new;
        }
    }
    if over && scroll != Vec2::ZERO {
        tab.pan += scroll;
    }
    if let Some(Gesture::Pan(button)) = state.gesture {
        if ctx.input(|i| i.pointer.button_down(button)) {
            tab.pan += delta;
        } else {
            state.gesture = None;
        }
    } else if state.gesture.is_none() && over {
        let (middle, primary) = ctx.input(|i| {
            (i.pointer.button_pressed(PointerButton::Middle), i.pointer.button_pressed(PointerButton::Primary))
        });
        if middle {
            state.gesture = Some(Gesture::Pan(PointerButton::Middle));
        } else if primary && space {
            state.gesture = Some(Gesture::Pan(PointerButton::Primary));
        }
    }
    tab.pan = clamp_pan(rect.size(), sprite * tab.zoom, tab.pan);
}

/// Mouse input for the tools (`CanvasNSView.mouseDown/Dragged/Up` and `DrawingTools.swift`).
#[allow(clippy::too_many_arguments)]
fn interact(
    state: &mut CanvasState,
    tab: &mut DocTab,
    tool: Tool,
    primary: &mut u32,
    secondary: u32,
    brush: u32,
    response: &egui::Response,
    srect: Rect,
    ctx: &egui::Context,
) {
    let (pos, mods, pressed_primary, pressed_secondary) = ctx.input(|i| {
        (
            i.pointer.latest_pos(),
            i.modifiers,
            i.pointer.button_pressed(PointerButton::Primary),
            i.pointer.button_pressed(PointerButton::Secondary),
        )
    });
    let doc_id = tab.doc.raw() as usize;
    if let Some(Gesture::Tool(g)) = state.gesture
        && g.doc != doc_id
    {
        state.gesture = None;
    }
    let p = pos.map(|pos| pixel_at(srect, tab.zoom, pos));
    state.cursor = if response.hovered() || matches!(state.gesture, Some(Gesture::Tool(_))) { p } else { None };

    // Press.
    if state.gesture.is_none()
        && response.hovered()
        && (pressed_primary || pressed_secondary)
        && let Some(p) = p
    {
        let button = if pressed_secondary { PointerButton::Secondary } else { PointerButton::Primary };
        state.gesture = begin(state, tab, tool, primary, secondary, brush, p, button, mods, doc_id);
    }

    // Drag and release.
    if let Some(Gesture::Tool(mut g)) = state.gesture {
        let down = ctx.input(|i| i.pointer.button_down(g.button));
        if !down {
            finish(tab, &g);
            state.gesture = None;
        } else if let Some(p) = p {
            let constrain = mods.shift;
            if p != g.last || constrain != g.constrain {
                drag(state, tab, &g, primary, brush, p, constrain);
                g.last = p;
                g.constrain = constrain;
                state.gesture = Some(Gesture::Tool(g));
            }
        }
    }
}

fn mutated(tab: &mut DocTab) {
    tab.doc.touch();
    tab.changed();
}

/// Sends the ink mode to the core; shading steps between the two colors, backwards for the right button.
fn apply_ink(state: &CanvasState, tab: &mut DocTab, primary: u32, secondary: u32, reverse: bool) {
    let mode = state.opts.ink;
    let ramp = if mode == 4 { vec![primary, secondary] } else { Vec::new() };
    tab.doc.set_ink(mode as u32, reverse, &ramp);
}

#[allow(clippy::too_many_arguments)]
fn begin(
    state: &mut CanvasState,
    tab: &mut DocTab,
    tool: Tool,
    primary: &mut u32,
    secondary: u32,
    brush: u32,
    p: (i32, i32),
    button: PointerButton,
    mods: egui::Modifiers,
    doc: usize,
) -> Option<Gesture> {
    let right = button == PointerButton::Secondary;
    let color = if right { secondary } else { *primary };
    let tool = if mods.alt && !is_selection(tool) { Tool::Picker } else { tool };
    let (layer, frame) = (tab.layer, tab.frame);
    let sym = state.opts.sym_flags();
    let mut gesture = ToolGesture { tool, button, start: p, last: p, color, constrain: mods.shift, doc };
    match tool {
        Tool::Picker => pick(state, tab, primary, p),
        Tool::Fill => {
            begin_edit(tab, "Fill");
            apply_ink(state, tab, *primary, secondary, right);
            let other = if right { *primary } else { secondary };
            tab.doc.fill_at(layer, frame, p.0, p.1, color, other, state.opts.contiguous, sym);
            mutated(tab);
            return None;
        }
        Tool::Pencil | Tool::Eraser => {
            begin_edit(tab, if tool == Tool::Eraser { "Erase" } else { "Draw" });
            apply_ink(state, tab, *primary, secondary, right);
            let flags = sym | if state.opts.pixel_perfect { flag::PIXEL_PERFECT } else { 0 };
            let c = if tool == Tool::Eraser { 0 } else { color };
            gesture.color = c;
            tab.doc.stroke_begin(layer, frame, p.0, p.1, c, brush, flags);
            mutated(tab);
        }
        Tool::Line | Tool::Rectangle | Tool::Ellipse => {
            begin_edit(tab, tool.title());
            apply_ink(state, tab, *primary, secondary, right);
            shape(state, tab, &gesture, p, brush, false);
            mutated(tab);
        }
        _ => return None,
    }
    Some(Gesture::Tool(gesture))
}

fn begin_edit(tab: &mut DocTab, label: &str) {
    tab.playing = false;
    tab.doc.begin_edit(label);
}

fn shape(state: &CanvasState, tab: &mut DocTab, g: &ToolGesture, to: (i32, i32), brush: u32, restore: bool) {
    let kind = match g.tool {
        Tool::Line => ShapeKind::Line,
        Tool::Rectangle => ShapeKind::Rect,
        _ => ShapeKind::Ellipse,
    };
    let flags = state.opts.sym_flags()
        | if state.opts.filled { flag::FILLED } else { 0 }
        | if g.constrain { flag::CONSTRAIN } else { 0 };
    let (layer, frame) = (tab.layer, tab.frame);
    tab.doc.draw_shape(kind, layer, frame, g.start, to, g.color, brush, flags, restore);
}

fn drag(
    state: &mut CanvasState,
    tab: &mut DocTab,
    g: &ToolGesture,
    primary: &mut u32,
    brush: u32,
    p: (i32, i32),
    constrain: bool,
) {
    match g.tool {
        Tool::Pencil | Tool::Eraser => {
            tab.doc.stroke_to(p.0, p.1);
            mutated(tab);
        }
        Tool::Line | Tool::Rectangle | Tool::Ellipse => {
            let g = ToolGesture { constrain, ..*g };
            shape(state, tab, &g, p, brush, true);
            mutated(tab);
        }
        Tool::Picker => pick(state, tab, primary, p),
        _ => {}
    }
}

fn finish(tab: &mut DocTab, g: &ToolGesture) {
    if matches!(g.tool, Tool::Pencil | Tool::Eraser | Tool::Line | Tool::Rectangle | Tool::Ellipse) {
        tab.doc.stroke_end();
        mutated(tab);
    }
}

/// Eyedropper: takes the composited pixel under `p` as the primary color (`SpriteModel.pick`).
fn pick(state: &mut CanvasState, tab: &DocTab, primary: &mut u32, p: (i32, i32)) {
    let (w, h) = (tab.doc.width(), tab.doc.height());
    if p.0 < 0 || p.1 < 0 || p.0 as u32 >= w || p.1 as u32 >= h {
        return;
    }
    state.rendered = None; // always read what is on screen right now
    if tab.doc.render(tab.frame, &mut state.pixels) {
        state.rendered = Some((tab.doc.raw() as usize, tab.revision, tab.frame, w, h));
        if let Some(c) = Doc::pixel_of(&state.pixels, w, p.0, p.1) {
            *primary = c;
        }
    }
}

/// Backdrop, sprite, grid, mirror guides and the two status chips (`CanvasOverlays`).
#[allow(clippy::too_many_arguments)]
fn paint(
    state: &CanvasState,
    tab: &DocTab,
    painter: &egui::Painter,
    c: &Colors,
    dark: bool,
    area: Rect,
    srect: Rect,
    ppp: f32,
) {
    let hairline = 1.0 / ppp;
    // Soft drop shadow so the sprite reads as an object on the workspace.
    let shadow = egui::Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 128 } else { 56 }),
    };
    painter.add(shadow.as_shape(srect, 0.0));

    // Checkerboard: 8 pt squares anchored at the sprite's corner.
    let (light, darker) = if dark {
        (Color32::from_rgb(0x3A, 0x3A, 0x3D), Color32::from_rgb(0x2F, 0x2F, 0x32))
    } else {
        (Color32::from_rgb(0xF3, 0xF3, 0xF5), Color32::from_rgb(0xDA, 0xDA, 0xDF))
    };
    let vis = srect.intersect(area);
    if vis.is_positive() {
        let mut mesh = egui::Mesh::default();
        mesh.add_colored_rect(vis, light);
        let cell = 8.0;
        let (i0, i1) =
            (((vis.left() - srect.left()) / cell).floor() as i32, ((vis.right() - srect.left()) / cell).ceil() as i32);
        let (j0, j1) =
            (((vis.top() - srect.top()) / cell).floor() as i32, ((vis.bottom() - srect.top()) / cell).ceil() as i32);
        for j in j0..j1 {
            for i in i0..i1 {
                if (i + j) % 2 == 0 {
                    let r = Rect::from_min_size(srect.min + Vec2::new(i as f32, j as f32) * cell, Vec2::splat(cell));
                    mesh.add_colored_rect(r.intersect(vis), darker);
                }
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }

    if let Some(t) = &state.texture {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        painter.image(t.id(), srect, uv, Color32::WHITE);
    }

    // One-pixel outline just outside the sprite.
    let outline = Color32::from_black_alpha(if dark { 153 } else { 46 });
    painter.rect_stroke(srect, 0.0, egui::Stroke::new(hairline, outline), egui::StrokeKind::Outside);

    // Pixel grid.
    if state.opts.show_grid && tab.zoom >= GRID_MIN_ZOOM && vis.is_positive() {
        let grid = Color32::from_black_alpha(31);
        let snap = |v: f32| (v * ppp).round() / ppp;
        let mut mesh = egui::Mesh::default();
        let (w, h) = (tab.doc.width() as i32, tab.doc.height() as i32);
        for i in 0..=w {
            let x = snap(srect.left() + i as f32 * tab.zoom);
            if x >= vis.left() - 1.0 && x <= vis.right() + 1.0 {
                mesh.add_colored_rect(
                    Rect::from_min_max(Pos2::new(x, vis.top()), Pos2::new(x + hairline, vis.bottom())),
                    grid,
                );
            }
        }
        for j in 0..=h {
            let y = snap(srect.top() + j as f32 * tab.zoom);
            if y >= vis.top() - 1.0 && y <= vis.bottom() + 1.0 {
                mesh.add_colored_rect(
                    Rect::from_min_max(Pos2::new(vis.left(), y), Pos2::new(vis.right(), y + hairline)),
                    grid,
                );
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }

    // Mirror guides: pink dashes through the canvas center.
    if state.opts.mirror_x || state.opts.mirror_y {
        let pink = if dark { Color32::from_rgb(0xFF, 0x37, 0x5F) } else { Color32::from_rgb(0xFF, 0x2D, 0x55) }
            .gamma_multiply(0.8);
        let stroke = egui::Stroke::new(1.0, pink);
        let center = srect.center();
        if state.opts.mirror_x {
            painter.extend(egui::Shape::dashed_line(
                &[Pos2::new(center.x, srect.top()), Pos2::new(center.x, srect.bottom())],
                stroke,
                4.0,
                3.0,
            ));
        }
        if state.opts.mirror_y {
            painter.extend(egui::Shape::dashed_line(
                &[Pos2::new(srect.left(), center.y), Pos2::new(srect.right(), center.y)],
                stroke,
                4.0,
                3.0,
            ));
        }
    }

    // Status chips, bottom left (`CanvasArea`).
    let top = area.bottom() - (space::SM + 2.0) - 22.0;
    let mut x = area.left() + space::MD;
    x += widgets::overlay_chip(
        painter,
        c,
        Pos2::new(x, top),
        None,
        &format!("{} \u{d7} {}", tab.doc.width(), tab.doc.height()),
    );
    x += space::XS + 2.0;
    let (w, h) = (tab.doc.width() as i32, tab.doc.height() as i32);
    let position = match state.cursor {
        Some((cx, cy)) if (0..w).contains(&cx) && (0..h).contains(&cy) => format!("{cx}, {cy}"),
        _ => "\u{2013}".to_string(),
    };
    widgets::overlay_chip(painter, c, Pos2::new(x, top), Some(egui_phosphor::regular::CURSOR), &position);
}

/// Keyboard: tool keys, `[` / `]` brush size, undo and redo.
fn handle_keys(app: &mut PixmithApp, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    use egui::{Key, Modifiers};
    let (redo, undo, smaller, larger) = ctx.input_mut(|i| {
        let redo =
            i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y);
        let undo = i.consume_key(Modifiers::COMMAND, Key::Z);
        let smaller = i.key_pressed(Key::OpenBracket) && !i.modifiers.command;
        let larger = i.key_pressed(Key::CloseBracket) && !i.modifiers.command;
        (redo, undo, smaller, larger)
    });
    if undo || redo {
        app.canvas.gesture = None;
        let tab = app.tab_mut();
        let moved = if redo { tab.doc.redo() } else { tab.doc.undo() };
        if moved {
            tab.changed();
        }
    }
    if smaller {
        app.brush_size = app.brush_size.saturating_sub(1).max(1);
    }
    if larger {
        app.brush_size = (app.brush_size + 1).min(64);
    }
    let (shift, command, alt) = ctx.input(|i| (i.modifiers.shift, i.modifiers.command, i.modifiers.alt));
    if command || alt {
        return;
    }
    for group in Tool::GROUPS {
        for &tool in group {
            let key = tool.key();
            let (wants_shift, name) = match key.strip_prefix('\u{21e7}') {
                Some(rest) => (true, rest),
                None => (false, key),
            };
            if wants_shift != shift {
                continue;
            }
            if Key::from_name(name).is_some_and(|k| ctx.input(|i| i.key_pressed(k))) {
                app.tool = tool;
            }
        }
    }
}
