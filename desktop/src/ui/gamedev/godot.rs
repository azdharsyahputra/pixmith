//! Export to Godot 4: a SpriteFrames `.tres` plus an atlas PNG, optionally a `.tscn` (`Dialogs/GodotExport.swift`).
//!
//! The sheet is 1000 pt wide: the form on the left (450 pt, scrolling), an atlas / `.tres` preview on the right
//! that rebuilds 150 ms after the last change. Export keeps the sheet open with the result; settings are
//! remembered in the document and the project folder across runs. Re-export repeats the saved export.

use super::form::{self, Tone};
use super::{GodotPreview, GodotSettings, LayerMode};
use crate::app::PixmithApp;
use crate::doc::Doc;
use crate::theme::{Colors, font, metric, radius, space, ui_font};
use crate::ui::widgets::{self, DialogAction, NumberOpts, SegGlyph};
use eframe::egui::{self, Align, Layout, Rect, Sense, Stroke, TextureHandle, Ui, UiBuilder, pos2, vec2};
use egui_phosphor::regular as icon;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const LABEL_WIDTH: f32 = 96.0;
const SIZES: [u32; 4] = [2048, 4096, 8192, 16384];
const LAYER_MODES: [&str; 3] = ["Visible layers", "Each layer as its own file", "Selected layers"];
/// The form column, the sheet's body height and the gap between columns.
const FORM_WIDTH: f32 = 450.0;
const BODY_HEIGHT: f32 = 548.0;

/// The first atlas page and the texts the core built for the current settings.
#[derive(Default)]
pub struct PreviewData {
    pub image: Option<(TextureHandle, [u32; 2])>,
    pub text: String,
    pub files: Vec<String>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

pub struct GodotSheet {
    pub s: GodotSettings,
    /// The result of the last export and whether it failed.
    pub message: Option<(String, bool)>,
    /// An export succeeded: Cancel becomes Close.
    pub done: bool,
    pub preview: PreviewData,
    pub show_text: bool,
    /// The settings the preview was built for, and the ones waiting for the 150 ms pause.
    previewed: Option<GodotSettings>,
    pending: Option<GodotSettings>,
    pending_since: Instant,
}

/// Name used for the default output file: the document's file name without extension.
fn sprite_name(doc: &Doc) -> String {
    doc.path.as_ref().and_then(|p| p.file_stem()).map_or("sprite".into(), |s| s.to_string_lossy().into_owned())
}

fn default_tres(root: &str, name: &str) -> String {
    Path::new(root).join("art").join(format!("{name}.tres")).to_string_lossy().into_owned()
}

impl GodotSheet {
    /// A sheet with the document's saved settings, or defaults with the last project folder.
    pub fn new(doc: &Doc, last_root: &str) -> GodotSheet {
        let s = match doc.godot_settings().and_then(|j| GodotSettings::from_json(&j)) {
            Some(saved) => saved,
            None => {
                let mut s = GodotSettings { root: last_root.to_string(), ..Default::default() };
                if !s.root.is_empty() {
                    s.tres = default_tres(&s.root, &sprite_name(doc));
                }
                s
            }
        };
        GodotSheet {
            s,
            message: None,
            done: false,
            preview: PreviewData::default(),
            show_text: false,
            previewed: None,
            pending: None,
            pending_since: Instant::now() - Duration::from_secs(1),
        }
    }

    fn ready(&self) -> bool {
        !self.s.root.is_empty() && !self.s.tres.is_empty()
    }

    /// Rebuilds the preview now (`refreshPreview`).
    pub fn refresh_preview(&mut self, doc: &Doc, ctx: &egui::Context) {
        self.previewed = Some(self.s.clone());
        let png = std::env::temp_dir().join(format!("pixmith-godot-preview-{}.png", std::process::id()));
        let _ = std::fs::remove_file(&png);
        if !self.ready() {
            self.preview.error = Some("Choose the project folder and the output file to see a preview.".into());
            self.preview.image = None;
            self.preview.text.clear();
            return;
        }
        match doc.godot_preview(&self.s.json(), &png) {
            Ok(GodotPreview { tres, files, warnings }) => {
                self.preview = PreviewData { image: load_image(ctx, &png), text: tres, files, warnings, error: None };
            }
            Err(e) => {
                self.preview.error = Some(e);
                self.preview.image = None;
                self.preview.text.clear();
                self.preview.files.clear();
            }
        }
        let _ = std::fs::remove_file(&png);
    }

