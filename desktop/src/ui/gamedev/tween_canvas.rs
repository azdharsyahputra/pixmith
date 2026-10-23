//! The tween overlay and handles on the canvas (`TweenView.swift`, "Canvas overlay and handles"): the motion
//! path, the transform box with white square handles, the rotate handle and the orange pivot, and the drag
//! that edits the keys. Hooked into `ui/canvas.rs` with two calls: [`input`] before the tools see the mouse and
//! [`paint`] after the sprite is drawn.

use super::tween::TweenState;
use super::{TweenProp, TweenTransform};
use crate::app::DocTab;
use crate::theme::Colors;
use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, pos2};

/// What a canvas drag edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Move,
    Rotate,
    Scale,
    Pivot,
}

/// A pointer drag in progress on the canvas (`TweenDrag`).
#[derive(Clone, Copy, Debug)]
pub struct CanvasDrag {
    kind: Kind,
    /// Where the drag started, in sprite pixels (for a scale drag: the grabbed corner).
    start: (f64, f64),
    /// The transform when the drag started.
    t: [f32; 8],
    start_angle: f64,
    start_dist: f64,
    /// Bit i is set when property i has a key at the current frame.
    keys_at_frame: u32,
    /// The document the drag started in.
    doc: usize,
    /// Something was written, so the transaction deserves a label.
    changed: bool,
}

/// The transform box: corners (TL, TR, BR, BL), the pivot's destination and the rotate handle, in sprite pixels.
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub corners: [(f64, f64); 4],
    pub pivot: (f64, f64),
    pub rotate: (f64, f64),
}

/// Transform at the current frame, or an identity one around the effective pivot when the layer has no keys.
fn current(tab: &DocTab) -> Option<[f32; 8]> {
    if let Some(TweenTransform(t)) = tab.doc.tween_transform(tab.layer, tab.frame) {
        return Some(t);
    }
    let (px, py) = tab.doc.tween_pivot(tab.layer)?;
    Some([0.0, 0.0, 0.0, 1.0, 1.0, 1.0, px, py])
}

