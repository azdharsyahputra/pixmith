//! Spray, gradient, curve, polygon, contour and text, plus ink modes and pen pressure
//! (`DrawingTools.swift`, `InkTools.swift`, `ShapeTools.swift`).
//!
//! The canvas owns the pointer gesture and calls in here at press, drag, hold and release. Every tool works the
//! way the Swift app does: it opens an undo step (`Doc::begin_edit`), draws a live preview that is redrawn from
//! that checkpoint, and the step stays one entry in the history.

use super::text::TextRequest;
use crate::app::{DocTab, Tool};
use crate::ui::canvas::{CanvasState, DrawOptions, flag};
use eframe::egui;

/// The two colors and brush size of the gesture in progress.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pen {
    pub primary: u32,
    pub secondary: u32,
    pub brush: u32,
}

/// An in-progress curve, polygon or contour (`ShapeSession`). The preview is drawn into the document and redrawn
/// from the checkpoint taken when the gesture began, so the whole shape stays a single undo step.
#[derive(Clone, Debug)]
pub struct ShapeSession {
    pub tool: Tool,
    /// The document it belongs to (its pointer).
    pub doc: usize,
    /// Curve: start, end, first and second control point. Polygon and contour: the vertices.
    pub points: Vec<(i32, i32)>,
    pub color: u32,
    pub cursor: (i32, i32),
    /// Curve only: 0 dragging the line, 1 placing the first control point, 2 the second.
    pub stage: usize,
    pub dragging: bool,
    /// Undo depth right after the gesture's checkpoint; a different depth means the history moved on.
    pub depth: u32,
}

/// Drawing-tool state that outlives a pointer event.
#[derive(Default)]
pub struct DrawState {
    pub session: Option<ShapeSession>,
    pen: Pen,
    /// Gradient: the checkpoint and the first preview exist.
    gradient_started: bool,
    rng: u32,
    last_spray: f64,
    /// Time and position of the previous press, to recognise a double click.
    last_press: Option<(f64, (i32, i32))>,
    /// Pen pressure of the latest touch event, 0..1 (tablets that report it).
    pub pressure: Option<f32>,
}

impl DrawState {
    fn next_random(&mut self) -> u32 {
        if self.rng == 0 {
            self.rng = 0x9E37_79B9;
        }
        // xorshift32
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }
}

/// A press of the pointer.
#[derive(Clone, Copy, Debug)]
pub struct Press {
    pub p: (i32, i32),
    pub right: bool,
    pub shift: bool,
    /// `input.time` in seconds.
    pub now: f64,
}

fn mutated(tab: &mut DocTab) {
    tab.doc.touch();
    tab.changed();
}

fn begin_edit(tab: &mut DocTab, label: &str) {
    tab.playing = false;
    tab.doc.begin_edit(label);
}

/// The ink ramp shading steps through: the palette between the two colors, or just the two colors
/// (`SpriteModel.shadingRamp`).
pub fn shading_ramp(palette: &[u32], primary: u32, secondary: u32) -> Vec<u32> {
    let a = palette.iter().position(|c| *c == primary);
    let b = palette.iter().position(|c| *c == secondary);
    match (a, b) {
        (Some(a), Some(b)) if a != b => {
            if a < b {
                palette[a..=b].to_vec()
            } else {
                palette[b..=a].iter().rev().copied().collect()
            }
        }
        _ => vec![primary, secondary],
    }
}

/// Sends the ink mode to the core. Shading steps through the palette between the two colors, toward the
/// secondary color, or back toward the primary when drawing with the right button (`SpriteModel.applyInk`).
pub fn apply_ink(opts: &DrawOptions, tab: &mut DocTab, primary: u32, secondary: u32, reverse: bool) {
    let ramp = if opts.ink == 4 { shading_ramp(&tab.doc.palette(), primary, secondary) } else { Vec::new() };
    tab.doc.set_ink(opts.ink as u32, reverse, &ramp);
}

