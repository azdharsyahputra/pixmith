//! Drives the timeline panel headlessly with synthetic pointer and key events (no GPU needed).

use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::Harness;
use pixmith_desktop::{PixmithApp, theme};

fn harness(setup: impl FnOnce(&mut PixmithApp)) -> Harness<'static, PixmithApp> {
    harness_stepping(0.25, setup)
}

/// A harness whose frames advance `dt` seconds (the default 0.25 s is longer than the double-click window).
fn harness_stepping(dt: f32, setup: impl FnOnce(&mut PixmithApp)) -> Harness<'static, PixmithApp> {
    let mut app = PixmithApp::default();
    setup(&mut app);
    let mut installed = false;
    let mut h = Harness::builder().with_size([1280.0, 800.0]).with_step_dt(dt).build_ui_state(
        move |ui, app: &mut PixmithApp| {
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
    h.run();
    h
}

fn click(h: &mut Harness<'static, PixmithApp>, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run();
}

fn with_frames_and_layers(app: &mut PixmithApp) {
    let tab = app.tab_mut();
    for _ in 0..3 {
        let last = tab.doc.frame_count() - 1;
        tab.doc.add_frame(last, false);
    }
    for _ in 0..2 {
        let top = tab.doc.layer_count() - 1;
        tab.layer = tab.doc.add_layer(top);
    }
    tab.frame = 0;
    tab.changed();
}

// Geometry: the panel sits at the bottom with the height `panel_height` computes.
fn grid_origin(h: &Harness<'static, PixmithApp>) -> Pos2 {
    let rows = h.state().tab().doc.layer_count() as f32;
    let height = 168.0_f32.max(36.0 + 48.0 + rows * 24.0 + 8.0).min(252.0);
    // 48 pt rail on the left; header 36 + rule 1 + tag area 20 + number row 20 (no tags: one empty row).
    pos2(48.0 + 200.0, 800.0 - height + 37.0)
}

#[test]
fn clicking_a_frame_number_selects_the_frame() {
    let mut h = harness(with_frames_and_layers);
    let o = grid_origin(&h);
    // Number row is 20 pt tall below the 20 pt empty tag row.
    click(&mut h, pos2(o.x + 2.0 * 32.0 + 16.0, o.y + 20.0 + 4.0 + 10.0));
    assert_eq!(h.state().tab().frame, 2);
}

#[test]
fn clicking_a_cel_selects_layer_and_frame() {
    let mut h = harness(with_frames_and_layers);
    let o = grid_origin(&h);
    // Tag area (one empty row: 16 + 4) and the number row (20) sit above the cels.
    let body_top = o.y + 20.0 + 20.0;
    // Bottom row (layer 0) at the third frame.
    let rows = h.state().tab().doc.layer_count();
    click(&mut h, pos2(o.x + 3.0 * 32.0 + 16.0, body_top + (rows - 1) as f32 * 24.0 + 12.0));
    let tab = h.state().tab();
    assert_eq!((tab.layer, tab.frame), (0, 3));
}

#[test]
fn space_toggles_playback_and_keys_step() {
    let mut h = harness(with_frames_and_layers);
    h.key_press(egui::Key::Space);
    h.step();
    assert!(h.state().tab().playing);
    h.key_press(egui::Key::Space);
    h.step();
    assert!(!h.state().tab().playing);
    h.state_mut().tab_mut().frame = 0;
    h.key_press(egui::Key::Period);
    h.run();
    assert_eq!(h.state().tab().frame, 1);
    h.key_press(egui::Key::Comma);
    h.run();
    assert_eq!(h.state().tab().frame, 0);
}

#[test]
fn collapsing_shrinks_the_panel() {
    let mut h = harness(with_frames_and_layers);
    let expanded = pixmith_desktop::ui::timeline::panel_height(h.state());
    h.state_mut().timeline.collapsed = true;
    h.run();
    assert!(pixmith_desktop::ui::timeline::panel_height(h.state()) < expanded);
}

fn drag(h: &mut Harness<'static, PixmithApp>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for k in 1..=4 {
        h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
        h.step();
    }
    h.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(5);
}

#[test]
fn dragging_the_top_edge_resizes_and_double_click_resets() {
    use pixmith_desktop::ui::timeline::{MIN_HEIGHT, panel_height};
    let mut h = harness_stepping(1.0 / 60.0, with_frames_and_layers);
    let auto = panel_height(h.state());
    let top = 800.0 - auto + 1.0;

    drag(&mut h, pos2(700.0, top), pos2(700.0, top - 100.0));
    let taller = panel_height(h.state());
    assert!((taller - (auto + 100.0)).abs() <= 2.0, "auto {auto}, after drag {taller}");
    assert_eq!(h.state().timeline.height.map(f32::round), Some(taller.round()));

    // Never past 70% of the column, never below the minimum.
    let top = 800.0 - taller + 1.0;
    drag(&mut h, pos2(700.0, top), pos2(700.0, 5.0));
    let max = h.state().timeline.max_height;
    assert!(panel_height(h.state()) <= max + 0.5 && max < 800.0 * 0.7 + 0.5);
    let top = 800.0 - panel_height(h.state()) + 1.0;
    drag(&mut h, pos2(700.0, top), pos2(700.0, 799.0));
    assert_eq!(panel_height(h.state()), MIN_HEIGHT);

    let at = pos2(700.0, 800.0 - MIN_HEIGHT + 1.0);
    h.event(Event::PointerMoved(at));
    h.step();
    for pressed in [true, false, true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(5);
    assert_eq!(h.state().timeline.height, None);
    assert_eq!(panel_height(h.state()), auto);
}
