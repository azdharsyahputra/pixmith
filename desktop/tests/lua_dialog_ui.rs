//! Drives the Lua dialog renderer and the Scripts plumbing headlessly (no GPU), against a `FakeHost` that replays
//! the JSON messages of `docs/LUA_DIALOG.md` and records what the front end sends back.

mod common;

use eframe::egui::{self, Event, Key, Modifiers, PointerButton, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pixmith_core::formats::json::{self, Value};
use pixmith_desktop::PixmithApp;
use pixmith_desktop::script_ui::FakeHost;

/// The `dialog.open` example of the contract.
const OPEN: &str = r#"{"type":"dialog.open","session":3,"dialog":{
  "id":"s3.d1","title":"Palette ramp","modal":true,"notitlebar":false,
  "rows":[
    {"widgets":[{"id":"info","kind":"label","text":"Pick two colors","enabled":true,"visible":true}]},
    {"widgets":[{"id":"from","kind":"color","label":"From","color":488714575,"enabled":true,"visible":true,"hexpand":true},
                {"id":"to","kind":"color","label":"To","color":4289790975,"enabled":true,"visible":true,"hexpand":true}]},
    {"widgets":[{"id":"steps","kind":"slider","label":"Steps","min":2,"max":32,"value":8,"enabled":true,"visible":true,"hexpand":true}]},
    {"widgets":[{"id":"_w5","kind":"separator","text":"Options"}]},
    {"widgets":[{"id":"ok","kind":"button","text":"&OK","focus":true,"enabled":true,"visible":true},
                {"id":"cancel","kind":"button","text":"Cancel","enabled":true,"visible":true}]}
  ]}}"#;

fn setup() -> (Harness<'static, PixmithApp>, FakeHost) {
    let host = FakeHost::new();
    let mut app = PixmithApp::default();
    app.gamedev.lua.set_host(Box::new(host.clone()));
    (common::harness(app), host)
}

/// Pushes messages and runs a few frames so they are polled, applied and drawn.
fn feed(h: &mut Harness<'static, PixmithApp>, host: &FakeHost, messages: &[&str]) {
    for m in messages {
        host.push(m);
    }
    for _ in 0..3 {
        h.step();
    }
    h.run_steps(3);
}

fn sent(host: &FakeHost) -> Vec<Value> {
    host.sent().iter().map(|(_, s)| json::parse(s).unwrap()).collect()
}

fn event_is(v: &Value, id: &str, event: &str) -> bool {
    v.get("id").and_then(Value::as_str) == Some(id) && v.get("event").and_then(Value::as_str) == Some(event)
}

fn press(h: &Harness<'static, PixmithApp>, pos: egui::Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
}

fn open_contract(h: &mut Harness<'static, PixmithApp>, host: &FakeHost) {
    feed(
        h,
        host,
        &[
            r#"{"type":"state","session":3,"state":"running"}"#,
            OPEN,
            r#"{"type":"state","session":3,"state":"waiting"}"#,
        ],
    );
}

#[test]
fn the_contract_dialog_renders_every_widget() {
    let (mut h, host) = setup();
    open_contract(&mut h, &host);
    assert!(h.state().gamedev.lua.dialog("s3.d1").is_some());
    assert!(h.state().gamedev.lua.modal_open());
    h.get_by_label("Palette ramp");
    h.get_by_label("OK");
    h.get_by_label("Cancel");
    h.get_by_label_contains("From");
    h.get_by_label_contains("To");
    h.get_by_label("Steps");
    assert!(h.query_by_label("&OK").is_none(), "the mnemonic marker is not shown");
}

