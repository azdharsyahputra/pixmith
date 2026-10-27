//! The Preview (`PreviewWindow.swift`), floating in a window or docked as a section of the inspector: the sprite at a
//! fixed zoom or fit to the available space, a play / pause button, previous and next frame, and the zoom popup.
//! Paused, it follows the editor's current frame and so shows every stroke as it is drawn; only Play runs its own
//! playback, independent of the editor's play state.

use super::{ToolsState, view::PreviewZoom};
use crate::app::{DocTab, PixmithApp};
use crate::theme::{self, Colors, font, metric, space};
use crate::ui::timeline::playback;
use crate::ui::widgets;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, TextureHandle, Vec2};
use egui_phosphor::regular as icon;
use pixmith_core::document::layers::PlayState;
use std::ops::RangeInclusive;

/// Wide enough for the whole control bar of the window.
const MIN_WIDTH: f32 = 290.0;

/// State of the preview, floating or docked (`PreviewState`).
#[derive(Default)]
pub struct PreviewState {
    /// The window is shown, or the docked section is expanded.
    pub open: bool,
    /// The frame shown while playing.
    pub frame: u32,
    pub playing: bool,
    /// Bumped to give the window a new identity, which makes egui apply the default size again.
    generation: u32,
    pos: Option<Pos2>,
    /// `input.time` when the current frame started showing.
    started: Option<f64>,
    cursor: PlayState,
    /// The document the window was last drawn for, to restart playback when the tab changes.
    doc: usize,
    texture: Option<TextureHandle>,
    texture_key: Option<(usize, u64, u32, u32, u32)>,
    /// The revision and editor frame the canvas last drew, to repaint a docked preview that lags behind the canvas.
    seen: (u64, u32),
    buf: Vec<u8>,
}

impl PreviewState {
    /// Shows the window; the loop only runs while it is visible (`PreviewController.show`).
    pub fn show(&mut self) {
        if !self.open {
            self.open = true;
            self.started = None;
            self.cursor = PlayState::default();
            self.generation += 1;
        }
    }

    /// The frame to draw: the playing frame, or the editor's current one while paused.
    pub fn shown_frame(&self, tab: &DocTab) -> u32 {
        let frame = if self.playing { self.frame } else { tab.frame };
        frame.min(tab.doc.frame_count().saturating_sub(1))
    }

    pub fn close(&mut self) {
        self.open = false;
        self.started = None;
        // A hidden preview does no work per edit.
        self.texture = None;
        self.texture_key = None;
    }
}

/// The tag that contains the editor's current frame, if tag looping is on (`ViewSettings.activeTag`).
fn active_tag(tab: &DocTab, loop_tags: bool) -> Option<crate::ui::timeline::TagInfo> {
    if !loop_tags {
        return None;
    }
    (0..tab.doc.tag_count()).filter_map(|i| tab.doc.tag(i)).find(|t| t.from <= tab.frame && tab.frame <= t.to)
}

/// Frames the preview plays: the active tag, or all of them (`ViewSettings.previewRange`).
pub fn preview_range(tab: &DocTab, loop_tags: bool) -> RangeInclusive<u32> {
    let last = tab.doc.frame_count().saturating_sub(1);
    match active_tag(tab, loop_tags) {
        Some(t) if t.from <= t.to => {
            let (lo, hi) = (t.from.min(last), t.to.min(last));
            if lo <= hi { lo..=hi } else { 0..=last }
        }
        _ => 0..=last,
    }
}

impl PreviewState {
    /// Moves the editor's frame by hand, which the paused preview follows; stepping while playing pauses first
    /// (`step`).
    pub fn step(&mut self, tab: &mut DocTab, delta: i32) {
        let n = tab.doc.frame_count().max(1) as i32;
        let current = self.shown_frame(tab) as i32;
        self.playing = false;
        self.started = None;
        playback::stop(tab);
        tab.frame = (current + delta).rem_euclid(n) as u32;
    }

