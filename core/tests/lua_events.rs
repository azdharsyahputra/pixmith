//! Lua events (`app.events`, `sprite.events`) and the plugin lifecycle (`init`/`exit`, `plugin:newCommand`,
//! `plugin.preferences`), driven the way a front end drives them (docs/LUA_DIALOG.md).
//!
//! The tests play the front end: they start sessions on a private `Host`, poll its JSON messages, send
//! `command` / `app.event` / `sprite.event` messages and apply the committed results to a real `Editor`.

use pixmith_core::document::{Editor, Sprite};
use pixmith_core::formats::json::{self, Value as J};
use pixmith_core::scripting::host::{DocSnap, Host, Spec};
use pixmith_core::scripting::Limits;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const TAG: u64 = 77;

// MARK: A fake front end

struct Fe {
    host: Host,
    ed: Editor,
    seen: Vec<J>,
}

impl Fe {
    fn new() -> Fe {
        Fe::with_host(Host::new())
    }

    fn with_limits(limits: Limits) -> Fe {
        Fe::with_host(Host::with_limits(limits))
    }

    fn with_host(host: Host) -> Fe {
        // 4x3 pixels, 3 layers, 2 frames: room for sites to change to.
        let mut sprite = Sprite::new(4, 3);
        sprite.add_layer(0);
        sprite.add_layer(0);
        sprite.add_frame(0, false);
        Fe { host, ed: Editor::new(sprite), seen: Vec::new() }
    }

    fn snap(&self, tag: u64) -> DocSnap {
        DocSnap::of(&self.ed, tag, 0, 0)
    }

    fn script(&mut self, src: &str) -> u64 {
        self.script_on(src, TAG)
    }

    fn script_on(&mut self, src: &str, tag: u64) -> u64 {
        let snap = self.snap(tag);
        self.host.start(Spec::Script { source: src.into(), name: "test".into() }, Some(snap)).unwrap()
    }

    fn plugin(&mut self, src: &str, prefs: Option<&PathBuf>) -> u64 {
        self.host
            .start(
                Spec::Plugin {
                    source: src.into(),
                    name: "plug".into(),
                    dir: Some("/plugins/plug".into()),
                    prefs_path: prefs.map(|p| p.to_string_lossy().into_owned()),
                },
                None,
            )
            .unwrap()
    }

    /// Pulls queued messages into `seen`.
    fn pump(&mut self) {
        if let Ok(J::Arr(batch)) = json::parse(&self.host.poll(Duration::from_millis(20))) {
            self.seen.extend(batch);
        }
    }

