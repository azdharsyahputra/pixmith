//! The floating list of undo steps (`Panels/HistoryPanel.swift`, View > History). Clicking a row jumps to that state.

use crate::app::PixmithApp;
use crate::theme::{Colors, font, metric, radius, space, ui_font};
use crate::ui::widgets;
use eframe::egui::{self, CornerRadius, Sense, Stroke, vec2};

/// What the list was built for: the document, its step counts and its revision (labels change with edits).
type CacheKey = (usize, u32, u32, u64);

/// Step labels, rebuilt only when the step counts or the document change.
#[derive(Default)]
pub struct HistoryCache {
    key: Option<CacheKey>,
    titles: Vec<String>,
    /// The applied count the list last scrolled to.
    scrolled_to: Option<(usize, u32)>,
}

/// Draws the History window when it is open; call every frame.
pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    if !app.menus.history_open {
        return;
    }
    let c = Colors::of(ctx);
    let frame = egui::Frame::new()
        .fill(c.panel)
        .stroke(Stroke::new(1.0, c.separator))
        .corner_radius(CornerRadius::same(radius::PANEL))
        .shadow(egui::Shadow { offset: [0, 6], blur: 20, spread: 0, color: egui::Color32::from_black_alpha(80) });
    let mut open = true;
    let mut goto = None;
    {
        let PixmithApp { tabs, active, menus, .. } = app;
        let tab = &tabs[*active];
        let (applied, undone) = tab.doc.history_counts();
        let key = (tab.doc.raw() as usize, applied, undone, tab.revision);
        let cache = &mut menus.history;
        if cache.key != Some(key) {
            cache.key = Some(key);
            cache.titles = std::iter::once("Original".to_string())
                .chain((0..applied + undone).map(|i| tab.doc.history_label(i)))
                .collect();
        }
        let scroll = cache.scrolled_to != Some((key.0, applied));
        cache.scrolled_to = Some((key.0, applied));
        egui::Window::new("History")
            .id(egui::Id::new("history_window"))
            .default_pos([64.0, 112.0])
            .default_size([240.0, 360.0])
            .min_size([180.0, 120.0])
            .collapsible(false)
            .frame(frame)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add_space(space::SM - 2.0);
                    for (position, title) in cache.titles.iter().enumerate() {
                        let (current, is_undone) = (position == applied as usize, position > applied as usize);
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(ui.available_width(), metric::CONTROL + 2.0), Sense::click());
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
                        let fill = if current {
                            c.selected
                        } else if response.hovered() {
                            c.hover
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        ui.painter().rect_filled(rect, radius::CONTROL, fill);
                        let color = if is_undone {
                            c.text_tertiary
                        } else if current {
                            c.text
                        } else {
                            c.text_secondary
                        };
                        let font = if current { widgets::strong(ui.ctx(), font::BODY) } else { ui_font(font::BODY) };
                        ui.painter().with_clip_rect(rect).text(
                            egui::pos2(rect.left() + space::SM, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            title,
                            font,
                            color,
                        );
                        if current && scroll {
                            ui.scroll_to_rect(rect, None);
                        }
                        if response.clicked() {
                            goto = Some(position as u32);
                        }
                    }
                    ui.add_space(space::SM - 2.0);
                });
            });
    }
    if let Some(position) = goto {
        let tab = app.tab_mut();
        crate::ui::timeline::playback::stop(tab);
        if tab.doc.history_goto(position) {
            (tab.layer, tab.frame) = tab.doc.clamp_selection(tab.layer, tab.frame);
            tab.changed();
        }
    }
    if !open {
        app.menus.history_open = false;
    }
}
