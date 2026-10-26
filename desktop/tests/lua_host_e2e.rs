//! The Lua host contract in the egui app (`docs/LUA_HOST_API.md`): the "ask first" permission dialog for scripts that
//! touch files, and the documents scripts create, save and close. The real core, driven through the UI headlessly.
//! Files live in per-test temporary folders.

mod common;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pixmith_core::scripting::access::{Access, reset_script_grants};
use pixmith_desktop::PixmithApp;
use pixmith_desktop::script_access;
use std::path::{Path, PathBuf};

/// An app that answers the permission questions of its own scripts with the real dialog.
fn setup() -> Harness<'static, PixmithApp> {
    let mut h = common::harness(PixmithApp::default());
    script_access::install(&h.ctx);
    h.state_mut().gamedev.lua.enable_permission_dialogs(false);
    h
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pixmith-lua-host-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Lua source text for a path.
fn lua(path: &Path) -> String {
    format!("{:?}", path.to_string_lossy())
}

fn run(h: &mut Harness<'static, PixmithApp>, name: &str, source: &str) {
    assert!(h.state_mut().lua_start_script(name, source));
}

fn asking(app: &PixmithApp) -> bool {
    app.gamedev.lua.asking()
}

fn finished(app: &PixmithApp) -> bool {
    app.gamedev.lua.sessions().next().is_none() && !app.gamedev.lua.asking()
}

/// Waits until the script is over and its notice (output or error) is on screen, if it has one.
fn finish(h: &mut Harness<'static, PixmithApp>) {
    common::settle(h, finished);
    h.run_ok();
}

#[test]
fn reading_a_file_asks_and_deny_is_a_script_error() {
    let dir = scratch("deny");
    let file = dir.join("secret.txt");
    std::fs::write(&file, "hello").unwrap();
    let mut h = setup();
    run(&mut h, "host-deny.lua", &format!("local f = assert(io.open({}, 'r')); print(f:read('a'))", lua(&file)));
    common::settle(&mut h, asking);
    let (script, path, access) = h.state().gamedev.lua.question().unwrap();
    assert_eq!((script.as_str(), access), ("host-deny.lua", Access::Read));
    assert_eq!(Path::new(&path), file.as_path());
    h.get_by_label("Allow Once");
    h.get_by_label("Always Allow for This Script");
    h.get_by_label("Deny").click();
    finish(&mut h);
    let notice = h.state().gamedev.notice.clone().expect("the script failed");
    assert!(notice.title.contains("Script error") && notice.warning, "{notice:?}");
    assert!(notice.text.contains("permission denied"), "{}", notice.text);
    assert!(!notice.text.contains("hello"));
}

#[test]
fn escape_denies() {
    let dir = scratch("escape");
    let file = dir.join("a.txt");
    std::fs::write(&file, "x").unwrap();
    let mut h = setup();
    run(&mut h, "host-escape.lua", &format!("io.open({}, 'r')", lua(&file)));
    common::settle(&mut h, asking);
    h.key_press(eframe::egui::Key::Escape);
    finish(&mut h);
    let notice = h.state().gamedev.notice.clone().expect("the script failed");
    assert!(notice.text.contains("permission denied"), "{}", notice.text);
}

#[test]
fn allow_once_lets_the_script_read_and_asks_again_next_run() {
    let dir = scratch("once");
    let file = dir.join("data.txt");
    std::fs::write(&file, "contents of the file").unwrap();
    let mut h = setup();
    let source = format!("local f = assert(io.open({}, 'r')); print(f:read('a')); f:close()", lua(&file));
    run(&mut h, "host-once.lua", &source);
    common::settle(&mut h, asking);
    h.get_by_label("Allow Once").click();
    finish(&mut h);
    let notice = h.state().gamedev.notice.clone().expect("the script printed");
    assert_eq!(notice.text, "contents of the file");
    h.state_mut().gamedev.notice = None;
    h.run_ok();

    // "Once" does not outlive the run.
    run(&mut h, "host-once.lua", &source);
    common::settle(&mut h, asking);
    h.get_by_label("Deny").click();
    finish(&mut h);
}

