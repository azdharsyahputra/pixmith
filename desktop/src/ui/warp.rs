//! Edit > Warp: a puppet / mesh warp of the cel or selection. A grid of handles lies over the selection
//! bounds (or the canvas); drag a handle to move the content under it, right-click it to put it back. The canvas
//! shows the majority-vote result live (`pixmith_warp_preview`), Apply makes it one undo step. Follows the free
//! transform overlay: the pointer is owned by the session until it ends.

use crate::app::{DocTab, PixmithApp};
use crate::theme::{Colors, space};
use crate::ui::timeline::playback;
use crate::ui::widgets::{self, DialogAction, SegGlyph};
use eframe::egui::{self, Align2, Color32, CornerRadius, Margin, Pos2, Rect, Stroke, StrokeKind, vec2};
use pixmith_core::paint::warp::{MAX_GRID, MAX_SAMPLES, Warp};

/// Screen distance within which a click grabs a handle.
const GRAB: f32 = 10.0;

/// A handle drag: the handle, the pointer's canvas position at the press and the handle's offset then.
#[derive(Clone, Copy, Debug)]
struct Grab {
    handle: usize,
    start: (f64, f64),
    offset: (f64, f64),
}

/// An open warp: the handles being edited and the preview bookkeeping.
pub struct WarpSession {
    /// The document it belongs to (`Doc::raw`).
    doc: usize,
    pub warp: Warp,
    grab: Option<Grab>,
    /// The warp changed since the preview was computed.
    dirty: bool,
    pixels: Vec<u8>,
}

impl WarpSession {
    pub fn dragging(&self) -> bool {
        self.grab.is_some()
    }

    /// Where handle `k` is now: its grid position plus its displacement, in canvas coordinates.
    fn handle(&self, k: usize) -> (f64, f64) {
        let (i, j) = ((k as u32) % self.warp.cols, (k as u32) / self.warp.cols);
        let p = self.warp.handle_pos(i, j);
        let o = self.warp.offsets[k];
        (p.0 + o.0, p.1 + o.1)
    }

    /// The same deformation on a grid of another size: every new handle takes the displacement the old field
    /// has at its position.
    fn regrid(&mut self, cols: u32, rows: u32) {
        let mut next = Warp::new(cols, rows, self.warp.region);
        next.samples = self.warp.samples;
        next.smooth = self.warp.smooth;
        for j in 0..next.rows {
            for i in 0..next.cols {
                next.offsets[(j * next.cols + i) as usize] = self.warp.displacement(next.handle_pos(i, j));
            }
        }
        self.warp = next;
        self.dirty = true;
    }
}

/// Edit > Warp…: starts a session on the active cel (nothing happens while one is open).
pub fn start(app: &mut PixmithApp) {
    if app.selection.warp.is_some() {
        return;
    }
    let tab = app.tab_mut();
    if tab.doc.xf_active() {
        return;
    }
    playback::stop(tab);
    let region = tab.doc.warp_bounds();
    if region.2 == 0 || region.3 == 0 {
        return;
    }
    tab.doc.touch();
    let doc = tab.doc.raw() as usize;
    app.selection.warp =
        Some(WarpSession { doc, warp: Warp::new(3, 3, region), grab: None, dirty: true, pixels: Vec::new() });
}

// ---- Pointer ----

