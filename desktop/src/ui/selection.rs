//! Selection tools, move, clipboard and the Edit / Select commands (`Selection.swift`, the outline part of
//! `CanvasOverlays.swift`).
//!
//! The canvas calls [`frame`] once per frame (tool and tab changes, keyboard), [`input`] for the pointer and
//! [`paint`] for the marching ants and handles. Every command the menus offer is a `pub fn` taking the app,
//! for example [`select_all`] or [`copy`]; the keyboard shortcuts run the same functions.

use crate::app::{DocTab, PixmithApp, Tool};
use crate::theme::{Colors, space};
use crate::ui::transform::{self, TransformState};
use crate::ui::widgets::{self, DialogAction};
use eframe::egui::{self, Color32, Key, Modifiers, PointerButton, Pos2, Rect, Stroke};

/// A selection mode of the interactive calls: replace, add (Shift), subtract (Option).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelMode {
    Replace = 0,
    Add = 1,
    Subtract = 2,
}

/// What a drag of the free transform does (`kind` of `pixmith_xf_drag`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XfDrag {
    Move = 0,
    Scale = 1,
    Rotate = 2,
    Skew = 3,
    Pivot = 4,
}

/// The pixels of the copy region (`pixmith_sel_copy`): the selection's bounds, or the whole canvas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopiedPixels {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// Straight-alpha RGBA, `w * h * 4` bytes.
    pub rgba: Vec<u8>,
}

/// Seconds between steps of the marching ants (`SelectionOutline`'s timer).
const ANTS_STEP: f64 = 0.12;
/// A second press this close in time and space is a double click.
const DOUBLE_CLICK: (f64, f32) = (0.5, 5.0);

/// The dialog of Select > Modify (`promptNumber`).
#[derive(Clone, Copy, Debug)]
pub struct ModifyDialog {
    /// 0 expand, 1 contract, 2 border.
    pub kind: u32,
    pub pixels: u32,
}

const MODIFY_TITLES: [&str; 3] = ["Expand Selection", "Contract Selection", "Border Selection"];

/// A press-drag-release of a selection tool (`SelectionDrag`).
#[derive(Clone, Debug)]
struct Drag {
    tool: Tool,
    start: (i32, i32),
    last: (i32, i32),
    moved: bool,
    mode: SelMode,
    points: Vec<(i32, i32)>,
    button: PointerButton,
    /// The document the drag started in.
    doc: usize,
}

/// The outline segments of the document at `(document, revision)`.
type Outline = ((usize, u64), Vec<[i32; 4]>);

/// State of the selection and transform code that outlives a frame.
pub struct SelectionState {
    /// Magic wand tolerance, 0 to 255 (`SelectionOptions`).
    pub tolerance: u32,
    pub contiguous: bool,
    pub xf: TransformState,
    pub modify: Option<ModifyDialog>,
    /// Off in tests so they never touch the system clipboard.
    pub system_clipboard: bool,
    drag: Option<Drag>,
    /// The open polygon of the Polygonal Lasso.
    polygon: Option<Vec<(i32, i32)>>,
    last_press: Option<(f64, Pos2)>,
    by_color_armed: bool,
    outline: Option<Outline>,
    /// The last copy, so a paste lands where it was copied from (`copyOrigin`).
    clip: Option<CopiedPixels>,
    /// Text put on the system clipboard by the last copy; see [`copy`].
    marker: String,
    pending_text: Option<String>,
    clipboard: Option<arboard::Clipboard>,
    seen_doc: usize,
    seen_tool: Option<Tool>,
}

impl Default for SelectionState {
    fn default() -> SelectionState {
        SelectionState {
            tolerance: 0,
            contiguous: true,
            xf: TransformState::default(),
            modify: None,
            system_clipboard: true,
            drag: None,
            polygon: None,
            last_press: None,
            by_color_armed: false,
            outline: None,
            clip: None,
            marker: String::new(),
            pending_text: None,
            clipboard: None,
            seen_doc: 0,
            seen_tool: None,
        }
    }
}

impl SelectionState {
    /// Whether a polygon of the Polygonal Lasso is still open.
    pub fn polygon_open(&self) -> bool {
        self.polygon.is_some()
    }

    /// Whether a selection drag or a transform handle drag is running.
    pub fn dragging(&self) -> bool {
        self.drag.is_some() || self.xf.dragging()
    }

