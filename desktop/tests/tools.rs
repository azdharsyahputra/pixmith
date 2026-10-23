//! The tools of `ui::tools` and the view features: through the `Doc` calls they use, and through the real UI
//! driven headlessly.

use pixmith_desktop::doc::Doc;

const RED: u32 = 0xFF0000FF;
const BLUE: u32 = 0x0000FFFF;

fn pixel(doc: &Doc, frame: u32, x: i32, y: i32) -> u32 {
    let mut buf = Vec::new();
    assert!(doc.render(frame, &mut buf));
    Doc::pixel_of(&buf, doc.width(), x, y).expect("inside the sprite")
}

fn count_opaque(doc: &Doc) -> usize {
    let mut buf = Vec::new();
    assert!(doc.render(0, &mut buf));
    buf.as_chunks::<4>().0.iter().filter(|p| p[3] != 0).count()
}

fn top(doc: &Doc) -> u32 {
    doc.layer_count() - 1
}

#[test]
fn spray_paints_a_dotted_disc_and_density_scales_it() {
    let mut sparse = Doc::new(40, 40);
    let mut dense = Doc::new(40, 40);
    for (doc, density) in [(&mut sparse, 10), (&mut dense, 100)] {
        let layer = top(doc);
        doc.begin_edit("Spray");
        doc.set_ink(0, false, &[]);
        doc.spray_at(layer, 0, (20, 20), RED, 9, density, 7, 0);
        doc.stroke_end();
    }
    let (a, b) = (count_opaque(&sparse), count_opaque(&dense));
    assert!(a > 0 && b > a, "density 10 gave {a} pixels, density 100 gave {b}");
    // Nothing lands outside the radius.
    assert_eq!(pixel(&dense, 0, 0, 0) & 0xFF, 0);
}

#[test]
fn linear_gradient_runs_from_the_first_to_the_second_color() {
    let mut doc = Doc::new(16, 4);
    let layer = top(&doc);
    doc.begin_edit("Gradient");
    doc.gradient_draw(layer, 0, (0, 0), (15, 0), RED, BLUE, false, 0, false);
    doc.stroke_end();
    let left = pixel(&doc, 0, 0, 1);
    let right = pixel(&doc, 0, 15, 1);
    assert_eq!(left, RED);
    assert_eq!(right, BLUE);
    let mid = pixel(&doc, 0, 8, 1);
    assert!(mid != RED && mid != BLUE, "smooth gradient blends in between: {mid:08x}");
    // Dragging on rewinds to the checkpoint: the later preview replaces the earlier one.
    doc.gradient_draw(layer, 0, (0, 0), (0, 3), BLUE, RED, false, 0, true);
    assert_eq!(pixel(&doc, 0, 0, 0), BLUE);
    assert!(doc.undo());
    assert_eq!(pixel(&doc, 0, 3, 1) & 0xFF, 0, "the whole gradient is one undo step");
}

#[test]
fn dithered_gradient_uses_only_the_two_colors() {
    let mut doc = Doc::new(16, 16);
    let layer = top(&doc);
    doc.begin_edit("Gradient");
    doc.gradient_draw(layer, 0, (0, 0), (15, 15), RED, BLUE, true, 4, false);
    doc.stroke_end();
    for y in 0..16 {
        for x in 0..16 {
            let p = pixel(&doc, 0, x, y);
            assert!(p == RED || p == BLUE, "pixel {x},{y} is {p:08x}");
        }
    }
}

#[test]
fn curve_reaches_both_end_points_and_bends_toward_the_controls() {
    let mut doc = Doc::new(32, 32);
    let layer = top(&doc);
    doc.begin_edit("Curve");
    doc.curve_draw(layer, 0, &[(2, 28), (2, 2), (29, 2), (29, 28)], RED, 1, 0);
    assert_eq!(pixel(&doc, 0, 2, 28), RED);
    assert_eq!(pixel(&doc, 0, 29, 28), RED);
    // The middle of this curve is near y = 28 - 0.75 * 26 = 8.5, far from the straight line at y = 28.
    let mid_top = (0..32).any(|y| pixel(&doc, 0, 15, y) == RED && y < 15);
    assert!(mid_top, "curve bows upward");
    assert_eq!(pixel(&doc, 0, 15, 28) & 0xFF, 0);
    // A second call rewinds first, so the old curve is gone.
    doc.curve_draw(layer, 0, &[(0, 0), (0, 0), (5, 5), (5, 5)], BLUE, 1, 0);
    assert_eq!(pixel(&doc, 0, 29, 28) & 0xFF, 0);
}

