//! Selection tools, move, clipboard and free transform: the `Doc` calls and the real UI driven headlessly.

use pixmith_desktop::doc::Doc;

const RED: u32 = 0xFF0000FF;
const BLUE: u32 = 0x0000FFFF;

fn pixel(doc: &Doc, x: i32, y: i32) -> u32 {
    let mut buf = Vec::new();
    assert!(doc.render(0, &mut buf));
    Doc::pixel_of(&buf, doc.width(), x, y).expect("inside the sprite")
}

fn dot(doc: &mut Doc, x: i32, y: i32, color: u32) {
    let layer = doc.layer_count() - 1;
    doc.begin_edit("Draw");
    doc.stroke_begin(layer, 0, x, y, color, 1, 0);
    doc.stroke_end();
}

mod calls {
    use super::*;
    use pixmith_desktop::ui::selection::SelMode;

    #[test]
    fn rect_selection_bounds_outline_and_delete() {
        let mut doc = Doc::new(16, 16);
        let layer = doc.layer_count() - 1;
        dot(&mut doc, 3, 3, RED);
        dot(&mut doc, 12, 12, BLUE);
        assert!(!doc.sel_active());
        doc.sel_begin(SelMode::Replace);
        doc.sel_rect((2, 2), (5, 5));
        doc.sel_end();
        assert!(doc.sel_active());
        assert_eq!(doc.sel_bounds(), Some([2, 2, 4, 4]));
        assert!(doc.sel_contains(3, 3) && !doc.sel_contains(6, 6));
        let mut outline = doc.sel_outline();
        outline.sort();
        assert_eq!(outline, vec![[2, 2, 2, 6], [2, 2, 6, 2], [2, 6, 6, 6], [6, 2, 6, 6]]);
        assert!(doc.sel_delete(layer, 0));
        assert_eq!(pixel(&doc, 3, 3) & 0xFF, 0, "the selected pixel is cleared");
        assert_eq!(pixel(&doc, 12, 12), BLUE, "the rest stays");
        assert!(doc.undo());
        assert_eq!(pixel(&doc, 3, 3), RED);
    }

    #[test]
    fn add_subtract_invert_all_clear() {
        let mut doc = Doc::new(8, 8);
        doc.sel_begin(SelMode::Replace);
        doc.sel_rect((0, 0), (3, 3));
        doc.sel_end();
        doc.sel_begin(SelMode::Add);
        doc.sel_rect((4, 4), (7, 7));
        doc.sel_end();
        assert!(doc.sel_contains(1, 1) && doc.sel_contains(5, 5) && !doc.sel_contains(5, 1));
        doc.sel_begin(SelMode::Subtract);
        doc.sel_rect((0, 0), (1, 1));
        doc.sel_end();
        assert!(!doc.sel_contains(1, 1) && doc.sel_contains(2, 2));
        doc.sel_invert();
        assert!(doc.sel_contains(1, 1) && !doc.sel_contains(2, 2));
        doc.sel_all();
        assert_eq!(doc.sel_bounds(), Some([0, 0, 8, 8]));
        doc.sel_clear();
        assert!(!doc.sel_active());
    }

    #[test]
    fn wand_selects_same_colors_contiguous_or_global() {
        let mut doc = Doc::new(8, 8);
        let layer = doc.layer_count() - 1;
        dot(&mut doc, 1, 1, RED);
        dot(&mut doc, 6, 6, RED);
        assert!(doc.sel_wand(layer, 0, 1, 1, 0, true, SelMode::Replace));
        assert!(doc.sel_contains(1, 1) && !doc.sel_contains(6, 6));
        assert!(doc.sel_wand(layer, 0, 1, 1, 0, false, SelMode::Replace));
        assert!(doc.sel_contains(1, 1) && doc.sel_contains(6, 6));
    }

    #[test]
    fn polygon_selects_the_inside() {
        let mut doc = Doc::new(12, 12);
        doc.sel_begin(SelMode::Replace);
        doc.sel_poly(&[(1, 1), (10, 1), (10, 10)]);
        doc.sel_end();
        assert!(doc.sel_contains(8, 3));
        assert!(!doc.sel_contains(2, 9));
    }