/// Snaps a gradient end point to the nearest multiple of 45 degrees from the start (`snapped`).
pub fn snapped(a: (i32, i32), b: (i32, i32)) -> (i32, i32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (ax, ay) = (dx.abs(), dy.abs());
    if ax > 2 * ay {
        return (b.0, a.1);
    }
    if ay > 2 * ax {
        return (a.0, b.1);
    }
    let m = ax.max(ay);
    (a.0 + m * if dx < 0 { -1 } else { 1 }, a.1 + m * if dy < 0 { -1 } else { 1 })
}

/// Color and brush size adjusted by pen pressure, or `None` for mouse input and when pressure is off
/// (`SpriteModel.pressureStyle`).
pub fn pressure_style(state: &CanvasState, color: u32, size: u32) -> Option<(u32, u32)> {
    let o = &state.opts;
    let pressure = state.tools.draw.pressure?;
    if !(o.pressure_size || o.pressure_alpha) {
        return None;
    }
    let pressure = (pressure as f64).clamp(0.02, 1.0);
    let sz = if o.pressure_size { ((size as f64 * pressure).round() as u32).max(1) } else { size };
    let mut c = color;
    if o.pressure_alpha && color & 0xFF != 0 {
        c = (color & !0xFF) | (((color & 0xFF) as f64 * pressure).round() as u32).max(1);
    }
    Some((c, sz))
}

/// Remembers the latest pen pressure from touch events; call once per frame.
pub fn track_pressure(state: &mut CanvasState, ctx: &egui::Context) {
    ctx.input(|i| {
        for e in &i.events {
            if let egui::Event::Touch { phase, force, .. } = e {
                state.tools.draw.pressure = match phase {
                    egui::TouchPhase::End | egui::TouchPhase::Cancel => None,
                    _ => force.map(|f| f.clamp(0.0, 1.0)).or(state.tools.draw.pressure),
                };
            }
        }
    });
}

// MARK: Press, drag, hold, release

/// Handles a press for the tools in this module. Returns true when the canvas should keep tracking the gesture
/// (drag and release).
pub fn down(state: &mut CanvasState, tab: &mut DocTab, tool: Tool, pen: Pen, press: Press) -> bool {
    let c = if press.right { pen.secondary } else { pen.primary };
    state.tools.draw.pen = pen;
    let double = state.tools.draw.last_press.is_some_and(|(t, p)| press.now - t < 0.4 && p == press.p);
    state.tools.draw.last_press = Some((press.now, press.p));
    match tool {
        Tool::Spray => {
            begin_edit(tab, "Spray");
            apply_ink(&state.opts, tab, pen.primary, pen.secondary, press.right);
            state.tools.draw.last_spray = press.now;
            spray(state, tab, press.p, c);
            mutated(tab);
            true
        }
        // The undo step starts with the first drag, so a plain click leaves nothing behind.
        Tool::Gradient => {
            state.tools.draw.gradient_started = false;
            true
        }
        Tool::Text => {
            begin_edit(tab, "Text");
            apply_ink(&state.opts, tab, pen.primary, pen.secondary, press.right);
            let depth = tab.doc.history_counts().0;
            state.tools.text.begin(TextRequest {
                at: press.p,
                color: c,
                started: false,
                depth,
                doc: tab.doc.raw() as usize,
            });
            false
        }
        Tool::Contour => {
            begin_shape(state, tab, tool, vec![press.p], c, press, true);
            draw_shape(state, tab, false);
            true
        }
        Tool::Curve => {
            if valid_session(state, tab)
                && state.tools.draw.session.as_ref().is_some_and(|s| s.tool == Tool::Curve && s.stage > 0)
            {
                if let Some(s) = &mut state.tools.draw.session {
                    s.points[s.stage + 1] = press.p;
                    s.dragging = true;
                }
            } else {
                begin_shape(state, tab, tool, vec![press.p; 4], c, press, true);
            }
            draw_shape(state, tab, false);
            true
        }
        Tool::ShapePolygon => {
            if valid_session(state, tab)
                && state.tools.draw.session.as_ref().is_some_and(|s| s.tool == Tool::ShapePolygon)
            {
                if double {
                    finish_shape(state, tab);
                    return false;
                }
                if let Some(s) = &mut state.tools.draw.session {
                    if s.points.last() != Some(&press.p) {
                        s.points.push(press.p);
                    }
                    s.cursor = press.p;
                }
            } else {
                begin_shape(state, tab, tool, vec![press.p], c, press, false);
            }
            draw_shape(state, tab, false);
            true
        }
        _ => false,
    }
}

