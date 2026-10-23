//! The Text tool: a sheet with a live preview, drawn with the core's built-in 5x8 pixel font or a system font
//! rasterized without antialiasing (`TextSheet` and `renderText` in `ShapeTools.swift`).

use crate::app::PixmithApp;
use crate::theme::{Colors, radius, space};
use crate::ui::widgets;
use eframe::egui::{self, Margin, Sense};
use std::collections::HashMap;
use std::path::PathBuf;

/// A click that started a text (`TextRequest`) and the preview state that belongs to it.
#[derive(Clone, Copy, Debug)]
pub struct TextRequest {
    pub at: (i32, i32),
    pub color: u32,
    /// The first preview exists, so later ones rewind to the checkpoint first.
    pub started: bool,
    /// Undo depth right after the gesture's checkpoint.
    pub depth: u32,
    pub doc: usize,
}

/// What the preview was last drawn with.
type Rendered = (String, u32, bool, String, u32);

pub struct TextState {
    pub request: Option<TextRequest>,
    pub string: String,
    /// Pixel scale of the built-in font.
    pub scale: u32,
    pub system_font: bool,
    pub font_name: String,
    pub font_size: u32,
    rendered: Option<Rendered>,
    focus: bool,
    fonts: HashMap<String, Option<Vec<u8>>>,
}

impl Default for TextState {
    fn default() -> TextState {
        TextState {
            request: None,
            string: String::new(),
            scale: 2,
            system_font: false,
            font_name: "System".into(),
            font_size: 12,
            rendered: None,
            focus: true,
            fonts: HashMap::new(),
        }
    }
}

impl TextState {
    /// Opens the sheet for a click. The text typed last time stays (`textString` is kept between uses).
    pub fn begin(&mut self, request: TextRequest) {
        self.request = Some(request);
        self.rendered = None;
        self.focus = true;
    }

    /// The font file for `name` (`"System"` is the UI font), read once.
    fn font_bytes(&mut self, name: &str) -> Option<&[u8]> {
        self.fonts.entry(name.to_owned()).or_insert_with(|| load_font(name)).as_deref()
    }
}

// MARK: Fonts

/// `(name, file)` pairs of fonts that may be installed, tried in order.
fn candidates() -> Vec<(&'static str, Vec<PathBuf>)> {
    let p = |s: &str| PathBuf::from(s);
    if cfg!(target_os = "macos") {
        let d = "/System/Library/Fonts/Supplemental/";
        vec![
            ("Arial", vec![p(&format!("{d}Arial.ttf"))]),
            ("Courier New", vec![p(&format!("{d}Courier New.ttf"))]),
            ("Georgia", vec![p(&format!("{d}Georgia.ttf"))]),
            ("Times New Roman", vec![p(&format!("{d}Times New Roman.ttf"))]),
            ("Verdana", vec![p(&format!("{d}Verdana.ttf"))]),
        ]
    } else if cfg!(windows) {
        let d = r"C:\Windows\Fonts\";
        vec![
            ("Arial", vec![p(&format!("{d}arial.ttf"))]),
            ("Consolas", vec![p(&format!("{d}consola.ttf"))]),
            ("Courier New", vec![p(&format!("{d}cour.ttf"))]),
            ("Georgia", vec![p(&format!("{d}georgia.ttf"))]),
            ("Times New Roman", vec![p(&format!("{d}times.ttf"))]),
            ("Verdana", vec![p(&format!("{d}verdana.ttf"))]),
        ]
    } else {
        vec![
            (
                "DejaVu Sans",
                vec![p("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"), p("/usr/share/fonts/dejavu/DejaVuSans.ttf")],
            ),
            (
                "DejaVu Serif",
                vec![
                    p("/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf"),
                    p("/usr/share/fonts/dejavu/DejaVuSerif.ttf"),
                ],
            ),
            (
                "Liberation Sans",
                vec![
                    p("/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf"),
                    p("/usr/share/fonts/liberation/LiberationSans-Regular.ttf"),
                ],
            ),
        ]
    }
}

/// Fonts offered by the sheet: the UI font, a monospaced one and whichever of the common families exist.
pub fn font_families() -> Vec<String> {
    let mut out = vec!["System".to_string(), "Monospace".to_string()];
    out.extend(
        candidates().into_iter().filter(|(_, files)| files.iter().any(|f| f.exists())).map(|(n, _)| n.to_string()),
    );
    out
}

