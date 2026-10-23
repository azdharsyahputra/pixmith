//! The right-hand inspector: Color, Palette, Layer, Cel and Sprite sections (`Inspector.swift`, `ColorPanel.swift`).
//!
//! Layers themselves live in the timeline. The Tilemap and Slices sections of the macOS app are not ported yet.

pub mod color;
mod palette;

use crate::app::{DocTab, PixmithApp};
use crate::theme::{Colors, font, space, ui_font};
use crate::ui::widgets::{NumberOpts, ds_icon_button, ds_number_field, ds_popup, ds_text_field, inspector_section};
use eframe::egui::{self, Align2, Id, Sense, Stroke, Ui, pos2, vec2};
use egui_phosphor::regular as icon;

/// Color modes in the order of `ColorMode` in `ColorModeView.swift`.
pub const COLOR_MODES: [&str; 3] = ["RGB Color", "Grayscale", "Indexed"];

/// Blend modes in the order of `BlendMode` in `LayerTools.swift`.
pub const BLEND_MODES: [&str; 19] = [
    "Normal",
    "Multiply",
    "Screen",
    "Overlay",
    "Darken",
    "Lighten",
    "Color Dodge",
    "Color Burn",
    "Hard Light",
    "Soft Light",
    "Difference",
    "Exclusion",
    "Hue",
    "Saturation",
    "Color",
    "Luminosity",
    "Addition",
    "Subtract",
    "Divide",
];

/// State of this region that outlives a frame.
#[derive(Default)]
pub struct InspectorState {
    /// Which drawing color the Color section edits (`ColorTarget.secondary`).
    pub secondary: bool,
    hsva: color::Hsva,
    hex: String,
    layer_name: String,
    /// The opacity scrub in progress already started its undo step.
    opacity_checkpointed: bool,
    palette: palette::PaletteState,
}

pub fn show(app: &mut PixmithApp, ui: &mut Ui) {
    let c = Colors::of(ui.ctx());
    let rect = ui.max_rect();
    let PixmithApp { tabs, active, primary, secondary, inspector: st, error, .. } = app;
    let tab = &mut tabs[*active];
    let mut changed = false;

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().scroll = egui::style::ScrollStyle::floating();
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        color::section(ui, st, primary, secondary, &tab.doc);
        changed |= palette::section(ui, &mut st.palette, primary, secondary, &mut tab.doc, error);
        changed |= layer_section(ui, st, tab);
        changed |= cel_section(ui, tab);
        changed |= sprite_section(ui, tab, primary, error);
    });
    // The hairline on the leading edge.
    ui.painter().vline(rect.left() + 0.5, rect.y_range(), Stroke::new(1.0, c.separator_soft));
    if changed {
        tab.changed();
    }
}

fn opacity_percent(raw: u8) -> f64 {
    raw as f64 / 255.0 * 100.0
}