    /// Play starts at the editor's frame; pausing goes back to following it.
    pub fn toggle_play(&mut self, tab: &DocTab) {
        self.playing = !self.playing;
        self.started = None;
        self.cursor = PlayState::default();
        if self.playing {
            self.frame = tab.frame;
        }
    }

    /// Advances the frame when its duration has passed. Plays inside the active tag honoring its direction, or
    /// through all frames when there is none (`startLoop`). Returns the time of the next change, if playing.
    pub fn advance(&mut self, tab: &DocTab, loop_tags: bool, now: f64) -> Option<f64> {
        let nf = tab.doc.frame_count();
        let r = preview_range(tab, loop_tags);
        if nf == 0 {
            return None;
        }
        if !self.playing {
            self.frame = tab.frame.min(nf - 1);
            self.started = None;
            return None;
        }
        if !r.contains(&self.frame) {
            // Entered a different tag (or left one): restart at the tag's first frame for its direction.
            let backwards = active_tag(tab, loop_tags).is_some_and(|t| t.direction == 1 || t.direction == 3);
            self.frame = if backwards { *r.end() } else { *r.start() };
            self.cursor = PlayState::default();
            self.started = Some(now);
        }
        let started = *self.started.get_or_insert(now);
        let due = started + tab.doc.frame_duration(self.frame).max(10) as f64 / 1000.0;
        if now >= due {
            let next = tab.doc.play_next(self.frame, &mut self.cursor, loop_tags);
            self.frame = if r.contains(&next) { next } else { *r.start() };
            self.started = Some(now);
            return Some(now + tab.doc.frame_duration(self.frame).max(10) as f64 / 1000.0);
        }
        Some(due)
    }
}

/// Size the window needs to show the whole sprite at a fixed zoom (`fitWindow`).
fn fitted_size(tab: &DocTab, zoom: PreviewZoom, screen: Vec2) -> Option<Vec2> {
    let n = zoom.scale()? as f32;
    let controls = 40.0;
    let mut size = Vec2::new(tab.doc.width() as f32 * n, tab.doc.height() as f32 * n + controls);
    size.x = size.x.min(screen.x * 0.9).max(MIN_WIDTH);
    size.y = size.y.min(screen.y * 0.9).max(120.0 + controls);
    Some(size)
}

/// Restarts playback when the tab changed, then advances it and asks for a repaint when the next frame is due.
fn tick(ctx: &egui::Context, st: &mut PreviewState, tab: &DocTab, loop_tags: bool) {
    let doc_id = tab.doc.raw() as usize;
    if st.doc != doc_id {
        st.doc = doc_id;
        st.frame = 0;
        st.started = None;
        st.cursor = PlayState::default();
    }
    let now = ctx.input(|i| i.time);
    if let Some(next) = st.advance(tab, loop_tags, now) {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((next - now).max(0.001)));
    }
}

