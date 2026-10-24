//! The context bar above the canvas: the active tool's options (`TopBar.swift` `ContextBar`, `DrawingTools.swift` `ToolOptionsBar`).
//!
//! Only what the tool uses is shown; the two mirror toggles stay at the right so nothing moves when the tool changes.

use crate::app::{PixmithApp, Tool};
use crate::doc::Doc;
use crate::theme::{self, Colors, space};
use crate::ui::canvas::{DrawOptions, INKS};
use crate::ui::selection;
use crate::ui::widgets::{self, SegGlyph};
use eframe::egui;
use egui_phosphor::regular as icon;

pub fn show(app: &mut PixmithApp, ui: &mut egui::Ui) {
    let c = Colors::of(ui.ctx());
    // The hairline under the bar (`DSToolbar` bottom edge).
    let r = ui.max_rect();
    ui.painter().hline(
        egui::Rangef::new(r.left() - space::MD, r.right() + space::MD),
        r.bottom() - 0.5,
        egui::Stroke::new(1.0, c.separator_soft),
    );
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM - 2.0;
        let title = crate::ui::tilemap::bar_title(app).unwrap_or(app.tool.title());
        ui.label(egui::RichText::new(title).font(widgets::strong(ui.ctx(), theme::font::BODY_LARGE)).color(c.text));
        options(app, ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = space::XS;
            let o = &mut app.canvas.opts;
            if widgets::bar_chip(ui, icon::FLIP_VERTICAL, o.mirror_y, "Mirror top/bottom across the canvas center")
                .clicked()
            {
                o.mirror_y = !o.mirror_y;
            }
            if widgets::bar_chip(ui, icon::FLIP_HORIZONTAL, o.mirror_x, "Mirror left/right across the canvas center")
                .clicked()
            {
                o.mirror_x = !o.mirror_x;
            }
        });
    });
}

/// Items of one option group, separated from the previous one by a hairline.
fn group(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM - 2.0;
        widgets::group_divider(ui);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            content(ui);
        });
    });
}

/// Size field and the square / round brush toggle (`BrushOptionsView`).
fn brush(ui: &mut egui::Ui, size: &mut u32, o: &mut DrawOptions) {
    widgets::bar_number_field(
        ui,
        "Size",
        size,
        (1, 64),
        "px",
        90.0,
        "Brush size ([ and ] change it). Drag the label to scrub.",
    );
    // A brush taken from the selection replaces the shape toggle (`BrushOptionsView`).
    if let Some((w, h)) = o.custom_brush {
        let c = Colors::of(ui.ctx());
        ui.spacing_mut().item_spacing.x = space::XS;
        ui.label(
            egui::RichText::new(format!("Brush {w}\u{d7}{h}"))
                .font(theme::ui_font(theme::font::CAPTION))
                .color(c.text_secondary),
        );
        if widgets::tools_bar_button(ui, "Reset", true).clicked() {
            Doc::brush_reset();
            o.custom_brush = Doc::brush_info();
        }
        return;
    }
    let options = [(SegGlyph::Square, "Square brush"), (SegGlyph::Circle, "Round brush")];
    if let Some(i) = widgets::bar_segmented(ui, o.circle_brush as usize, &options) {
        o.circle_brush = i == 1;
        Doc::set_brush_circle(o.circle_brush);
    }
}

/// The ink popup (`InkOptionsView` without the brush).
fn ink(ui: &mut egui::Ui, o: &mut DrawOptions) {
    widgets::bar_popup(
        ui,
        &mut o.ink,
        &INKS,
        120.0,
        "Ink mode. Shading steps along the palette from the primary toward the secondary color; right-click steps back.",
    );
}

fn pressure_size(ui: &mut egui::Ui, o: &mut DrawOptions) {
    if widgets::bar_chip_with(
        ui,
        o.pressure_size,
        "Scale the brush size by pen pressure (tablet only)",
        diagonal_arrows_glyph,
    )
    .clicked()
    {
        o.pressure_size = !o.pressure_size;
    }
}