/// The pointer moved with the button down (or Shift changed).
#[allow(clippy::too_many_arguments)]
pub fn drag(
    state: &mut CanvasState,
    tab: &mut DocTab,
    tool: Tool,
    start: (i32, i32),
    right: bool,
    color: u32,
    p: (i32, i32),
    constrain: bool,
) {
    match tool {
        Tool::Spray => {
            spray(state, tab, p, color);
            mutated(tab);
        }
        Tool::Gradient => {
            gradient_drag(state, tab, start, p, right, constrain);
            mutated(tab);
        }
        Tool::Contour | Tool::Curve | Tool::ShapePolygon => {
            if !valid_session(state, tab) {
                return;
            }
            let Some(s) = &mut state.tools.draw.session else { return };
            match s.tool {
                Tool::Contour => {
                    if s.points.last() == Some(&p) {
                        return;
                    }
                    s.points.push(p);
                }
                Tool::Curve => {
                    if !s.dragging {
                        return;
                    }
                    let i = if s.stage == 0 { 1 } else { s.stage + 1 };
                    s.points[i] = p;
                }
                _ => {}
            }
            s.cursor = p;
            draw_shape(state, tab, false);
        }
        _ => {}
    }
}

/// The button is held and the pointer has not moved: the spray keeps puffing every 30 ms
/// (`DrawingOptions.Drag.timer`).
pub fn hold(state: &mut CanvasState, tab: &mut DocTab, tool: Tool, at: (i32, i32), color: u32, ctx: &egui::Context) {
    if tool != Tool::Spray {
        return;
    }
    let now = ctx.input(|i| i.time);
    if now - state.tools.draw.last_spray >= 0.03 {
        state.tools.draw.last_spray = now;
        spray(state, tab, at, color);
        mutated(tab);
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(30));
}

/// The button was released.
pub fn up(state: &mut CanvasState, tab: &mut DocTab, tool: Tool) {
    match tool {
        Tool::Spray | Tool::Gradient => {
            tab.doc.stroke_end();
            mutated(tab);
        }
        Tool::Contour | Tool::Curve | Tool::ShapePolygon => shape_up(state, tab),
        _ => {}
    }
}

/// The pointer moved with no button down: an open polygon or curve previews the next vertex or control point
/// (`shapeMoved`).
pub fn moved(state: &mut CanvasState, tab: &mut DocTab, p: (i32, i32)) {
    if !valid_session(state, tab) {
        return;
    }
    let Some(s) = &mut state.tools.draw.session else { return };
    if s.dragging || s.cursor == p {
        return;
    }
    s.cursor = p;
    let redraw = s.tool == Tool::ShapePolygon || (s.tool == Tool::Curve && s.stage > 0);
    if redraw {
        draw_shape(state, tab, false);
    }
}

fn spray(state: &mut CanvasState, tab: &mut DocTab, p: (i32, i32), color: u32) {
    let brush = state.tools.draw.pen.brush;
    let seed = state.tools.draw.next_random();
    let (layer, frame) = (tab.layer, tab.frame);
    tab.doc.spray_at(
        layer,
        frame,
        p,
        color,
        (brush * 3).max(3),
        state.opts.spray_density.round() as u32,
        seed,
        state.opts.sym_flags(),
    );
}