    /// Forgets the drag state of the document being left (`resetSelectionTransients`).
    fn reset_transients(&mut self) {
        self.drag = None;
        self.polygon = None;
        self.by_color_armed = false;
        self.xf.reset_transients();
    }
}

fn is_selection_tool(tool: Tool) -> bool {
    matches!(tool, Tool::Marquee | Tool::Lasso | Tool::Polygon | Tool::Wand | Tool::Move)
}

fn mutated(tab: &mut DocTab) {
    tab.doc.touch();
    tab.changed();
}

fn mode_of(mods: Modifiers) -> SelMode {
    if mods.shift {
        SelMode::Add
    } else if mods.alt {
        SelMode::Subtract
    } else {
        SelMode::Replace
    }
}

// ---- Commands (menu entries and shortcuts) ----

/// Edit > Select All.
pub fn select_all(app: &mut PixmithApp) {
    let tab = app.tab_mut();
    tab.doc.sel_all();
    tab.doc.label_last_step("Select All");
    mutated(tab);
}

/// Select > Deselect.
pub fn deselect(app: &mut PixmithApp) {
    let tab = app.tab_mut();
    tab.doc.sel_clear();
    tab.doc.label_last_step("Deselect");
    mutated(tab);
}

/// Select > Invert Selection.
pub fn invert_selection(app: &mut PixmithApp) {
    let tab = app.tab_mut();
    tab.doc.sel_invert();
    tab.doc.label_last_step("Invert Selection");
    mutated(tab);
}

/// Whether the active document has a selection (enables Crop to Selection and the Modify items).
pub fn has_selection(app: &PixmithApp) -> bool {
    app.tab().doc.sel_active()
}

/// Edit > Delete (also the Delete key).
pub fn delete_selection(app: &mut PixmithApp) {
    let tab = app.tab_mut();
    tab.playing = false;
    let (layer, frame) = (tab.layer, tab.frame);
    if tab.doc.sel_delete(layer, frame) {
        tab.doc.label_last_step("Delete");
        mutated(tab);
    }
}

/// Edit > Flip / Rotate: 0 flip horizontal, 1 flip vertical, 2 rotate clockwise, 3 rotate counter-clockwise.
pub fn transform_selection(app: &mut PixmithApp, op: u32) {
    let tab = app.tab_mut();
    tab.playing = false;
    let (layer, frame) = (tab.layer, tab.frame);
    if tab.doc.sel_transform(layer, frame, op) {
        tab.doc.label_last_step("Transform");
        mutated(tab);
    }
}

/// Edit > Crop to Selection.
pub fn crop_to_selection(app: &mut PixmithApp) {
    let tab = app.tab_mut();
    tab.playing = false;
    if tab.doc.crop_to_selection() {
        tab.doc.label_last_step("Crop");
        mutated(tab);
    }
}

/// Select > Modify > Expand (0), Contract (1) or Border (2): opens the pixel count dialog.
pub fn modify_selection(app: &mut PixmithApp, kind: u32) {
    if has_selection(app) && (kind as usize) < MODIFY_TITLES.len() {
        app.selection.modify = Some(ModifyDialog { kind, pixels: 1 });
    }
}

/// Select > Select by Color: with the pointer over the canvas it selects every pixel of the current cel with
/// the color under it; otherwise the next click on the canvas picks the color.
pub fn select_by_color(app: &mut PixmithApp) {
    match app.canvas.cursor {
        Some(p) => select_by_color_at(app, p),
        None => app.selection.by_color_armed = true,
    }
}

fn select_by_color_at(app: &mut PixmithApp, p: (i32, i32)) {
    let tolerance = app.selection.tolerance;
    let tab = app.tab_mut();
    let (layer, frame) = (tab.layer, tab.frame);
    if tab.doc.sel_wand(layer, frame, p.0, p.1, tolerance, false, SelMode::Replace) {
        mutated(tab);
    }
}

/// Edit > Cut and Copy (`copySelection`): the selection's pixels, or the whole cel without a selection.
pub fn copy(app: &mut PixmithApp, cut: bool) {
    let tab = app.tab();
    let Some(pixels) = tab.doc.sel_copy(tab.layer, tab.frame) else { return };
    let sel = &mut app.selection;
    sel.marker = format!("Pixmith selection {} x {}", pixels.w, pixels.h);
    if sel.system_clipboard {
        // Ctrl+V reaches the app only as a paste event, which exists only for text on the clipboard.
        sel.pending_text = Some(sel.marker.clone());
    }
    sel.clip = Some(pixels);
    if cut {
        delete_selection(app);
    }
}

