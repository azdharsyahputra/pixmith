//! Customizable single-key shortcuts (`App/Shortcuts.swift`): the registry, key dispatch and the encoding that
//! is persisted in eframe's storage.

use crate::app::{PixmithApp, Tool};
use crate::ui::timeline::playback;
use eframe::egui::{self, Event, Key, Modifiers};
use std::collections::BTreeMap;

/// A single-key shortcut. `key` is the lowercased character the key produces.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyBinding {
    pub key: String,
    pub shift: bool,
}

impl KeyBinding {
    fn new(key: &str, shift: bool) -> KeyBinding {
        KeyBinding { key: key.to_string(), shift }
    }

    /// "B", "⇧G", "Space".
    pub fn label(&self) -> String {
        format!(
            "{}{}",
            if self.shift { "\u{21e7}" } else { "" },
            if self.key == " " { "Space".into() } else { self.key.to_uppercase() }
        )
    }
}

/// Everything that can be triggered by a customizable single key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Tool(Tool),
    SwapColors,
    BrushSmaller,
    BrushLarger,
    PreviousFrame,
    NextFrame,
    PlayPause,
}

/// The tools in the order of `Tool.allCases` in `SpriteModel.swift`.
const TOOLS: [Tool; 21] = [
    Tool::Pencil,
    Tool::Eraser,
    Tool::Fill,
    Tool::Picker,
    Tool::Line,
    Tool::Rectangle,
    Tool::Ellipse,
    Tool::Spray,
    Tool::Marquee,
    Tool::Move,
    Tool::Wand,
    Tool::Lasso,
    Tool::Polygon,
    Tool::Gradient,
    Tool::Slice,
    Tool::Curve,
    Tool::Contour,
    Tool::Text,
    Tool::ShapePolygon,
    Tool::Blur,
    Tool::Jumble,
];

impl Action {
    pub fn all() -> Vec<Action> {
        TOOLS
            .iter()
            .map(|&t| Action::Tool(t))
            .chain([
                Action::SwapColors,
                Action::BrushSmaller,
                Action::BrushLarger,
                Action::PreviousFrame,
                Action::NextFrame,
                Action::PlayPause,
            ])
            .collect()
    }

    pub fn is_tool(self) -> bool {
        matches!(self, Action::Tool(_))
    }

    pub fn id(self) -> String {
        match self {
            Action::Tool(t) => format!("tool.{}", tool_raw(t)),
            Action::SwapColors => "swapColors".into(),
            Action::BrushSmaller => "brushSmaller".into(),
            Action::BrushLarger => "brushLarger".into(),
            Action::PreviousFrame => "previousFrame".into(),
            Action::NextFrame => "nextFrame".into(),
            Action::PlayPause => "playPause".into(),
        }
    }

    /// `ShortcutAction.title`: the tool's raw name capitalized.
    pub fn title(self) -> String {
        match self {
            Action::Tool(Tool::ShapePolygon) => "Polygon (Draw)".into(),
            Action::Tool(t) => {
                let raw = tool_raw(t);
                raw[..1].to_uppercase() + &raw[1..]
            }
            Action::SwapColors => "Swap Colors".into(),
            Action::BrushSmaller => "Smaller Brush".into(),
            Action::BrushLarger => "Larger Brush".into(),
            Action::PreviousFrame => "Previous Frame".into(),
            Action::NextFrame => "Next Frame".into(),
            Action::PlayPause => "Play / Pause".into(),
        }
    }

