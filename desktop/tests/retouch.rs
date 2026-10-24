//! The Blur and Jumble tools: through the `Doc` calls they use, and through the real UI driven headlessly.
//! The `#[ignore]`d tests render screenshots (they need a GPU): `cargo test --test retouch -- --ignored`.

use pixmith_desktop::doc::Doc;

const RED: u32 = 0xFF0000FF;
const BLUE: u32 = 0x0000FFFF;

fn render(doc: &Doc) -> Vec<u8> {
    let mut buf = Vec::new();
    assert!(doc.render(0, &mut buf));
    buf
}

fn pixel(doc: &Doc, x: i32, y: i32) -> u32 {
    Doc::pixel_of(&render(doc), doc.width(), x, y).expect("inside the sprite")
}

/// A sprite with a red left half and a blue right half, drawn on its top layer.
fn halves(w: u32, h: u32) -> Doc {
    let mut doc = Doc::new(w, h);
    let layer = doc.layer_count() - 1;
    doc.begin_edit("Draw");
    for x in 0..w as i32 {
        let c = if x < w as i32 / 2 { RED } else { BLUE };
        doc.stroke_begin(layer, 0, x, 0, c, 1, 0);
        doc.stroke_to(x, h as i32 - 1);
        doc.stroke_end();
    }
    doc.touch();
    doc
}

fn stroke(doc: &mut Doc, jumble: bool, size: u32, strength: u32, seed: u32, from: (i32, i32), to: (i32, i32)) {
    let layer = doc.layer_count() - 1;
    doc.begin_edit(if jumble { "Jumble" } else { "Blur" });
    doc.retouch_begin(layer, 0, from, jumble, size, strength, seed, 0);
    doc.retouch_to(to);
    doc.retouch_end();
    doc.touch();
}

#[test]
fn blur_softens_a_hard_edge_and_is_one_undo_step() {
    let mut doc = halves(16, 16);
    let before = render(&doc);
    assert_eq!((pixel(&doc, 7, 8), pixel(&doc, 8, 8)), (RED, BLUE));
    stroke(&mut doc, false, 3, 100, 0, (4, 8), (12, 8));
    let (a, b) = (pixel(&doc, 7, 8), pixel(&doc, 8, 8));
    assert!(a != RED && a >> 24 != 0, "the red side picked up some blue: {a:08x}");
    assert!(b != BLUE, "the blue side picked up some red: {b:08x}");
    assert_eq!(pixel(&doc, 0, 0), RED, "far from the brush nothing changed");
    assert_eq!(pixel(&doc, 15, 15), BLUE);
    assert!(doc.undo());
    assert_eq!(render(&doc), before, "one undo reverts the whole stroke");
}

#[test]
fn jumble_is_seeded_and_only_moves_pixels_around() {
    let run = |seed| {
        let mut doc = halves(24, 8);
        stroke(&mut doc, true, 6, 100, seed, (8, 4), (16, 4));
        render(&doc)
    };
    let (a, b, c) = (run(11), run(11), run(12));
    assert_eq!(a, b);
    assert_ne!(a, c);
    let count = |buf: &[u8], color: u32| {
        buf.as_chunks::<4>().0.iter().filter(|p| u32::from_be_bytes([p[0], p[1], p[2], p[3]]) == color).count()
    };
    assert_eq!(count(&a, RED), 12 * 8, "no color is created or lost");
    assert_eq!(count(&a, BLUE), 12 * 8);
    assert_ne!(a, render(&halves(24, 8)), "the edge got mixed");
}

