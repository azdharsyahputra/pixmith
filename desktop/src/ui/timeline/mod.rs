//! The timeline: transport, layers column and cel grid (`Timeline.swift`, `TagsView.swift`, `LayerTools.swift`,
//! `LayerDrag.swift`).
//!
//! Layout, one-to-one with `TimelinePanel`: a 36 pt header (transport, frame and time fields, loop / onion /
//! grid toggles, frame actions, collapse chevron) above a 1 pt soft rule and the grid: the 200 pt layers column
//! on the left, the tag bars, frame numbers and cel grid on the right.

mod controls;
mod grid;
pub mod header;
pub mod layers;
pub mod playback;
mod sheets;
mod tags;

use crate::app::{DocTab, PixmithApp};
use crate::theme::{Colors, metric};
use eframe::egui::{self, Rect, pos2, vec2};
use pixmith_core::document::layers::PlayState;
use std::collections::HashSet;

pub use tags::TagInfo;

/// Where a dragged layer lands relative to the row it is dropped on (`LayerDropZone`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayerDropZone {
    Above,
    Below,
    Into,
}

/// Which cels of the timeline grid are selected: an inclusive rectangle of layers x frames.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CelRect {
    pub l0: u32,
    pub l1: u32,
    pub f0: u32,
    pub f1: u32,
}

impl CelRect {
    pub fn contains(&self, layer: u32, frame: u32) -> bool {
        (self.l0..=self.l1).contains(&layer) && (self.f0..=self.f1).contains(&frame)
    }

    pub fn is_multi(&self) -> bool {
        self.l0 != self.l1 || self.f0 != self.f1
    }

    pub fn shifted(&self, dl: i32, df: i32) -> CelRect {
        let s = |v: u32, d: i32| (v as i64 + d as i64).max(0) as u32;
        CelRect { l0: s(self.l0, dl), l1: s(self.l1, dl), f0: s(self.f0, df), f1: s(self.f1, df) }
    }
}

/// A press in the cel grid that may become a drag of the selected cels.
#[derive(Clone, Copy)]
struct Press {
    start: (u32, u32),
    rect: CelRect,
    delta: (i32, i32),
}

/// A layer row being renamed in place.
struct Renaming {
    layer: u32,
    text: String,
    focused: bool,
}

/// A layer row being dragged to reorder.
struct LayerDrag {
    source: u32,
    target: Option<(u32, LayerDropZone)>,
}

/// State of this region that outlives a frame.
pub struct TimelineState {
    /// The panel is folded to its header (`panel.timeline.collapsed`).
    pub collapsed: bool,
    /// Loop inside the tag containing the current frame.
    pub loop_tags: bool,
    /// Onion skin on the canvas.
    pub onion_skin: bool,
    /// Pixel grid on the canvas.
    pub show_grid: bool,
    /// The selected cel range, if more than the current cel.
    range: Option<CelRect>,
    anchor: Option<(u32, u32)>,
    /// True after a click in the grid: copy, paste and delete then act on cels instead of pixels.
    pub focused: bool,
    press_active: bool,
    press: Option<Press>,
    collapsed_groups: HashSet<u32>,
    scroll: egui::Vec2,
    cells: Vec<u8>,
    cells_key: Option<(usize, u64, u32, u32)>,
    renaming: Option<Renaming>,
    layer_drag: Option<LayerDrag>,
    tag_sheet: Option<sheets::TagSheet>,
    layer_sheet: Option<sheets::LayerSheet>,
    frame_buf: String,
    time_buf: String,
    scrub: Option<controls::Scrub>,
    play_cursor: PlayState,
    /// `input.time` when the current frame started showing during playback.
    frame_started: Option<f64>,
}

impl TimelineState {
    /// A tag or layer sheet is open.
    pub fn sheet_open(&self) -> bool {
        self.tag_sheet.is_some() || self.layer_sheet.is_some()
    }
}

