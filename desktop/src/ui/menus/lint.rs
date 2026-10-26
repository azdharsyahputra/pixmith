//! FX > Lint Sprite…: a read-only report of what is wrong with the sprite (paper halo, holes, detached specks,
//! edge pixels without an outline, stray and semi-transparent pixels, colors outside the palette), per cel.
//! The report comes from the core (`pixmith_sprite_lint`) as text; this module parses and lists it.

use super::controls::{divider, sheet, value_slider};
use super::filters::TARGETS;
use crate::app::PixmithApp;
use crate::theme::{Colors, font, ui_font};
use crate::ui::widgets::{self, DialogAction};
use eframe::egui::{self, RichText};

/// One kind of finding in one cel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Category {
    pub name: String,
    pub count: usize,
    /// `(x, y, pixels)`; capped by the core, `count` is always complete.
    pub items: Vec<(u32, u32, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CelReport {
    pub layer: u32,
    pub frame: u32,
    pub categories: Vec<Category>,
}

impl CelReport {
    pub fn total(&self) -> usize {
        self.categories.iter().map(|c| c.count).sum()
    }
}

/// Parses the core's report: `cel <layer> <frame>`, then per category `<name> <count>` and `x y size` lines.
pub fn parse(text: &str) -> Vec<CelReport> {
    let mut out: Vec<CelReport> = Vec::new();
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["cel", l, f] => out.push(CelReport {
                layer: l.parse().unwrap_or(0),
                frame: f.parse().unwrap_or(0),
                categories: Vec::new(),
            }),
            [name, count] if name.chars().all(|c| c.is_ascii_alphabetic() || c == '_') => {
                if let Some(cel) = out.last_mut() {
                    cel.categories.push(Category {
                        name: name.to_string(),
                        count: count.parse().unwrap_or(0),
                        items: Vec::new(),
                    });
                }
            }
            [x, y, n] => {
                if let (Some(cat), Ok(x), Ok(y), Ok(n)) =
                    (out.last_mut().and_then(|c| c.categories.last_mut()), x.parse(), y.parse(), n.parse())
                {
                    cat.items.push((x, y, n));
                }
            }
            _ => {}
        }
    }
    out
}

/// The title and the explanation of each category.
fn describe(name: &str) -> (&'static str, &'static str) {
    match name {
        "light_edge" => {
            ("Light pixels on the outline ring", "Light, grayish pixels in the two outermost rings (paper halo).")
        }
        "holes" => ("Enclosed holes", "Transparent areas closed off by the sprite. Listed: first pixel and size."),
        "specks" => ("Detached specks", "Small pieces apart from the body. Listed: first pixel and size."),
        "no_outline" => (
            "Edge pixels with no outline",
            "Outer pixels that are not outline color and have no outline pixel next to them.",
        ),
        "strays" => ("Stray pixels", "Single pixels that differ from all of their neighbors."),
        "semi_transparent" => {
            ("Semi-transparent pixels", "Pixels that are neither fully opaque nor fully transparent.")
        }
        "off_palette" => ("Colors not in the palette", "Pixels whose color is missing from the palette."),
        _ => ("Other", ""),
    }
}

/// The open Lint Sprite sheet.
pub struct LintSession {
    target: usize,
    speck: i32,
    palette: bool,
    /// What the report below was computed for, with the document revision it saw.
    shown: Option<(usize, i32, bool, u32, u32)>,
    report: Vec<CelReport>,
}

impl LintSession {
    pub fn new() -> LintSession {
        LintSession { target: 0, speck: 4, palette: false, shown: None, report: Vec::new() }
    }
}

/// Draws the sheet while it is open; call every frame.
pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(mut s) = app.menus.lint.take() else { return };
    let (layer, frame) = (app.tab().layer, app.tab().frame);
    let key = (s.target, s.speck, s.palette, layer, frame);
    if s.shown != Some(key) {
        let params = [s.speck, 64, s.palette as i32];
        let text = app.tab().doc.sprite_lint(s.target as u32, layer, frame, &params);
        s.report = parse(&text);
        s.shown = Some(key);
    }
    let mut goto: Option<(u32, u32)> = None;
    let action =
        sheet(ctx, "lint", 420.0, "Lint Sprite", Some("Reports problems without changing anything."), false, |ui| {
            widgets::form_row(ui, "Check", |ui| widgets::popup(ui, &TARGETS, &mut s.target, None));
            value_slider(ui, "lint_speck", "Speck size (px)", &mut s.speck, 2..=32);
            ui.checkbox(&mut s.palette, "Report colors that are not in the palette");
            divider(ui);
            let c = Colors::of(ui.ctx());
            let total: usize = s.report.iter().map(CelReport::total).sum();
            let summary = if total == 0 { "No problems found.".to_string() } else { format!("{total} findings.") };
            ui.label(RichText::new(summary).font(ui_font(font::BODY)).color(c.text_secondary));
            egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
                for (n, cel) in s.report.iter().enumerate() {
                    if cel.total() == 0 {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Layer {}, frame {}", cel.layer + 1, cel.frame + 1)).strong());
                        if ui.small_button("Go to cel").clicked() {
                            goto = Some((cel.layer, cel.frame));
                        }
                    });
                    for cat in cel.categories.iter().filter(|c| c.count > 0) {
                        let (title, help) = describe(&cat.name);
                        egui::CollapsingHeader::new(format!("{title} ({})", cat.count))
                            .id_salt(("lint_cat", n, &cat.name))
                            .show(ui, |ui| {
                                ui.label(RichText::new(help).font(ui_font(font::BODY)).color(c.text_secondary));
                                for &(x, y, size) in &cat.items {
                                    let label = if size > 1 {
                                        format!("x {x}, y {y} ({size} px)")
                                    } else {
                                        format!("x {x}, y {y}")
                                    };
                                    ui.label(RichText::new(label).monospace());
                                }
                                if cat.count > cat.items.len() {
                                    let more = cat.count - cat.items.len();
                                    ui.label(RichText::new(format!("... and {more} more")).color(c.text_secondary));
                                }
                            });
                    }
                }
            });
            widgets::dialog_buttons(ui, None, None, "Close", true)
        });
    if let Some((l, f)) = goto {
        let tab = app.tab_mut();
        tab.layer = l;
        tab.frame = f;
    }
    if action != DialogAction::Primary {
        app.menus.lint = Some(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_core_report() {
        let text = "cel 0 1\nlight_edge 0\nholes 2\n3 4 1\n7 8 5\nspecks 0\ncel 1 1\nstrays 1\n2 2 1\n";
        let r = parse(text);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].layer, r[0].frame), (0, 1));
        assert_eq!(r[0].categories[1].name, "holes");
        assert_eq!(r[0].categories[1].items, vec![(3, 4, 1), (7, 8, 5)]);
        assert_eq!(r[0].total(), 2);
        assert_eq!(r[1].categories[0].items, vec![(2, 2, 1)]);
    }
}