/// Whether Paste has something to paste.
pub fn can_paste(app: &mut PixmithApp) -> bool {
    app.selection.clip.is_some() || (app.selection.system_clipboard && system_image(&mut app.selection).is_some())
}

/// Edit > Paste (`pasteFromPasteboard`): a floating selection on the move tool, committed with Return,
/// cancelled with Esc. Pastes the last copy where it came from, or an image from the system clipboard in the
/// middle of the canvas.
pub fn paste(app: &mut PixmithApp) {
    let (cw, ch) = (app.tab().doc.width() as i32, app.tab().doc.height() as i32);
    let sel = &mut app.selection;
    let (text, image) = if sel.system_clipboard { (system_text(sel), system_image(sel)) } else { (None, None) };
    let internal_fresh = sel.clip.is_some() && (!sel.system_clipboard || text.as_deref() == Some(&sel.marker));
    let (w, h, rgba, origin) = if let (false, Some((w, h, rgba))) = (internal_fresh, image) {
        (w, h, rgba, ((cw - w as i32) / 2, (ch - h as i32) / 2))
    } else if let (Some(c), true) = (&sel.clip, internal_fresh || text.is_none()) {
        (c.w, c.h, c.rgba.clone(), (c.x, c.y))
    } else {
        return;
    };
    let origin = (origin.0.max(0), origin.1.max(0));
    // Switching to the move tool must not commit the paste that follows.
    sel.seen_tool = Some(Tool::Move);
    app.tool = Tool::Move;
    let tab = app.tab_mut();
    tab.playing = false;
    let (layer, frame) = (tab.layer, tab.frame);
    if tab.doc.sel_paste(layer, frame, w, h, &rgba, origin.0, origin.1) {
        tab.doc.label_last_step("Paste");
        mutated(tab);
    }
}

fn clipboard(sel: &mut SelectionState) -> Option<&mut arboard::Clipboard> {
    if sel.clipboard.is_none() {
        sel.clipboard = arboard::Clipboard::new().ok();
    }
    sel.clipboard.as_mut()
}

fn system_text(sel: &mut SelectionState) -> Option<String> {
    clipboard(sel)?.get_text().ok()
}

fn system_image(sel: &mut SelectionState) -> Option<(u32, u32, Vec<u8>)> {
    let img = clipboard(sel)?.get_image().ok()?;
    let (w, h) = (img.width as u32, img.height as u32);
    (w > 0 && h > 0 && w <= u16::MAX as u32 && h <= u16::MAX as u32).then(|| (w, h, img.bytes.into_owned()))
}

/// Commits a floating paste and a free transform, and closes an open polygon, so the document is what is on
/// screen (`finishTransients`). Call before saving or leaving the document.
pub fn finish_transients(sel: &mut SelectionState, tab: &mut DocTab) {
    tab.playing = false;
    if sel.polygon.take().is_some() {
        tab.doc.sel_end();
        mutated(tab);
    }
    if tab.doc.float_active() {
        tab.doc.float_commit();
        mutated(tab);
    }
    transform::commit(tab);
    sel.reset_transients();
}

// ---- Per frame: tool and tab changes, keyboard ----

/// Called once per frame before the canvas handles the pointer.
pub fn frame(app: &mut PixmithApp, ctx: &egui::Context) {
    if let Some(t) = app.selection.pending_text.take() {
        ctx.copy_text(t);
    }
    // A tab switch commits what floats in the document being left.
    let doc = app.tab().doc.raw() as usize;
    if app.selection.seen_doc != doc {
        let left = app.selection.seen_doc;
        let PixmithApp { tabs, selection: sel, .. } = app;
        if let Some(tab) = tabs.iter_mut().find(|t| t.doc.raw() as usize == left) {
            finish_transients(sel, tab);
        }
        sel.reset_transients();
        sel.seen_doc = doc;
    }
    // Changing tools commits an open polygon, a floating paste and a transform.
    if app.selection.seen_tool != Some(app.tool) {
        let first = app.selection.seen_tool.is_none();
        app.selection.seen_tool = Some(app.tool);
        if !first {
            let PixmithApp { tabs, active, selection: sel, .. } = app;
            finish_transients(sel, &mut tabs[*active]);
        }
    }
    if modal_open(app, ctx) {
        return;
    }
    shortcuts(app, ctx);
    plain_keys(app, ctx);
}