fn load_font(name: &str) -> Option<Vec<u8>> {
    let bundled = |mono: bool| {
        let defs = egui::FontDefinitions::default();
        let family = if mono { egui::FontFamily::Monospace } else { egui::FontFamily::Proportional };
        let key = defs.families.get(&family)?.first()?.clone();
        defs.font_data.get(&key).map(|d| d.font.to_vec())
    };
    match name {
        "System" => {
            let system: &[&str] = if cfg!(target_os = "macos") {
                &["/System/Library/Fonts/SFNS.ttf"]
            } else if cfg!(windows) {
                &[r"C:\Windows\Fonts\segoeui.ttf"]
            } else {
                &[]
            };
            system.iter().find_map(|f| std::fs::read(f).ok()).or_else(|| bundled(false))
        }
        "Monospace" => bundled(true),
        _ => candidates()
            .into_iter()
            .find(|(n, _)| *n == name)
            .and_then(|(_, files)| files.iter().find_map(|f| std::fs::read(f).ok()))
            .or_else(|| bundled(false)),
    }
}

/// Rasterizes `text` with the font in `bytes` and no antialiasing into a 0 / 255 coverage mask, one line per
/// `\n` (`SpriteModel.rasterize`). `None` for empty text or a mask above 16 million pixels.
pub fn rasterize(text: &str, bytes: &[u8], size: f32) -> Option<(usize, usize, Vec<u8>)> {
    use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
    let font = FontRef::try_from_slice(bytes).ok()?;
    let scale = PxScale::from(size);
    let scaled = font.as_scaled(scale);
    let line_height = (scaled.ascent() - scaled.descent() + scaled.line_gap()).ceil().max(1.0) as usize;
    let lines: Vec<&str> = text.split('\n').collect();
    let advance = |line: &str| {
        let mut x = 0.0;
        let mut prev = None;
        for ch in line.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(p) = prev {
                x += scaled.kern(p, id);
            }
            x += scaled.h_advance(id);
            prev = Some(id);
        }
        x
    };
    let width = lines.iter().map(|l| advance(l)).fold(0.0f32, f32::max).ceil() as usize;
    let height = line_height * lines.len();
    if width == 0 || height == 0 || width * height > 1 << 24 {
        return None;
    }
    let mut mask = vec![0u8; width * height];
    for (i, line) in lines.iter().enumerate() {
        let baseline = (i * line_height) as f32 + scaled.ascent();
        let mut x = 0.0;
        let mut prev = None;
        for ch in line.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(p) = prev {
                x += scaled.kern(p, id);
            }
            let mut glyph = id.with_scale(scale);
            glyph.position = point(x, baseline);
            if let Some(outline) = font.outline_glyph(glyph) {
                let b = outline.px_bounds();
                outline.draw(|gx, gy, coverage| {
                    let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                    if coverage >= 0.5 && px >= 0 && py >= 0 && (px as usize) < width && (py as usize) < height {
                        mask[py as usize * width + px as usize] = 255;
                    }
                });
            }
            x += scaled.h_advance(id);
            prev = Some(id);
        }
    }
    Some((width, height, mask))
}

// MARK: Drawing the preview

/// Draws the pending text at its click position, replacing the previous preview (`renderText`).
fn render(app: &mut PixmithApp) {
    let PixmithApp { tabs, active, canvas, .. } = app;
    let tab = &mut tabs[*active];
    let sym = canvas.opts.sym_flags();
    let t = &mut canvas.tools.text;
    let Some(r) = t.request else { return };
    // A command run under the sheet moved the history on: the preview can no longer be rewound safely.
    if tab.doc.raw() as usize != r.doc || tab.doc.history_counts().0 != r.depth {
        t.request = None;
        return;
    }
    let restore = r.started;
    if let Some(req) = &mut t.request {
        req.started = true;
    }
    let (layer, frame) = (tab.layer, tab.frame);
    let text = t.string.clone();
    let mask = if t.system_font && !text.is_empty() {
        let (name, size) = (t.font_name.clone(), t.font_size as f32);
        t.font_bytes(&name).and_then(|b| rasterize(&text, b, size))
    } else {
        None
    };
    match mask {
        Some((w, h, mask)) => {
            tab.doc.stamp_mask_draw(layer, frame, r.at, (w as u32, h as u32), &mask, r.color, sym, restore)
        }
        None => {
            let drawn = if t.system_font { "" } else { text.as_str() };
            tab.doc.text_draw(layer, frame, r.at, drawn, t.scale, r.color, sym, restore);
        }
    }
    t.rendered = Some((t.string.clone(), t.scale, t.system_font, t.font_name.clone(), t.font_size));
    tab.doc.touch();
    tab.changed();
}

