//! The Lua `Dialog` object: a description of widgets the front end renders (docs/LUA_DIALOG.md).
//!
//! A dialog is plain data here (`DialogState`): rows of widgets with their current values. The script
//! builds it with the chained methods, `show` hands the description to the front end as a
//! `dialog.open` message, and widget events coming back update the values and call the Lua callbacks.

use super::host::Rt;
use super::Ctx;
use crate::formats::json::Value as J;
use mlua::{
    AnyUserData, Error, Function, Lua, Result as LuaResult, Table, UserData, UserDataFields, UserDataMethods, Value,
};
use std::cell::RefCell;
use std::rc::Rc;

/// Widgets in one dialog, and the longest text a widget may carry (the description is sent as JSON).
const MAX_WIDGETS: usize = 512;
const MAX_TEXT: usize = 1 << 16;
const MAX_OPTIONS: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
    Label,
    Button,
    Check,
    Radio,
    Entry,
    Number,
    Slider,
    Color,
    Combo,
    File,
    Shades,
    Separator,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Label => "label",
            Kind::Button => "button",
            Kind::Check => "check",
            Kind::Radio => "radio",
            Kind::Entry => "entry",
            Kind::Number => "number",
            Kind::Slider => "slider",
            Kind::Color => "color",
            Kind::Combo => "combobox",
            Kind::File => "file",
            Kind::Shades => "shades",
            Kind::Separator => "separator",
        }
    }

    /// Widgets that stretch over the free width of their row unless told otherwise.
    fn expands(self) -> bool {
        matches!(self, Kind::Entry | Kind::Number | Kind::Slider | Kind::Color | Kind::Combo | Kind::File)
    }
}

pub(super) struct Widget {
    pub id: String,
    pub kind: Kind,
    pub row: usize,
    pub label: Option<String>,
    pub enabled: bool,
    pub visible: bool,
    pub focus: bool,
    pub hexpand: bool,
    pub text: String,
    pub selected: bool,
    pub value: f64,
    pub min: i64,
    pub max: i64,
    pub decimals: u32,
    pub color: u32,
    pub colors: Vec<u32>,
    pub options: Vec<String>,
    pub option: Option<String>,
    pub filename: String,
    pub open: bool,
    pub save: bool,
    pub filetypes: Vec<String>,
    pub title: String,
    pub group: usize,
    pub onclick: Option<Function>,
    pub onchange: Option<Function>,
    pub onrelease: Option<Function>,
}

fn num(n: impl Into<f64>) -> J {
    J::Num(n.into())
}

fn s(v: &str) -> J {
    J::Str(v.to_string())
}

fn strings(v: &[String]) -> J {
    J::Arr(v.iter().map(|x| s(x)).collect())
}

fn colors(v: &[u32]) -> J {
    J::Arr(v.iter().map(|&c| num(c)).collect())
}

impl Widget {
    fn new(id: String, kind: Kind, row: usize) -> Widget {
        Widget {
            id,
            kind,
            row,
            label: None,
            enabled: true,
            visible: true,
            focus: false,
            hexpand: kind.expands(),
            text: String::new(),
            selected: false,
            value: 0.0,
            min: 0,
            max: 100,
            decimals: 0,
            color: 0,
            colors: Vec::new(),
            options: Vec::new(),
            option: None,
            filename: String::new(),
            open: true,
            save: false,
            filetypes: Vec::new(),
            title: String::new(),
            group: row,
            onclick: None,
            onchange: None,
            onrelease: None,
        }
    }