/// Draws the floating window and runs its playback. Call every frame (after the canvas, so a docked preview, which
/// the inspector draws before it, catches up with the last stroke).
pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    if !app.canvas.tools.preview.open {
        return;
    }
    let loop_tags = app.timeline.loop_tags;
    let PixmithApp { tabs, active, canvas, dark, .. } = app;
    let tab = &mut tabs[*active];
    let tools = &mut canvas.tools;
    if tools.settings.preview_docked {
        let st = &tools.preview;
        if st.seen != (tab.revision, tab.frame) {
            ctx.request_repaint();
        }
        return;
    }
    tick(ctx, &mut tools.preview, tab, loop_tags);

    let c = Colors::of(ctx);
    let screen = ctx.content_rect().size();
    let zoom = tools.settings.preview_zoom;
    let default_size = fitted_size(tab, zoom, screen).unwrap_or(Vec2::new(300.0, 340.0));
    let frame = egui::Frame::new()
        .fill(c.workspace)
        .stroke(egui::Stroke::new(1.0, c.separator))
        .corner_radius(egui::CornerRadius::same(theme::radius::PANEL));
    let mut open = true;
    let mut new_zoom = zoom;
    let mut dock = false;
    let mut window = egui::Window::new("Preview")
        .id(egui::Id::new(("preview-window", tools.preview.generation)))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(default_size)
        .min_size(Vec2::new(MIN_WIDTH, 160.0))
        .frame(frame);
    // Centered on first use, like `NSPanel.center()`; later openings come back where the window was.
    let pos = tools.preview.pos.unwrap_or_else(|| ctx.content_rect().center() - default_size / 2.0);
    window = window.default_pos(pos);
    let shown = window.show(ctx, |ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let avail = ui.available_size();
        let canvas_h = (avail.y - metric::BAR_HEIGHT - 1.0).max(0.0);
        let (area, _) = ui.allocate_exact_size(Vec2::new(avail.x, canvas_h), Sense::hover());
        paint_canvas(ui, &mut tools.preview, tab, zoom, area, *dark);
        // Divider and the 40 pt control bar.
        let (line, _) = ui.allocate_exact_size(Vec2::new(avail.x, 1.0), Sense::hover());
        ui.painter().rect_filled(line, 0.0, c.separator);
        let (bar, _) = ui.allocate_exact_size(Vec2::new(avail.x, metric::BAR_HEIGHT), Sense::hover());
        ui.painter().rect_filled(bar, theme::radius::PANEL as f32 - 1.0, c.panel);
        ui.painter().rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width(), 8.0)), 0.0, c.panel);
        let mut inner = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bar.shrink2(Vec2::new(space::SM, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        controls(&mut inner, &mut tools.preview, tab, &mut new_zoom, &c, Some(&mut dock));
    });
    if let Some(r) = shown {
        tools.preview.pos = Some(r.response.rect.min);
    }
    if new_zoom != zoom {
        tools.settings.preview_zoom = new_zoom;
        // A fixed zoom resizes the window so the whole sprite shows.
        tools.preview.generation += 1;
    }
    if !open {
        tools.preview.close();
    }
    if dock {
        tools.set_preview_docked(true);
    }
}

/// The preview as a collapsible inspector section near the top, shown while docked (`DockedPreviewSection`). Its
/// expanded state is `PreviewState::open`, so View > Preview expands and collapses it.
pub fn docked_section(ui: &mut egui::Ui, tools: &mut ToolsState, tab: &mut DocTab, loop_tags: bool, dark: bool) {
    if !tools.settings.preview_docked {
        return;
    }
    let ctx = ui.ctx().clone();
    let c = Colors::of(&ctx);
    widgets::set_section_collapsed(&ctx, "preview", !tools.preview.open);
    let subtitle = format!("frame {} / {}", tools.preview.shown_frame(tab) + 1, tab.doc.frame_count());
    let zoom = tools.settings.preview_zoom;
    let mut new_zoom = zoom;
    let mut float = false;
    let st = &mut tools.preview;
    widgets::inspector_section(
        ui,
        "preview",
        "Preview",
        Some(&subtitle),
        |ui| {
            if widgets::ds_icon_button(ui, icon::APP_WINDOW, 24.0, 13.0, false, false)
                .on_hover_text("Float the preview in its own window")
                .clicked()
            {
                float = true;
            }
        },
        |ui| {
            tick(&ctx, st, tab, loop_tags);
            ui.spacing_mut().item_spacing.y = space::SM - 2.0;
            let height = match zoom.scale() {
                Some(n) => (tab.doc.height() as f32 * n as f32).clamp(64.0, 240.0),
                None => 176.0,
            };
            let (area, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
            ui.painter().rect_filled(area, theme::radius::FIELD, c.workspace);
            paint_canvas(ui, st, tab, zoom, area, dark);
            let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), metric::ICON_BUTTON), Sense::hover());
            let mut inner = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(bar.expand2(Vec2::new(space::XS, 0.0)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            controls(&mut inner, st, tab, &mut new_zoom, &c, None);
        },
    );
    tools.preview.open = !widgets::section_collapsed(&ctx, "preview");
    if new_zoom != zoom {
        tools.settings.preview_zoom = new_zoom;
    }
    if float {
        tools.set_preview_docked(false);
    }
}

