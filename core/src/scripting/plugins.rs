//! Lua plugins: a script that is loaded once (not run), defines `init(plugin)` and `exit(plugin)`, adds
//! commands with `plugin:newCommand{...}` and keeps settings in `plugin.preferences`
//! (docs/LUA_DIALOG.md section 1.3).
//!
//! A plugin is a session (host.rs) that stays alive until it is stopped. Its commands are announced to the
//! front end with `command.add` / `command.update` / `command.remove`; a `command` message runs the
//! command's `onclick` on the active document as one undo step. `plugin.preferences` is an ordinary Lua
//! table that is read from a JSON file before `init` and written back after every transaction that changed
//! it and after `exit`.

use super::host::{DocSnap, Host, Rt, Spec};
use super::Ctx;
use crate::formats::json::{self, Value as J};
use mlua::{
    AnyUserData, Error, Function, Lua, MultiValue, Result as LuaResult, Table, UserData, UserDataFields,
    UserDataMethods, Value,
};
use std::rc::Rc;

/// Largest `plugin.preferences` file (read and written), and deepest nesting stored.
pub(super) const MAX_PREFS: usize = 1 << 20;
pub(super) const MAX_JSON_DEPTH: u32 = 16;
/// Nodes of a preferences table that are looked at when saving (guards against cycles and huge tables).
const MAX_PREFS_NODES: usize = 100_000;
/// Commands one plugin may register.
const MAX_COMMANDS: usize = 256;

pub(super) struct Command {
    id: String,
    title: String,
    onclick: Option<Function>,
    onenabled: Option<Function>,
    onchecked: Option<Function>,
    enabled: bool,
    checked: bool,
}

impl Command {
    /// True when the front end has to be told about changes of `onenabled` / `onchecked`.
    pub(super) fn wants_refresh(&self) -> bool {
        self.onenabled.is_some() || self.onchecked.is_some()
    }
}

pub(super) struct Prefs {
    table: Table,
    path: Option<String>,
    /// What was stored last (or found on disk), to skip writes that would change nothing.
    last: String,
    /// The "too large" warning is shown once.
    warned: bool,
}

impl Host {
    /// Loads a plugin: runs `source`, then its global `init(plugin)`. `dir` is `plugin.path`; `prefs_path`
    /// is the JSON file behind `plugin.preferences` (`None`: not persisted). Returns the session id.
    pub fn load_plugin(
        &self,
        source: &str,
        name: &str,
        dir: Option<&str>,
        prefs_path: Option<&str>,
    ) -> Result<u64, String> {
        let spec = Spec::Plugin {
            source: source.into(),
            name: name.into(),
            dir: dir.map(String::from),
            prefs_path: prefs_path.map(String::from),
        };
        self.start(spec, None)
    }

    /// Asks a plugin session to run one of its commands on `doc`. False when the session is gone.
    pub fn run_command(&self, session: u64, id: &str, doc: Option<DocSnap>) -> bool {
        let msg = J::Obj(vec![("type".into(), J::Str("command".into())), ("id".into(), J::Str(id.into()))]);
        self.send(session, msg, doc)
    }
}

impl Rt {
    /// A `command` message: run the command's `onclick`; its title names the undo step unless the script
    /// sets another label.
    pub(super) fn on_command(&self, lua: &Lua, ev: &J) -> LuaResult<()> {
        let id = ev.get("id").and_then(J::as_str).unwrap_or("");
        let found = self.commands.borrow().iter().find(|c| c.id == id).map(|c| (c.onclick.clone(), c.title.clone()));
        if let Some((Some(f), title)) = found {
            self.ctx().label.borrow_mut().get_or_insert(title);
            self.run_callback(lua, &f, ())?;
        }
        Ok(())
    }

    /// Re-evaluates `onenabled` / `onchecked` of the plugin's commands and reports what changed. They must
    /// not change the document: whatever they did to it is undone. An error counts as `false`.
    pub(super) fn refresh_commands(&self) {
        if !self.commands.borrow().iter().any(Command::wants_refresh) {
            return;
        }
        let ctx = self.ctx();
        let n = self.commands.borrow().len();
        for i in 0..n {
            let (id, oe, oc, was) = {
                let c = self.commands.borrow();
                let Some(c) = c.get(i) else { break };
                (c.id.clone(), c.onenabled.clone(), c.onchecked.clone(), (c.enabled, c.checked))
            };
            let eval = |f: &Option<Function>, default: bool| match f {
                Some(f) => {
                    let (sprite, mask) = (ctx.sprite().clone(), ctx.mask.borrow().clone());
                    let r =
                        f.call::<Value>(()).map(|v| !matches!(v, Value::Nil | Value::Boolean(false))).unwrap_or(false);
                    *ctx.sprite() = sprite;
                    *ctx.mask.borrow_mut() = mask;
                    r
                }
                None => default,
            };
            let enabled = eval(&oe, true);
            let checked = eval(&oc, false);
            if (enabled, checked) != was {
                if let Some(c) = self.commands.borrow_mut().get_mut(i) {
                    c.enabled = enabled;
                    c.checked = checked;
                }
                self.emit(
                    "command.update",
                    vec![(
                        "command",
                        J::Obj(vec![
                            ("id".into(), J::Str(id)),
                            ("enabled".into(), J::Bool(enabled)),
                            ("checked".into(), J::Bool(checked)),
                        ]),
                    )],
                );
            }
        }
    }

