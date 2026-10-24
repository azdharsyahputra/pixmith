//! The dialog the core describes (`docs/LUA_DIALOG.md` section 2.1) as plain Rust data, the patches that
//! `dialog.modify` applies to it, and the events the renderer sends back (section 2.2).
//!
//! Nothing here draws or calls the core, so the whole model is unit tested with JSON taken from the contract.

use pixmith_core::formats::json::{self, Value};

/// The widget kinds of the contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Label,
    Button,
    Check,
    Radio,
    Entry,
    Number,
    Slider,
    Color,
    Combobox,
    File,
    Shades,
    Separator,
}

impl Kind {
    fn parse(name: &str) -> Option<Kind> {
        Some(match name {
            "label" => Kind::Label,
            "button" => Kind::Button,
            "check" => Kind::Check,
            "radio" => Kind::Radio,
            "entry" => Kind::Entry,
            "number" => Kind::Number,
            "slider" => Kind::Slider,
            "color" => Kind::Color,
            "combobox" => Kind::Combobox,
            "file" => Kind::File,
            "shades" => Kind::Shades,
            "separator" => Kind::Separator,
            _ => return None,
        })
    }

    /// Whether the widget takes the free width of its row unless the script says otherwise.
    pub fn expands_by_default(self) -> bool {
        matches!(self, Kind::Entry | Kind::Number | Kind::Slider | Kind::Color | Kind::Combobox | Kind::File)
    }
}

/// One widget with every field any kind can have; a kind reads the ones it uses.
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    pub id: String,
    pub kind: Kind,
    pub label: Option<String>,
    pub enabled: bool,
    pub visible: bool,
    pub focus: bool,
    pub hexpand: bool,
    pub text: String,
    pub selected: bool,
    /// Radios with the same group are exclusive.
    pub group: i64,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub decimals: usize,
    pub color: u32,
    pub colors: Vec<u32>,
    pub options: Vec<String>,
    pub option: Option<String>,
    pub filename: String,
    pub open: bool,
    pub save: bool,
    pub filetypes: Vec<String>,
    pub title: String,
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(*n),
        _ => None,
    }
}