#[test]
fn a_selection_protects_what_is_outside_it() {
    let mut doc = halves(16, 16);
    // SAFETY: the document is live.
    unsafe { pixmith_core::paint::selection::pixmith_sel_rect(doc.raw(), 0, 0, 7, 15) };
    stroke(&mut doc, false, 5, 100, 0, (2, 8), (14, 8));
    for y in 0..16 {
        for x in 8..16 {
            assert_eq!(pixel(&doc, x, y), BLUE, "({x},{y}) is outside the selection");
        }
    }
    assert_ne!(pixel(&doc, 7, 8), RED, "the selected side did blur");
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

    const SIZE: [f32; 2] = [1280.0, 800.0];

    fn harness() -> Harness<'static, PixmithApp> {
        let mut app = PixmithApp { primary: RED, secondary: BLUE, ..Default::default() };
        app.tabs[0] = pixmith_desktop::app::DocTab::new(halves(32, 32));
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

    fn hover(h: &mut Harness<PixmithApp>, pos: egui::Pos2) {
        h.event(egui::Event::PointerMoved(pos));
        h.run_ok();
    }

    fn press(h: &mut Harness<PixmithApp>, pos: egui::Pos2, pressed: bool) {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
        h.run_ok();
    }

    fn drag(h: &mut Harness<PixmithApp>, from: (i32, i32), to: (i32, i32)) {
        let a = at(h, from.0, from.1);
        hover(h, a);
        press(h, a, true);
        let b = at(h, to.0, to.1);
        hover(h, b);
        press(h, b, false);
    }

    fn px(h: &Harness<PixmithApp>, x: i32, y: i32) -> u32 {
        pixel(&h.state().tab().doc, x, y)
    }

    #[test]
    fn the_rail_lists_both_tools_with_titles_keys_and_distinct_icons() {
        let paint = pixmith_desktop::app::Tool::GROUPS[1];
        assert!(paint.contains(&Tool::Blur) && paint.contains(&Tool::Jumble));
        assert_eq!((Tool::Blur.title(), Tool::Blur.key()), ("Blur", "R"));
        assert_eq!((Tool::Jumble.title(), Tool::Jumble.key()), ("Jumble", "\u{21e7}R"));
        let icons: std::collections::HashSet<_> =
            Tool::GROUPS.iter().flat_map(|g| g.iter()).map(|t| t.icon()).collect();
        assert_eq!(icons.len(), Tool::GROUPS.iter().map(|g| g.len()).sum::<usize>(), "every tool has its own icon");
    }

    #[test]
    fn r_and_shift_r_pick_the_tools() {
        let mut h = harness();
        h.key_press(egui::Key::R);
        h.run_ok();
        assert_eq!(h.state().tool, Tool::Blur);
        h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::R);
        h.run_ok();
        assert_eq!(h.state().tool, Tool::Jumble);
        h.key_press(egui::Key::B);
        h.run_ok();
        assert_eq!(h.state().tool, Tool::Pencil);
    }

    #[test]
    fn blur_drag_softens_the_edge_and_undoes_as_one_step() {
        let mut h = harness();
        h.state_mut().tool = Tool::Blur;
        h.state_mut().brush_size = 3;
        h.state_mut().canvas.opts.retouch_strength = 100.0;
        let depth = h.state().tab().doc.history_counts().0;
        drag(&mut h, (8, 16), (24, 16));
        assert_ne!(px(&h, 15, 16), RED, "the edge blurred");
        assert_eq!(px(&h, 2, 2), RED);
        assert_eq!(h.state().tab().doc.history_counts().0, depth + 1);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_ok();
        assert_eq!(px(&h, 15, 16), RED);
        assert_eq!(px(&h, 16, 16), BLUE);
    }

    #[test]
    fn jumble_drag_mixes_the_edge_and_mirroring_reaches_the_other_side() {
        let mut h = harness();
        h.state_mut().tool = Tool::Jumble;
        h.state_mut().brush_size = 4;
        drag(&mut h, (14, 4), (18, 4));
        let mixed = (0..32).filter(|&y| (12..20).any(|x| px(&h, x, y) != if x < 16 { RED } else { BLUE })).count();
        assert!(mixed > 0 && px(&h, 14, 28) == RED, "only the strokes' neighbourhood is touched");

        let mut h = harness();
        h.state_mut().tool = Tool::Blur;
        h.state_mut().brush_size = 3;
        h.state_mut().canvas.opts.retouch_strength = 100.0;
        h.state_mut().canvas.opts.mirror_y = true;
        drag(&mut h, (14, 4), (18, 4));
        assert_eq!(px(&h, 15, 4), px(&h, 15, 27), "top/bottom mirror: the same blur lands on the mirrored side");
        assert_ne!(px(&h, 15, 27), RED);
    }

    /// Clicks along the context bar and reports whether the Blur strength moved.
    fn strength_responds(tool: Tool) -> bool {
        let mut h = harness();
        h.state_mut().tool = tool;
        h.run_ok();
        let y = 1.5 * metric::BAR_HEIGHT;
        // Drag across where the slider sits; any other chip in the way ignores a drag.
        let (from, to) = (egui::pos2(280.0, y), egui::pos2(400.0, y));
        hover(&mut h, from);
        press(&mut h, from, true);
        hover(&mut h, to);
        press(&mut h, to, false);
        h.state().canvas.opts.retouch_strength != 50.0
    }

    #[test]
    fn the_context_bar_shows_strength_for_blur_only() {
        use egui_kittest::kittest::Queryable;
        assert!(strength_responds(Tool::Blur), "Blur has a strength slider");
        assert!(!strength_responds(Tool::Jumble), "Jumble has none");
        let mut h = harness();
        h.state_mut().tool = Tool::Jumble;
        h.run_ok();
        assert!(h.query_by_label("Jumble").is_some(), "the bar names the tool");
    }
}

