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

    /// Clicks sprite pixel `(x, y)` with `modifiers` held.
    fn click_with(h: &mut Harness<PixmithApp>, (x, y): (i32, i32), modifiers: egui::Modifiers) {
        let pos = at(h, x, y);
        h.event(egui::Event::ModifiersChanged(modifiers));
        hover(h, pos);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers });
            h.run();
        }
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.run();
    }

    #[test]
    fn shift_click_draws_a_line_from_the_last_stroke_and_cmd_snaps_it_to_2_to_1() {
        let mut h = harness();
        click_with(&mut h, (2, 10), egui::Modifiers::NONE);
        click_with(&mut h, (12, 10), egui::Modifiers::SHIFT);
        for x in 2..=12 {
            assert_eq!(at_pixel(&h, x, 10), RED, "pixel {x},10");
        }
        // From (12, 10) toward (21, 15): snapped to the 2:1 line ending at (21, 14), in steps of two pixels.
        click_with(&mut h, (21, 15), egui::Modifiers::SHIFT | egui::Modifiers::COMMAND);
        for (x, y) in [(13, 10), (14, 11), (15, 11), (16, 12), (17, 12), (20, 14), (21, 14)] {
            assert_eq!(at_pixel(&h, x, y), RED, "pixel {x},{y}");
        }
        assert_eq!(at_pixel(&h, 21, 15) & 0xFF, 0);
    }

    #[test]
    fn mirroring_uses_the_moved_axis() {
        let mut h = harness();
        h.state_mut().canvas.opts.mirror_x = true;
        h.state_mut().tab_mut().mirror_axes = (Some(10), None);
        click_with(&mut h, (1, 1), egui::Modifiers::NONE);
        assert_eq!(at_pixel(&h, 1, 1), RED);
        assert_eq!(at_pixel(&h, 8, 1), RED, "mirrored across x = 5");
        assert_eq!(at_pixel(&h, 30, 1) & 0xFF, 0, "not across the canvas center");
    }

    #[test]
    fn space_tapped_plays_and_space_held_pans_without_playing() {
        let mut h = harness();
        let key = |h: &mut Harness<PixmithApp>, pressed: bool| {
            h.event(egui::Event::Key {
                key: egui::Key::Space,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            // Playback keeps repainting, so step a few frames instead of running until idle.
            h.run_steps(4);
        };
        h.state_mut().tab_mut().doc.add_frame(0, false);
        key(&mut h, true);
        key(&mut h, false);
        assert!(h.state().tab().playing, "a tap plays");
        key(&mut h, true);
        key(&mut h, false);
        assert!(!h.state().tab().playing, "a second tap pauses");

        // Larger than the view, so the pan is not held centered.
        h.state_mut().tab_mut().zoom = 48.0;
        h.run_steps(4);
        let pan = h.state().tab().pan;
        let a = at(&h, 16, 16);
        key(&mut h, true);
        hover(&mut h, a);
        press(&mut h, a, egui::PointerButton::Primary, true);
        hover(&mut h, a + egui::vec2(40.0, 25.0));
        press(&mut h, a + egui::vec2(40.0, 25.0), egui::PointerButton::Primary, false);
        key(&mut h, false);
        assert_ne!(h.state().tab().pan, pan, "Space-drag pans");
        assert!(!h.state().tab().playing, "releasing Space after a pan does not play");
        assert_eq!(at_pixel(&h, 16, 16) & 0xFF, 0, "and does not draw");
    }
}