/// Whether a dialog covers the canvas or a text field has the keyboard.
fn modal_open(app: &PixmithApp, ctx: &egui::Context) -> bool {
    ctx.egui_wants_keyboard_input() || app.selection.modify.is_some() || ctx.memory(|m| m.top_modal_layer().is_some())
}

/// Ctrl/Cmd shortcuts of the Edit and Select menus.
fn shortcuts(app: &mut PixmithApp, ctx: &egui::Context) {
    const COMMAND_ALT: Modifiers = Modifiers { alt: true, command: true, ..Modifiers::NONE };
    const COMMAND_SHIFT: Modifiers = Modifiers { shift: true, command: true, ..Modifiers::NONE };
    let (by_color, invert, all, deselect_, transform_, copy_, cut_, paste_) = ctx.input_mut(|i| {
        let by_color = i.consume_key(COMMAND_ALT, Key::B);
        let invert = i.consume_key(COMMAND_SHIFT, Key::I);
        let all = i.consume_key(Modifiers::COMMAND, Key::A);
        let deselect_ = i.consume_key(Modifiers::COMMAND, Key::D);
        let transform_ = i.consume_key(Modifiers::COMMAND, Key::T);
        let copy_ = i.consume_key(Modifiers::COMMAND, Key::C);
        let cut_ = i.consume_key(Modifiers::COMMAND, Key::X);
        let paste_ = i.consume_key(Modifiers::COMMAND, Key::V);
        let (mut ec, mut ex, mut ep) = (false, false, false);
        for e in &i.events {
            match e {
                egui::Event::Copy => ec = true,
                egui::Event::Cut => ex = true,
                egui::Event::Paste(_) => ep = true,
                _ => {}
            }
        }
        (by_color, invert, all, deselect_, transform_, copy_ || ec, cut_ || ex, paste_ || ep)
    });
    if by_color {
        select_by_color(app);
    } else if invert {
        invert_selection(app);
    } else if all {
        select_all(app);
    } else if deselect_ {
        deselect(app);
    } else if transform_ {
        transform::start(app);
    } else if cut_ {
        copy(app, true);
    } else if copy_ {
        copy(app, false);
    } else if paste_ {
        paste(app);
    }
}

/// Arrow keys, Delete, Return and Escape without Ctrl, Cmd or Option.
fn plain_keys(app: &mut PixmithApp, ctx: &egui::Context) {
    let (blocked, shift) =
        ctx.input(|i| (i.modifiers.command || i.modifiers.ctrl || i.modifiers.alt, i.modifiers.shift));
    if blocked {
        return;
    }
    let step = if shift { 10 } else { 1 };
    let pressed = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
    let xf_active = app.tab().doc.xf_active();
    if xf_active {
        let tab = app.tab_mut();
        if pressed(Key::Enter) {
            transform::commit(tab);
        } else if pressed(Key::Escape) {
            transform::cancel(tab);
        } else if pressed(Key::ArrowLeft) {
            transform::nudge(tab, -step as f64, 0.0);
        } else if pressed(Key::ArrowRight) {
            transform::nudge(tab, step as f64, 0.0);
        } else if pressed(Key::ArrowDown) {
            transform::nudge(tab, 0.0, step as f64);
        } else if pressed(Key::ArrowUp) {
            transform::nudge(tab, 0.0, -step as f64);
        }
        return;
    }
    let can_nudge = app.tool == Tool::Move || has_selection(app);
    let nudge = |app: &mut PixmithApp, dx: i32, dy: i32| {
        let tab = app.tab_mut();
        tab.playing = false;
        let (layer, frame) = (tab.layer, tab.frame);
        if tab.doc.sel_nudge(layer, frame, dx, dy) {
            tab.doc.label_last_step("Nudge");
            mutated(tab);
        }
    };
    if can_nudge && pressed(Key::ArrowLeft) {
        nudge(app, -step, 0);
    } else if can_nudge && pressed(Key::ArrowRight) {
        nudge(app, step, 0);
    } else if can_nudge && pressed(Key::ArrowDown) {
        nudge(app, 0, step);
    } else if can_nudge && pressed(Key::ArrowUp) {
        nudge(app, 0, -step);
    } else if app.tool != Tool::Slice && (pressed(Key::Backspace) || pressed(Key::Delete)) {
        delete_selection(app);
    } else if (app.selection.polygon_open() || app.tab().doc.float_active()) && pressed(Key::Enter) {
        let PixmithApp { tabs, active, selection: sel, .. } = app;
        let tab = &mut tabs[*active];
        finish_polygon(sel, tab);
        tab.doc.float_commit();
        mutated(tab);
    } else if pressed_escape_target(app) && pressed(Key::Escape) {
        let PixmithApp { tabs, active, selection: sel, .. } = app;
        let tab = &mut tabs[*active];
        if sel.polygon.is_some() {
            cancel_polygon(sel, tab);
        } else if tab.doc.float_active() {
            if tab.doc.float_cancel() {
                tab.changed();
            }
        } else {
            tab.doc.sel_clear();
            tab.doc.label_last_step("Deselect");
            mutated(tab);
        }
    }
}