/// Play / pause, previous and next frame, and the zoom popup. The window's bar (`dock` is set) also has the frame
/// count and the button that docks the preview.
fn controls(
    ui: &mut egui::Ui,
    st: &mut PreviewState,
    tab: &mut DocTab,
    zoom: &mut PreviewZoom,
    c: &Colors,
    dock: Option<&mut bool>,
) {
    ui.spacing_mut().item_spacing.x = space::XXS;
    let glyph = if st.playing { icon::PAUSE } else { icon::PLAY };
    let tip = "Play / pause the preview. Paused, it shows the editor's frame.";
    if widgets::ds_icon_button(ui, glyph, metric::ICON_BUTTON, 14.0, st.playing, false).on_hover_text(tip).clicked() {
        st.toggle_play(tab);
    }
    if widgets::ds_icon_button(ui, icon::SKIP_BACK, metric::ICON_BUTTON, 14.0, false, false)
        .on_hover_text("Previous frame")
        .clicked()
    {
        st.step(tab, -1);
    }
    if widgets::ds_icon_button(ui, icon::SKIP_FORWARD, metric::ICON_BUTTON, 14.0, false, false)
        .on_hover_text("Next frame")
        .clicked()
    {
        st.step(tab, 1);
    }
    let windowed = dock.is_some();
    if windowed {
        ui.add_space(space::XS);
        let n = tab.doc.frame_count();
        let shown = st.shown_frame(tab) + 1;
        ui.label(
            egui::RichText::new(format!("{shown} / {n}"))
                .font(theme::ui_font(font::BODY))
                .color(c.text_secondary)
                .monospace(),
        );
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if let Some(dock) = dock
            && widgets::ds_icon_button(ui, icon::SIDEBAR_SIMPLE, metric::ICON_BUTTON, 14.0, false, false)
                .on_hover_text("Dock the preview in the side panel")
                .clicked()
        {
            *dock = true;
        }
        let names: Vec<&str> = PreviewZoom::ALL.iter().map(|z| z.title()).collect();
        let sel = PreviewZoom::ALL.iter().position(|z| z == zoom).unwrap_or(1);
        if let Some(i) = widgets::ds_popup(ui, "preview-zoom", sel, &names, Some(76.0), false, "Preview zoom") {
            *zoom = PreviewZoom::ALL[i];
        }
    });
}