    /// The widget as the front end sees it.
    fn to_json(&self) -> J {
        let mut o: Vec<(String, J)> = vec![("id".into(), s(&self.id)), ("kind".into(), s(self.kind.name()))];
        if let Some(l) = &self.label {
            o.push(("label".into(), s(l)));
        }
        let mut put = |k: &str, v: J| o.push((k.to_string(), v));
        match self.kind {
            Kind::Separator => {
                put("text", s(&self.text));
            }
            _ => {
                put("enabled", J::Bool(self.enabled));
                put("visible", J::Bool(self.visible));
                if self.focus {
                    put("focus", J::Bool(true));
                }
                put("hexpand", J::Bool(self.hexpand));
            }
        }
        match self.kind {
            Kind::Label | Kind::Button | Kind::Entry => put("text", s(&self.text)),
            Kind::Check => {
                put("text", s(&self.text));
                put("selected", J::Bool(self.selected));
            }
            Kind::Radio => {
                put("text", s(&self.text));
                put("selected", J::Bool(self.selected));
                put("group", num(self.group as f64));
            }
            Kind::Number => {
                put("value", num(self.value));
                put("decimals", num(self.decimals));
            }
            Kind::Slider => {
                put("value", num(self.value));
                put("min", num(self.min as f64));
                put("max", num(self.max as f64));
            }
            Kind::Color => put("color", num(self.color)),
            Kind::Combo => {
                put("options", strings(&self.options));
                put("option", self.option.as_deref().map_or(J::Null, s));
            }
            Kind::File => {
                put("filename", s(&self.filename));
                put("open", J::Bool(self.open));
                put("save", J::Bool(self.save));
                put("filetypes", strings(&self.filetypes));
                put("title", s(&self.title));
            }
            Kind::Shades => put("colors", colors(&self.colors)),
            Kind::Separator => {}
        }
        J::Obj(o)
    }

    /// The value `dlg.data[id]` shows, as a Lua value (labels and the like have none).
    fn data(&self, lua: &Lua, closer: Option<&str>) -> LuaResult<Value> {
        Ok(match self.kind {
            Kind::Button => Value::Boolean(closer == Some(self.id.as_str())),
            Kind::Check | Kind::Radio => Value::Boolean(self.selected),
            Kind::Entry => Value::String(lua.create_string(&self.text)?),
            Kind::Number if self.decimals == 0 => Value::Integer(self.value.round() as i64),
            Kind::Number => Value::Number(self.value),
            Kind::Slider => Value::Integer(self.value as i64),
            Kind::Color => Value::Integer(self.color as i64),
            Kind::Combo => match &self.option {
                Some(o) => Value::String(lua.create_string(o)?),
                None => Value::Nil,
            },
            Kind::File => Value::String(lua.create_string(&self.filename)?),
            Kind::Label | Kind::Shades | Kind::Separator => Value::Nil,
        })
    }