mod screens {
    use super::*;
    use eframe::egui;
    use egui_kittest::Harness;
    use pixmith_desktop::app::Tool;
    use pixmith_desktop::{PixmithApp, theme};

    /// Renders the window at 2x in both themes; `interact` runs on the harness before the picture.
    fn shot(
        name: &str,
        mut setup: impl FnMut(&mut PixmithApp),
        mut interact: impl FnMut(&mut Harness<'_, PixmithApp>),
    ) {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/screens");
        std::fs::create_dir_all(&dir).unwrap();
        for dark in [true, false] {
            let mut app = PixmithApp { dark, ..Default::default() };
            setup(&mut app);
            let mut installed = false;
            let mut harness = Harness::builder().with_size([1280.0, 800.0]).with_pixels_per_point(2.0).build_ui_state(
                |ui, app: &mut PixmithApp| {
                    if !installed {
                        theme::install(ui.ctx());
                        theme::apply(ui.ctx(), app.dark);
                        installed = true;
                        ui.ctx().request_discard("fonts installed");
                        return;
                    }
                    let full = ui.ctx().content_rect();
                    let mut root = ui.new_child(egui::UiBuilder::new().max_rect(full));
                    root.set_clip_rect(full);
                    app.ui(&mut root);
                },
                app,
            );
            harness.run();
            interact(&mut harness);
            harness.run();
            let image = harness.render().expect("render");
            let path = dir.join(format!("{name}-{}.png", if dark { "dark" } else { "light" }));
            image.save(&path).unwrap();
            println!("wrote {}", path.display());
        }
    }

    /// A sprite with hard color edges, so the effect of the tools shows.
    fn artwork() -> Doc {
        let mut doc = Doc::new(32, 32);
        let layer = doc.layer_count() - 1;
        doc.begin_edit("Draw");
        for (i, c) in [0xE8A862FF_u32, 0x2B3566FF, 0x9ED06CFF, 0xF08AB0FF].into_iter().enumerate() {
            for y in 0..32 {
                doc.stroke_begin(layer, 0, i as i32 * 8, y, c, 1, 0);
                doc.stroke_to(i as i32 * 8 + 7, y);
                doc.stroke_end();
            }
        }
        doc.touch();
        doc
    }

    fn setup(tool: Tool, jumble: bool) -> impl FnMut(&mut PixmithApp) {
        move |app| {
            app.tabs[0] = pixmith_desktop::app::DocTab::new(artwork());
            app.tool = tool;
            app.brush_size = 6;
            let tab = app.tab_mut();
            stroke(&mut tab.doc, jumble, 6, 100, 3, (4, 10), (28, 14));
            if jumble {
                stroke(&mut tab.doc, true, 6, 100, 4, (4, 22), (28, 20));
            } else {
                stroke(&mut tab.doc, false, 6, 100, 4, (4, 22), (28, 20));
            }
            tab.changed();
        }
    }

    #[test]
    #[ignore = "needs a GPU; run with --ignored"]
    fn blur_screens() {
        shot("tool-blur", setup(Tool::Blur, false), |_| {});
    }

    #[test]
    #[ignore = "needs a GPU; run with --ignored"]
    fn jumble_screens() {
        shot("tool-jumble", setup(Tool::Jumble, true), |_| {});
    }
}
