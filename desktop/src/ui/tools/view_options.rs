//! The View Options window: onion skin, frame difference, tiled mode, pixel grid, custom grid and reference image
//! (`ViewOptionsWindow.swift`). A floating window of grouped sections, 380 pt wide.

use super::view::{GridType, ReferencePlacement, TiledMode};
use crate::app::PixmithApp;
use crate::theme::{self, Colors, font, radius, space};
use crate::ui::widgets::{self, NumberOpts};
use eframe::egui::{
    self, Align, Align2, Color32, Layout, Margin, Pos2, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, pos2, vec2,
};

const WIDTH: f32 = 380.0;
const ROW: f32 = 32.0;

/// A grouped-form section: a caption above a rounded card whose rows are separated by hairlines.
struct Card<'a> {
    ui: &'a mut Ui,
    rows: usize,
}

fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Card)) {
    let c = Colors::of(ui.ctx());
    ui.add_space(space::MD);
    ui.label(egui::RichText::new(title).font(widgets::strong(ui.ctx(), font::CAPTION)).color(c.text_secondary));
    ui.add_space(space::XS);
    egui::Frame::new().fill(c.panel2).corner_radius(egui::CornerRadius::same(radius::PANEL)).show(ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.set_width(ui.available_width());
        body(&mut Card { ui, rows: 0 });
    });
}

impl Card<'_> {
    /// The hairline between rows.
    fn separator(&mut self) {
        let c = Colors::of(self.ui.ctx());
        if self.rows > 0 {
            let (line, _) = self.ui.allocate_exact_size(vec2(self.ui.available_width(), 1.0), Sense::hover());
            self.ui.painter().hline(
                (line.left() + space::MD)..=(line.right() - space::MD),
                line.center().y,
                Stroke::new(1.0, c.separator_soft),
            );
        }
        self.rows += 1;
    }

    /// A row laid out by `content`, left to right.
    fn raw_row(&mut self, content: impl FnOnce(&mut Ui)) {
        self.separator();
        let w = self.ui.available_width();
        self.ui.allocate_ui(vec2(w, ROW), content);
    }

    /// One row: `content` gets the row's area, laid out right to left; the label sits at the left.
    fn row(&mut self, label: &str, content: impl FnOnce(&mut Ui)) {
        let c = Colors::of(self.ui.ctx());
        self.separator();
        let (rect, _) = self.ui.allocate_exact_size(vec2(self.ui.available_width(), ROW), Sense::hover());
        let label_w = if label.is_empty() {
            0.0
        } else {
            self.ui
                .painter()
                .text(
                    pos2(rect.left() + space::MD, rect.center().y),
                    Align2::LEFT_CENTER,
                    label,
                    theme::ui_font(font::BODY_LARGE),
                    c.text,
                )
                .width()
        };
        let area = Rect::from_min_max(
            pos2(rect.left() + space::MD + label_w + space::SM, rect.top()),
            pos2(rect.right() - space::MD, rect.bottom()),
        );
        let mut child = self.ui.new_child(UiBuilder::new().max_rect(area).layout(Layout::right_to_left(Align::Center)));
        child.spacing_mut().item_spacing.x = space::SM;
        content(&mut child);
    }

    fn toggle(&mut self, label: &str, value: &mut bool) {
        self.row(label, |ui| {
            switch(ui, value);
        });
    }

    fn stepper(&mut self, label: &str, value: &mut u32, range: (u32, u32)) {
        self.row(label, |ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            if widgets::ds_icon_button(ui, egui_phosphor::regular::PLUS, 22.0, 11.0, false, false).clicked() {
                *value = (*value + 1).min(range.1);
            }
            if widgets::ds_icon_button(ui, egui_phosphor::regular::MINUS, 22.0, 11.0, false, false).clicked() {
                *value = value.saturating_sub(1).max(range.0);
            }
        });
    }

    fn slider(&mut self, label: &str, value: &mut f64, range: (f64, f64)) {
        self.row(label, |ui| {
            thin_slider(ui, value, range);
        });
    }

    fn color(&mut self, label: &str, value: &mut [u8; 3]) {
        self.row(label, |ui| {
            ui.color_edit_button_srgb(value);
        });
    }

    fn picker(&mut self, label: &str, id: &str, selected: usize, options: &[&str]) -> Option<usize> {
        let mut chosen = None;
        self.row(label, |ui| {
            chosen = widgets::ds_popup(ui, id, selected, options, Some(170.0), false, "");
        });
        chosen
    }

    /// Two number fields side by side.
    fn pair(&mut self, id: &str, a: (&str, &mut i32), b: (&str, &mut i32)) {
        self.row("", |ui| {
            let o = NumberOpts { range: (-65536.0, 65536.0), width: Some(110.0), ..Default::default() };
            // Right to left: the second field first.
            for (k, (label, value)) in [b, a].into_iter().enumerate() {
                let mut v = *value as f64;
                widgets::ds_number_field(ui, (id, k, label), label, &mut v, &o);
                *value = v.round() as i32;
            }
        });
    }
}