#[test]
fn buttons_send_click_events_and_the_dialog_waits_for_the_core_to_close_it() {
    let (mut h, host) = setup();
    open_contract(&mut h, &host);
    h.get_by_label("OK").click();
    h.run_steps(3);
    let events = sent(&host);
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(event_is(&events[0], "ok", "click"));
    assert_eq!(events[0].get("dialog").and_then(Value::as_str), Some("s3.d1"));
    assert_eq!(host.sent()[0].0, 3, "to session 3");
    assert!(h.state().gamedev.lua.dialog("s3.d1").is_some(), "only the core closes it");
    feed(&mut h, &host, &[r#"{"type":"dialog.close","session":3,"dialog":"s3.d1"}"#]);
    assert!(h.state().gamedev.lua.dialog("s3.d1").is_none());
    assert!(h.query_by_label("OK").is_none());
}

#[test]
fn return_presses_the_default_button_and_escape_closes() {
    let (mut h, host) = setup();
    open_contract(&mut h, &host);
    h.key_press(Key::Enter);
    h.run_steps(3);
    let events = sent(&host);
    assert!(event_is(&events[0], "ok", "click"), "{events:?}");

    let (mut h, host) = setup();
    open_contract(&mut h, &host);
    h.key_press(Key::Escape);
    h.run_steps(3);
    let events = sent(&host);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].get("type").and_then(Value::as_str), Some("dialog.closed"));
    assert!(h.state().gamedev.lua.dialog("s3.d1").is_none(), "the dialog is already gone");
}

#[test]
fn a_slider_sends_changes_while_dragging_and_release_at_the_end() {
    let (mut h, host) = setup();
    open_contract(&mut h, &host);
    let rect = h.get_by_label("Steps").rect();
    let y = rect.center().y;
    h.event(Event::PointerMoved(pos2(rect.left() + 2.0, y)));
    h.step();
    press(&h, pos2(rect.left() + 2.0, y), true);
    h.step();
    h.event(Event::PointerMoved(pos2(rect.center().x, y)));
    h.step();
    h.event(Event::PointerMoved(pos2(rect.right() - 1.0, y)));
    h.step();
    press(&h, pos2(rect.right() - 1.0, y), false);
    h.run_steps(3);
    let events = sent(&host);
    let changes: Vec<f64> = events
        .iter()
        .filter(|e| event_is(e, "steps", "change"))
        .map(|e| e.get("value").and_then(Value::as_f64).unwrap())
        .collect();
    assert!(changes.len() >= 2, "{events:?}");
    assert_eq!(*changes.last().unwrap(), 32.0, "dragged to the right end");
    assert!(changes.iter().all(|v| (2.0..=32.0).contains(v)));
    assert!(events.last().is_some_and(|e| event_is(e, "steps", "release")), "{events:?}");
    let steps = h.state().gamedev.lua.dialog("s3.d1").unwrap().widget("steps").unwrap().value;
    assert_eq!(steps, 32.0, "the dialog keeps the user's edit");
}

#[test]
fn modify_overwrites_values_and_the_title() {
    let (mut h, host) = setup();
    open_contract(&mut h, &host);
    feed(
        &mut h,
        &host,
        &[r#"{"type":"dialog.modify","session":3,"dialog":"s3.d1","title":"New title",
              "widgets":{"steps":{"value":12,"max":64},"info":{"text":"Done"},"ok":{"enabled":false}}}"#],
    );
    h.get_by_label("New title");
    h.get_by_label("Done");
    let d = h.state().gamedev.lua.dialog("s3.d1").unwrap();
    assert_eq!((d.widget("steps").unwrap().value, d.widget("steps").unwrap().max), (12.0, 64.0));
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(sent(&host).is_empty(), "a disabled button sends nothing");
}

const FORM: &str = r#"{"type":"dialog.open","session":5,"dialog":{"id":"s5.d1","title":"Form","modal":false,"rows":[
  {"widgets":[{"id":"name","kind":"entry","label":"Name","text":"abc","hexpand":true}]},
  {"widgets":[{"id":"dither","kind":"check","text":"Dither","selected":false}]},
  {"widgets":[{"id":"r1","kind":"radio","text":"One","selected":true,"group":1},
              {"id":"r2","kind":"radio","text":"Two","selected":false,"group":1}]},
  {"widgets":[{"id":"mode","kind":"combobox","label":"Mode","options":["Nearest","Linear","Cubic"],"option":"Linear"}]},
  {"widgets":[{"id":"pal","kind":"shades","label":"Palette","colors":[4278190335,16711935,65535]}]},
  {"widgets":[{"id":"close","kind":"button","text":"Close"}]}
]}}"#;

