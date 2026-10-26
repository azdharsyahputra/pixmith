//! WebP, JPEG, BMP, TGA, ICO, PCX and FLI/FLC (`Dialogs/ImageFormats.swift` in the macOS app).
//!
//! Import: Open, drag and drop and File > Import > Image take these files as new, unsaved sprites; an icon with
//! several sizes asks which one to open. Export: they are entries of the Export sheet (`ExportKind::Webp` ...), each
//! with the options of its format.

use crate::app::PixmithApp;
use crate::calls::images::ImageInfo;
use crate::doc::Doc;
use crate::theme::{Colors, font, ui_font};
use crate::ui::dialogs::ExportKind;
use crate::ui::widgets::{self, DialogAction};
use eframe::egui::{self, Ui};
use std::path::{Path, PathBuf};

/// Everything the Open panel offers: documents, PNG and the image formats of this module.
pub const OPEN_EXTENSIONS: [&str; 21] = [
    "pxmt", "aseprite", "ase", "png", "gif", "webp", "jpg", "jpeg", "jpe", "jfif", "bmp", "dib", "tga", "icb", "vda",
    "vst", "ico", "cur", "pcx", "flc", "fli",
];

/// Extensions that open as an import through this module.
const IMPORT_EXTENSIONS: [&str; 17] = [
    "webp", "jpg", "jpeg", "jpe", "jfif", "bmp", "dib", "tga", "icb", "vda", "vst", "ico", "cur", "pcx", "flc", "fli",
    "gif",
];

/// Whether the file name says it is one of the formats opened here (PNG and Pixmith documents have their own paths).
pub fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMPORT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The export formats of this module, in menu order.
pub const EXPORT_KINDS: [ExportKind; 7] = [
    ExportKind::Webp,
    ExportKind::Jpeg,
    ExportKind::Bmp,
    ExportKind::Tga,
    ExportKind::Ico,
    ExportKind::Pcx,
    ExportKind::Flc,
];

pub fn is_image_kind(kind: ExportKind) -> bool {
    EXPORT_KINDS.contains(&kind)
}

/// Title of the Export sheet.
pub fn title(kind: ExportKind) -> &'static str {
    match kind {
        ExportKind::Webp => "Export WebP",
        ExportKind::Jpeg => "Export JPEG",
        ExportKind::Bmp => "Export BMP",
        ExportKind::Tga => "Export TGA",
        ExportKind::Ico => "Export Windows Icon",
        ExportKind::Pcx => "Export PCX",
        ExportKind::Flc => "Export FLI/FLC Animation",
        _ => "Export",
    }
}

/// Menu label.
pub fn menu_label(kind: ExportKind) -> &'static str {
    match kind {
        ExportKind::Webp => "WebP\u{2026}",
        ExportKind::Jpeg => "JPEG\u{2026}",
        ExportKind::Bmp => "BMP\u{2026}",
        ExportKind::Tga => "TGA\u{2026}",
        ExportKind::Ico => "Windows Icon (ICO)\u{2026}",
        ExportKind::Pcx => "PCX\u{2026}",
        ExportKind::Flc => "FLI/FLC Animation\u{2026}",
        _ => "",
    }
}

/// Extension written by default.
pub fn extension(kind: ExportKind) -> &'static str {
    match kind {
        ExportKind::Webp => "webp",
        ExportKind::Jpeg => "jpg",
        ExportKind::Bmp => "bmp",
        ExportKind::Tga => "tga",
        ExportKind::Ico => "ico",
        ExportKind::Pcx => "pcx",
        _ => "flc",
    }
}

