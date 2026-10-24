//! Modal dialogs (`DSDialog` sheets in the macOS app): New Sprite, unsaved changes, export options, errors.
//! Also routes the file shortcuts, dropped files and the window close request, since they all end in a dialog.

use crate::app::PixmithApp;
use crate::theme::{Colors, font, ui_font};
use crate::ui::widgets::{self, DialogAction};
use eframe::egui::{self, Key, Modifiers};

/// What happens once the unsaved-changes question about a tab is settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    /// Close that tab.
    CloseTab,
    /// Carry on quitting: ask about the next dirty tab, or leave.
    Quit,
}

/// "Do you want to save the changes made to ...?" for one tab.
#[derive(Clone, Copy, Debug)]
pub struct Prompt {
    pub tab: usize,
    pub after: After,
    /// The tab that was showing before the question, to return to if this one closes.
    pub previous: usize,
}

/// The New Sprite sheet (`NewSpriteSheet`).
#[derive(Clone, Copy, Debug)]
pub struct NewSheet {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    Png,
    Sheet,
    Gif,
    Apng,
    Sequence,
    // WebP, JPEG, BMP, TGA, ICO, PCX and FLI/FLC (`ui/image_formats.rs`).
    Webp,
    Jpeg,
    Bmp,
    Tga,
    Ico,
    Pcx,
    Flc,
}

impl ExportKind {
    pub fn title(self) -> &'static str {
        match self {
            ExportKind::Png => "Export PNG",
            ExportKind::Sheet => "Export Sprite Sheet",
            ExportKind::Gif => "Export Animated GIF",
            ExportKind::Apng => "Export Animated PNG",
            ExportKind::Sequence => "Export PNG Sequence",
            other => crate::ui::image_formats::title(other),
        }
    }
}

pub const SCALES: [u32; 5] = [1, 2, 4, 8, 16];
pub const SCALE_LABELS: [&str; 5] = ["1x", "2x", "4x", "8x", "16x"];
pub const LAYOUTS: [&str; 3] = ["Horizontal strip", "Vertical strip", "Grid"];

/// Options collected before the save panel opens (the accessory views of `FileActions.swift`).
#[derive(Clone, Copy, Debug)]
pub struct ExportSheet {
    pub kind: ExportKind,
    /// Index into [`SCALES`].
    pub scale: usize,
    /// Index into [`LAYOUTS`].
    pub layout: usize,
    pub columns: u32,
    pub padding: u32,
    pub json: bool,
    /// Settings of the formats in `ui/image_formats.rs`.
    pub image: crate::ui::image_formats::ImageSettings,
}

impl ExportSheet {
    pub fn new(kind: ExportKind, frames: u32) -> ExportSheet {
        ExportSheet {
            kind,
            scale: 0,
            layout: 0,
            columns: frames.clamp(1, 4),
            padding: 0,
            json: true,
            image: Default::default(),
        }
    }
}

pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    app.handle_window_events(ctx);
    if app.error.is_some() {
        error_sheet(app, ctx);
    } else if app.files.prompt.is_some() {
        unsaved_sheet(app, ctx);
    } else if app.files.new_sheet.is_some() {
        new_sheet(app, ctx);
    } else if app.files.icon_choice.is_some() {
        crate::ui::image_formats::icon_sheet(app, ctx);
    } else if app.files.export.is_some() {
        export_sheet(app, ctx);
    } else if app.projects.has_sheet() {
        // The New Project / New Canvas sheet owns the keyboard.
    } else {
        file_shortcuts(app, ctx);
    }
}

/// Ctrl/Cmd+N, O, S, Shift+S and W.
fn file_shortcuts(app: &mut PixmithApp, ctx: &egui::Context) {
    let (new, open, save_as, save, close) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::COMMAND, Key::N),
            i.consume_key(Modifiers::COMMAND, Key::O),
            i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S),
            i.consume_key(Modifiers::COMMAND, Key::S),
            i.consume_key(Modifiers::COMMAND, Key::W),
        )
    });
    if new {
        app.request_new_sprite();
    } else if open {
        app.open_dialog();
    } else if save_as {
        app.save_as();
    } else if save {
        app.save();
    } else if close {
        app.request_close(app.active);
    }
}