    /// Calls `refresh_preview` once the settings have been quiet for 150 ms.
    fn refresh_when_quiet(&mut self, doc: &Doc, ctx: &egui::Context) {
        if self.pending.as_ref() != Some(&self.s) {
            self.pending = Some(self.s.clone());
            self.pending_since = Instant::now();
        }
        if self.previewed.as_ref() == Some(&self.s) {
            return;
        }
        let quiet = self.pending_since.elapsed();
        if quiet >= Duration::from_millis(150) {
            self.refresh_preview(doc, ctx);
        } else {
            ctx.request_repaint_after(Duration::from_millis(150) - quiet);
        }
    }
}

/// Decodes the atlas page the core wrote, through the core's own PNG import.
fn load_image(ctx: &egui::Context, png: &Path) -> Option<(TextureHandle, [u32; 2])> {
    let doc = Doc::import_png(png).ok()?;
    let (w, h) = (doc.width(), doc.height());
    let mut pixels = Vec::new();
    if w == 0 || h == 0 || !doc.render(0, &mut pixels) {
        return None;
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels);
    Some((ctx.load_texture("godot-atlas", image, egui::TextureOptions::NEAREST), [w, h]))
}

/// Exports with the sheet's settings and shows the result in the sheet (`GodotExportState.export`).
pub fn export(app: &mut PixmithApp, sheet: &mut GodotSheet) {
    match app.tab().doc.godot_export(&sheet.s.json()) {
        Ok(warnings) => {
            if app.tab_mut().doc.set_godot_settings(&sheet.s.json()) {
                app.tab_mut().changed();
            }
            app.gamedev.prefs.set_godot_root(&sheet.s.root);
            let shown = app.gamedev.prefs.godot_warned();
            app.gamedev.prefs.set_godot_warned();
            let mut lines = vec!["Exported.".to_string()];
            lines.extend(warnings.into_iter().filter(|w| !(shown && w.contains("Nearest"))));
            sheet.message = Some((lines.join("\n\n"), false));
            sheet.done = true;
        }
        Err(e) => sheet.message = Some((e, true)),
    }
}

/// Opens the sheet (`GodotExportSheet.present`).
pub fn open(app: &mut PixmithApp) {
    let root = app.gamedev.prefs.godot_root().to_string();
    app.gamedev.godot = Some(GodotSheet::new(&app.tab().doc, &root));
}

/// Repeats the last export saved in the document; opens the sheet when there is none (`reexport`).
pub fn reexport(app: &mut PixmithApp) {
    let saved = app.tab().doc.godot_settings().and_then(|j| GodotSettings::from_json(&j));
    let Some(s) = saved.filter(|s| !s.root.is_empty() && !s.tres.is_empty()) else {
        return open(app);
    };
    match app.tab().doc.godot_export(&s.json()) {
        Err(e) => app.error = Some(e),
        Ok(warnings) => {
            let shown = app.gamedev.prefs.godot_warned();
            app.gamedev.prefs.set_godot_warned();
            let notes: Vec<String> = warnings.into_iter().filter(|w| !w.contains("Nearest") || !shown).collect();
            if !notes.is_empty() {
                app.gamedev.notice =
                    Some(super::Notice { title: "Exported to Godot".into(), text: notes.join("\n\n"), warning: false });
            }
        }
    }
}

pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(mut sheet) = app.gamedev.godot.take() else { return };
    sheet.refresh_when_quiet(&app.tab().doc, ctx);
    let doc = &app.tab().doc;
    let layers: Vec<String> =
        (0..doc.layer_count()).filter(|&l| !doc.layer_is_group(l)).map(|l| doc.layer_name(l)).collect();
    let tags: Vec<String> = (0..doc.tag_count()).filter_map(|i| doc.tag(i)).map(|t| t.name).collect();
    let name = sprite_name(doc);
    let show_hint = !app.gamedev.prefs.godot_warned();
    let mut chose_root = None;