    /// Keys that work out of the box.
    pub fn defaults(self) -> Vec<KeyBinding> {
        let k = KeyBinding::new;
        match self {
            Action::Tool(t) => match t {
                Tool::Pencil => vec![k("b", false)],
                Tool::Eraser => vec![k("e", false)],
                Tool::Fill => vec![k("g", false)],
                Tool::Picker => vec![k("i", false)],
                Tool::Marquee => vec![k("m", false)],
                Tool::Move => vec![k("v", false)],
                Tool::Line => vec![k("l", false)],
                Tool::Rectangle => vec![k("u", false)],
                Tool::Ellipse => vec![k("o", false), k("u", true)],
                Tool::Spray => vec![k("a", false)],
                Tool::Wand => vec![k("w", false)],
                Tool::Lasso => vec![k("q", false)],
                Tool::Polygon => vec![k("p", false)],
                Tool::Gradient => vec![k("g", true)],
                Tool::Slice => vec![k("s", false)],
                Tool::Curve => vec![k("c", false)],
                Tool::Contour => vec![k("d", false)],
                Tool::ShapePolygon => vec![k("p", true)],
                Tool::Text => vec![k("t", false)],
                Tool::Blur => vec![k("r", false)],
                Tool::Jumble => vec![k("r", true)],
            },
            Action::SwapColors => vec![k("x", false)],
            Action::BrushSmaller => vec![k("[", false)],
            Action::BrushLarger => vec![k("]", false)],
            Action::PreviousFrame => vec![k(",", false)],
            Action::NextFrame => vec![k(".", false)],
            Action::PlayPause => vec![k(" ", false)],
        }
    }
}

/// The Swift raw value of a tool, used for ids and titles.
fn tool_raw(t: Tool) -> &'static str {
    match t {
        Tool::Pencil => "pencil",
        Tool::Eraser => "eraser",
        Tool::Fill => "fill",
        Tool::Picker => "picker",
        Tool::Line => "line",
        Tool::Rectangle => "rectangle",
        Tool::Ellipse => "ellipse",
        Tool::Spray => "spray",
        Tool::Marquee => "marquee",
        Tool::Move => "move",
        Tool::Wand => "wand",
        Tool::Lasso => "lasso",
        Tool::Polygon => "polygon",
        Tool::Gradient => "gradient",
        Tool::Slice => "slice",
        Tool::Curve => "curve",
        Tool::Contour => "contour",
        Tool::Text => "text",
        Tool::ShapePolygon => "shapePolygon",
        Tool::Blur => "blur",
        Tool::Jumble => "jumble",
    }
}

/// A key the user tried to give to an action while another one has it.
#[derive(Clone, Debug)]
pub struct Conflict {
    pub action: Action,
    pub binding: KeyBinding,
    pub other: Action,
}

/// The central registry of customizable single-key shortcuts.
#[derive(Default)]
pub struct Shortcuts {
    /// Only actions that differ from their defaults; an empty list means unassigned.
    overrides: BTreeMap<String, Vec<KeyBinding>>,
    /// The action waiting for its new key in the Keyboard tab.
    pub recording: Option<Action>,
    pub conflict: Option<Conflict>,
}

impl Shortcuts {
    pub fn bindings(&self, a: Action) -> Vec<KeyBinding> {
        self.overrides.get(&a.id()).cloned().unwrap_or_else(|| a.defaults())
    }

    /// The first binding's label, or "none".
    pub fn label(&self, a: Action) -> String {
        self.bindings(a).first().map_or_else(|| "none".to_string(), KeyBinding::label)
    }

    pub fn is_customized(&self) -> bool {
        !self.overrides.is_empty()
    }

    fn set(&mut self, a: Action, list: Vec<KeyBinding>) {
        if list == a.defaults() {
            self.overrides.remove(&a.id());
        } else {
            self.overrides.insert(a.id(), list);
        }
    }

    pub fn reset(&mut self) {
        self.overrides.clear();
        self.conflict = None;
        self.recording = None;
    }

    /// The action bound to a key press: an exact (key, shift) match, else a plain binding ignoring Shift.
    pub fn action_for(&self, key: &str, shift: bool) -> Option<Action> {
        let all = Action::all();
        let exact = KeyBinding { key: key.to_string(), shift };
        all.iter()
            .copied()
            .find(|&a| self.bindings(a).contains(&exact))
            .or_else(|| all.iter().copied().find(|&a| self.bindings(a).iter().any(|b| b.key == key && !b.shift)))
    }

