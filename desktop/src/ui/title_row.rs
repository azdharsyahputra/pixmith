//! The title row: document tabs, zoom control and Export (`TopBar.swift` `TitleRow`, `Documents.swift` `TabBar`).

use crate::app::PixmithApp;
use crate::theme::{Colors, font, metric, radius, space, ui_font};
use crate::ui::dialogs::{ExportKind, ExportSheet, NewSheet, Prompt};
use crate::ui::widgets;
use eframe::egui::{
    self, Align, Align2, Color32, Key, Layout, Modifiers, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2,
};
use egui_phosphor::regular as icon;
use std::sync::Arc;

/// State of this region that outlives a frame: the file dialogs and the bookkeeping behind tab names.
pub struct FilesState {
    /// The New Sprite sheet while it is open.
    pub new_sheet: Option<NewSheet>,
    /// The unsaved-changes question while it is open.
    pub prompt: Option<Prompt>,
    /// The export options sheet while it is open.
    pub export: Option<ExportSheet>,
    /// Size the New Sprite sheet starts with: the last one created (`Prefs.newWidth`).
    pub new_size: (u32, u32),
    /// "Untitled N" numbers, keyed by the address of each file-less tab's document.
    pub untitled: Vec<(usize, u32)>,
    /// The close button was pressed and the dirty tabs are being asked about.
    pub quitting: bool,
    /// Every dirty tab has been settled; the window may close.
    pub quit_approved: bool,
    /// Tabs whose changes the user chose to discard while quitting.
    pub quit_skip: Vec<usize>,
    /// Hands out distinct starting revisions to new tabs, so the canvas never mistakes one document for another.
    pub next_revision: u64,
}

impl Default for FilesState {
    fn default() -> FilesState {
        FilesState {
            new_sheet: None,
            prompt: None,
            export: None,
            new_size: (32, 32),
            untitled: Vec::new(),
            quitting: false,
            quit_approved: false,
            quit_skip: Vec::new(),
            next_revision: 0,
        }
    }
}

/// Zoom presets of the zoom menu (`ZoomControl.presets`).
const PRESETS: [f32; 10] = [1.0, 2.0, 4.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0];
const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 64.0;

/// "⌘" on macOS, "Ctrl+" elsewhere, for tooltips.
fn command() -> &'static str {
    if cfg!(target_os = "macos") { "\u{2318}" } else { "Ctrl+" }
}

/// `text` laid out on one row, cut with an ellipsis when wider than `max_width`.
fn fit_text(ui: &Ui, text: &str, font: egui::FontId, color: Color32, max_width: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap =
        egui::text::TextWrapping { max_width, max_rows: 1, break_anywhere: true, overflow_character: Some('\u{2026}') };
    ui.fonts_mut(|f| f.layout_job(job))
}

/// `%.5g` of the zoom as a percentage: 800, 37.5, 12.5.
pub(crate) fn zoom_label(zoom: f32) -> String {
    let pct = zoom as f64 * 100.0;
    let digits = (4 - pct.abs().log10().floor() as i32).clamp(0, 6) as usize;
    let s = format!("{pct:.digits$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    format!("{s}%")
}

pub fn show(app: &mut PixmithApp, ui: &mut Ui) {
    app.sync_untitled();
    let c = Colors::of(ui.ctx());
    let full = ui.max_rect(); // DSToolbar's hairline along the bottom edge.
    let bar = full.expand2(vec2(space::MD, 0.0));
    ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, c.separator_soft));

    zoom_shortcuts(app, ui.ctx());

    let export_w = space::MD
        + 13.0
        + 6.0
        + ui.painter()
            .layout_no_wrap("Export".into(), crate::ui::widgets::strong(ui.ctx(), font::BODY_LARGE), c.text)
            .size()
            .x
        + space::MD;
    let right_w = 28.0 + 2.0 + 70.0 + 2.0 + 28.0 + space::XS + export_w;
    let right = Rect::from_min_max(pos2(full.right() - right_w, full.top()), full.right_bottom());
    let left = Rect::from_min_max(full.left_top(), pos2(right.left() - space::SM, full.bottom()));

    let mut tabs = TabEvents::default();
    ui.scope_builder(UiBuilder::new().max_rect(left).layout(Layout::left_to_right(Align::Center)), |ui| {
        egui::ScrollArea::horizontal()
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| tab_bar(app, ui, &mut tabs));
            });
    });
    ui.scope_builder(UiBuilder::new().max_rect(right).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = space::XXS;
        zoom_control(app, ui);
        ui.add_space(space::XS - space::XXS);
        export_button(app, ui);
    });

    if let Some(i) = tabs.activate {
        app.active = i;
    }
    if let Some(i) = tabs.close {
        app.request_close(i);
    }
    if tabs.new_tab {
        app.request_new_sprite();
    }
}