#[test]
fn polygon_fills_or_leaves_an_open_path() {
    let mut doc = Doc::new(32, 32);
    let layer = top(&doc);
    let tri = [(4, 4), (27, 4), (4, 27)];
    doc.begin_edit("Polygon");
    doc.polygon_draw(layer, 0, &tri, RED, 1, 8);
    assert_eq!(pixel(&doc, 0, 8, 8), RED, "filled interior");
    doc.polygon_draw(layer, 0, &tri, RED, 1, 32);
    assert_eq!(pixel(&doc, 0, 8, 8) & 0xFF, 0, "open path is not filled");
    assert_eq!(pixel(&doc, 0, 4, 4), RED, "but the path is drawn");
    doc.polygon_draw(layer, 0, &tri, RED, 1, 0);
    assert_eq!(pixel(&doc, 0, 8, 8) & 0xFF, 0, "closed outline only");
    assert_eq!(pixel(&doc, 0, 15, 16), RED, "the closing edge is drawn");
}

#[test]
fn built_in_text_draws_and_the_preview_is_replaced() {
    let mut doc = Doc::new(64, 32);
    let layer = top(&doc);
    let (w1, h1) = Doc::text_size_of("Hi", 1);
    let (w2, _) = Doc::text_size_of("Hi", 2);
    assert!(w1 > 0 && h1 > 0 && w2 == 2 * w1, "size {w1}x{h1}, doubled {w2}");
    doc.begin_edit("Text");
    doc.text_draw(layer, 0, (2, 2), "Hi", 1, RED, 0, false);
    let first = count_opaque(&doc);
    assert!(first > 0);
    doc.text_draw(layer, 0, (2, 2), "Hi Hi", 1, RED, 0, true);
    assert!(count_opaque(&doc) > first, "longer text, more pixels");
    doc.text_draw(layer, 0, (2, 2), "", 1, RED, 0, true);
    assert_eq!(count_opaque(&doc), 0, "empty text with restore leaves nothing");
    assert!(doc.rollback());
}

#[test]
fn rasterized_mask_is_stamped_in_the_color() {
    let mut doc = Doc::new(8, 8);
    let layer = top(&doc);
    doc.begin_edit("Text");
    let mask = [255, 0, 0, 255];
    doc.stamp_mask_draw(layer, 0, (3, 3), (2, 2), &mask, RED, 0, false);
    assert_eq!(pixel(&doc, 0, 3, 3), RED);
    assert_eq!(pixel(&doc, 0, 4, 3) & 0xFF, 0);
    assert_eq!(pixel(&doc, 0, 4, 4), RED);
    // A mask shorter than its size is ignored instead of read out of bounds.
    doc.stamp_mask_draw(layer, 0, (0, 0), (8, 8), &mask, RED, 0, false);
    assert_eq!(pixel(&doc, 0, 0, 0) & 0xFF, 0);
}

#[test]
fn system_font_rasterizes_without_antialiasing() {
    use pixmith_desktop::ui::tools::text::{font_families, rasterize};
    assert_eq!(font_families()[0], "System");
    // The bundled monospaced font is always available.
    let defs = eframe::egui::FontDefinitions::default();
    let key = defs.families[&eframe::egui::FontFamily::Monospace][0].clone();
    let bytes = defs.font_data[&key].font.to_vec();
    let (w, h, mask) = rasterize("Hi\nHi", &bytes, 16.0).expect("rasterizes");
    assert!(w > 8 && h >= 32, "two lines of 16 px text: {w}x{h}");
    assert!(mask.iter().all(|v| *v == 0 || *v == 255), "only 0 and 255, no antialiasing");
    assert!(mask.contains(&255));
    assert!(rasterize("", &bytes, 16.0).is_none(), "empty text has no mask");
}

