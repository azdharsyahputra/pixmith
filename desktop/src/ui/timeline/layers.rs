//! The layers column: header with the add buttons and the layer menu, and one row per layer
//! (`layersHeader`, `names`, `TimelineLayerRow`, `LayerMenuItems`, `LayerReorder`).

use super::controls::{self, Look, fade};
use super::sheets::LayerSheet;
use super::{LayerDrag, LayerDropZone, Renaming, TimelineState};
use crate::app::DocTab;
use crate::theme::{Colors, font, radius, ui_font};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, Layout, Margin, Rect, Sense, Stroke, StrokeKind, UiBuilder,
    pos2, vec2,
};
use egui_phosphor::regular as icon;

pub const ROW_HEIGHT: f32 = 24.0;

/// Keeps the selection inside the document after a structural change.
fn clamp(tab: &mut DocTab) {
    (tab.layer, tab.frame) = tab.doc.clamp_selection(tab.layer, tab.frame);
}

/// Selects the drawable layer at or above the result of a structural layer edit (`finish`).
fn finish(tab: &mut DocTab, result: Option<u32>) {
    let Some(r) = result else { return };
    if let Some(l) = tab.doc.drawable_from(r) {
        tab.layer = l;
    }
    clamp(tab);
    tab.changed();
}

pub fn add_layer(tab: &mut DocTab) {
    tab.layer = tab.doc.add_layer(tab.layer);
    tab.changed();
}

pub fn add_group(tab: &mut DocTab) {
    if tab.doc.add_group(tab.layer).is_some() {
        tab.changed();
    }
}

pub fn remove_layer(tab: &mut DocTab) {
    if tab.doc.remove_layer(tab.layer) {
        clamp(tab);
        tab.changed();
    }
}

/// The entries of the layer menu for layer `index` (`LayerMenuItems`).
fn menu_items(ui: &mut egui::Ui, tab: &mut DocTab, st: &mut TimelineState, index: u32) {
    let item =
        |ui: &mut egui::Ui, label: &str, enabled: bool| ui.add_enabled(enabled, egui::Button::new(label)).clicked();
    if item(ui, "Properties\u{2026}", true) {
        st.layer_sheet = Some(LayerSheet::new(tab, index));
        ui.close();
    }
    if item(ui, "Move Up", true) {
        let r = tab.doc.move_layer(index, true);
        finish(tab, r);
        ui.close();
    }
    if item(ui, "Move Down", true) {
        let r = tab.doc.move_layer(index, false);
        finish(tab, r);
        ui.close();
    }
    if item(ui, "Duplicate", true) {
        let r = tab.doc.duplicate_layer(index);
        finish(tab, r);
        ui.close();
    }
    if item(ui, "Merge Down", tab.doc.can_merge_down(index)) {
        let r = tab.doc.merge_layer_down(index);
        finish(tab, r);
        ui.close();
    }
    ui.separator();
    if item(ui, "New Group", true) {
        tab.layer = index;
        add_group(tab);
        ui.close();
    }
    if item(ui, "Group Layer", true) {
        let r = tab.doc.wrap_in_group(index);
        finish(tab, r);
        ui.close();
    }
    let targets = tab.doc.group_targets(index);
    ui.add_enabled_ui(!targets.is_empty(), |ui| {
        ui.menu_button("Move Into Group", |ui| {
            for g in &targets {
                if ui.button(tab.doc.layer_name(*g)).clicked() {
                    let r = tab.doc.move_layer_into(index, *g);
                    finish(tab, r);
                    ui.close();
                }
            }
        });
    });
    if item(ui, "Move Out of Group", tab.doc.layer_level(index) > 0) {
        let r = tab.doc.move_out_of_group(index);
        finish(tab, r);
        ui.close();
    }
}

