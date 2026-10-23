//! The Tileset inspector section (`TilemapPanel`) with the Terrain controls inside it (`AutotilePanel`).

use super::controls::{bar_button, caption, caption_medium, system_green, terrain_swatch, title_chip};
use super::{MODE_NAMES, Sheet, TilemapState, open_sheet};
use crate::app::DocTab;
use crate::doc::Doc;
use crate::theme::{Colors, font, metric, radius, space, ui_font};
use crate::ui::widgets::{
    self, draw_checker, ds_icon_button, ds_menu_button, ds_popup, ds_text_field, inspector_section, rgba_color, strong,
};
use eframe::egui::{
    self, Align, Color32, Layout, Rect, Sense, Stroke, StrokeKind, TextureId, Ui, UiBuilder, pos2, vec2,
};
use egui_phosphor::regular as icon;

/// What the tilemap menu can ask for (`TilemapMenuItems`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    NewLayer,
    Convert,
    ToNormal,
    Rename,
    /// Switch the layer to another tileset of the same tile size.
    UseTileset(u32),
}

/// What the menu needs to know about the current layer, read before the menu opens.
#[derive(Clone, Debug)]
pub struct MenuInfo {
    pub is_tilemap: bool,
    pub is_group: bool,
    /// `(tileset index, name)` of the tilesets the layer could switch to.
    pub tilesets: Vec<(u32, String)>,
}

impl MenuInfo {
    pub fn of(doc: &Doc, layer: u32) -> MenuInfo {
        MenuInfo {
            is_tilemap: doc.layer_is_tilemap(layer),
            is_group: doc.layer_is_group(layer),
            tilesets: doc.compatible_tilesets(layer).into_iter().map(|t| (t, doc.tileset_title(t))).collect(),
        }
    }
}

/// The items of the tilemap menu: New, Convert, Convert back, and for tilemap layers Rename and Use Tileset. Call
/// it inside a menu; the action is applied with [`apply_menu_action`].
pub fn menu_items(ui: &mut Ui, info: &MenuInfo) -> Option<MenuAction> {
    let mut action = None;
    if ui.button("New Tilemap Layer\u{2026}").clicked() {
        action = Some(MenuAction::NewLayer);
    }
    if ui
        .add_enabled(!(info.is_tilemap || info.is_group), egui::Button::new("Convert Layer to Tilemap\u{2026}"))
        .clicked()
    {
        action = Some(MenuAction::Convert);
    }
    if ui.add_enabled(info.is_tilemap, egui::Button::new("Convert Tilemap to Normal Layer")).clicked() {
        action = Some(MenuAction::ToNormal);
    }
    if info.is_tilemap {
        ui.separator();
        if ui.button("Rename Tileset\u{2026}").clicked() {
            action = Some(MenuAction::Rename);
        }
        ui.add_enabled_ui(!info.tilesets.is_empty(), |ui| {
            ui.menu_button("Use Tileset", |ui| {
                for (i, name) in &info.tilesets {
                    if ui.button(name).clicked() {
                        action = Some(MenuAction::UseTileset(*i));
                    }
                }
            });
        });
    }
    if action.is_some() {
        ui.close();
    }
    action
}

/// Carries out a menu item. Returns true when the document changed.
pub fn apply_menu_action(tab: &mut DocTab, tm: &mut TilemapState, action: MenuAction) -> bool {
    let layer = tab.layer;
    match action {
        MenuAction::NewLayer => open_sheet(tm, Sheet::NewLayer),
        MenuAction::Convert => open_sheet(tm, Sheet::Convert),
        MenuAction::Rename => {
            open_sheet(tm, Sheet::Rename);
            tm.sheet_name = tab.doc.tileset_of_layer(layer).map(|t| tab.doc.tileset_title(t)).unwrap_or_default();
        }
        MenuAction::ToNormal => {
            if tab.doc.convert_layer_to_normal(layer) {
                tm.stamp = false;
                return true;
            }
        }
        MenuAction::UseTileset(ts) => {
            if tab.doc.change_tileset(layer, ts) {
                let count = tab.doc.tileset_of_layer(layer).map_or(0, |t| tab.doc.tile_count(t));
                tm.selected = tm.selected.min(count.saturating_sub(1).max(1));
                return true;
            }
        }
    }
    false
}