    let action = widgets::dialog(
        ctx,
        "godot",
        1000.0,
        "Export to Godot 4",
        Some(
            "Writes a SpriteFrames .tres (format 3) and an atlas PNG. Godot 4 only: Godot 3 projects use another \
             format and are not supported.",
        ),
        |ui| {
            let full = ui.available_width();
            let top = ui.cursor().min;
            let body = Rect::from_min_size(top, vec2(full, BODY_HEIGHT));
            ui.allocate_rect(body, Sense::hover());
            let form_rect = Rect::from_min_size(top, vec2(FORM_WIDTH, BODY_HEIGHT));
            let divider_x = form_rect.right() + space::LG + 0.5;
            let preview_rect = Rect::from_min_max(pos2(divider_x + 0.5 + space::LG, top.y), body.right_bottom());
            let c = Colors::of(ui.ctx());
            ui.painter().vline(divider_x, body.y_range(), Stroke::new(1.0, c.separator_soft));

            let mut form_ui = ui.new_child(UiBuilder::new().max_rect(form_rect).layout(Layout::top_down(Align::LEFT)));
            egui::ScrollArea::vertical().id_salt("godot_form").auto_shrink([false, false]).show(&mut form_ui, |ui| {
                ui.set_width(FORM_WIDTH - space::MD);
                form_body(ui, &mut sheet.s, &layers, &tags, &name, &mut chose_root);
            });
            let mut preview_ui =
                ui.new_child(UiBuilder::new().max_rect(preview_rect).layout(Layout::top_down(Align::LEFT)));
            preview_body(&mut preview_ui, &mut sheet.preview, &mut sheet.show_text, preview_rect);

            if show_hint {
                form::notice(
                    ui,
                    "Pixel art looks blurry in Godot by default. Set Project Settings > Rendering > Textures > \
                     Default Texture Filter = Nearest.",
                    Tone::Warning,
                );
            }
            if let Some((m, err)) = &sheet.message {
                form::notice(ui, m, if *err { Tone::Error } else { Tone::Info });
            }
            widgets::dialog_buttons(
                ui,
                None,
                Some(if sheet.done { "Close" } else { "Cancel" }),
                "Export",
                !sheet.s.root.is_empty() && !sheet.s.tres.is_empty(),
            )
        },
    );
    if let Some(root) = chose_root {
        app.gamedev.prefs.set_godot_root(&root);
    }
    match action {
        DialogAction::Primary => {
            export(app, &mut sheet);
            app.gamedev.godot = Some(sheet);
        }
        DialogAction::Cancel => {}
        _ => app.gamedev.godot = Some(sheet),
    }
}

fn path_row(ui: &mut Ui, id: &str, label: &str, hint: &str, text: &mut String, choose: bool) -> bool {
    let mut pressed = false;
    form::form_row(ui, LABEL_WIDTH, label, |ui| {
        let button = if choose { form::bar_button_width(ui, "Choose\u{2026}") + space::SM } else { 0.0 };
        form::text_field(ui, id, text, hint, Some(ui.available_width() - button));
        if choose && form::bar_button(ui, "Choose\u{2026}").clicked() {
            pressed = true;
        }
    });
    pressed
}