/// Updates the gradient while dragging. The checkpoint is taken on the first real drag.
fn gradient_drag(
    state: &mut CanvasState,
    tab: &mut DocTab,
    start: (i32, i32),
    p: (i32, i32),
    right: bool,
    constrain: bool,
) {
    let pen = state.tools.draw.pen;
    let started = state.tools.draw.gradient_started;
    if !started {
        begin_edit(tab, "Gradient");
        apply_ink(&state.opts, tab, pen.primary, pen.secondary, right);
    }
    let (c0, c1) = if right { (pen.secondary, pen.primary) } else { (pen.primary, pen.secondary) };
    let end = if constrain { snapped(start, p) } else { p };
    let dither = [0, 2, 4, 8][state.opts.gradient_dither.min(3)];
    let (layer, frame) = (tab.layer, tab.frame);
    tab.doc.gradient_draw(layer, frame, start, end, c0, c1, state.opts.gradient_radial, dither, started);
    state.tools.draw.gradient_started = true;
}

// MARK: Multi-point shapes

/// Whether the open session still belongs to this document and history position; drops it if not.
fn valid_session(state: &mut CanvasState, tab: &DocTab) -> bool {
    let ok = state
        .tools
        .draw
        .session
        .as_ref()
        .is_some_and(|s| s.doc == tab.doc.raw() as usize && tab.doc.history_counts().0 == s.depth);
    if !ok {
        state.tools.draw.session = None;
    }
    ok
}

/// True while a curve, polygon or contour is being built on this tab (`drawingSessionOpen`).
pub fn session_open(state: &mut CanvasState, tab: &DocTab) -> bool {
    valid_session(state, tab)
}

fn begin_shape(
    state: &mut CanvasState,
    tab: &mut DocTab,
    tool: Tool,
    points: Vec<(i32, i32)>,
    color: u32,
    press: Press,
    dragging: bool,
) {
    let label = match tool {
        Tool::Curve => "Curve",
        Tool::Contour => "Contour",
        _ => "Polygon",
    };
    begin_edit(tab, label);
    let pen = state.tools.draw.pen;
    apply_ink(&state.opts, tab, pen.primary, pen.secondary, press.right);
    state.tools.draw.session = Some(ShapeSession {
        tool,
        doc: tab.doc.raw() as usize,
        points,
        color,
        cursor: press.p,
        stage: 0,
        dragging,
        depth: tab.doc.history_counts().0,
    });
}

fn shape_up(state: &mut CanvasState, tab: &mut DocTab) {
    if !valid_session(state, tab) {
        return;
    }
    let Some(s) = &mut state.tools.draw.session else { return };
    match s.tool {
        Tool::Contour => {
            finish_shape(state, tab);
        }
        Tool::Curve => {
            s.dragging = false;
            match s.stage {
                0 => {
                    if s.points[0] == s.points[1] {
                        finish_shape(state, tab);
                        return;
                    }
                    s.stage = 1;
                }
                1 => s.stage = 2,
                _ => {
                    finish_shape(state, tab);
                    return;
                }
            }
            draw_shape(state, tab, false);
        }
        _ => {}
    }
}

/// Redraws the shape from the checkpoint, including the cursor as the next vertex or control point.
fn draw_shape(state: &mut CanvasState, tab: &mut DocTab, is_final: bool) {
    if !valid_session(state, tab) {
        return;
    }
    let Some(s) = state.tools.draw.session.clone() else { return };
    let brush = state.tools.draw.pen.brush;
    let flags = state.opts.sym_flags() | if state.opts.filled { flag::FILLED } else { 0 };
    let (layer, frame) = (tab.layer, tab.frame);
    match s.tool {
        Tool::Curve => {
            let (a, b) = (s.points[0], s.points[1]);
            let (c1, c2) = match s.stage {
                0 => (a, b),
                1 => {
                    if s.dragging || is_final {
                        (s.points[2], s.points[2])
                    } else {
                        (s.cursor, s.cursor)
                    }
                }
                _ => (s.points[2], if s.dragging || is_final { s.points[3] } else { s.cursor }),
            };
            tab.doc.curve_draw(layer, frame, &[a, c1, c2, b], s.color, brush, flags);
        }
        _ => {
            let mut pts = s.points.clone();
            let mut flags = flags;
            if !is_final && s.tool == Tool::ShapePolygon && pts.last() != Some(&s.cursor) {
                pts.push(s.cursor);
            }
            // While dragging a contour only the path is shown; releasing closes and fills it.
            if !is_final && s.tool == Tool::Contour {
                flags |= 32;
            }
            tab.doc.polygon_draw(layer, frame, &pts, s.color, brush, flags);
        }
    }
    mutated(tab);
}