/// A tile cell's size from the tile size (`max(24, min(44, max(tw, th) * 3))`).
fn cell_size(tw: u32, th: u32) -> f32 {
    (tw.max(th) as f32 * 3.0).clamp(24.0, 44.0).max(24.0)
}

fn is_dark(ctx: &egui::Context) -> bool {
    ctx.theme() == egui::Theme::Dark
}

/// Draws `f` with its top-left corner at `rect.min`, without moving the parent's cursor.
fn child_at<R>(ui: &mut Ui, rect: Rect, f: impl FnOnce(&mut Ui) -> R) -> R {
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)));
    f(&mut child)
}

/// The Tileset section; shown only while a tilemap layer is selected. Returns true when the document changed.
pub fn tileset_section(ui: &mut Ui, tab: &mut DocTab, tm: &mut TilemapState, error: &mut Option<String>) -> bool {
    let layer = tab.layer;
    let Some(ts) = tab.doc.tileset_of_layer(layer) else { return false };
    let (tw, th) = tab.doc.tile_size(ts);
    let count = tab.doc.tile_count(ts);
    let subtitle = format!("{tw}\u{d7}{th} \u{b7} {count} tiles");
    let info = MenuInfo::of(&tab.doc, layer);
    let mut menu: Option<MenuAction> = None;
    let mut add_tile = false;
    let mut changed = false;
    inspector_section(
        ui,
        "tileset",
        "Tileset",
        Some(&subtitle),
        |ui| {
            ds_menu_button(ui, icon::DOTS_THREE, "Tilemap layers", |ui| {
                menu = menu_items(ui, &info);
            });
            if ds_icon_button(ui, icon::PLUS, 24.0, 13.0, false, false).on_hover_text("Add empty tile").clicked() {
                add_tile = true;
            }
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = space::MD;
            changed |= tileset_body(ui, tab, tm, ts, (tw, th));
            changed |= autotile_panel(ui, tab, tm, error);
        },
    );
    if add_tile {
        if let Some(i) = tab.doc.add_tile(ts) {
            tm.selected = i;
        }
        changed = true;
    }
    if let Some(a) = menu {
        changed |= apply_menu_action(tab, tm, a);
    }
    changed
}

/// A tile image as an egui texture, re-uploaded when the document changed.
fn tile_texture(
    ctx: &egui::Context,
    tm: &mut TilemapState,
    doc: &Doc,
    revision: u64,
    ts: u32,
    tile: u32,
    size: (u32, u32),
) -> Option<TextureId> {
    let key = (doc.raw() as usize, ts, tile);
    if let Some((rev, tex)) = tm.textures.get(&key)
        && *rev == revision
    {
        return Some(tex.id());
    }
    let pixels = doc.tile_rgba(ts, tile)?;
    let image = egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
    if tm.textures.len() > 4096 {
        tm.textures.clear();
    }
    match tm.textures.get_mut(&key) {
        Some((rev, tex)) => {
            tex.set(image, egui::TextureOptions::NEAREST);
            *rev = revision;
            Some(tex.id())
        }
        None => {
            let tex = ctx.load_texture(format!("tile-{ts}-{tile}"), image, egui::TextureOptions::NEAREST);
            let id = tex.id();
            tm.textures.insert(key, (revision, tex));
            Some(id)
        }
    }
}

/// A small label on a dark or green plate, as the corner badges of a tile cell. `None` draws the empty-set sign
/// of tile 0, which not every font has.
fn corner_badge(
    painter: &egui::Painter,
    ctx: &egui::Context,
    anchor: egui::Align2,
    at: egui::Pos2,
    text: Option<&str>,
    fill: Color32,
) {
    let g = text.map(|t| painter.layout_no_wrap(t.to_owned(), strong(ctx, 8.0), Color32::WHITE));
    let size = g.as_ref().map_or(vec2(10.0, 10.0), |g| vec2(g.size().x + 4.0, g.size().y));
    let rect = anchor.anchor_size(at, size);
    painter.rect_filled(rect, 2.0, fill);
    match g {
        Some(g) => {
            painter.galley(rect.min + vec2(2.0, 0.0), g, Color32::WHITE);
        }
        None => {
            let stroke = Stroke::new(1.0, Color32::WHITE);
            painter.circle_stroke(rect.center(), 2.6, stroke);
            painter.line_segment([rect.center() + vec2(-3.0, 3.0), rect.center() + vec2(3.0, -3.0)], stroke);
        }
    }
}