fn open_form(h: &mut Harness<'static, PixmithApp>, host: &FakeHost) {
    feed(
        h,
        host,
        &[r#"{"type":"state","session":5,"state":"running"}"#, FORM, r#"{"type":"state","session":5,"state":"idle"}"#],
    );
}

#[test]
fn check_radio_entry_combobox_and_shades_send_their_events() {
    let (mut h, host) = setup();
    open_form(&mut h, &host);
    assert!(!h.state().gamedev.lua.modal_open(), "wait=false is a tool window");

    h.get_by_label("Dither").click();
    h.run_steps(3);
    let e = sent(&host);
    assert!(event_is(&e[0], "dither", "click") && e[0].get("value") == Some(&Value::Bool(true)), "{e:?}");

    h.get_by_label("Two").click();
    h.run_steps(3);
    let e = sent(&host);
    assert!(event_is(&e[1], "r2", "click") && e[1].get("value") == Some(&Value::Bool(true)), "{e:?}");
    let d = h.state().gamedev.lua.dialog("s5.d1").unwrap();
    assert!(d.widget("r2").unwrap().selected && !d.widget("r1").unwrap().selected, "exclusive in its group");
    // Clicking the selected radio again sends nothing.
    h.get_by_label("Two").click();
    h.run_steps(3);
    assert_eq!(sent(&host).len(), 2);

    h.get_by_label("Mode").click();
    h.run_steps(3);
    h.get_by_label("Cubic").click();
    h.run_steps(3);
    let e = sent(&host);
    assert!(event_is(&e[2], "mode", "change") && e[2].get("value").and_then(Value::as_str) == Some("Cubic"), "{e:?}");

    h.get_by_label_contains("#FF0000").click();
    h.run_steps(3);
    let e = sent(&host);
    assert!(event_is(&e[3], "pal", "click"), "{e:?}");
    assert_eq!(e[3].get("color").and_then(Value::as_f64), Some(4278190335.0));
}

#[test]
fn an_entry_sends_every_text_change() {
    let (mut h, host) = setup();
    open_form(&mut h, &host);
    h.get_all_by_value("abc").next().unwrap().click();
    h.run_steps(3);
    h.get_all_by_value("abc").next().unwrap().type_text("de");
    h.run_steps(3);
    let e = sent(&host);
    let values: Vec<&str> =
        e.iter().filter(|x| event_is(x, "name", "change")).filter_map(|x| x.get("value")?.as_str()).collect();
    assert_eq!(values.last().copied(), Some("abcde"), "{e:?}");
    assert_eq!(h.state().gamedev.lua.dialog("s5.d1").unwrap().widget("name").unwrap().text, "abcde");
}

#[test]
fn the_color_popup_sets_a_typed_hex_color() {
    let (mut h, host) = setup();
    open_contract(&mut h, &host);
    h.get_by_label_contains("From").click();
    h.run_steps(3);
    // The popup has the picker and a hex field; the dialog behind has no text field.
    h.get_all_by_value("#1D21314F").next().unwrap().click();
    h.run_steps(3);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(3);
    h.get_all_by_value("#1D21314F").next().unwrap().type_text("FF8800");
    h.key_press(Key::Enter);
    h.run_steps(3);
    let e = sent(&host);
    let change = e.iter().find(|x| event_is(x, "from", "change")).unwrap_or_else(|| panic!("{e:?}"));
    assert_eq!(change.get("value").and_then(Value::as_f64), Some(f64::from(0xFF8800FFu32)));
    assert_eq!(h.state().gamedev.lua.dialog("s3.d1").unwrap().widget("from").unwrap().color, 0xFF8800FF);
}

#[test]
fn the_title_bar_close_button_sends_dialog_closed_and_session_end_removes_dialogs() {
    let (mut h, host) = setup();
    open_form(&mut h, &host);
    h.get_by_label("Form");
    // Two "Close": the title bar's X above and the dialog's button below.
    let mut closers = h.get_all_by_label("Close").map(|n| n.rect()).collect::<Vec<_>>();
    assert_eq!(closers.len(), 2);
    closers.sort_by(|a, b| a.top().total_cmp(&b.top()));
    for (i, rect) in [closers[1], closers[0]].into_iter().enumerate() {
        let target = rect.center();
        h.event(Event::PointerMoved(target));
        h.step();
        press(&h, target, true);
        h.step();
        press(&h, target, false);
        h.run_steps(3);
        let e = sent(&host);
        if i == 0 {
            assert!(event_is(&e[0], "close", "click"), "{e:?}");
        }
    }
    let e = sent(&host);
    assert_eq!(e.last().unwrap().get("type").and_then(Value::as_str), Some("dialog.closed"), "{e:?}");
    assert!(h.state().gamedev.lua.dialog("s5.d1").is_none(), "the dialog is already gone");
    open_form(&mut h, &host);
    feed(&mut h, &host, &[r#"{"type":"session.end","session":5}"#]);
    assert_eq!(h.state().gamedev.lua.dialogs().count(), 0);
}

#[test]
fn a_txn_with_a_result_is_applied_once_and_selects_the_site() {
    let (mut h, host) = setup();
    host.0.lock().unwrap().apply_result = 1;
    let tag = h.state().tab().doc.raw() as usize;
    feed(
        &mut h,
        &host,
        &[&format!(
            r#"{{"type":"txn","session":7,"txn":4,"doc":{tag},"ok":true,"error":null,"messages":["hello"],"commit":true,"label":"Ramp","select":{{"layer":1,"frame":1}}}}"#
        )],
    );
    assert_eq!(host.0.lock().unwrap().applied, vec![(7, 4, true)]);
    assert_eq!(
        h.state().gamedev.notice.as_ref().map(|n| (n.title.as_str(), n.text.as_str())),
        Some(("script", "hello"))
    );
}

#[test]
fn a_txn_for_a_closed_document_is_discarded_and_a_conflict_is_reported() {
    let (mut h, host) = setup();
    feed(
        &mut h,
        &host,
        &[
            r#"{"type":"txn","session":7,"txn":1,"doc":12345,"ok":true,"error":null,"messages":[],"commit":true,"label":"x"}"#,
        ],
    );
    assert_eq!(host.0.lock().unwrap().applied, vec![(7, 1, false)], "applied with no document: discarded");

    let tag = h.state().tab().doc.raw() as usize;
    host.0.lock().unwrap().apply_result = -1;
    feed(
        &mut h,
        &host,
        &[&format!(
            r#"{{"type":"txn","session":7,"txn":2,"doc":{tag},"ok":true,"error":null,"messages":[],"commit":true,"label":"x"}}"#
        )],
    );
    let n = h.state().gamedev.notice.clone().expect("conflict notice");
    assert!(n.text.contains("changed while the script was running") && n.warning, "{}", n.text);
}

#[test]
fn a_failed_txn_shows_the_error_and_a_cancelled_one_stays_quiet() {
    let (mut h, host) = setup();
    feed(
        &mut h,
        &host,
        &[
            r#"{"type":"txn","session":7,"txn":1,"doc":0,"ok":false,"error":"boom","messages":["before"],"commit":false}"#,
        ],
    );
    let n = h.state().gamedev.notice.clone().expect("error notice");
    assert_eq!(n.title, "Script error in script");
    assert!(n.text.contains("boom") && n.text.contains("before") && n.text.contains("The document was not changed."));
    assert!(host.0.lock().unwrap().applied.is_empty(), "nothing to apply");

    let (mut h, host) = setup();
    feed(
        &mut h,
        &host,
        &[
            r#"{"type":"txn","session":7,"txn":1,"doc":0,"ok":false,"error":"script cancelled","messages":[],"commit":false}"#,
        ],
    );
    assert!(h.state().gamedev.notice.is_none());
}

#[test]
fn plugin_commands_fill_the_scripts_menu_and_run_on_click() {
    let (mut h, host) = setup();
    feed(
        &mut h,
        &host,
        &[
            r#"{"type":"state","session":4,"state":"running"}"#,
            r#"{"type":"command.add","session":4,"plugin":"my-plugin","command":{"id":"palette_ramp","title":"Palette Ramp...","group":"file_scripts","enabled":true,"checked":false}}"#,
            r#"{"type":"command.add","session":4,"plugin":"my-plugin","command":{"id":"other","title":"Other","enabled":false,"checked":false}}"#,
            r#"{"type":"state","session":4,"state":"idle"}"#,
        ],
    );
    assert_eq!(h.state().gamedev.lua.commands().len(), 2);
    h.get_by_label("File").click();
    h.run_steps(3);
    h.get_by_label_contains("Scripts").click();
    h.run_steps(3);
    h.get_by_label("Palette Ramp...").click();
    h.run_steps(3);
    // (An idle plugin also hears a no-op `refresh` so it can re-evaluate onenabled for the active document.)
    let commands: Vec<(u64, Value)> = host
        .sent()
        .into_iter()
        .map(|(s, j)| (s, json::parse(&j).unwrap()))
        .filter(|(_, v)| v.get("type").and_then(Value::as_str) == Some("command"))
        .collect();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].0, 4);
    assert_eq!(commands[0].1.get("id").and_then(Value::as_str), Some("palette_ramp"));

    feed(
        &mut h,
        &host,
        &[
            r#"{"type":"command.update","session":4,"command":{"id":"palette_ramp","enabled":false}}"#,
            r#"{"type":"command.remove","session":4,"id":"other"}"#,
        ],
    );
    assert_eq!(h.state().gamedev.lua.commands().len(), 1);
    assert!(!h.state().gamedev.lua.commands()[0].enabled);
    feed(&mut h, &host, &[r#"{"type":"session.end","session":4}"#]);
    assert!(h.state().gamedev.lua.commands().is_empty(), "a plugin's commands go with its session");
}

#[test]
fn app_set_changes_the_drawing_colors_without_echoing_an_event() {
    let (mut h, host) = setup();
    feed(&mut h, &host, &[r#"{"type":"app.set","session":3,"fgColor":4278190335,"bgColor":255}"#]);
    assert_eq!((h.state().primary, h.state().secondary), (0xFF0000FF, 0x000000FF));
    assert_eq!(host.0.lock().unwrap().colors, Some((0xFF0000FF, 0x000000FF)));
    assert!(host.0.lock().unwrap().broadcasts.is_empty());
}

#[test]
fn a_running_session_locks_the_document_and_shows_a_cancel_sheet_after_a_while() {
    let (mut h, host) = setup();
    feed(&mut h, &host, &[r#"{"type":"state","session":9,"state":"running"}"#]);
    assert!(h.state().job_running(), "a script that is running is a busy job");
    std::thread::sleep(std::time::Duration::from_millis(350));
    h.step();
    h.run_steps(3);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(host.0.lock().unwrap().stopped, vec![9]);
    feed(&mut h, &host, &[r#"{"type":"state","session":9,"state":"idle"}"#]);
    assert!(!h.state().job_running());
}

fn broadcasts(host: &FakeHost) -> Vec<Value> {
    host.0.lock().unwrap().broadcasts.iter().map(|s| json::parse(s).unwrap()).collect()
}

fn name_is(v: &Value, name: &str) -> bool {
    v.get("name").and_then(Value::as_str) == Some(name)
}

#[test]
fn nothing_is_broadcast_while_no_script_is_alive() {
    let (mut h, host) = setup();
    h.step();
    h.state_mut().primary = 0x112233FF;
    h.state_mut().tab_mut().frame = 0;
    h.step();
    h.step();
    assert!(host.0.lock().unwrap().broadcasts.is_empty());
    assert_eq!(host.0.lock().unwrap().colors, Some((0x112233FF, 0xFFFFFFFF)), "the colors are still reported");
}

#[test]
fn site_and_color_changes_are_broadcast_to_the_scripts() {
    let (mut h, host) = setup();
    host.0.lock().unwrap().live = 1;
    h.step();
    h.step();
    assert!(broadcasts(&host).is_empty(), "the first frame only learns the state");

    h.state_mut().tab_mut().doc.add_frame(0, false);
    h.state_mut().tab_mut().frame = 1;
    h.step();
    h.step();
    let b = broadcasts(&host);
    let site = b.iter().find(|e| name_is(e, "sitechange")).unwrap_or_else(|| panic!("{b:?}"));
    assert_eq!(site.get("type").and_then(Value::as_str), Some("app.event"));
    assert_eq!(
        (site.get("layer").and_then(Value::as_f64), site.get("frame").and_then(Value::as_f64)),
        (Some(1.0), Some(2.0)),
        "1-based"
    );

    h.state_mut().primary = 0xFF0000FF;
    h.state_mut().secondary = 0x00FF00FF;
    h.step();
    h.step();
    let b = broadcasts(&host);
    let fg = b.iter().find(|e| name_is(e, "fgcolorchange")).unwrap();
    assert_eq!(fg.get("color").and_then(Value::as_f64), Some(f64::from(0xFF0000FFu32)));
    assert!(b.iter().any(|e| name_is(e, "bgcolorchange")));
    assert_eq!(host.0.lock().unwrap().colors, Some((0xFF0000FF, 0x00FF00FF)));
}

#[test]
fn menu_commands_are_announced_before_and_after_and_undo_marks_sprite_change() {
    let (mut h, host) = setup();
    host.0.lock().unwrap().live = 1;
    {
        let tab = h.state_mut().tab_mut();
        tab.doc.begin_edit("Pencil");
        tab.doc.stroke_begin(0, 0, 1, 1, 0xFF0000FF, 1, 0);
        tab.doc.stroke_end();
        tab.doc.touch();
        tab.changed();
    }
    h.step();
    h.step();
    host.0.lock().unwrap().broadcasts.clear();
    h.get_by_label("Edit").click();
    h.run_steps(3);
    h.get_by_label_contains("Undo").click();
    h.run_steps(3);
    let b = broadcasts(&host);
    let names: Vec<String> = b
        .iter()
        .map(|e| {
            format!(
                "{}:{}",
                e.get("name").and_then(Value::as_str).unwrap_or(""),
                e.get("command").and_then(Value::as_str).unwrap_or("")
            )
        })
        .collect();
    assert_eq!(names[0], "beforecommand:Undo", "{names:?}");
    assert_eq!(names[1], "aftercommand:Undo", "{names:?}");
    let change = b.iter().find(|e| name_is(e, "change")).unwrap_or_else(|| panic!("{names:?}"));
    assert_eq!(change.get("type").and_then(Value::as_str), Some("sprite.event"));
    assert_eq!(change.get("fromUndo"), Some(&Value::Bool(true)));
}

#[test]
fn an_edit_is_a_sprite_change_but_applying_a_script_result_is_not() {
    let (mut h, host) = setup();
    host.0.lock().unwrap().live = 1;
    h.step();
    h.step();
    host.0.lock().unwrap().broadcasts.clear();
    h.state_mut().tab_mut().changed();
    h.step();
    h.step();
    let b = broadcasts(&host);
    let change = b.iter().find(|e| name_is(e, "change")).unwrap_or_else(|| panic!("{b:?}"));
    assert_eq!(change.get("fromUndo"), Some(&Value::Bool(false)));

    host.0.lock().unwrap().broadcasts.clear();
    host.0.lock().unwrap().apply_result = 1;
    let tag = h.state().tab().doc.raw() as usize;
    feed(
        &mut h,
        &host,
        &[&format!(
            r#"{{"type":"txn","session":7,"txn":1,"doc":{tag},"ok":true,"error":null,"messages":[],"commit":true,"label":"x"}}"#
        )],
    );
    h.step();
    assert!(
        broadcasts(&host).iter().all(|e| !name_is(e, "change")),
        "a script result must not trigger its own handlers"
    );
}