fn error_sheet(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(message) = app.error.clone() else { return };
    let c = Colors::of(ctx);
    let action = widgets::dialog(ctx, "error", 340.0, "Something went wrong", None, |ui| {
        ui.label(egui::RichText::new(message).font(ui_font(font::BODY)).color(c.text_secondary));
        widgets::dialog_buttons(ui, None, None, "OK", true)
    });
    if action != DialogAction::None {
        app.error = None;
    }
}

fn unsaved_sheet(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(prompt) = app.files.prompt else { return };
    let name = app.tab_name(prompt.tab);
    let action = widgets::dialog(
        ctx,
        "unsaved",
        380.0,
        &format!("Do you want to save the changes made to \u{201C}{name}\u{201D}?"),
        Some("Your changes will be lost if you don\u{2019}t save them."),
        |ui| widgets::dialog_buttons(ui, Some("Don\u{2019}t Save"), Some("Cancel"), "Save", true),
    );
    match action {
        DialogAction::Primary => app.answer_prompt(Answer::Save),
        DialogAction::Destructive => app.answer_prompt(Answer::Discard),
        DialogAction::Cancel => app.answer_prompt(Answer::Cancel),
        DialogAction::None => {}
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Save,
    Discard,
    Cancel,
}

fn new_sheet(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(mut sheet) = app.files.new_sheet else { return };
    let action = widgets::dialog(ctx, "new", 300.0, "New Sprite", None, |ui| {
        widgets::form_row(ui, "Width", |ui| widgets::number_field(ui, "W", &mut sheet.width, 1..=4096, "px", 120.0));
        widgets::form_row(ui, "Height", |ui| widgets::number_field(ui, "H", &mut sheet.height, 1..=4096, "px", 120.0));
        widgets::dialog_buttons(ui, None, Some("Cancel"), "Create", true)
    });
    app.files.new_sheet = Some(sheet);
    match action {
        DialogAction::Primary => {
            app.files.new_sheet = None;
            app.files.new_size = (sheet.width, sheet.height);
            app.new_sprite(sheet.width, sheet.height);
        }
        DialogAction::Cancel => app.files.new_sheet = None,
        _ => {}
    }
}

fn export_sheet(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(mut sheet) = app.files.export else { return };
    let action = widgets::dialog(ctx, "export", 340.0, sheet.kind.title(), None, |ui| {
        if sheet.kind == ExportKind::Sheet {
            widgets::form_row(ui, "Layout", |ui| widgets::popup(ui, &LAYOUTS, &mut sheet.layout, None));
            widgets::form_row(ui, "Columns", |ui| {
                widgets::number_field(ui, "C", &mut sheet.columns, 1..=4096, "", 120.0)
            });
            widgets::form_row(ui, "Padding", |ui| {
                widgets::number_field(ui, "P", &mut sheet.padding, 0..=4096, "px", 120.0)
            });
        }
        widgets::form_row(ui, "Scale", |ui| widgets::popup(ui, &SCALE_LABELS, &mut sheet.scale, None));
        match sheet.kind {
            ExportKind::Sheet => {
                ui.checkbox(&mut sheet.json, "Write JSON metadata next to the PNG");
                widgets::note(ui, "Columns apply to the grid layout. Padding is in output pixels.");
            }
            ExportKind::Sequence => widgets::note(ui, "Frames are saved as name-001.png, name-002.png, \u{2026}"),
            ExportKind::Apng => widgets::note(ui, "Saves an animated PNG."),
            kind if crate::ui::image_formats::is_image_kind(kind) => {
                crate::ui::image_formats::options_ui(ui, kind, &mut sheet.image)
            }
            _ => {}
        }
        widgets::dialog_buttons(ui, None, Some("Cancel"), "Export\u{2026}", true)
    });
    app.files.export = Some(sheet);
    match action {
        DialogAction::Primary => {
            app.files.export = None;
            app.run_export(sheet);
        }
        DialogAction::Cancel => app.files.export = None,
        _ => {}
    }
}