/// The tile grid, the Stamp chip and the flip toggles (`tilesetBody`).
fn tileset_body(ui: &mut Ui, tab: &mut DocTab, tm: &mut TilemapState, ts: u32, tile: (u32, u32)) -> bool {
    let c = Colors::of(ui.ctx());
    let layer = tab.layer;
    let count = tab.doc.tile_count(ts);
    let base = tab.doc.tile_base(ts);
    let cell = cell_size(tile.0, tile.1);
    let masks = tm.show_masks && tab.doc.terrain_mode(layer) >= 0;
    let mut changed = false;
    let mut duplicate = None;
    let mut delete = None;
    ui.spacing_mut().item_spacing.y = space::SM;

    let avail = ui.available_width();
    let inner = avail - 4.0;
    let n = (((inner + 3.0) / (cell + 3.0)).floor() as usize).max(1);
    let col_w = (inner - 3.0 * (n - 1) as f32) / n as f32;
    let rows = (count as usize).div_ceil(n);
    let height = rows as f32 * cell + rows.saturating_sub(1) as f32 * 3.0 + 4.0;
    egui::ScrollArea::vertical().max_height(132.0).auto_shrink([false, true]).show(ui, |ui| {
        let (area, _) = ui.allocate_exact_size(vec2(avail, height), Sense::hover());
        for i in 0..count {
            let (row, col) = (i as usize / n, i as usize % n);
            let rect = Rect::from_min_size(
                pos2(
                    area.left() + 2.0 + col as f32 * (col_w + 3.0) + (col_w - cell) / 2.0,
                    area.top() + 2.0 + row as f32 * (cell + 3.0),
                ),
                vec2(cell, cell),
            );
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let response = ui.interact(rect, ui.id().with(("tile", i)), Sense::click());
            if response.clicked() {
                tm.selected = i;
            }
            let painter = ui.painter();
            draw_checker(painter, rect, 4.0, 3.0);
            let (rev, ctx) = (tab.revision, ui.ctx().clone());
            if let Some(tex) = tile_texture(&ctx, tm, &tab.doc, rev, ts, i, tile) {
                let scale = (cell / tile.0 as f32).min(cell / tile.1 as f32);
                let img = Rect::from_center_size(rect.center(), vec2(tile.0 as f32, tile.1 as f32) * scale);
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                ui.painter().image(tex, img, uv, Color32::WHITE);
            }
            let painter = ui.painter();
            let label = (i != 0).then(|| (base + i as i32 - 1).to_string());
            corner_badge(
                painter,
                ui.ctx(),
                egui::Align2::RIGHT_BOTTOM,
                rect.right_bottom() - vec2(1.0, 1.0),
                label.as_deref(),
                Color32::from_black_alpha(140),
            );
            painter.rect_stroke(rect, 3.0, Stroke::new(1.0, Color32::from_white_alpha(18)), StrokeKind::Inside);
            if i == tm.selected {
                painter.rect_stroke(rect.expand(1.5), 4.0, Stroke::new(2.0, c.accent), StrokeKind::Middle);
            }
            if masks {
                let mask = tab.doc.tile_mask(layer, i);
                if mask >= 0 {
                    corner_badge(
                        ui.painter(),
                        ui.ctx(),
                        egui::Align2::LEFT_TOP,
                        rect.left_top() + vec2(1.0, 1.0),
                        Some(&mask.to_string()),
                        system_green(is_dark(ui.ctx())).gamma_multiply(0.75),
                    );
                }
            }
            response.context_menu(|ui| {
                if ui.button("Duplicate Tile").clicked() {
                    duplicate = Some(i);
                    ui.close();
                }
                if ui.add_enabled(i != 0, egui::Button::new("Delete Tile")).clicked() {
                    delete = Some(i);
                    ui.close();
                }
            });
        }
    });
    if let Some(i) = duplicate {
        if let Some(n) = tab.doc.duplicate_tile(ts, i) {
            tm.selected = n;
        }
        changed = true;
    }
    if let Some(i) = delete
        && tab.doc.delete_tile(ts, i)
    {
        let total = tab.doc.tile_count(ts);
        if tm.selected >= total {
            tm.selected = total.saturating_sub(1).max(1);
        }
        changed = true;
    }

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::XS;
        let on = tm.stamp;
        if title_chip(
            ui,
            "Stamp",
            Some(icon::STAMP),
            on,
            "Place the selected tile on click. Shift-click erases, Option-click picks.",
        )
        .clicked()
        {
            tm.set_stamp(!on);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = space::XS;
            if title_chip(ui, "D", None, tm.flip_d, "Flip diagonally (square tiles)").clicked() {
                tm.flip_d = !tm.flip_d;
            }
            if title_chip(ui, "Y", None, tm.flip_y, "Flip vertically").clicked() {
                tm.flip_y = !tm.flip_y;
            }
            if title_chip(ui, "X", None, tm.flip_x, "Flip horizontally").clicked() {
                tm.flip_x = !tm.flip_x;
            }
        });
    });
    changed
}