/// Escape does something only with an open polygon, a floating paste or a selection.
fn pressed_escape_target(app: &PixmithApp) -> bool {
    app.selection.polygon_open() || app.tab().doc.float_active() || has_selection(app)
}

fn finish_polygon(sel: &mut SelectionState, tab: &mut DocTab) {
    if sel.polygon.take().is_some() {
        tab.doc.sel_end();
        mutated(tab);
    }
}

/// Closes the polygon and undoes the step it recorded (`cancelPolygon`).
fn cancel_polygon(sel: &mut SelectionState, tab: &mut DocTab) {
    if sel.polygon.is_some() {
        finish_polygon(sel, tab);
        if tab.doc.undo() {
            tab.changed();
        }
    }
}

// ---- Pointer ----

/// Pixel under `pos` and the continuous canvas coordinates of it.
fn locate(srect: Rect, zoom: f32, pos: Pos2) -> ((i32, i32), (f64, f64)) {
    let p = (pos - srect.min) / zoom;
    ((p.x.floor() as i32, p.y.floor() as i32), (p.x as f64, p.y as f64))
}

/// Handles the pointer for the selection tools, the free transform handles and a pending "Select by Color".
/// `idle` is false while the canvas pans. Returns true when this module owns the pointer, so the drawing
/// tools stay out (`CanvasNSView.mouseDown` order: transform, select by color, selection tools).
pub fn input(
    app_sel: &mut SelectionState,
    tab: &mut DocTab,
    tool: Tool,
    idle: bool,
    response: &egui::Response,
    srect: Rect,
    ctx: &egui::Context,
) -> bool {
    let sel = app_sel;
    let doc = tab.doc.raw() as usize;
    if sel.drag.as_ref().is_some_and(|d| d.doc != doc) {
        sel.drag = None;
    }
    if sel.xf.drag_doc().is_some_and(|d| d != doc) {
        sel.xf.reset_transients();
    }
    let (pos, mods, primary, secondary, time) = ctx.input(|i| {
        (
            i.pointer.latest_pos(),
            i.modifiers,
            i.pointer.button_pressed(PointerButton::Primary),
            i.pointer.button_pressed(PointerButton::Secondary),
            i.time,
        )
    });
    let located = pos.map(|pos| locate(srect, tab.zoom, pos));
    let mut owned = false;

    // A drag in progress keeps the pointer until release.
    if sel.xf.dragging() {
        transform::drag(sel, tab, ctx, located.map(|l| l.1));
        owned = true;
    } else if sel.drag.is_some() {
        selection_drag(sel, tab, ctx, located.map(|l| l.0));
        owned = true;
    } else if idle && response.hovered() && (primary || secondary) {
        let button = if secondary { PointerButton::Secondary } else { PointerButton::Primary };
        if let (Some((p, c)), Some(pos)) = (located, pos) {
            owned = press(sel, tab, tool, p, c, pos, button, mods, time, ctx);
        }
    }
    owned
        || tab.doc.xf_active()
        || sel.by_color_armed
        || sel.dragging()
        || (is_selection_tool(tool) && !(tool == Tool::Move && mods.alt))
}