#[test]
fn always_allow_is_remembered_for_the_script() {
    let dir = scratch("always");
    let file = dir.join("data.txt");
    std::fs::write(&file, "kept").unwrap();
    reset_script_grants("host-always.lua");
    let mut h = setup();
    let source = format!("local f = assert(io.open({}, 'r')); print(f:read('a')); f:close()", lua(&file));
    run(&mut h, "host-always.lua", &source);
    common::settle(&mut h, asking);
    h.get_by_label("Always Allow for This Script").click();
    finish(&mut h);
    assert_eq!(h.state().gamedev.notice.clone().unwrap().text, "kept");
    h.state_mut().gamedev.notice = None;
    h.run_ok();

    // No question this time.
    run(&mut h, "host-always.lua", &source);
    finish(&mut h);
    assert_eq!(h.state().gamedev.notice.clone().unwrap().text, "kept");
    reset_script_grants("host-always.lua");
}

#[test]
fn writing_asks_for_write_access() {
    let dir = scratch("write");
    let file = dir.join("out.txt");
    let mut h = setup();
    run(&mut h, "host-write.lua", &format!("local f = assert(io.open({}, 'w')); f:write('hi'); f:close()", lua(&file)));
    common::settle(&mut h, asking);
    assert_eq!(h.state().gamedev.lua.question().unwrap().2, Access::Write);
    h.get_by_label("Allow Once").click();
    finish(&mut h);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "hi");
}

#[test]
fn a_script_can_create_a_document() {
    let mut h = setup();
    let before = h.state().tabs.len();
    run(&mut h, "host-new.lua", "local s = Sprite(16, 16); s:newCel(s.layers[1], 1)");
    finish(&mut h);
    let app = h.state();
    assert_eq!(app.tabs.len(), before + 1);
    assert_eq!(app.active, app.tabs.len() - 1, "the new document is the active one");
    let tab = app.tab();
    assert_eq!((tab.doc.width(), tab.doc.height()), (16, 16));
    assert!(tab.doc.path.is_none());
    assert!(tab.doc.is_dirty(), "unsaved: closing asks to save");
    assert!(app.tab_name(app.active).starts_with("Untitled"));
    assert!(app.gamedev.notice.is_none(), "{:?}", app.gamedev.notice);
}

#[test]
fn save_as_writes_the_file_and_the_tab_shows_its_name() {
    let dir = scratch("saveas");
    let file = dir.join("made.aseprite");
    let mut h = setup();
    let before = h.state().tabs.len();
    run(&mut h, "host-saveas.lua", &format!("local s = Sprite(8, 8); s:saveAs({})", lua(&file)));
    common::settle(&mut h, asking);
    let (_, path, access) = h.state().gamedev.lua.question().unwrap();
    assert_eq!((access, Path::new(&path)), (Access::Write, file.as_path()));
    h.get_by_label("Allow Once").click();
    finish(&mut h);
    assert!(file.exists());
    let app = h.state();
    assert_eq!(app.tabs.len(), before + 1);
    assert_eq!(app.tab().doc.path.as_deref(), Some(file.as_path()));
    assert_eq!(app.tab_name(app.active), "made.aseprite");
    assert!(!app.tab().doc.is_dirty(), "what is on disk is what the tab shows");
}

#[test]
fn saving_the_open_document_adopts_the_path_and_is_clean() {
    let dir = scratch("original");
    let file = dir.join("mine.aseprite");
    let mut h = setup();
    run(&mut h, "host-original.lua", &format!("app.activeSprite:saveAs({})", lua(&file)));
    common::settle(&mut h, asking);
    h.get_by_label("Allow Once").click();
    finish(&mut h);
    let app = h.state();
    assert!(file.exists());
    assert_eq!(app.tab().doc.path.as_deref(), Some(file.as_path()));
    assert_eq!(app.tab_name(app.active), "mine.aseprite");
    assert!(!app.tab().doc.is_dirty());
}

#[test]
fn closing_the_open_document_closes_its_tab_without_a_question() {
    let mut h = setup();
    run(&mut h, "host-open.lua", "Sprite(4, 4)");
    finish(&mut h);
    assert_eq!(h.state().tabs.len(), 2);
    // The script runs on the new, active tab and closes it.
    run(&mut h, "host-close.lua", "app.activeSprite:close()");
    finish(&mut h);
    let app = h.state();
    assert_eq!(app.tabs.len(), 1);
    assert!(app.files.prompt.is_none(), "no save question");
}

#[test]
fn the_sprite_filename_is_the_tabs_file() {
    let dir = scratch("filename");
    let file = dir.join("named.aseprite");
    let mut h = setup();
    h.state_mut().tab_mut().doc.save(&file).unwrap();
    run(&mut h, "host-filename.lua", "print(app.activeSprite.filename)");
    finish(&mut h);
    assert_eq!(h.state().gamedev.notice.clone().unwrap().text, file.to_string_lossy());
}