fn boolean(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

fn string_list(v: &Value) -> Option<Vec<String>> {
    Some(v.as_array()?.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
}

fn color_list(v: &Value) -> Option<Vec<u32>> {
    Some(v.as_array()?.iter().filter_map(num).map(|n| n as u32).collect())
}

impl Widget {
    fn blank(id: String, kind: Kind) -> Widget {
        Widget {
            id,
            kind,
            label: None,
            enabled: true,
            visible: true,
            focus: false,
            hexpand: kind.expands_by_default(),
            text: String::new(),
            selected: false,
            group: 0,
            value: 0.0,
            min: 0.0,
            max: 100.0,
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
        }
    }

    /// Parses one entry of `rows[].widgets[]`. Unknown kinds are skipped (a front end ignores what it does not
    /// know).
    pub fn from_json(v: &Value) -> Option<Widget> {
        let kind = Kind::parse(v.get("kind")?.as_str()?)?;
        let id = v.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        let mut w = Widget::blank(id, kind);
        w.patch(v);
        Some(w)
    }

    /// Overwrites the fields present in `p` (a full widget or a `dialog.modify` patch).
    pub fn patch(&mut self, p: &Value) {
        if let Some(Value::Str(s)) = p.get("label") {
            self.label = Some(s.clone());
        } else if matches!(p.get("label"), Some(Value::Null)) {
            self.label = None;
        }
        let set_bool = |name: &str, field: &mut bool| {
            if let Some(b) = p.get(name).and_then(boolean) {
                *field = b;
            }
        };
        let set_num = |name: &str, field: &mut f64| {
            if let Some(n) = p.get(name).and_then(num) {
                *field = n;
            }
        };
        set_bool("enabled", &mut self.enabled);
        set_bool("visible", &mut self.visible);
        set_bool("focus", &mut self.focus);
        set_bool("hexpand", &mut self.hexpand);
        set_bool("selected", &mut self.selected);
        set_bool("open", &mut self.open);
        set_bool("save", &mut self.save);
        set_num("value", &mut self.value);
        set_num("min", &mut self.min);
        set_num("max", &mut self.max);
        if let Some(n) = p.get("group").and_then(num) {
            self.group = n as i64;
        }
        if let Some(n) = p.get("decimals").and_then(num) {
            self.decimals = (n.max(0.0) as usize).min(6);
        }
        if let Some(n) = p.get("color").and_then(num) {
            self.color = n as u32;
        }
        if let Some(s) = p.get("text").and_then(Value::as_str) {
            self.text = s.to_string();
        }
        if let Some(s) = p.get("filename").and_then(Value::as_str) {
            self.filename = s.to_string();
        }
        if let Some(s) = p.get("title").and_then(Value::as_str) {
            self.title = s.to_string();
        }
        if let Some(c) = p.get("colors").and_then(color_list) {
            self.colors = c;
        }
        if let Some(o) = p.get("options").and_then(string_list) {
            self.options = o;
        }
        if let Some(t) = p.get("filetypes").and_then(string_list) {
            self.filetypes = t;
        }
        match p.get("option") {
            Some(Value::Str(s)) => self.option = Some(s.clone()),
            Some(Value::Null) => self.option = None,
            _ => {}
        }
    }

    /// Index of the selected option of a combobox.
    pub fn option_index(&self) -> Option<usize> {
        let current = self.option.as_deref()?;
        self.options.iter().position(|o| o == current)
    }

    /// The value a `change` event carries for this widget.
    pub fn change_value(&self) -> Value {
        match self.kind {
            Kind::Entry => Value::Str(self.text.clone()),
            Kind::Number | Kind::Slider => Value::Num(self.value),
            Kind::Color => Value::Num(f64::from(self.color)),
            Kind::Combobox => self.option.clone().map_or(Value::Null, Value::Str),
            Kind::File => Value::Str(self.filename.clone()),
            Kind::Check | Kind::Radio => Value::Bool(self.selected),
            _ => Value::Null,
        }
    }
}

/// A row of widgets laid out left to right.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub widgets: Vec<Widget>,
}

/// An open dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub id: String,
    pub session: u64,
    pub title: String,
    pub modal: bool,
    pub notitlebar: bool,
    pub rows: Vec<Row>,
}

impl Dialog {
    /// Parses the `dialog` object of a `dialog.open` message.
    pub fn from_json(session: u64, v: &Value) -> Option<Dialog> {
        let id = v.get("id")?.as_str()?.to_string();
        let rows = v
            .get("rows")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .map(|r| Row {
                widgets: r
                    .get("widgets")
                    .and_then(Value::as_array)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(Widget::from_json)
                    .collect(),
            })
            .collect();
        Some(Dialog {
            id,
            session,
            title: v.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
            modal: matches!(v.get("modal"), Some(Value::Bool(true))),
            notitlebar: matches!(v.get("notitlebar"), Some(Value::Bool(true))),
            rows,
        })
    }

    pub fn widget(&self, id: &str) -> Option<&Widget> {
        self.rows.iter().flat_map(|r| &r.widgets).find(|w| w.id == id)
    }

    pub fn widget_mut(&mut self, id: &str) -> Option<&mut Widget> {
        self.rows.iter_mut().flat_map(|r| &mut r.widgets).find(|w| w.id == id)
    }

    /// Applies a `dialog.modify` message; returns the ids whose `focus` was set by it.
    pub fn modify(&mut self, msg: &Value) -> Vec<String> {
        if let Some(t) = msg.get("title").and_then(Value::as_str) {
            self.title = t.to_string();
        }
        let mut focus = Vec::new();
        if let Some(patches) = msg.get("widgets").and_then(Value::as_object) {
            for (id, patch) in patches {
                if let Some(w) = self.widget_mut(id) {
                    w.patch(patch);
                    if matches!(patch.get("focus"), Some(Value::Bool(true))) {
                        focus.push(id.clone());
                    }
                }
            }
        }
        focus
    }