    /// Waits for a message accepted by `pred` (the oldest one that matches, whatever came before it), removes it
    /// from `seen` and returns it; panics after 20 seconds.
    fn wait(&mut self, what: &str, pred: impl Fn(&J) -> bool) -> J {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(i) = self.seen.iter().position(&pred) {
                return self.seen.remove(i);
            }
            assert!(Instant::now() < end, "timed out waiting for {what}; seen:\n{}", self.dump());
            self.pump();
        }
    }

    fn dump(&self) -> String {
        self.seen.iter().map(json::to_string).collect::<Vec<_>>().join("\n")
    }

    fn of_type(&mut self, ty: &'static str) -> J {
        self.wait(ty, move |m| ty_of(m) == ty)
    }

    fn txn(&mut self) -> J {
        self.of_type("txn")
    }

    fn idle(&mut self) {
        self.wait("idle", |m| ty_of(m) == "state" && text(m, "state") == "idle");
    }

    /// Forgets everything seen so far (also what is already queued).
    fn clear(&mut self) {
        std::thread::sleep(Duration::from_millis(30));
        self.pump();
        self.seen.clear();
    }

    /// Nothing but `state` messages arrives within `ms` (used to prove that an event was NOT delivered).
    fn quiet(&mut self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
        self.pump();
        let other: Vec<String> = self.seen.iter().filter(|m| ty_of(m) != "state").map(json::to_string).collect();
        assert!(other.is_empty(), "unexpected messages:\n{}", other.join("\n"));
    }

    fn send(&self, session: u64, ev: &str) {
        let snap = self.snap(TAG);
        assert!(self.host.send(session, json::parse(ev).unwrap(), Some(snap)), "session {session} is gone");
    }

    fn command(&self, session: u64, id: &str) {
        self.send(session, &format!(r#"{{"type":"command","id":"{id}"}}"#));
    }

    fn broadcast(&self, ev: &str) -> u32 {
        self.broadcast_on(ev, TAG)
    }

    fn broadcast_on(&self, ev: &str, tag: u64) -> u32 {
        self.host.broadcast(&json::parse(ev).unwrap(), Some(&self.ed), tag, 0, 0)
    }

    fn apply(&mut self, txn: &J) -> i32 {
        self.host.apply(num(txn, "session"), num(txn, "txn"), Some(&mut self.ed))
    }

    fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let r = self.ed.sprite.render(0);
        let i = (y * self.ed.sprite.width as usize + x) * 4;
        [r[i], r[i + 1], r[i + 2], r[i + 3]]
    }

    fn end(&mut self, session: u64) {
        self.host.stop(session);
        self.wait("session.end", |m| ty_of(m) == "session.end" && num(m, "session") == session);
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

fn msgs(txn: &J) -> Vec<String> {
    txn.get("messages")
        .and_then(J::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

fn ok(txn: &J) -> bool {
    txn.get("ok") == Some(&J::Bool(true))
}

fn committed(txn: &J) -> bool {
    txn.get("commit") == Some(&J::Bool(true))
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pixmith-lua-events-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn read_prefs(path: &Path) -> J {
    json::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn tight() -> Limits {
    Limits { instructions: 2_000_000, time: Duration::from_millis(400), memory: 16 << 20 }
}

// MARK: app.events

#[test]
fn listener_ids_grow_and_off_takes_an_id_or_the_function() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        local a = app.events:on("sitechange", function() print("a") end)
        local f = function() print("f") end
        local b = app.events:on("sitechange", f)
        local c = app.events:on("fgcolorchange", f)
        print(math.type(a), a < b, b < c)
        app.events:off(a)
        app.events:off(12345)      -- unknown ids are ignored
        "#,
    );
    let t = fe.txn();
    assert!(ok(&t), "{}", fe.dump());
    assert_eq!(msgs(&t), ["integer\ttrue\ttrue"]);
    fe.idle();
    fe.clear();

    assert_eq!(fe.broadcast(r#"{"type":"app.event","name":"sitechange"}"#), 1);
    assert_eq!(msgs(&fe.txn()), ["f"], "a was removed by id");
    assert_eq!(fe.broadcast(r#"{"type":"app.event","name":"fgcolorchange","color":5}"#), 1);
    assert_eq!(msgs(&fe.txn()), ["f"]);
    fe.end(s);

    // Off by function removes it from every event of that object.
    let s = fe.script(
        r#"
        local f = function() print("f") end
        app.events:on("sitechange", f)
        app.events:on("fgcolorchange", f)
        app.events:on("bgcolorchange", function() print("bg") end)
        app.events:off(f)
        "#,
    );
    fe.idle();
    fe.clear();
    assert_eq!(fe.broadcast(r#"{"type":"app.event","name":"sitechange"}"#), 0);
    assert_eq!(fe.broadcast(r#"{"type":"app.event","name":"fgcolorchange","color":5}"#), 0);
    assert_eq!(fe.broadcast(r#"{"type":"app.event","name":"bgcolorchange","color":5}"#), 1);
    assert_eq!(msgs(&fe.txn()), ["bg"]);
    fe.end(s);
}

#[test]
fn listeners_run_in_registration_order_with_the_event_fields() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        for i = 1, 4 do
          app.events:on("sitechange", function(ev) print(i, ev.layer, ev.frame, app.activeLayer.stackIndex, app.activeFrame.frameNumber) end)
        end
        "#,
    );
    fe.idle();
    fe.clear();
    let n = fe.host.broadcast(
        &json::parse(r#"{"type":"app.event","name":"sitechange","layer":3,"frame":2}"#).unwrap(),
        Some(&fe.ed),
        TAG,
        2,
        1,
    );
    assert_eq!(n, 1);
    let t = fe.txn();
    assert!(ok(&t));
    assert_eq!(msgs(&t), ["1\t3\t2\t3\t2", "2\t3\t2\t3\t2", "3\t3\t2\t3\t2", "4\t3\t2\t3\t2"]);
    fe.end(s);
}

#[test]
fn every_app_event_reaches_its_listeners_with_its_fields() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        local function show(label) return function(ev)
          print(label, tostring(ev.color), tostring(ev.name), tostring(ev.type), tostring(ev.command))
        end end
        app.events:on("fgcolorchange", show("fg"))
        app.events:on("bgcolorchange", show("bg"))
        app.events:on("beforecommand", show("before"))
        app.events:on("aftercommand", show("after"))
        app.events:on("sitechange", show("site"))
        "#,
    );
    fe.idle();
    fe.clear();
    let one = |fe: &mut Fe, ev: &str| {
        assert_eq!(fe.broadcast(ev), 1, "{ev}");
        let t = fe.txn();
        assert!(ok(&t), "{}", fe.dump());
        msgs(&t)
    };
    assert_eq!(
        one(&mut fe, r#"{"type":"app.event","name":"fgcolorchange","color":4278190335}"#),
        ["fg\t4278190335\tnil\tnil\tnil"]
    );
    assert_eq!(one(&mut fe, r#"{"type":"app.event","name":"bgcolorchange","color":255}"#), ["bg\t255\tnil\tnil\tnil"]);
    assert_eq!(
        one(&mut fe, r#"{"type":"app.event","name":"beforecommand","command":"Flip Horizontal"}"#),
        ["before\tnil\tFlip Horizontal\tnil\tnil"]
    );
    assert_eq!(
        one(&mut fe, r#"{"type":"app.event","name":"aftercommand","command":"Undo"}"#),
        ["after\tnil\tUndo\tnil\tnil"]
    );
    assert_eq!(
        one(&mut fe, r#"{"type":"app.event","name":"sitechange","layer":1,"frame":1}"#),
        ["site\tnil\tnil\tnil\tnil"]
    );
    fe.end(s);
}

#[test]
fn events_nobody_listens_for_are_not_delivered() {
    let mut fe = Fe::new();
    let s = fe.script(r#"app.events:on("sitechange", function() print("site") end)"#);
    fe.idle();
    fe.clear();
    // Other event, unknown event, wrong type, garbage: nothing is queued for the session.
    for ev in [
        r#"{"type":"app.event","name":"fgcolorchange","color":1}"#,
        r#"{"type":"app.event","name":"nonsense"}"#,
        r#"{"type":"sprite.event","name":"change","fromUndo":false}"#,
        r#"{"type":"widget","name":"sitechange"}"#,
        r#"{}"#,
        r#"[1]"#,
    ] {
        assert_eq!(fe.broadcast(ev), 0, "{ev}");
    }
    fe.quiet(150);
    fe.end(s);
}

#[test]
fn sprite_change_events_carry_from_undo_and_belong_to_one_document() {
    let mut fe = Fe::new();
    let a = fe.script_on(
        r#"sprite.events:on("change", function(ev) print("A change", tostring(ev.fromUndo), sprite.width) end)
           sprite.events:on("filenamechange", function(ev) print("A rename") end)"#,
        TAG,
    );
    let b = fe
        .script_on(r#"sprite.events:on("change", function(ev) print("B change", tostring(ev.fromUndo)) end)"#, TAG + 1);
    fe.idle();
    fe.idle();
    fe.clear();

    assert_eq!(
        fe.broadcast_on(r#"{"type":"sprite.event","name":"change","fromUndo":false}"#, TAG),
        2,
        "both sessions are told"
    );
    let t = fe.wait("A", |m| ty_of(m) == "txn" && num(m, "session") == a);
    assert_eq!(msgs(&t), ["A change\tfalse\t4"]);
    // B listens for a different document: its handler does not run.
    let tb = fe.wait("B txn", |m| ty_of(m) == "txn" && num(m, "session") == b);
    assert!(msgs(&tb).is_empty(), "{tb:?}");

    assert_eq!(fe.broadcast_on(r#"{"type":"sprite.event","name":"change","fromUndo":true}"#, TAG + 1), 2);
    let tb = fe.wait("B", |m| ty_of(m) == "txn" && num(m, "session") == b);
    assert_eq!(msgs(&tb), ["B change\ttrue"]);
    let ta = fe.wait("A txn", |m| ty_of(m) == "txn" && num(m, "session") == a);
    assert!(msgs(&ta).is_empty());

    assert_eq!(fe.broadcast_on(r#"{"type":"sprite.event","name":"filenamechange"}"#, TAG), 1);
    let t = fe.wait("rename", |m| ty_of(m) == "txn" && num(m, "session") == a);
    assert_eq!(msgs(&t), ["A rename"]);
    fe.end(a);
    fe.end(b);
}

#[test]
fn a_sprite_listener_registered_in_a_handler_follows_that_document() {
    // The usual Aseprite pattern: listen to the site, then to the sprite of the current site.
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        local token
        app.events:on("sitechange", function()
          if token then sprite.events:off(token) end
          token = sprite.events:on("change", function(ev) print("changed " .. tostring(ev.fromUndo)) end)
        end)
        "#,
    );
    fe.idle();
    fe.clear();
    // The script itself had a document, so its tag is TAG: a site change with another tag re-registers.
    let n = fe.host.broadcast(
        &json::parse(r#"{"type":"app.event","name":"sitechange"}"#).unwrap(),
        Some(&fe.ed),
        500,
        0,
        0,
    );
    assert_eq!(n, 1);
    assert!(ok(&fe.txn()));
    fe.clear();
    assert_eq!(fe.broadcast_on(r#"{"type":"sprite.event","name":"change","fromUndo":true}"#, 500), 1);
    assert_eq!(msgs(&fe.txn()), ["changed true"]);
    // Edits of the first document are not what the listener registered for.
    assert_eq!(fe.broadcast_on(r#"{"type":"sprite.event","name":"change","fromUndo":false}"#, TAG), 1);
    assert!(msgs(&fe.txn()).is_empty());
    fe.end(s);
}

#[test]
fn handlers_edits_are_one_undo_step_each_and_failed_ones_are_rolled_back() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        local n = 0
        app.events:on("sitechange", function()
          n = n + 1
          sprite:setPixel(n, 0, 0xFF0000FF)
          app.undoLabel("Site " .. n)
        end)
        "#,
    );
    fe.idle();
    fe.clear();
    for i in 1..=3u32 {
        fe.broadcast(r#"{"type":"app.event","name":"sitechange"}"#);
        let t = fe.txn();
        assert!(committed(&t), "{}", fe.dump());
        assert_eq!(text(&t, "label"), format!("Site {i}"));
        assert_eq!(num(&t, "doc"), TAG);
        assert_eq!(fe.apply(&t), 1);
        // The front end snapshot comes from the document, so it includes the earlier steps.
    }
    assert_eq!(fe.ed.history_len().0, 3);
    assert_eq!([fe.pixel(1, 0), fe.pixel(2, 0), fe.pixel(3, 0)], [[255, 0, 0, 255]; 3]);
    fe.ed.undo();
    assert_eq!(fe.pixel(3, 0), [0, 0, 0, 0]);
    fe.end(s);
}

#[test]
fn an_error_in_one_handler_is_reported_rolled_back_and_does_not_stop_the_others() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        app.events:on("sitechange", function() sprite:setPixel(0, 0, 0xFF0000FF); error("boom") end)
        app.events:on("sitechange", function() sprite:setPixel(1, 0, 0x00FF00FF); print("second ran") end)
        "#,
    );
    fe.idle();
    fe.clear();
    for _ in 0..2 {
        fe.broadcast(r#"{"type":"app.event","name":"sitechange"}"#);
        let t = fe.txn();
        let m = msgs(&t);
        assert!(m.iter().any(|x| x.starts_with("error: ") && x.contains("boom")), "{m:?}");
        assert!(m.iter().any(|x| x == "second ran"), "{m:?}");
        assert!(committed(&t));
        assert_eq!(fe.apply(&t), 1);
        // Remove the green pixel again so that the next round commits again.
        fe.ed.undo();
    }
    assert_eq!(fe.pixel(0, 0), [0, 0, 0, 0], "the failing handler's pixel never got in");
    fe.host.broadcast(&json::parse(r#"{"type":"app.event","name":"sitechange"}"#).unwrap(), Some(&fe.ed), TAG, 0, 0);
    let t = fe.txn();
    assert!(ok(&t), "the listener is still registered and the session alive");
    fe.end(s);
}

#[test]
fn a_handler_that_fails_alone_leaves_the_document_alone() {
    let mut fe = Fe::new();
    let s = fe.script(r#"app.events:on("fgcolorchange", function(ev) sprite:setPixel(0, 0, ev.color); nope() end)"#);
    fe.idle();
    fe.clear();
    fe.broadcast(r#"{"type":"app.event","name":"fgcolorchange","color":4278190335}"#);
    let t = fe.txn();
    assert!(!committed(&t));
    assert!(msgs(&t).iter().any(|m| m.contains("nope")), "{t:?}");
    assert_eq!(fe.apply(&t), -2);
    assert_eq!(fe.pixel(0, 0), [0, 0, 0, 0]);
    fe.end(s);
}

#[test]
fn handlers_run_in_the_sandbox() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        app.events:on("sitechange", function()
          local bad = {}
          for _, name in ipairs({ "package", "debug", "require" }) do
            if _G[name] ~= nil then bad[#bad + 1] = name end
          end
          print("bad=" .. #bad, tostring(load(string.dump(function() end))))
          print(pcall(function() return os.execute("echo hi") end))
        end)
        "#,
    );
    fe.idle();
    fe.clear();
    fe.broadcast(r#"{"type":"app.event","name":"sitechange"}"#);
    let m = msgs(&fe.txn());
    assert_eq!(m[0], "bad=0\tnil");
    assert!(m[1].starts_with("false\t"), "{m:?}");
    fe.end(s);

    // The same holds for a plugin command.
    let p = fe.plugin(
        r#"function init(plugin)
             plugin:newCommand{ id = "x", title = "X", onclick = function() print(type(os), type(io), type(require), type(package)) end }
           end"#,
        None,
    );
    fe.idle();
    fe.clear();
    fe.command(p, "x");
    assert_eq!(msgs(&fe.txn()), ["table\ttable\tnil\tnil"]);
    fe.end(p);
}

#[test]
fn a_runaway_handler_is_stopped_by_the_instruction_limit_and_the_session_survives() {
    let mut fe = Fe::with_limits(tight());
    let s = fe.script(
        r#"
        local spin = true
        app.events:on("sitechange", function() if spin then while true do end end print("fine") end)
        app.events:on("fgcolorchange", function() spin = false end)
        "#,
    );
    fe.idle();
    fe.clear();
    fe.broadcast(r#"{"type":"app.event","name":"sitechange"}"#);
    let t = fe.txn();
    let seen = format!("{} {:?}", text(&t, "error"), msgs(&t));
    assert!(seen.contains("limit"), "{seen}");
    // Budgets are per transaction: the next event is served normally.
    fe.broadcast(r#"{"type":"app.event","name":"fgcolorchange","color":1}"#);
    assert!(ok(&fe.txn()));
    fe.broadcast(r#"{"type":"app.event","name":"sitechange"}"#);
    let t = fe.txn();
    assert_eq!(msgs(&t), ["fine"]);
    fe.end(s);
}

#[test]
fn a_slow_handler_hits_the_time_limit_and_a_greedy_one_the_memory_limit() {
    let limits = Limits { instructions: u64::MAX, time: Duration::from_millis(300), memory: 16 << 20 };
    let mut fe = Fe::with_limits(limits);
    let s = fe.script(
        r#"
        app.events:on("sitechange", function() local n = 0 while true do n = n + 1 end end)
        app.events:on("bgcolorchange", function() local t = {} while true do t[#t + 1] = string.rep("x", 1 << 20) .. #t end end)
        "#,
    );
    fe.idle();
    fe.clear();
    let started = Instant::now();
    fe.broadcast(r#"{"type":"app.event","name":"sitechange"}"#);
    let t = fe.txn();
    assert!(format!("{} {:?}", text(&t, "error"), msgs(&t)).contains("time limit"), "{t:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
    fe.broadcast(r#"{"type":"app.event","name":"bgcolorchange","color":1}"#);
    let t = fe.txn();
    assert!(format!("{} {:?}", text(&t, "error"), msgs(&t)).to_lowercase().contains("memory"), "{t:?}");
    fe.end(s);
}

#[test]
fn stop_interrupts_a_handler_that_is_running() {
    let mut fe = Fe::new();
    let s = fe.script(r#"app.events:on("sitechange", function() while true do end end)"#);
    fe.idle();
    fe.clear();
    fe.broadcast(r#"{"type":"app.event","name":"sitechange"}"#);
    fe.wait("running", |m| ty_of(m) == "state" && text(m, "state") == "running");
    std::thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    fe.host.stop(s);
    let t = fe.txn();
    assert!(format!("{} {:?}", text(&t, "error"), msgs(&t)).contains("script cancelled"), "{t:?}");
    fe.of_type("session.end");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(fe.host.count(), 0);
}

#[test]
fn a_session_with_only_listeners_ends_when_the_last_listener_is_gone() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        local id
        id = app.events:on("sitechange", function() print("once"); app.events:off(id) end)
        "#,
    );
    fe.idle();
    assert_eq!(fe.host.count(), 1);
    fe.clear();
    fe.broadcast(r#"{"type":"app.event","name":"sitechange"}"#);
    assert_eq!(msgs(&fe.txn()), ["once"]);
    fe.wait("end", |m| ty_of(m) == "session.end" && num(m, "session") == s);
    assert_eq!(fe.host.count(), 0);
    assert_eq!(fe.broadcast(r#"{"type":"app.event","name":"sitechange"}"#), 0);
}

#[test]
fn a_script_without_listeners_ends_on_its_own() {
    let mut fe = Fe::new();
    let s = fe.script(r#"local id = app.events:on("sitechange", function() end); app.events:off(id)"#);
    fe.wait("end", |m| ty_of(m) == "session.end" && num(m, "session") == s);
    assert_eq!(fe.host.count(), 0);
}

#[test]
fn events_are_served_while_a_modal_dialog_waits() {
    let mut fe = Fe::new();
    let s = fe.script(
        r#"
        app.events:on("fgcolorchange", function(ev) print("fg " .. ev.color) end)
        local dlg = Dialog("t")
        dlg:button{ id = "ok", text = "OK" }:show()
        print("after")
        "#,
    );
    let open = fe.of_type("dialog.open");
    fe.wait("waiting", |m| ty_of(m) == "state" && text(m, "state") == "waiting");
    assert_eq!(fe.broadcast(r#"{"type":"app.event","name":"fgcolorchange","color":9}"#), 1);
    std::thread::sleep(Duration::from_millis(100));
    let id = text(open.get("dialog").unwrap(), "id").to_string();
    fe.send(s, &format!(r#"{{"type":"widget","dialog":"{id}","id":"ok","event":"click"}}"#));
    let t = fe.txn();
    assert_eq!(msgs(&t), ["fg 9", "after"], "the event ran inside the transaction of the main chunk");
    fe.end(s);
}

#[test]
fn registering_is_validated() {
    let cases = [
        (r#"app.events:on("nope", function() end)"#, "unknown event"),
        (r#"app.events:on("sitechange")"#, ""),
        (r#"app.events:on("sitechange", 5)"#, ""),
        (r#"app.events:on(5, function() end)"#, ""),
        (r#"app.events:off("x")"#, "listener id or a function"),
        (r#"app.events:off()"#, "listener id or a function"),
        (r#"sprite.events:on("sitechange", function() end)"#, "unknown event"),
        (r#"app.events:on("change", function() end)"#, "unknown event"),
    ];
    for (src, needle) in cases {
        let mut fe = Fe::new();
        fe.script(src);
        let t = fe.txn();
        assert!(!ok(&t), "{src}");
        assert!(text(&t, "error").contains(needle), "{src}: {t:?}");
        assert!(!text(&t, "error").contains("internal error"), "{src}");
    }
}

#[test]
fn the_number_of_listeners_is_capped() {
    let mut fe = Fe::new();
    fe.script(r#"for i = 1, 5000 do app.events:on("sitechange", function() end) end"#);
    let t = fe.txn();
    assert!(!ok(&t));
    assert!(text(&t, "error").contains("too many listeners"), "{t:?}");
}

#[test]
fn without_a_front_end_events_are_accepted_and_never_fire() {
    let mut e = Editor::new(Sprite::new(2, 2));
    let o = pixmith_core::scripting::run(
        &mut e,
        r#"
        local id = app.events:on("sitechange", function() end)
        sprite.events:on("change", function() end)
        app.events:off(id)
        print(tostring(app.isUIAvailable), type(app.events), type(sprite.events))
        "#,
        (0, 0),
        Limits::default(),
    );
    assert_eq!(o.error, None);
    assert_eq!(o.messages, ["false\tuserdata\tuserdata"]);
}

#[test]
fn script_colors_reach_handlers_and_assignments_are_reported() {
    let mut fe = Fe::new();
    fe.host.set_colors(0x112233FF, 0x445566FF);
    let s = fe.script(
        r#"
        app.events:on("fgcolorchange", function(ev) print(tostring(app.fgColor), ev.color) end)
        app.events:on("bgcolorchange", function() app.fgColor = 0x010203FF end)
        "#,
    );
    fe.idle();
    fe.clear();
    fe.host.set_colors(0xAABBCCFF, 0x445566FF);
    fe.broadcast(r#"{"type":"app.event","name":"fgcolorchange","color":2864434431}"#);
    assert_eq!(msgs(&fe.txn()), ["Color{ r=170, g=187, b=204, a=255 }\t2864434431"]);
    fe.broadcast(r#"{"type":"app.event","name":"bgcolorchange","color":1}"#);
    let set = fe.of_type("app.set");
    assert_eq!(num(&set, "fgColor"), 0x010203FF);
    fe.end(s);
}

// MARK: Plugins

const PLUGIN: &str = r#"
print("load")
function init(plugin)
  print("init", plugin.name, plugin.path, tostring(sprite), tostring(app.sprite), type(plugin.preferences))
  plugin.preferences.runs = (plugin.preferences.runs or 0) + 1
  plugin:newCommand{
    id = "paint", title = "Paint Pixel", group = "file_scripts",
    onclick = function() sprite:setPixel(2, 1, 0x00FF00FF) end,
  }
end
function exit(plugin)
  print("exit " .. plugin.preferences.runs)
  plugin.preferences.exited = true
end
"#;

#[test]
fn plugin_loads_runs_init_and_exit_in_order() {
    let dir = tmp("lifecycle");
    let prefs = dir.join("prefs.json");
    let mut fe = Fe::new();
    let s = fe.plugin(PLUGIN, Some(&prefs));
    let t = fe.txn();
    assert!(ok(&t), "{}", fe.dump());
    assert_eq!(msgs(&t), ["load", "init\tplug\t/plugins/plug\tnil\tnil\ttable"]);
    assert!(!committed(&t));
    fe.idle();
    assert_eq!(fe.host.count(), 1, "a plugin stays loaded without dialogs or listeners");
    fe.clear();

    fe.host.stop(s);
    let t = fe.txn();
    assert!(ok(&t));
    assert_eq!(msgs(&t), ["exit 1"]);
    fe.of_type("command.remove");
    fe.of_type("session.end");
    assert_eq!(fe.host.count(), 0);
    let saved = read_prefs(&prefs);
    assert_eq!(saved.get("runs").and_then(J::as_f64), Some(1.0));
    assert_eq!(saved.get("exited"), Some(&J::Bool(true)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn plugin_without_init_or_exit_is_fine() {
    let mut fe = Fe::new();
    let s = fe.plugin("local x = 1", None);
    let t = fe.txn();
    assert!(ok(&t) && msgs(&t).is_empty());
    fe.idle();
    fe.end(s);
}

#[test]
fn plugin_without_directory_or_prefs_path_still_works() {
    let mut fe = Fe::new();
    let s = fe
        .host
        .start(Spec::Plugin { source: PLUGIN.into(), name: "p".into(), dir: None, prefs_path: None }, None)
        .unwrap();
    let t = fe.txn();
    assert_eq!(msgs(&t), ["load", "init\tp\t\tnil\tnil\ttable"]);
    fe.end(s);
}

#[test]
fn new_command_announces_itself_and_delete_command_removes_it() {
    let mut fe = Fe::new();
    let s = fe.plugin(
        r#"
        function init(plugin)
          plugin:newCommand{ id = "a", title = "Alpha...", onclick = function() end }
          plugin:newCommand{ id = "b", title = "Beta", group = "edit_scripts", onclick = function() end,
                             onenabled = function() return false end, onchecked = function() return true end }
          plugin:newCommand{ id = "c", title = "Gamma" }
          plugin:newCommand{ id = "a", title = "Alpha 2", onclick = function() end }   -- replaces a
          plugin:deleteCommand("c")
          plugin:deleteCommand("zzz")                                                  -- unknown: ignored
        end
        "#,
        None,
    );
    let mut kinds = Vec::new();
    let t = loop {
        let m = fe.wait("command or txn", |m| {
            matches!(ty_of(m), "command.add" | "command.remove" | "command.update" | "txn")
        });
        if ty_of(&m) == "txn" {
            break m;
        }
        let c = m.get("command");
        let id = c.map_or_else(|| text(&m, "id").to_string(), |c| text(c, "id").to_string());
        kinds.push(format!("{} {id}", ty_of(&m)));
        match ty_of(&m) {
            "command.add" => {
                assert_eq!(text(&m, "plugin"), "plug");
                assert_eq!(num(&m, "session"), s);
                let c = c.unwrap();
                match id.as_str() {
                    "a" => assert_eq!(
                        (text(c, "title"), text(c, "group")),
                        (if kinds.len() > 3 { "Alpha 2" } else { "Alpha..." }, "file_scripts")
                    ),
                    "b" => assert_eq!((text(c, "title"), text(c, "group")), ("Beta", "edit_scripts")),
                    _ => {}
                }
                assert!(c.get("enabled").is_some() && c.get("checked").is_some());
            }
            // onenabled / onchecked are evaluated at the end of init, before the txn is reported.
            "command.update" => {
                let c = c.unwrap();
                assert_eq!(
                    (id.as_str(), c.get("enabled"), c.get("checked")),
                    ("b", Some(&J::Bool(false)), Some(&J::Bool(true)))
                );
            }
            _ => {}
        }
    };
    assert!(ok(&t), "{}", fe.dump());
    assert_eq!(
        kinds,
        [
            "command.add a",
            "command.add b",
            "command.add c",
            "command.remove a",
            "command.add a",
            "command.remove c",
            "command.update b"
        ]
    );
    fe.idle();
    fe.clear();
    // Stopping removes what is left, newest last registered order irrelevant but both go.
    fe.host.stop(s);
    let mut gone = vec![
        text(&fe.of_type("command.remove"), "id").to_string(),
        text(&fe.of_type("command.remove"), "id").to_string(),
    ];
    gone.sort();
    assert_eq!(gone, ["a", "b"]);
    fe.of_type("session.end");
}

#[test]
fn new_command_validates_its_arguments() {
    let cases = [
        (r#"plugin:newCommand{ title = "T" }"#, "needs an id"),
        (r#"plugin:newCommand{ id = "x" }"#, "needs a title"),
        (r#"plugin:newCommand{ id = "x", title = "T", onclick = 5 }"#, "must be a function"),
        (r#"plugin:newCommand{ id = "x", title = "T", onenabled = "yes" }"#, "must be a function"),
        (r#"plugin:newCommand{ id = "x", title = {}, onclick = print }"#, ""),
        (r#"plugin:newCommand(5)"#, ""),
        (r#"plugin:newMenuGroup{ id = "g", title = "G" }"#, "not supported"),
        (r#"plugin:newMenuSeparator{ group = "g" }"#, "not supported"),
        (r#"for i = 1, 1000 do plugin:newCommand{ id = "c" .. i, title = "T" } end"#, "too many commands"),
    ];
    for (body, needle) in cases {
        let mut fe = Fe::new();
        let s = fe.plugin(&format!("function init(plugin) {body} end"), None);
        let t = fe.wait("init txn", |m| ty_of(m) == "txn");
        assert!(!ok(&t), "{body}");
        assert!(text(&t, "error").contains(needle), "{body}: {t:?}");
        assert!(!text(&t, "error").contains("internal error"), "{body}");
        fe.end(s);
    }
}

#[test]
fn a_plugin_that_fails_to_init_reports_it_but_keeps_the_commands_it_got_to_register() {
    let mut fe = Fe::new();
    let s = fe.plugin(
        r#"function init(plugin)
             plugin:newCommand{ id = "ok", title = "Fine", onclick = function() print("still here") end }
             error("init went wrong")
           end"#,
        None,
    );
    fe.of_type("command.add");
    let t = fe.txn();
    assert!(!ok(&t));
    assert!(text(&t, "error").contains("init went wrong"));
    fe.idle();
    fe.clear();
    fe.command(s, "ok");
    assert_eq!(msgs(&fe.txn()), ["still here"]);
    fe.end(s);
}

#[test]
fn a_plugin_that_does_not_parse_is_reported_and_unloads_cleanly() {
    let mut fe = Fe::new();
    let s = fe.plugin("function init(", None);
    let t = fe.txn();
    assert!(!ok(&t));
    assert!(!text(&t, "error").is_empty());
    fe.end(s);
}

#[test]
fn a_command_runs_on_the_active_document_as_one_undo_step() {
    let mut fe = Fe::new();
    let s = fe.plugin(PLUGIN, None);
    fe.txn();
    fe.idle();
    fe.clear();
    fe.command(s, "paint");
    let t = fe.wait("commit", |m| ty_of(m) == "txn");
    assert!(ok(&t) && committed(&t), "{}", fe.dump());
    assert_eq!(text(&t, "label"), "Paint Pixel", "the command title names the undo step");
    assert_eq!(num(&t, "doc"), TAG);
    assert_eq!(fe.apply(&t), 1);
    assert_eq!(fe.pixel(2, 1), [0, 255, 0, 255]);
    assert_eq!(fe.ed.history_len().0, 1);
    fe.ed.undo();
    assert_eq!(fe.pixel(2, 1), [0, 0, 0, 0]);
    // Unknown commands do nothing, twice in a row is two steps.
    fe.command(s, "does-not-exist");
    let t = fe.txn();
    assert!(ok(&t) && !committed(&t));
    fe.command(s, "paint");
    let t = fe.txn();
    assert_eq!(fe.apply(&t), 1);
    fe.end(s);
}

#[test]
fn a_command_can_set_the_label_and_the_selection_and_use_dialogs() {
    let mut fe = Fe::new();
    let s = fe.plugin(
        r#"function init(plugin)
             plugin:newCommand{ id = "go", title = "Go", onclick = function()
               local dlg = Dialog("Go")
               dlg:entry{ id = "text", text = "hi" }:button{ id = "ok", text = "OK" }:show()
               if dlg.data.ok then
                 sprite:newLayer("Made")
                 app.activeLayer = 2
                 app.undoLabel("Made a layer " .. dlg.data.text)
               end
             end }
           end"#,
        None,
    );
    fe.txn();
    fe.idle();
    fe.clear();
    fe.command(s, "go");
    let open = fe.of_type("dialog.open");
    let id = text(open.get("dialog").unwrap(), "id").to_string();
    fe.send(s, &format!(r#"{{"type":"widget","dialog":"{id}","id":"text","event":"change","value":"there"}}"#));
    fe.send(s, &format!(r#"{{"type":"widget","dialog":"{id}","id":"ok","event":"click"}}"#));
    let t = fe.wait("txn", |m| ty_of(m) == "txn");
    assert!(committed(&t), "{}", fe.dump());
    assert_eq!(text(&t, "label"), "Made a layer there");
    assert_eq!(t.get("select").and_then(|s| s.get("layer")).and_then(J::as_f64), Some(2.0));
    assert_eq!(fe.apply(&t), 1);
    assert_eq!(fe.ed.sprite.layers.len(), 4);
    assert_eq!(fe.ed.history_len().0, 1);
    fe.end(s);
}

#[test]
fn a_command_without_a_document_sees_no_sprite_and_commits_nothing() {
    let mut fe = Fe::new();
    let s = fe.plugin(
        r#"function init(plugin)
             plugin:newCommand{ id = "peek", title = "Peek", onclick = function() print(tostring(sprite), tostring(app.sprite)) end }
           end"#,
        None,
    );
    fe.txn();
    fe.idle();
    fe.clear();
    assert!(fe.host.send(s, json::parse(r#"{"type":"command","id":"peek"}"#).unwrap(), None));
    let t = fe.txn();
    assert_eq!(msgs(&t), ["nil\tnil"]);
    assert!(!committed(&t));
    fe.end(s);
}

#[test]
fn a_failing_command_is_rolled_back_and_the_plugin_keeps_working() {
    let mut fe = Fe::new();
    let s = fe.plugin(
        r#"function init(plugin)
             plugin:newCommand{ id = "bad", title = "Bad", onclick = function() sprite:setPixel(0, 0, 0xFF0000FF); error("kaput") end }
             plugin:newCommand{ id = "good", title = "Good", onclick = function() sprite:setPixel(1, 0, 0x00FF00FF) end }
           end"#,
        None,
    );
    fe.txn();
    fe.idle();
    fe.clear();
    fe.command(s, "bad");
    let t = fe.txn();
    assert!(!ok(&t) && !committed(&t));
    assert!(text(&t, "error").contains("kaput"));
    assert_eq!(fe.apply(&t), -2);
    assert_eq!(fe.pixel(0, 0), [0, 0, 0, 0]);
    fe.command(s, "good");
    let t = fe.txn();
    assert_eq!(fe.apply(&t), 1);
    assert_eq!(fe.pixel(1, 0), [0, 255, 0, 255]);
    fe.end(s);
}

#[test]
fn onenabled_and_onchecked_are_re_evaluated_after_events_and_commands() {
    let mut fe = Fe::new();
    let s = fe.plugin(
        r#"function init(plugin)
             plugin:newCommand{ id = "toggle", title = "Toggle",
               onenabled = function() return sprite ~= nil and sprite.width == 4 end,
               onchecked = function() return plugin.preferences.on == true end,
               onclick = function() plugin.preferences.on = not plugin.preferences.on end }
           end"#,
        None,
    );
    fe.of_type("command.add");
    fe.txn();
    let u = fe.of_type("command.update");
    assert_eq!(u.get("command").unwrap().get("enabled"), Some(&J::Bool(false)), "no document at load time");
    fe.idle();
    fe.clear();

    // A document appears: sitechange enables the command (sprite events do the same).
    fe.broadcast(r#"{"type":"app.event","name":"sitechange"}"#);
    let u = fe.of_type("command.update");
    assert_eq!(u.get("command").unwrap().get("enabled"), Some(&J::Bool(true)));
    // Clicking it flips the checkmark.
    fe.command(s, "toggle");
    let u = fe.of_type("command.update");
    assert_eq!(u.get("command").unwrap().get("checked"), Some(&J::Bool(true)));
    fe.command(s, "toggle");
    let u = fe.of_type("command.update");
    assert_eq!(u.get("command").unwrap().get("checked"), Some(&J::Bool(false)));
    // The user edits another document: sprite events keep the state fresh as well.
    fe.clear();
    fe.host.broadcast(
        &json::parse(r#"{"type":"sprite.event","name":"change","fromUndo":false}"#).unwrap(),
        None,
        0,
        0,
        0,
    );
    let t = fe.txn();
    assert!(ok(&t));
    let u = fe.of_type("command.update");
    assert_eq!(u.get("command").unwrap().get("enabled"), Some(&J::Bool(false)), "no document any more");
    // Nothing changes, nothing is sent.
    fe.clear();
    fe.host.broadcast(&json::parse(r#"{"type":"app.event","name":"sitechange"}"#).unwrap(), None, 0, 0, 0);
    fe.txn();
    fe.quiet(100);
    fe.end(s);
}

#[test]
fn onenabled_must_not_change_the_document_and_errors_disable_the_command() {
    let mut fe = Fe::new();
    let s = fe.plugin(
        r#"function init(plugin)
             plugin:newCommand{ id = "sneaky", title = "S", onenabled = function() sprite:setPixel(0, 0, 0xFF0000FF); return true end, onclick = function() end }
             plugin:newCommand{ id = "broken", title = "B", onenabled = function() error("no") end, onclick = function() end }
           end"#,
        None,
    );
    fe.txn();
    fe.idle();
    fe.clear();
    fe.broadcast(r#"{"type":"app.event","name":"sitechange"}"#);
    let t = fe.txn();
    assert!(ok(&t) && !committed(&t), "onenabled's edit is thrown away: {t:?}");
    fe.end(s);
}

#[test]
fn a_plugin_listens_to_events_and_offers_commands_at_once() {
    let mut fe = Fe::new();
    let s = fe.plugin(
        r#"
        local sites = 0
        function init(plugin)
          app.events:on("sitechange", function() sites = sites + 1 end)
          plugin:newCommand{ id = "report", title = "Report", onclick = function() print("sites=" .. sites) end }
        end"#,
        None,
    );
    fe.txn();
    fe.idle();
    fe.clear();
    fe.broadcast(r#"{"type":"app.event","name":"sitechange"}"#);
    fe.broadcast(r#"{"type":"app.event","name":"sitechange"}"#);
    fe.command(s, "report");
    let t = fe.wait("report", |m| ty_of(m) == "txn" && !msgs(m).is_empty());
    assert_eq!(msgs(&t), ["sites=2"]);
    fe.end(s);
}

// MARK: Preferences

#[test]
fn preferences_persist_across_reloads_and_updates_are_saved_without_a_restart() {
    let dir = tmp("prefs");
    let prefs = dir.join("deeper").join("prefs.json"); // the folder does not exist yet
    let src = r#"
        function init(plugin)
          local p = plugin.preferences
          p.runs = (p.runs or 0) + 1
          p.name = p.name or "ü \"quoted\"\n"
          p.ratio = 0.25
          p.flag = false
          p.nested = p.nested or { a = 1, list = { "x", "y", 3 } }
          plugin:newCommand{ id = "bump", title = "Bump", onclick = function() p.clicks = (p.clicks or 0) + 1 end }
        end"#;
    let mut fe = Fe::new();
    let s = fe.plugin(src, Some(&prefs));
    fe.txn();
    fe.idle();
    let saved = read_prefs(&prefs);
    assert_eq!(saved.get("runs").and_then(J::as_f64), Some(1.0));
    assert_eq!(saved.get("name").and_then(J::as_str), Some("ü \"quoted\"\n"));
    assert_eq!(saved.get("ratio").and_then(J::as_f64), Some(0.25));
    assert_eq!(saved.get("flag"), Some(&J::Bool(false)));
    assert_eq!(saved.get("nested").unwrap().get("list").unwrap().as_array().unwrap().len(), 3);

    fe.clear();
    fe.command(s, "bump");
    fe.command(s, "bump");
    fe.txn();
    fe.txn();
    assert_eq!(
        read_prefs(&prefs).get("clicks").and_then(J::as_f64),
        Some(2.0),
        "saved after the commands, not only on exit"
    );
    fe.end(s);

    // Loaded again, also by a different host: the stored values are there before init runs.
    let mut fe2 = Fe::new();
    let s2 = fe2.plugin(src, Some(&prefs));
    fe2.txn();
    fe2.idle();
    let saved = read_prefs(&prefs);
    assert_eq!(saved.get("runs").and_then(J::as_f64), Some(2.0));
    assert_eq!(saved.get("clicks").and_then(J::as_f64), Some(2.0));
    assert_eq!(saved.get("nested").unwrap().get("a").and_then(J::as_f64), Some(1.0));
    fe2.end(s2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn preferences_are_saved_after_exit_even_if_exit_fails() {
    let dir = tmp("exit-fails");
    let prefs = dir.join("p.json");
    let mut fe = Fe::new();
    let s = fe.plugin(
        r#"function init(plugin) plugin.preferences.a = 1 end
           function exit(plugin) plugin.preferences.b = 2; error("exit failed") end"#,
        Some(&prefs),
    );
    fe.txn();
    fe.idle();
    fe.clear();
    fe.host.stop(s);
    let t = fe.txn();
    assert!(!ok(&t));
    assert!(text(&t, "error").contains("exit failed"));
    fe.of_type("session.end");
    let saved = read_prefs(&prefs);
    assert_eq!((saved.get("a").and_then(J::as_f64), saved.get("b").and_then(J::as_f64)), (Some(1.0), Some(2.0)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unreadable_preferences_start_empty() {
    let dir = tmp("corrupt");
    std::fs::create_dir_all(&dir).unwrap();
    let src = r#"function init(plugin) print(tostring(next(plugin.preferences) == nil)); plugin.preferences.x = 1 end"#;
    for (i, content) in ["{not json", "[1,2,3]", "\"str\"", "", "null", "{\"a\": }"].iter().enumerate() {
        let prefs = dir.join(format!("p{i}.json"));
        std::fs::write(&prefs, content).unwrap();
        let mut fe = Fe::new();
        let s = fe.plugin(src, Some(&prefs));
        let t = fe.txn();
        assert_eq!(msgs(&t), ["true"], "{content:?}");
        fe.idle();
        assert_eq!(read_prefs(&prefs).get("x").and_then(J::as_f64), Some(1.0), "{content:?}");
        fe.end(s);
    }
    // A preferences path that is a directory cannot be written: the plugin still runs.
    let as_dir = dir.join("dir.json");
    std::fs::create_dir_all(&as_dir).unwrap();
    let mut fe = Fe::new();
    let s = fe.plugin(src, Some(&as_dir));
    assert!(ok(&fe.txn()));
    fe.end(s);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn odd_preference_values_neither_crash_nor_lose_the_good_ones() {
    let dir = tmp("odd");
    let prefs = dir.join("p.json");
    let mut fe = Fe::new();
    let s = fe.plugin(
        r#"function init(plugin)
             local p = plugin.preferences
             p.keep = "me"
             p.fn = function() end
             p.nan = 0 / 0
             p.inf = math.huge
             p[1] = "int key"
             p.cycle = {}
             p.cycle.self = p.cycle
             p.deep = {}
             local t = p.deep
             for i = 1, 40 do t.d = {}; t = t.d end
           end"#,
        Some(&prefs),
    );
    let t = fe.txn();
    assert!(ok(&t), "{t:?}");
    fe.idle();
    let saved = read_prefs(&prefs);
    assert_eq!(
        saved.get("keep").and_then(J::as_str),
        Some("me"),
        "a value that cannot be stored must not take the others with it"
    );
    assert!(saved.get("fn").is_none());
    fe.end(s);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn oversized_preferences_are_not_written_and_do_not_clobber_the_old_file() {
    let dir = tmp("big");
    let prefs = dir.join("p.json");
    let mut fe = Fe::new();
    let s = fe.plugin(
        r#"function init(plugin)
             plugin.preferences.small = 1
             plugin:newCommand{ id = "grow", title = "Grow", onclick = function() plugin.preferences.big = string.rep("x", 2 * 1024 * 1024) end }
           end"#,
        Some(&prefs),
    );
    fe.txn();
    fe.idle();
    fe.clear();
    fe.command(s, "grow");
    assert!(ok(&fe.txn()));
    fe.idle();
    let saved = read_prefs(&prefs);
    assert_eq!(saved.get("small").and_then(J::as_f64), Some(1.0));
    assert!(saved.get("big").is_none(), "over the 1 MB cap");
    fe.end(s);
    assert!(std::fs::metadata(&prefs).unwrap().len() < 1 << 20);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_oversized_preferences_file_is_ignored() {
    let dir = tmp("huge-file");
    std::fs::create_dir_all(&dir).unwrap();
    let prefs = dir.join("p.json");
    std::fs::write(&prefs, format!("{{\"junk\":\"{}\"}}", "x".repeat(2 << 20))).unwrap();
    let mut fe = Fe::new();
    let s = fe.plugin(r#"function init(plugin) print(tostring(plugin.preferences.junk)) end"#, Some(&prefs));
    assert_eq!(msgs(&fe.txn()), ["nil"]);
    fe.end(s);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn preferences_changed_by_a_failed_command_are_still_the_scripts_state_but_unchanged_ones_cost_no_write() {
    let dir = tmp("nowrite");
    let prefs = dir.join("p.json");
    let mut fe = Fe::new();
    let s = fe.plugin(
        r#"function init(plugin)
             plugin.preferences.n = 1
             plugin:newCommand{ id = "noop", title = "N", onclick = function() end }
           end"#,
        Some(&prefs),
    );
    fe.txn();
    fe.idle();
    let first = std::fs::metadata(&prefs).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    fe.clear();
    fe.command(s, "noop");
    fe.txn();
    fe.idle();
    assert_eq!(
        std::fs::metadata(&prefs).unwrap().modified().unwrap(),
        first,
        "unchanged preferences are not rewritten"
    );
    fe.end(s);
    let _ = std::fs::remove_dir_all(dir);
}

// MARK: Stop, limits, many plugins

#[test]
fn exit_cannot_run_forever() {
    let dir = tmp("exit-loop");
    let prefs = dir.join("p.json");
    let mut fe = Fe::with_limits(tight());
    let s = fe.plugin(
        r#"function init(plugin) plugin.preferences.saved = true end
           function exit(plugin) while true do end end"#,
        Some(&prefs),
    );
    fe.txn();
    fe.idle();
    fe.clear();
    let started = Instant::now();
    fe.host.stop(s);
    let t = fe.txn();
    assert!(!ok(&t));
    assert!(text(&t, "error").contains("limit"), "{t:?}");
    fe.of_type("session.end");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(read_prefs(&prefs).get("saved"), Some(&J::Bool(true)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_plugin_stuck_in_a_command_can_be_stopped_and_still_gets_to_exit() {
    let mut fe = Fe::new();
    let s = fe.plugin(
        r#"function init(plugin) plugin:newCommand{ id = "spin", title = "Spin", onclick = function() while true do end end } end
           function exit(plugin) print("bye") end"#,
        None,
    );
    fe.txn();
    fe.idle();
    fe.clear();
    fe.command(s, "spin");
    std::thread::sleep(Duration::from_millis(150));
    fe.host.stop(s);
    let t = fe.txn();
    assert!(text(&t, "error").contains("script cancelled"), "{t:?}");
    let t = fe.txn();
    assert_eq!(msgs(&t), ["bye"], "exit(plugin) ran after the interruption");
    fe.of_type("session.end");
}

#[test]
fn a_stopped_plugin_closes_its_dialogs_and_removes_its_commands() {
    let mut fe = Fe::new();
    let s = fe.plugin(
        r#"function init(plugin)
             plugin:newCommand{ id = "open", title = "Open", onclick = function()
               Dialog("Tool"):button{ id = "b", text = "b", onclick = function() end }:show{ wait = false }
             end }
           end"#,
        None,
    );
    fe.txn();
    fe.idle();
    fe.clear();
    fe.command(s, "open");
    let open = fe.of_type("dialog.open");
    fe.idle();
    fe.clear();
    fe.host.stop(s);
    let closed = fe.of_type("dialog.close");
    assert_eq!(text(&closed, "dialog"), text(open.get("dialog").unwrap(), "id"));
    fe.of_type("command.remove");
    fe.of_type("session.end");
}

#[test]
fn several_plugins_are_independent() {
    let dir = tmp("many");
    let mut fe = Fe::new();
    let src = |n: &str| {
        format!(
            r#"function init(plugin)
                 plugin.preferences.me = "{n}"
                 plugin:newCommand{{ id = "who", title = "Who {n}", onclick = function() print("{n}") end }}
               end"#
        )
    };
    let a = fe.plugin(&src("a"), Some(&dir.join("a.json")));
    let b = fe.plugin(&src("b"), Some(&dir.join("b.json")));
    fe.idle();
    fe.idle();
    fe.clear();
    fe.command(b, "who");
    let t = fe.wait("b", |m| ty_of(m) == "txn");
    assert_eq!((num(&t, "session"), msgs(&t)), (b, vec!["b".to_string()]));
    fe.command(a, "who");
    let t = fe.wait("a", |m| ty_of(m) == "txn");
    assert_eq!((num(&t, "session"), msgs(&t)), (a, vec!["a".to_string()]));
    // Stopping one does not disturb the other.
    fe.end(a);
    fe.clear();
    fe.command(b, "who");
    assert_eq!(msgs(&fe.txn()), ["b"]);
    fe.end(b);
    assert_eq!(read_prefs(&dir.join("a.json")).get("me").and_then(J::as_str), Some("a"));
    assert_eq!(read_prefs(&dir.join("b.json")).get("me").and_then(J::as_str), Some("b"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_session_wide_panic_proof_a_broken_event_stream_does_not_hurt() {
    let mut fe = Fe::new();
    let s = fe.plugin(PLUGIN, None);
    fe.txn();
    fe.idle();
    fe.clear();
    for ev in [
        r#"{}"#,
        r#"{"type":"command"}"#,
        r#"{"type":"command","id":5}"#,
        r#"{"type":"app.event"}"#,
        r#"{"type":"app.event","name":5}"#,
        r#"{"type":"sprite.event","name":"change"}"#,
        r#"{"type":"app.event","name":"sitechange","layer":"x","frame":null}"#,
        r#"[]"#,
        r#"null"#,
    ] {
        assert!(fe.host.send(s, json::parse(ev).unwrap(), None), "{ev}");
    }
    fe.command(s, "paint");
    let t = fe.wait("paint", |m| ty_of(m) == "txn" && m.get("messages").is_some() && num(m, "txn") > 0 && committed(m));
    assert_eq!(fe.apply(&t), 1);
    fe.end(s);
}

// MARK: The C ABI

#[test]
fn the_c_abi_loads_plugins_and_delivers_events() {
    use pixmith_core::scripting::host::*;
    use std::ffi::{CStr, CString};
    let dir = tmp("abi");
    let prefs = dir.join("p.json");
    let src = CString::new(
        r#"function init(plugin)
             plugin.preferences.loaded = true
             app.events:on("fgcolorchange", function(ev) print("fg=" .. ev.color) end)
             plugin:newCommand{ id = "hi", title = "Hi", onclick = function() print("hi from " .. plugin.name) end }
           end
           function exit(plugin) plugin.preferences.exited = true end"#,
    )
    .unwrap();
    let name = CString::new("abi-plugin").unwrap();
    let pdir = CString::new("/p").unwrap();
    let pp = CString::new(prefs.to_string_lossy().as_ref()).unwrap();
    let session = unsafe { pixmith_plugin_load(src.as_ptr(), name.as_ptr(), pdir.as_ptr(), pp.as_ptr()) };
    assert!(session > 0);

    let mut seen: Vec<J> = Vec::new();
    let mut wait = |pred: &dyn Fn(&J) -> bool| -> J {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            let raw = pixmith_script_poll(50);
            assert!(!raw.is_null());
            let text = unsafe { CStr::from_ptr(raw).to_string_lossy().into_owned() };
            unsafe { pixmith_core::ffi::pixmith_string_free(raw) };
            if let Ok(J::Arr(a)) = json::parse(&text) {
                seen.extend(a.into_iter().filter(|m| num(m, "session") == session));
            }
            if let Some(i) = seen.iter().position(pred) {
                return seen.remove(i);
            }
            assert!(Instant::now() < end, "timed out: {seen:?}");
        }
    };
    let add = wait(&|m| ty_of(m) == "command.add");
    assert_eq!(text(add.get("command").unwrap(), "id"), "hi");
    wait(&|m| ty_of(m) == "state" && text(m, "state") == "idle");
    assert!(pixmith_script_count() >= 1);

    let ev = CString::new(r#"{"type":"app.event","name":"fgcolorchange","color":7}"#).unwrap();
    let n = unsafe { pixmith_script_broadcast(std::ptr::null_mut(), 0, 0, 0, ev.as_ptr()) };
    assert!(n >= 1);
    assert_eq!(msgs(&wait(&|m| ty_of(m) == "txn" && !msgs(m).is_empty())), ["fg=7"]);

    let cmd = CString::new(r#"{"type":"command","id":"hi"}"#).unwrap();
    assert!(unsafe { pixmith_script_send(session, std::ptr::null_mut(), 0, 0, 0, cmd.as_ptr()) });
    assert_eq!(msgs(&wait(&|m| ty_of(m) == "txn" && !msgs(m).is_empty())), ["hi from abi-plugin"]);
    let bad = CString::new("{nope").unwrap();
    assert!(!unsafe { pixmith_script_send(session, std::ptr::null_mut(), 0, 0, 0, bad.as_ptr()) });

    pixmith_script_stop(session);
    wait(&|m| ty_of(m) == "command.remove");
    wait(&|m| ty_of(m) == "session.end");
    assert_eq!(read_prefs(&prefs).get("exited"), Some(&J::Bool(true)));
    assert!(!unsafe { pixmith_script_send(session, std::ptr::null_mut(), 0, 0, 0, cmd.as_ptr()) });
    let _ = std::fs::remove_dir_all(dir);
}

// MARK: Examples

#[test]
fn the_events_example_plugin_counts_what_the_editor_reports() {
    let dir = tmp("example");
    let prefs = dir.join("p.json");
    let mut fe = Fe::new();
    let s = fe.plugin(include_str!("../../scripts/lua-examples/plugin-events.lua"), Some(&prefs));
    assert!(ok(&fe.txn()), "{}", fe.dump());
    fe.idle();
    fe.clear();
    fe.broadcast(r#"{"type":"app.event","name":"sitechange","layer":1,"frame":1}"#);
    fe.broadcast(r#"{"type":"app.event","name":"fgcolorchange","color":1}"#);
    fe.broadcast(r#"{"type":"sprite.event","name":"change","fromUndo":false}"#);
    fe.broadcast(r#"{"type":"sprite.event","name":"change","fromUndo":false}"#);
    fe.broadcast(r#"{"type":"sprite.event","name":"change","fromUndo":true}"#);
    for _ in 0..5 {
        assert!(ok(&fe.wait("txn", |m| ty_of(m) == "txn")), "{}", fe.dump());
    }
    fe.clear();
    fe.command(s, "event_report");
    let open = fe.of_type("dialog.open");
    let labels: Vec<String> = open
        .get("dialog")
        .unwrap()
        .get("rows")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r.get("widgets").unwrap().as_array().unwrap().iter())
        .map(|w| text(w, "text").to_string())
        .collect();
    assert!(labels.contains(&"Site changes: 1".to_string()), "{labels:?}");
    assert!(labels.contains(&"Color changes: 1".to_string()), "{labels:?}");
    assert!(labels.contains(&"Edits: 2, undo/redo: 1".to_string()), "{labels:?}");
    fe.end(s);
    let saved = read_prefs(&prefs);
    assert_eq!(saved.get("totalEdits").and_then(J::as_f64), Some(2.0));
    assert_eq!(saved.get("lastExit").and_then(J::as_f64), Some(1.0));
    let _ = std::fs::remove_dir_all(dir);
}