/// A macOS-style switch.
fn switch(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let c = Colors::of(ui.ctx());
    let (rect, mut response) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(response.id, *on, 0.12);
    let off = if ui.ctx().theme() == egui::Theme::Dark { Color32::from_gray(0x55) } else { Color32::from_gray(0xC8) };
    let fill = off.lerp_to_gamma(c.accent, t);
    ui.painter().rect_filled(rect, 10.0, fill);
    let knob = pos2(rect.left() + 10.0 + t * (rect.width() - 20.0), rect.center().y);
    ui.painter().circle_filled(knob + vec2(0.0, 0.5), 8.0, Color32::from_black_alpha(40));
    ui.painter().circle_filled(knob, 8.0, Color32::WHITE);
    response
}

/// A thin track slider (`Slider`): accent fill up to a round thumb.
fn thin_slider(ui: &mut Ui, value: &mut f64, range: (f64, f64)) -> egui::Response {
    let c = Colors::of(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(200.0, 20.0), Sense::click_and_drag());
    if let Some(p) = response.interact_pointer_pos() {
        let f = ((p.x - rect.left() - 8.0) / (rect.width() - 16.0)).clamp(0.0, 1.0) as f64;
        *value = range.0 + f * (range.1 - range.0);
    }
    let frac = ((*value - range.0) / (range.1 - range.0)).clamp(0.0, 1.0) as f32;
    let x = rect.left() + 8.0 + frac * (rect.width() - 16.0);
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 8.0, 4.0));
    ui.painter().rect_filled(track, 2.0, c.field.gamma_multiply(1.4));
    ui.painter().rect_filled(Rect::from_min_max(track.min, pos2(x, track.max.y)), 2.0, c.accent);
    let center = pos2(x, rect.center().y);
    ui.painter().circle_filled(center + vec2(0.0, 0.5), 8.0, Color32::from_black_alpha(50));
    ui.painter().circle_filled(center, 8.0, Color32::WHITE);
    response
}

