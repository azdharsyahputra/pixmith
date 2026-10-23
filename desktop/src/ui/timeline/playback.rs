//! Playback with per-frame durations and the Space / , / . shortcuts (`play`, `stop`, `step` in `SpriteModel`).

use super::TimelineState;
use crate::app::DocTab;
use eframe::egui::{self, Key, Modifiers};
use pixmith_core::document::layers::PlayState;

pub fn stop(tab: &mut DocTab) {
    tab.playing = false;
}

pub fn play(tab: &mut DocTab, st: &mut TimelineState, now: f64) {
    if tab.doc.frame_count() <= 1 {
        return;
    }
    tab.playing = true;
    st.play_cursor = PlayState::default();
    st.frame_started = Some(now);
}

pub fn toggle(tab: &mut DocTab, st: &mut TimelineState, now: f64) {
    if tab.playing { stop(tab) } else { play(tab, st, now) }
}

/// Steps the current frame by `delta`, wrapping.
pub fn step(tab: &mut DocTab, delta: i32) {
    let n = tab.doc.frame_count().max(1) as i32;
    tab.frame = ((tab.frame as i32 + delta).rem_euclid(n)) as u32;
}

/// Handles the shortcuts and advances playback when the current frame's duration has passed. Asks for a
/// repaint at the moment the next frame is due.
pub fn update(tab: &mut DocTab, st: &mut TimelineState, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    if !ctx.egui_wants_keyboard_input() {
        let (space, prev, next) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Space),
                i.consume_key(Modifiers::NONE, Key::Comma),
                i.consume_key(Modifiers::NONE, Key::Period),
            )
        });
        if space {
            toggle(tab, st, now);
        }
        if prev {
            stop(tab);
            step(tab, -1);
        }
        if next {
            stop(tab);
            step(tab, 1);
        }
    }
    if !tab.playing {
        st.frame_started = None;
        return;
    }
    if tab.doc.frame_count() <= 1 {
        tab.playing = false;
        return;
    }
    let started = *st.frame_started.get_or_insert(now);
    let due = tab.doc.frame_duration(tab.frame).max(10) as f64 / 1000.0;
    if now - started >= due {
        tab.frame = tab.doc.play_next(tab.frame, &mut st.play_cursor, st.loop_tags);
        st.frame_started = Some(now);
        ctx.request_repaint_after(std::time::Duration::from_millis(tab.doc.frame_duration(tab.frame).max(10) as u64));
    } else {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((due - (now - started)).max(0.001)));
    }
}