#[test]
fn ink_modes_lock_alpha_and_behind() {
    let mut doc = Doc::new(8, 8);
    let layer = top(&doc);
    // A red pixel, then draw blue over it and next to it with Lock Alpha: only the red pixel changes.
    doc.begin_edit("Draw");
    doc.stroke_begin(layer, 0, 3, 3, RED, 1, 0);
    doc.stroke_end();
    doc.begin_edit("Draw");
    doc.set_ink(2, false, &[]);
    doc.stroke_begin(layer, 0, 3, 3, BLUE, 1, 0);
    doc.stroke_to(5, 3);
    doc.stroke_end();
    assert_eq!(pixel(&doc, 0, 3, 3), BLUE);
    assert_eq!(pixel(&doc, 0, 4, 3) & 0xFF, 0, "lock alpha does not paint transparent pixels");
    // Behind: new color only shows where the layer is transparent.
    doc.begin_edit("Draw");
    doc.set_ink(3, false, &[]);
    doc.stroke_begin(layer, 0, 3, 3, RED, 1, 0);
    doc.stroke_to(5, 3);
    doc.stroke_end();
    assert_eq!(pixel(&doc, 0, 3, 3), BLUE, "behind keeps what is already there");
    assert_eq!(pixel(&doc, 0, 5, 3), RED);
    doc.set_ink(0, false, &[]);
}

#[test]
fn custom_brush_comes_from_the_selection() {
    let mut doc = Doc::new(16, 16);
    let layer = top(&doc);
    assert!(!doc.brush_from_selection(layer, 0), "no selection, no brush");
    doc.begin_edit("Draw");
    doc.stroke_begin(layer, 0, 2, 2, RED, 3, 0);
    doc.stroke_end();
    // SAFETY: the document is live.
    unsafe { pixmith_core::paint::selection::pixmith_sel_rect(doc.raw(), 1, 1, 3, 3) };
    assert!(doc.brush_from_selection(layer, 0));
    let (w, h) = Doc::brush_info().expect("custom brush");
    assert_eq!((w, h), (3, 3));
    // Stamping with the brush paints its shape (the selection would clip it, so drop that first).
    // SAFETY: the document is live.
    unsafe { pixmith_core::paint::selection::pixmith_sel_clear(doc.raw()) };
    doc.begin_edit("Draw");
    doc.stroke_begin(layer, 0, 10, 10, RED, 1, 0);
    doc.stroke_end();
    assert_eq!(pixel(&doc, 0, 10, 10), RED);
    assert_eq!(pixel(&doc, 0, 11, 11), RED, "a 3x3 brush");
    assert_eq!(pixel(&doc, 0, 12, 12) & 0xFF, 0);
    Doc::brush_reset();
    assert!(Doc::brush_info().is_none());
}

#[test]
fn fill_pattern_alternates_the_two_colors() {
    let mut doc = Doc::new(8, 8);
    let layer = top(&doc);
    doc.begin_edit("Fill");
    doc.fill_pattern_at(layer, 0, (0, 0), RED, BLUE, 1, true, 0);
    // The checker pattern works in 2x2 cells.
    let (a, b) = (pixel(&doc, 0, 0, 0), pixel(&doc, 0, 2, 0));
    assert!(a != b && [a, b].iter().all(|p| *p == RED || *p == BLUE), "checker: {a:08x} {b:08x}");
    assert_eq!(pixel(&doc, 0, 1, 1), a, "cells are 2x2");
}

/// The real UI, driven headlessly.
mod ui {
    use super::*;
    use eframe::egui;
    use egui_kittest::Harness;
    use pixmith_desktop::PixmithApp;
    use pixmith_desktop::app::Tool;
    use pixmith_desktop::theme::{self, metric};
    use pixmith_desktop::ui::canvas::sprite_rect;
    use pixmith_desktop::ui::tools;
    use pixmith_desktop::ui::tools::view::TiledMode;