    /// Gives the key to the pending action and unassigns it from the one that had it.
    pub fn resolve_conflict(&mut self) {
        let Some(c) = self.conflict.take() else { return };
        let rest: Vec<KeyBinding> = self.bindings(c.other).into_iter().filter(|b| *b != c.binding).collect();
        self.set(c.other, rest);
        self.set(c.action, vec![c.binding]);
    }

    /// Handles a key while recording: Esc cancels, Delete unassigns, another key rebinds (`capture`).
    fn capture(&mut self, a: Action, key: Key, shift: bool) {
        match key {
            Key::Escape => {
                self.recording = None;
                return;
            }
            Key::Backspace | Key::Delete => {
                self.set(a, Vec::new());
                self.recording = None;
                return;
            }
            _ => {}
        }
        let Some(name) = key_char(key) else { return };
        let b = KeyBinding { key: name, shift };
        self.recording = None;
        if let Some(other) = Action::all().into_iter().find(|&o| o != a && self.bindings(o).contains(&b)) {
            self.conflict = Some(Conflict { action: a, binding: b, other });
        } else {
            self.set(a, vec![b]);
        }
    }

    // MARK: Persistence

    /// `id=hexchar,shift;hexchar,shift` per line.
    pub fn encode(&self) -> String {
        let mut out = String::new();
        for (id, list) in &self.overrides {
            let items: Vec<String> = list
                .iter()
                .map(|b| format!("{:x},{}", b.key.chars().next().map_or(0, |c| c as u32), u8::from(b.shift)))
                .collect();
            out.push_str(&format!("{id}={}\n", items.join(";")));
        }
        out
    }

    pub fn decode(text: &str) -> Shortcuts {
        let mut s = Shortcuts::default();
        let known: Vec<String> = Action::all().iter().map(|a| a.id()).collect();
        for line in text.lines() {
            let Some((id, rest)) = line.split_once('=') else { continue };
            if !known.iter().any(|k| k == id) {
                continue;
            }
            let list = rest
                .split(';')
                .filter_map(|item| {
                    let (code, shift) = item.split_once(',')?;
                    let c = char::from_u32(u32::from_str_radix(code, 16).ok()?)?;
                    Some(KeyBinding { key: c.to_string(), shift: shift == "1" })
                })
                .collect();
            s.overrides.insert(id.to_string(), list);
        }
        s
    }
}

/// The character a key produces (lowercase), for keys that can be a single-key shortcut.
pub fn key_char(key: Key) -> Option<String> {
    if key == Key::Space {
        return Some(" ".into());
    }
    let name = key.symbol_or_name();
    let mut chars = name.chars();
    let (c, rest) = (chars.next()?, chars.next());
    (rest.is_none() && c.is_ascii_graphic()).then(|| c.to_ascii_lowercase().to_string())
}

/// The key presses of this frame, as (key, shift).
fn presses(ctx: &egui::Context) -> Vec<(Key, bool)> {
    ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                Event::Key { key, pressed: true, modifiers, .. } if !modifiers.command && !modifiers.alt => {
                    Some((*key, modifiers.shift))
                }
                _ => None,
            })
            .collect()
    })
}

/// Runs this frame's single-key shortcuts, or records a key for the Keyboard tab. Called from the canvas, with
/// the focus rules (no text field, no dialog) already checked.
pub fn dispatch(app: &mut PixmithApp, ctx: &egui::Context) {
    if let Some(a) = app.menus.shortcuts.recording {
        for (key, shift) in presses(ctx) {
            ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key));
            app.menus.shortcuts.capture(a, key, shift);
            if app.menus.shortcuts.recording.is_none() {
                break;
            }
        }
        return;
    }
    if super::blocked(app) {
        return;
    }
    for (key, shift) in presses(ctx) {
        let Some(name) = key_char(key) else { continue };
        let Some(action) = app.menus.shortcuts.action_for(&name, shift) else { continue };
        let mods = if shift { Modifiers::SHIFT } else { Modifiers::NONE };
        ctx.input_mut(|i| i.consume_key(mods, key));
        perform(app, ctx, action);
    }
}

