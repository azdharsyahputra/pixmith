//! The slice tool, its canvas overlay and the Slices inspector section (`SlicesView.swift`, `SlicesSection`).

use super::canvas::Pointer;
use super::{SliceDrag, SliceMode, SliceSheet, TilemapState, UserDataSheet};
use crate::app::{DocTab, PixmithApp, Tool};
use crate::calls::SliceInfo;
use crate::theme::{self, Colors, font, metric, radius, space};
use crate::ui::widgets::{ds_icon_button, inspector_section, rgba_color, strong};
use eframe::egui::{self, Color32, Painter, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as icon;

const HANDLES: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/// Position of a resize handle relative to the sprite's top-left, in points.
fn handle_point(info: &SliceInfo, h: (i32, i32), z: f32) -> egui::Vec2 {
    let x = match h.0 {
        -1 => info.x as f32,
        1 => (info.x + info.w) as f32,
        _ => info.x as f32 + info.w as f32 / 2.0,
    };
    let y = match h.1 {
        -1 => info.y as f32,
        1 => (info.y + info.h) as f32,
        _ => info.y as f32 + info.h as f32 / 2.0,
    };
    vec2(x * z, y * z)
}

/// Mouse down of the slice tool (`sliceMouseDown`): grab a handle of the selected slice, else select and move the
/// slice under the pointer, else start creating one.
pub(super) fn press(tm: &mut TilemapState, tab: &mut DocTab, ptr: &Pointer, p: (i32, i32)) {
    tab.playing = false;
    let frame = tab.frame;
    let mut drag = SliceDrag { mode: SliceMode::Create, start: p, original: (0, 0, 0, 0), began: false };
    if let Some(sel) = tm.selected_slice(tab)
        && let Some(info) = tab.doc.slice_info(sel, frame)
        && let Some(pos) = ptr.pos
    {
        let pt = pos - ptr.srect.min;
        let reach = 7f32.min(info.w.min(info.h) as f32 * ptr.zoom / 3.0);
        if let Some(h) = HANDLES.iter().find(|h| (handle_point(&info, **h, ptr.zoom) - pt).length() <= reach) {
            drag.mode = SliceMode::Resize(h.0, h.1);
            drag.original = (info.x, info.y, info.w, info.h);
            tm.slice_drag = Some(drag);
            return;
        }
    }
    let hit = (0..tab.doc.slice_count()).rev().find(|&i| {
        tab.doc.slice_info(i, frame).is_some_and(|s| p.0 >= s.x && p.0 < s.x + s.w && p.1 >= s.y && p.1 < s.y + s.h)
    });
    if let Some(hit) = hit
        && let Some(info) = tab.doc.slice_info(hit, frame)
    {
        tm.slice_selected = Some(hit);
        drag.mode = SliceMode::Move;
        drag.original = (info.x, info.y, info.w, info.h);
    } else {
        tm.slice_selected = None;
    }
    tm.slice_drag = Some(drag);
}

/// Mouse drag of the slice tool (`sliceMouseDragged`).
pub(super) fn drag(tm: &mut TilemapState, tab: &mut DocTab, p: (i32, i32)) {
    let Some(mut s) = tm.slice_drag else { return };
    if p == s.start && !s.began {
        return;
    }
    let (frame, o) = (tab.frame, s.original);
    match s.mode {
        SliceMode::Create => {
            let (x0, x1) = (s.start.0.min(p.0), s.start.0.max(p.0));
            let (y0, y1) = (s.start.1.min(p.1), s.start.1.max(p.1));
            let (w, h) = ((x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32);
            if !s.began {
                tm.slice_selected = tab.doc.add_slice(x0, y0, w, h);
            } else if let Some(i) = tm.slice_selected {
                tab.doc.set_slice_bounds(i, frame, x0, y0, w, h);
            }
        }
        SliceMode::Move => {
            let Some(i) = tm.slice_selected else { return };
            if !s.began {
                tab.doc.begin_edit("Move Slice");
            }
            tab.doc.set_slice_bounds(i, frame, o.0 + p.0 - s.start.0, o.1 + p.1 - s.start.1, o.2 as u32, o.3 as u32);
        }
        SliceMode::Resize(hx, hy) => {
            let Some(i) = tm.slice_selected else { return };
            if !s.began {
                tab.doc.begin_edit("Resize Slice");
            }
            let (mut x0, mut x1, mut y0, mut y1) = (o.0, o.0 + o.2 - 1, o.1, o.1 + o.3 - 1);
            match hx {
                -1 => x0 = p.0,
                1 => x1 = p.0,
                _ => {}
            }
            match hy {
                -1 => y0 = p.1,
                1 => y1 = p.1,
                _ => {}
            }
            tab.doc.set_slice_bounds(
                i,
                frame,
                x0.min(x1),
                y0.min(y1),
                ((x1 - x0).abs() + 1) as u32,
                ((y1 - y0).abs() + 1) as u32,
            );
        }
    }
    s.began = true;
    tm.slice_drag = Some(s);
    tab.doc.touch();
    tab.changed();
}

/// Mouse up (`sliceMouseUp`). A double-click on a slice opens its properties; the Swift app does that on the
/// second mouse down.
pub(super) fn release(tm: &mut TilemapState, tab: &mut DocTab, double_click: bool) {
    let s = tm.slice_drag.take();
    if double_click
        && let Some(s) = s
        && matches!(s.mode, SliceMode::Move)
        && !s.began
        && let Some(i) = tm.selected_slice(tab)
    {
        open_slice_sheet_for(tm, tab, i);
    }
}

/// Deletes the selected slice. Returns true when the document changed.
pub(super) fn delete_selected(tm: &mut TilemapState, tab: &mut DocTab) -> bool {
    let Some(i) = tm.selected_slice(tab) else { return false };
    if tab.doc.remove_slice(i) {
        tm.slice_selected = None;
        return true;
    }
    false
}

// MARK: Sheets

pub(super) fn open_slice_sheet(app: &mut PixmithApp, i: u32) {
    let PixmithApp { tabs, active, tilemap, .. } = app;
    open_slice_sheet_for(tilemap, &tabs[*active], i);
}

/// Loads the slice's values into the Slice Properties sheet (`SliceSheet.load`).
pub(super) fn open_slice_sheet_for(tm: &mut TilemapState, tab: &DocTab, i: u32) {
    let doc = &tab.doc;
    if i >= doc.slice_count() {
        return;
    }
    let info = doc.slice_info(i, tab.frame);
    let color = doc.user_data_color(4, i, 0);
    let mut sheet = SliceSheet {
        index: i,
        name: doc.slice_name(i),
        nine: false,
        center: [0.0, 0.0, 1.0, 1.0],
        pivot_on: false,
        pivot: [0.0, 0.0],
        text: doc.user_data_text(4, i, 0),
        has_color: color.is_some(),
        color: rgba_color(color.unwrap_or(0x0000FFFF)),
    };
    if let Some(s) = info {
        sheet.nine = s.center.is_some();
        let c = s.center.unwrap_or((0, 0, s.w.max(1), s.h.max(1)));
        sheet.center = [c.0 as f64, c.1 as f64, c.2 as f64, c.3 as f64];
        sheet.pivot_on = s.pivot.is_some();
        let p = s.pivot.unwrap_or((0, 0));
        sheet.pivot = [p.0 as f64, p.1 as f64];
    }
    tm.slice_sheet = Some(sheet);
}

/// Loads the target's user data into the User Data sheet (`UserDataSheet.load`).
pub(super) fn open_user_sheet(app: &mut PixmithApp, kind: u32) {
    let PixmithApp { tabs, active, tilemap, .. } = app;
    let tab = &tabs[*active];
    if kind == 3 && tab.doc.tag_count() == 0 {
        return;
    }
    let mut sheet = UserDataSheet { kind, tag: 0, text: String::new(), has_color: false, color: Color32::BLACK };
    load_user_sheet(&mut sheet, tab);
    tilemap.user_sheet = Some(sheet);
}

/// The `(a, b)` of the user-data target for the sheet's kind.
pub(super) fn user_target(sheet: &UserDataSheet, tab: &DocTab) -> (u32, u32) {
    match sheet.kind {
        1 => (tab.layer, 0),
        2 => (tab.frame, tab.layer),
        3 => (sheet.tag as u32, 0),
        _ => (0, 0),
    }
}

pub(super) fn load_user_sheet(sheet: &mut UserDataSheet, tab: &DocTab) {
    let (a, b) = user_target(sheet, tab);
    sheet.text = tab.doc.user_data_text(sheet.kind, a, b);
    let c = tab.doc.user_data_color(sheet.kind, a, b);
    sheet.has_color = c.is_some();
    sheet.color = rgba_color(c.unwrap_or(0x000000FF));
}

// MARK: Canvas overlay

fn white(a: f32) -> Color32 {
    Color32::WHITE.gamma_multiply(a)
}

/// Slice outlines with name labels; the selected slice also shows resize handles (`drawSlices`).
pub(super) fn paint(painter: &Painter, tm: &TilemapState, tab: &DocTab, srect: Rect, c: &Colors) {
    let z = tab.zoom;
    let selected_index = tm.selected_slice(tab);
    for i in 0..tab.doc.slice_count() {
        let Some(info) = tab.doc.slice_info(i, tab.frame) else { continue };
        let selected = selected_index == Some(i);
        let color = tab.doc.user_data_color(4, i, 0).map_or(c.accent, |v| rgba_color(v | 0xFF));
        let r = Rect::from_min_size(
            srect.min + vec2(info.x as f32, info.y as f32) * z,
            vec2(info.w as f32, info.h as f32) * z,
        );
        let width = if selected { 2.0 } else { 1.0 };
        painter.rect_stroke(r.expand(1.0), 0.0, Stroke::new(width, white(0.8)), StrokeKind::Middle);
        painter.rect_stroke(r, 0.0, Stroke::new(width, color), StrokeKind::Middle);
        if let Some((cx, cy, cw, ch)) = info.center {
            let inner = Rect::from_min_size(r.min + vec2(cx as f32, cy as f32) * z, vec2(cw as f32, ch as f32) * z);
            let pts =
                [inner.left_top(), inner.right_top(), inner.right_bottom(), inner.left_bottom(), inner.left_top()];
            painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(width, color), 3.0, 3.0));
        }
        if let Some((px, py)) = info.pivot {
            let pt = r.min + vec2(px as f32 + 0.5, py as f32 + 0.5) * z;
            let stroke = Stroke::new(width, color);
            painter.line_segment([pos2(pt.x - 5.0, pt.y), pos2(pt.x + 5.0, pt.y)], stroke);
            painter.line_segment([pos2(pt.x, pt.y - 5.0), pos2(pt.x, pt.y + 5.0)], stroke);
        }
        // Name label above the top-left corner.
        let g =
            painter.layout_no_wrap(format!(" {} ", tab.doc.slice_name(i)), strong(painter.ctx(), 10.0), Color32::WHITE);
        let label = Rect::from_min_size(pos2(r.left(), (r.top() - g.size().y).max(srect.top())), g.size());
        painter.rect_filled(label, 0.0, color.gamma_multiply(0.85));
        painter.galley(label.min, g, Color32::WHITE);
        if selected {
            for h in HANDLES {
                let x = match h.0 {
                    -1 => r.left(),
                    1 => r.right(),
                    _ => r.center().x,
                };
                let y = match h.1 {
                    -1 => r.top(),
                    1 => r.bottom(),
                    _ => r.center().y,
                };
                let b = Rect::from_center_size(pos2(x, y), vec2(6.0, 6.0));
                painter.rect_filled(b, 0.0, Color32::WHITE);
                painter.rect_stroke(b, 0.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Middle);
            }
        }
    }
}