impl Default for TimelineState {
    fn default() -> TimelineState {
        TimelineState {
            collapsed: false,
            loop_tags: true,
            onion_skin: false,
            show_grid: true,
            range: None,
            anchor: None,
            focused: false,
            press_active: false,
            press: None,
            collapsed_groups: HashSet::new(),
            scroll: egui::Vec2::ZERO,
            cells: Vec::new(),
            cells_key: None,
            renaming: None,
            layer_drag: None,
            tag_sheet: None,
            layer_sheet: None,
            frame_buf: String::new(),
            time_buf: String::new(),
            scrub: None,
            play_cursor: PlayState::default(),
            frame_started: None,
        }
    }
}

impl TimelineState {
    /// Opens the layer properties sheet (Layer > Layer Properties…).
    pub fn open_layer_sheet(&mut self, tab: &DocTab, layer: u32) {
        self.layer_sheet = Some(sheets::LayerSheet::new(tab, layer));
    }

    /// Applies and closes the open layer properties sheet as its OK button does.
    pub fn apply_layer_sheet(&mut self, tab: &mut DocTab) {
        if let Some(sheet) = self.layer_sheet.take() {
            sheets::apply_layer(&sheet, tab);
        }
    }

    /// The open layer properties sheet, for tests.
    pub fn layer_sheet_mut(&mut self) -> Option<&mut sheets::LayerSheet> {
        self.layer_sheet.as_mut()
    }

    /// Opens the sheet for a new tag over the current frame (Frame > New Tag…).
    pub fn open_tag_sheet(&mut self, tab: &DocTab) {
        self.tag_sheet = Some(sheets::TagSheet { index: None, tag: tags::new_tag_draft(&tab.doc, tab.frame) });
    }

    /// The selected cels, clamped to the sprite (`selectedCels`).
    fn selected_cels(&self, tab: &DocTab) -> CelRect {
        let (nl, nf) = (tab.doc.layer_count(), tab.doc.frame_count());
        let one = CelRect { l0: tab.layer, l1: tab.layer, f0: tab.frame, f1: tab.frame };
        let Some(mut r) = self.range else { return one };
        r.l1 = r.l1.min(nl.saturating_sub(1));
        r.f1 = r.f1.min(nf.saturating_sub(1));
        if r.l0 <= r.l1 && r.f0 <= r.f1 { r } else { one }
    }

    /// Forgets everything tied to the document's shape (after switching tabs or replacing the document).
    pub fn reset_for_document(&mut self) {
        self.range = None;
        self.anchor = None;
        self.press = None;
        self.press_active = false;
        self.collapsed_groups.clear();
        self.renaming = None;
        self.layer_drag = None;
        self.cells_key = None;
        self.frame_started = None;
    }
}

/// Height of the panel: the header alone when collapsed, else it grows with the layer count up to 252 pt.
pub fn panel_height(app: &PixmithApp) -> f32 {
    if app.timeline.collapsed {
        return metric::TIMELINE_HEADER;
    }
    let rows = app.tab().doc.layer_count() as f32;
    metric::TIMELINE_HEIGHT.min(168.0_f32.max(metric::TIMELINE_HEADER + 48.0 + rows * metric::TIMELINE_ROW + 8.0))
}

pub fn show(app: &mut PixmithApp, ui: &mut egui::Ui) {
    let PixmithApp { tabs, active, timeline: st, .. } = app;
    let Some(tab) = tabs.get_mut(*active) else { return };
    let ctx = ui.ctx().clone();
    let c = Colors::of(&ctx);
    let rect = ui.max_rect();

    playback::update(tab, st, &ctx);

    let head = Rect::from_min_size(rect.min, vec2(rect.width(), metric::TIMELINE_HEADER));
    header::show(tab, st, ui, head);
    if !st.collapsed {
        let y = head.bottom();
        ui.painter().rect_filled(
            Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), 1.0)),
            0.0,
            c.separator_soft,
        );
        let area = Rect::from_min_max(pos2(rect.left(), y + 1.0), rect.max);
        grid::show(tab, st, *active, ui, area);
    }
    // The hairline on the top edge.
    ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(rect.width(), 1.0)), 0.0, c.separator);

    if let Some(mut sheet) = st.tag_sheet.take()
        && !sheets::show_tag(&ctx, &mut sheet, tab)
    {
        st.tag_sheet = Some(sheet);
    }
    if let Some(mut sheet) = st.layer_sheet.take()
        && !sheets::show_layer(&ctx, &mut sheet, tab)
    {
        st.layer_sheet = Some(sheet);
    }
}
