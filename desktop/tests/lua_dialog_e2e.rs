//! The real core and the real example scripts of `scripts/lua-examples/`, driven through the egui UI headlessly:
//! dialogs open, widgets are used, and the script's result lands in the document as one undo step.

mod common;

use eframe::egui::{self, Event, Key, Modifiers, PointerButton, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pixmith_desktop::PixmithApp;
use pixmith_desktop::doc::Doc;
use std::path::PathBuf;

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/lua-examples").join(name)
}

fn setup() -> Harness<'static, PixmithApp> {
    // `PixmithApp::default()` has a host that only sees its own sessions, so tests can run side by side.
    common::harness(PixmithApp::default())
}

fn first_dialog_id(app: &PixmithApp) -> Option<String> {
    app.gamedev.lua.dialogs().next().map(|d| d.id.clone())
}

fn session_over(app: &PixmithApp) -> bool {
    app.gamedev.lua.sessions().next().is_none()
}

fn pixel(h: &Harness<'static, PixmithApp>, x: i32, y: i32) -> u32 {
    let doc = &h.state().tab().doc;
    let mut buf = Vec::new();
    assert!(doc.render(0, &mut buf));
    Doc::pixel_of(&buf, doc.width(), x, y).unwrap()
}

#[test]
fn palette_ramp_dialog_adds_colors_as_one_undo_step() {
    let mut h = setup();
    let before = h.state().tab().doc.palette().len();
    h.state_mut().run_script_at(&example("palette-ramp-dialog.lua"));
    common::settle(&mut h, |app| first_dialog_id(app).is_some());
    let d = h.state().gamedev.lua.dialogs().next().unwrap().clone();
    assert!(d.modal && d.title == "Palette ramp");
    assert_eq!(d.widget("steps").unwrap().value, 8.0);
    assert_eq!(d.widget("preview").unwrap().colors.len(), 8);
    assert!(h.state().gamedev.lua.modal_open());

    // Drag the slider to its end: the script's onchange rebuilds the preview through dialog.modify.
    common::settle(&mut h, |app| !app.job_running());
    let rect = h.get_by_label("Steps").rect();
    let y = rect.center().y;
    h.event(Event::PointerMoved(pos2(rect.left() + 2.0, y)));
    h.step();
    h.event(Event::PointerButton {
        pos: pos2(rect.left() + 2.0, y),
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(Event::PointerMoved(pos2(rect.right() - 1.0, y)));
    h.step();
    h.event(Event::PointerButton {
        pos: pos2(rect.right() - 1.0, y),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    common::settle(&mut h, |app| {
        app.gamedev.lua.dialogs().next().is_some_and(|d| d.widget("preview").unwrap().colors.len() == 32)
    });
    assert_eq!(h.state().gamedev.lua.dialogs().next().unwrap().widget("steps").unwrap().value, 32.0);

    h.get_by_label("Add to palette").click();
    common::settle(&mut h, session_over);
    assert!(h.state().gamedev.lua.dialogs().next().is_none());
    assert!(!h.state().gamedev.lua.modal_open());
    let tab = h.state().tab();
    // `addPaletteColor` skips colors the palette has already.
    assert!(
        (before + 25..=before + 32).contains(&tab.doc.palette().len()),
        "{} -> {}",
        before,
        tab.doc.palette().len()
    );
    let added = tab.doc.palette().len() - before;
    assert_eq!(tab.doc.history_counts(), (1, 0), "dialog, callbacks and the code after show are one undo step");
    let notice = h.state().gamedev.notice.clone().expect("app.alert output");
    assert_eq!(notice.text, "Added 32 colors to the palette.");
    assert!(added > 0);
    assert!(h.state_mut().tab_mut().doc.undo());
    assert_eq!(h.state().tab().doc.palette().len(), before);
}

#[test]
fn closing_the_dialog_with_escape_leaves_the_document_alone() {
    let mut h = setup();
    h.state_mut().run_script_at(&example("palette-ramp-dialog.lua"));
    common::settle(&mut h, |app| first_dialog_id(app).is_some());
    common::settle(&mut h, |app| !app.job_running());
    h.key_press(Key::Escape);
    common::settle(&mut h, session_over);
    let tab = h.state().tab();
    assert_eq!(tab.doc.history_counts(), (0, 0));
    assert!(h.state().gamedev.notice.is_none(), "no ok, no alert");
}

#[test]
fn cancel_button_closes_without_changes() {
    let mut h = setup();
    h.state_mut().run_script_at(&example("palette-ramp-dialog.lua"));
    common::settle(&mut h, |app| first_dialog_id(app).is_some());
    common::settle(&mut h, |app| !app.job_running());
    h.get_by_label("Cancel").click();
    common::settle(&mut h, session_over);
    assert_eq!(h.state().tab().doc.history_counts(), (0, 0));
}

#[test]
fn a_plugin_command_opens_a_dialog_and_edits_the_active_document() {
    let mut h = setup();
    let source = std::fs::read_to_string(example("plugin-hello.lua")).unwrap();
    let id = h.state_mut().lua_load_plugin(&source, "plugin-hello", None, None).expect("plugin loads");
    common::settle(&mut h, |app| !app.gamedev.lua.commands().is_empty());
    // The plugin is idle: it does not lock the app. Its `onenabled` answers once it is (slow runners see the
    // command before that).
    common::settle(&mut h, |app| !app.job_running());
    common::settle(&mut h, |app| app.gamedev.lua.commands().first().is_some_and(|c| c.enabled));
    let cmd = h.state().gamedev.lua.commands()[0].clone();
    assert_eq!((cmd.session, cmd.id.as_str(), cmd.title.as_str()), (id, "mark-corner", "Mark Top-Left Corner..."));
    assert!(cmd.enabled);

    h.state_mut().lua_run_command(0);
    common::settle(&mut h, |app| first_dialog_id(app).is_some());
    h.get_by_label_contains("Used 0 times");
    common::settle(&mut h, |app| !app.job_running());
    h.get_by_label("Mark").click();
    common::settle(&mut h, |app| first_dialog_id(app).is_none() && !app.job_running());
    assert_eq!(pixel(&h, 0, 0), 0xFF004DFF);
    assert_eq!(h.state().tab().doc.history_counts(), (1, 0), "one undo step per command run");

    // The second run reads the preference the first one saved.
    h.state_mut().lua_run_command(0);
    common::settle(&mut h, |app| first_dialog_id(app).is_some());
    h.get_by_label_contains("Used 1 times");
    h.key_press(Key::Escape);
    common::settle(&mut h, |app| first_dialog_id(app).is_none() && !app.job_running());
    assert_eq!(h.state().tab().doc.history_counts(), (1, 0), "closing the dialog changes nothing");

    h.state_mut().lua_unload_plugin(id);
    common::settle(&mut h, session_over);
    assert!(h.state().gamedev.lua.commands().is_empty());
}

#[test]
fn the_plugin_hears_sitechange_from_the_app() {
    let mut h = setup();
    let source = std::fs::read_to_string(example("plugin-hello.lua")).unwrap();
    h.state_mut().lua_load_plugin(&source, "plugin-hello", None, None).unwrap();
    common::settle(&mut h, |app| !app.gamedev.lua.commands().is_empty() && !app.job_running());
    {
        let tab = h.state_mut().tab_mut();
        tab.doc.add_layer(0);
        tab.layer = 1;
        tab.changed();
    }
    common::settle(&mut h, |app| app.gamedev.notice.is_some());
    let notice = h.state().gamedev.notice.clone().unwrap();
    assert!(notice.text.starts_with("site: layer "), "{}", notice.text);
    let _ = egui::Id::NULL;
}