    #[test]
    fn modify_and_crop() {
        let mut doc = Doc::new(16, 16);
        doc.sel_begin(SelMode::Replace);
        doc.sel_rect((4, 4), (7, 7));
        doc.sel_end();
        assert!(doc.sel_modify(0, 2));
        assert_eq!(doc.sel_bounds(), Some([2, 2, 8, 8]));
        assert!(doc.sel_modify(1, 1));
        assert_eq!(doc.sel_bounds(), Some([3, 3, 6, 6]));
        assert!(doc.crop_to_selection());
        assert_eq!((doc.width(), doc.height()), (6, 6));
    }

    #[test]
    fn move_nudge_and_flip() {
        let mut doc = Doc::new(16, 16);
        let layer = doc.layer_count() - 1;
        dot(&mut doc, 3, 3, RED);
        doc.sel_begin(SelMode::Replace);
        doc.sel_rect((2, 2), (4, 4));
        doc.sel_end();
        assert!(doc.sel_move_begin(layer, 0));
        doc.sel_move_to(5, 0);
        doc.sel_move_end();
        assert_eq!(pixel(&doc, 8, 3), RED);
        assert_eq!(pixel(&doc, 3, 3) & 0xFF, 0);
        assert!(doc.sel_nudge(layer, 0, 0, 2));
        assert_eq!(pixel(&doc, 8, 5), RED);
        assert_eq!(doc.sel_bounds(), Some([7, 4, 3, 3]));
        assert!(doc.sel_transform(layer, 0, 0), "flip horizontal");
        assert_eq!(pixel(&doc, 8, 5), RED, "the center column stays");
    }

    #[test]
    fn copy_then_paste_floats_until_committed() {
        let mut doc = Doc::new(16, 16);
        let layer = doc.layer_count() - 1;
        dot(&mut doc, 3, 3, RED);
        doc.sel_begin(SelMode::Replace);
        doc.sel_rect((2, 2), (4, 4));
        doc.sel_end();
        let copied = doc.sel_copy(layer, 0).expect("copy");
        assert_eq!((copied.x, copied.y, copied.w, copied.h), (2, 2, 3, 3));
        assert!(doc.sel_delete(layer, 0));
        assert!(doc.sel_paste(layer, 0, copied.w, copied.h, &copied.rgba, 2, 2));
        assert!(doc.float_active());
        assert_eq!(pixel(&doc, 3, 3), RED);
        doc.float_commit();
        assert!(!doc.float_active());
        assert_eq!(pixel(&doc, 3, 3), RED);
        // A second paste, cancelled, leaves nothing behind.
        assert!(doc.sel_paste(layer, 0, copied.w, copied.h, &copied.rgba, 9, 9));
        assert!(doc.float_cancel());
        assert_eq!(pixel(&doc, 10, 10) & 0xFF, 0);
    }

    #[test]
    fn free_transform_fields_commit_and_cancel() {
        let mut doc = Doc::new(16, 16);
        let layer = doc.layer_count() - 1;
        dot(&mut doc, 3, 3, RED);
        doc.sel_begin(SelMode::Replace);
        doc.sel_rect((2, 2), (4, 4));
        doc.sel_end();
        assert!(doc.xf_begin(layer, 0, false));
        assert!(doc.xf_active());
        let f = doc.xf_fields();
        assert_eq!((f[0], f[1], f[2], f[3]), (2.0, 2.0, 3.0, 3.0));
        let (handles, pivot) = doc.xf_geometry().expect("geometry");
        assert_eq!(handles[0], (2.0, 2.0));
        assert_eq!(handles[7], (5.0, 5.0));
        assert_eq!(pivot, (3.5, 3.5));
        doc.xf_set_field(0, 6.0);
        assert_eq!(doc.xf_fields()[0], 6.0);
        doc.xf_commit();
        assert!(!doc.xf_active());
        assert_eq!(pixel(&doc, 7, 3), RED, "moved by four pixels");
        assert_eq!(pixel(&doc, 3, 3) & 0xFF, 0);

        doc.sel_all();
        assert!(doc.xf_begin(layer, 0, false));
        doc.xf_drag_begin();
        doc.xf_drag(pixmith_desktop::ui::selection::XfDrag::Move, (0, 0), false, (0.0, 0.0), (2.0, 0.0));
        assert_eq!(doc.xf_fields()[0], 2.0);
        assert!(doc.xf_cancel());
        assert!(!doc.xf_active());
        assert_eq!(pixel(&doc, 7, 3), RED, "cancel restores");
    }
}