#[derive(Default)]
struct TabEvents {
    activate: Option<usize>,
    close: Option<usize>,
    new_tab: bool,
}

/// One tab (`DSTab` / `TabItem`): name, then a dirty dot or, on hover, a close button.
fn tab_bar(app: &PixmithApp, ui: &mut Ui, events: &mut TabEvents) {
    ui.spacing_mut().item_spacing.x = space::XXS;
    let c = Colors::of(ui.ctx());
    let single = app.tabs.len() == 1;
    for i in 0..app.tabs.len() {
        let active = i == app.active;
        let dirty = app.tabs[i].doc.is_dirty();
        let name = app.tab_name(i);
        let name_color = if active { c.text } else { c.text_secondary };
        let font = ui_font(font::BODY_LARGE);
        let (min_w, max_w) = if single { (0.0, 260.0) } else { (90.0, 200.0) };
        // padding: 12 leading, 8 trailing (10 on the single chip), 8 between name and the 16 pt slot.
        let trailing = if single { 10.0 } else { space::SM };
        let slot_w = if single { if dirty { 6.0 } else { 0.0 } } else { 16.0 };
        let gap = if slot_w > 0.0 { space::SM } else { 0.0 };
        let chrome = 12.0 + gap + slot_w + trailing;
        let natural = ui.painter().layout_no_wrap(name.clone(), font.clone(), name_color).size().x;
        let width = (natural + chrome).clamp(min_w, max_w);
        let galley = fit_text(ui, &name, font, name_color, width - chrome);
        let (rect, response) = ui.allocate_exact_size(vec2(width, metric::ICON_BUTTON), Sense::click());
        let hovering = ui.rect_contains_pointer(rect) && ui.is_enabled();

        let fill = if active {
            c.panel2
        } else if hovering {
            c.hover.gamma_multiply(if single { 0.5 } else { 0.6 })
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, radius::CONTROL, fill);
        ui.painter().galley(
            pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0),
            galley.clone(),
            name_color,
        );

        let slot_center = pos2(rect.right() - trailing - slot_w / 2.0, rect.center().y);
        if single {
            if dirty {
                ui.painter().circle_filled(slot_center, 3.0, c.text_secondary);
            }
        } else {
            let slot = Rect::from_center_size(slot_center, vec2(16.0, 16.0));
            if hovering {
                let close = ui.interact(slot, response.id.with("close"), Sense::click());
                let hot = close.hovered();
                ui.painter().text(
                    slot.center(),
                    Align2::CENTER_CENTER,
                    icon::X,
                    egui::FontId::proportional(9.0),
                    if hot { c.text } else { c.text_secondary },
                );
                if close.clicked() {
                    events.close = Some(i);
                }
                close.on_hover_text("Close tab");
            } else if dirty {
                ui.painter().circle_filled(slot.center(), 3.0, c.text_secondary);
            }
        }

        if response.clicked() {
            events.activate = Some(i);
        }
        if response.middle_clicked() {
            events.close = Some(i);
        }
        response.context_menu(|ui| {
            if ui.button("Close Tab").clicked() {
                events.close = Some(i);
                ui.close();
            }
        });
        let tip = app.tabs[i].doc.path.as_ref().map_or_else(|| name.clone(), |p| p.display().to_string());
        response.on_hover_text(tip);
    }
    if !single
        && widgets::icon_button_sized(ui, icon::PLUS, metric::CONTROL, 13.0)
            .on_hover_text(format!("New sprite ({}N)", command()))
            .clicked()
    {
        events.new_tab = true;
    }
}

/// Zoom out, a menu with the current zoom and presets, zoom in (`ZoomControl`).
fn zoom_control(app: &mut PixmithApp, ui: &mut Ui) {
    let c = Colors::of(ui.ctx());
    let zoom = app.tab().zoom;
    if widgets::icon_button_sized(ui, icon::MINUS, metric::ICON_BUTTON, 14.0)
        .on_hover_text(format!("Zoom out ({}-)", command()))
        .clicked()
    {
        app.tab_mut().zoom = (zoom / 2.0).max(1.0);
    }

    let (rect, response) = ui.allocate_exact_size(vec2(70.0, metric::CONTROL), Sense::click());
    let fill = if response.hovered() || response.is_pointer_button_down_on() { c.hover } else { c.field };
    ui.painter().rect_filled(rect, radius::FIELD, fill);
    let galley = ui.painter().layout_no_wrap(zoom_label(zoom), ui_font(font::BODY), c.text);
    let group = galley.size().x + 4.0 + 9.0;
    let left = rect.center().x - group / 2.0;
    ui.painter().galley(pos2(left, rect.center().y - galley.size().y / 2.0), galley.clone(), c.text);
    ui.painter().text(
        pos2(left + galley.size().x + 4.0 + 4.5, rect.center().y),
        Align2::CENTER_CENTER,
        icon::CARET_DOWN,
        egui::FontId::proportional(9.0),
        c.text_tertiary,
    );
    let response = response.on_hover_text("Zoom. Pinch or Command-scroll also zooms.");
    let mut new_zoom = None;
    egui::Popup::menu(&response).width(rect.width()).show(|ui| {
        for p in PRESETS {
            if ui.button(format!("{}%", (p * 100.0) as i32)).clicked() {
                new_zoom = Some(p);
            }
        }
        ui.separator();
        if ui.button("Zoom to Fit").clicked() {
            new_zoom = Some(fit_zoom(ui.ctx(), app.tab().doc.width(), app.tab().doc.height()));
        }
        if ui.button("Actual Size").clicked() {
            new_zoom = Some(1.0);
        }
    });
    if let Some(z) = new_zoom {
        app.tab_mut().zoom = z;
    }

    if widgets::icon_button_sized(ui, icon::PLUS, metric::ICON_BUTTON, 14.0)
        .on_hover_text(format!("Zoom in ({}=)", command()))
        .clicked()
    {
        app.tab_mut().zoom = (zoom * 2.0).min(MAX_ZOOM);
    }
}

