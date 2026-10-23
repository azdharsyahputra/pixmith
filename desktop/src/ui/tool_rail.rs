//! The slim vertical tool rail: icon-only tools grouped by function, then the color chips (`ToolsPanel.swift`).

use crate::app::{PixmithApp, Tool};
use crate::theme::{Colors, metric};
use crate::ui::inspector::color;
use crate::ui::widgets;
use eframe::egui;

pub fn show(app: &mut PixmithApp, ui: &mut egui::Ui) {
    let c = Colors::of(ui.ctx());
    let rect = ui.max_rect();
    ui.painter().vline(rect.right() - 0.5, rect.y_range(), egui::Stroke::new(1.0, c.separator_soft));
    ui.vertical_centered(|ui| {
        ui.add_space(6.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        for (g, group) in Tool::GROUPS.iter().enumerate() {
            if g > 0 {
                widgets::rail_divider(ui);
            }
            for &tool in group.iter() {
                let tip = format!("{} ({})", tool.title(), tool.key());
                if widgets::tool_button(ui, tool.icon(), app.tool == tool, metric::TOOL_BUTTON)
                    .on_hover_text(tip)
                    .clicked()
                {
                    app.tool = tool;
                }
            }
        }
    });
    color::chips(app, ui);
}
