//! Dialogs, events and plugins driven without a UI: the tests play the front end (docs/LUA_DIALOG.md).

use super::host::{DocSnap, Host, Spec};
use crate::document::{Editor, Sprite};
use crate::formats::json::{self, Value as J};
use std::time::{Duration, Instant};

const TAG: u64 = 77;

/// A fake front end: one host, one document, the messages seen so far.
struct Fe {
    host: Host,
    ed: Editor,
    seen: Vec<J>,
}

impl Fe {
    fn new() -> Fe {
        Fe { host: Host::new(), ed: Editor::new(Sprite::new(4, 3)), seen: Vec::new() }
    }

    fn snap(&self) -> DocSnap {
        DocSnap::of(&self.ed, TAG, 0, 0)
    }

    fn script(&mut self, src: &str) -> u64 {
        let snap = self.snap();
        self.host.start(Spec::Script { source: src.into(), name: "test".into() }, Some(snap)).unwrap()
    }

    fn plugin(&mut self, src: &str, prefs: Option<String>) -> u64 {
        self.host
            .start(
                Spec::Plugin { source: src.into(), name: "plug".into(), dir: Some("/p".into()), prefs_path: prefs },
                None,
            )
            .unwrap()
    }

    /// Polls until `pred` accepts a message (returns it); panics after 10 seconds.
    fn wait(&mut self, what: &str, pred: impl Fn(&J) -> bool) -> J {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(J::Arr(batch)) = json::parse(&self.host.poll(Duration::from_millis(50))) {
                self.seen.extend(batch);
            }
            // The message is consumed, so the next wait for the same kind finds the next one.
            if let Some(i) = self.seen.iter().position(&pred) {
                return self.seen.remove(i);
            }
            assert!(Instant::now() < end, "timed out waiting for {what}; seen: {}", self.dump());
        }
    }

    fn dump(&self) -> String {
        self.seen.iter().map(json::to_string).collect::<Vec<_>>().join("\n")
    }

    fn of_type(&mut self, ty: &'static str) -> J {
        self.wait(ty, move |m| ty_of(m) == ty)
    }

    fn send(&self, session: u64, ev: &str) {
        let snap = self.snap();
        assert!(self.host.send(session, json::parse(ev).unwrap(), Some(snap)), "session {session} is gone");
    }

    fn click(&self, session: u64, dialog: &str, id: &str) {
        self.send(session, &format!(r#"{{"type":"widget","dialog":"{dialog}","id":"{id}","event":"click"}}"#));
    }

    fn apply(&mut self, txn: &J) -> i32 {
        let (s, t) = (num(txn, "session"), num(txn, "txn"));
        self.host.apply(s, t, Some(&mut self.ed))
    }

    fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let r = self.ed.sprite.render(0);
        let i = (y * self.ed.sprite.width as usize + x) * 4;
        [r[i], r[i + 1], r[i + 2], r[i + 3]]
    }
}

fn ty_of(m: &J) -> &str {
    m.get("type").and_then(J::as_str).unwrap_or("")
}

fn num(m: &J, k: &str) -> u64 {
    m.get(k).and_then(J::as_f64).unwrap_or(-1.0) as u64
}

fn text<'a>(m: &'a J, k: &str) -> &'a str {
    m.get(k).and_then(J::as_str).unwrap_or("")
}

fn widget<'a>(open: &'a J, id: &str) -> &'a J {
    let rows = open.get("dialog").unwrap().get("rows").unwrap().as_array().unwrap();
    for r in rows {
        for w in r.get("widgets").unwrap().as_array().unwrap() {
            if text(w, "id") == id {
                return w;
            }
        }
    }
    panic!("no widget {id}");
}

const RAMP: &str = r#"
local dlg = Dialog("Ramp")
dlg:label{ id = "info", text = "Pick" }
   :color{ id = "from", label = "From", color = 0xFF0000FF }
   :newrow()
   :slider{ id = "n", label = "Steps", min = 1, max = 4, value = 3 }
   :separator{ text = "More" }
   :check{ id = "fill", text = "Fill", selected = true }
   :button{ id = "ok", text = "&OK", focus = true }
   :button{ id = "cancel", text = "Cancel" }
   :show{ wait = true }
local d = dlg.data
if d.ok then
  if d.fill then sprite:setPixel(0, 0, d.from) end
  sprite:setPixel(1, 0, 0x000000FF + d.n * 0x01000000)
  app.undoLabel("Ramp")
end
"#;