/// The window. Closing it with its button hides it; open it again from the menu.
pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    if !app.canvas.tools.view_options_open {
        return;
    }
    let c = Colors::of(ctx);
    let following = crate::ui::tilemap::active_iso(app.tab());
    let PixmithApp { timeline, canvas, .. } = app;
    let tools = &mut canvas.tools;
    let mut open = true;
    let frame = egui::Frame::new()
        .fill(c.panel)
        .stroke(Stroke::new(1.0, c.separator))
        .corner_radius(egui::CornerRadius::same(radius::PANEL))
        .inner_margin(Margin::symmetric(space::LG as i8, space::MD as i8))
        .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(90) });
    let mut pending_load = false;
    let mut pending_clear = false;
    egui::Window::new("View Options")
        .id(egui::Id::new("view-options-window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_pos(Pos2::new(ctx.content_rect().center().x - WIDTH / 2.0, 80.0))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(WIDTH - 2.0 * space::LG);
            egui::ScrollArea::vertical().max_height((ctx.content_rect().height() - 140.0).clamp(200.0, 520.0)).show(
                ui,
                |ui| {
                    let s = &mut tools.settings;
                    section(ui, "Onion Skin", |card| {
                        card.toggle("Enabled", &mut timeline.onion_skin);
                        let past = format!("Previous frames: {}", s.onion_past);
                        card.stepper(&past, &mut s.onion_past, (0, 3));
                        let future = format!("Next frames: {}", s.onion_future);
                        card.stepper(&future, &mut s.onion_future, (0, 3));
                        card.slider("Opacity", &mut s.onion_opacity, (0.05, 0.8));
                        card.slider("Falloff", &mut s.onion_falloff, (0.1, 1.0));
                        card.toggle("Tint past red, future blue", &mut s.onion_tint);
                    });
                    section(ui, "Frame Difference", |card| {
                        card.toggle("Enabled", &mut timeline.frame_diff);
                        card.slider("Opacity", &mut s.diff_opacity, (0.1, 1.0));
                        card.toggle("Active layer only", &mut s.diff_active_layer);
                        card.toggle("Compare first frame with last", &mut s.diff_wrap);
                    });
                    section(ui, "Tiled Mode", |card| {
                        let names: Vec<&str> = TiledMode::ALL.iter().map(|m| m.title()).collect();
                        let sel = TiledMode::ALL.iter().position(|m| *m == s.tiled).unwrap_or(0);
                        if let Some(i) = card.picker("Mode", "tiled", sel, &names) {
                            s.tiled = TiledMode::ALL[i];
                        }
                    });
                    section(ui, "Pixel Grid", |card| {
                        card.toggle("Show grid", &mut timeline.show_grid);
                        card.color("Color", &mut s.grid_color);
                        card.slider("Opacity", &mut s.grid_opacity, (0.02, 1.0));
                        let zoom = format!("Show at zoom \u{2265} {}x", s.grid_min_zoom);
                        card.stepper(&zoom, &mut s.grid_min_zoom, (1, 64));
                    });
                    section(ui, "Custom Grid", |card| {
                        card.toggle("Show custom grid", &mut s.custom_grid);
                        let names: Vec<&str> = GridType::ALL.iter().map(|t| t.title()).collect();
                        let sel = GridType::ALL.iter().position(|t| *t == s.custom_grid_type).unwrap_or(0);
                        if let Some(i) = card.picker("Type", "grid-type", sel, &names) {
                            s.custom_grid_type = GridType::ALL[i];
                        }
                        if s.custom_grid_type == GridType::Isometric
                            && let Some(g) = following
                        {
                            let text =
                                format!("Follows the active tilemap layer: {}\u{d7}{} at {}, {}", g.w, g.h, g.x, g.y);
                            card.raw_row(|ui| widgets::note(ui, &text));
                        } else if s.custom_grid_type == GridType::Isometric {
                            card.pair("iso-size", ("Tile W", &mut s.iso_grid_w), ("Tile H", &mut s.iso_grid_h));
                            card.pair("iso-offset", ("X offset", &mut s.iso_grid_x), ("Y offset", &mut s.iso_grid_y));
                        } else {
                            card.pair("size", ("W", &mut s.custom_grid_w), ("H", &mut s.custom_grid_h));
                            card.pair("offset", ("X offset", &mut s.custom_grid_x), ("Y offset", &mut s.custom_grid_y));
                        }
                        card.color("Color", &mut s.custom_grid_color);
                        card.slider("Opacity", &mut s.custom_grid_opacity, (0.05, 1.0));
                    });
                    section(ui, "Reference Image", |card| {
                        let has = tools.reference.is_some();
                        let name = tools.reference.as_ref().map_or("None".to_owned(), |r| r.name.clone());
                        card.raw_row(|ui| {
                            ui.horizontal_centered(|ui| {
                                ui.add_space(space::MD);
                                let c = Colors::of(ui.ctx());
                                let avail = ui.available_width() - 150.0;
                                ui.add_sized(
                                    vec2(avail.max(40.0), 20.0),
                                    egui::Label::new(
                                        egui::RichText::new(name).font(theme::ui_font(font::BODY_LARGE)).color(c.text),
                                    )
                                    .truncate(),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.add_space(space::MD);
                                    if widgets::tools_bar_button(ui, "Clear", has).clicked() {
                                        pending_clear = true;
                                    }
                                    if widgets::tools_bar_button(ui, "Load\u{2026}", true).clicked() {
                                        pending_load = true;
                                    }
                                });
                            });
                        });
                        card.toggle("Show", &mut s.show_reference);
                        card.slider("Opacity", &mut s.reference_opacity, (0.05, 1.0));
                        let names: Vec<&str> = ReferencePlacement::ALL.iter().map(|p| p.title()).collect();
                        let sel = ReferencePlacement::ALL.iter().position(|p| *p == s.reference_placement).unwrap_or(0);
                        if let Some(i) = card.picker("Placement", "placement", sel, &names) {
                            s.reference_placement = ReferencePlacement::ALL[i];
                        }
                        card.pair("ref-pos", ("X", &mut s.reference_x), ("Y", &mut s.reference_y));
                        card.slider("Scale %", &mut s.reference_scale, (10.0, 800.0));
                    });
                    ui.add_space(space::XS);
                },
            );
        });
    if !open {
        tools.view_options_open = false;
    }
    if pending_load {
        super::choose_reference(app, ctx);
    } else if pending_clear {
        super::clear_reference(app);
    }
}