/// The "Layers" caption with the new layer / new group / more buttons, bottom-aligned in `rect`.
pub fn header(ui: &mut egui::Ui, tab: &mut DocTab, st: &mut TimelineState, rect: Rect) {
    let c = Colors::of(ui.ctx());
    let band = Rect::from_min_max(
        pos2(rect.left(), rect.bottom() - 4.0 - 24.0),
        pos2(rect.right() - 8.0, rect.bottom() - 4.0),
    );
    ui.painter().text(
        pos2(rect.left() + 12.0, band.center().y),
        Align2::LEFT_CENTER,
        "Layers",
        crate::ui::widgets::strong(ui.ctx(), font::CAPTION),
        c.text_secondary,
    );
    let mut row = ui.new_child(UiBuilder::new().max_rect(band).layout(Layout::right_to_left(Align::Center)));
    row.spacing_mut().item_spacing.x = 0.0;
    let plain = Look::default();
    let more = controls::icon_button(
        &mut row,
        icon::DOTS_THREE,
        24.0,
        13.0,
        plain,
        "Layer actions: move, duplicate, merge, group, properties",
    );
    egui::Popup::menu(&more).show(|ui| {
        let can_delete = tab.doc.layer_count() > 1;
        if ui.add_enabled(can_delete, egui::Button::new("Delete Layer")).clicked() {
            remove_layer(tab);
            ui.close();
        }
        ui.separator();
        menu_items(ui, tab, st, tab.layer);
    });
    // Left to right order is "+", folder, "...", so lay out the first two from the left edge of the group.
    let mut left = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(band.right() - 72.0, band.top()),
                pos2(band.right() - 24.0, band.bottom()),
            ))
            .layout(Layout::left_to_right(Align::Center)),
    );
    left.spacing_mut().item_spacing.x = 0.0;
    if controls::icon_button(&mut left, icon::PLUS, 24.0, 13.0, plain, "New layer (\u{21e7}\u{2318}N)").clicked() {
        add_layer(tab);
    }
    if controls::icon_button(&mut left, icon::FOLDER_SIMPLE_PLUS, 24.0, 13.0, plain, "New group").clicked() {
        add_group(tab);
    }
}

fn zone_at(tab: &DocTab, layer: u32, y_in_row: f32) -> LayerDropZone {
    if tab.doc.layer_is_group(layer) {
        if y_in_row < ROW_HEIGHT / 4.0 {
            LayerDropZone::Above
        } else if y_in_row > ROW_HEIGHT * 3.0 / 4.0 {
            LayerDropZone::Below
        } else {
            LayerDropZone::Into
        }
    } else if y_in_row < ROW_HEIGHT / 2.0 {
        LayerDropZone::Above
    } else {
        LayerDropZone::Below
    }
}

/// The rows of the layers column inside `body`, scrolled by `scroll_y`.
pub fn rows(ui: &mut egui::Ui, tab: &mut DocTab, st: &mut TimelineState, rows: &[u32], body: Rect, scroll_y: f32) {
    let c = Colors::of(ui.ctx());
    let mut child = ui.new_child(UiBuilder::new().max_rect(body));
    child.set_clip_rect(body.intersect(ui.clip_rect()));
    let ui = &mut child;

    // Where a drag in progress would land, and the drop when it ends.
    if st.layer_drag.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        let pos = ui.input(|i| i.pointer.latest_pos());
        let target = pos.filter(|p| body.contains(*p)).and_then(|p| {
            let y = p.y - body.top() + scroll_y;
            let r = (y / ROW_HEIGHT).floor() as usize;
            rows.get(r).map(|&l| (l, zone_at(tab, l, y - r as f32 * ROW_HEIGHT)))
        });
        let released = ui.input(|i| i.pointer.primary_released() || !i.pointer.primary_down());
        if let Some(d) = st.layer_drag.as_mut() {
            d.target = target;
        }
        if released {
            if let Some(LayerDrag { source, target: Some((t, zone)) }) = st.layer_drag.take()
                && let Some(new) = tab.doc.drop_layer(source, t, zone, tab.layer)
            {
                tab.layer = new;
                clamp(tab);
                tab.changed();
            }
            st.layer_drag = None;
        }
    }

    for (r, &index) in rows.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(body.left(), body.top() + r as f32 * ROW_HEIGHT - scroll_y),
            vec2(body.width(), ROW_HEIGHT),
        );
        if row.bottom() < body.top() || row.top() > body.bottom() {
            continue;
        }
        layer_row(ui, tab, st, &c, index, row);
    }

    // The drop indicator.
    if let Some(LayerDrag { target: Some((t, zone)), .. }) = &st.layer_drag
        && let Some(r) = rows.iter().position(|l| l == t)
    {
        let row = Rect::from_min_size(
            pos2(body.left(), body.top() + r as f32 * ROW_HEIGHT - scroll_y),
            vec2(body.width(), ROW_HEIGHT),
        );
        let p = ui.painter();
        match zone {
            LayerDropZone::Into => {
                p.rect_stroke(row.shrink(1.0), CornerRadius::same(5), Stroke::new(2.0, c.accent), StrokeKind::Inside);
            }
            LayerDropZone::Above => {
                p.rect_filled(Rect::from_min_size(row.min, vec2(row.width(), 2.0)), 0.0, c.accent);
            }
            LayerDropZone::Below => {
                p.rect_filled(Rect::from_min_max(pos2(row.left(), row.bottom() - 2.0), row.max), 0.0, c.accent);
            }
        }
    }
}

