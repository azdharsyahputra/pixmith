//! Small app-level sheets: Canvas Size (`Selection.swift`), Cel Properties (`LayerTools.swift` `CelSheet`) and About.

use super::controls::{bar_button, divider, sheet, value_slider_fmt};
use crate::app::{DocTab, PixmithApp};
use crate::theme::{Colors, font, ui_font};
use crate::ui::timeline::playback;
use crate::ui::widgets::{self, DialogAction};
use eframe::egui::{self, Align, Layout};

pub const ANCHORS: [&str; 9] =
    ["Top Left", "Top", "Top Right", "Left", "Center", "Right", "Bottom Left", "Bottom", "Bottom Right"];

/// The Canvas Size dialog.
pub struct CanvasSizeSession {
    pub width: u32,
    pub height: u32,
    /// Index into [`ANCHORS`].
    pub anchor: usize,
}

impl CanvasSizeSession {
    pub fn new(width: u32, height: u32) -> CanvasSizeSession {
        CanvasSizeSession { width, height, anchor: 4 }
    }
}

/// Where the old pixels land: the anchor is a cell of a 3 x 3 grid (`showCanvasSizeDialog`).
pub fn anchor_offset(old: (u32, u32), new: (u32, u32), anchor: usize) -> (i32, i32) {
    let (ax, ay) = ((anchor % 3) as i32, (anchor / 3) as i32);
    ((new.0 as i32 - old.0 as i32) * ax / 2, (new.1 as i32 - old.1 as i32) * ay / 2)
}

/// Draws the Canvas Size dialog; returns true once it is closed.
pub fn canvas_size(app: &mut PixmithApp, ctx: &egui::Context, s: &mut CanvasSizeSession) -> bool {
    let action = widgets::dialog(
        ctx,
        "canvas_size",
        320.0,
        "Canvas Size",
        Some("Resizes every frame and layer. Anchor decides where existing pixels stay."),
        |ui| {
            widgets::form_row(ui, "Width", |ui| {
                widgets::number_field(ui, "W", &mut s.width, 1..=16384, "px", 140.0);
            });
            widgets::form_row(ui, "Height", |ui| {
                widgets::number_field(ui, "H", &mut s.height, 1..=16384, "px", 140.0);
            });
            widgets::form_row(ui, "Anchor", |ui| {
                widgets::popup(ui, &ANCHORS, &mut s.anchor, Some(140.0));
            });
            widgets::dialog_buttons(ui, None, Some("Cancel"), "Resize", true)
        },
    );
    match action {
        DialogAction::Primary => {
            let tab = app.tab_mut();
            playback::stop(tab);
            let (w, h) = (tab.doc.width(), tab.doc.height());
            let (dx, dy) = anchor_offset((w, h), (s.width, s.height), s.anchor);
            if tab.doc.resize_canvas(s.width, s.height, dx, dy) {
                (tab.layer, tab.frame) = tab.doc.clamp_selection(tab.layer, tab.frame);
                tab.changed();
            }
            true
        }
        DialogAction::Cancel => true,
        _ => false,
    }
}

/// The Cel Properties sheet: opacity of the current cel and linking of cels across frames.
pub struct CelSession {
    pub opacity: i32,
    pub from: u32,
    pub to: u32,
}

impl CelSession {
    pub fn new(tab: &DocTab) -> CelSession {
        let frames = tab.doc.frame_count().max(1);
        CelSession {
            opacity: tab.doc.cel_opacity(tab.layer, tab.frame).unwrap_or(255) as i32,
            from: tab.frame + 1,
            to: (tab.frame + 2).min(frames),
        }
    }
}