// MARK: Terrain

/// The terrain list, authoring helpers, the 3x3 mask editor and the missing-tile report (`AutotilePanel`).
fn autotile_panel(ui: &mut Ui, tab: &mut DocTab, tm: &mut TilemapState, _error: &mut Option<String>) -> bool {
    let c = Colors::of(ui.ctx());
    let layer = tab.layer;
    let mode = tab.doc.terrain_mode(layer);
    let mut changed = false;
    tm.refresh_cache(tab);

    ui.add_space(space::XS);
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(line, 0.0, c.separator_soft);

    // Header.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::XS;
        ui.label(egui::RichText::new("Terrain").font(strong(ui.ctx(), font::CAPTION)).color(c.text));
        caption(ui, "auto-tiling", c.text_tertiary);
        if mode >= 0 {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let on = tm.brush;
                if title_chip(
                    ui,
                    "Brush",
                    Some(icon::PAINT_BRUSH),
                    on,
                    "Paint terrain on this layer; tiles are chosen automatically. Options appear in the bar above the canvas.",
                )
                .clicked()
                {
                    tm.brush = !on;
                }
            });
        }
    });

    if mode < 0 {
        changed |= create_terrain(ui, tab, tm);
        return changed;
    }
    // Mode picker.
    if let Some(m) = ds_popup(ui, "terrain_mode", mode as usize, &MODE_NAMES, None, true, "How tiles connect") {
        changed |= tab.doc.set_terrain_mode(layer, m as i32);
    }
    changed |= terrain_list(ui, tab, tm);
    changed |= mask_editor(ui, tab, tm, mode);
    changed |= authoring(ui, tab);
    changed |= options(ui, tab, tm);
    missing(ui, tm);
    changed
}

/// Buttons of equal width filling the row; returns the index clicked.
fn equal_buttons(ui: &mut Ui, labels: &[(&str, &str)]) -> Option<usize> {
    let gap = space::SM - 2.0;
    let w = (ui.available_width() - gap * (labels.len() - 1) as f32) / labels.len() as f32;
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (i, (label, help)) in labels.iter().enumerate() {
            let enabled = ui.is_enabled();
            let r = ui
                .allocate_ui(vec2(w, metric::CONTROL), |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    bar_button(ui, label, None, true, false)
                })
                .inner;
            if enabled && r.on_hover_text(*help).clicked() {
                clicked = Some(i);
            }
        }
    });
    clicked
}

fn create_terrain(ui: &mut Ui, tab: &mut DocTab, tm: &mut TilemapState) -> bool {
    let c = Colors::of(ui.ctx());
    let layer = tab.layer;
    let mut changed = false;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = space::SM;
        ui.label(
            egui::RichText::new(
                "Name a terrain and pick how its tiles connect. Painting terrain then picks the right tile for you.",
            )
            .font(ui_font(font::CAPTION))
            .color(c.text_secondary),
        );
        ds_text_field(ui, "terrain_new_name", &mut tm.new_name, None);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::SM - 2.0;
            let text_w =
                ui.painter().layout_no_wrap("Create".into(), strong(ui.ctx(), font::CAPTION), Color32::WHITE).size().x;
            let button_w = text_w + 2.0 * (space::SM + 2.0);
            let popup_w = ui.available_width() - button_w - (space::SM - 2.0);
            if let Some(m) =
                ds_popup(ui, "terrain_new_mode", tm.new_mode, &MODE_NAMES, Some(popup_w), false, "How tiles connect")
            {
                tm.new_mode = m;
            }
            if bar_button(ui, "Create", None, false, true).clicked() {
                changed |= tab.doc.create_terrain(layer, &tm.new_name.clone(), tm.new_mode as i32);
            }
        });
    });
    changed
}

