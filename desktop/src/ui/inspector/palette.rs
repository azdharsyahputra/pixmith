//! The Palette section (`PaletteEditor.swift`): swatch grid, insert, duplicate, delete, presets, load, save, sort
//! and live editing of the selected entry. Drag to reorder and color ramps are not ported yet.

use crate::doc::Doc;
use crate::theme::{Colors, font, space, ui_font};
use crate::ui::widgets::{self, ds_icon_button, ds_menu_button, inspector_section, rgba_color};
use eframe::egui::{self, Align, Color32, Layout, Rect, Sense, Stroke, StrokeKind, Ui, vec2};
use egui_phosphor::regular as icon;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

const COLUMNS: usize = 8;
const GAP: f32 = 3.0;

#[derive(Default)]
pub struct PaletteState {
    pub(super) selection: BTreeSet<u32>,
    anchor: Option<u32>,
    /// The entry being edited live and when it last changed; one undo step per session.
    session: Option<(u32, Instant)>,
}

impl PaletteState {
    /// The entry the live editor shows: the only selected one, or the anchor of a multiple selection.
    fn active(&self) -> Option<u32> {
        if self.selection.len() == 1 {
            return self.selection.iter().next().copied();
        }
        self.anchor.filter(|a| self.selection.contains(a))
    }
}

enum Action {
    Insert,
    AddPrimary,
    Duplicate,
    Delete,
    Preset(u32),
    Load,
    Save,
    Sort(bool),
}