    /// Sets one property from a Lua value (constructor options, `:modify`, `dlg.data = ...`) and returns the
    /// front-end fields that changed. Unknown keys are ignored (Aseprite parity); wrong types raise errors.
    fn set(&mut self, key: &str, v: &Value) -> LuaResult<Vec<(String, J)>> {
        let mut out: Vec<(String, J)> = Vec::new();
        let mut note = |k: &str, j: J| out.push((k.to_string(), j));
        let kind = self.kind;
        match key {
            "label" => {
                let t = text_arg(key, v)?;
                self.label = (!t.is_empty()).then_some(t.clone());
                note("label", s(&t));
            }
            "enabled" => {
                self.enabled = bool_arg(key, v)?;
                note("enabled", J::Bool(self.enabled));
            }
            "visible" => {
                self.visible = bool_arg(key, v)?;
                note("visible", J::Bool(self.visible));
            }
            "focus" => {
                self.focus = bool_arg(key, v)?;
                note("focus", J::Bool(self.focus));
            }
            "hexpand" => {
                self.hexpand = bool_arg(key, v)?;
                note("hexpand", J::Bool(self.hexpand));
            }
            "text" if kind == Kind::Number => return self.set("value", v),
            "text"
                if matches!(
                    kind,
                    Kind::Label | Kind::Button | Kind::Check | Kind::Radio | Kind::Entry | Kind::Separator
                ) =>
            {
                self.text = text_arg(key, v)?;
                note("text", s(&self.text));
            }
            "selected" if matches!(kind, Kind::Check | Kind::Radio) => {
                self.selected = bool_arg(key, v)?;
                note("selected", J::Bool(self.selected));
            }
            "value" if kind == Kind::Number => {
                self.value = number_arg(key, v)?;
                note("value", num(self.value));
            }
            "value" if kind == Kind::Slider => {
                self.value = (number_arg(key, v)?.round()).clamp(self.min as f64, self.max as f64);
                note("value", num(self.value));
            }
            "min" | "max" if kind == Kind::Slider => {
                let n = number_arg(key, v)?.round().clamp(-1e9, 1e9) as i64;
                if key == "min" {
                    self.min = n;
                    self.max = self.max.max(n);
                } else {
                    self.max = n;
                    self.min = self.min.min(n);
                }
                let clamped = self.value.clamp(self.min as f64, self.max as f64);
                note("min", num(self.min as f64));
                note("max", num(self.max as f64));
                if clamped != self.value {
                    self.value = clamped;
                    note("value", num(self.value));
                }
            }
            "decimals" if kind == Kind::Number => {
                self.decimals = number_arg(key, v)?.clamp(0.0, 6.0) as u32;
                note("decimals", num(self.decimals));
            }
            "color" if kind == Kind::Color => {
                self.color = color_arg(key, v)?;
                note("color", num(self.color));
            }
            "colors" if kind == Kind::Shades => {
                self.colors = list_arg(key, v, |x| color_arg(key, x))?;
                note("colors", colors(&self.colors));
            }
            "options" if kind == Kind::Combo => {
                self.options = list_arg(key, v, |x| text_arg(key, x))?;
                if self.option.as_ref().is_none_or(|o| !self.options.contains(o)) {
                    self.option = self.options.first().cloned();
                }
                note("options", strings(&self.options));
                note("option", self.option.as_deref().map_or(J::Null, s));
            }
            "option" if kind == Kind::Combo => {
                let o = text_arg(key, v)?;
                if !self.options.contains(&o) {
                    return Err(Error::runtime(format!("option '{o}' is not one of the combobox options")));
                }
                self.option = Some(o);
                note("option", self.option.as_deref().map_or(J::Null, s));
            }
            "filename" if kind == Kind::File => {
                self.filename = text_arg(key, v)?;
                note("filename", s(&self.filename));
            }
            "open" if kind == Kind::File => {
                self.open = bool_arg(key, v)?;
                note("open", J::Bool(self.open));
            }
            "save" if kind == Kind::File => {
                self.save = bool_arg(key, v)?;
                if self.save {
                    self.open = false;
                    note("open", J::Bool(false));
                }
                note("save", J::Bool(self.save));
            }
            "filetypes" if kind == Kind::File => {
                self.filetypes = list_arg(key, v, |x| text_arg(key, x))?;
                note("filetypes", strings(&self.filetypes));
            }
            "title" if kind == Kind::File => {
                self.title = text_arg(key, v)?;
                note("title", s(&self.title));
            }
            _ => {}
        }
        Ok(out)
    }

    /// The property `dlg.data = { id = value }` assigns for this kind of widget.
    fn data_key(&self) -> Option<&'static str> {
        match self.kind {
            Kind::Check | Kind::Radio => Some("selected"),
            Kind::Entry => Some("text"),
            Kind::Number | Kind::Slider => Some("value"),
            Kind::Color => Some("color"),
            Kind::Combo => Some("option"),
            Kind::File => Some("filename"),
            _ => None,
        }
    }
}

