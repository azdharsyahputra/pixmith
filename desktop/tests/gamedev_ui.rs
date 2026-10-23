//! Drives the game-development UI headlessly with synthetic pointer events (no GPU needed): the Tween panel and
//! its canvas handles, the Godot sheets, the script alert and progress sheet, and the crash recovery question.

use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::Harness;
use pixmith_desktop::app::DocTab;
use pixmith_desktop::doc::Doc;
use pixmith_desktop::ui::canvas::{ShapeKind, flag};
use pixmith_desktop::ui::gamedev::{Notice, TweenEase, TweenProp};
use pixmith_desktop::{PixmithApp, theme};

fn harness(setup: impl FnOnce(&mut PixmithApp)) -> Harness<'static, PixmithApp> {
    let mut app = PixmithApp::default();
    setup(&mut app);
    let mut installed = false;
    let mut h = Harness::builder().with_size([1280.0, 800.0]).build_ui_state(
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

fn press(h: &mut Harness<'static, PixmithApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

/// A 64 x 64 sprite with a filled square from (10, 10) to (29, 29) and four frames.
fn box_tab() -> DocTab {
    let mut doc = Doc::new(64, 64);
    doc.begin_edit("Shape");
    doc.draw_shape(ShapeKind::Rect, 0, 0, (10, 10), (29, 29), 0xFF8800FF, 1, flag::FILLED, false);
    doc.stroke_end();
    for _ in 0..3 {
        let last = doc.frame_count() - 1;
        doc.add_frame(last, true);
    }
    doc.touch();
    let mut tab = DocTab::new(doc);
    tab.layer = 0;
    tab.zoom = 8.0;
    tab
}

/// Screen position of sprite pixel (x, y) for a 64 x 64 sprite zoomed 8x in the unscrolled canvas.
fn at(x: f32, y: f32) -> Pos2 {
    // The canvas area: right of the 48 pt rail, below the two 40 pt bars, left of the 272 pt inspector,
    // above the 252 pt timeline; the sprite is centered in it.
    let center = pos2((48.0 + 1280.0 - 272.0) / 2.0, (80.0 + 800.0 - 252.0) / 2.0);
    pos2(center.x + (x - 32.0) * 8.0, center.y + (y - 32.0) * 8.0)
}

#[test]
fn dragging_on_the_canvas_with_edit_on_canvas_creates_keys() {
    let mut h = harness(|app| {
        app.tabs[0] = box_tab();
        let tab = &app.tabs[0];
        app.canvas.keep_view(tab);
        app.gamedev.tween.edit_on_canvas = true;
    });
    let before = h.state().tab().doc.history_counts().0;
    let start = at(14.0, 25.0);
    h.event(Event::PointerMoved(start));
    h.step();
    press(&mut h, start, true);
    for dx in [8.0, 16.0] {
        h.event(Event::PointerMoved(pos2(start.x + dx, start.y)));
        h.step();
    }
    press(&mut h, pos2(start.x + 16.0, start.y), false);
    h.run();
    let tab = h.state().tab();
    assert_eq!(tab.doc.tween_value(0, TweenProp::PosX, 0), 2.0, "dragged two pixels right");
    assert_eq!(tab.doc.tween_value(0, TweenProp::PosY, 0), 0.0);
    assert_eq!(tab.doc.history_counts().0, before + 1, "the drag is one undo step");
    // The pencil did not draw under the drag.
    let mut px = Vec::new();
    assert!(tab.doc.render(0, &mut px));
    assert_eq!(&px[(25 * 64 + 8) * 4..][..4], &[0, 0, 0, 0]);
}

#[test]
fn without_edit_on_canvas_the_tools_get_the_mouse() {
    let mut h = harness(|app| {
        app.tabs[0] = box_tab();
        let tab = &app.tabs[0];
        app.canvas.keep_view(tab);
        app.primary = 0x00FF00FF;
    });
    let p = at(40.0, 40.0);
    h.event(Event::PointerMoved(p));
    h.step();
    press(&mut h, p, true);
    press(&mut h, p, false);
    h.run();
    assert!(h.state().tab().doc.tween_keys(0, TweenProp::PosX).is_empty());
    let mut px = Vec::new();
    assert!(h.state().tab().doc.render(0, &mut px));
    let drawn: Vec<usize> =
        (0..64 * 64).filter(|i| i % 64 > 31 && px[i * 4 + 3] != 0 && px[i * 4 + 1] == 255 && px[i * 4] == 0).collect();
    assert!(!drawn.is_empty(), "the pencil drew: {:?} {:?}", h.state().tab().doc.history_counts(), h.state().tool);
}

#[test]
fn the_panel_and_the_sheets_draw() {
    let mut h = harness(|app| {
        app.tabs[0] = box_tab();
        let tab = &mut app.tabs[0];
        tab.doc.tween_set_key(0, TweenProp::PosX, 0, 0.0, TweenEase::Linear, [0.0; 4]);
        tab.doc.tween_set_key(0, TweenProp::PosX, 3, 9.0, TweenEase::Bezier, [0.2, 0.0, 0.8, 1.0]);
        app.toggle_tween_panel();
        app.gamedev.tween.edit_on_canvas = true;
        app.gamedev.tween.ease = TweenEase::Bezier;
    });
    h.run();
    assert!(h.state().tween_panel_open());
    // The Godot sheet, then the notice that follows a re-export, then the progress sheet of a script.
    h.state_mut().gamedev.tween.open = false;
    h.state_mut().open_godot_export();
    h.run();
    assert!(h.state().gamedev.godot.is_some());
    h.state_mut().gamedev.godot = None;
    h.state_mut().gamedev.notice = Some(Notice { title: "x.lua".into(), text: "hello".into(), warning: false });
    h.run();
    assert!(h.state().gamedev.notice.is_some(), "stays until OK");
    h.key_press(egui::Key::Enter);
    h.run();
    assert!(h.state().gamedev.notice.is_none(), "Return answers OK");
}

#[test]
fn the_recovery_question_restores_leftover_copies() {
    let dir = std::env::temp_dir().join(format!("pixmith-gamedev-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let copy = dir.join("abc.aseprite");
    Doc::new(12, 9).save(&copy).unwrap();
    std::fs::write(dir.join("abc.json"), r#"{"path":null,"name":"Untitled 3","date":"2026-01-01T00:00:00Z"}"#).unwrap();
    let mut h = harness(|app| app.gamedev.autosave.start_in(dir.clone()));
    h.run();
    assert!(h.state().gamedev.autosave.offer().is_some(), "offered at launch");
    // Return answers the default button: Restore.
    h.key_press(egui::Key::Enter);
    h.run();
    assert!(h.state().gamedev.autosave.offer().is_none());
    assert_eq!(h.state().tabs.len(), 1, "the untouched Untitled tab is replaced");
    assert_eq!((h.state().tab().doc.width(), h.state().tab().doc.height()), (12, 9));
    assert!(h.state().tab().doc.is_dirty(), "restored work is unsaved");
    let _ = std::fs::remove_dir_all(&dir);
}
