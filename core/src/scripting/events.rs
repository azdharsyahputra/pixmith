//! Lua events: `app.events:on(name, fn)` / `:off(id|fn)` and `sprite.events:on("change", fn)`
//! (docs/LUA_DIALOG.md section 1.2).
//!
//! A front end reports what happened with `app.event` / `sprite.event` messages
//! (`pixmith_script_broadcast`). Sessions only receive the events they listen for: the set of listened-for
//! kinds is published as a bit mask (`Shared::listens`), so broadcasting is cheap when nobody listens.
//! Handlers run on the session's Lua state inside a transaction (host.rs), so what they change in the
//! sprite is one undo step, and the instruction and time budgets apply to them like to any callback.
//!
//! A handler that fails does not stop its siblings and stays registered: its own changes are rolled back
//! and the error is reported in the transaction's messages as `error: ...`. Limits and cancellation do end
//! the transaction.

use super::host::{is_fatal, Rt};
use super::plugins::json_to_lua;
use super::{clean_error, Ctx};
use crate::formats::json::Value as J;
use mlua::{Error, Function, Lua, Result as LuaResult, Table, UserData, UserDataMethods, Value};
use std::rc::Rc;
use std::sync::atomic::Ordering;

/// Listeners one session may hold (a script that registers one per site change would otherwise leak).
pub(super) const MAX_LISTENERS: usize = 1024;

/// Event kinds a session can listen for: `(object, name, bit)`. The bit is a position in `Shared::listens`.
pub(super) const EVENTS: [(&str, &str, u32); 7] = [
    ("app", "sitechange", 1),
    ("app", "fgcolorchange", 2),
    ("app", "bgcolorchange", 4),
    ("app", "beforecommand", 8),
    ("app", "aftercommand", 16),
    ("sprite", "change", 32),
    ("sprite", "filenamechange", 64),
];

/// Bits for the events that make a plugin re-evaluate `onenabled` / `onchecked`.
pub(super) const REFRESH_BITS: u32 = 1 | 32;

pub(super) fn event_bit(scope: &str, name: &str) -> Option<u32> {
    EVENTS.iter().find(|e| e.0 == scope && e.1 == name).map(|e| e.2)
}

pub(super) struct Listener {
    id: u64,
    scope: &'static str,
    event: String,
    f: Function,
    /// Sprite listeners fire for the document that was active when they were registered.
    tag: Option<u64>,
}

impl Rt {
    /// Delivers an `app.event` / `sprite.event` message to the matching listeners, in registration order.
    pub(super) fn on_app_event(&self, lua: &Lua, ev: &J) -> LuaResult<()> {
        let scope = if ev.get("type").and_then(J::as_str) == Some("sprite.event") { "sprite" } else { "app" };
        let name = ev.get("name").and_then(J::as_str).unwrap_or("");
        let ctx = self.ctx();
        let tag = self.cur_tag.get();
        let handlers: Vec<Function> = self
            .listeners
            .borrow()
            .iter()
            .filter(|l| {
                l.scope == scope && l.event == name && (scope == "app" || (l.tag == Some(tag) && ctx.has_doc.get()))
            })
            .map(|l| l.f.clone())
            .collect();
        for f in handlers {
            let arg = event_table(lua, ev)?;
            if let Err(e) = self.run_callback(lua, &f, arg) {
                if is_fatal(&e) {
                    return Err(e);
                }
                ctx.messages.borrow_mut().push(format!("error: {}", clean_error(&e)));
            }
        }
        Ok(())
    }

    fn add_listener(&self, scope: &'static str, event: String, f: Function) -> LuaResult<u64> {
        if self.listeners.borrow().len() >= MAX_LISTENERS {
            return Err(Error::runtime(format!(
                "too many listeners (at most {MAX_LISTENERS}); remove old ones with events:off"
            )));
        }
        self.listener_seq.set(self.listener_seq.get() + 1);
        let id = self.listener_seq.get();
        let tag = self.ctx().has_doc.get().then(|| self.cur_tag.get());
        self.listeners.borrow_mut().push(Listener { id, scope, event, f, tag });
        self.update_listens();
        Ok(id)
    }

    fn remove_listener(&self, scope: &str, by_id: Option<u64>, by_fn: Option<&Function>) {
        self.listeners
            .borrow_mut()
            .retain(|l| !(l.scope == scope && (by_id == Some(l.id) || by_fn.is_some_and(|f| *f == l.f))));
        self.update_listens();
    }

    /// Publishes which events this session wants to hear about.
    pub(super) fn update_listens(&self) {
        let mut bits = 0;
        for l in self.listeners.borrow().iter() {
            bits |= event_bit(l.scope, &l.event).unwrap_or(0);
        }
        if self.commands.borrow().iter().any(|c| c.wants_refresh()) {
            bits |= REFRESH_BITS;
        }
        self.shared.listens.store(bits, Ordering::Relaxed);
    }

    pub(super) fn has_listeners(&self) -> bool {
        !self.listeners.borrow().is_empty()
    }
}

/// The table a handler receives: the fields of the message, `name` for command events, and a
/// `stopPropagation` that says plainly it is not available (a silent no-op would let scripts believe they
/// cancelled a command).
fn event_table(lua: &Lua, ev: &J) -> LuaResult<Table> {
    let t = lua.create_table()?;
    if let J::Obj(m) = ev {
        for (k, v) in m {
            if !matches!(k.as_str(), "type" | "name" | "command") {
                t.set(k.as_str(), json_to_lua(lua, v, 0)?)?;
            }
        }
    }
    if let Some(c) = ev.get("command").and_then(J::as_str) {
        t.set("name", c)?;
    }
    t.set(
        "stopPropagation",
        lua.create_function(|_, _: mlua::MultiValue| -> LuaResult<()> {
            Err(Error::runtime("ev:stopPropagation is not supported by Pixmith"))
        })?,
    )?;
    Ok(t)
}

/// `app.events` and `sprite.events`.
pub(super) struct EventsH {
    pub ctx: Rc<Ctx>,
    pub scope: &'static str,
}

impl UserData for EventsH {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("on", |_, this, (name, f): (String, Function)| {
            if event_bit(this.scope, &name).is_none() {
                return Err(Error::runtime(format!("unknown event '{name}'")));
            }
            let rt = this.ctx.rt.borrow().clone();
            // Without a front end (command line) nothing can fire; accept the call and do nothing.
            match rt {
                Some(rt) => rt.add_listener(this.scope, name, f),
                None => Ok(0),
            }
        });
        m.add_method("off", |_, this, what: Value| {
            let rt = this.ctx.rt.borrow().clone();
            match (what, rt) {
                (Value::Function(f), Some(rt)) => rt.remove_listener(this.scope, None, Some(&f)),
                (Value::Integer(i), Some(rt)) if i > 0 => rt.remove_listener(this.scope, Some(i as u64), None),
                (Value::Number(n), Some(rt)) if n >= 1.0 && n.fract() == 0.0 => {
                    rt.remove_listener(this.scope, Some(n as u64), None)
                }
                (Value::Function(_) | Value::Integer(_) | Value::Number(_), None) => {}
                _ => return Err(Error::runtime("events:off expects a listener id or a function")),
            }
            Ok(())
        });
    }
}