fn form_body(
    ui: &mut Ui,
    s: &mut GodotSettings,
    layers: &[String],
    tags: &[String],
    name: &str,
    chose_root: &mut Option<String>,
) {
    ui.spacing_mut().item_spacing = vec2(space::SM, space::MD);
    form::form_group(ui, "Destination", true, |ui| {
        if path_row(ui, "root", "Project folder", "Folder with project.godot", &mut s.root, true)
            && let Some(dir) = rfd::FileDialog::new()
                .set_title("Choose the Godot project folder (the one containing project.godot)")
                .pick_folder()
        {
            s.root = dir.to_string_lossy().into_owned();
            s.tres = default_tres(&s.root, name);
            *chose_root = Some(s.root.clone());
        }
        if path_row(ui, "tres", "Output file", "SpriteFrames .tres inside the project", &mut s.tres, true) {
            let mut dialog =
                rfd::FileDialog::new().set_file_name(format!("{name}.tres")).add_filter("Godot resource", &["tres"]);
            if !s.root.is_empty() {
                dialog = dialog.set_directory(&s.root);
            }
            if let Some(file) = dialog.save_file() {
                s.tres = if file.extension().is_some_and(|e| e == "tres") {
                    file
                } else {
                    PathBuf::from(format!("{}.tres", file.display()))
                }
                .to_string_lossy()
                .into_owned();
            }
        }
        path_row(ui, "name", "File name", "{name} {layer}", &mut s.name_template, false);
    });
    form::form_group(ui, "Content", false, |ui| {
        form::form_row(ui, LABEL_WIDTH, "Layers", |ui| {
            let selected = match s.layer_mode {
                LayerMode::Visible => 0,
                LayerMode::Each => 1,
                LayerMode::Selected => 2,
            };
            if let Some(i) =
                widgets::ds_popup(ui, "layer_mode", selected, &LAYER_MODES, None, true, "Which layers to export")
            {
                s.layer_mode = [LayerMode::Visible, LayerMode::Each, LayerMode::Selected][i];
            }
        });
        if s.layer_mode == LayerMode::Selected {
            form::indented(ui, LABEL_WIDTH, |ui| {
                for layer in layers {
                    let mut on = s.selected_layers.contains(layer);
                    if form::check(ui, &format!("layer:{layer}"), &mut on, layer) {
                        s.selected_layers.retain(|n| n != layer);
                        if on {
                            s.selected_layers.push(layer.clone());
                        }
                    }
                }
            });
        }
        if tags.is_empty() {
            form::indented(ui, LABEL_WIDTH, |ui| {
                widgets::note(ui, "No tags: one \"default\" animation with all frames.");
            });
        } else {
            form::form_row(ui, LABEL_WIDTH, "Animations", |ui| {
                let c = Colors::of(ui.ctx());
                ui.label(egui::RichText::new("One per tag").font(ui_font(font::CAPTION)).color(c.text_tertiary));
            });
            form::indented(ui, LABEL_WIDTH, |ui| {
                for tag in tags {
                    // Stored as an exclude list so that new tags are exported by default.
                    let mut on =
                        !s.tags_exclude.contains(tag) && (s.tags_include.is_empty() || s.tags_include.contains(tag));
                    if form::check(ui, &format!("tag:{tag}"), &mut on, tag) {
                        s.tags_include.clear();
                        s.tags_exclude.retain(|n| n != tag);
                        if !on {
                            s.tags_exclude.push(tag.clone());
                        }
                    }
                }
            });
        }
    });
    form::form_group(ui, "Atlas", false, |ui| {
        form::form_row(ui, LABEL_WIDTH, "Packing", |ui| {
            ui.push_id("packing", |ui| {
                let options = [
                    (SegGlyph::Text("Tight (deduplicated)"), "Tight (deduplicated)"),
                    (SegGlyph::Text("Grid"), "Grid"),
                ];
                if let Some(i) = widgets::bar_segmented(ui, s.grid as usize, &options) {
                    s.grid = i == 1;
                }
            });
        });
        form::form_row(ui, LABEL_WIDTH, "Spacing", |ui| {
            let mut padding = s.padding as f64;
            let o = NumberOpts { range: (0.0, 16.0), unit: "px", width: Some(128.0), ..Default::default() };
            if widgets::ds_number_field(ui, "padding", "Padding", &mut padding, &o).changed {
                s.padding = padding as u32;
            }
            let mut scale = s.scale as f64;
            let o = NumberOpts { range: (1.0, 8.0), unit: "\u{d7}", width: Some(96.0), ..Default::default() };
            if widgets::ds_number_field(ui, "scale", "Scale", &mut scale, &o).changed {
                s.scale = scale as u32;
            }
        });
        form::form_row(ui, LABEL_WIDTH, "Max texture", |ui| {
            let labels: Vec<String> = SIZES.iter().map(|n| format!("{n} px")).collect();
            let options: Vec<&str> = labels.iter().map(String::as_str).collect();
            let selected = SIZES.iter().position(|n| *n == s.max_texture).unwrap_or(1);
            if let Some(i) = widgets::ds_popup(ui, "max_texture", selected, &options, None, false, "Largest atlas page")
            {
                s.max_texture = SIZES[i];
            }
        });
        if s.max_texture > 4096 {
            form::indented(ui, LABEL_WIDTH, |ui| {
                form::notice(ui, "Above 4096px some mobile GPUs cannot load the texture.", Tone::Warning);
            });
        }
        form::indented(ui, LABEL_WIDTH, |ui| {
            form::check(ui, "extrude", &mut s.extrude, "Extrude edges 1px (stops texture bleeding)");
            form::check(ui, "trim", &mut s.trim, "Trim transparent borders (AtlasTexture margin)");
            form::check(ui, "per_png", &mut s.per_animation_png, "Separate PNG per animation");
        });
    });
    form::form_group(ui, "Scene", false, |ui| {
        form::indented(ui, LABEL_WIDTH, |ui| {
            form::check(ui, "tscn", &mut s.tscn, "Also write a .tscn with an AnimatedSprite2D");
            ui.add_enabled_ui(s.tscn, |ui| {
                form::check(ui, "overwrite", &mut s.overwrite_tscn, "Replace the .tscn if it exists");
            });
        });
    });
}