    const SIZE: [f32; 2] = [1280.0, 800.0];

    fn harness() -> Harness<'static, PixmithApp> {
        let app = PixmithApp { primary: RED, secondary: BLUE, ..Default::default() };
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
        h.run_ok();
        h
    }

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
        h.run_ok();
    }

    fn hover(h: &mut Harness<PixmithApp>, pos: egui::Pos2) {
        h.event(egui::Event::PointerMoved(pos));
        h.run_ok();
    }

    fn px(h: &Harness<PixmithApp>, x: i32, y: i32) -> u32 {
        pixel(&h.state().tab().doc, 0, x, y)
    }

    fn click(h: &mut Harness<PixmithApp>, x: i32, y: i32) {
        let p = at(h, x, y);
        hover(h, p);
        press(h, p, egui::PointerButton::Primary, true);
        press(h, p, egui::PointerButton::Primary, false);
    }

    #[test]
    fn spray_drag_paints_and_undoes_as_one_step() {
        let mut h = harness();
        h.state_mut().tool = Tool::Spray;
        h.state_mut().canvas.opts.spray_density = 100.0;
        h.state_mut().brush_size = 2;
        let a = at(&h, 10, 10);
        hover(&mut h, a);
        press(&mut h, a, egui::PointerButton::Primary, true);
        let b = at(&h, 20, 10);
        hover(&mut h, b);
        press(&mut h, b, egui::PointerButton::Primary, false);
        assert!(count_opaque(&h.state().tab().doc) > 0, "spray painted something");
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_ok();
        assert_eq!(px(&h, 10, 10) & 0xFF, 0);
        assert_eq!(px(&h, 20, 10) & 0xFF, 0, "the whole drag is one undo step");
    }

    #[test]
    fn gradient_drag_previews_and_a_plain_click_leaves_nothing() {
        let mut h = harness();
        h.state_mut().tool = Tool::Gradient;
        let depth = h.state().tab().doc.history_counts().0;
        click(&mut h, 5, 5);
        assert_eq!(h.state().tab().doc.history_counts().0, depth, "a click makes no undo step");
        assert_eq!(px(&h, 5, 5) & 0xFF, 0);

        let a = at(&h, 0, 0);
        hover(&mut h, a);
        press(&mut h, a, egui::PointerButton::Primary, true);
        let b = at(&h, 31, 0);
        hover(&mut h, b);
        press(&mut h, b, egui::PointerButton::Primary, false);
        assert_eq!(px(&h, 0, 10), RED);
        assert_eq!(px(&h, 31, 10), BLUE);
        assert_eq!(h.state().tab().doc.history_counts().0, depth + 1);
    }

    #[test]
    fn gradient_with_the_right_button_swaps_the_colors() {
        let mut h = harness();
        h.state_mut().tool = Tool::Gradient;
        let a = at(&h, 0, 0);
        hover(&mut h, a);
        press(&mut h, a, egui::PointerButton::Secondary, true);
        let b = at(&h, 31, 0);
        hover(&mut h, b);
        press(&mut h, b, egui::PointerButton::Secondary, false);
        assert_eq!(px(&h, 0, 10), BLUE);
        assert_eq!(px(&h, 31, 10), RED);
    }

    #[test]
    fn contour_drag_closes_and_fills_on_release() {
        let mut h = harness();
        h.state_mut().tool = Tool::Contour;
        h.state_mut().canvas.opts.filled = true;
        let corners = [(5, 5), (20, 5), (20, 20), (5, 20)];
        let first = at(&h, corners[0].0, corners[0].1);
        hover(&mut h, first);
        press(&mut h, first, egui::PointerButton::Primary, true);
        for (x, y) in &corners[1..] {
            let p = at(&h, *x, *y);
            hover(&mut h, p);
        }
        assert_eq!(px(&h, 12, 12) & 0xFF, 0, "while dragging only the path shows");
        let last = at(&h, 20, 20);
        press(&mut h, last, egui::PointerButton::Primary, false);
        assert_eq!(px(&h, 12, 12), RED, "released: closed and filled");
    }

    #[test]
    fn polygon_adds_vertices_and_return_commits_without_the_cursor_vertex() {
        let mut h = harness();
        h.state_mut().tool = Tool::ShapePolygon;
        h.state_mut().canvas.opts.filled = true;
        for (x, y) in [(4, 4), (24, 4), (24, 24)] {
            click(&mut h, x, y);
            // Moving away previews the next vertex.
        }
        let away = at(&h, 4, 24);
        hover(&mut h, away);
        assert_eq!(px(&h, 5, 22), RED, "the preview includes the cursor as a vertex");
        h.key_press(egui::Key::Enter);
        h.run_ok();
        assert_eq!(px(&h, 5, 22) & 0xFF, 0, "Return commits the polygon without the cursor vertex");
        assert_eq!(px(&h, 20, 8), RED, "the triangle is filled");
        assert!(h.state().canvas.tools.draw.session.is_none());
    }

    #[test]
    fn polygon_escape_cancels_everything() {
        let mut h = harness();
        h.state_mut().tool = Tool::ShapePolygon;
        let depth = h.state().tab().doc.history_counts().0;
        click(&mut h, 4, 4);
        click(&mut h, 24, 4);
        click(&mut h, 24, 24);
        assert!(count_opaque(&h.state().tab().doc) > 0);
        h.key_press(egui::Key::Escape);
        h.run_ok();
        assert_eq!(count_opaque(&h.state().tab().doc), 0);
        assert_eq!(h.state().tab().doc.history_counts().0, depth);
    }

    #[test]
    fn switching_tools_commits_an_open_polygon() {
        let mut h = harness();
        h.state_mut().tool = Tool::ShapePolygon;
        click(&mut h, 4, 4);
        click(&mut h, 24, 4);
        click(&mut h, 24, 24);
        h.state_mut().tool = Tool::Pencil;
        h.run_ok();
        assert!(h.state().canvas.tools.draw.session.is_none());
        assert_eq!(px(&h, 14, 4), RED, "the edge stays");
    }

    #[test]
    fn curve_is_drag_then_two_bends_then_a_single_undo_step() {
        let mut h = harness();
        h.state_mut().tool = Tool::Curve;
        let depth = h.state().tab().doc.history_counts().0;
        let a = at(&h, 3, 28);
        hover(&mut h, a);
        press(&mut h, a, egui::PointerButton::Primary, true);
        let b = at(&h, 28, 28);
        hover(&mut h, b);
        press(&mut h, b, egui::PointerButton::Primary, false);
        assert_eq!(px(&h, 15, 28), RED, "the straight line first");
        // First bend: drag toward the top.
        let c1 = at(&h, 3, 3);
        hover(&mut h, c1);
        press(&mut h, c1, egui::PointerButton::Primary, true);
        press(&mut h, c1, egui::PointerButton::Primary, false);
        // Second bend.
        let c2 = at(&h, 28, 3);
        hover(&mut h, c2);
        press(&mut h, c2, egui::PointerButton::Primary, true);
        press(&mut h, c2, egui::PointerButton::Primary, false);
        assert!(h.state().canvas.tools.draw.session.is_none(), "the curve ends after the second control point");
        assert_eq!(px(&h, 15, 28) & 0xFF, 0, "the line became a curve");
        assert_eq!(px(&h, 3, 28), RED);
        assert_eq!(px(&h, 28, 28), RED);
        assert_eq!(h.state().tab().doc.history_counts().0, depth + 1);
    }

    #[test]
    fn curve_return_commits_early() {
        let mut h = harness();
        h.state_mut().tool = Tool::Curve;
        let a = at(&h, 3, 28);
        hover(&mut h, a);
        press(&mut h, a, egui::PointerButton::Primary, true);
        let b = at(&h, 28, 28);
        hover(&mut h, b);
        press(&mut h, b, egui::PointerButton::Primary, false);
        assert!(!h.state().canvas.tools.draw.session.is_none());
        h.key_press(egui::Key::Enter);
        h.run_ok();
        assert!(h.state().canvas.tools.draw.session.is_none());
        assert_eq!(px(&h, 15, 28), RED);
    }

    #[test]
    fn text_tool_previews_in_the_sheet_and_place_keeps_it() {
        let mut h = harness();
        h.state_mut().tool = Tool::Text;
        let depth = h.state().tab().doc.history_counts().0;
        click(&mut h, 2, 2);
        assert!(h.state().canvas.tools.text.request.is_some(), "the sheet opens");
        h.state_mut().canvas.tools.text.string = "Hi".into();
        h.run_ok();
        assert!(count_opaque(&h.state().tab().doc) > 0, "live preview");
        h.state_mut().canvas.tools.text.string = String::new();
        h.run_ok();
        assert_eq!(count_opaque(&h.state().tab().doc), 0, "empty text previews nothing");
        h.state_mut().canvas.tools.text.string = "Hi".into();
        h.state_mut().canvas.tools.text.scale = 3;
        h.run_ok();
        let big = count_opaque(&h.state().tab().doc);
        tools::text::place(h.state_mut());
        h.run_ok();
        assert!(h.state().canvas.tools.text.request.is_none());
        assert_eq!(count_opaque(&h.state().tab().doc), big);
        assert_eq!(h.state().tab().doc.history_counts().0, depth + 1);
    }

    #[test]
    fn text_cancel_removes_the_preview() {
        let mut h = harness();
        h.state_mut().tool = Tool::Text;
        let depth = h.state().tab().doc.history_counts().0;
        click(&mut h, 2, 2);
        h.state_mut().canvas.tools.text.string = "Hello".into();
        h.run_ok();
        assert!(count_opaque(&h.state().tab().doc) > 0);
        tools::text::cancel(h.state_mut());
        h.run_ok();
        assert_eq!(count_opaque(&h.state().tab().doc), 0);
        assert_eq!(h.state().tab().doc.history_counts().0, depth);
    }

    #[test]
    fn ink_popup_modes_reach_the_core() {
        let mut h = harness();
        // Lock Alpha: scribbling over transparent pixels draws nothing.
        h.state_mut().canvas.opts.ink = 2;
        click(&mut h, 4, 4);
        assert_eq!(px(&h, 4, 4) & 0xFF, 0);
        h.state_mut().canvas.opts.ink = 0;
        click(&mut h, 4, 4);
        assert_eq!(px(&h, 4, 4), RED);
    }

    #[test]
    fn shading_ink_steps_along_the_palette() {
        let mut h = harness();
        let mid = 0x800080FF;
        {
            let tab = h.state_mut().tab_mut();
            tab.doc.palette_add(RED);
            tab.doc.palette_add(mid);
            tab.doc.palette_add(BLUE);
        }
        h.state_mut().canvas.opts.ink = 4;
        click(&mut h, 6, 6);
        // Shading only recolors pixels that already hold a ramp color: draw red first with Simple.
        h.state_mut().canvas.opts.ink = 0;
        click(&mut h, 6, 6);
        assert_eq!(px(&h, 6, 6), RED);
        h.state_mut().canvas.opts.ink = 4;
        click(&mut h, 6, 6);
        assert_eq!(px(&h, 6, 6), mid, "one step from the primary toward the secondary through the palette");
    }

    #[test]
    fn brush_from_selection_needs_a_selection_and_switches_to_the_pencil() {
        let mut h = harness();
        h.state_mut().tool = Tool::Eraser;
        tools::brush_from_selection(h.state_mut());
        assert_eq!(
            h.state().error.as_deref(),
            Some("Select some visible pixels on the current layer to use as a brush.")
        );
        h.state_mut().error = None;
        h.state_mut().tool = Tool::Pencil;
        click_draw(&mut h);
        let tab = h.state_mut().tab_mut();
        // SAFETY: the document is live.
        unsafe { pixmith_core::paint::selection::pixmith_sel_rect(tab.doc.raw(), 5, 5, 7, 7) };
        tools::brush_from_selection(h.state_mut());
        assert!(h.state().error.is_none());
        assert_eq!(h.state().tool, Tool::Pencil);
        h.run_ok();
        assert!(h.state().canvas.opts.custom_brush.is_some());
        tools::reset_brush(h.state_mut());
        assert!(h.state().canvas.opts.custom_brush.is_none());
    }

    fn click_draw(h: &mut Harness<PixmithApp>) {
        h.state_mut().brush_size = 5;
        click(h, 6, 6);
        h.state_mut().brush_size = 1;
    }

    #[test]
    fn view_commands_toggle_state() {
        let mut h = harness();
        assert!(!tools::is_view_options_open(h.state()));
        tools::open_view_options(h.state_mut());
        h.run_ok();
        assert!(tools::is_view_options_open(h.state()));
        tools::toggle_view_options(h.state_mut());
        assert!(!tools::is_view_options_open(h.state()));
        tools::toggle_preview(h.state_mut());
        h.run_ok();
        assert!(tools::is_preview_open(h.state()));
        tools::toggle_preview(h.state_mut());
        assert!(!tools::is_preview_open(h.state()));
        let grid = h.state().timeline.show_grid;
        tools::toggle_grid(h.state_mut());
        assert_eq!(h.state().timeline.show_grid, !grid);
        tools::set_tiled_mode(h.state_mut(), TiledMode::Both);
        h.run_ok();
        assert_eq!(tools::tiled_mode(h.state()), TiledMode::Both);
    }

    #[test]
    fn onion_skin_and_tiling_draw_without_trouble() {
        let mut h = harness();
        {
            let tab = h.state_mut().tab_mut();
            let last = tab.doc.frame_count() - 1;
            tab.doc.add_frame(last, false);
            tab.frame = 1;
            tab.changed();
        }
        h.state_mut().timeline.onion_skin = true;
        h.state_mut().canvas.tools.settings.onion_tint = true;
        h.state_mut().canvas.tools.settings.onion_future = 2;
        tools::set_tiled_mode(h.state_mut(), TiledMode::X);
        h.state_mut().canvas.tools.settings.custom_grid = true;
        h.run_ok();
        h.run_ok();
        tools::set_tiled_mode(h.state_mut(), TiledMode::Both);
        h.run_ok();
    }

    #[test]
    fn reference_image_loads_and_clears() {
        let dir = std::env::temp_dir().join(format!("pixmith-ref-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ref.png");
        let mut sprite = Doc::new(4, 3);
        let layer = top(&sprite);
        sprite.begin_edit("Draw");
        sprite.stroke_begin(layer, 0, 1, 1, RED, 1, 0);
        sprite.stroke_end();
        // Export a PNG through the core, the way the app does.
        sprite.export_png(0, 1, &path).expect("export");
        let mut h = harness();
        let ctx = h.ctx.clone();
        tools::load_reference(h.state_mut(), &path, &ctx);
        assert!(tools::has_reference(h.state()));
        let r = h.state().canvas.tools.reference.as_ref().unwrap();
        assert_eq!((r.name.as_str(), r.size), ("ref.png", (4, 3)));
        h.run_ok();
        tools::clear_reference(h.state_mut());
        assert!(!tools::has_reference(h.state()));
        tools::load_reference(h.state_mut(), &dir.join("missing.png"), &ctx);
        assert_eq!(h.state().error.as_deref(), Some("Could not read missing.png as an image."));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preview_window_plays_its_own_frames() {
        let mut h = harness();
        {
            let tab = h.state_mut().tab_mut();
            for _ in 0..2 {
                let last = tab.doc.frame_count() - 1;
                tab.doc.add_frame(last, false);
            }
            tab.changed();
        }
        tools::open_preview(h.state_mut());
        h.run_ok();
        assert!(tools::is_preview_open(h.state()));
        assert_eq!(h.state().tab().frame, 0, "the editor stays on its own frame");
    }
}