/// Drives the real UI headlessly.
mod ui {
    use super::*;
    use eframe::egui;
    use egui_kittest::Harness;
    use pixmith_desktop::PixmithApp;
    use pixmith_desktop::app::Tool;
    use pixmith_desktop::theme::{self, metric};
    use pixmith_desktop::ui::canvas::sprite_rect;
    use pixmith_desktop::ui::selection;

    const SIZE: [f32; 2] = [1280.0, 800.0];

    fn harness() -> Harness<'static, PixmithApp> {
        let mut app = PixmithApp { primary: RED, ..Default::default() };
        app.selection.system_clipboard = false;
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
        h.run_ok();
        h
    }

    /// Screen position of the center of sprite pixel `(x, y)`.
    fn at(h: &Harness<PixmithApp>, x: i32, y: i32) -> egui::Pos2 {
        let app = h.state();
        let timeline = pixmith_desktop::ui::timeline::panel_height(app);
        let strip = if app.tab().doc.xf_active() {
            pixmith_desktop::ui::transform::bar_height(
                app,
                SIZE[0] - metric::TOOL_RAIL - metric::INSPECTOR_WIDTH,
                &h.ctx,
            )
            .unwrap_or(0.0)
        } else {
            0.0
        };
        let area = egui::Rect::from_min_max(
            egui::pos2(metric::TOOL_RAIL, 2.0 * metric::BAR_HEIGHT + strip),
            egui::pos2(SIZE[0] - metric::INSPECTOR_WIDTH, SIZE[1] - timeline),
        );
        sprite_rect(area, app.tab(), 1.0).min + egui::vec2(x as f32 + 0.5, y as f32 + 0.5) * app.tab().zoom
    }

    fn button(h: &mut Harness<PixmithApp>, pos: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers });
        h.run_ok();
    }

    fn hover(h: &mut Harness<PixmithApp>, pos: egui::Pos2) {
        h.event(egui::Event::PointerMoved(pos));
        h.run_ok();
    }

    fn drag(h: &mut Harness<PixmithApp>, from: (i32, i32), to: (i32, i32), modifiers: egui::Modifiers) {
        let (a, b) = (at(h, from.0, from.1), at(h, to.0, to.1));
        hover(h, a);
        h.event(egui::Event::ModifiersChanged(modifiers));
        h.run_ok();
        button(h, a, true, modifiers);
        hover(h, b);
        button(h, b, false, modifiers);
        h.event(egui::Event::ModifiersChanged(Default::default()));
        h.run_ok();
    }

    fn click(h: &mut Harness<PixmithApp>, p: (i32, i32)) {
        let a = at(h, p.0, p.1);
        hover(h, a);
        button(h, a, true, Default::default());
        button(h, a, false, Default::default());
    }

    fn bounds(h: &Harness<PixmithApp>) -> Option<[i32; 4]> {
        h.state().tab().doc.sel_bounds()
    }

    fn px(h: &Harness<PixmithApp>, x: i32, y: i32) -> u32 {
        pixel(&h.state().tab().doc, x, y)
    }

    fn key(h: &mut Harness<PixmithApp>, key: egui::Key) {
        h.key_press(key);
        h.run_ok();
    }

    fn cmd(h: &mut Harness<PixmithApp>, key: egui::Key) {
        h.key_press_modifiers(egui::Modifiers::COMMAND, key);
        h.run_ok();
    }

    #[test]
    fn marquee_drags_a_rectangle_adds_and_clears() {
        let mut h = harness();
        h.state_mut().tool = Tool::Marquee;
        drag(&mut h, (2, 2), (5, 5), Default::default());
        assert_eq!(bounds(&h), Some([2, 2, 4, 4]));
        assert_eq!(h.state().tab().doc.sel_outline().len(), 4);
        // Shift adds a second rectangle.
        drag(&mut h, (8, 8), (10, 10), egui::Modifiers::SHIFT);
        assert_eq!(bounds(&h), Some([2, 2, 9, 9]));
        // Option subtracts.
        drag(&mut h, (8, 8), (10, 10), egui::Modifiers::ALT);
        assert_eq!(bounds(&h), Some([2, 2, 4, 4]));
        // A plain click without a drag deselects.
        click(&mut h, (12, 12));
        assert_eq!(bounds(&h), None);
        assert!(h.state_mut().tab_mut().doc.undo(), "selection edits are undoable");
    }

    #[test]
    fn lasso_follows_the_pointer() {
        let mut h = harness();
        h.state_mut().tool = Tool::Lasso;
        let pts = [(2, 2), (12, 2), (12, 12), (2, 12)];
        let first = at(&h, pts[0].0, pts[0].1);
        hover(&mut h, first);
        button(&mut h, first, true, Default::default());
        for p in &pts[1..] {
            let pos = at(&h, p.0, p.1);
            hover(&mut h, pos);
        }
        let last = at(&h, 2, 12);
        button(&mut h, last, false, Default::default());
        assert_eq!(bounds(&h), Some([2, 2, 11, 11]));
    }

    #[test]
    fn polygon_closes_with_return_and_cancels_with_escape() {
        let mut h = harness();
        h.state_mut().tool = Tool::Polygon;
        for p in [(2, 2), (13, 2), (13, 13)] {
            click(&mut h, p);
        }
        assert!(h.state().selection.polygon_open());
        assert!(h.state().tab().doc.sel_active());
        key(&mut h, egui::Key::Enter);
        assert!(!h.state().selection.polygon_open());
        assert!(h.state().tab().doc.sel_contains(11, 4));
        assert!(!h.state().tab().doc.sel_contains(3, 12));

        h.state_mut().tab_mut().doc.sel_clear();
        click(&mut h, (2, 2));
        click(&mut h, (8, 2));
        assert!(h.state().selection.polygon_open());
        key(&mut h, egui::Key::Escape);
        assert!(!h.state().selection.polygon_open());
        assert!(h.state().tab().doc.sel_outline().is_empty(), "cancelling undoes the polygon");
    }

    #[test]
    fn wand_uses_tolerance_and_contiguity_options() {
        let mut h = harness();
        {
            let doc = &mut h.state_mut().tab_mut().doc;
            dot(doc, 1, 1, RED);
            dot(doc, 14, 14, RED);
        }
        h.state_mut().tab_mut().changed();
        h.state_mut().tool = Tool::Wand;
        h.state_mut().selection.contiguous = false;
        click(&mut h, (1, 1));
        assert!(h.state().tab().doc.sel_contains(14, 14), "global wand takes every matching pixel");
        h.state_mut().selection.contiguous = true;
        click(&mut h, (1, 1));
        assert!(!h.state().tab().doc.sel_contains(14, 14));
        assert!(h.state().tab().doc.sel_contains(1, 1));
    }

    #[test]
    fn move_tool_drags_pixels_and_arrows_nudge() {
        let mut h = harness();
        {
            let doc = &mut h.state_mut().tab_mut().doc;
            dot(doc, 3, 3, RED);
        }
        h.state_mut().tab_mut().changed();
        h.state_mut().tool = Tool::Marquee;
        drag(&mut h, (2, 2), (5, 5), Default::default());
        h.state_mut().tool = Tool::Move;
        h.run_ok();
        drag(&mut h, (3, 3), (7, 3), Default::default());
        assert_eq!(px(&h, 7, 3), RED);
        assert_eq!(px(&h, 3, 3) & 0xFF, 0);
        assert_eq!(bounds(&h), Some([6, 2, 4, 4]));
        key(&mut h, egui::Key::ArrowDown);
        assert_eq!(px(&h, 7, 4), RED);
        h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowRight);
        h.run_ok();
        assert_eq!(px(&h, 17, 4), RED, "Shift moves ten pixels");
        assert_eq!(bounds(&h).map(|b| b[0]), Some(16));
    }

    #[test]
    fn select_all_deselect_invert_and_delete_shortcuts() {
        let mut h = harness();
        {
            let doc = &mut h.state_mut().tab_mut().doc;
            dot(doc, 4, 4, RED);
        }
        h.state_mut().tab_mut().changed();
        cmd(&mut h, egui::Key::A);
        assert_eq!(bounds(&h), Some([0, 0, 32, 32]));
        cmd(&mut h, egui::Key::D);
        assert_eq!(bounds(&h), None);
        h.state_mut().tool = Tool::Marquee;
        drag(&mut h, (3, 3), (5, 5), Default::default());
        h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::I);
        h.run_ok();
        assert!(!h.state().tab().doc.sel_contains(4, 4));
        assert!(h.state().tab().doc.sel_contains(20, 20));
        cmd(&mut h, egui::Key::D);
        cmd(&mut h, egui::Key::A);
        key(&mut h, egui::Key::Delete);
        assert_eq!(px(&h, 4, 4) & 0xFF, 0, "Delete clears the selection");
        let revision = h.state().tab().revision;
        assert!(revision > 0);
    }

    #[test]
    fn escape_deselects_and_menu_commands_work_on_the_app() {
        let mut h = harness();
        h.state_mut().tool = Tool::Marquee;
        drag(&mut h, (2, 2), (6, 6), Default::default());
        assert!(selection::has_selection(h.state()));
        key(&mut h, egui::Key::Escape);
        assert!(!selection::has_selection(h.state()));
        selection::select_all(h.state_mut());
        assert!(selection::has_selection(h.state()));
        selection::invert_selection(h.state_mut());
        assert!(!selection::has_selection(h.state()), "inverting everything selects nothing");
        selection::select_all(h.state_mut());
        selection::crop_to_selection(h.state_mut());
        h.run_ok();
        assert_eq!(h.state().tab().doc.width(), 32);
    }

    #[test]
    fn copy_paste_floats_on_the_move_tool_and_return_commits() {
        let mut h = harness();
        {
            let doc = &mut h.state_mut().tab_mut().doc;
            dot(doc, 3, 3, RED);
        }
        h.state_mut().tab_mut().changed();
        h.state_mut().tool = Tool::Marquee;
        drag(&mut h, (2, 2), (4, 4), Default::default());
        cmd(&mut h, egui::Key::X);
        assert_eq!(px(&h, 3, 3) & 0xFF, 0, "cut clears the pixels");
        cmd(&mut h, egui::Key::V);
        assert_eq!(h.state().tool, Tool::Move, "paste switches to the move tool");
        assert!(h.state().tab().doc.float_active());
        assert_eq!(px(&h, 3, 3), RED, "the paste lands where it was copied from");
        // Esc cancels the floating paste.
        key(&mut h, egui::Key::Escape);
        assert!(!h.state().tab().doc.float_active());
        assert_eq!(px(&h, 3, 3) & 0xFF, 0);
        cmd(&mut h, egui::Key::V);
        assert!(h.state().tab().doc.float_active());
        key(&mut h, egui::Key::Enter);
        assert!(!h.state().tab().doc.float_active());
        assert_eq!(px(&h, 3, 3), RED, "Return commits it");
    }

    #[test]
    fn switching_tools_commits_a_floating_paste() {
        let mut h = harness();
        {
            let doc = &mut h.state_mut().tab_mut().doc;
            dot(doc, 3, 3, RED);
        }
        h.state_mut().tab_mut().changed();
        selection::select_all(h.state_mut());
        selection::copy(h.state_mut(), false);
        selection::paste(h.state_mut());
        h.run_ok();
        assert!(h.state().tab().doc.float_active());
        h.state_mut().tool = Tool::Pencil;
        h.run_ok();
        assert!(!h.state().tab().doc.float_active());
    }

    #[test]
    fn select_by_color_arms_until_the_next_canvas_click() {
        let mut h = harness();
        {
            let doc = &mut h.state_mut().tab_mut().doc;
            dot(doc, 1, 1, RED);
            dot(doc, 14, 14, RED);
        }
        h.state_mut().tab_mut().changed();
        h.state_mut().tool = Tool::Pencil;
        h.state_mut().canvas.cursor = None;
        selection::select_by_color(h.state_mut());
        click(&mut h, (1, 1));
        assert!(h.state().tab().doc.sel_contains(14, 14));
        assert_eq!(px(&h, 1, 1), RED, "the click selected instead of drawing");
        assert_eq!(px(&h, 2, 2) & 0xFF, 0);
    }

    #[test]
    fn modify_dialog_expands_the_selection() {
        let mut h = harness();
        h.state_mut().tool = Tool::Marquee;
        drag(&mut h, (8, 8), (10, 10), Default::default());
        selection::modify_selection(h.state_mut(), 0);
        h.run_ok();
        assert!(h.state().selection.modify.is_some());
        key(&mut h, egui::Key::Enter);
        h.run_ok();
        assert!(h.state().selection.modify.is_none());
        assert_eq!(bounds(&h), Some([7, 7, 5, 5]), "expanded by one pixel");
    }

    #[test]
    fn free_transform_strip_and_keys() {
        let mut h = harness();
        {
            let doc = &mut h.state_mut().tab_mut().doc;
            dot(doc, 3, 3, RED);
        }
        h.state_mut().tab_mut().changed();
        h.state_mut().tool = Tool::Marquee;
        drag(&mut h, (2, 2), (4, 4), Default::default());
        cmd(&mut h, egui::Key::T);
        assert!(h.state().tab().doc.xf_active());
        h.run_ok();
        let width = SIZE[0] - metric::TOOL_RAIL - metric::INSPECTOR_WIDTH;
        let strip = pixmith_desktop::ui::transform::bar_height(h.state(), width, &h.ctx).expect("strip");
        assert!(strip == metric::BAR_HEIGHT || strip == 2.0 * metric::BAR_HEIGHT);
        // Arrow keys move the box instead of the selection.
        key(&mut h, egui::Key::ArrowRight);
        assert_eq!(h.state().tab().doc.xf_fields()[0], 3.0);
        // Dragging inside the box moves it by whole pixels.
        drag(&mut h, (5, 4), (8, 4), Default::default());
        assert_eq!(h.state().tab().doc.xf_fields()[0], 6.0);
        // Return applies.
        key(&mut h, egui::Key::Enter);
        assert!(!h.state().tab().doc.xf_active());
        assert_eq!(px(&h, 7, 3), RED);
        assert_eq!(px(&h, 3, 3) & 0xFF, 0);
        // Esc discards.
        cmd(&mut h, egui::Key::T);
        key(&mut h, egui::Key::ArrowDown);
        key(&mut h, egui::Key::Escape);
        assert!(!h.state().tab().doc.xf_active());
        assert_eq!(px(&h, 7, 3), RED);
        assert!(selection::has_selection(h.state()), "Esc closed the transform, not the selection");
    }

    #[test]
    fn grabbing_a_handle_with_the_move_tool_starts_a_transform() {
        let mut h = harness();
        {
            let doc = &mut h.state_mut().tab_mut().doc;
            dot(doc, 8, 8, RED);
        }
        h.state_mut().tab_mut().changed();
        h.state_mut().tool = Tool::Marquee;
        drag(&mut h, (8, 8), (15, 15), Default::default());
        h.state_mut().tool = Tool::Move;
        h.run_ok();
        // The bottom right handle of the box is the corner at (16, 16); drag it outwards to scale.
        let corner = at(&h, 15, 15) + egui::vec2(h.state().tab().zoom / 2.0, h.state().tab().zoom / 2.0);
        hover(&mut h, corner);
        button(&mut h, corner, true, Default::default());
        assert!(h.state().tab().doc.xf_active(), "a handle press begins the free transform");
        let target = corner + egui::vec2(h.state().tab().zoom * 4.0, h.state().tab().zoom * 4.0);
        hover(&mut h, target);
        button(&mut h, target, false, Default::default());
        let f = h.state().tab().doc.xf_fields();
        assert!(f[2] > 8.0 && f[3] > 8.0, "the box grew: {f:?}");
    }
}