fn preview_body(ui: &mut Ui, p: &mut PreviewData, show_text: &mut bool, rect: Rect) {
    let c = Colors::of(ui.ctx());
    ui.spacing_mut().item_spacing.y = space::SM;
    let names: Vec<String> = p
        .files
        .iter()
        .map(|f| Path::new(f).file_name().map_or(f.clone(), |n| n.to_string_lossy().into_owned()))
        .collect();
    let files = (!names.is_empty()).then(|| format!("Files: {}", names.join(", ")));
    let size = (!*show_text)
        .then(|| p.image.as_ref().map(|(_, [w, h])| format!("{w} \u{d7} {h} px \u{b7} first page")))
        .flatten();
    let error = p.error.clone().filter(|_| p.image.is_some() || *show_text);
    let warnings: Vec<String> = p.warnings.iter().filter(|w| !w.contains("Nearest")).cloned().collect();
    // Heights of the lines under the panel, so the panel takes exactly what is left.
    let line = |ui: &Ui, text: &str, inset: f32, max_rows: usize| {
        let mut job = egui::text::LayoutJob::simple(
            text.to_owned(),
            ui_font(font::CAPTION),
            c.text,
            (rect.width() - inset).max(40.0),
        );
        job.wrap.max_rows = max_rows;
        ui.fonts_mut(|f| f.layout_job(job)).size().y
    };
    let mut footer = 0.0;
    let mut add = |h: f32| footer += space::SM + h;
    if let Some(e) = &error {
        add(line(ui, e, 17.0, usize::MAX));
    }
    if size.is_some() {
        add(line(ui, "0", 0.0, 1));
    }
    if let Some(f) = &files {
        add(line(ui, f, 0.0, 1));
    }
    for w in &warnings {
        add(line(ui, w, 17.0, usize::MAX));
    }
    ui.push_id("preview_seg", |ui| {
        let options = [(SegGlyph::Text("Atlas"), "Atlas"), (SegGlyph::Text(".tres"), ".tres")];
        if let Some(i) = widgets::bar_segmented(ui, *show_text as usize, &options) {
            *show_text = i == 1;
        }
    });
    let height = (rect.height() - metric::CONTROL - space::SM - footer).max(40.0);
    let (panel, _) = ui.allocate_exact_size(vec2(rect.width(), height), Sense::hover());
    paint_panel(ui, p, *show_text, panel);
    if let Some(e) = &error {
        form::notice(ui, e, Tone::Error);
    }
    if let Some(t) = &size {
        ui.label(egui::RichText::new(t).font(ui_font(font::CAPTION)).color(c.text_tertiary));
    }
    if let Some(f) = &files {
        ui.add(egui::Label::new(egui::RichText::new(f).font(ui_font(font::CAPTION)).color(c.text_tertiary)).truncate());
    }
    for w in &warnings {
        form::notice(ui, w, Tone::Warning);
    }
}