    /// Writes `plugin.preferences` when it differs from what is stored. Values that cannot be stored
    /// (functions, userdata, too deep, not finite) are left out; more than 1 MB is not written.
    pub(super) fn save_prefs(&self) {
        let mut guard = self.prefs.borrow_mut();
        let Some(p) = guard.as_mut() else { return };
        let mut budget = MAX_PREFS_NODES;
        let Some(j) = lua_to_json(&Value::Table(p.table.clone()), 0, &mut budget) else { return };
        let text = json::to_string(&j);
        if text == p.last {
            return;
        }
        if text.len() > MAX_PREFS {
            if !p.warned {
                p.warned = true;
                self.ctx().messages.borrow_mut().push(format!(
                    "warning: the plugin preferences are {} KB; over {} KB they are not saved",
                    text.len() / 1024,
                    MAX_PREFS / 1024
                ));
            }
            return;
        }
        p.warned = false;
        if let Some(path) = &p.path {
            if let Some(dir) = std::path::Path::new(path).parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            // A failed write is not retried until the preferences change again.
            let _ = crate::formats::durable::write_file(path, text.as_bytes());
        }
        p.last = text;
    }
}

/// The `plugin` argument of `init` / `exit`.
struct PluginH {
    ctx: Rc<Ctx>,
    name: String,
    path: String,
    prefs: Table,
}

impl UserData for PluginH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("name", |_, p| Ok(p.name.clone()));
        f.add_field_method_get("path", |_, p| Ok(p.path.clone()));
        f.add_field_method_get("preferences", |_, p| Ok(p.prefs.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("newCommand", |_, p, t: Table| {
            let rt = p.ctx.rt.borrow().clone().ok_or_else(|| Error::runtime("plugins need the Pixmith app"))?;
            let id: String = t.get::<Option<String>>("id")?.ok_or_else(|| Error::runtime("newCommand needs an id"))?;
            if id.is_empty() || id.len() > 256 {
                return Err(Error::runtime("newCommand needs an id of 1 to 256 characters"));
            }
            let title: String =
                t.get::<Option<String>>("title")?.ok_or_else(|| Error::runtime("newCommand needs a title"))?;
            let group = t.get::<Option<String>>("group")?.unwrap_or_else(|| "file_scripts".into());
            let func = |key: &str| -> LuaResult<Option<Function>> {
                match t.get::<Value>(key)? {
                    Value::Function(f) => Ok(Some(f)),
                    Value::Nil => Ok(None),
                    _ => Err(Error::runtime(format!("'{key}' must be a function"))),
                }
            };
            let cmd = Command {
                id: id.clone(),
                title: title.clone(),
                onclick: func("onclick")?,
                onenabled: func("onenabled")?,
                onchecked: func("onchecked")?,
                enabled: true,
                checked: false,
            };
            let existed = rt.commands.borrow().iter().any(|c| c.id == id);
            if !existed && rt.commands.borrow().len() >= MAX_COMMANDS {
                return Err(Error::runtime("too many commands"));
            }
            if existed {
                // A command with the same id is replaced: the front end sees it go and come back.
                rt.commands.borrow_mut().retain(|c| c.id != id);
                rt.emit("command.remove", vec![("id", J::Str(id.clone()))]);
            }
            rt.commands.borrow_mut().push(cmd);
            rt.update_listens();
            rt.emit(
                "command.add",
                vec![
                    ("plugin", J::Str(p.name.clone())),
                    (
                        "command",
                        J::Obj(vec![
                            ("id".into(), J::Str(id)),
                            ("title".into(), J::Str(title)),
                            ("group".into(), J::Str(group)),
                            ("enabled".into(), J::Bool(true)),
                            ("checked".into(), J::Bool(false)),
                        ]),
                    ),
                ],
            );
            Ok(())
        });
        m.add_method("deleteCommand", |_, p, id: String| {
            if let Some(rt) = p.ctx.rt.borrow().clone() {
                let before = rt.commands.borrow().len();
                rt.commands.borrow_mut().retain(|c| c.id != id);
                if rt.commands.borrow().len() != before {
                    rt.emit("command.remove", vec![("id", J::Str(id))]);
                }
                rt.update_listens();
            }
            Ok(())
        });
        for name in ["newMenuGroup", "newMenuSeparator"] {
            m.add_method(name, move |_, _, _: MultiValue| -> LuaResult<()> {
                Err(Error::runtime(format!("plugin:{name} is not supported by Pixmith")))
            });
        }
    }
}