/// One row: group disclosure, thumbnail, name and the visibility eye.
fn layer_row(ui: &mut egui::Ui, tab: &mut DocTab, st: &mut TimelineState, c: &Colors, index: u32, row: Rect) {
    let group = tab.doc.layer_is_group(index);
    let visible = tab.doc.layer_visible(index);
    let selected = index == tab.layer;
    let collapsed = st.collapsed_groups.contains(&index);
    let level = tab.doc.layer_level(index) as f32;

    let resp = ui.interact(row, ui.id().with(("layer_row", index)), Sense::click_and_drag());
    let eye_rect = Rect::from_min_size(pos2(row.right() - 4.0 - 24.0, row.top()), vec2(24.0, 24.0));
    let eye = ui.interact(eye_rect, ui.id().with(("layer_eye", index)), Sense::click());
    let hover = resp.hovered() || eye.hovered();

    let bg = if selected {
        c.selected
    } else if hover {
        fade(c.hover, 0.6)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(row, 0.0, bg);

    let mut x = row.left() + 8.0 + level * 14.0;
    let cy = row.center().y;
    if group {
        let chev = Rect::from_min_size(pos2(x, row.top()), vec2(14.0, 24.0));
        let r = ui.interact(chev, ui.id().with(("layer_chevron", index)), Sense::click());
        let glyph = if collapsed { icon::CARET_RIGHT } else { icon::CARET_DOWN };
        ui.painter().text(chev.center(), Align2::CENTER_CENTER, glyph, ui_font(11.0), c.text_tertiary);
        if r.clicked() {
            if collapsed {
                st.collapsed_groups.remove(&index);
            } else {
                st.collapsed_groups.insert(index);
            }
        }
        r.on_hover_text(if collapsed { "Expand group" } else { "Collapse group" });
        x += 14.0 + 8.0;
        ui.painter().text(
            pos2(x + 8.0, cy),
            Align2::CENTER_CENTER,
            icon::FOLDER_SIMPLE,
            ui_font(15.0),
            c.text_secondary,
        );
        x += 16.0 + 8.0;
    } else {
        paint_thumbnail(
            ui.painter(),
            Rect::from_center_size(pos2(x + 8.0, cy), vec2(16.0, 16.0)),
            visible,
            tab.doc.layer_is_tilemap(index),
            c,
        );
        x += 16.0 + 8.0;
    }

    // The name, or the field while renaming.
    let name_rect = Rect::from_min_max(pos2(x, row.top()), pos2(eye_rect.left(), row.bottom()));
    let renaming_here = st.renaming.as_ref().is_some_and(|r| r.layer == index);
    if renaming_here {
        let mut commit = None;
        if let Some(rn) = st.renaming.as_mut() {
            let mut edit_ui = ui.new_child(UiBuilder::new().max_rect(name_rect.shrink2(vec2(0.0, 3.0))));
            let edit = edit_ui.add(
                egui::TextEdit::singleline(&mut rn.text)
                    .font(ui_font(font::BODY))
                    .text_color(c.text)
                    .margin(Margin::symmetric(4, 0))
                    .vertical_align(Align::Center)
                    .frame(egui::Frame::NONE),
            );
            ui.painter().rect_stroke(
                name_rect.shrink2(vec2(0.0, 3.0)),
                CornerRadius::same(radius::CHIP),
                Stroke::new(1.5, c.accent_line),
                StrokeKind::Inside,
            );
            if !rn.focused {
                edit.request_focus();
                rn.focused = true;
            }
            let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if escape {
                commit = Some(None);
            } else if edit.lost_focus() {
                commit = Some(Some(rn.text.trim().to_owned()));
            }
        }
        if let Some(result) = commit
            && let Some(Renaming { layer, .. }) = st.renaming.take()
            && let Some(name) = result.filter(|n| !n.is_empty() && *n != tab.doc.layer_name(layer))
        {
            tab.doc.rename_layer(layer, &name);
            tab.changed();
        }
    } else {
        let name = tab.doc.layer_name(index);
        let (font_id, color) = if selected {
            (crate::ui::widgets::strong(ui.ctx(), font::BODY), c.text)
        } else {
            (ui_font(font::BODY), c.text_secondary)
        };
        let color = if visible { color } else { fade(color, 0.5) };
        ui.painter().with_clip_rect(name_rect.intersect(ui.clip_rect())).text(
            pos2(x, cy),
            Align2::LEFT_CENTER,
            name,
            font_id,
            color,
        );
    }

    // The eye.
    {
        let color = if visible { c.text_tertiary } else { c.text_secondary };
        let glyph = if visible { icon::EYE } else { icon::EYE_SLASH };
        ui.painter().text(eye_rect.center(), Align2::CENTER_CENTER, glyph, ui_font(14.0), color);
        if eye.clicked() {
            tab.doc.set_layer_visible(index, !visible);
            tab.changed();
        }
        eye.on_hover_text(if visible { "Hide layer" } else { "Show layer" });
    }

    if resp.double_clicked() && !eye_rect.contains(resp.interact_pointer_pos().unwrap_or_default()) {
        st.renaming = Some(Renaming { layer: index, text: tab.doc.layer_name(index), focused: false });
    } else if resp.clicked() {
        st.focused = false;
        if !group {
            tab.layer = index;
            st.range = None;
        }
    }
    if resp.drag_started() && st.layer_drag.is_none() {
        st.layer_drag = Some(LayerDrag { source: index, target: None });
    }
    resp.context_menu(|ui| {
        if ui.button("Rename\u{2026}").clicked() {
            st.renaming = Some(Renaming { layer: index, text: tab.doc.layer_name(index), focused: false });
            ui.close();
        }
        if ui.add_enabled(tab.doc.layer_count() > 1, egui::Button::new("Delete")).clicked() {
            tab.layer = index;
            remove_layer(tab);
            ui.close();
        }
        ui.separator();
        menu_items(ui, tab, st, index);
    });
}

/// The 16 pt checkerboard thumbnail with a hairline border; a grid glyph marks tilemap layers.
fn paint_thumbnail(p: &egui::Painter, rect: Rect, visible: bool, tilemap: bool, c: &Colors) {
    let a = if visible { 1.0 } else { 0.4 };
    let light = fade(Color32::from_gray(235), a);
    let dark = fade(Color32::from_gray(184), a);
    p.rect_filled(rect, CornerRadius::same(3), light);
    for r in 0..4 {
        for col in 0..4 {
            if (r + col) % 2 == 1 {
                let cell = Rect::from_min_size(rect.min + vec2(col as f32 * 4.0, r as f32 * 4.0), vec2(4.0, 4.0));
                // Keep the rounded corner clean.
                let inset = if (r == 0 || r == 3) && (col == 0 || col == 3) { 0.6 } else { 0.0 };
                p.rect_filled(cell.shrink(inset), 0.0, dark);
            }
        }
    }
    p.rect_stroke(rect, CornerRadius::same(3), Stroke::new(1.0, fade(Color32::WHITE, 0.08 * a)), StrokeKind::Inside);
    if tilemap {
        p.text(rect.center(), Align2::CENTER_CENTER, icon::SQUARES_FOUR, ui_font(11.0), c.text_secondary);
    }
}
