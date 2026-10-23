//! Core calls used by the timeline code. Keep the `unsafe` here; panels call these safe methods.
//!
//! One-to-one with the `SpriteModel` extensions in `Timeline.swift`, `TagsView.swift`, `LayerTools.swift` and
//! `LayerDrag.swift`. Every mutating method marks the document dirty (`pixmith_touch`), names its undo step and
//! returns whether the core accepted the edit; the caller then calls `DocTab::changed`.

use crate::doc::{Doc, take_string};
use crate::ui::timeline::{LayerDropZone, TagInfo};
use pixmith_core::document::layers as layers_ffi;
use pixmith_core::document::layers::{PixmithTag, PlayState};
use pixmith_core::document::tilemap::pixmith_layer_is_tilemap;
use pixmith_core::ffi::document as core;
use pixmith_core::history;
use std::ffi::CString;

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

fn raw_tag(t: &TagInfo) -> PixmithTag {
    PixmithTag {
        from: t.from,
        to: t.to,
        direction: t.direction,
        repeat: t.repeat.min(u16::MAX as u32) as u16,
        color: t.color & 0xFF_FFFF,
    }
}

impl Doc {
    // SAFETY note for this file: `self.raw()` is a live document owned by `self`, and every string passed is a
    // NUL-terminated `CString` that outlives the call.

    /// Names the most recent undo step (`labelLastStep`).
    pub fn label_last_step(&mut self, name: &str) {
        let c = cstr(name);
        unsafe { history::pixmith_checkpoint_label(self.raw(), c.as_ptr()) }
    }

    /// Runs `f` so that every core operation inside it forms a single undo step.
    pub fn in_transaction<T>(&mut self, f: impl FnOnce(&mut Doc) -> T) -> T {
        unsafe { history::pixmith_begin_transaction(self.raw()) };
        let out = f(self);
        unsafe { history::pixmith_end_transaction(self.raw()) };
        out
    }

    // MARK: Layers

    pub fn layer_visible(&self, layer: u32) -> bool {
        unsafe { core::pixmith_layer_visible(self.raw(), layer) }
    }

    pub fn layer_is_group(&self, layer: u32) -> bool {
        unsafe { core::pixmith_layer_is_group(self.raw(), layer) }
    }

    /// Nesting depth: 0 at the top level.
    pub fn layer_level(&self, layer: u32) -> u32 {
        unsafe { core::pixmith_layer_level(self.raw(), layer) }
    }

    pub fn layer_is_tilemap(&self, layer: u32) -> bool {
        unsafe { pixmith_layer_is_tilemap(self.raw(), layer) }
    }

    pub fn layer_opacity(&self, layer: u32) -> u8 {
        unsafe { core::pixmith_layer_opacity(self.raw(), layer) }
    }

    pub fn layer_blend_mode(&self, layer: u32) -> u16 {
        unsafe { layers_ffi::pixmith_layer_blend_mode(self.raw(), layer) }
    }

    pub fn can_merge_down(&self, layer: u32) -> bool {
        unsafe { layers_ffi::pixmith_layer_can_merge_down(self.raw(), layer) }
    }

    pub fn set_layer_visible(&mut self, layer: u32, visible: bool) {
        unsafe { core::pixmith_set_layer_visible(self.raw(), layer, visible) };
        self.touch();
    }

    pub fn rename_layer(&mut self, layer: u32, name: &str) {
        let c = cstr(name);
        unsafe { core::pixmith_rename_layer(self.raw(), layer, c.as_ptr()) };
        self.label_last_step("Rename Layer");
        self.touch();
    }

    /// Adds a layer above `above`; returns its index.
    pub fn add_layer(&mut self, above: u32) -> u32 {
        let i = unsafe { core::pixmith_add_layer(self.raw(), above) };
        self.label_last_step("Add Layer");
        self.touch();
        i
    }

    pub fn remove_layer(&mut self, layer: u32) -> bool {
        let ok = unsafe { core::pixmith_remove_layer(self.raw(), layer) };
        if ok {
            self.label_last_step("Delete Layer");
            self.touch();
        }
        ok
    }

    fn layer_result(&mut self, result: i32, name: &str) -> Option<u32> {
        if result < 0 {
            return None;
        }
        self.label_last_step(name);
        self.touch();
        Some(result as u32)
    }