fn terrain_list(ui: &mut Ui, tab: &mut DocTab, tm: &mut TilemapState) -> bool {
    let c = Colors::of(ui.ctx());
    let layer = tab.layer;
    let ids = tab.doc.terrain_ids(layer);
    let mut changed = false;
    let mut action: Option<(u32, i64, bool)> = None; // (id, move to, remove)
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = space::XS;
        ui.horizontal(|ui| {
            caption_medium(ui, "Terrains", c.text_tertiary);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                caption(ui, "top = highest priority", c.text_tertiary);
            });
        });
        ui.spacing_mut().item_spacing.y = 2.0;
        let count = ids.len();
        for (index, &id) in ids.iter().enumerate() {
            let selected = tm.terrain == id;
            let (rect0, _) = ui.allocate_exact_size(vec2(ui.available_width(), metric::CONTROL + 4.0), Sense::hover());
            let rect = rect0.expand2(vec2(space::SM, 0.0));
            let body = ui.interact(rect, ui.id().with(("terrain_row", id)), Sense::click());
            if body.clicked() {
                tm.terrain = id;
            }
            if selected {
                ui.painter().rect_filled(rect, radius::CONTROL, c.selected);
            }
            let swatch = Rect::from_center_size(pos2(rect.left() + space::SM + 7.0, rect.center().y), vec2(14.0, 14.0));
            terrain_swatch(ui.painter(), swatch, tab.doc.terrain_color(layer, id));
            ui.painter().text(
                pos2(swatch.right() + space::SM, rect.center().y),
                egui::Align2::LEFT_CENTER,
                tab.doc.terrain_name(layer, id),
                ui_font(font::BODY),
                if selected { c.text } else { c.text_secondary },
            );
            // Buttons, right to left: remove, lower priority, higher priority.
            let button = |slot: f32| {
                Rect::from_min_size(
                    pos2(rect.right() - space::XS - 22.0 - slot * 30.0, rect.center().y - 11.0),
                    vec2(22.0, 22.0),
                )
            };
            child_at(ui, button(0.0), |ui| {
                ui.add_enabled_ui(count > 1, |ui| {
                    if ds_icon_button(ui, icon::MINUS, 22.0, 10.0, false, true)
                        .on_hover_text("Remove terrain")
                        .clicked()
                    {
                        action = Some((id, 0, true));
                    }
                });
            });
            child_at(ui, button(1.0), |ui| {
                ui.add_enabled_ui(index + 1 < count, |ui| {
                    if ds_icon_button(ui, icon::ARROW_DOWN, 22.0, 10.0, false, false)
                        .on_hover_text("Lower priority")
                        .clicked()
                    {
                        action = Some((id, index as i64 + 1, false));
                    }
                });
            });
            child_at(ui, button(2.0), |ui| {
                ui.add_enabled_ui(index > 0, |ui| {
                    if ds_icon_button(ui, icon::ARROW_UP, 22.0, 10.0, false, false)
                        .on_hover_text("Higher priority")
                        .clicked()
                    {
                        action = Some((id, index as i64 - 1, false));
                    }
                });
            });
        }
        ui.spacing_mut().item_spacing.y = space::XS;
        if bar_button(ui, "Add terrain", Some(icon::PLUS), true, false).clicked() {
            if let Some(id) = tab.doc.add_terrain(layer) {
                tm.terrain = id;
            }
            changed = true;
        }
    });
    if let Some((id, to, remove)) = action {
        if remove {
            if tab.doc.remove_terrain(layer, id) {
                if let Some(&first) = tab.doc.terrain_ids(layer).first() {
                    tm.terrain = first;
                }
                changed = true;
            }
        } else {
            changed |= tab.doc.move_terrain(layer, id, to);
        }
    }
    changed
}

/// Directions of the 3x3 mask grid, row-major; `None` is the center (`AutotileState.ring`).
const RING: [Option<u32>; 9] = [Some(7), Some(0), Some(1), Some(6), None, Some(2), Some(5), Some(4), Some(3)];