/// Builds the `plugin` object: loads the preferences (an unreadable, oversized or non-object file counts as
/// empty) and remembers them for saving.
pub(super) fn create(
    rt: &Rc<Rt>,
    lua: &Lua,
    ctx: &Rc<Ctx>,
    name: &str,
    dir: Option<&str>,
    prefs_path: Option<&str>,
) -> LuaResult<AnyUserData> {
    let stored = prefs_path.and_then(read_prefs);
    let prefs = match &stored {
        Some(j) => match json_to_lua(lua, j, 0)? {
            Value::Table(t) => t,
            _ => lua.create_table()?,
        },
        None => lua.create_table()?,
    };
    *rt.prefs.borrow_mut() = Some(Prefs {
        table: prefs.clone(),
        path: prefs_path.map(String::from),
        last: stored.map(|j| json::to_string(&j)).unwrap_or_default(),
        warned: false,
    });
    lua.create_userdata(PluginH { ctx: ctx.clone(), name: name.into(), path: dir.unwrap_or_default().into(), prefs })
}

/// The global `init(plugin)`, if the script defined one.
pub(super) fn call_init(lua: &Lua, plugin: &AnyUserData) -> LuaResult<()> {
    if let Value::Function(init) = lua.globals().get::<Value>("init")? {
        init.call::<()>(plugin.clone())?;
    }
    Ok(())
}

/// Unloads a plugin: `exit(plugin)` as a transaction of its own (a failing `exit` is reported), then the
/// preferences are written one last time.
pub(super) fn unload(rt: &Rc<Rt>, lua: &Lua, plugin: &AnyUserData) {
    rt.set_state("running");
    let _ = rt.run_txn(lua, None, || {
        if let Value::Function(exit) = lua.globals().get::<Value>("exit")? {
            exit.call::<()>(plugin.clone())?;
        }
        Ok(())
    });
    rt.save_prefs();
}

/// Removes every command the plugin still has (on unload) and tells the front end.
pub(super) fn drop_commands(rt: &Rt) {
    for c in rt.commands.borrow_mut().drain(..) {
        rt.emit("command.remove", vec![("id", J::Str(c.id))]);
    }
}

fn read_prefs(path: &str) -> Option<J> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() as usize > MAX_PREFS {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    json::parse(&text).ok().filter(|j| matches!(j, J::Obj(_)))
}

// MARK: JSON <-> Lua

pub(super) fn json_to_lua(lua: &Lua, v: &J, depth: u32) -> LuaResult<Value> {
    Ok(match v {
        J::Null => Value::Nil,
        J::Bool(b) => Value::Boolean(*b),
        J::Num(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Value::Integer(*n as i64),
        J::Num(n) => Value::Number(*n),
        J::Str(s) => Value::String(lua.create_string(s)?),
        J::Arr(a) if depth < MAX_JSON_DEPTH => {
            let t = lua.create_table()?;
            for (i, x) in a.iter().enumerate() {
                t.raw_set(i + 1, json_to_lua(lua, x, depth + 1)?)?;
            }
            Value::Table(t)
        }
        J::Obj(m) if depth < MAX_JSON_DEPTH => {
            let t = lua.create_table()?;
            for (k, x) in m {
                t.raw_set(k.as_str(), json_to_lua(lua, x, depth + 1)?)?;
            }
            Value::Table(t)
        }
        _ => Value::Nil,
    })
}

/// A Lua value as JSON: booleans, finite numbers, strings and tables of them. `None` means "leave it out":
/// nil, functions, userdata, too deep, or past the node budget. Tables whose keys are exactly `1..n` become
/// arrays, others objects (integer keys turn into strings).
pub(super) fn lua_to_json(v: &Value, depth: u32, budget: &mut usize) -> Option<J> {
    *budget = budget.checked_sub(1)?;
    Some(match v {
        Value::Boolean(b) => J::Bool(*b),
        Value::Integer(i) => J::Num(*i as f64),
        Value::Number(n) if n.is_finite() => J::Num(*n),
        Value::String(s) => J::Str(s.to_string_lossy()),
        Value::Table(t) if depth < MAX_JSON_DEPTH => {
            let len = t.raw_len();
            let pairs: Vec<(Value, Value)> = t.pairs::<Value, Value>().filter_map(Result::ok).collect();
            let is_array = len > 0
                && pairs.len() == len
                && pairs.iter().all(|(k, _)| matches!(k, Value::Integer(i) if *i >= 1 && *i as usize <= len));
            if is_array {
                let mut arr = Vec::with_capacity(len);
                for i in 1..=len {
                    let x: Value = t.raw_get(i).ok()?;
                    arr.push(lua_to_json(&x, depth + 1, budget).unwrap_or(J::Null));
                }
                J::Arr(arr)
            } else {
                let mut o = Vec::new();
                for (k, x) in pairs {
                    let key = match k {
                        Value::String(s) => s.to_string_lossy(),
                        Value::Integer(i) => i.to_string(),
                        Value::Number(n) if n.is_finite() => n.to_string(),
                        _ => continue,
                    };
                    if let Some(j) = lua_to_json(&x, depth + 1, budget) {
                        o.push((key, j));
                    }
                }
                J::Obj(o)
            }
        }
        _ => return None,
    })
}