    /// Adds a group above `above`; returns its index.
    pub fn add_group(&mut self, above: u32) -> Option<u32> {
        let r = unsafe { layers_ffi::pixmith_layer_add_group(self.raw(), above) };
        self.layer_result(r, "New Group")
    }

    /// Moves a layer one step; returns its new index.
    pub fn move_layer(&mut self, layer: u32, up: bool) -> Option<u32> {
        let r = unsafe { layers_ffi::pixmith_layer_move(self.raw(), layer, up) };
        self.layer_result(r, "Move Layer")
    }

    pub fn duplicate_layer(&mut self, layer: u32) -> Option<u32> {
        let r = unsafe { layers_ffi::pixmith_layer_duplicate(self.raw(), layer) };
        self.layer_result(r, "Duplicate Layer")
    }

    pub fn merge_layer_down(&mut self, layer: u32) -> Option<u32> {
        let r = unsafe { layers_ffi::pixmith_layer_merge_down(self.raw(), layer) };
        self.layer_result(r, "Merge Layer Down")
    }

    pub fn wrap_in_group(&mut self, layer: u32) -> Option<u32> {
        let r = unsafe { layers_ffi::pixmith_layer_group(self.raw(), layer) };
        self.layer_result(r, "Group Layer")
    }

    pub fn move_out_of_group(&mut self, layer: u32) -> Option<u32> {
        let r = unsafe { layers_ffi::pixmith_layer_out_of_group(self.raw(), layer) };
        self.layer_result(r, "Move Out of Group")
    }

    pub fn move_layer_into(&mut self, layer: u32, group: u32) -> Option<u32> {
        let r = unsafe { layers_ffi::pixmith_layer_into_group(self.raw(), layer, group) };
        self.layer_result(r, "Move Into Group")
    }

    /// The drawable (non-group) layer at or above `from`, if any: where the selection goes after a
    /// structural change (`finish` in `LayerTools.swift`).
    pub fn drawable_from(&self, from: u32) -> Option<u32> {
        (from..self.layer_count()).find(|&i| !self.layer_is_group(i))
    }

    /// Keeps `layer` and `frame` inside the document and off group rows (`clampSelection`).
    pub fn clamp_selection(&self, layer: u32, frame: u32) -> (u32, u32) {
        let frame = frame.min(self.frame_count().saturating_sub(1));
        let mut layer = layer.min(self.layer_count().saturating_sub(1));
        if self.layer_is_group(layer)
            && let Some(i) = (0..self.layer_count()).rev().find(|&i| !self.layer_is_group(i))
        {
            layer = i;
        }
        (layer, frame)
    }

    /// Number of layers in the subtree rooted at `i` (itself plus its children).
    fn subtree_size(&self, i: u32) -> u32 {
        let mut end = i + 1;
        while end < self.layer_count() && self.layer_level(end) > self.layer_level(i) {
            end += 1;
        }
        end - i
    }

    /// Groups a layer can be moved into: any group outside its own subtree.
    pub fn group_targets(&self, layer: u32) -> Vec<u32> {
        let end = layer + self.subtree_size(layer);
        (0..self.layer_count()).filter(|&g| self.layer_is_group(g) && !(layer..end).contains(&g)).collect()
    }

    fn parent_layer(&self, i: u32) -> Option<u32> {
        let l = self.layer_level(i);
        if l == 0 {
            return None;
        }
        (0..i).rev().find(|&j| self.layer_level(j) + 1 == l)
    }

    /// Indices of the layers directly inside `parent` (or at the top level), bottom to top.
    fn siblings(&self, parent: Option<u32>) -> Vec<u32> {
        let l = parent.map_or(0, |p| self.layer_level(p) + 1);
        let mut out = Vec::new();
        let mut j = parent.map_or(0, |p| p + 1);
        while j < self.layer_count() && self.layer_level(j) >= l {
            if self.layer_level(j) == l {
                out.push(j);
            }
            j += 1;
        }
        out
    }