    /// Makes `id` the selected radio of its group, as the click that selects a radio does at once (the core
    /// confirms with patches).
    pub fn select_radio(&mut self, id: &str) {
        let Some(group) = self.widget(id).filter(|w| w.kind == Kind::Radio).map(|w| w.group) else { return };
        for w in self.rows.iter_mut().flat_map(|r| &mut r.widgets) {
            if w.kind == Kind::Radio && w.group == group {
                w.selected = w.id == id;
            }
        }
    }

    /// The button the Return key presses: the first enabled button that asked for the focus.
    pub fn default_button(&self) -> Option<&Widget> {
        self.rows.iter().flat_map(|r| &r.widgets).find(|w| w.kind == Kind::Button && w.focus && w.enabled && w.visible)
    }
}

/// `"&OK"` -> `"OK"`; `"&&"` is a literal `&`. Returns the text and the byte index of the mnemonic letter.
pub fn strip_mnemonic(text: &str) -> (String, Option<usize>) {
    let mut out = String::with_capacity(text.len());
    let mut mnemonic = None;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '&' {
            match chars.peek() {
                Some('&') => {
                    out.push('&');
                    chars.next();
                }
                Some(_) if mnemonic.is_none() => mnemonic = Some(out.len()),
                _ => {}
            }
        } else {
            out.push(ch);
        }
    }
    (out, mnemonic)
}

/// What the user did, as the events of section 2.2 describe it.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Change {
        id: String,
        value: Value,
    },
    Release {
        id: String,
    },
    Click {
        id: String,
        value: Option<bool>,
        color: Option<u32>,
    },
    /// The user closed the window (Esc, the close button).
    Closed,
}

impl Event {
    pub fn to_json(&self, dialog: &str) -> String {
        let s = |t: &str| Value::Str(t.to_string());
        let mut members: Vec<(String, Value)> = Vec::new();
        let mut put = |k: &str, v: Value| members.push((k.to_string(), v));
        match self {
            Event::Change { id, value } => {
                put("type", s("widget"));
                put("dialog", s(dialog));
                put("id", s(id));
                put("event", s("change"));
                put("value", value.clone());
            }
            Event::Release { id } => {
                put("type", s("widget"));
                put("dialog", s(dialog));
                put("id", s(id));
                put("event", s("release"));
            }
            Event::Click { id, value, color } => {
                put("type", s("widget"));
                put("dialog", s(dialog));
                put("id", s(id));
                put("event", s("click"));
                if let Some(b) = value {
                    put("value", Value::Bool(*b));
                }
                if let Some(c) = color {
                    put("color", Value::Num(f64::from(*c)));
                }
            }
            Event::Closed => {
                put("type", s("dialog.closed"));
                put("dialog", s(dialog));
            }
        }
        json::to_string(&Value::Obj(members))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `dialog.open` example of the contract.
    pub const CONTRACT_OPEN: &str = r#"{"type":"dialog.open","session":3,"dialog":{
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

    fn open() -> Dialog {
        let msg = json::parse(CONTRACT_OPEN).unwrap();
        Dialog::from_json(3, msg.get("dialog").unwrap()).unwrap()
    }

    #[test]
    fn the_contract_example_parses() {
        let d = open();
        assert_eq!((d.id.as_str(), d.title.as_str(), d.modal, d.rows.len()), ("s3.d1", "Palette ramp", true, 5));
        let from = d.widget("from").unwrap();
        assert_eq!(
            (from.kind, from.color, from.label.as_deref(), from.hexpand),
            (Kind::Color, 488714575, Some("From"), true)
        );
        let steps = d.widget("steps").unwrap();
        assert_eq!((steps.min, steps.max, steps.value), (2.0, 32.0, 8.0));
        assert_eq!(d.widget("_w5").unwrap().text, "Options");
        assert_eq!(d.default_button().unwrap().id, "ok");
    }

    #[test]
    fn modify_patches_fields_and_reports_focus() {
        let mut d = open();
        let msg = json::parse(
            r#"{"type":"dialog.modify","session":3,"dialog":"s3.d1","title":"New title",
                "widgets":{"steps":{"value":12,"max":64},"info":{"text":"Done","focus":true},"nope":{"text":"x"}}}"#,
        )
        .unwrap();
        let focus = d.modify(&msg);
        assert_eq!(d.title, "New title");
        let steps = d.widget("steps").unwrap();
        assert_eq!((steps.value, steps.max, steps.min), (12.0, 64.0, 2.0));
        assert_eq!(d.widget("info").unwrap().text, "Done");
        assert_eq!(focus, vec!["info".to_string()]);
    }

