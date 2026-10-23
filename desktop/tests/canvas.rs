//! Drawing through the `Doc` methods the canvas tools use, checked against the composited pixels.

use pixmith_desktop::doc::Doc;
use pixmith_desktop::ui::canvas::{ShapeKind, flag};

const RED: u32 = 0xFF0000FF;

fn pixel(doc: &Doc, x: i32, y: i32) -> u32 {
    let mut buf = Vec::new();
    assert!(doc.render(0, &mut buf));
    Doc::pixel_of(&buf, doc.width(), x, y).expect("inside the sprite")
}

#[test]
fn pencil_draws_and_undoes() {
    let mut doc = Doc::new(8, 8);
    let layer = doc.layer_count() - 1;
    assert_eq!(pixel(&doc, 2, 3) & 0xFF, 0, "starts transparent");

    doc.begin_edit("Draw");
    doc.set_ink(0, false, &[]);
    doc.stroke_begin(layer, 0, 2, 3, RED, 1, 0);
    doc.stroke_to(5, 3);
    doc.stroke_end();
    doc.touch();

    for x in 2..=5 {
        assert_eq!(pixel(&doc, x, 3), RED, "pixel {x},3");
    }
    assert_eq!(pixel(&doc, 6, 3) & 0xFF, 0);
    assert_eq!(pixel(&doc, 3, 4) & 0xFF, 0);
    assert!(doc.is_dirty());

    assert!(doc.undo());
    assert_eq!(pixel(&doc, 3, 3) & 0xFF, 0, "undo removes the stroke");
    assert!(doc.redo());
    assert_eq!(pixel(&doc, 3, 3), RED, "redo brings it back");
}

#[test]
fn eraser_clears_pixels() {
    let mut doc = Doc::new(8, 8);
    let layer = doc.layer_count() - 1;
    doc.begin_edit("Draw");
    doc.stroke_begin(layer, 0, 1, 1, RED, 3, 0);
    doc.stroke_end();
    assert_eq!(pixel(&doc, 1, 1), RED);
    doc.begin_edit("Erase");
    doc.stroke_begin(layer, 0, 1, 1, 0, 1, 0);
    doc.stroke_end();
    assert_eq!(pixel(&doc, 1, 1) & 0xFF, 0);
    assert_eq!(pixel(&doc, 2, 1), RED, "only the brush footprint is erased");
}

#[test]
fn shape_preview_replaces_itself() {
    let mut doc = Doc::new(16, 16);
    let layer = doc.layer_count() - 1;
    doc.begin_edit("Rectangle");
    doc.draw_shape(ShapeKind::Rect, layer, 0, (2, 2), (4, 4), RED, 1, flag::FILLED, false);
    assert_eq!(pixel(&doc, 3, 3), RED);
    // Dragging on: the first preview is rewound, so only the new rectangle remains.
    doc.draw_shape(ShapeKind::Rect, layer, 0, (2, 2), (9, 9), RED, 1, flag::FILLED, true);
    doc.stroke_end();
    assert_eq!(pixel(&doc, 8, 8), RED);
    assert!(doc.undo());
    assert_eq!(pixel(&doc, 3, 3) & 0xFF, 0, "one undo step removes the whole shape");
}

#[test]
fn bucket_fills_the_connected_area() {
    let mut doc = Doc::new(8, 8);
    let layer = doc.layer_count() - 1;
    doc.begin_edit("Fill");
    doc.fill_at(layer, 0, 0, 0, RED, 0xFFFFFFFF, true, 0);
    assert_eq!(pixel(&doc, 7, 7), RED);
}

/// Drives the real UI headlessly: a pencil drag, the right button, shape previews, the eyedropper, keys.
mod ui {
    use super::*;
    use eframe::egui;
    use egui_kittest::Harness;
    use pixmith_desktop::PixmithApp;
    use pixmith_desktop::app::Tool;
    use pixmith_desktop::theme::{self, metric};
    use pixmith_desktop::ui::canvas::sprite_rect;

    const SIZE: [f32; 2] = [1280.0, 800.0];