/// Commits the shape as last drawn (a polygon without its cursor vertex).
pub fn finish_shape(state: &mut CanvasState, tab: &mut DocTab) {
    if !valid_session(state, tab) {
        return;
    }
    if state.tools.draw.session.as_ref().is_some_and(|s| matches!(s.tool, Tool::ShapePolygon | Tool::Contour)) {
        draw_shape(state, tab, true);
    }
    state.tools.draw.session = None;
    mutated(tab);
}

/// Throws the open shape away, rewinding to before it started.
pub fn cancel_shape(state: &mut CanvasState, tab: &mut DocTab) {
    if !valid_session(state, tab) {
        return;
    }
    state.tools.draw.session = None;
    if tab.doc.rollback() {
        mutated(tab);
    }
}

/// Switching tools ends an open shape (the observer on `$tool` in `beginShape`).
pub fn tool_changed(state: &mut CanvasState, tab: &mut DocTab, tool: Tool) {
    if state.tools.draw.session.as_ref().is_some_and(|s| s.tool != tool) {
        finish_shape(state, tab);
    }
}

/// Return commits and Escape cancels an open shape (`handleShapeKey`). Returns true if a key was used.
pub fn handle_key(state: &mut CanvasState, tab: &mut DocTab, ctx: &egui::Context) -> bool {
    if !valid_session(state, tab) {
        return false;
    }
    let (enter, escape) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
        )
    });
    if enter {
        finish_shape(state, tab);
    } else if escape {
        cancel_shape(state, tab);
    }
    enter || escape
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shading_ramp_follows_the_palette() {
        let pal = [10, 20, 30, 40, 50];
        assert_eq!(shading_ramp(&pal, 20, 40), vec![20, 30, 40]);
        assert_eq!(shading_ramp(&pal, 40, 20), vec![40, 30, 20]);
        assert_eq!(shading_ramp(&pal, 20, 99), vec![20, 99]);
        assert_eq!(shading_ramp(&pal, 20, 20), vec![20, 20]);
    }

    #[test]
    fn gradient_snaps_to_45_degree_steps() {
        assert_eq!(snapped((0, 0), (10, 2)), (10, 0));
        assert_eq!(snapped((0, 0), (2, -10)), (0, -10));
        assert_eq!(snapped((5, 5), (9, 8)), (9, 9));
        assert_eq!(snapped((5, 5), (1, 3)), (1, 1));
    }

    #[test]
    fn pen_pressure_scales_size_and_opacity_only_when_enabled() {
        let mut state = CanvasState::default();
        assert_eq!(pressure_style(&state, 0xFF0000FF, 10), None, "mouse input has no pressure");
        state.tools.draw.pressure = Some(0.5);
        assert_eq!(pressure_style(&state, 0xFF0000FF, 10), None, "both options are off");
        state.opts.pressure_size = true;
        assert_eq!(pressure_style(&state, 0xFF0000FF, 10), Some((0xFF0000FF, 5)));
        state.opts.pressure_alpha = true;
        assert_eq!(pressure_style(&state, 0xFF0000FF, 10), Some((0xFF000080, 5)));
        // The eraser's transparent color stays transparent; very light pressure keeps at least one pixel.
        state.tools.draw.pressure = Some(0.0);
        assert_eq!(pressure_style(&state, 0, 3), Some((0, 1)));
        assert_eq!(pressure_style(&state, 0xFF0000FF, 3).map(|v| v.0 & 0xFF), Some(5));
    }
}