    #[test]
    fn every_kind_reads_its_fields() {
        let v = json::parse(
            r#"{"rows":[{"widgets":[
              {"id":"a","kind":"check","text":"Dither","selected":true},
              {"id":"r1","kind":"radio","text":"One","selected":true,"group":1},
              {"id":"r2","kind":"radio","text":"Two","selected":false,"group":1},
              {"id":"e","kind":"entry","text":"hi"},
              {"id":"n","kind":"number","value":1.5,"decimals":2},
              {"id":"c","kind":"combobox","options":["x","y"],"option":"y"},
              {"id":"f","kind":"file","filename":"a.png","open":false,"save":true,"filetypes":["png","gif"],"title":"Pick"},
              {"id":"s","kind":"shades","colors":[255,4294967295]},
              {"id":"z","kind":"canvas"}
            ]}],"id":"d"}"#,
        )
        .unwrap();
        let mut d = Dialog::from_json(1, &v).unwrap();
        assert_eq!(d.rows[0].widgets.len(), 8, "the unknown kind is skipped");
        assert!(d.widget("a").unwrap().selected);
        assert_eq!(d.widget("n").unwrap().decimals, 2);
        assert_eq!(d.widget("c").unwrap().option_index(), Some(1));
        let f = d.widget("f").unwrap();
        assert_eq!((f.open, f.save, f.filetypes.len(), f.title.as_str()), (false, true, 2, "Pick"));
        assert_eq!(d.widget("s").unwrap().colors, vec![255, 4294967295]);
        d.select_radio("r2");
        assert!(!d.widget("r1").unwrap().selected && d.widget("r2").unwrap().selected);
    }

    #[test]
    fn mnemonics_are_stripped() {
        assert_eq!(strip_mnemonic("&OK"), ("OK".to_string(), Some(0)));
        assert_eq!(strip_mnemonic("Save && &Close"), ("Save & Close".to_string(), Some(7)));
        assert_eq!(strip_mnemonic("Plain"), ("Plain".to_string(), None));
    }

    #[test]
    fn events_serialize_as_the_contract_says() {
        let change = Event::Change { id: "steps".into(), value: Value::Num(12.0) };
        assert_eq!(
            change.to_json("s3.d1"),
            r#"{"type":"widget","dialog":"s3.d1","id":"steps","event":"change","value":12}"#
        );
        let click = Event::Click { id: "pal".into(), value: None, color: Some(4278190335) };
        assert_eq!(
            click.to_json("s3.d1"),
            r#"{"type":"widget","dialog":"s3.d1","id":"pal","event":"click","color":4278190335}"#
        );
        let check = Event::Click { id: "dither".into(), value: Some(true), color: None };
        assert!(check.to_json("d").contains(r#""value":true"#));
        assert_eq!(Event::Closed.to_json("s3.d1"), r#"{"type":"dialog.closed","dialog":"s3.d1"}"#);
        assert_eq!(
            Event::Release { id: "steps".into() }.to_json("s3.d1"),
            r#"{"type":"widget","dialog":"s3.d1","id":"steps","event":"release"}"#
        );
    }
}
