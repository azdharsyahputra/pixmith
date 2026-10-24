//! Pen pressure end to end: the touch events egui-winit produces for a pen (on Windows, winit reads the pen's
//! `WM_POINTER` pressure) scale the brush, and a mouse stroke is unaffected.

use eframe::egui;
use egui_kittest::Harness;
use pixmith_desktop::PixmithApp;
use pixmith_desktop::app::Tool;
use pixmith_desktop::doc::Doc;
use pixmith_desktop::theme::{self, metric};
use pixmith_desktop::ui::canvas::sprite_rect;

const SIZE: [f32; 2] = [1280.0, 800.0];
const RED: u32 = 0xFF0000FF;

fn harness() -> Harness<'static, PixmithApp> {
    let mut app = PixmithApp { primary: RED, ..Default::default() };
    app.tool = Tool::Pencil;
    app.brush_size = 10;
    app.canvas.opts.pressure_size = true;
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

fn touch(h: &mut Harness<PixmithApp>, phase: egui::TouchPhase, pos: egui::Pos2, force: Option<f32>) {
    h.event(egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase, pos, force });
}

/// The vertical thickness of the stroke's column at `x`.
fn thickness(h: &Harness<PixmithApp>, x: i32) -> usize {
    let doc = &h.state().tab().doc;
    let mut buf = Vec::new();
    assert!(doc.render(0, &mut buf));
    (0..doc.height() as i32).filter(|&y| Doc::pixel_of(&buf, doc.width(), x, y).is_some_and(|c| c & 0xFF != 0)).count()
}

/// Draws a horizontal stroke the way egui-winit reports a pen: a touch event, then the emulated mouse events.
fn stroke(h: &mut Harness<PixmithApp>, force: Option<f32>) {
    let (a, b) = (at(h, 5, 16), at(h, 25, 16));
    touch(h, egui::TouchPhase::Start, a, force);
    h.event(egui::Event::PointerMoved(a));
    h.event(egui::Event::PointerButton {
        pos: a,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    h.run_ok();
    touch(h, egui::TouchPhase::Move, b, force);
    h.event(egui::Event::PointerMoved(b));
    h.run_ok();
    touch(h, egui::TouchPhase::End, b, None);
    h.event(egui::Event::PointerButton {
        pos: b,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    h.run_ok();
}

#[test]
fn a_light_pen_draws_a_thinner_line_than_a_hard_press_or_a_mouse() {
    let (mut light, mut hard, mut mouse) = (harness(), harness(), harness());
    stroke(&mut light, Some(0.3));
    stroke(&mut hard, Some(1.0));
    stroke(&mut mouse, None);
    let (l, hd, m) = (thickness(&light, 15), thickness(&hard, 15), thickness(&mouse, 15));
    assert_eq!(l, 3, "10 px brush at 30% pressure");
    assert_eq!(hd, 10, "full pressure keeps the full size");
    assert_eq!(m, 10, "a mouse has no pressure and draws at full size");
}

#[test]
fn pressure_does_nothing_while_the_option_is_off() {
    let mut h = harness();
    h.state_mut().canvas.opts.pressure_size = false;
    stroke(&mut h, Some(0.3));
    assert_eq!(thickness(&h, 15), 10);
}

#[test]
fn the_lift_frame_keeps_the_pressure_and_the_next_mouse_stroke_forgets_it() {
    let mut h = harness();
    stroke(&mut h, Some(0.3));
    // A mouse stroke afterwards (no touch events) is full size again.
    let (a, b) = (at(&h, 5, 4), at(&h, 25, 4));
    h.event(egui::Event::PointerMoved(a));
    h.event(egui::Event::PointerButton {
        pos: a,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    h.run_ok();
    h.event(egui::Event::PointerMoved(b));
    h.run_ok();
    h.event(egui::Event::PointerButton {
        pos: b,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    h.run_ok();
    assert_eq!(thickness(&h, 15), 13, "the earlier pen line (3) and the later mouse line (10) cross x = 15");
}