/// The atlas on a checkerboard, the `.tres` text, or the empty state.
fn paint_panel(ui: &mut Ui, p: &PreviewData, show_text: bool, panel: Rect) {
    let c = Colors::of(ui.ctx());
    let painter = ui.painter().clone();
    if show_text {
        painter.rect_filled(panel, radius::PANEL, c.recessed);
        let mut inner = ui
            .new_child(UiBuilder::new().max_rect(panel.shrink(space::SM + 2.0)).layout(Layout::top_down(Align::LEFT)));
        inner.set_clip_rect(panel.intersect(ui.clip_rect()));
        egui::ScrollArea::both().id_salt("godot_tres").auto_shrink([false, false]).show(&mut inner, |ui| {
            let text = if p.text.is_empty() { " " } else { p.text.as_str() };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(text).font(egui::FontId::monospace(font::CAPTION)).color(c.text_secondary),
                )
                .extend(),
            );
        });
    } else if let Some((tex, [w, h])) = &p.image {
        let clipped = painter.with_clip_rect(panel);
        widgets::draw_checker(&clipped, panel, 8.0, radius::PANEL as f32);
        let avail = panel.shrink(space::SM);
        let scale = (avail.width() / *w as f32).min(avail.height() / *h as f32);
        let image = Rect::from_center_size(avail.center(), vec2(*w as f32 * scale, *h as f32 * scale));
        clipped.image(tex.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        painter.rect_stroke(panel, radius::PANEL, Stroke::new(1.0, c.separator), egui::StrokeKind::Inside);
    } else {
        painter.rect_filled(panel, radius::PANEL, c.recessed);
        let text = p.error.as_deref().unwrap_or("The atlas appears here.");
        let galley =
            painter.layout(text.to_owned(), ui_font(font::BODY), c.text_secondary, panel.width() - 2.0 * space::LG);
        let total = 24.0 + space::SM + galley.size().y;
        let top = panel.center().y - total / 2.0;
        painter.text(
            pos2(panel.center().x, top + 12.0),
            egui::Align2::CENTER_CENTER,
            icon::IMAGES,
            egui::FontId::proportional(24.0),
            c.text_tertiary,
        );
        let pos = pos2(panel.center().x - galley.size().x / 2.0, top + 24.0 + space::SM);
        painter.galley(pos, galley, c.text_secondary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pixmith-godot-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn drawn_app() -> PixmithApp {
        let mut app = PixmithApp::default();
        let doc = &mut app.tab_mut().doc;
        doc.begin_edit("Shape");
        doc.draw_shape(crate::ui::canvas::ShapeKind::Rect, 0, 0, (2, 2), (20, 20), 0xE8A862FF, 1, 8, false);
        doc.stroke_end();
        app
    }

    #[test]
    fn settings_json_round_trips() {
        let s = GodotSettings {
            root: "/p".into(),
            tres: "/p/art/a \"b\".tres".into(),
            padding: 3,
            extrude: false,
            grid: true,
            layer_mode: LayerMode::Selected,
            selected_layers: vec!["Body".into(), "Eyes".into()],
            tags_exclude: vec!["blink".into()],
            tscn: true,
            ..Default::default()
        };
        assert_eq!(GodotSettings::from_json(&s.json()), Some(s));
        assert_eq!(GodotSettings::from_json("[]"), None);
    }

    #[test]
    fn export_writes_files_and_remembers_the_settings() {
        let dir = scratch("export");
        let project = dir.join("game");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("project.godot"), "config_version=5\n").unwrap();
        let mut app = drawn_app();
        let mut sheet = GodotSheet::new(&app.tab().doc, "");
        assert!(!sheet.ready(), "nothing chosen yet");
        sheet.s.root = project.to_string_lossy().into_owned();
        sheet.s.tres = default_tres(&sheet.s.root, "hero");
        sheet.s.tscn = true;
        app.gamedev.prefs.clear_recent();
        export(&mut app, &mut sheet);
        assert_eq!(sheet.message.as_ref().map(|m| m.1), Some(false), "{:?}", sheet.message);
        assert!(sheet.done);
        assert!(sheet.message.as_ref().unwrap().0.starts_with("Exported."));
        assert!(project.join("art/hero.tres").exists(), "SpriteFrames written");
        assert!(project.join("art/hero.tscn").exists(), "scene written");
        assert!(app.tab().doc.is_dirty(), "the settings changed the document");
        let again = GodotSheet::new(&app.tab().doc, "/ignored");
        assert_eq!(again.s, sheet.s, "settings come back from the document");
        // The "blurry" hint is shown once.
        assert!(app.gamedev.prefs.godot_warned());
        assert_eq!(app.gamedev.prefs.godot_root(), project.to_string_lossy());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn export_errors_stay_in_the_sheet() {
        let mut app = drawn_app();
        let mut sheet = GodotSheet::new(&app.tab().doc, "");
        sheet.s.root = "/definitely/not/a/project".into();
        sheet.s.tres = "/definitely/not/a/project/art/x.tres".into();
        export(&mut app, &mut sheet);
        let (msg, err) = sheet.message.expect("message");
        assert!(err && !msg.is_empty());
        assert!(!sheet.done);
    }

    #[test]
    fn preview_builds_the_atlas_and_the_tres_text() {
        let dir = scratch("preview");
        std::fs::write(dir.join("project.godot"), "config_version=5\n").unwrap();
        let app = drawn_app();
        let ctx = egui::Context::default();
        let mut sheet = GodotSheet::new(&app.tab().doc, "");
        sheet.refresh_preview(&app.tab().doc, &ctx);
        assert!(sheet.preview.error.as_deref().is_some_and(|e| e.contains("Choose the project folder")));
        sheet.s.root = dir.to_string_lossy().into_owned();
        sheet.s.tres = default_tres(&sheet.s.root, "hero");
        sheet.refresh_preview(&app.tab().doc, &ctx);
        assert!(sheet.preview.error.is_none(), "{:?}", sheet.preview.error);
        assert!(sheet.preview.text.contains("SpriteFrames"), "{}", sheet.preview.text);
        assert!(sheet.preview.image.is_some());
        assert!(sheet.preview.files.iter().any(|f| f.ends_with("hero.tres")));
        assert!(!dir.join("art").exists(), "a preview writes nothing");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reexport_repeats_the_saved_export_or_opens_the_sheet() {
        let dir = scratch("reexport");
        std::fs::write(dir.join("project.godot"), "config_version=5\n").unwrap();
        let mut app = drawn_app();
        reexport(&mut app);
        assert!(app.gamedev.godot.is_some(), "nothing saved yet: the sheet opens");
        app.gamedev.godot = None;
        let mut s = GodotSettings::default();
        s.root = dir.to_string_lossy().into_owned();
        s.tres = default_tres(&s.root, "again");
        assert!(app.tab_mut().doc.set_godot_settings(&s.json()));
        reexport(&mut app);
        assert!(app.gamedev.godot.is_none() && app.error.is_none(), "{:?}", app.error);
        assert!(dir.join("art/again.tres").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tags_are_listed_as_animations() {
        let mut app = drawn_app();
        let doc = &mut app.tab_mut().doc;
        doc.add_frame(0, false);
        doc.add_tag(&crate::ui::timeline::TagInfo {
            name: "idle".into(),
            from: 0,
            to: 1,
            direction: 0,
            repeat: 0,
            color: 0x3B82F6,
        });
        assert_eq!(app.tab().doc.tag_count(), 1);
    }
}