    /// Moves layer `i` (with its children) next to, or into, layer `t` as one undo step (`dropLayer`).
    /// Returns the layer to select afterwards, or `None` when the drop is invalid.
    pub fn drop_layer(&mut self, i: u32, t: u32, zone: LayerDropZone, selected: u32) -> Option<u32> {
        let count = self.layer_count();
        if i == t || i >= count || t >= count {
            return None;
        }
        let n = self.subtree_size(i);
        if (i..i + n).contains(&t) {
            return None;
        }
        let (mut cur, mut tgt) = (i, t);
        let mut steps = 0;
        let mut ok = true;
        // Records that the block moved from `cur` to `new`, keeping `tgt` pointing at the same layer.
        let mut moved = |new: i32, cur: &mut u32, tgt: &mut u32| -> bool {
            if new < 0 {
                return false;
            }
            let mut t2 = *tgt;
            if t2 > *cur {
                t2 -= n;
            }
            if t2 >= new as u32 {
                t2 += n;
            }
            *tgt = t2;
            *cur = new as u32;
            steps += 1;
            true
        };
        unsafe { history::pixmith_begin_transaction(self.raw()) };
        if zone == LayerDropZone::Into && self.layer_is_group(tgt) {
            if self.parent_layer(cur) != Some(tgt) {
                let r = unsafe { layers_ffi::pixmith_layer_into_group(self.raw(), cur, tgt) };
                ok = moved(r, &mut cur, &mut tgt);
            }
        } else {
            if self.parent_layer(cur) != self.parent_layer(tgt) {
                if let Some(p) = self.parent_layer(tgt) {
                    let r = unsafe { layers_ffi::pixmith_layer_into_group(self.raw(), cur, p) };
                    ok = moved(r, &mut cur, &mut tgt);
                } else {
                    while ok && self.layer_level(cur) > 0 {
                        let r = unsafe { layers_ffi::pixmith_layer_out_of_group(self.raw(), cur) };
                        ok = moved(r, &mut cur, &mut tgt);
                    }
                }
            }
            let mut guard = 0;
            while ok && guard < 2 * count {
                guard += 1;
                let sib = self.siblings(self.parent_layer(tgt));
                let (Some(p), Some(q)) = (sib.iter().position(|&x| x == cur), sib.iter().position(|&x| x == tgt))
                else {
                    break;
                };
                let goal = if zone == LayerDropZone::Below { q as i64 - 1 } else { q as i64 + 1 };
                if p as i64 == goal {
                    break;
                }
                let r = unsafe { layers_ffi::pixmith_layer_move(self.raw(), cur, (p as i64) < goal) };
                ok = moved(r, &mut cur, &mut tgt);
            }
        }
        unsafe { history::pixmith_end_transaction(self.raw()) };
        if steps > 0 {
            self.label_last_step("Reorder Layers");
        }
        self.touch();
        let new = if self.layer_is_group(cur) { selected.min(self.layer_count() - 1) } else { cur };
        ok.then_some(new)
    }

    /// Sets name, opacity, blend mode and visibility in one step (`setLayerProps`).
    pub fn set_layer_props(&mut self, layer: u32, name: &str, opacity: u8, blend: u16, visible: bool) -> bool {
        let c = cstr(name);
        let ok = unsafe { layers_ffi::pixmith_layer_set_props(self.raw(), layer, c.as_ptr(), opacity, blend, visible) };
        if ok {
            self.label_last_step("Layer Properties");
            self.touch();
        }
        ok
    }

    // MARK: Frames and cels

    /// Adds a frame after `after` (a copy of it when `duplicate`); returns its index.
    pub fn add_frame(&mut self, after: u32, duplicate: bool) -> u32 {
        let f = unsafe { core::pixmith_add_frame(self.raw(), after, duplicate) };
        self.label_last_step(if duplicate { "Duplicate Frame" } else { "New Frame" });
        self.touch();
        f
    }

    pub fn remove_frame(&mut self, frame: u32) -> bool {
        let ok = unsafe { core::pixmith_remove_frame(self.raw(), frame) };
        if ok {
            self.label_last_step("Delete Frame");
            self.touch();
        }
        ok
    }

    /// Duration of `frame` in milliseconds.
    pub fn frame_duration(&self, frame: u32) -> u32 {
        unsafe { core::pixmith_frame_duration(self.raw(), frame) as u32 }
    }

    pub fn set_frame_duration(&mut self, frame: u32, ms: u32) {
        unsafe { core::pixmith_set_frame_duration(self.raw(), frame, ms.min(u16::MAX as u32) as u16) };
        self.touch();
    }

    pub fn move_frame(&mut self, from: u32, to: u32) -> bool {
        if to >= self.frame_count() {
            return false;
        }
        let ok = unsafe { layers_ffi::pixmith_frame_move(self.raw(), from, to) };
        if ok {
            self.label_last_step("Move Frame");
            self.touch();
        }
        ok
    }