fn opacity_raw(percent: f64) -> u8 {
    (percent / 100.0 * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Name, visibility, blend mode and opacity of the selected layer (`LayerSection`).
fn layer_section(ui: &mut Ui, st: &mut InspectorState, tab: &mut DocTab) -> bool {
    let layer = tab.layer;
    let doc = &mut tab.doc;
    let visible = doc.layer_visible(layer);
    let mut toggle_visible = false;
    let mut changed = false;
    inspector_section(
        ui,
        "layer",
        "Layer",
        None,
        |ui| {
            // Right to left: the properties button, then the eye.
            ds_icon_button(ui, icon::SLIDERS_HORIZONTAL, 24.0, 13.0, false, false).on_hover_text("Layer properties");
            let (glyph, tip) = if visible { (icon::EYE, "Hide layer") } else { (icon::EYE_SLASH, "Show layer") };
            if ds_icon_button(ui, glyph, 24.0, 13.0, false, false).on_hover_text(tip).clicked() {
                toggle_visible = true;
            }
        },
        |ui| {
            ui.spacing_mut().item_spacing = vec2(space::SM - 2.0, space::SM - 2.0);
            let name_id = Id::new("inspector_layer_name");
            if !ui.memory(|m| m.has_focus(name_id)) {
                let current = doc.layer_name(layer);
                if st.layer_name != current {
                    st.layer_name = current;
                }
            }
            let r = ds_text_field(ui, name_id, &mut st.layer_name, None).on_hover_text("Layer name");
            if r.lost_focus() {
                let n = st.layer_name.trim().to_string();
                if !n.is_empty() && n != doc.layer_name(layer) {
                    doc.rename_layer(layer, &n);
                    changed = true;
                }
            }
            ui.horizontal(|ui| {
                let opacity_width = 118.0;
                let blend = doc.layer_blend_mode(layer) as usize;
                let popup_width = ui.available_width() - opacity_width - ui.spacing().item_spacing.x;
                if let Some(b) = ds_popup(ui, "layer_blend", blend, &BLEND_MODES, Some(popup_width), true, "Blend mode")
                {
                    changed |= doc.set_layer_blend_mode(layer, b as u16);
                }
                let mut value = opacity_percent(doc.layer_opacity(layer));
                let opts = NumberOpts {
                    width: Some(opacity_width),
                    unit: "%",
                    help: "Layer opacity. Drag the label to scrub.",
                    ..Default::default()
                };
                let res = ds_number_field(ui, "layer_opacity", "Opacity", &mut value, &opts);
                if res.changed {
                    // One undo step per drag; every typed value is its own step.
                    let first = !res.scrubbing || !st.opacity_checkpointed;
                    doc.set_layer_opacity_undoable(layer, opacity_raw(value), first);
                    changed = true;
                }
                st.opacity_checkpointed = res.scrubbing;
            });
        },
    );
    if toggle_visible {
        doc.set_layer_visible(layer, !visible);
        changed = true;
    }
    changed
}

/// Opacity and link state of the selected cel (`CelSection`). Linking is visual only for now: clicking a linked
/// cel unlinks it, the link dialog is not ported.
fn cel_section(ui: &mut Ui, tab: &mut DocTab) -> bool {
    let (layer, frame) = (tab.layer, tab.frame);
    let doc = &mut tab.doc;
    let opacity = doc.cel_opacity(layer, frame);
    let linked = doc.cel_is_linked(layer, frame);
    let subtitle = format!("{} \u{b7} frame {}", doc.layer_name(layer), frame + 1);
    let mut changed = false;
    inspector_section(
        ui,
        "cel",
        "Cel",
        Some(&subtitle),
        |_| {},
        |ui| {
            let c = Colors::of(ui.ctx());
            ui.spacing_mut().item_spacing.x = space::SM - 2.0;
            ui.horizontal(|ui| {
                let mut value = opacity_percent(opacity.unwrap_or(255));
                let opts = NumberOpts {
                    width: Some(128.0),
                    unit: "%",
                    help: "Opacity of this cel. Drag the label to scrub.",
                    defer_scrub: true,
                    ..Default::default()
                };
                ui.add_enabled_ui(opacity.is_some(), |ui| {
                    if ds_number_field(ui, "cel_opacity", "Opacity", &mut value, &opts).changed {
                        changed |= doc.set_cel_opacity(layer, frame, opacity_raw(value));
                    }
                });
                // The Link… button fills the rest of the row.
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), crate::theme::metric::CONTROL), Sense::click());
                ui.painter().rect_filled(rect, crate::theme::radius::FIELD, c.field);
                let fg = if linked { c.accent } else { c.text_secondary };
                let label = if linked { "Linked" } else { "Link\u{2026}" };
                let glyph = if linked { icon::LINK } else { icon::LINK_SIMPLE };
                let g = ui.painter().layout_no_wrap(label.to_owned(), ui_font(font::BODY), fg);
                let width = 12.0 + 6.0 + g.size().x;
                let left = rect.center().x - width / 2.0;
                ui.painter().text(
                    pos2(left + 6.0, rect.center().y),
                    Align2::CENTER_CENTER,
                    glyph,
                    egui::FontId::proportional(13.0),
                    fg,
                );
                ui.painter().galley(pos2(left + 18.0, rect.center().y - g.size().y / 2.0), g, fg);
                if response.on_hover_text("Link cels across frames, or unlink this one").clicked() && linked {
                    changed |= doc.unlink_cel(layer, frame);
                }
            });
        },
    );
    changed
}

/// Read-only facts about the sprite, plus the color mode (`SpriteSection`).
fn sprite_section(ui: &mut Ui, tab: &mut DocTab, primary: &mut u32, error: &mut Option<String>) -> bool {
    let doc = &mut tab.doc;
    let mut changed = false;
    inspector_section(
        ui,
        "sprite",
        "Sprite",
        None,
        |_| {},
        |ui| {
            let c = Colors::of(ui.ctx());
            ui.spacing_mut().item_spacing = vec2(0.0, space::SM);
            let caption = ui_font(font::CAPTION);
            let label_width = ["Size", "Color mode", "Frames"]
                .iter()
                .map(|l| ui.painter().layout_no_wrap((*l).to_owned(), caption.clone(), c.text_tertiary).size().x)
                .fold(0.0, f32::max);
            let row = |ui: &mut Ui, label: &str, add: &mut dyn FnMut(&mut Ui)| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = space::LG;
                    let (rect, _) =
                        ui.allocate_exact_size(vec2(label_width, crate::theme::metric::CONTROL), Sense::hover());
                    ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, label, caption.clone(), c.text_tertiary);
                    add(ui);
                });
            };
            let value = |ui: &mut Ui, s: String| {
                ui.label(egui::RichText::new(s).font(ui_font(font::BODY)).color(c.text));
            };
            row(ui, "Size", &mut |ui| value(ui, format!("{} \u{d7} {} px", doc.width(), doc.height())));
            row(ui, "Color mode", &mut |ui| {
                let mode = doc.color_mode() as usize;
                if let Some(m) =
                    ds_popup(ui, "color_mode", mode, &COLOR_MODES, None, false, "Convert the sprite's color mode")
                    && (m != mode || m == 2)
                {
                    match doc.convert_color_mode(m as u32) {
                        Ok(()) => {
                            *primary = doc.snap_color(*primary);
                            changed = true;
                        }
                        Err(e) => *error = Some(e),
                    }
                }
            });
            row(ui, "Frames", &mut |ui| {
                let tags = doc.tag_count();
                let mut s = format!("{} \u{b7} {} ms", doc.frame_count(), doc.total_duration());
                if tags > 0 {
                    s += &format!(" \u{b7} {tags} tag{}", if tags == 1 { "" } else { "s" });
                }
                value(ui, s);
            });
        },
    );
    changed
}