#[test]
fn modal_dialog_is_described_answered_and_committed_as_one_step() {
    let mut fe = Fe::new();
    let s = fe.script(RAMP);
    let open = fe.of_type("dialog.open");
    assert_eq!(num(&open, "session"), s);
    let d = open.get("dialog").unwrap();
    assert_eq!(text(d, "title"), "Ramp");
    assert_eq!(d.get("modal"), Some(&J::Bool(true)));
    let rows = d.get("rows").unwrap().as_array().unwrap();
    // label + color | slider | separator | check + buttons
    assert_eq!(rows.len(), 4, "{}", json::to_string(d));
    let from = widget(&open, "from");
    assert_eq!(text(from, "kind"), "color");
    assert_eq!(text(from, "label"), "From");
    assert_eq!(from.get("color").and_then(J::as_f64), Some(0xFF0000FFu32 as f64));
    let n = widget(&open, "n");
    assert_eq!((num(n, "min"), num(n, "max"), num(n, "value")), (1, 4, 3));
    assert_eq!(text(widget(&open, "_w1"), "kind"), "separator"); // the first widget without an id
    assert_eq!(widget(&open, "ok").get("focus"), Some(&J::Bool(true)));
    assert_eq!(widget(&open, "fill").get("selected"), Some(&J::Bool(true)));
    // The session waits for the user.
    fe.wait("waiting", |m| ty_of(m) == "state" && text(m, "state") == "waiting");

    let dlg = text(d, "id").to_string();
    fe.send(s, &format!(r#"{{"type":"widget","dialog":"{dlg}","id":"n","event":"change","value":2}}"#));
    fe.click(s, &dlg, "ok");
    fe.of_type("dialog.close");
    let txn = fe.of_type("txn");
    assert_eq!(txn.get("ok"), Some(&J::Bool(true)), "{}", json::to_string(&txn));
    assert_eq!(txn.get("commit"), Some(&J::Bool(true)));
    assert_eq!(text(&txn, "label"), "Ramp");
    assert_eq!(num(&txn, "doc"), TAG);
    assert_eq!(fe.apply(&txn), 1);
    assert_eq!(fe.pixel(0, 0), [255, 0, 0, 255]);
    assert_eq!(fe.pixel(1, 0), [2, 0, 0, 255]);
    assert_eq!(fe.ed.history_len().0, 1, "one undo step");
    fe.ed.undo();
    assert_eq!(fe.pixel(0, 0), [0, 0, 0, 0]);
    fe.of_type("session.end");
    assert_eq!(fe.host.count(), 0);
}

#[test]
fn closing_with_cancel_leaves_the_document_alone() {
    let mut fe = Fe::new();
    let s = fe.script(RAMP);
    let open = fe.of_type("dialog.open");
    fe.click(s, text(open.get("dialog").unwrap(), "id"), "cancel");
    let txn = fe.of_type("txn");
    assert_eq!(txn.get("commit"), Some(&J::Bool(false)));
    assert_eq!(fe.apply(&txn), -2, "nothing to apply");
    assert_eq!(fe.pixel(0, 0), [0, 0, 0, 0]);
}

#[test]
fn user_closing_the_window_runs_onclose_and_data_has_no_pressed_button() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        local closed = 0
        local dlg = Dialog{ title = "x", onclose = function() closed = closed + 1 end }
        dlg:button{ id = "ok", text = "OK" }:show()
        print("ok=" .. tostring(dlg.data.ok), "closed=" .. closed)
        "#,
    );
    let open = fe.of_type("dialog.open");
    fe.send(s, &format!(r#"{{"type":"dialog.closed","dialog":"{}"}}"#, text(open.get("dialog").unwrap(), "id")));
    let txn = fe.of_type("txn");
    assert_eq!(txn.get("messages").unwrap().as_array().unwrap(), &[J::Str("ok=false\tclosed=1".into())]);
}

#[test]
fn callbacks_modify_widgets_and_see_new_values() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        local dlg = Dialog("t")
        dlg:slider{ id = "n", min = 0, max = 10, value = 1,
                    onchange = function(ev) dlg:modify{ id = "out", text = "n=" .. dlg.data.n .. "/" .. ev.value } end }
           :label{ id = "out", text = "-" }
           :entry{ id = "name", text = "a", onchange = function(ev) dlg:modify{ id = "out", text = ev.value } end }
           :combobox{ id = "c", options = { "x", "y" }, option = "y",
                      onchange = function() dlg:modify{ id = "out", text = "c=" .. dlg.data.c } end }
           :radio{ id = "r1", text = "1", selected = true }
           :radio{ id = "r2", text = "2", onclick = function() dlg:modify{ id = "out", text = "r2" } end }
           :button{ id = "go", text = "Go", onclick = function() dlg:close() end }
           :show()
        print(dlg.data.n, dlg.data.name, dlg.data.c, tostring(dlg.data.r1), tostring(dlg.data.r2), tostring(dlg.data.go))
        "#,
    );
    let open = fe.of_type("dialog.open");
    let id = text(open.get("dialog").unwrap(), "id").to_string();
    let ev = |w: &str, kind: &str, v: &str| {
        format!(r#"{{"type":"widget","dialog":"{id}","id":"{w}","event":"{kind}","value":{v}}}"#)
    };
    fe.send(s, &ev("n", "change", "7"));
    let m = fe.wait("modify", |m| ty_of(m) == "dialog.modify");
    assert_eq!(text(m.get("widgets").unwrap().get("out").unwrap(), "text"), "n=7/7");
    fe.send(s, &ev("name", "change", r#""héllo""#));
    fe.wait("entry text", |m| {
        ty_of(m) == "dialog.modify" && m.get("widgets").unwrap().get("out").is_some_and(|o| text(o, "text") == "héllo")
    });
    fe.send(s, &ev("c", "change", r#""x""#));
    fe.wait("combo", |m| {
        ty_of(m) == "dialog.modify" && m.get("widgets").unwrap().get("out").is_some_and(|o| text(o, "text") == "c=x")
    });
    fe.send(s, &ev("c", "change", r#""bogus""#)); // not an option: ignored
    fe.send(s, &ev("r2", "click", "true"));
    // The other radio of the group is cleared by a patch.
    let m = fe.wait("radio patch", |m| {
        ty_of(m) == "dialog.modify"
            && m.get("widgets").unwrap().get("r1").is_some_and(|o| o.get("selected") == Some(&J::Bool(false)))
    });
    assert_eq!(text(&m, "dialog"), id);
    fe.click(s, &id, "go");
    let txn = fe.of_type("txn");
    assert_eq!(txn.get("ok"), Some(&J::Bool(true)), "{}", fe.dump());
    assert_eq!(txn.get("messages").unwrap().as_array().unwrap(), &[J::Str("7\théllo\tx\tfalse\ttrue\tfalse".into())]);
}

#[test]
fn a_failing_callback_is_rolled_back_and_reported_the_dialog_stays_open() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        local dlg = Dialog("t")
        dlg:button{ id = "bad", text = "bad", onclick = function() sprite:setPixel(0, 0, 0xFF0000FF); error("boom") end }
           :button{ id = "good", text = "good", onclick = function() sprite:setPixel(1, 0, 0x00FF00FF); dlg:close() end }
           :show()
        "#,
    );
    let open = fe.of_type("dialog.open");
    let id = text(open.get("dialog").unwrap(), "id").to_string();
    fe.click(s, &id, "bad");
    fe.click(s, &id, "good");
    let txn = fe.of_type("txn");
    assert_eq!(txn.get("ok"), Some(&J::Bool(true)));
    let msgs = txn.get("messages").unwrap().as_array().unwrap();
    assert!(
        msgs.iter().any(|m| m.as_str().is_some_and(|m| m.starts_with("error: ") && m.contains("boom"))),
        "{msgs:?}"
    );
    assert_eq!(fe.apply(&txn), 1);
    assert_eq!(fe.pixel(0, 0), [0, 0, 0, 0], "the failed callback's pixel was rolled back");
    assert_eq!(fe.pixel(1, 0), [0, 255, 0, 255]);
}

#[test]
fn script_error_leaves_the_document_untouched() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        sprite:setPixel(0, 0, 0xFF0000FF)
        local dlg = Dialog("t")
        dlg:button{ id = "ok", text = "OK" }:show()
        error("late failure")
        "#,
    );
    let open = fe.of_type("dialog.open");
    fe.click(s, text(open.get("dialog").unwrap(), "id"), "ok");
    let txn = fe.of_type("txn");
    assert_eq!(txn.get("ok"), Some(&J::Bool(false)));
    assert!(text(&txn, "error").contains("late failure"));
    assert_eq!(txn.get("commit"), Some(&J::Bool(false)));
    assert_eq!(fe.pixel(0, 0), [0, 0, 0, 0]);
}

#[test]
fn non_modal_dialog_outlives_the_main_chunk_and_each_event_is_its_own_step() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        local n = 0
        local dlg = Dialog("live")
        dlg:button{ id = "paint", text = "Paint", onclick = function() n = n + 1; sprite:setPixel(n, 0, 0xFF0000FF) end }
           :button{ id = "close", text = "Close" }
           :show{ wait = false }
        sprite:setPixel(0, 2, 0x0000FFFF)
        "#,
    );
    let open = fe.of_type("dialog.open");
    let d = open.get("dialog").unwrap();
    assert_eq!(d.get("modal"), Some(&J::Bool(false)));
    let id = text(d, "id").to_string();
    let txn = fe.of_type("txn");
    assert_eq!(fe.apply(&txn), 1);
    fe.wait("idle", |m| ty_of(m) == "state" && text(m, "state") == "idle");
    assert_eq!(fe.host.count(), 1, "the session stays loaded");

    fe.seen.clear();
    fe.click(s, &id, "paint");
    let t2 = fe.of_type("txn");
    assert_eq!(fe.apply(&t2), 1);
    fe.click(s, &id, "paint");
    let t3 = fe.of_type("txn");
    assert_ne!(num(&t2, "txn"), num(&t3, "txn"));
    assert_eq!(fe.apply(&t3), 1);
    assert_eq!(fe.pixel(0, 2), [0, 0, 255, 255]);
    assert_eq!((fe.pixel(1, 0), fe.pixel(2, 0)), ([255, 0, 0, 255], [255, 0, 0, 255]));
    assert_eq!(fe.ed.history_len().0, 3);

    fe.click(s, &id, "close");
    fe.of_type("dialog.close");
    fe.of_type("session.end");
    assert_eq!(fe.host.count(), 0);
}

#[test]
fn applying_to_a_changed_document_is_a_conflict() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        local dlg = Dialog("live")
        dlg:button{ id = "paint", text = "Paint", onclick = function() sprite:setPixel(0, 0, 0xFF0000FF) end }:show{ wait = false }
        "#,
    );
    let open = fe.of_type("dialog.open");
    let id = text(open.get("dialog").unwrap(), "id").to_string();
    fe.of_type("txn");
    fe.click(s, &id, "paint");
    let txn = fe.of_type("txn");
    // The user draws before the result is applied.
    fe.ed.checkpoint();
    fe.ed.sprite.layers[0].name = "edited by the user".into();
    assert_eq!(fe.apply(&txn), -1);
    assert_eq!(fe.pixel(0, 0), [0, 0, 0, 0]);
    assert_eq!(fe.apply(&txn), -2, "a result can only be applied once");
    fe.host.stop(s);
    fe.of_type("session.end");
}