#[allow(clippy::too_many_arguments)]
fn press(
    sel: &mut SelectionState,
    tab: &mut DocTab,
    tool: Tool,
    p: (i32, i32),
    c: (f64, f64),
    pos: Pos2,
    button: PointerButton,
    mods: Modifiers,
    time: f64,
    ctx: &egui::Context,
) -> bool {
    if transform::press(sel, tab, tool, c, mods, ctx) {
        return true;
    }
    if sel.by_color_armed {
        sel.by_color_armed = false;
        let tolerance = sel.tolerance;
        let (layer, frame) = (tab.layer, tab.frame);
        if tab.doc.sel_wand(layer, frame, p.0, p.1, tolerance, false, SelMode::Replace) {
            mutated(tab);
        }
        return true;
    }
    // Alt with the move tool picks a color, like the drawing tools.
    if !is_selection_tool(tool) || (tool == Tool::Move && mods.alt) {
        return false;
    }
    tab.playing = false;
    let mode = mode_of(mods);
    let (layer, frame) = (tab.layer, tab.frame);
    match tool {
        Tool::Wand => {
            if tab.doc.sel_wand(layer, frame, p.0, p.1, sel.tolerance, sel.contiguous, mode) {
                mutated(tab);
            }
        }
        Tool::Polygon => {
            let double =
                sel.last_press.is_some_and(|(t, q)| time - t <= DOUBLE_CLICK.0 && q.distance(pos) <= DOUBLE_CLICK.1);
            sel.last_press = Some((time, pos));
            if double {
                finish_polygon(sel, tab);
                return true;
            }
            if sel.polygon.is_none() {
                tab.doc.sel_begin(mode);
                sel.polygon = Some(Vec::new());
            }
            if let Some(points) = &mut sel.polygon {
                points.push(p);
                tab.doc.sel_poly(points);
            }
            mutated(tab);
        }
        Tool::Move => {
            if tab.doc.float_active() && !tab.doc.sel_contains(p.0, p.1) {
                tab.doc.float_commit();
                mutated(tab);
                return true;
            }
            if tab.doc.sel_move_begin(layer, frame) {
                tab.doc.label_last_step("Move");
                sel.drag = Some(Drag {
                    tool,
                    start: p,
                    last: p,
                    moved: false,
                    mode,
                    points: Vec::new(),
                    button,
                    doc: tab.doc.raw() as usize,
                });
            }
        }
        _ => {
            sel.drag = Some(Drag {
                tool,
                start: p,
                last: p,
                moved: false,
                mode,
                points: vec![p],
                button,
                doc: tab.doc.raw() as usize,
            });
        }
    }
    true
}

/// `selectionMouseDragged` and `selectionMouseUp`.
fn selection_drag(sel: &mut SelectionState, tab: &mut DocTab, ctx: &egui::Context, p: Option<(i32, i32)>) {
    let Some(mut d) = sel.drag.take() else { return };
    let down = ctx.input(|i| i.pointer.button_down(d.button));
    if let Some(p) = p.filter(|p| *p != d.last) {
        d.last = p;
        if p != d.start || d.moved {
            if d.tool == Tool::Move {
                tab.doc.sel_move_to(p.0 - d.start.0, p.1 - d.start.1);
            } else {
                if !d.moved {
                    tab.doc.sel_begin(d.mode);
                }
                if d.tool == Tool::Lasso {
                    if d.points.last() != Some(&p) {
                        d.points.push(p);
                    }
                    tab.doc.sel_poly(&d.points);
                } else {
                    tab.doc.sel_rect(d.start, p);
                }
            }
            d.moved = true;
            mutated(tab);
        }
    }
    if down {
        sel.drag = Some(d);
        return;
    }
    if d.tool == Tool::Move {
        tab.doc.sel_move_end();
    } else if d.moved {
        tab.doc.sel_end();
    } else if d.mode == SelMode::Replace {
        tab.doc.sel_clear();
    }
    mutated(tab);
}

// ---- Painting ----

/// Refreshes the cached outline when the document changed.
pub fn update_outline(sel: &mut SelectionState, tab: &DocTab) {
    let key = (tab.doc.raw() as usize, tab.revision);
    if sel.outline.as_ref().map(|o| o.0) != Some(key) {
        sel.outline = Some((key, tab.doc.sel_outline()));
    }
}