    fn harness() -> Harness<'static, PixmithApp> {
        let app = PixmithApp { primary: RED, ..Default::default() };
        let mut installed = false;
        let mut h = Harness::builder().with_size(SIZE).build_ui_state(
            move |ui, app: &mut PixmithApp| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                app.ui(ui);
            },
            app,
        );
        h.run();
        h
    }

    /// Screen position of the center of sprite pixel `(x, y)`.
    fn at(h: &Harness<PixmithApp>, x: i32, y: i32) -> egui::Pos2 {
        let app = h.state();
        let timeline = pixmith_desktop::ui::timeline::panel_height(app);
        let area = egui::Rect::from_min_max(
            egui::pos2(metric::TOOL_RAIL, 2.0 * metric::BAR_HEIGHT),
            egui::pos2(SIZE[0] - metric::INSPECTOR_WIDTH, SIZE[1] - timeline),
        );
        sprite_rect(area, app.tab(), 1.0).min + egui::vec2(x as f32 + 0.5, y as f32 + 0.5) * app.tab().zoom
    }

    fn press(h: &mut Harness<PixmithApp>, pos: egui::Pos2, button: egui::PointerButton, pressed: bool) {
        h.event(egui::Event::PointerButton { pos, button, pressed, modifiers: Default::default() });
        h.run();
    }

    fn hover(h: &mut Harness<PixmithApp>, pos: egui::Pos2) {
        h.event(egui::Event::PointerMoved(pos));
        h.run();
    }

    fn at_pixel(h: &Harness<PixmithApp>, x: i32, y: i32) -> u32 {
        pixel(&h.state().tab().doc, x, y)
    }

    #[test]
    fn pencil_drag_draws_a_line_and_undo_removes_it() {
        let mut h = harness();
        let (a, b) = (at(&h, 4, 6), at(&h, 9, 6));
        hover(&mut h, a);
        press(&mut h, a, egui::PointerButton::Primary, true);
        hover(&mut h, b);
        press(&mut h, b, egui::PointerButton::Primary, false);
        for x in 4..=9 {
            assert_eq!(at_pixel(&h, x, 6), RED, "pixel {x},6");
        }
        assert!(h.state().tab().revision > 0);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run();
        assert_eq!(at_pixel(&h, 6, 6) & 0xFF, 0);
    }

    #[test]
    fn right_button_uses_the_secondary_color_and_eraser_clears() {
        let mut h = harness();
        h.state_mut().secondary = 0x00FF00FF;
        let p = at(&h, 3, 3);
        hover(&mut h, p);
        press(&mut h, p, egui::PointerButton::Secondary, true);
        press(&mut h, p, egui::PointerButton::Secondary, false);
        assert_eq!(at_pixel(&h, 3, 3), 0x00FF00FF);
        h.state_mut().tool = Tool::Eraser;
        press(&mut h, p, egui::PointerButton::Primary, true);
        press(&mut h, p, egui::PointerButton::Primary, false);
        assert_eq!(at_pixel(&h, 3, 3) & 0xFF, 0);
    }

    #[test]
    fn rectangle_previews_while_dragging_and_eyedropper_picks() {
        let mut h = harness();
        h.state_mut().tool = Tool::Rectangle;
        h.state_mut().canvas.opts.filled = true;
        let a = at(&h, 2, 2);
        hover(&mut h, a);
        press(&mut h, a, egui::PointerButton::Primary, true);
        let far = at(&h, 12, 12);
        hover(&mut h, far);
        let near = at(&h, 6, 6);
        hover(&mut h, near);
        assert_eq!(at_pixel(&h, 5, 5), RED);
        assert_eq!(at_pixel(&h, 10, 10) & 0xFF, 0, "the larger preview was replaced");
        press(&mut h, near, egui::PointerButton::Primary, false);

        h.state_mut().primary = 0x112233FF;
        h.state_mut().tool = Tool::Picker;
        let p = at(&h, 4, 4);
        hover(&mut h, p);
        press(&mut h, p, egui::PointerButton::Primary, true);
        press(&mut h, p, egui::PointerButton::Primary, false);
        assert_eq!(h.state().primary, RED);
    }

    #[test]
    fn bucket_fills_and_keys_switch_tools() {
        let mut h = harness();
        h.key_press(egui::Key::G);
        h.run();
        assert_eq!(h.state().tool, Tool::Fill);
        let p = at(&h, 10, 10);
        hover(&mut h, p);
        press(&mut h, p, egui::PointerButton::Primary, true);
        press(&mut h, p, egui::PointerButton::Primary, false);
        assert_eq!(at_pixel(&h, 0, 0), RED);
        h.key_press(egui::Key::CloseBracket);
        h.run();
        assert_eq!(h.state().brush_size, 2);
    }
}