    /// One byte per cel, layer-major: bit 0 has a cel, bit 1 has content, bit 2 linked. Empty on failure.
    pub fn cel_grid(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.layer_count() as usize * self.frame_count() as usize];
        if unsafe { layers_ffi::pixmith_cel_grid(self.raw(), out.as_mut_ptr(), out.len()) } { out } else { Vec::new() }
    }

    /// Moves (or copies) the cels in the rectangle by the given offset.
    pub fn move_cels(&mut self, layers: (u32, u32), frames: (u32, u32), dl: i32, df: i32, copy: bool) -> bool {
        let ok =
            unsafe { layers_ffi::pixmith_cels_move(self.raw(), layers.0, layers.1, frames.0, frames.1, dl, df, copy) };
        if ok {
            self.label_last_step(if copy { "Copy Cels" } else { "Move Cels" });
            self.touch();
        }
        ok
    }

    pub fn clear_cels(&mut self, layers: (u32, u32), frames: (u32, u32)) -> bool {
        let ok = unsafe { layers_ffi::pixmith_cels_clear(self.raw(), layers.0, layers.1, frames.0, frames.1) };
        if ok {
            self.label_last_step("Clear Cels");
            self.touch();
        }
        ok
    }

    /// Copies the cels to the cel clipboard; not an edit.
    pub fn copy_cels(&self, layers: (u32, u32), frames: (u32, u32)) -> bool {
        unsafe { layers_ffi::pixmith_cels_copy(self.raw(), layers.0, layers.1, frames.0, frames.1) }
    }

    pub fn paste_cels(&mut self, layer: u32, frame: u32) -> bool {
        let ok = unsafe { layers_ffi::pixmith_cels_paste(self.raw(), layer, frame) };
        if ok {
            self.label_last_step("Paste Cels");
            self.touch();
        }
        ok
    }

    pub fn cel_is_linked(&self, layer: u32, frame: u32) -> bool {
        unsafe { layers_ffi::pixmith_cel_is_linked(self.raw(), layer, frame) }
    }

    // MARK: Tags

    pub fn tag_count(&self) -> u32 {
        unsafe { layers_ffi::pixmith_tag_count(self.raw()) }
    }

    pub fn tag(&self, index: u32) -> Option<TagInfo> {
        let mut raw = PixmithTag { from: 0, to: 0, direction: 0, repeat: 0, color: 0 };
        if !unsafe { layers_ffi::pixmith_tag_get(self.raw(), index, &mut raw) } {
            return None;
        }
        let p = unsafe { layers_ffi::pixmith_tag_name(self.raw(), index) };
        if p.is_null() {
            return None;
        }
        let name = unsafe { take_string(p) };
        Some(TagInfo {
            name,
            from: raw.from,
            to: raw.to,
            direction: raw.direction,
            repeat: raw.repeat as u32,
            color: raw.color,
        })
    }

    pub fn add_tag(&mut self, tag: &TagInfo) -> bool {
        let c = cstr(&tag.name);
        let ok = unsafe { layers_ffi::pixmith_tag_add(self.raw(), c.as_ptr(), raw_tag(tag)) } >= 0;
        if ok {
            self.label_last_step("Add Tag");
            self.touch();
        }
        ok
    }

    pub fn update_tag(&mut self, index: u32, tag: &TagInfo) -> bool {
        let c = cstr(&tag.name);
        let ok = unsafe { layers_ffi::pixmith_tag_update(self.raw(), index, c.as_ptr(), raw_tag(tag)) };
        if ok {
            self.label_last_step("Edit Tag");
            self.touch();
        }
        ok
    }

    pub fn remove_tag(&mut self, index: u32) -> bool {
        let ok = unsafe { layers_ffi::pixmith_tag_remove(self.raw(), index) };
        if ok {
            self.label_last_step("Delete Tag");
            self.touch();
        }
        ok
    }

    // MARK: Playback

    /// The frame after `frame` during playback, looping inside the current tag when `loop_tags` is set and
    /// honoring its direction and repeat count. `cursor` carries the ping-pong / repeat state.
    pub fn play_next(&self, frame: u32, cursor: &mut PlayState, loop_tags: bool) -> u32 {
        unsafe { layers_ffi::pixmith_play_next(self.raw(), frame, cursor, loop_tags) }
    }
}