/// Draws the Cel Properties sheet; returns true once it is closed.
pub fn cel(app: &mut PixmithApp, ctx: &egui::Context, s: &mut CelSession) -> bool {
    let tab = app.tab_mut();
    let (layer, frame) = (tab.layer, tab.frame);
    let frames = tab.doc.frame_count().max(1);
    let has_cel = tab.doc.cel_opacity(layer, frame).is_some();
    let linked = tab.doc.cel_is_linked(layer, frame);
    let subtitle = format!("{} \u{b7} frame {}", tab.doc.layer_name(layer), frame + 1);
    let mut changed = false;
    let mut close = false;
    let c = Colors::of(ctx);
    let action = sheet(ctx, "cel", 380.0, "Cel Properties", Some(&subtitle), true, |ui| {
        ui.add_enabled_ui(has_cel, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let width = ui.available_width() - 8.0 - 64.0;
                ui.allocate_ui(egui::vec2(width, 24.0), |ui| {
                    value_slider_fmt(ui, "opacity", "Opacity", &mut s.opacity, 0..=255, |v| {
                        format!("{}%", (v as f32 / 255.0 * 100.0).round() as i32)
                    });
                });
                if widgets::secondary_button(ui, "Apply", false).clicked() {
                    changed |= tab.doc.set_cel_opacity(layer, frame, s.opacity.clamp(0, 255) as u8);
                }
            });
        });
        divider(ui);
        widgets::note(ui, "Linked cels share their image: editing one edits all.");
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            widgets::number_field(ui, "From", &mut s.from, 1..=frames, "", 110.0);
            widgets::number_field(ui, "To", &mut s.to, 1..=frames, "", 110.0);
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if bar_button(ui, "Link Cels", s.from < s.to, false)
                .on_hover_text("Make the cels in this frame range copies of the cel in the first frame")
                .clicked()
            {
                changed |= tab.doc.link_cels(layer, s.from - 1, s.to - 1);
                close = true;
            }
            if bar_button(ui, "Unlink This Cel", linked, false).clicked() {
                changed |= tab.doc.unlink_cel(layer, frame);
                close = true;
            }
        });
        ui.with_layout(Layout::top_down(Align::Min), |ui| widgets::dialog_buttons(ui, None, None, "Done", true)).inner
    });
    let _ = c;
    if changed {
        tab.changed();
    }
    close || action != DialogAction::None
}

/// Draws the About dialog; returns true once it is closed.
pub fn about(ctx: &egui::Context) -> bool {
    let c = Colors::of(ctx);
    let action =
        widgets::dialog(ctx, "about", 320.0, "Pixmith", Some(concat!("Version ", env!("CARGO_PKG_VERSION"))), |ui| {
            ui.label(
                egui::RichText::new("A lightweight pixel-art editor that reads and writes .aseprite files.")
                    .font(ui_font(font::BODY))
                    .color(c.text),
            );
            widgets::note(ui, "Copyright \u{a9} Pixmith contributors. MIT License.");
            widgets::dialog_buttons(ui, None, None, "OK", true)
        });
    action != DialogAction::None
}

/// The command line tool shipped next to the app (`pixmith-cli` / `pixmith-cli.exe`; not `pixmith.exe`, which
/// Windows would treat as the same name as `Pixmith.exe`), if it is there.
pub fn bundled_cli() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) { "pixmith-cli.exe" } else { "pixmith-cli" };
    let cli = exe.parent()?.join(name);
    cli.is_file().then_some(cli)
}

/// Help > Command Line Tool…: where the bundled `pixmith` command is and how to put it on the PATH. Returns
/// true when the dialog is dismissed.
pub fn command_line_tool(ctx: &egui::Context) -> bool {
    let c = Colors::of(ctx);
    let cli = bundled_cli();
    let folder = cli.as_ref().and_then(|p| p.parent()).map(|p| p.display().to_string());
    let action = widgets::dialog(ctx, "cli", 440.0, "Command Line Tool", None, |ui| {
        let body = |ui: &mut egui::Ui, text: &str| {
            ui.label(egui::RichText::new(text).font(ui_font(font::BODY)).color(c.text));
        };
        match &folder {
            Some(folder) => {
                body(ui, "Pixmith ships with the \u{201c}pixmith-cli\u{201d} command, in the same folder as the app:");
                ui.label(egui::RichText::new(folder.as_str()).monospace().color(c.text));
                if ui.button("Copy Folder Path").clicked() {
                    ui.ctx().copy_text(folder.clone());
                }
                if cfg!(windows) {
                    widgets::note(
                        ui,
                        "To run it from any terminal, add this folder to your PATH: Start \u{2192} \u{201c}Edit the \
                         system environment variables\u{201d} \u{2192} Environment Variables \u{2192} Path \u{2192} New.",
                    );
                } else {
                    widgets::note(
                        ui,
                        "To run it from any terminal, link it onto your PATH, for example:\n\
                         ln -s \"<folder>/pixmith-cli\" ~/.local/bin/pixmith",
                    );
                }
                widgets::note(
                    ui,
                    "Try: pixmith info hero.pxmt \u{2014} pixmith export hero.pxmt --gif hero.gif --scale 4",
                );
            }
            None => body(
                ui,
                "This build does not include the command line tool. Release downloads have it next to the app.",
            ),
        }
        widgets::dialog_buttons(ui, None, None, "OK", true)
    });
    action != DialogAction::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_place_the_old_pixels() {
        // Center: half the growth on each side.
        assert_eq!(anchor_offset((32, 32), (48, 40), 4), (8, 4));
        assert_eq!(anchor_offset((32, 32), (48, 40), 0), (0, 0));
        assert_eq!(anchor_offset((32, 32), (48, 40), 8), (16, 8));
        // Shrinking centers with a negative offset.
        assert_eq!(anchor_offset((32, 32), (16, 16), 4), (-8, -8));
    }
}