fn uses_dir(mode: i32, dir: u32) -> bool {
    match mode {
        0 => dir.is_multiple_of(2),
        2 => dir % 2 == 1,
        _ => true,
    }
}

fn mask_editor(ui: &mut Ui, tab: &mut DocTab, tm: &mut TilemapState, mode: i32) -> bool {
    let c = Colors::of(ui.ctx());
    let layer = tab.layer;
    let tile = tm.selected;
    let mut changed = false;
    if tile == 0 {
        widgets::note(ui, "Select a tile to edit its terrain edges.");
        return false;
    }
    let mask = tab.doc.tile_mask(layer, tile);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = space::SM;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::XS;
            caption_medium(ui, &format!("Tile {tile}"), c.text);
            caption(ui, &if mask < 0 { "no mask".to_string() } else { format!("mask {mask}") }, c.text_tertiary);
        });
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = space::MD;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                for r in 0..3 {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        for col in 0..3 {
                            changed |= peer_button(ui, tab, mode, tile, RING[r * 3 + col]);
                        }
                    });
                }
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = space::SM - 2.0;
                // Weight.
                let doc_id = tab.doc.raw() as usize;
                let key = (doc_id, tile, tab.revision as u32);
                let field_id = egui::Id::new("terrain_weight");
                let focused = ui.memory(|m| m.has_focus(ui.id().with(field_id)));
                if !focused && tm.weight_of != Some(key) {
                    tm.weight_of = Some(key);
                    tm.weight_text = tab.doc.tile_weight(layer, tile).to_string();
                }
                ui.add_enabled_ui(mask >= 0, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = space::SM - 2.0;
                        let (rect, _) = ui.allocate_exact_size(vec2(36.0, metric::CONTROL), Sense::hover());
                        ui.painter().text(
                            rect.left_center(),
                            egui::Align2::LEFT_CENTER,
                            "Weight",
                            ui_font(font::CAPTION),
                            c.text_tertiary,
                        );
                        let r = ds_text_field(ui, field_id, &mut tm.weight_text, Some(56.0));
                        if r.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && let Ok(w) = tm.weight_text.trim().parse::<f32>()
                        {
                            changed |= tab.doc.set_tile_weight(layer, tile, w);
                        }
                    });
                    let sym = tab.doc.tile_sym(layer, tile);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (title, bit, what) in
                            [("X", 1u32, "horizontally"), ("Y", 2, "vertically"), ("D", 4, "diagonally")]
                        {
                            let on = sym & bit != 0;
                            if title_chip(ui, title, None, on, &format!("Reusable flipped {what}")).clicked() {
                                changed |= tab.doc.set_tile_sym(layer, tile, if on { sym & !bit } else { sym | bit });
                            }
                        }
                    });
                    if bar_button(ui, "Clear data", None, false, false).clicked() {
                        changed |= tab.doc.set_tile_mask(layer, tile, -1);
                    }
                });
            });
        });
    });
    changed
}

/// One cell of the 3x3 editor: the center sets the tile's own terrain, edges cycle through terrains.
fn peer_button(ui: &mut Ui, tab: &mut DocTab, mode: i32, tile: u32, dir: Option<u32>) -> bool {
    let c = Colors::of(ui.ctx());
    let layer = tab.layer;
    let usable = dir.map_or(mode != 2, |d| uses_dir(mode, d));
    let value = match dir {
        Some(d) => tab.doc.tile_peer(layer, tile, d).max(0) as u32,
        None => tab.doc.tile_terrain(layer, tile).max(0) as u32,
    };
    let (rect, response) =
        ui.allocate_exact_size(vec2(22.0, 22.0), if usable { Sense::click() } else { Sense::hover() });
    let painter = ui.painter();
    if usable {
        let fill = if value == 0 { c.field } else { rgba_color(tab.doc.terrain_color(layer, value) | 0xFF) };
        painter.rect_filled(rect, 4.0, fill);
    }
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, if usable { c.separator } else { c.separator_soft }),
        StrokeKind::Inside,
    );
    if dir.is_none() && usable {
        painter.circle_filled(rect.center(), 2.0, Color32::WHITE.gamma_multiply(0.85));
    }
    let response = response.on_hover_text(if dir.is_none() {
        "Terrain of this tile. Click to cycle."
    } else {
        "Terrain on this side. Click to cycle."
    });
    if usable && response.clicked() {
        let mut ids = vec![0];
        ids.extend(tab.doc.terrain_ids(layer));
        let next = ids[(ids.iter().position(|&i| i == value).unwrap_or(0) + 1) % ids.len()];
        return match dir {
            Some(d) => tab.doc.set_tile_peer(layer, tile, d, next),
            None => tab.doc.set_tile_terrain(layer, tile, next),
        };
    }
    false
}