/// `tweenGeometry`: where the source bounds `b` end up under transform `t`.
pub fn geometry(t: &[f32; 8], b: &[f32; 4], zoom: f32) -> Geometry {
    let rad = (t[2] as f64).to_radians();
    let (c, s) = (rad.cos(), rad.sin());
    let pv = (t[6] as f64, t[7] as f64);
    let dest = (pv.0 + t[0] as f64, pv.1 + t[1] as f64);
    let map = |x: f32, y: f32| {
        let (dx, dy) = ((x as f64 - pv.0) * t[3] as f64, (y as f64 - pv.1) * t[4] as f64);
        (dest.0 + c * dx - s * dy, dest.1 + s * dx + c * dy)
    };
    let corners = [map(b[0], b[1]), map(b[2], b[1]), map(b[2], b[3]), map(b[0], b[3])];
    let top = ((corners[0].0 + corners[1].0) / 2.0, (corners[0].1 + corners[1].1) / 2.0);
    let center = ((corners[0].0 + corners[2].0) / 2.0, (corners[0].1 + corners[2].1) / 2.0);
    let len = (top.0 - center.0).hypot(top.1 - center.1).max(0.001);
    let off = 14.0 / zoom as f64;
    let rotate = (top.0 + (top.0 - center.0) / len * off, top.1 + (top.1 - center.1) / len * off);
    Geometry { corners, pivot: dest, rotate }
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Whether the overlay shows: the panel is open or "Edit on canvas" is on.
pub fn visible(st: &TweenState) -> bool {
    st.open || st.edit_on_canvas
}

/// Motion path, box and handles of the current layer's tween (`drawTweenOverlay`).
pub fn paint(st: &TweenState, tab: &DocTab, painter: &egui::Painter, c: &Colors, srect: Rect) {
    if !visible(st) {
        return;
    }
    let Some((lo, hi)) = tab.doc.tween_range(tab.layer) else { return };
    let z = tab.zoom;
    let screen = |x: f64, y: f64| pos2(srect.left() + x as f32 * z, srect.top() + y as f32 * z);
    let mut key_frames = std::collections::HashSet::new();
    for prop in TweenProp::ALL {
        for k in tab.doc.tween_keys(tab.layer, prop) {
            key_frames.insert(k.frame);
        }
    }
    let mut path: Vec<Pos2> = Vec::new();
    let mut dots: Vec<(Pos2, bool, bool)> = Vec::new();
    for f in lo..=hi {
        let Some(TweenTransform(t)) = tab.doc.tween_transform(tab.layer, f) else { continue };
        let pt = screen(t[6] as f64 + t[0] as f64, t[7] as f64 + t[1] as f64);
        path.push(pt);
        dots.push((pt, key_frames.contains(&f), f == tab.frame));
    }
    // The motion path in the accent color with a dark hairline under it, so it reads on any art.
    if path.len() > 1 {
        painter.add(Shape::line(path.clone(), Stroke::new(3.0, Color32::from_black_alpha(89))));
        painter.add(Shape::line(path, Stroke::new(1.5, c.accent)));
    }
    for (pt, is_key, is_current) in dots {
        let r = if is_key { 4.5 } else { 2.5 };
        let fill = if is_current {
            Color32::WHITE
        } else if is_key {
            c.accent
        } else {
            c.accent.gamma_multiply(0.7)
        };
        let stroke = if is_key || is_current { Stroke::new(1.0, Color32::BLACK) } else { Stroke::NONE };
        painter.circle(pt, r, fill, stroke);
    }
    if st.edit_on_canvas
        && let (Some(t), Some(b)) = (current(tab), tab.doc.tween_source_bounds(tab.layer))
    {
        paint_handles(painter, &geometry(&t, &b, z), &screen);
    }
}

/// The same handle look as the free transform: white and black dashed outline, square corner handles with a
/// black edge, an orange pivot, and a white dot on a stem for rotation.
fn paint_handles(painter: &egui::Painter, g: &Geometry, screen: &dyn Fn(f64, f64) -> Pos2) {
    let pts: Vec<Pos2> = g.corners.iter().map(|p| screen(p.0, p.1)).collect();
    let top = pos2((pts[0].x + pts[1].x) / 2.0, (pts[0].y + pts[1].y) / 2.0);
    let rot = screen(g.rotate.0, g.rotate.1);
    let mut closed = pts.clone();
    closed.push(pts[0]);
    let stem = [top, rot];
    painter.add(Shape::line(closed.clone(), Stroke::new(1.0, Color32::WHITE)));
    painter.add(Shape::line_segment(stem, Stroke::new(1.0, Color32::WHITE)));
    painter.extend(Shape::dashed_line(&closed, Stroke::new(1.0, Color32::BLACK), 4.0, 4.0));
    painter.extend(Shape::dashed_line(&stem, Stroke::new(1.0, Color32::BLACK), 4.0, 4.0));
    for p in &pts {
        let r = Rect::from_center_size(*p, egui::vec2(8.0, 8.0));
        painter.rect(r, 0.0, Color32::WHITE, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Middle);
    }
    painter.circle(rot, 4.0, Color32::WHITE, Stroke::new(1.0, Color32::BLACK));
    let pv = screen(g.pivot.0, g.pivot.1);
    painter.circle(pv, 4.0, super::form::ORANGE, Stroke::new(1.0, Color32::BLACK));
}

impl TweenState {
    /// A drag is in progress (`tweenDragging`).
    pub fn dragging(&self) -> bool {
        self.canvas_drag.is_some()
    }
}

/// Handles the mouse for "Edit on canvas": starts a handle drag on press, follows it, and ends it on release.
/// Returns whether the tween took the mouse, so the tools must ignore it.
pub fn input(
    st: &mut TweenState,
    tab: &mut DocTab,
    response: &egui::Response,
    srect: Rect,
    ctx: &egui::Context,
) -> bool {
    let (pos, pressed, down, shift, space) = ctx.input(|i| {
        (
            i.pointer.latest_pos(),
            i.pointer.button_pressed(egui::PointerButton::Primary),
            i.pointer.button_down(egui::PointerButton::Primary),
            i.modifiers.shift,
            i.key_down(egui::Key::Space),
        )
    });
    let to_sprite =
        |p: Pos2| ((p.x - srect.left()) as f64 / tab.zoom as f64, (p.y - srect.top()) as f64 / tab.zoom as f64);
    if st.canvas_drag.is_none() && st.edit_on_canvas && pressed && !space && response.hovered() {
        if let Some(pos) = pos {
            begin(st, tab, to_sprite(pos));
        }
        return st.canvas_drag.is_some();
    }
    let Some(mut drag) = st.canvas_drag else { return false };
    if drag.doc != tab.doc.raw() as usize {
        st.canvas_drag = None;
        return false;
    }
    if !down {
        end(st, tab);
        return true;
    }
    if let Some(pos) = pos {
        update(st, tab, &mut drag, to_sprite(pos), shift);
        st.canvas_drag = Some(drag);
    }
    true
}

/// `tweenMouseDown`: picks the handle under `p` and opens a transaction.
pub fn begin(st: &mut TweenState, tab: &mut DocTab, p: (f64, f64)) {
    let (Some(t), Some(b)) = (current(tab), tab.doc.tween_source_bounds(tab.layer)) else { return };
    let g = geometry(&t, &b, tab.zoom);
    let tol = 8.0 / tab.zoom as f64;
    let near = |q: (f64, f64)| dist(q, p) <= tol;
    let mut keys_at_frame = 0;
    for prop in TweenProp::ALL {
        if tab.doc.tween_keys(tab.layer, prop).iter().any(|k| k.frame == tab.frame) {
            keys_at_frame |= 1 << prop.code();
        }
    }
    let mut drag = CanvasDrag {
        kind: Kind::Move,
        start: p,
        t,
        start_angle: 0.0,
        start_dist: 1.0,
        keys_at_frame,
        doc: tab.doc.raw() as usize,
        changed: false,
    };
    if near(g.pivot) {
        drag.kind = Kind::Pivot;
    } else if near(g.rotate) {
        drag.kind = Kind::Rotate;
        drag.start_angle = (p.1 - g.pivot.1).atan2(p.0 - g.pivot.0);
    } else if let Some(corner) = g.corners.iter().find(|q| near(**q)) {
        drag.kind = Kind::Scale;
        drag.start_dist = dist(*corner, g.pivot).max(0.001);
        drag.start = *corner;
    }
    tab.playing = false;
    tab.doc.begin_transaction();
    st.canvas_drag = Some(drag);
}

/// `tweenMouseDragged`: writes the keys (or the pivot) for the pointer at `p`.
fn update(st: &TweenState, tab: &mut DocTab, d: &mut CanvasDrag, p: (f64, f64), shift: bool) {
    let mut values = [d.t[0], d.t[1], d.t[2], d.t[3], d.t[4], d.t[5]];
    let mut mask;
    let pivot_dest = (d.t[6] as f64 + d.t[0] as f64, d.t[7] as f64 + d.t[1] as f64);
    match d.kind {
        Kind::Move => {
            values[0] = d.t[0] + (p.0 - d.start.0) as f32;
            values[1] = d.t[1] + (p.1 - d.start.1) as f32;
            mask = 0b11;
        }
        Kind::Rotate => {
            let delta = ((p.1 - pivot_dest.1).atan2(p.0 - pivot_dest.0) - d.start_angle).to_degrees();
            let mut angle = d.t[2] as f64 + delta;
            if shift {
                angle = (angle / 15.0).round() * 15.0;
            }
            values[2] = angle as f32;
            mask = 0b100;
        }
        Kind::Scale => {
            let f = (dist(p, pivot_dest) / d.start_dist) as f32;
            values[3] = d.t[3] * f;
            values[4] = d.t[4] * f;
            mask = 0b11000;
        }
        Kind::Pivot => {
            let x = (((p.0 - d.t[0] as f64) * 2.0).round() / 2.0) as f32;
            let y = (((p.1 - d.t[1] as f64) * 2.0).round() / 2.0) as f32;
            if tab.doc.tween_set_pivot_quiet(tab.layer, x, y) {
                d.changed = true;
                tab.changed();
            }
            return;
        }
    }
    if !st.auto_key {
        mask &= d.keys_at_frame;
    }
    if mask != 0 && tab.doc.tween_set_keys_at(tab.layer, tab.frame, mask, values) {
        d.changed = true;
        tab.changed();
    }
}

/// `tweenMouseUp`: closes the transaction as one undo step.
pub fn end(st: &mut TweenState, tab: &mut DocTab) {
    let Some(d) = st.canvas_drag.take() else { return };
    tab.doc.end_transaction();
    if d.changed {
        tab.doc.label_last_step("Tween Canvas Edit");
        tab.doc.touch();
    }
    tab.changed();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Doc;
    use crate::ui::gamedev::TweenEase;

    fn tab_with_box() -> DocTab {
        let mut doc = Doc::new(64, 64);
        doc.begin_edit("Shape");
        doc.draw_shape(crate::ui::canvas::ShapeKind::Rect, 0, 0, (10, 10), (29, 29), 0xFF8800FF, 1, 8, false);
        doc.stroke_end();
        for _ in 0..3 {
            let last = doc.frame_count() - 1;
            doc.add_frame(last, true);
        }
        doc.touch();
        let mut tab = DocTab::new(doc);
        tab.layer = 0;
        tab.zoom = 4.0;
        tab
    }

    fn key(tab: &mut DocTab, prop: TweenProp, frame: u32, value: f32) {
        assert!(tab.doc.tween_set_key(tab.layer, prop, frame, value, TweenEase::Linear, [0.0; 4]));
    }

    #[test]
    fn identity_geometry_is_the_source_box() {
        let t = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 20.0, 20.0];
        let g = geometry(&t, &[10.0, 10.0, 30.0, 30.0], 4.0);
        assert_eq!(g.corners[0], (10.0, 10.0));
        assert_eq!(g.corners[2], (30.0, 30.0));
        assert_eq!(g.pivot, (20.0, 20.0));
        // The rotate handle sits 14 screen points above the top edge.
        assert!((g.rotate.0 - 20.0).abs() < 1e-9 && (g.rotate.1 - (10.0 - 14.0 / 4.0)).abs() < 1e-9);
    }

    #[test]
    fn a_quarter_turn_moves_the_corners() {
        let t = [5.0, 0.0, 90.0, 1.0, 1.0, 1.0, 20.0, 20.0];
        let g = geometry(&t, &[10.0, 10.0, 30.0, 30.0], 1.0);
        assert_eq!(g.pivot, (25.0, 20.0));
        let tl = g.corners[0];
        assert!((tl.0 - 35.0).abs() < 1e-6 && (tl.1 - 10.0).abs() < 1e-6, "{tl:?}");
    }

    #[test]
    fn dragging_the_box_creates_keys_and_is_one_undo_step() {
        let mut tab = tab_with_box();
        let mut st = TweenState::default();
        st.edit_on_canvas = true;
        tab.frame = 1;
        let before = tab.doc.history_counts().0;
        // Press inside the box (away from the handles), drag by (6, -3) pixels, release.
        begin(&mut st, &mut tab, (14.0, 25.0));
        let mut d = st.canvas_drag.expect("drag started");
        assert_eq!(d.kind, Kind::Move);
        update(&st, &mut tab, &mut d, (20.0, 22.0), false);
        st.canvas_drag = Some(d);
        end(&mut st, &mut tab);
        assert!(!st.dragging());
        assert_eq!(tab.doc.tween_value(0, TweenProp::PosX, 1), 6.0);
        assert_eq!(tab.doc.tween_value(0, TweenProp::PosY, 1), -3.0);
        assert_eq!(tab.doc.history_counts().0, before + 1, "one undo step");
        assert!(tab.doc.undo());
        assert!(tab.doc.tween_keys(0, TweenProp::PosX).is_empty());
    }

    #[test]
    fn without_auto_key_only_existing_keys_move() {
        let mut tab = tab_with_box();
        let mut st = TweenState::default();
        st.edit_on_canvas = true;
        st.auto_key = false;
        tab.frame = 0;
        key(&mut tab, TweenProp::PosX, 0, 0.0);
        key(&mut tab, TweenProp::PosX, 3, 9.0);
        begin(&mut st, &mut tab, (14.0, 25.0));
        let mut d = st.canvas_drag.unwrap();
        update(&st, &mut tab, &mut d, (16.0, 33.0), false);
        end(&mut st, &mut tab);
        assert_eq!(tab.doc.tween_keys(0, TweenProp::PosX)[0].value, 2.0, "X had a key here");
        assert!(tab.doc.tween_keys(0, TweenProp::PosY).is_empty(), "Y had none, so none was made");
    }

    #[test]
    fn corner_scales_and_top_handle_rotates_and_pivot_moves() {
        let mut tab = tab_with_box();
        let mut st = TweenState::default();
        st.edit_on_canvas = true;
        key(&mut tab, TweenProp::PosX, 0, 0.0);
        key(&mut tab, TweenProp::PosX, 3, 0.0);
        let t = current(&tab).unwrap();
        let b = tab.doc.tween_source_bounds(0).unwrap();
        let g = geometry(&t, &b, tab.zoom);

        // Scale: grab the bottom-right corner and pull it twice as far from the pivot.
        begin(&mut st, &mut tab, g.corners[2]);
        let mut d = st.canvas_drag.unwrap();
        assert_eq!(d.kind, Kind::Scale);
        let far = (g.pivot.0 + (g.corners[2].0 - g.pivot.0) * 2.0, g.pivot.1 + (g.corners[2].1 - g.pivot.1) * 2.0);
        update(&st, &mut tab, &mut d, far, false);
        end(&mut st, &mut tab);
        assert!((tab.doc.tween_value(0, TweenProp::ScaleX, 0) - 2.0).abs() < 1e-4);

        // Rotate: the handle above the top edge, dragged to the right of the pivot, with Shift snapping to 15 degrees.
        let t = current(&tab).unwrap();
        let g = geometry(&t, &b, tab.zoom);
        begin(&mut st, &mut tab, g.rotate);
        let mut d = st.canvas_drag.unwrap();
        assert_eq!(d.kind, Kind::Rotate);
        update(&st, &mut tab, &mut d, (g.pivot.0 + 30.0, g.pivot.1 + 2.0), true);
        end(&mut st, &mut tab);
        let rot = tab.doc.tween_value(0, TweenProp::Rotation, 0);
        assert!((rot / 15.0).fract().abs() < 1e-4 && rot > 60.0, "snapped: {rot}");

        // Pivot: the orange dot, snapped to half pixels.
        let t = current(&tab).unwrap();
        let g = geometry(&t, &b, tab.zoom);
        begin(&mut st, &mut tab, g.pivot);
        let mut d = st.canvas_drag.unwrap();
        assert_eq!(d.kind, Kind::Pivot);
        update(&st, &mut tab, &mut d, (22.3, 19.8), false);
        end(&mut st, &mut tab);
        assert_eq!(tab.doc.tween_pivot(0), Some((22.5 - t[0], 20.0 - t[1])));
    }

    #[test]
    fn a_click_that_changes_nothing_adds_no_step() {
        let mut tab = tab_with_box();
        let mut st = TweenState::default();
        st.edit_on_canvas = true;
        st.auto_key = false;
        let before = tab.doc.history_counts();
        begin(&mut st, &mut tab, (14.0, 25.0));
        end(&mut st, &mut tab);
        assert_eq!(tab.doc.history_counts(), before);
    }
}