fn pressure(ui: &mut egui::Ui, o: &mut DrawOptions) {
    pressure_size(ui, o);
    if widgets::bar_chip_with(
        ui,
        o.pressure_alpha,
        "Scale the color's opacity by pen pressure (tablet only)",
        half_circle_glyph,
    )
    .clicked()
    {
        o.pressure_alpha = !o.pressure_alpha;
    }
}

fn filled_chip(ui: &mut egui::Ui, o: &mut DrawOptions, help: &str) {
    if widgets::bar_chip(ui, icon::SQUARE, o.filled, help).clicked() {
        o.filled = !o.filled;
    }
}

fn options(app: &mut PixmithApp, ui: &mut egui::Ui) {
    if crate::ui::tilemap::bar_options(app, ui) {
        return;
    }
    let (o, size) = (&mut app.canvas.opts, &mut app.brush_size);
    match app.tool {
        Tool::Pencil => {
            group(ui, |ui| brush(ui, size, o));
            group(ui, |ui| ink(ui, o));
            group(ui, |ui| {
                if widgets::bar_chip_with(ui, o.pixel_perfect, "Remove L-shaped corners while drawing", stairs_glyph)
                    .clicked()
                {
                    o.pixel_perfect = !o.pixel_perfect;
                }
                pressure(ui, o);
            });
        }
        Tool::Eraser => {
            group(ui, |ui| brush(ui, size, o));
            group(ui, |ui| pressure(ui, o));
        }
        Tool::Spray => {
            group(ui, |ui| brush(ui, size, o));
            group(ui, |ui| {
                widgets::tools_slider(ui, "spray-density", "Density", &mut o.spray_density, (5.0, 100.0), "%", 150.0);
            });
            group(ui, |ui| ink(ui, o));
        }
        Tool::Blur => {
            group(ui, |ui| brush(ui, size, o));
            group(ui, |ui| {
                widgets::tools_slider(
                    ui,
                    "blur-strength",
                    "Strength",
                    &mut o.retouch_strength,
                    (5.0, 100.0),
                    "%",
                    150.0,
                );
            });
            group(ui, |ui| pressure_size(ui, o));
        }
        Tool::Jumble => {
            group(ui, |ui| brush(ui, size, o));
            group(ui, |ui| widgets::hint_label(ui, &[], "swaps nearby pixels"));
            group(ui, |ui| pressure_size(ui, o));
        }
        Tool::Fill => group(ui, |ui| {
            if widgets::bar_text_chip(ui, "Contiguous", o.contiguous, "Off: replace every matching pixel in the cel")
                .clicked()
            {
                o.contiguous = !o.contiguous;
            }
            widgets::bar_popup(
                ui,
                &mut o.fill_pattern,
                &["Solid", "Checker", "25%", "50%", "75%"],
                92.0,
                "Dither the fill between the primary and secondary colors",
            );
        }),
        Tool::Gradient => group(ui, |ui| {
            let kinds = [(SegGlyph::Text("Linear"), "Linear gradient"), (SegGlyph::Text("Radial"), "Radial gradient")];
            if let Some(i) = widgets::bar_segmented(ui, o.gradient_radial as usize, &kinds) {
                o.gradient_radial = i == 1;
            }
            widgets::bar_popup(
                ui,
                &mut o.gradient_dither,
                &["Smooth", "Dither 2\u{d7}2", "Dither 4\u{d7}4", "Dither 8\u{d7}8"],
                110.0,
                "Dithering",
            );
            widgets::hint_label(ui, &[], "primary \u{2192} secondary");
        }),
        Tool::Line => {
            group(ui, |ui| brush(ui, size, o));
            group(ui, |ui| ink(ui, o));
            group(ui, |ui| widgets::hint_label(ui, &["\u{21e7}"], "45\u{b0} angles"));
        }
        Tool::Rectangle | Tool::Ellipse => {
            group(ui, |ui| brush(ui, size, o));
            group(ui, |ui| {
                filled_chip(ui, o, "Filled shape (off: outline). Hold Shift for squares and circles");
                ink(ui, o);
            });
            group(ui, |ui| widgets::hint_label(ui, &["\u{21e7}"], "constrain"));
        }
        Tool::ShapePolygon | Tool::Contour => {
            group(ui, |ui| brush(ui, size, o));
            group(ui, |ui| {
                filled_chip(ui, o, "Filled shape (off: outline)");
                ink(ui, o);
            });
        }
        Tool::Curve => {
            group(ui, |ui| brush(ui, size, o));
            group(ui, |ui| ink(ui, o));
            group(ui, |ui| widgets::hint_label(ui, &["\u{21a9}"], "finish"));
        }
        Tool::Text => group(ui, |ui| widgets::hint_label(ui, &[], "Click where the text should start")),
        Tool::Wand => {
            group(ui, |ui| selection::wand_options(ui, &mut app.selection));
            group(ui, |ui| widgets::hint_label(ui, &["\u{21e7}", "\u{2325}"], "add / subtract"))
        }
        Tool::Marquee | Tool::Lasso | Tool::Polygon => {
            group(ui, |ui| widgets::hint_label(ui, &["\u{21e7}", "\u{2325}"], "add / subtract"))
        }
        Tool::Move => {
            group(ui, |ui| widgets::hint_label(ui, &["\u{2190}", "\u{2191}"], "nudge (\u{21e7} moves 10 px)"))
        }
        Tool::Picker => group(ui, |ui| widgets::hint_label(ui, &["\u{2325}"], "picks from any tool")),
        Tool::Slice => group(ui, |ui| widgets::hint_label(ui, &[], "Drag to create a slice; double-click to edit")),
    }
}