// MARK: Inspector section

/// The sprite's slices; shown only when there are some or the slice tool is active (`SlicesSection`).
/// Returns true when the document changed.
pub fn slices_section(ui: &mut Ui, tab: &mut DocTab, tm: &mut TilemapState, tool: Tool) -> bool {
    let count = tab.doc.slice_count();
    if count == 0 && tool != Tool::Slice {
        return false;
    }
    let c = Colors::of(ui.ctx());
    let selected = tm.selected_slice(tab);
    let frame = tab.frame;
    let mut properties = None;
    let mut props_button = false;
    let mut delete = false;
    let mut pick = None;
    let mut delete_row = None;
    inspector_section(
        ui,
        "slices",
        "Slices",
        Some(&count.to_string()),
        |ui| {
            ui.add_enabled_ui(selected.is_some(), |ui| {
                if ds_icon_button(ui, icon::TRASH, 24.0, 13.0, false, true).on_hover_text("Delete slice").clicked() {
                    delete = true;
                }
                if ds_icon_button(ui, icon::SLIDERS_HORIZONTAL, 24.0, 13.0, false, false)
                    .on_hover_text("Slice properties")
                    .clicked()
                {
                    props_button = true;
                }
            });
        },
        |ui| {
            if count == 0 {
                ui.label(
                    egui::RichText::new("Drag on the canvas with the slice tool to create one.")
                        .font(theme::ui_font(font::CAPTION))
                        .color(c.text_tertiary),
                );
                return;
            }
            ui.spacing_mut().item_spacing.y = 2.0;
            for i in 0..count {
                let (rect0, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), metric::CONTROL + 2.0), Sense::hover());
                let rect = rect0.expand2(vec2(space::SM, 0.0));
                let response = ui.interact(rect, ui.id().with(("slice_row", i)), Sense::click());
                let on = selected == Some(i);
                if on {
                    ui.painter().rect_filled(rect, radius::CONTROL, c.selected);
                }
                let p = ui.painter();
                p.text(
                    pos2(rect.left() + space::SM + 6.0, rect.center().y),
                    egui::Align2::CENTER_CENTER,
                    icon::CROP,
                    egui::FontId::proportional(11.0),
                    c.text_tertiary,
                );
                let name_x = rect.left() + space::SM + 12.0 + space::SM;
                if let Some(info) = tab.doc.slice_info(i, frame) {
                    let g = p.layout_no_wrap(
                        format!("{}\u{d7}{}", info.w, info.h),
                        theme::ui_font(font::CAPTION),
                        c.text_tertiary,
                    );
                    let gx = rect.right() - space::SM - g.size().x;
                    p.galley(pos2(gx, rect.center().y - g.size().y / 2.0), g, c.text_tertiary);
                }
                p.with_clip_rect(Rect::from_min_max(
                    pos2(name_x, rect.top()),
                    pos2(rect.right() - 60.0, rect.bottom()),
                ))
                .text(
                    pos2(name_x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    tab.doc.slice_name(i),
                    theme::ui_font(font::BODY),
                    if on { c.text } else { c.text_secondary },
                );
                if response.double_clicked() {
                    pick = Some(i);
                    properties = Some(i);
                } else if response.clicked() {
                    pick = Some(i);
                }
                response.context_menu(|ui| {
                    if ui.button("Properties\u{2026}").clicked() {
                        pick = Some(i);
                        properties = Some(i);
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        pick = Some(i);
                        delete_row = Some(i);
                        ui.close();
                    }
                });
            }
        },
    );
    if let Some(i) = pick {
        tm.slice_selected = Some(i);
    }
    let mut changed = false;
    if delete || delete_row.is_some() {
        changed |= delete_selected(tm, tab);
    }
    if props_button {
        properties = selected;
    }
    if let Some(i) = properties {
        open_slice_sheet_for(tm, tab, i);
    }
    changed
}