/// Runs the section. Returns true if the document changed.
pub fn section(
    ui: &mut Ui,
    ps: &mut PaletteState,
    primary: &mut u32,
    secondary: &mut u32,
    doc: &mut Doc,
    error: &mut Option<String>,
) -> bool {
    let c = Colors::of(ui.ctx());
    let palette = doc.palette();
    let n = palette.len() as u32;
    ps.selection.retain(|&i| i < n);
    let indexed = doc.color_mode() == 2;
    let transparent = doc.transparent_index();
    let selection_len = ps.selection.len();

    let mut action: Option<Action> = None;
    let mut entry_edit: Option<(u32, u32)> = None;
    inspector_section(
        ui,
        "palette",
        "Palette",
        Some(&n.to_string()),
        |ui| {
            // Right to left: the menu, then the plus button.
            ds_menu_button(
                ui,
                icon::DOTS_THREE,
                "Palette actions: duplicate, delete, presets, load, save, sort",
                |ui| {
                    if ui.button("Add Primary Color to Palette").clicked() {
                        action = Some(Action::AddPrimary);
                    }
                    if ui.add_enabled(selection_len > 0, egui::Button::new("Duplicate Selected")).clicked() {
                        action = Some(Action::Duplicate);
                    }
                    if ui
                        .add_enabled(
                            selection_len > 0 && selection_len < n as usize,
                            egui::Button::new("Delete Selected"),
                        )
                        .clicked()
                    {
                        action = Some(Action::Delete);
                    }
                    ui.separator();
                    ui.menu_button("Load Preset", |ui| {
                        for (i, name) in Doc::palette_preset_names().iter().enumerate() {
                            if ui.button(name).clicked() {
                                action = Some(Action::Preset(i as u32));
                            }
                        }
                    });
                    if ui.button("Load Palette\u{2026}").clicked() {
                        action = Some(Action::Load);
                    }
                    if ui.button("Save Palette\u{2026}").clicked() {
                        action = Some(Action::Save);
                    }
                    ui.separator();
                    if ui.button("Sort by Hue").clicked() {
                        action = Some(Action::Sort(true));
                    }
                    if ui.button("Sort by Luminance").clicked() {
                        action = Some(Action::Sort(false));
                    }
                },
            );
            if ds_icon_button(ui, icon::PLUS, 24.0, 13.0, false, false)
                .on_hover_text("Insert the current color after the selection")
                .clicked()
            {
                action = Some(Action::Insert);
            }
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = space::SM;
            let modifiers = ui.input(|i| i.modifiers);
            let cell = (ui.available_width() - 4.0 - GAP * (COLUMNS as f32 - 1.0)) / COLUMNS as f32;
            let rows = palette.len().div_ceil(COLUMNS);
            let height = rows as f32 * cell + rows.saturating_sub(1) as f32 * GAP + 4.0;
            let (grid, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
            for (i, &color) in palette.iter().enumerate() {
                let (row, col) = (i / COLUMNS, i % COLUMNS);
                let r = Rect::from_min_size(
                    grid.min + vec2(2.0 + col as f32 * (cell + GAP), 2.0 + row as f32 * (cell + GAP)),
                    vec2(cell, cell),
                );
                let response = ui.interact(r, ui.id().with(("palette_swatch", i)), Sense::click());
                let selected = ps.selection.contains(&(i as u32));
                let is_primary = color == *primary;
                let painter = ui.painter();
                if color & 0xFF != 0xFF {
                    widgets::draw_checker(painter, r, 4.0, 3.0);
                }
                painter.rect_filled(r, 3.0, rgba_color(color));
                if indexed && i as u32 == transparent {
                    painter.text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        icon::PROHIBIT,
                        egui::FontId::proportional(10.0),
                        c.text_secondary,
                    );
                }
                let (alpha, w) = if is_primary && !selected { (178, 1.5) } else { (20, 1.0) };
                painter.rect_stroke(r, 3.0, Stroke::new(w, Color32::from_white_alpha(alpha)), StrokeKind::Middle);
                if selected {
                    painter.rect_stroke(r.expand(2.0), 4.0, Stroke::new(2.0, c.accent), StrokeKind::Middle);
                }
                if response.secondary_clicked() {
                    *secondary = color;
                }
                if response.clicked() {
                    let i = i as u32;
                    if modifiers.command {
                        if !ps.selection.remove(&i) {
                            ps.selection.insert(i);
                        }
                        ps.anchor = Some(i);
                    } else if let (true, Some(a)) = (modifiers.shift, ps.anchor) {
                        ps.selection = (a.min(i)..=a.max(i)).collect();
                    } else {
                        ps.selection = BTreeSet::from([i]);
                        ps.anchor = Some(i);
                        *primary = doc.snap_color(color);
                    }
                }
                response.on_hover_text(format!("#{i}"));
            }
            if let Some(i) = ps.active().filter(|&i| i < n) {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("Entry #{i}")).font(ui_font(font::CAPTION)).color(c.text_secondary),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let mut col = rgba_color(palette[i as usize]);
                        if egui::color_picker::color_edit_button_srgba(
                            ui,
                            &mut col,
                            egui::color_picker::Alpha::OnlyBlend,
                        )
                        .changed()
                        {
                            let [r, g, b, a] = col.to_srgba_unmultiplied();
                            entry_edit = Some((i, u32::from_be_bytes([r, g, b, a])));
                        }
                    })
                    .response
                    .on_hover_text("Edit the selected entry; pixels using it update live");
                });
            } else if selection_len > 1 {
                ui.label(
                    egui::RichText::new(format!("{selection_len} selected"))
                        .font(ui_font(font::CAPTION))
                        .color(c.text_secondary),
                );
            }
            if indexed {
                ui.label(
                    egui::RichText::new(format!("Indexed \u{b7} transparent #{transparent}"))
                        .font(ui_font(font::CAPTION))
                        .color(c.text_tertiary),
                );
            }
        },
    );

    let mut changed = false;
    let fail = |error: &mut Option<String>, r: Result<(), String>| {
        if let Err(e) = r {
            *error = Some(e);
        }
    };
    if let Some((i, rgba)) = entry_edit {
        // One undo step per edit session: it continues while the same entry keeps changing.
        let begin = ps.session.is_none_or(|(s, last)| s != i || last.elapsed() > Duration::from_millis(1500));
        let follows_primary = palette.get(i as usize) == Some(primary);
        if doc.edit_palette_entry(i, rgba, begin) {
            ps.session = Some((i, Instant::now()));
            if follows_primary {
                *primary = rgba;
            }
            changed = true;
        }
    }
    match action {
        Some(Action::Insert) => {
            let at = ps.selection.last().map_or(n, |m| m + 1);
            match doc.insert_palette_entry(at, *primary) {
                Ok(()) => {
                    ps.selection = BTreeSet::from([at]);
                    ps.anchor = Some(at);
                    changed = true;
                }
                Err(e) => *error = Some(e),
            }
        }
        Some(Action::AddPrimary) => changed = doc.palette_add(*primary),
        Some(Action::Duplicate) => {
            let sorted: Vec<u32> = ps.selection.iter().copied().collect();
            if doc.duplicate_palette_entries(&sorted) {
                ps.selection = sorted.iter().enumerate().map(|(k, e)| e + k as u32 + 1).collect();
                ps.anchor = ps.selection.first().copied();
                changed = true;
            }
        }
        Some(Action::Delete) => {
            let sorted: Vec<u32> = ps.selection.iter().copied().collect();
            if doc.remove_palette_entries(&sorted) {
                ps.selection.clear();
                ps.anchor = None;
                *primary = doc.snap_color(*primary);
                changed = true;
            }
        }
        Some(Action::Preset(i)) => {
            let r = doc.load_palette_preset(i);
            changed = r.is_ok();
            *primary = doc.snap_color(*primary);
            fail(error, r);
        }
        Some(Action::Load) => {
            if let Some(path) = rfd::FileDialog::new().add_filter("Palette", &["gpl", "hex"]).pick_file() {
                let r = doc.load_palette(&path);
                changed = r.is_ok();
                fail(error, r);
            }
        }
        Some(Action::Save) => {
            if let Some(path) =
                rfd::FileDialog::new().add_filter("Palette", &["gpl", "hex"]).set_file_name("palette.gpl").save_file()
            {
                fail(error, doc.save_palette(&path));
            }
        }
        Some(Action::Sort(by_hue)) => {
            doc.sort_palette(by_hue);
            changed = true;
        }
        None => {}
    }
    if changed {
        ps.selection.retain(|&i| i < doc.palette().len() as u32);
    }
    changed
}