/// `stairs`: three steps rising to the right.
fn stairs_glyph(p: &egui::Painter, rect: egui::Rect, fg: egui::Color32) {
    let o = rect.center() - egui::vec2(6.0, 6.0);
    let pts: Vec<egui::Pos2> = [(0.0, 12.0), (4.0, 12.0), (4.0, 8.0), (8.0, 8.0), (8.0, 4.0), (12.0, 4.0), (12.0, 0.5)]
        .iter()
        .map(|&(x, y)| o + egui::vec2(x, y))
        .collect();
    p.add(egui::Shape::line(pts, egui::Stroke::new(1.5, fg)));
}

/// `circle.lefthalf.filled`: a ring with half of it solid.
fn half_circle_glyph(p: &egui::Painter, rect: egui::Rect, fg: egui::Color32) {
    let center = rect.center();
    p.circle_stroke(center, 6.25, egui::Stroke::new(1.5, fg));
    let arc: Vec<egui::Pos2> = (0..=16)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 16.0;
            center + egui::vec2(a.cos(), a.sin()) * 5.5
        })
        .collect();
    p.add(egui::Shape::convex_polygon(arc, fg, egui::Stroke::NONE));
}

/// `arrow.up.left.and.arrow.down.right`: a diagonal with an arrowhead at each end.
fn diagonal_arrows_glyph(p: &egui::Painter, rect: egui::Rect, fg: egui::Color32) {
    let c = rect.center();
    let stroke = egui::Stroke::new(1.5, fg);
    for dir in [egui::vec2(-1.0, -1.0), egui::vec2(1.0, 1.0)] {
        let tip = c + dir * 5.5;
        let tail = c + dir * 1.5;
        p.line_segment([tail, tip], stroke);
        p.line_segment([tip, tip - egui::vec2(dir.x * 4.5, 0.0)], stroke);
        p.line_segment([tip, tip - egui::vec2(0.0, dir.y * 4.5)], stroke);
    }
}