/// `SpriteModel.perform`.
pub fn perform(app: &mut PixmithApp, ctx: &egui::Context, action: Action) {
    let now = ctx.input(|i| i.time);
    match action {
        Action::Tool(t) => app.tool = t,
        Action::SwapColors => std::mem::swap(&mut app.primary, &mut app.secondary),
        Action::BrushSmaller => app.brush_size = app.brush_size.saturating_sub(1).max(1),
        Action::BrushLarger => app.brush_size = (app.brush_size + 1).min(64),
        Action::PreviousFrame => {
            let tab = &mut app.tabs[app.active];
            playback::stop(tab);
            playback::step(tab, -1);
        }
        Action::NextFrame => {
            let tab = &mut app.tabs[app.active];
            playback::stop(tab);
            playback::step(tab, 1);
        }
        Action::PlayPause => playback::toggle(&mut app.tabs[app.active], &mut app.timeline, now),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_swift() {
        let s = Shortcuts::default();
        assert_eq!(s.action_for("b", false), Some(Action::Tool(Tool::Pencil)));
        assert_eq!(s.action_for("g", true), Some(Action::Tool(Tool::Gradient)));
        assert_eq!(s.action_for("u", true), Some(Action::Tool(Tool::Ellipse)));
        // A plain binding also answers when Shift is held and nothing is bound with it.
        assert_eq!(s.action_for("b", true), Some(Action::Tool(Tool::Pencil)));
        assert_eq!(s.action_for(" ", false), Some(Action::PlayPause));
        assert_eq!(s.label(Action::Tool(Tool::Gradient)), "\u{21e7}G");
        assert_eq!(s.label(Action::PlayPause), "Space");
    }

    #[test]
    fn rebinding_conflicts_and_persists() {
        let mut s = Shortcuts::default();
        s.capture(Action::Tool(Tool::Pencil), Key::E, false);
        let c = s.conflict.clone().expect("e belongs to the eraser");
        assert_eq!(c.other, Action::Tool(Tool::Eraser));
        s.resolve_conflict();
        assert_eq!(s.action_for("e", false), Some(Action::Tool(Tool::Pencil)));
        assert_eq!(s.label(Action::Tool(Tool::Eraser)), "none");
        assert!(s.is_customized());
        let again = Shortcuts::decode(&s.encode());
        assert_eq!(again.action_for("e", false), Some(Action::Tool(Tool::Pencil)));
        assert_eq!(again.label(Action::Tool(Tool::Eraser)), "none");
        s.reset();
        assert!(!s.is_customized());
        assert_eq!(s.action_for("e", false), Some(Action::Tool(Tool::Eraser)));
    }

    #[test]
    fn delete_unassigns_and_escape_cancels() {
        let mut s = Shortcuts::default();
        s.capture(Action::SwapColors, Key::Escape, false);
        assert!(s.recording.is_none() && !s.is_customized());
        s.capture(Action::SwapColors, Key::Delete, false);
        assert_eq!(s.label(Action::SwapColors), "none");
        assert_eq!(s.action_for("x", false), None);
    }

    #[test]
    fn keys_map_to_characters() {
        assert_eq!(key_char(Key::A).as_deref(), Some("a"));
        assert_eq!(key_char(Key::OpenBracket).as_deref(), Some("["));
        assert_eq!(key_char(Key::Comma).as_deref(), Some(","));
        assert_eq!(key_char(Key::Space).as_deref(), Some(" "));
        assert_eq!(key_char(Key::F1), None);
        assert_eq!(key_char(Key::ArrowUp), None);
    }
}