fn text_arg(key: &str, v: &Value) -> LuaResult<String> {
    let t = match v {
        Value::String(x) => x.to_string_lossy(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Nil => String::new(),
        _ => return Err(Error::runtime(format!("'{key}' must be a string"))),
    };
    if t.len() > MAX_TEXT {
        return Err(Error::runtime(format!("'{key}' is too long")));
    }
    Ok(t)
}

fn bool_arg(key: &str, v: &Value) -> LuaResult<bool> {
    match v {
        Value::Boolean(b) => Ok(*b),
        Value::Nil => Ok(false),
        _ => Err(Error::runtime(format!("'{key}' must be a boolean"))),
    }
}

fn number_arg(key: &str, v: &Value) -> LuaResult<f64> {
    let n = match v {
        Value::Integer(i) => *i as f64,
        Value::Number(n) => *n,
        Value::String(x) => x.to_string_lossy().trim().parse::<f64>().unwrap_or(f64::NAN),
        _ => f64::NAN,
    };
    if n.is_finite() {
        Ok(n)
    } else {
        Err(Error::runtime(format!("'{key}' must be a number")))
    }
}

fn color_arg(key: &str, v: &Value) -> LuaResult<u32> {
    let n = match v {
        Value::Integer(i) => Some(*i),
        Value::Number(n) if n.fract() == 0.0 => Some(*n as i64),
        _ => None,
    };
    n.and_then(|n| u32::try_from(n).ok()).ok_or_else(|| Error::runtime(format!("'{key}' must be a color (0xRRGGBBAA)")))
}

fn list_arg<T>(key: &str, v: &Value, mut f: impl FnMut(&Value) -> LuaResult<T>) -> LuaResult<Vec<T>> {
    let Value::Table(t) = v else { return Err(Error::runtime(format!("'{key}' must be a list"))) };
    let mut out = Vec::new();
    for item in t.sequence_values::<Value>() {
        if out.len() >= MAX_OPTIONS {
            return Err(Error::runtime(format!("'{key}' has too many entries")));
        }
        out.push(f(&item?)?);
    }
    Ok(out)
}

pub(super) struct DialogState {
    pub id: String,
    pub title: String,
    pub notitlebar: bool,
    pub widgets: Vec<Widget>,
    row: usize,
    row_used: bool,
    pub open: bool,
    pub modal: bool,
    /// The button that closed the dialog (`data[id]` is true for it).
    pub closer: Option<String>,
    pub onclose: Option<Function>,
    next_auto: u32,
    pub bounds: Option<(i64, i64, i64, i64)>,
}

pub(super) type Dlg = Rc<RefCell<DialogState>>;

impl DialogState {
    fn new(id: String) -> Self {
        DialogState {
            id,
            title: String::new(),
            notitlebar: false,
            widgets: Vec::new(),
            row: 0,
            row_used: false,
            open: false,
            modal: false,
            closer: None,
            onclose: None,
            next_auto: 0,
            bounds: None,
        }
    }

    /// The whole dialog for a `dialog.open` message.
    pub fn to_json(&self) -> J {
        let mut rows: Vec<J> = Vec::new();
        let mut i = 0;
        while i < self.widgets.len() {
            let row = self.widgets[i].row;
            let mut ws = Vec::new();
            while i < self.widgets.len() && self.widgets[i].row == row {
                ws.push(self.widgets[i].to_json());
                i += 1;
            }
            rows.push(J::Obj(vec![("widgets".into(), J::Arr(ws))]));
        }
        J::Obj(vec![
            ("id".into(), s(&self.id)),
            ("title".into(), s(&self.title)),
            ("modal".into(), J::Bool(self.modal)),
            ("notitlebar".into(), J::Bool(self.notitlebar)),
            ("rows".into(), J::Arr(rows)),
        ])
    }

    pub fn widget_index(&self, id: &str) -> Option<usize> {
        self.widgets.iter().position(|w| w.id == id)
    }

    fn add(&mut self, kind: Kind, opts: Option<&Table>, name: &str) -> LuaResult<()> {
        if self.widgets.len() >= MAX_WIDGETS {
            return Err(Error::runtime("too many widgets in one dialog"));
        }
        if kind == Kind::Separator && self.row_used {
            self.row += 1;
        }
        let id = match opts.map(|t| t.get::<Value>("id")).transpose()? {
            Some(Value::String(x)) => x.to_string_lossy(),
            Some(Value::Integer(i)) => i.to_string(),
            Some(Value::Nil) | None => {
                self.next_auto += 1;
                format!("_w{}", self.next_auto)
            }
            Some(_) => return Err(Error::runtime("'id' must be a string")),
        };
        if self.widget_index(&id).is_some() {
            return Err(Error::runtime(format!("{name}: duplicate widget id '{id}'")));
        }
        let mut w = Widget::new(id, kind, self.row);
        if let Some(t) = opts {
            let mut pairs: Vec<(String, Value)> = Vec::new();
            for p in t.pairs::<Value, Value>() {
                let (k, v) = p?;
                if let Value::String(k) = k {
                    pairs.push((k.to_string_lossy(), v));
                }
            }
            // Order matters for a few keys: bounds before the value, options before the option.
            let rank = |k: &str| match k {
                "options" | "min" | "max" | "decimals" => 0,
                "text" | "value" | "option" | "selected" | "color" => 1,
                _ => 2,
            };
            pairs.sort_by_key(|(k, _)| rank(k));
            for (k, v) in &pairs {
                match k.as_str() {
                    "id" => {}
                    "onclick" | "onchange" | "onrelease" => {
                        let f = match v {
                            Value::Function(f) => Some(f.clone()),
                            Value::Nil => None,
                            _ => return Err(Error::runtime(format!("'{k}' must be a function"))),
                        };
                        match k.as_str() {
                            "onclick" => w.onclick = f,
                            "onchange" => w.onchange = f,
                            _ => w.onrelease = f,
                        }
                    }
                    _ => {
                        w.set(k, v)?;
                    }
                }
            }
        }
        if kind == Kind::Combo && w.option.is_none() {
            w.option = w.options.first().cloned();
        }
        if kind == Kind::Slider {
            w.value = w.value.clamp(w.min as f64, w.max as f64);
        }
        self.widgets.push(w);
        self.row_used = true;
        if kind == Kind::Separator {
            self.row += 1;
            self.row_used = false;
        }
        Ok(())
    }

    fn new_row(&mut self) {
        if self.row_used {
            self.row += 1;
            self.row_used = false;
        }
    }

    /// Radios of the same row form a group: selecting one clears the others, returning the patches.
    pub fn select_radio(&mut self, idx: usize) -> Vec<(String, Vec<(String, J)>)> {
        let group = self.widgets[idx].group;
        let mut patches = Vec::new();
        for (i, w) in self.widgets.iter_mut().enumerate() {
            if w.kind == Kind::Radio && w.group == group && i != idx && w.selected {
                w.selected = false;
                patches.push((w.id.clone(), vec![("selected".to_string(), J::Bool(false))]));
            }
        }
        self.widgets[idx].selected = true;
        patches
    }
}

/// `Dialog` as seen by Lua.
pub(super) struct DialogH {
    pub state: Dlg,
    pub ctx: Rc<Ctx>,
}

impl DialogH {
    fn rt(&self) -> Option<Rc<Rt>> {
        self.ctx.rt.borrow().clone()
    }

    /// Sends a `dialog.modify` message for an open dialog.
    fn patch(&self, patches: Vec<(String, Vec<(String, J)>)>, title: Option<&str>) {
        let (Some(rt), true) = (self.rt(), self.state.borrow().open) else { return };
        if patches.is_empty() && title.is_none() {
            return;
        }
        rt.emit_dialog_modify(&self.state.borrow().id, patches, title);
    }
}

fn pick_ids(t: &Table) -> LuaResult<Vec<String>> {
    match t.get::<Value>("id")? {
        Value::String(x) => Ok(vec![x.to_string_lossy()]),
        Value::Integer(i) => Ok(vec![i.to_string()]),
        Value::Table(l) => l.sequence_values::<String>().collect(),
        Value::Nil => Ok(Vec::new()),
        _ => Err(Error::runtime("'id' must be a string or a list of strings")),
    }
}

impl UserData for DialogH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("data", |lua, d| {
            let st = d.state.borrow();
            let t = lua.create_table()?;
            for w in &st.widgets {
                let v = w.data(lua, st.closer.as_deref())?;
                if !matches!(v, Value::Nil) {
                    t.set(w.id.as_str(), v)?;
                }
            }
            Ok(t)
        });
        f.add_field_method_set("data", |_, d, t: Table| {
            let mut all = Vec::new();
            for p in t.pairs::<Value, Value>() {
                let (k, v) = p?;
                let Value::String(k) = k else { continue };
                let id = k.to_string_lossy();
                let mut st = d.state.borrow_mut();
                let Some(i) = st.widget_index(&id) else { continue };
                let Some(key) = st.widgets[i].data_key() else { continue };
                let mut changed = st.widgets[i].set(key, &v)?;
                if st.widgets[i].kind == Kind::Radio && st.widgets[i].selected {
                    all.extend(st.select_radio(i));
                }
                if st.widgets[i].kind == Kind::Slider || st.widgets[i].kind == Kind::Number {
                    changed.retain(|(k, _)| k == "value");
                }
                all.push((id, changed));
            }
            d.patch(all, None);
            Ok(())
        });
        f.add_field_method_get("bounds", |lua, d| {
            let st = d.state.borrow();
            match st.bounds {
                Some((x, y, w, h)) => {
                    let t = lua.create_table()?;
                    for (k, v) in [("x", x), ("y", y), ("width", w), ("height", h), ("w", w), ("h", h)] {
                        t.set(k, v)?;
                    }
                    Ok(Value::Table(t))
                }
                None => Ok(Value::Nil),
            }
        });
        f.add_field_method_set("bounds", |_, d, t: Option<Table>| {
            d.state.borrow_mut().bounds = match t {
                Some(t) => Some((
                    t.get::<Option<i64>>("x")?.unwrap_or(0),
                    t.get::<Option<i64>>("y")?.unwrap_or(0),
                    t.get::<Option<i64>>("width").or_else(|_| t.get::<Option<i64>>("w"))?.unwrap_or(0),
                    t.get::<Option<i64>>("height").or_else(|_| t.get::<Option<i64>>("h"))?.unwrap_or(0),
                )),
                None => None,
            };
            Ok(())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        macro_rules! builder {
            ($name:literal, $kind:expr) => {
                m.add_function($name, |_, (ud, opts): (AnyUserData, Option<Table>)| {
                    {
                        let d = ud.borrow::<DialogH>()?;
                        if d.state.borrow().open {
                            return Err(Error::runtime("widgets cannot be added while the dialog is shown"));
                        }
                        d.state.borrow_mut().add($kind, opts.as_ref(), $name)?;
                    }
                    Ok(ud)
                });
            };
        }
        builder!("label", Kind::Label);
        builder!("button", Kind::Button);
        builder!("check", Kind::Check);
        builder!("radio", Kind::Radio);
        builder!("entry", Kind::Entry);
        builder!("number", Kind::Number);
        builder!("slider", Kind::Slider);
        builder!("color", Kind::Color);
        builder!("combobox", Kind::Combo);
        builder!("separator", Kind::Separator);
        builder!("file", Kind::File);
        builder!("shades", Kind::Shades);

        m.add_function("newrow", |_, (ud, _opts): (AnyUserData, Option<Table>)| {
            ud.borrow::<DialogH>()?.state.borrow_mut().new_row();
            Ok(ud)
        });

        for name in ["canvas", "image", "tab", "endtabs", "menu", "menuItem", "endmenu"] {
            m.add_function(name, move |_, _: mlua::MultiValue| -> LuaResult<()> {
                Err(Error::runtime(format!("Dialog:{name} is not supported by Pixmith")))
            });
        }

        m.add_method("repaint", |_, _, ()| Ok(()));

        m.add_method("modify", |_, d, t: Table| {
            let ids = pick_ids(&t)?;
            let mut title = None;
            let mut patches: Vec<(String, Vec<(String, J)>)> = Vec::new();
            let mut pairs: Vec<(String, Value)> = Vec::new();
            for p in t.pairs::<Value, Value>() {
                if let (Value::String(k), v) = p? {
                    let k = k.to_string_lossy();
                    if k != "id" {
                        pairs.push((k, v));
                    }
                }
            }
            let rank = |k: &str| match k {
                "options" | "min" | "max" | "decimals" => 0,
                _ => 1,
            };
            pairs.sort_by_key(|(k, _)| rank(k));
            if ids.is_empty() {
                for (k, v) in &pairs {
                    if k == "title" {
                        let t = text_arg("title", v)?;
                        d.state.borrow_mut().title = t.clone();
                        title = Some(t);
                    }
                }
            }
            for id in &ids {
                let mut st = d.state.borrow_mut();
                let i = st.widget_index(id).ok_or_else(|| Error::runtime(format!("no widget with id '{id}'")))?;
                let mut changed: Vec<(String, J)> = Vec::new();
                for (k, v) in &pairs {
                    match k.as_str() {
                        "onclick" | "onchange" | "onrelease" => {
                            let f = if let Value::Function(f) = v { Some(f.clone()) } else { None };
                            match k.as_str() {
                                "onclick" => st.widgets[i].onclick = f,
                                "onchange" => st.widgets[i].onchange = f,
                                _ => st.widgets[i].onrelease = f,
                            }
                        }
                        _ => changed.extend(st.widgets[i].set(k, v)?),
                    }
                }
                if st.widgets[i].kind == Kind::Radio && st.widgets[i].selected {
                    patches.extend(st.select_radio(i));
                }
                patches.push((id.clone(), changed));
            }
            d.patch(patches, title.as_deref());
            Ok(())
        });

        m.add_function("show", |lua, (ud, opts): (AnyUserData, Option<Table>)| {
            let wait = match &opts {
                Some(t) => t.get::<Option<bool>>("wait")?.unwrap_or(true),
                None => true,
            };
            let (state, ctx) = {
                let d = ud.borrow::<DialogH>()?;
                (d.state.clone(), d.ctx.clone())
            };
            if state.borrow().open {
                return Err(Error::runtime("this dialog is already shown"));
            }
            let rt = ctx.rt.borrow().clone();
            let Some(rt) = rt.filter(|r| r.ui_available()) else {
                // No front end (command line, plain script run): behave as if the user closed it at once.
                return Ok(ud);
            };
            {
                let mut st = state.borrow_mut();
                st.open = true;
                st.modal = wait;
                st.closer = None;
            }
            rt.open_dialog(&state);
            if wait {
                let st = state.clone();
                rt.pump(lua, move || st.borrow().open)?;
            }
            Ok(ud)
        });

        m.add_method("close", |lua, d, ()| {
            let Some(rt) = d.rt() else { return Ok(()) };
            rt.close_dialog(lua, &d.state, true)
        });
    }
}

/// `Dialog{...}` / `Dialog("title")`.
pub(super) fn create(lua: &Lua, ctx: &Rc<Ctx>, arg: Value) -> LuaResult<AnyUserData> {
    let id = match ctx.rt.borrow().as_ref() {
        Some(rt) => rt.next_dialog_id(),
        None => "d0".to_string(),
    };
    let mut st = DialogState::new(id);
    match arg {
        Value::Nil => {}
        Value::String(t) => st.title = t.to_string_lossy(),
        Value::Table(t) => {
            if let Some(title) = t.get::<Option<Value>>("title")? {
                st.title = text_arg("title", &title)?;
            }
            st.notitlebar = t.get::<Option<bool>>("notitlebar")?.unwrap_or(false);
            match t.get::<Value>("onclose")? {
                Value::Function(f) => st.onclose = Some(f),
                Value::Nil => {}
                _ => return Err(Error::runtime("'onclose' must be a function")),
            }
        }
        _ => return Err(Error::runtime("Dialog expects a title or a table of options")),
    }
    lua.create_userdata(DialogH { state: Rc::new(RefCell::new(st)), ctx: ctx.clone() })
}

/// Applies a front-end event to a widget: updates its value and says which Lua callback to run.
pub(super) enum Reaction {
    None,
    Callback(Function, Option<(&'static str, Value)>),
    /// A button without `onclick`: closes the dialog.
    Close,
}

/// What the front end reported for a widget (`event` is "click", "change" or "release").
#[allow(clippy::needless_late_init)]
pub(super) fn apply_widget_event(
    st: &mut DialogState,
    id: &str,
    event: &str,
    value: Option<&J>,
    color: Option<u32>,
) -> (Reaction, Vec<(String, Vec<(String, J)>)>, Option<Value>) {
    let mut patches = Vec::new();
    let Some(i) = st.widget_index(id) else { return (Reaction::None, patches, None) };
    if !st.widgets[i].enabled {
        return (Reaction::None, patches, None);
    }
    let kind = st.widgets[i].kind;
    let as_num = value.and_then(|v| v.as_f64());
    let as_bool = |v: Option<&J>| matches!(v, Some(J::Bool(true)));
    let evalue: Option<Value>;
    match (kind, event) {
        (Kind::Button, "click") => {
            evalue = None;
            return match st.widgets[i].onclick.clone() {
                Some(f) => (Reaction::Callback(f, None), patches, evalue),
                None => {
                    st.closer = Some(id.to_string());
                    (Reaction::Close, patches, evalue)
                }
            };
        }
        (Kind::Check, "click" | "change") => {
            st.widgets[i].selected = value.map_or(!st.widgets[i].selected, |v| as_bool(Some(v)));
            evalue = Some(Value::Boolean(st.widgets[i].selected));
            let f = st.widgets[i].onclick.clone().or_else(|| st.widgets[i].onchange.clone());
            return (f.map_or(Reaction::None, |f| Reaction::Callback(f, None)), patches, evalue);
        }
        (Kind::Radio, "click" | "change") => {
            patches = st.select_radio(i);
            evalue = Some(Value::Boolean(true));
            let f = st.widgets[i].onclick.clone().or_else(|| st.widgets[i].onchange.clone());
            return (f.map_or(Reaction::None, |f| Reaction::Callback(f, None)), patches, evalue);
        }
        (Kind::Shades, "click") => {
            let f = st.widgets[i].onclick.clone();
            let c = color.unwrap_or(0);
            return (
                f.map_or(Reaction::None, |f| Reaction::Callback(f, Some(("color", Value::Integer(c as i64))))),
                patches,
                Some(Value::Integer(c as i64)),
            );
        }
        (Kind::Slider, "release") => {
            let f = st.widgets[i].onrelease.clone();
            let v = st.widgets[i].value as i64;
            return (f.map_or(Reaction::None, |f| Reaction::Callback(f, None)), patches, Some(Value::Integer(v)));
        }
        (Kind::Entry, "change") => {
            let Some(t) = value.and_then(|v| v.as_str()) else { return (Reaction::None, patches, None) };
            st.widgets[i].text = t.chars().take(MAX_TEXT).collect();
            evalue = Some(Value::Nil); // filled by the caller from the text
        }
        (Kind::Number, "change") => {
            let Some(n) = as_num.filter(|n| n.is_finite()) else { return (Reaction::None, patches, None) };
            st.widgets[i].value = n;
            evalue = Some(Value::Number(n));
        }
        (Kind::Slider, "change") => {
            let Some(n) = as_num.filter(|n| n.is_finite()) else { return (Reaction::None, patches, None) };
            st.widgets[i].value = n.round().clamp(st.widgets[i].min as f64, st.widgets[i].max as f64);
            evalue = Some(Value::Integer(st.widgets[i].value as i64));
        }
        (Kind::Color, "change") => {
            let Some(n) = as_num.filter(|n| (0.0..=4294967295.0).contains(n)) else {
                return (Reaction::None, patches, None);
            };
            st.widgets[i].color = n as u32;
            evalue = Some(Value::Integer(n as i64));
        }
        (Kind::Combo, "change") => {
            let Some(t) = value.and_then(|v| v.as_str()) else { return (Reaction::None, patches, None) };
            if !st.widgets[i].options.iter().any(|o| o == t) {
                return (Reaction::None, patches, None);
            }
            st.widgets[i].option = Some(t.to_string());
            evalue = Some(Value::Nil);
        }
        (Kind::File, "change") => {
            let Some(t) = value.and_then(|v| v.as_str()) else { return (Reaction::None, patches, None) };
            st.widgets[i].filename = t.chars().take(MAX_TEXT).collect();
            evalue = Some(Value::Nil);
        }
        _ => return (Reaction::None, patches, None),
    }
    let extra = (kind == Kind::Color).then(|| ("color", Value::Integer(st.widgets[i].color as i64)));
    let f = st.widgets[i].onchange.clone();
    (f.map_or(Reaction::None, |f| Reaction::Callback(f, extra)), patches, evalue)
}

/// `ev.value` for entry, combobox and file callbacks (the text itself).
pub(super) fn text_value(st: &DialogState, id: &str, lua: &Lua) -> Value {
    st.widget_index(id).map_or(Value::Nil, |i| {
        let w = &st.widgets[i];
        let t = match w.kind {
            Kind::Entry => Some(w.text.clone()),
            Kind::Combo => w.option.clone(),
            Kind::File => Some(w.filename.clone()),
            _ => None,
        };
        t.and_then(|t| lua.create_string(&t).ok()).map_or(Value::Nil, Value::String)
    })
}