/// Moves a grabbed handle, or grabs the one under a press. Returns true while a session owns the pointer.
pub fn input(
    session: &mut Option<WarpSession>,
    tab: &DocTab,
    idle: bool,
    response: &egui::Response,
    srect: Rect,
    ctx: &egui::Context,
) -> bool {
    let Some(s) = session.as_mut().filter(|s| s.doc == tab.doc.raw() as usize) else { return false };
    let zoom = tab.zoom;
    let canvas_of = |pos: Pos2| {
        let p = (pos - srect.min) / zoom;
        (p.x as f64, p.y as f64)
    };
    let (pos, down, primary, secondary) = ctx.input(|i| {
        (
            i.pointer.latest_pos(),
            i.pointer.any_down(),
            i.pointer.button_pressed(egui::PointerButton::Primary),
            i.pointer.button_pressed(egui::PointerButton::Secondary),
        )
    });
    if let Some(g) = s.grab {
        if !down {
            s.grab = None;
        } else if let Some(pos) = pos {
            let c = canvas_of(pos);
            let o = (g.offset.0 + c.0 - g.start.0, g.offset.1 + c.1 - g.start.1);
            if s.warp.offsets[g.handle] != o {
                s.warp.offsets[g.handle] = o;
                s.dirty = true;
            }
        }
    } else if idle
        && response.hovered()
        && (primary || secondary)
        && let Some(pos) = pos
    {
        let near = (0..s.warp.offsets.len())
            .map(|k| {
                let h = s.handle(k);
                (k, (srect.min + vec2(h.0 as f32, h.1 as f32) * zoom).distance(pos))
            })
            .filter(|(_, d)| *d <= GRAB)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((k, _)) = near {
            if secondary {
                s.warp.offsets[k] = (0.0, 0.0);
                s.dirty = true;
            } else {
                s.grab = Some(Grab { handle: k, start: canvas_of(pos), offset: s.warp.offsets[k] });
            }
        }
    }
    true
}

// ---- Overlay ----

/// The deformed mesh and its handles.
pub fn paint_overlay(session: &Option<WarpSession>, tab: &DocTab, painter: &egui::Painter, srect: Rect) {
    let Some(s) = session.as_ref().filter(|s| s.doc == tab.doc.raw() as usize) else { return };
    let at = |p: (f64, f64)| srect.min + vec2(p.0 as f32, p.1 as f32) * tab.zoom;
    let (cols, rows) = (s.warp.cols as usize, s.warp.rows as usize);
    let pts: Vec<Pos2> = (0..cols * rows).map(|k| at(s.handle(k))).collect();
    let (white, black) = (Stroke::new(1.0, Color32::WHITE), Stroke::new(1.0, Color32::from_black_alpha(160)));
    for j in 0..rows {
        for i in 0..cols {
            if i + 1 < cols {
                painter.line_segment([pts[j * cols + i], pts[j * cols + i + 1]], black);
                painter.line_segment([pts[j * cols + i], pts[j * cols + i + 1]], white);
            }
            if j + 1 < rows {
                painter.line_segment([pts[j * cols + i], pts[(j + 1) * cols + i]], black);
                painter.line_segment([pts[j * cols + i], pts[(j + 1) * cols + i]], white);
            }
        }
    }
    let accent = Colors::of(painter.ctx()).accent;
    for (k, p) in pts.iter().enumerate() {
        let moved = s.warp.offsets[k] != (0.0, 0.0);
        let r = Rect::from_center_size(*p, egui::Vec2::splat(8.0));
        painter.rect_filled(r, 0.0, if moved { accent } else { Color32::WHITE });
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Middle);
    }
}

// ---- Panel and preview ----

/// The floating panel (Samples, grid, Apply, Cancel) and the live preview; call every frame.
pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(mut s) = app.selection.warp.take() else { return };
    if s.doc != app.tab().doc.raw() as usize {
        // The tab changed under the session.
        app.canvas.preview = None;
        return;
    }
    let c = Colors::of(ctx);
    let frame = egui::Frame::new()
        .fill(c.panel)
        .stroke(Stroke::new(1.0, c.separator))
        .corner_radius(CornerRadius::same(crate::theme::radius::PANEL + 4))
        .inner_margin(Margin::same(16))
        .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(90) });
    let action = egui::Area::new(egui::Id::new("warp_panel"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_TOP, vec2(0.0, 27.0))
        .show(ctx, |ui| {
            frame
                .show(ui, |ui| {
                    ui.set_width(300.0);
                    ui.spacing_mut().item_spacing = vec2(space::SM, space::MD);
                    ui.label(egui::RichText::new("Warp").font(widgets::strong(ui.ctx(), 15.0)));
                    widgets::note(ui, "Drag a handle to bend the pixels. Right-click a handle to reset it.");
                    controls(ui, &mut s);
                    widgets::dialog_buttons(ui, Some("Reset"), Some("Cancel"), "Apply", true)
                })
                .inner
        })
        .inner;
    match action {
        DialogAction::Primary => {
            let tab = app.tab_mut();
            if tab.doc.warp_apply(tab.layer, tab.frame, &s.warp) {
                tab.changed();
            }
            app.canvas.preview = None;
            return;
        }
        DialogAction::Cancel => {
            app.canvas.preview = None;
            return;
        }
        DialogAction::Destructive => {
            let region = s.warp.region;
            let (cols, rows, samples, smooth) = (s.warp.cols, s.warp.rows, s.warp.samples, s.warp.smooth);
            s.warp = Warp::new(cols, rows, region);
            (s.warp.samples, s.warp.smooth, s.dirty) = (samples, smooth, true);
        }
        DialogAction::None => {}
    }
    if s.dirty {
        update_preview(app, ctx, &mut s);
    }
    app.selection.warp = Some(s);
}

