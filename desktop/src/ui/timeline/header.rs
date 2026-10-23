//! The 36 pt header: transport, frame and time fields, toggles and frame actions (`TimelineHeader`).

use super::controls::{self, Look, Transport};
use super::sheets::TagSheet;
use super::{TimelineState, playback, tags};
use crate::app::DocTab;
use eframe::egui::{self, Align, Layout, Rect, UiBuilder, vec2};
use egui_phosphor::regular as icon;

pub fn show(tab: &mut DocTab, st: &mut TimelineState, ui: &mut egui::Ui, rect: Rect) {
    let inner = rect.shrink2(vec2(8.0, 0.0));
    let now = ui.input(|i| i.time);
    let nf = tab.doc.frame_count();
    let plain = Look::default();

    // Left group: transport, fields, toggles.
    let mut left = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    left.spacing_mut().item_spacing = vec2(2.0, 0.0);
    let ui_l = &mut left;

    if controls::transport_button(ui_l, Transport::Back, plain, "Previous frame (,)").clicked() {
        playback::stop(tab);
        playback::step(tab, -1);
    }
    let (kind, tip) = if tab.playing { (Transport::Pause, "Pause (Space)") } else { (Transport::Play, "Play (Space)") };
    if controls::transport_button(ui_l, kind, Look { selected: tab.playing, ..plain }, tip).clicked() {
        playback::toggle(tab, st, now);
    }
    if controls::transport_button(ui_l, Transport::Forward, plain, "Next frame (.)").clicked() {
        playback::stop(tab);
        playback::step(tab, 1);
    }
    ui_l.add_space(4.0);
    if let Some(v) = controls::number_field(
        ui_l,
        0,
        &mut st.frame_buf,
        &mut st.scrub,
        "Frame",
        tab.frame as i64 + 1,
        (1, nf.max(1) as i64),
        &format!("/ {nf}"),
        112.0,
        false,
        "Current frame. Drag the label to scrub.",
    ) {
        playback::stop(tab);
        tab.frame = (v - 1).clamp(0, nf as i64 - 1) as u32;
    }
    ui_l.add_space(4.0);
    if let Some(v) = controls::number_field(
        ui_l,
        1,
        &mut st.time_buf,
        &mut st.scrub,
        "Time",
        tab.doc.frame_duration(tab.frame) as i64,
        (10, 10000),
        "ms",
        96.0,
        true,
        "Duration of this frame. Drag the label to scrub.",
    ) {
        tab.doc.set_frame_duration(tab.frame, v as u32);
        tab.changed();
    }
    controls::group_divider(ui_l);
    let toggle = |ui: &mut egui::Ui, glyph: &str, on: &mut bool, tip: &str| {
        if controls::icon_button(ui, glyph, 28.0, 14.0, Look { selected: *on, ..plain }, tip).clicked() {
            *on = !*on;
        }
    };
    toggle(ui_l, icon::REPEAT, &mut st.loop_tags, "Loop tag: play inside the tag that contains the current frame");
    toggle(ui_l, icon::STACK, &mut st.onion_skin, "Onion skin (options in View > View Options)");
    toggle(ui_l, icon::HASH, &mut st.show_grid, "Pixel grid");

    // Right group, laid out right to left.
    let mut right = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::right_to_left(Align::Center)));
    right.spacing_mut().item_spacing = vec2(2.0, 0.0);
    let ui_r = &mut right;
    let (glyph, tip) =
        if st.collapsed { (icon::CARET_UP, "Show timeline") } else { (icon::CARET_DOWN, "Hide timeline") };
    if controls::icon_button(ui_r, glyph, 28.0, 14.0, plain, tip).clicked() {
        st.collapsed = !st.collapsed;
    }
    controls::group_divider(ui_r);
    let can_delete = nf > 1;
    if controls::icon_button(
        ui_r,
        icon::TRASH,
        28.0,
        14.0,
        Look { destructive: true, disabled: !can_delete, ..plain },
        "Delete frame",
    )
    .clicked()
    {
        playback::stop(tab);
        if tab.doc.remove_frame(tab.frame) {
            clamp(tab);
            tab.changed();
        }
    }
    if controls::icon_button(ui_r, icon::TAG, 28.0, 14.0, plain, "New tag from the current frame").clicked() {
        st.tag_sheet = Some(TagSheet { index: None, tag: tags::new_tag_draft(&tab.doc, tab.frame) });
    }
    let can_right = tab.frame + 1 < nf;
    if controls::icon_button(
        ui_r,
        icon::ARROW_LINE_RIGHT,
        28.0,
        14.0,
        Look { disabled: !can_right, ..plain },
        "Move frame right",
    )
    .clicked()
    {
        move_frame(tab, 1);
    }
    let can_left = tab.frame > 0;
    if controls::icon_button(
        ui_r,
        icon::ARROW_LINE_LEFT,
        28.0,
        14.0,
        Look { disabled: !can_left, ..plain },
        "Move frame left",
    )
    .clicked()
    {
        move_frame(tab, -1);
    }
    if controls::icon_button(ui_r, icon::COPY, 28.0, 14.0, plain, "Duplicate frame (\u{21e7}\u{2318}D)").clicked() {
        add_frame(tab, true);
    }
    if controls::icon_button(ui_r, icon::PLUS_SQUARE, 28.0, 14.0, plain, "New frame (\u{2325}\u{2318}N)").clicked() {
        add_frame(tab, false);
    }
}

fn clamp(tab: &mut DocTab) {
    (tab.layer, tab.frame) = tab.doc.clamp_selection(tab.layer, tab.frame);
}

/// `addFrame`: the new frame becomes current.
pub fn add_frame(tab: &mut DocTab, duplicate: bool) {
    playback::stop(tab);
    tab.frame = tab.doc.add_frame(tab.frame, duplicate);
    tab.changed();
}

/// `moveFrame(by:)`: the frame moves and stays current.
pub fn move_frame(tab: &mut DocTab, delta: i32) {
    playback::stop(tab);
    let to = tab.frame as i64 + delta as i64;
    if to < 0 || to >= tab.doc.frame_count() as i64 {
        return;
    }
    if tab.doc.move_frame(tab.frame, to as u32) {
        tab.frame = to as u32;
        tab.changed();
    }
}