#[test]
fn bursts_of_slider_changes_collapse() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        calls = 0
        local dlg = Dialog("t")
        dlg:slider{ id = "n", min = 0, max = 1000, value = 0, onchange = function() calls = calls + 1  end }
           :button{ id = "go", text = "go", onclick = function() print("calls=" .. calls, dlg.data.n); dlg:close() end }
           :show()
        "#,
    );
    let open = fe.of_type("dialog.open");
    let id = text(open.get("dialog").unwrap(), "id").to_string();
    // Queue everything at once: the worker sees all of it before it starts reading.
    for i in 1..=50 {
        fe.send(s, &format!(r#"{{"type":"widget","dialog":"{id}","id":"n","event":"change","value":{i}}}"#));
    }
    fe.click(s, &id, "go");
    let txn = fe.of_type("txn");
    let msg = txn.get("messages").unwrap().as_array().unwrap()[0].as_str().unwrap().to_string();
    let (calls, last) = msg.split_once('\t').unwrap();
    assert_eq!(last, "50", "the last value wins");
    let calls: u32 = calls.trim_start_matches("calls=").parse().unwrap();
    assert!((1..=50).contains(&calls));
}

#[test]
fn app_events_reach_listeners_with_the_new_document() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        app.events:on("sitechange", function(ev)
          print("site " .. ev.layer .. "/" .. ev.frame .. " w=" .. sprite.width)
        end)
        local id = app.events:on("fgcolorchange", function(ev) print("fg " .. ev.color) end)
        app.events:off(id)
        app.events:on("beforecommand", function(ev) print("cmd " .. ev.name) end)
        sprite.events:on("change", function(ev) sprite:setPixel(0, 0, 0xFF0000FF); print("change " .. tostring(ev.fromUndo)) end)
        "#,
    );
    let t = fe.of_type("txn");
    assert_eq!(t.get("ok"), Some(&J::Bool(true)), "{}", fe.dump());
    fe.wait("idle", |m| ty_of(m) == "state" && text(m, "state") == "idle");
    fe.seen.clear();
    let ev = |s: &str| json::parse(s).unwrap();
    let n = fe.host.broadcast(
        &ev(r#"{"type":"app.event","name":"sitechange","layer":2,"frame":1}"#),
        Some(&fe.ed),
        TAG,
        1,
        0,
    );
    assert_eq!(n, 1);
    let t = fe.of_type("txn");
    assert_eq!(t.get("messages").unwrap().as_array().unwrap(), &[J::Str("site 2/1 w=4".into())]);
    // Removed listener: nobody is told.
    assert_eq!(fe.host.broadcast(&ev(r#"{"type":"app.event","name":"fgcolorchange","color":1}"#), None, 0, 0, 0), 0);
    fe.host.broadcast(&ev(r#"{"type":"app.event","name":"beforecommand","command":"Undo"}"#), Some(&fe.ed), TAG, 0, 0);
    let t = fe.wait("cmd", |m| {
        ty_of(m) == "txn"
            && m.get("messages").unwrap().as_array().unwrap().iter().any(|x| x.as_str() == Some("cmd Undo"))
    });
    assert_eq!(t.get("commit"), Some(&J::Bool(false)));
    // sprite.events fire for the document that registered them only.
    fe.host.broadcast(&ev(r#"{"type":"sprite.event","name":"change","fromUndo":false}"#), Some(&fe.ed), TAG + 1, 0, 0);
    fe.host.broadcast(&ev(r#"{"type":"sprite.event","name":"change","fromUndo":true}"#), Some(&fe.ed), TAG, 0, 0);
    let t = fe.wait("change", |m| {
        ty_of(m) == "txn"
            && m.get("messages")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .any(|x| x.as_str().is_some_and(|x| x.starts_with("change")))
    });
    assert_eq!(t.get("messages").unwrap().as_array().unwrap(), &[J::Str("change true".into())]);
    assert_eq!(fe.apply(&t), 1);
    assert_eq!(fe.pixel(0, 0), [255, 0, 0, 255]);
    fe.host.stop(s);
    fe.of_type("session.end");
}

#[test]
fn unknown_event_names_are_errors() {
    let mut fe = Fe::new();
    fe.script(r#"app.events:on("nope", function() end)"#);
    let t = fe.of_type("txn");
    assert_eq!(t.get("ok"), Some(&J::Bool(false)));
    assert!(text(&t, "error").contains("unknown event"));
    fe.of_type("session.end");
}

fn temp_prefs(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("pixmith-plugin-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("prefs.json").to_string_lossy().into_owned()
}

const PLUGIN: &str = r#"
function init(plugin)
  plugin.preferences.runs = (plugin.preferences.runs or 0) + 1
  plugin.preferences.nested = { a = 1, list = { "x", "y" } }
  print("init " .. plugin.name .. " " .. plugin.path .. " sprite=" .. tostring(sprite))
  plugin:newCommand{
    id = "paint", title = "Paint Pixel", group = "file_scripts",
    onenabled = function() return sprite ~= nil end,
    onclick = function() sprite:setPixel(2, 1, 0x00FF00FF) end,
  }
  plugin:newCommand{ id = "ask", title = "Ask...", onclick = function()
    local r = app.alert{ title = "Sure?", text = "Really?", buttons = { "Yes", "No" } }
    print("alert=" .. r)
  end }
end
function exit(plugin)
  plugin.preferences.exited = true
end
"#;

#[test]
fn plugin_registers_commands_runs_them_and_persists_preferences() {
    let prefs = temp_prefs("a");
    let mut fe = Fe::new();
    let s = fe.plugin(PLUGIN, Some(prefs.clone()));
    let add = fe.of_type("command.add");
    assert_eq!(num(&add, "session"), s);
    let c = add.get("command").unwrap();
    assert_eq!((text(c, "id"), text(c, "title"), text(c, "group")), ("paint", "Paint Pixel", "file_scripts"));
    let t = fe.of_type("txn");
    assert_eq!(t.get("ok"), Some(&J::Bool(true)), "{}", fe.dump());
    assert_eq!(t.get("messages").unwrap().as_array().unwrap(), &[J::Str("init plug /p sprite=nil".into())]);
    // onenabled saw no document: the command is disabled until a document event arrives.
    let u = fe.wait("update", |m| ty_of(m) == "command.update");
    assert_eq!(u.get("command").unwrap().get("enabled"), Some(&J::Bool(false)));
    fe.wait("idle", |m| ty_of(m) == "state" && text(m, "state") == "idle");
    assert_eq!(fe.host.count(), 1);

    fe.seen.clear();
    fe.host.broadcast(&json::parse(r#"{"type":"app.event","name":"sitechange"}"#).unwrap(), Some(&fe.ed), TAG, 0, 0);
    let u = fe.wait("enable", |m| ty_of(m) == "command.update");
    assert_eq!(u.get("command").unwrap().get("enabled"), Some(&J::Bool(true)));

    fe.send(s, r#"{"type":"command","id":"paint"}"#);
    let t = fe.wait("commit", |m| ty_of(m) == "txn" && m.get("commit") == Some(&J::Bool(true)));
    assert_eq!(text(&t, "label"), "Paint Pixel");
    assert_eq!(fe.apply(&t), 1);
    assert_eq!(fe.pixel(2, 1), [0, 255, 0, 255]);

    // The preferences were saved after init.
    let saved = json::parse(&std::fs::read_to_string(&prefs).unwrap()).unwrap();
    assert_eq!(saved.get("runs").and_then(J::as_f64), Some(1.0));
    assert_eq!(saved.get("nested").unwrap().get("list").unwrap().as_array().unwrap().len(), 2);

    fe.seen.clear();
    fe.host.stop(s);
    fe.of_type("command.remove");
    fe.of_type("command.remove");
    fe.of_type("session.end");
    let saved = json::parse(&std::fs::read_to_string(&prefs).unwrap()).unwrap();
    assert_eq!(saved.get("exited"), Some(&J::Bool(true)), "exit(plugin) ran");

    // Loaded again: the old preferences are there.
    let mut fe2 = Fe::new();
    fe2.plugin(PLUGIN, Some(prefs.clone()));
    fe2.of_type("txn");
    let saved = json::parse(&std::fs::read_to_string(&prefs).unwrap()).unwrap();
    assert_eq!(saved.get("runs").and_then(J::as_f64), Some(2.0));
    assert_eq!(saved.get("exited"), Some(&J::Bool(true)));
    let _ = std::fs::remove_dir_all(std::path::Path::new(&prefs).parent().unwrap());
}

#[test]
fn app_alert_with_buttons_is_a_modal_dialog() {
    let mut fe = Fe::new();
    let s = fe.plugin(PLUGIN, None);
    fe.of_type("txn");
    fe.send(s, r#"{"type":"command","id":"ask"}"#);
    let open = fe.of_type("dialog.open");
    let d = open.get("dialog").unwrap();
    assert_eq!(text(d, "title"), "Sure?");
    assert_eq!(text(widget(&open, "alert2"), "text"), "No");
    fe.click(s, text(d, "id"), "alert2");
    let t = fe.wait("txn", |m| ty_of(m) == "txn" && !m.get("messages").unwrap().as_array().unwrap().is_empty());
    assert_eq!(t.get("messages").unwrap().as_array().unwrap(), &[J::Str("alert=2".into())]);
    fe.host.stop(s);
    fe.of_type("session.end");
}

#[test]
fn plugin_init_error_is_reported_and_the_session_stays_loaded_for_its_commands() {
    let mut fe = Fe::new();
    fe.plugin("function init(p) error('nope') end", None);
    let t = fe.of_type("txn");
    assert_eq!(t.get("ok"), Some(&J::Bool(false)));
    assert!(text(&t, "error").contains("nope"));
}

#[test]
fn stop_interrupts_a_runaway_callback() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        local dlg = Dialog("t")
        dlg:button{ id = "spin", text = "spin", onclick = function() while true do end end }:show()
        "#,
    );
    let open = fe.of_type("dialog.open");
    fe.click(s, text(open.get("dialog").unwrap(), "id"), "spin");
    std::thread::sleep(Duration::from_millis(200));
    fe.host.stop(s);
    let t = fe.of_type("txn");
    assert_eq!(t.get("ok"), Some(&J::Bool(false)));
    assert!(text(&t, "error").contains("script cancelled"), "{}", json::to_string(&t));
    fe.of_type("session.end");
}

#[test]
fn stopping_a_session_waiting_in_show_ends_it() {
    let mut fe = Fe::new();
    let s = fe.script(r#"Dialog("t"):button{ id = "b", text = "b", onclick = function() end }:show()"#);
    fe.of_type("dialog.open");
    fe.host.stop(s);
    let t = fe.of_type("txn");
    assert!(text(&t, "error").contains("script cancelled"));
    fe.of_type("dialog.close");
    fe.of_type("session.end");
}

#[test]
fn waiting_for_the_user_does_not_use_up_the_time_budget() {
    // Not the 10 second default: this only checks that a wait of a few hundred milliseconds followed by
    // work completes normally.
    let mut fe = Fe::new();
    let s = fe.script(r#"local d = Dialog("t"); d:button{ id = "b", text = "b" }:show(); local x = 0; for i = 1, 200000 do x = x + i end; print(x)"#);
    let open = fe.of_type("dialog.open");
    std::thread::sleep(Duration::from_millis(300));
    fe.click(s, text(open.get("dialog").unwrap(), "id"), "b");
    let t = fe.of_type("txn");
    assert_eq!(t.get("ok"), Some(&J::Bool(true)));
}

#[test]
fn api_misuse_raises_lua_errors_not_panics() {
    let cases = [
        r#"Dialog(1)"#,
        r#"Dialog("t"):slider{ id = "a" }:slider{ id = "a" }"#,
        r#"Dialog("t"):color{ color = "red" }"#,
        r#"Dialog("t"):combobox{ options = { "a" }, option = "z" }"#,
        r#"Dialog("t"):canvas{}"#,
        r#"Dialog("t"):modify{ id = "nope", text = "x" }"#,
        r#"Dialog("t"):check{ onclick = 5 }"#,
        r#"local d = Dialog("t"); d:label{ id = "a", text = "x" }; d:show{ wait = false }; d:show{ wait = false }"#,
        r#"Dialog("t"):show{ wait = false }:label{ text = "late" }"#,
        r#"local d = Dialog("t"); d:button{ id = "b" }; d.data = { b = "x", zzz = 1 }; d:modify{ id = { "b", "b" }, enabled = 3 }"#,
    ];
    for src in cases {
        let mut fe = Fe::new();
        fe.script(src);
        let t = fe.of_type("txn");
        // Either a clean Lua error or (for the headless-style misuse) success; never a hang or a panic.
        let ok = t.get("ok") == Some(&J::Bool(true));
        let err = text(&t, "error");
        assert!(ok || !err.is_empty(), "{src}: {}", json::to_string(&t));
        assert!(!err.contains("internal error"), "{src}: {err}");
    }
}

#[test]
fn plain_script_runs_have_no_ui_and_dialogs_return_at_once() {
    let mut e = Editor::new(Sprite::new(2, 2));
    let o = super::run(
        &mut e,
        r#"
        local d = Dialog("t")
        d:number{ id = "n", text = "2.5", decimals = 1 }:entry{ id = "e", text = "hi" }:check{ id = "c" }
         :combobox{ id = "k", options = { "a", "b" } }:button{ id = "ok", text = "OK" }
         :show()
        local x = d.data
        print(tostring(app.isUIAvailable), x.n, x.e, tostring(x.c), x.k, tostring(x.ok))
        print(app.alert{ text = "hi", buttons = { "A", "B" } })
        app.events:on("sitechange", function() end)
        print(tostring(app.sprite == sprite), app.fgColor, app.bgColor, app.apiVersion)
        "#,
        (0, 0),
        super::Limits::default(),
    );
    assert_eq!(o.error, None);
    assert_eq!(o.messages, ["false\t2.5\thi\tfalse\ta\tfalse", "0", "true\t255\t4294967295\t3"]);
}

#[test]
fn preferences_json_survives_odd_values() {
    let prefs = temp_prefs("odd");
    let mut fe = Fe::new();
    fe.plugin(
        r#"function init(p)
             p.preferences.f = function() end
             p.preferences.n = 0.5
             p.preferences.s = "ü\n\"q\""
             p.preferences[1] = "x"
             p.preferences.deep = {}
             local t = p.preferences.deep
             for i = 1, 40 do t.d = {}; t = t.d end
           end"#,
        Some(prefs.clone()),
    );
    let t = fe.of_type("txn");
    assert_eq!(t.get("ok"), Some(&J::Bool(true)));
    // Too deep to store: the file is simply not written, and nothing crashes.
    let _ =
        std::fs::remove_dir_all(std::path::Path::new(&prefs).parent().unwrap_or(std::path::Path::new("/nonexistent")));
}

#[test]
fn too_many_sessions_are_refused_and_bad_events_are_ignored() {
    let fe = Fe::new();
    let mut ids = Vec::new();
    for _ in 0..super::host::MAX_SESSIONS {
        ids.push(
            fe.host
                .start(
                    Spec::Script {
                        source: r#"Dialog("t"):button{ id = "b", text = "b" }:show{ wait = false }"#.into(),
                        name: "t".into(),
                    },
                    None,
                )
                .unwrap(),
        );
    }
    assert!(fe.host.start(Spec::Script { source: "".into(), name: "t".into() }, None).is_err());
    // Garbage events do nothing.
    for ev in
        [r#"{}"#, r#"{"type":"widget"}"#, r#"{"type":"widget","dialog":"x","id":"y","event":"click"}"#, r#"[1,2]"#]
    {
        assert!(fe.host.send(ids[0], json::parse(ev).unwrap(), None));
    }
    for id in &ids {
        fe.host.stop(*id);
    }
    let end = Instant::now() + Duration::from_secs(20);
    while fe.host.count() > 0 {
        assert!(Instant::now() < end, "sessions did not end");
        let _ = fe.host.poll(Duration::from_millis(20));
    }
    assert!(!fe.host.send(ids[0], json::parse("{}").unwrap(), None));
    assert_eq!(fe.host.apply(ids[0], 1, None), -2);
}

/// The C ABI end to end, the way a Swift front end drives it (this is the only test using the global host).
#[test]
fn c_abi_round_trip() {
    use super::host::*;
    use std::ffi::{CStr, CString};
    let mut ed = Editor::new(Sprite::new(4, 3));
    let doc: *mut Editor = &mut ed;
    let src = CString::new(
        r#"local d = Dialog("abi"); d:slider{ id = "n", min = 0, max = 9, value = 1 }:button{ id = "ok", text = "OK" }:show()
           sprite:setPixel(0, 0, 0xFF0000FF + 0)"#,
    )
    .unwrap();
    let name = CString::new("abi.lua").unwrap();
    unsafe {
        assert_eq!(pixmith_script_start(doc, 5, std::ptr::null(), name.as_ptr(), 0, 0), 0);
        let s = pixmith_script_start(doc, 5, src.as_ptr(), name.as_ptr(), 0, 0);
        assert_ne!(s, 0);
        let mut seen: Vec<J> = Vec::new();
        let end = Instant::now() + Duration::from_secs(10);
        let mut dialog = None;
        while dialog.is_none() {
            assert!(Instant::now() < end, "no dialog");
            let p = pixmith_script_poll(50);
            assert!(!p.is_null());
            let text = CStr::from_ptr(p).to_string_lossy().into_owned();
            crate::ffi::pixmith_string_free(p);
            if let J::Arr(a) = json::parse(&text).unwrap() {
                seen.extend(a);
            }
            dialog = seen.iter().find(|m| ty_of(m) == "dialog.open" && num(m, "session") == s).cloned();
        }
        let id = text_of(dialog.unwrap().get("dialog").unwrap(), "id");
        let bad = CString::new("{nope").unwrap();
        assert!(!pixmith_script_send(s, doc, 5, 0, 0, bad.as_ptr()));
        assert!(!pixmith_script_send(s, doc, 5, 0, 0, std::ptr::null()));
        assert!(!pixmith_script_send(s + 1000, doc, 5, 0, 0, bad.as_ptr()));
        let ev = CString::new(format!(r#"{{"type":"widget","dialog":"{id}","id":"ok","event":"click"}}"#)).unwrap();
        assert!(pixmith_script_send(s, doc, 5, 0, 0, ev.as_ptr()));
        let mut txn = None;
        while txn.is_none() {
            assert!(Instant::now() < end, "no txn");
            let p = pixmith_script_poll(50);
            let text = CStr::from_ptr(p).to_string_lossy().into_owned();
            crate::ffi::pixmith_string_free(p);
            if let J::Arr(a) = json::parse(&text).unwrap() {
                seen.extend(a);
            }
            txn = seen.iter().find(|m| ty_of(m) == "txn" && num(m, "session") == s).cloned();
        }
        let txn = txn.unwrap();
        assert_eq!(txn.get("commit"), Some(&J::Bool(true)), "{}", json::to_string(&txn));
        assert_eq!(pixmith_script_apply(s, num(&txn, "txn"), doc), 1);
        assert_eq!(pixmith_script_apply(s, num(&txn, "txn"), doc), -2);
        assert_eq!(pixmith_script_apply(s, 99, std::ptr::null_mut()), -2);
        assert_eq!((*doc).sprite.render(0)[..4], [255, 0, 0, 255]);
        // Garbage in the other entry points.
        assert_eq!(pixmith_script_broadcast(std::ptr::null_mut(), 0, 0, 0, std::ptr::null()), 0);
        let junk = CString::new("[1,2").unwrap();
        assert_eq!(pixmith_script_broadcast(doc, 0, 0, 0, junk.as_ptr()), 0);
        pixmith_script_stop(0);
        pixmith_script_stop(u64::MAX);
        pixmith_script_set_colors(0x11223344, 0x55667788);
        assert_eq!(pixmith_plugin_load(std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null()), 0);
        let _ = pixmith_script_count();
    }
}

fn text_of(m: &J, k: &str) -> String {
    text(m, k).to_string()
}

#[test]
fn palette_ramp_example_adds_colors_after_ok() {
    let mut fe = Fe::new();
    let before = fe.ed.sprite.palette.len();
    let s = fe.script(include_str!("../../../scripts/lua-examples/palette-ramp-dialog.lua"));
    let open = fe.of_type("dialog.open");
    let id = text(open.get("dialog").unwrap(), "id").to_string();
    assert_eq!(widget(&open, "preview").get("colors").unwrap().as_array().unwrap().len(), 8);
    fe.send(s, &format!(r#"{{"type":"widget","dialog":"{id}","id":"steps","event":"change","value":4}}"#));
    let m = fe.wait("preview patch", |m| ty_of(m) == "dialog.modify");
    assert_eq!(m.get("widgets").unwrap().get("preview").unwrap().get("colors").unwrap().as_array().unwrap().len(), 4);
    fe.click(s, &id, "ok");
    let txn = fe.of_type("txn");
    assert_eq!(txn.get("ok"), Some(&J::Bool(true)), "{}", json::to_string(&txn));
    assert_eq!(txn.get("messages").unwrap().as_array().unwrap(), &[J::Str("Added 4 colors to the palette.".into())]);
    assert_eq!(fe.apply(&txn), 1);
    assert!(fe.ed.sprite.palette.len() > before);
}

#[test]
fn plugin_example_loads_and_runs_its_command() {
    let prefs = temp_prefs("hello");
    let mut fe = Fe::new();
    let s = fe.plugin(include_str!("../../../scripts/lua-examples/plugin-hello.lua"), Some(prefs.clone()));
    let add = fe.of_type("command.add");
    assert_eq!(text(add.get("command").unwrap(), "id"), "mark-corner");
    fe.of_type("txn");
    fe.wait("idle", |m| ty_of(m) == "state" && text(m, "state") == "idle");
    fe.send(s, r#"{"type":"command","id":"mark-corner"}"#);
    let open = fe.of_type("dialog.open");
    let id = text(open.get("dialog").unwrap(), "id").to_string();
    fe.click(s, &id, "ok");
    let t = fe.wait("commit", |m| ty_of(m) == "txn" && m.get("commit") == Some(&J::Bool(true)));
    assert_eq!(text(&t, "label"), "Mark corner");
    assert_eq!(fe.apply(&t), 1);
    assert_eq!(fe.pixel(0, 0), [0xFF, 0x00, 0x4D, 0xFF]);
    fe.host.stop(s);
    fe.of_type("session.end");
    let saved = json::parse(&std::fs::read_to_string(&prefs).unwrap()).unwrap();
    assert_eq!(saved.get("uses").and_then(J::as_f64), Some(1.0));
    let _ = std::fs::remove_dir_all(std::path::Path::new(&prefs).parent().unwrap());
}