fn controls(ui: &mut egui::Ui, s: &mut WarpSession) {
    let (mut cols, mut rows) = (s.warp.cols, s.warp.rows);
    widgets::form_row(ui, "Grid", |ui| {
        let a = widgets::number_field(ui, "Cols", &mut cols, 2..=MAX_GRID, "", 100.0);
        let b = widgets::number_field(ui, "Rows", &mut rows, 2..=MAX_GRID, "", 100.0);
        a || b
    });
    if (cols, rows) != (s.warp.cols, s.warp.rows) {
        s.regrid(cols, rows);
    }
    widgets::form_row(ui, "Samples", |ui| {
        let mut n = s.warp.samples;
        if widgets::number_field(ui, "Samples", &mut n, 1..=MAX_SAMPLES, "per axis", 140.0) {
            s.warp.samples = n;
            s.dirty = true;
        }
    });
    widgets::form_row(ui, "Curve", |ui| {
        ui.push_id("warp-smooth", |ui| {
            let options = [(SegGlyph::Text("Linear"), "Linear"), (SegGlyph::Text("Smooth"), "Smooth")];
            if let Some(i) = widgets::bar_segmented(ui, s.warp.smooth as usize, &options) {
                s.warp.smooth = i == 1;
                s.dirty = true;
            }
        });
    });
}

/// Puts the warped frame on the canvas in place of the sprite.
fn update_preview(app: &mut PixmithApp, ctx: &egui::Context, s: &mut WarpSession) {
    s.dirty = false;
    let PixmithApp { tabs, active, canvas, .. } = app;
    let tab = &tabs[*active];
    if tab.doc.warp_preview(tab.layer, tab.frame, &s.warp, &mut s.pixels) {
        let size = [tab.doc.width() as usize, tab.doc.height() as usize];
        let image = egui::ColorImage::from_rgba_unmultiplied(size, &s.pixels);
        match &mut canvas.preview {
            Some(t) => t.set(image, egui::TextureOptions::NEAREST),
            None => canvas.preview = Some(ctx.load_texture("warp-preview", image, egui::TextureOptions::NEAREST)),
        }
    } else {
        canvas.preview = None;
    }
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> WarpSession {
        WarpSession { doc: 0, warp: Warp::new(3, 3, (0, 0, 16, 8)), grab: None, dirty: false, pixels: Vec::new() }
    }

    #[test]
    fn handles_sit_on_the_grid_plus_their_offset() {
        let mut s = session();
        assert_eq!(s.handle(0), (0.0, 0.0));
        assert_eq!(s.handle(4), (8.0, 4.0));
        s.warp.offsets[4] = (1.5, -2.0);
        assert_eq!(s.handle(4), (9.5, 2.0));
        assert_eq!(s.handle(8), (16.0, 8.0));
    }

    #[test]
    fn regridding_keeps_the_deformation() {
        let mut s = session();
        s.warp.offsets[4] = (2.0, 0.0);
        s.regrid(5, 3);
        assert_eq!((s.warp.cols, s.warp.rows), (5, 3));
        // The center handle is still at the center and keeps its displacement; the edge ones stay put.
        assert_eq!(s.warp.offsets[7].0, 2.0);
        assert_eq!(s.warp.offsets[0], (0.0, 0.0));
        assert!(s.dirty);
    }
}