/// `ViewSettings.zoomToFit`: the largest zoom that shows the whole sprite with 64 points around it, whole
/// steps once it is 1 or more. The canvas area is what the window leaves beside the rail, inspector, bars
/// and timeline.
fn fit_zoom(ctx: &egui::Context, width: u32, height: u32) -> f32 {
    let screen = ctx.content_rect();
    let avail_w = screen.width() - metric::TOOL_RAIL - metric::INSPECTOR_WIDTH - 128.0;
    let avail_h = screen.height() - 2.0 * metric::BAR_HEIGHT - metric::TIMELINE_HEIGHT - 128.0;
    if avail_w <= 0.0 || avail_h <= 0.0 || width == 0 || height == 0 {
        return 1.0;
    }
    let mut z = (avail_w / width as f32).min(avail_h / height as f32);
    if z >= 1.0 {
        z = z.floor();
    }
    z.clamp(MIN_ZOOM, MAX_ZOOM)
}

/// Ctrl/Cmd and minus or equals zoom the active tab, like the View menu of the macOS app.
fn zoom_shortcuts(app: &mut PixmithApp, ctx: &egui::Context) {
    let (out, zoom_in) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::COMMAND, Key::Minus),
            i.consume_key(Modifiers::COMMAND, Key::Equals)
                | i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Plus),
        )
    });
    let zoom = app.tab().zoom;
    if out {
        app.tab_mut().zoom = (zoom / 2.0).max(1.0);
    } else if zoom_in {
        app.tab_mut().zoom = (zoom * 2.0).min(MAX_ZOOM);
    }
}

/// The Export menu in the primary-button style (`ExportButton`).
fn export_button(app: &mut PixmithApp, ui: &mut Ui) {
    let c = Colors::of(ui.ctx());
    let galley = ui.painter().layout_no_wrap(
        "Export".into(),
        crate::ui::widgets::strong(ui.ctx(), font::BODY_LARGE),
        Color32::WHITE,
    );
    let width = space::MD + 13.0 + 6.0 + galley.size().x + space::MD;
    let (rect, response) = ui.allocate_exact_size(vec2(width, metric::ICON_BUTTON), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let alpha = if response.is_pointer_button_down_on() || open {
        0.75
    } else if response.hovered() {
        0.92
    } else {
        1.0
    };
    ui.painter().rect_filled(rect, radius::CONTROL, c.accent.gamma_multiply(alpha));
    ui.painter().text(
        pos2(rect.left() + space::MD + 6.5, rect.center().y),
        Align2::CENTER_CENTER,
        icon::EXPORT,
        egui::FontId::proportional(13.0),
        Color32::WHITE,
    );
    ui.painter().galley(
        pos2(rect.left() + space::MD + 13.0 + 6.0, rect.center().y - galley.size().y / 2.0),
        galley,
        Color32::WHITE,
    );
    let response = response.on_hover_text("Export the sprite");
    let mut chosen = None;
    egui::Popup::menu(&response).show(|ui| {
        for (kind, label) in [(ExportKind::Png, "PNG\u{2026}"), (ExportKind::Sheet, "Sprite Sheet\u{2026}")] {
            if ui.button(label).clicked() {
                chosen = Some(kind);
            }
        }
        ui.separator();
        for (kind, label) in [
            (ExportKind::Gif, "Animated GIF\u{2026}"),
            (ExportKind::Apng, "Animated PNG\u{2026}"),
            (ExportKind::Sequence, "PNG Sequence\u{2026}"),
        ] {
            if ui.button(label).clicked() {
                chosen = Some(kind);
            }
        }
    });
    if let Some(kind) = chosen {
        app.request_export(kind);
    }
}