fn authoring(ui: &mut Ui, tab: &mut DocTab) -> bool {
    let c = Colors::of(ui.ctx());
    let layer = tab.layer;
    let mut changed = false;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = space::XS + 2.0;
        caption_medium(ui, "Template", c.text_tertiary);
        match equal_buttons(
            ui,
            &[
                ("Full", "Add a labelled empty tile for every mask"),
                ("Reduced", "Add only the shapes that rotations and flips cannot make"),
                ("Assign", "Give tiles 1\u{2026} masks in template order (47 / 16 / reduced layouts)"),
            ],
        ) {
            Some(0) => changed |= tab.doc.generate_template(layer, false),
            Some(1) => changed |= tab.doc.generate_template(layer, true),
            Some(2) => changed |= tab.doc.assign_template_masks(layer),
            _ => {}
        }
    });
    changed
}

fn options(ui: &mut Ui, tab: &mut DocTab, tm: &mut TilemapState) -> bool {
    let c = Colors::of(ui.ctx());
    let layer = tab.layer;
    let mut changed = false;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = space::SM;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::SM - 2.0;
            let edge = tab.doc.terrain_edge_same(layer);
            if title_chip(ui, "Edges as terrain", None, edge, "Cells beyond the map edge count as this terrain")
                .clicked()
            {
                changed |= tab.doc.set_terrain_edge_same(layer, !edge);
            }
            if title_chip(ui, "Masks", Some(icon::HASH), tm.show_masks, "Show each tile's mask number on the tileset")
                .clicked()
            {
                tm.show_masks = !tm.show_masks;
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            let (rect, _) = ui.allocate_exact_size(vec2(96.0, metric::CONTROL), Sense::hover());
            ui.painter().text(
                rect.left_center(),
                egui::Align2::LEFT_CENTER,
                "Variation seed",
                ui_font(font::BODY),
                c.text_secondary,
            );
            let id = egui::Id::new("terrain_seed");
            let focused = ui.memory(|m| m.has_focus(ui.id().with(id)));
            if !focused {
                tm.seed_text = tab.doc.terrain_seed(layer).to_string();
            }
            let r = ds_text_field(ui, id, &mut tm.seed_text, Some(64.0))
                .on_hover_text("Picks between tiles that share a mask. Press Return to apply.");
            if r.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Ok(s) = tm.seed_text.trim().parse::<u32>()
            {
                changed |= tab.doc.set_terrain_seed(layer, s);
            }
        });
        match equal_buttons(
            ui,
            &[
                ("Rebuild all", "Recompute every tile from the painted terrain"),
                ("Clear manual tiles", "Remove manual tiles of this frame"),
            ],
        ) {
            Some(0) => changed |= tab.doc.rebuild_terrain(layer),
            Some(1) => changed |= tab.doc.clear_overrides(layer, tab.frame),
            _ => {}
        }
    });
    changed
}

fn missing(ui: &mut Ui, tm: &TilemapState) {
    let c = Colors::of(ui.ctx());
    let lines = &tm.cache.report;
    if lines.is_empty() {
        return;
    }
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = space::XS + 2.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            ui.label(egui::RichText::new(icon::WARNING).size(10.0).color(c.destructive));
            caption_medium(ui, "Missing tiles", c.text);
            caption(ui, "red on canvas", c.text_tertiary);
        });
        let bg = ui.painter().add(egui::Shape::Noop);
        let out = egui::ScrollArea::vertical().max_height(84.0).auto_shrink([false, true]).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin::same(space::SM as i8)).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                for l in lines {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(l).font(egui::FontId::monospace(font::CAPTION)).color(c.text_secondary),
                        )
                        .selectable(true),
                    );
                }
            });
        });
        ui.painter().set(bg, egui::epaint::RectShape::filled(out.inner_rect, radius::FIELD, c.recessed));
    });
}