/// File name and save-panel filter for an export of this kind.
pub fn save_name(kind: ExportKind, base: &str) -> (String, (&'static str, &'static [&'static str])) {
    let filter: (&'static str, &'static [&'static str]) = match kind {
        ExportKind::Webp => ("WebP", &["webp"]),
        ExportKind::Jpeg => ("JPEG", &["jpg", "jpeg"]),
        ExportKind::Bmp => ("BMP", &["bmp"]),
        ExportKind::Tga => ("TGA", &["tga"]),
        ExportKind::Ico => ("Windows icon", &["ico"]),
        ExportKind::Pcx => ("PCX", &["pcx"]),
        _ => ("FLI/FLC", &["flc", "fli"]),
    };
    (format!("{base}.{}", extension(kind)), filter)
}

pub const ICO_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

/// BMP and TGA depths, and the values they stand for.
const BITS_LABELS: [&str; 4] = ["Automatic", "256 colors", "24 bit", "32 bit with alpha"];
const BITS_VALUES: [u32; 4] = [0, 8, 24, 32];
const PCX_LABELS: [&str; 3] = ["Automatic", "256 colors", "24 bit"];

/// The settings of the formats of this module (`ImageExportOptions` in the macOS app).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageSettings {
    /// JPEG quality, and WebP quality when lossy.
    pub quality: u32,
    pub lossless: bool,
    /// WebP: every frame as an animation.
    pub animated: bool,
    /// What transparent pixels turn into in formats without alpha.
    pub background: [u8; 3],
    /// Index into [`BITS_LABELS`] (PCX: [`PCX_LABELS`]).
    pub bits: usize,
    pub rle: bool,
    /// Bit k set: `ICO_SIZES[k]` is written. No bit: the sprite's own size.
    pub ico_sizes: u16,
}

impl Default for ImageSettings {
    fn default() -> ImageSettings {
        ImageSettings {
            quality: 90,
            lossless: true,
            animated: true,
            background: [255, 255, 255],
            bits: 0,
            rle: true,
            ico_sizes: 0,
        }
    }
}

impl ImageSettings {
    pub fn ico_size_list(&self) -> Vec<u32> {
        ICO_SIZES.iter().enumerate().filter(|(k, _)| self.ico_sizes >> k & 1 == 1).map(|(_, s)| *s).collect()
    }

    /// The options object of `pixmith_export_image`.
    pub fn json(&self) -> String {
        let [r, g, b] = self.background;
        let sizes: Vec<String> = self.ico_size_list().iter().map(u32::to_string).collect();
        format!(
            "{{\"quality\":{},\"lossless\":{},\"animated\":{},\"background\":\"#{r:02x}{g:02x}{b:02x}\",\"bits\":{},\"rle\":{},\"ico_sizes\":[{}]}}",
            self.quality.clamp(1, 100),
            self.lossless,
            self.animated,
            BITS_VALUES[self.bits.min(3)],
            self.rle,
            sizes.join(",")
        )
    }

    /// Whether the export writes the whole animation.
    pub fn all_frames(&self, kind: ExportKind) -> bool {
        kind == ExportKind::Flc || (kind == ExportKind::Webp && self.animated)
    }
}

/// The option rows of the Export sheet for a format of this module.
pub fn options_ui(ui: &mut Ui, kind: ExportKind, s: &mut ImageSettings) {
    let c = Colors::of(ui.ctx());
    let background = |ui: &mut Ui, s: &mut ImageSettings| {
        widgets::form_row(ui, "Background", |ui| {
            ui.color_edit_button_srgb(&mut s.background)
                .on_hover_text("Transparent pixels are flattened onto this color");
            ui.label(
                egui::RichText::new(format!("#{:02X}{:02X}{:02X}", s.background[0], s.background[1], s.background[2]))
                    .font(ui_font(font::CAPTION))
                    .color(c.text_secondary),
            );
        });
    };
    match kind {
        ExportKind::Webp => {
            widgets::form_row(ui, "Compression", |ui| {
                if let Some(i) = widgets::bar_segmented(
                    ui,
                    usize::from(!s.lossless),
                    &[
                        (widgets::SegGlyph::Text("Lossless"), "Exact pixels"),
                        (widgets::SegGlyph::Text("Lossy"), "Smaller files"),
                    ],
                ) {
                    s.lossless = i == 0;
                }
            });
            if !s.lossless {
                widgets::form_row(ui, "Quality", |ui| {
                    widgets::number_field(ui, "Q", &mut s.quality, 1..=100, "", 120.0)
                });
            }
            ui.checkbox(&mut s.animated, "Animated (all frames)");
            widgets::note(
                ui,
                if s.lossless {
                    "Lossless keeps every pixel exactly, transparency included."
                } else {
                    "Lossy makes smaller files and blurs sharp edges; use it for large images, not pixel art."
                },
            );
        }
        ExportKind::Jpeg => {
            widgets::form_row(ui, "Quality", |ui| widgets::number_field(ui, "Q", &mut s.quality, 1..=100, "", 120.0));
            background(ui, s);
            widgets::note(ui, "JPEG has no transparency. Colors keep full detail (no chroma subsampling).");
        }
        ExportKind::Bmp | ExportKind::Tga | ExportKind::Pcx => {
            let labels: &[&str] = if kind == ExportKind::Pcx { &PCX_LABELS } else { &BITS_LABELS };
            s.bits = s.bits.min(labels.len() - 1);
            widgets::form_row(ui, "Depth", |ui| {
                widgets::popup(ui, labels, &mut s.bits, None);
            });
            if kind == ExportKind::Tga {
                ui.checkbox(&mut s.rle, "Run-length compression");
            }
            background(ui, s);
            widgets::note(
                ui,
                if kind == ExportKind::Pcx {
                    "PCX has no transparency."
                } else {
                    "Transparency needs 32 bit; the other depths flatten onto the background."
                },
            );
        }
        ExportKind::Ico => {
            widgets::form_row(ui, "Sizes", |ui| {
                for (k, size) in ICO_SIZES.iter().enumerate() {
                    let on = s.ico_sizes >> k & 1 == 1;
                    if widgets::bar_text_chip(ui, &size.to_string(), on, &format!("{size} \u{00D7} {size} pixels"))
                        .clicked()
                    {
                        s.ico_sizes ^= 1 << k;
                    }
                }
            });
            widgets::note(
                ui,
                if s.ico_sizes == 0 {
                    "No size chosen: the icon has the sprite's own size (up to 256 pixels)."
                } else {
                    "The sprite is scaled by a whole number to fit each square, and centered."
                },
            );
        }
        ExportKind::Flc => {
            background(ui, s);
            widgets::note(ui, "FLI/FLC has 256 colors and no transparency; every frame and its duration is kept.");
        }
        _ => {}
    }
}

/// The formats of this module as items of the Export menu; returns the one clicked.
pub fn export_menu(ui: &mut Ui) -> Option<ExportKind> {
    let mut chosen = None;
    for kind in EXPORT_KINDS {
        if ui.button(menu_label(kind)).clicked() {
            chosen = Some(kind);
        }
    }
    chosen
}

// ---- Import --------------------------------------------------------------------------------------

/// The question "which size of this icon?" while it is open.
#[derive(Clone, Debug)]
pub struct IconChoice {
    pub path: PathBuf,
    /// Ascending sizes the file holds.
    pub sizes: Vec<u32>,
    /// Index into `sizes`.
    pub selected: usize,
}

impl PixmithApp {
    /// Opens an image file as a new sprite; an icon with several sizes asks first.
    pub fn open_image_path(&mut self, path: &Path) {
        match Doc::image_info(path) {
            Ok(ImageInfo { sizes, .. }) if sizes.len() > 1 => {
                let selected = sizes.len() - 1;
                self.files.icon_choice = Some(IconChoice { path: path.to_path_buf(), sizes, selected });
            }
            _ => self.finish_open_image(path, None),
        }
    }

    /// Reads the file (`size` picks an icon size) and shows it in a tab.
    pub fn finish_open_image(&mut self, path: &Path, size: Option<u32>) {
        match Doc::import_image(path, size) {
            Ok(doc) => {
                let mut doc = doc.with_import_source(path);
                self.add_recent(path);
                doc.touch();
                self.present(doc);
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// File > Import > Image (WebP, JPEG, BMP, TGA, ICO, PCX, FLC).
    pub fn import_image_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new().add_filter("Images", &OPEN_EXTENSIONS[3..]).pick_file() {
            self.open_path(&path);
        }
    }
}

/// The "Open Icon" sheet.
pub fn icon_sheet(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(mut choice) = app.files.icon_choice.clone() else { return };
    let name = choice.path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let labels: Vec<String> = choice
        .sizes
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{s} \u{00D7} {s}{}", if i + 1 == choice.sizes.len() { "  (largest)" } else { "" }))
        .collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let action = widgets::dialog(
        ctx,
        "icon",
        360.0,
        "Which size of the icon do you want to open?",
        Some(&format!(
            "\u{201C}{name}\u{201D} holds {} sizes of the same icon. The one you choose opens as a new sprite.",
            choice.sizes.len()
        )),
        |ui| {
            widgets::form_row(ui, "Size", |ui| {
                widgets::popup(ui, &label_refs, &mut choice.selected, None);
            });
            widgets::dialog_buttons(ui, None, Some("Cancel"), "Open", true)
        },
    );
    match action {
        DialogAction::Primary => {
            app.files.icon_choice = None;
            let size = choice.sizes.get(choice.selected).copied();
            app.finish_open_image(&choice.path, size);
        }
        DialogAction::Cancel => app.files.icon_choice = None,
        _ => app.files.icon_choice = Some(choice),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_json_matches_what_the_core_reads() {
        let mut s =
            ImageSettings { quality: 55, lossless: false, background: [16, 32, 48], bits: 2, ..Default::default() };
        s.ico_sizes = 0b101;
        let o = pixmith_core::formats::raster::ImageOptions::from_json(&s.json()).unwrap();
        assert_eq!(
            (o.jpeg_quality, o.webp_quality, o.webp_lossless, o.background, o.bits),
            (55, 55, false, [16, 32, 48], 24)
        );
        assert_eq!(o.ico_size_list(), vec![16, 32]);
        assert_eq!(s.ico_size_list(), vec![16, 32]);
    }

    #[test]
    fn extensions_route_to_imports() {
        for name in ["a.webp", "a.JPG", "a.jpeg", "a.bmp", "a.tga", "a.ico", "a.pcx", "a.flc", "a.fli"] {
            assert!(is_image_path(Path::new(name)), "{name}");
        }
        for name in ["a.png", "a.pxmt", "a.aseprite", "a", "a.txt"] {
            assert!(!is_image_path(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn every_kind_has_a_name_extension_and_title() {
        for kind in EXPORT_KINDS {
            assert!(is_image_kind(kind));
            assert!(!menu_label(kind).is_empty() && title(kind).starts_with("Export"));
            let (name, filter) = save_name(kind, "hero");
            assert_eq!(name, format!("hero.{}", extension(kind)));
            assert!(filter.1.contains(&extension(kind)));
        }
        assert!(!is_image_kind(ExportKind::Png));
    }
}