/// The sprite on a checkerboard, centered at a whole-point position (`PreviewCanvasNSView.draw`).
fn paint_canvas(ui: &mut egui::Ui, st: &mut PreviewState, tab: &DocTab, zoom: PreviewZoom, area: Rect, dark: bool) {
    let (w, h) = (tab.doc.width(), tab.doc.height());
    if w == 0 || h == 0 {
        return;
    }
    let mut scale = match zoom.scale() {
        Some(n) => n as f32,
        None => (area.width() / w as f32).min(area.height() / h as f32),
    };
    if zoom == PreviewZoom::Fit && scale >= 1.0 {
        scale = scale.floor();
    }
    let size = Vec2::new(w as f32 * scale, h as f32 * scale);
    let min = area.min + ((area.size() - size) / 2.0).round();
    let rect = Rect::from_min_size(min, size);
    let painter = ui.painter_at(area);
    // The checkerboard of the canvas, anchored at the sprite's corner.
    let (light, darker) = if dark {
        (Color32::from_rgb(0x3A, 0x3A, 0x3D), Color32::from_rgb(0x2F, 0x2F, 0x32))
    } else {
        (Color32::from_rgb(0xF3, 0xF3, 0xF5), Color32::from_rgb(0xDA, 0xDA, 0xDF))
    };
    let vis = rect.intersect(area);
    if vis.is_positive() {
        let mut mesh = egui::Mesh::default();
        mesh.add_colored_rect(vis, light);
        let cell = 8.0;
        let (i1, j1) =
            (((vis.right() - rect.left()) / cell).ceil() as i32, ((vis.bottom() - rect.top()) / cell).ceil() as i32);
        let (i0, j0) =
            (((vis.left() - rect.left()) / cell).floor() as i32, ((vis.top() - rect.top()) / cell).floor() as i32);
        for j in j0..j1 {
            for i in i0..i1 {
                if (i + j) % 2 == 0 {
                    let r = Rect::from_min_size(rect.min + Vec2::new(i as f32, j as f32) * cell, Vec2::splat(cell));
                    mesh.add_colored_rect(r.intersect(vis), darker);
                }
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }
    let frame = st.shown_frame(tab);
    st.seen = (tab.revision, tab.frame);
    let key = (tab.doc.raw() as usize, tab_revision(tab), frame, w, h);
    if st.texture_key != Some(key) && tab.doc.render(frame, &mut st.buf) {
        let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &st.buf);
        match &mut st.texture {
            Some(t) => t.set(image, egui::TextureOptions::NEAREST),
            None => st.texture = Some(ui.ctx().load_texture("preview", image, egui::TextureOptions::NEAREST)),
        }
        st.texture_key = Some(key);
    }
    if let Some(t) = &st.texture {
        painter.image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    }
}

fn tab_revision(tab: &DocTab) -> u64 {
    tab.revision
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Doc;

    fn tab_with_frames(n: u32) -> DocTab {
        let mut doc = Doc::new(8, 8);
        for _ in 1..n {
            let last = doc.frame_count() - 1;
            doc.add_frame(last, false);
        }
        DocTab::new(doc)
    }

    #[test]
    fn preview_plays_on_its_own_and_wraps() {
        let tab = tab_with_frames(3);
        let mut st = PreviewState::default();
        st.toggle_play(&tab);
        st.advance(&tab, false, 0.0);
        assert_eq!(st.frame, 0);
        // Frame durations default to 100 ms.
        st.advance(&tab, false, 0.05);
        assert_eq!(st.frame, 0, "not due yet");
        st.advance(&tab, false, 0.11);
        assert_eq!(st.frame, 1);
        st.advance(&tab, false, 0.22);
        st.advance(&tab, false, 0.33);
        assert_eq!(st.frame, 0, "wraps after the last frame");
        assert_eq!(tab.frame, 0, "the editor's frame is untouched");
    }

    #[test]
    fn stepping_pauses_and_moves_the_editor_frame() {
        let mut tab = tab_with_frames(3);
        let mut st = PreviewState::default();
        st.step(&mut tab, -1);
        assert!(!st.playing);
        assert_eq!((tab.frame, st.shown_frame(&tab)), (2, 2), "wraps backwards");
        st.step(&mut tab, 1);
        assert_eq!(tab.frame, 0);
    }

    #[test]
    fn preview_starts_paused_and_follows_the_editor_frame() {
        let mut tab = tab_with_frames(3);
        let mut st = PreviewState::default();
        assert!(!st.playing, "the preview starts paused");
        for f in [0, 2, 1] {
            tab.frame = f;
            st.advance(&tab, false, 1.0);
            assert_eq!(st.shown_frame(&tab), f);
        }
        // Playing runs its own frame from the editor's; pausing again returns to the editor's frame.
        st.toggle_play(&tab);
        st.advance(&tab, false, 2.0);
        st.advance(&tab, false, 2.2);
        assert_eq!((st.shown_frame(&tab), tab.frame), (2, 1));
        st.toggle_play(&tab);
        assert_eq!(st.shown_frame(&tab), 1);
    }

    #[test]
    fn range_follows_the_tag_of_the_current_frame() {
        use crate::ui::timeline::TagInfo;
        let mut tab = tab_with_frames(6);
        tab.doc.add_tag(&TagInfo { name: "a".into(), from: 2, to: 4, direction: 0, repeat: 0, color: 0 });
        tab.frame = 3;
        assert_eq!(preview_range(&tab, true), 2..=4);
        assert_eq!(preview_range(&tab, false), 0..=5);
        tab.frame = 0;
        assert_eq!(preview_range(&tab, true), 0..=5);
    }
}