/// Marching ants, then the transform box and handles (`drawSelectionOutline`, `drawTransformOverlay`).
pub fn paint(sel: &SelectionState, tab: &DocTab, tool: Tool, painter: &egui::Painter, srect: Rect) {
    let zoom = tab.zoom;
    let at = |x: f64, y: f64| srect.min + egui::vec2(x as f32, y as f32) * zoom;
    if let Some((_, segs)) = &sel.outline
        && !segs.is_empty()
    {
        let time = painter.ctx().input(|i| i.time);
        let phase = ((time / ANTS_STEP).floor() as i64).rem_euclid(8) as f32;
        painter.ctx().request_repaint_after(std::time::Duration::from_secs_f64(ANTS_STEP));
        let clip = painter.clip_rect().expand(2.0);
        let white = Stroke::new(1.0, Color32::WHITE);
        let black = Stroke::new(1.0, Color32::BLACK);
        let visible: Vec<[Pos2; 2]> = segs
            .iter()
            .map(|s| [at(s[0] as f64, s[1] as f64), at(s[2] as f64, s[3] as f64)])
            .filter(|[a, b]| clip.intersects(Rect::from_two_pos(*a, *b)))
            .collect();
        for s in &visible {
            painter.line_segment(*s, white);
        }
        for s in &visible {
            painter.extend(egui::Shape::dashed_line_with_offset(s, black, &[4.0], &[4.0], phase));
        }
    }
    transform::paint_overlay(&sel.xf, tab, tool, painter, srect);
}

// ---- Context bar and dialogs ----

/// The magic wand's options in the context bar (`SelectionOptionsView`).
pub fn wand_options(ui: &mut egui::Ui, sel: &mut SelectionState) {
    widgets::bar_number_field(
        ui,
        "Tolerance",
        &mut sel.tolerance,
        (0, 255),
        "",
        128.0,
        "Magic wand tolerance (0-255). Drag the label to scrub.",
    );
    let (icon, title, help) = if sel.contiguous {
        (egui_phosphor::regular::LINK, "Contiguous", "Contiguous (click for global)")
    } else {
        (egui_phosphor::regular::GLOBE, "Global", "Global (click for contiguous)")
    };
    if icon_text_chip(ui, icon, title, sel.contiguous, help).clicked() {
        sel.contiguous = !sel.contiguous;
    }
}

/// A toggle chip with a leading symbol and a title (`DSChip(title, symbol:)`).
pub fn icon_text_chip(ui: &mut egui::Ui, icon: &str, title: &str, on: bool, help: &str) -> egui::Response {
    let c = Colors::of(ui.ctx());
    let probe = ui.painter().layout_no_wrap(title.to_owned(), widgets::strong(ui.ctx(), 11.0), c.text);
    let w = 14.0 + 5.0 + probe.size().x + 2.0 * (space::SM + 2.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(w, 26.0), egui::Sense::click());
    let (hover, down) = (response.hovered(), response.is_pointer_button_down_on());
    let fill = if on {
        if hover || down { c.selected_hover } else { c.selected }
    } else if down {
        c.pressed
    } else if hover {
        c.hover
    } else {
        c.field
    };
    ui.painter().rect_filled(rect, crate::theme::radius::CONTROL, fill);
    let fg = if on {
        c.accent
    } else if hover {
        c.text
    } else {
        c.text_secondary
    };
    let x = rect.left() + space::SM + 2.0;
    ui.painter().text(
        egui::pos2(x + 7.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(14.0),
        fg,
    );
    let galley = ui.painter().layout_no_wrap(title.to_owned(), widgets::strong(ui.ctx(), 11.0), fg);
    ui.painter().galley(egui::pos2(x + 19.0, rect.center().y - galley.size().y / 2.0), galley, fg);
    response.on_hover_text(help)
}

/// The Modify Selection dialog. Call after the other dialogs.
pub fn dialogs(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(mut m) = app.selection.modify else { return };
    let title = MODIFY_TITLES[(m.kind as usize).min(2)];
    let action = widgets::dialog(ctx, "modify-selection", 300.0, title, None, |ui| {
        widgets::form_row(ui, "Pixels", |ui| widgets::number_field(ui, "N", &mut m.pixels, 0..=1024, "px", 120.0));
        widgets::dialog_buttons(ui, None, Some("Cancel"), "OK", true)
    });
    app.selection.modify = Some(m);
    match action {
        DialogAction::Primary => {
            app.selection.modify = None;
            let tab = app.tab_mut();
            if tab.doc.sel_modify(m.kind, m.pixels) {
                tab.doc.label_last_step(title);
                mutated(tab);
            }
        }
        DialogAction::Cancel => app.selection.modify = None,
        _ => {}
    }
}