#[cfg(test)]
mod tests {
    use super::color;
    use crate::doc::Doc;
    use crate::{PixmithApp, theme};
    use eframe::egui;
    use egui_kittest::Harness;

    /// Runs a few frames of the whole window (no GPU needed) and checks the inspector leaves the colors alone.
    #[test]
    fn window_with_inspector_runs() {
        let app = PixmithApp { primary: 0xF2A65AFF, ..Default::default() };
        // Fonts take effect one frame after they are installed, so the first pass only installs them.
        let mut installed = false;
        let mut harness = Harness::builder().with_size([1280.0, 800.0]).build_ui_state(
            move |ui, app: &mut PixmithApp| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                    return;
                }
                theme::apply(ui.ctx(), app.dark);
                app.ui(ui);
            },
            app,
        );
        harness.run();
        assert_eq!(harness.state().primary, 0xF2A65AFF);
        assert!(!harness.state().inspector.secondary);

        let click = |harness: &mut Harness<'_, PixmithApp>, pos: egui::Pos2, button: egui::PointerButton| {
            harness.hover_at(pos);
            harness.run();
            for pressed in [true, false] {
                harness.event(egui::Event::PointerButton { pos, button, pressed, modifiers: Default::default() });
                harness.run();
            }
        };

        // A click in the middle of the saturation/value plane picks 50% / 50% of the current hue.
        click(&mut harness, egui::pos2(1280.0 - 8.0 - 136.0, 8.0 + 40.0 + 36.0 + 48.0), egui::PointerButton::Primary);
        let picked = color::Hsva::from_rgba(harness.state().primary, None);
        assert!((picked.s - 0.5).abs() < 0.05 && (picked.v - 0.5).abs() < 0.05, "{picked:?}");
        assert!((picked.h - 30.0).abs() < 2.0);

        // Clicking the first palette swatch makes it the primary color; right-click makes it the secondary.
        let first = harness.state().tab().doc.palette()[0];
        click(&mut harness, egui::pos2(1031.0, 360.0), egui::PointerButton::Primary);
        assert_eq!(harness.state().primary, first);
        click(&mut harness, egui::pos2(1031.0, 360.0), egui::PointerButton::Secondary);
        assert_eq!(harness.state().secondary, first);
    }

    #[test]
    fn palette_and_layer_calls_round_trip() {
        let mut doc = Doc::new(8, 8);
        let n = doc.palette().len();
        assert!(doc.palette_add(0x12345678 | 0xFF));
        assert_eq!(doc.palette().len(), n + 1);
        assert_eq!(doc.palette().last().copied(), Some(0x123456FF));
        assert!(!doc.palette_add(0x123456FF), "duplicates are ignored");
        doc.insert_palette_entry(0, 0xAABBCCFF).unwrap();
        assert_eq!(doc.palette()[0], 0xAABBCCFF);

        let layer = doc.layer_count() - 1;
        doc.set_layer_opacity_undoable(layer, 128, true);
        assert_eq!(doc.layer_opacity(layer), 128);
        assert!(doc.undo());
        assert_eq!(doc.layer_opacity(layer), 255);

        doc.rename_layer(layer, "Sky");
        assert_eq!(doc.layer_name(layer), "Sky");
        doc.set_layer_visible(layer, false);
        assert!(!doc.layer_visible(layer));
        assert!(doc.set_layer_blend_mode(layer, 1));
        assert_eq!(doc.layer_blend_mode(layer), 1);
        assert_eq!(doc.cel_opacity(layer, 0), None, "empty cels have no opacity");
        // SAFETY: a live document; draws one pixel so the cel exists.
        unsafe { pixmith_core::ffi::document::pixmith_draw_line(doc.raw(), layer, 0, 1, 1, 1, 1, 0xFF0000FF, 1) };
        assert_eq!(doc.cel_opacity(layer, 0), Some(255));
        assert!(doc.set_cel_opacity(layer, 0, 100));
        assert_eq!(doc.cel_opacity(layer, 0), Some(100));
        assert_eq!(doc.color_mode(), 0);
        assert_eq!(doc.total_duration(), doc.frame_duration(0));
    }
}