/// Keeps the text and closes the sheet; an empty text counts as cancelled (`placeText`).
pub fn place(app: &mut PixmithApp) {
    if app.canvas.tools.text.string.is_empty() {
        return cancel(app);
    }
    app.canvas.tools.text.request = None;
}

/// Closes the sheet and removes the preview (`cancelText`).
pub fn cancel(app: &mut PixmithApp) {
    let Some(r) = app.canvas.tools.text.request.take() else { return };
    let tab = app.tab_mut();
    if tab.doc.raw() as usize == r.doc && tab.doc.history_counts().0 == r.depth && tab.doc.rollback() {
        tab.doc.touch();
        tab.changed();
    }
}

// MARK: The sheet

/// The Text sheet, shown while a click is waiting for its text.
pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    if app.canvas.tools.text.request.is_none() {
        return;
    }
    let t = &app.canvas.tools.text;
    let key: Rendered = (t.string.clone(), t.scale, t.system_font, t.font_name.clone(), t.font_size);
    if t.rendered.as_ref() != Some(&key) {
        render(app);
        if app.canvas.tools.text.request.is_none() {
            return;
        }
    }
    let c = Colors::of(ctx);
    let st = &mut app.canvas.tools.text;
    let families = font_families();
    let action = widgets::dialog(
        ctx,
        "text",
        380.0,
        "Text",
        Some("Drawn in the primary color at the clicked position (right-click: secondary)."),
        |ui| {
            // The editor sits on a field-colored surface, 90 pt high, in a monospaced face.
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 90.0), Sense::hover());
            ui.painter().rect_filled(rect, radius::FIELD, c.field);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            egui::ScrollArea::vertical().id_salt("text-scroll").max_height(rect.height()).show(&mut child, |ui| {
                let edit = egui::TextEdit::multiline(&mut st.string)
                    .id(egui::Id::new("text-sheet-editor"))
                    .frame(egui::Frame::NONE)
                    .margin(Margin::same(space::XS as i8))
                    .font(egui::FontId::monospace(13.0))
                    .desired_width(rect.width() - 2.0 * space::XS)
                    .desired_rows(5);
                let out = ui.add(edit);
                if st.focus {
                    out.request_focus();
                    st.focus = false;
                }
            });
            ui.checkbox(&mut st.system_font, "Use system font (rasterized without antialiasing)");
            if st.system_font {
                widgets::form_row(ui, "Font", |ui| {
                    let names: Vec<&str> = families.iter().map(String::as_str).collect();
                    let mut i = names.iter().position(|n| *n == st.font_name).unwrap_or(0);
                    widgets::popup(ui, &names, &mut i, None);
                    st.font_name = names[i].to_owned();
                });
                widgets::form_row(ui, "Size", |ui| {
                    widgets::number_field(ui, "Size", &mut st.font_size, 6..=72, "pt", 120.0);
                });
            } else {
                widgets::form_row(ui, "Pixel scale", |ui| {
                    widgets::number_field(ui, "Scale", &mut st.scale, 1..=16, "x", 120.0);
                });
            }
            let mut action = widgets::DialogAction::None;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::SM;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::primary_button(ui, "Place", true).clicked() {
                        action = widgets::DialogAction::Primary;
                    }
                    if widgets::secondary_button(ui, "Cancel", false).clicked() {
                        action = widgets::DialogAction::Cancel;
                    }
                });
            });
            // Return types a new line, so Place is Cmd/Ctrl+Return.
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter) {
                    action = widgets::DialogAction::Primary;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    action = widgets::DialogAction::Cancel;
                }
            });
            action
        },
    );
    match action {
        widgets::DialogAction::Primary => place(app),
        widgets::DialogAction::Cancel => cancel(app),
        _ => {}
    }
}
