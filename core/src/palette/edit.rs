//! Palette editing: reorder, insert, duplicate, remove, color ramps and live entry edits.
//!
//! Cel pixels are RGBA colors, so these edits never change the picture. What they must keep
//! in step is the *index* bookkeeping of Indexed sprites: the transparent index and the
//! remembered per-cel indices (`Cel::indices`), which are remapped here.

use crate::document::color_mode::{
    convert, ensure_hints, prepare_transparent, rewrite_cels, set_palette_entry, ColorMode,
};
use crate::document::{Editor, Sprite};
use crate::ffi::{editor, set_error};
use crate::palette::io::hue_sat;

pub type Color = [u8; 4];

const MAX_COLORS: usize = 256;

/// Applies an old-index to new-index map to the transparent index and remembered cel indices.
fn remap_indices(s: &mut Sprite, map: &[u8; 256]) {
    s.transparent_index = map[s.transparent_index as usize];
    rewrite_cels(
        s,
        |_| true,
        |_, cel| {
            if let Some(old) = cel.indices.take() {
                cel.indices = Some(std::sync::Arc::new(old.iter().map(|&i| map[i as usize]).collect()));
            }
        },
    );
}

fn identity_map() -> [u8; 256] {
    std::array::from_fn(|i| i as u8)
}

/// Replaces the palette (first 256 colors). In Indexed mode the sprite gets a transparent entry
/// if needed and its pixels are re-snapped to the new palette.
pub fn replace_palette(s: &mut Sprite, mut colors: Vec<Color>) {
    colors.truncate(MAX_COLORS);
    s.palette = colors;
    if s.color_mode == ColorMode::Indexed {
        prepare_transparent(s);
        convert(s, ColorMode::Indexed, 0);
    }
}

/// Reorders the palette so that `new[i] = old[order[i]]`; `order` must be a permutation.
pub fn reorder(s: &mut Sprite, order: &[usize]) -> bool {
    let n = s.palette.len();
    let mut seen = vec![false; n];
    if order.len() != n || n > MAX_COLORS || order.iter().any(|&o| o >= n || std::mem::replace(&mut seen[o], true)) {
        return false;
    }
    let mut map = identity_map();
    for (new, &old) in order.iter().enumerate() {
        map[old] = new as u8;
    }
    s.palette = order.iter().map(|&o| s.palette[o]).collect();
    remap_indices(s, &map);
    true
}

/// Order that moves the `selected` entries, keeping their relative order, to just before the
/// entry that is currently at `dest` (`dest == len` appends). `None` if nothing would change.
pub fn move_order(len: usize, selected: &[usize], dest: usize) -> Option<Vec<usize>> {
    let mut sel: Vec<usize> = selected.iter().copied().filter(|&i| i < len).collect();
    sel.sort_unstable();
    sel.dedup();
    if sel.is_empty() {
        return None;
    }
    let rest: Vec<usize> = (0..len).filter(|i| !sel.contains(i)).collect();
    let at = rest.iter().filter(|&&i| i < dest.min(len)).count();
    let order: Vec<usize> = rest[..at].iter().chain(&sel).chain(&rest[at..]).copied().collect();
    (order.iter().enumerate().any(|(i, &o)| i != o)).then_some(order)
}

/// Moves entries (see `move_order`). Returns the new index of the first moved entry.
pub fn move_entries(s: &mut Sprite, selected: &[usize], dest: usize) -> Option<usize> {
    let order = move_order(s.palette.len(), selected, dest)?;
    let first = order.iter().position(|o| selected.contains(o))?;
    reorder(s, &order).then_some(first)
}

/// Re-applies a sort done directly on `s.palette`: `before` is the palette before sorting and
/// the current palette must be a permutation of it. Duplicate colors keep their relative order.
pub fn adopt_permutation(s: &mut Sprite, before: &[Color]) {
    let mut used = vec![false; before.len()];
    let mut order = Vec::with_capacity(before.len());
    for c in &s.palette {
        match (0..before.len()).find(|&i| !used[i] && before[i] == *c) {
            Some(i) => {
                used[i] = true;
                order.push(i);
            }
            None => return,
        }
    }
    let sorted = std::mem::replace(&mut s.palette, before.to_vec());
    if order.len() != before.len() || !reorder(s, &order) {
        s.palette = sorted;
    }
}

/// Inserts colors at `at` (clamped to the palette length), as many as fit in 256 entries.
/// Returns the index of the first inserted color.
pub fn insert_entries(s: &mut Sprite, at: usize, colors: &[Color]) -> Option<usize> {
    let at = at.min(s.palette.len());
    let n = colors.len().min(MAX_COLORS.saturating_sub(s.palette.len()));
    if n == 0 {
        return None;
    }
    let mut map = identity_map();
    for old in at..s.palette.len() {
        map[old] = (old + n).min(255) as u8;
    }
    s.palette.splice(at..at, colors[..n].iter().copied());
    remap_indices(s, &map);
    Some(at)
}

/// Inserts a copy of each selected entry right after it. Returns how many were added.
pub fn duplicate_entries(s: &mut Sprite, selected: &[usize]) -> usize {
    let mut sel: Vec<usize> = selected.iter().copied().filter(|&i| i < s.palette.len()).collect();
    sel.sort_unstable();
    sel.dedup();
    sel.truncate(MAX_COLORS.saturating_sub(s.palette.len()));
    if sel.is_empty() {
        return 0;
    }
    let old = std::mem::take(&mut s.palette);
    let mut map = identity_map();
    for (j, slot) in map.iter_mut().enumerate().take(old.len()) {
        *slot = (j + sel.iter().filter(|&&i| i < j).count()).min(255) as u8;
    }
    for (j, &c) in old.iter().enumerate() {
        s.palette.push(c);
        if sel.contains(&j) {
            s.palette.push(c);
        }
    }
    remap_indices(s, &map);
    sel.len()
}

/// Removes entries, always keeping at least one. In Indexed mode pixels that used a removed
/// color are re-snapped to the nearest remaining one. Returns false if nothing was removed.
pub fn remove_entries(s: &mut Sprite, selected: &[usize]) -> bool {
    let n = s.palette.len();
    let mut gone = vec![false; n];
    for &i in selected {
        if i < n {
            gone[i] = true;
        }
    }
    let kept = gone.iter().filter(|g| !**g).count();
    if kept == 0 || kept == n {
        return false;
    }
    let mut map = [255u8; 256];
    let mut next = 0;
    let mut palette = Vec::with_capacity(kept);
    for (old, &g) in gone.iter().enumerate() {
        if !g {
            // Palettes read from files may exceed 256 entries; only the first 256 can be indexed.
            if old < map.len() {
                map[old] = next;
            }
            palette.push(s.palette[old]);
            next = next.wrapping_add(1);
        }
    }
    let ti_removed = gone.get(s.transparent_index as usize).copied().unwrap_or(false);
    s.palette = palette;
    remap_indices(s, &map);
    if s.color_mode == ColorMode::Indexed {
        if ti_removed {
            s.transparent_index = 0;
            prepare_transparent(s);
        }
        convert(s, ColorMode::Indexed, 0);
    }
    true
}

fn to_hsv(c: Color) -> (f32, f32, f32) {
    let (h, s) = hue_sat(c);
    (h, s, c[0].max(c[1]).max(c[2]) as f32 / 255.0)
}

fn from_hsv(h: f32, s: f32, v: f32, a: u8) -> Color {
    let (h, s, v) = (h.rem_euclid(360.0) / 60.0, s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    let (c, m) = (v * s, v - v * s);
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let q = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [q(r), q(g), q(b), a]
}

/// `n` colors strictly between `a` and `b`, evenly spaced, interpolated in RGB or HSV (hue
/// along the shorter arc; a gray endpoint takes the other end's hue).
pub fn ramp(a: Color, b: Color, n: usize, hsv: bool) -> Vec<Color> {
    let lerp = |x: u8, y: u8, t: f32| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    (1..=n)
        .map(|k| {
            let t = k as f32 / (n + 1) as f32;
            let alpha = lerp(a[3], b[3], t);
            if !hsv {
                return [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t), alpha];
            }
            let ((ha, sa, va), (hb, sb, vb)) = (to_hsv(a), to_hsv(b));
            let (ha, hb) = match (sa == 0.0, sb == 0.0) {
                (true, false) => (hb, hb),
                (false, true) => (ha, ha),
                _ => (ha, hb),
            };
            let mut d = hb - ha;
            if d > 180.0 {
                d -= 360.0;
            } else if d < -180.0 {
                d += 360.0;
            }
            from_hsv(ha + d * t, sa + (sb - sa) * t, va + (vb - va) * t, alpha)
        })
        .collect()
}

/// Inserts a ramp of `n` colors between entries `from` and `to`, right after the later one.
/// Returns the index of the first new color.
pub fn insert_ramp(s: &mut Sprite, from: usize, to: usize, n: usize, hsv: bool) -> Option<usize> {
    let (a, b) = (*s.palette.get(from)?, *s.palette.get(to)?);
    // A longer ramp could never be inserted anyway; stop early instead of computing it.
    let n = n.min(MAX_COLORS.saturating_sub(s.palette.len()));
    insert_entries(s, from.max(to) + 1, &ramp(a, b, n, hsv))
}

// ---- C ABI ----

unsafe fn index_list<'a>(p: *const u32, n: u32) -> &'a [u32] {
    if p.is_null() {
        &[]
    } else {
        std::slice::from_raw_parts(p, n as usize)
    }
}

fn to_usize(v: &[u32]) -> Vec<usize> {
    v.iter().map(|&i| i as usize).collect()
}

/// Moves entries to just before `dest` (`dest == count` appends). One undo step. Returns the
/// new index of the first moved entry, or -1 if nothing moved.
#[no_mangle]
pub unsafe extern "C" fn pixmith_pe_move(doc: *mut Editor, indices: *const u32, count: u32, dest: u32) -> i32 {
    crate::ffi::guarded("pixmith_pe_move", doc, || {
        let Some(e) = editor(doc) else { return -1 };
        let sel = to_usize(index_list(indices, count));
        if move_order(e.sprite.palette.len(), &sel, dest as usize).is_none() {
            return -1;
        }
        e.checkpoint();
        e.label_last("Move Palette Entries");
        move_entries(&mut e.sprite, &sel, dest as usize).map_or(-1, |i| i as i32)
    })
}

/// Inserts one color at `at`. One undo step.
#[no_mangle]
pub unsafe extern "C" fn pixmith_pe_insert(doc: *mut Editor, at: u32, rgba: u32) -> bool {
    crate::ffi::guarded("pixmith_pe_insert", doc, || {
        let Some(e) = editor(doc) else { return false };
        if e.sprite.palette.len() >= MAX_COLORS {
            set_error("the palette is full (256 colors)");
            return false;
        }
        e.checkpoint();
        e.label_last("Insert Palette Entry");
        insert_entries(&mut e.sprite, at as usize, &[rgba.to_be_bytes()]).is_some()
    })
}

/// Duplicates entries in place. One undo step. Returns how many were added (0 if none).
#[no_mangle]
pub unsafe extern "C" fn pixmith_pe_duplicate(doc: *mut Editor, indices: *const u32, count: u32) -> u32 {
    crate::ffi::guarded("pixmith_pe_duplicate", doc, || {
        let Some(e) = editor(doc) else { return 0 };
        if e.sprite.palette.len() >= MAX_COLORS {
            set_error("the palette is full (256 colors)");
            return 0;
        }
        if index_list(indices, count).iter().all(|&i| i as usize >= e.sprite.palette.len()) {
            return 0;
        }
        e.checkpoint();
        e.label_last("Duplicate Palette Entries");
        duplicate_entries(&mut e.sprite, &to_usize(index_list(indices, count))) as u32
    })
}

/// Removes entries (at least one stays). One undo step.
#[no_mangle]
pub unsafe extern "C" fn pixmith_pe_remove(doc: *mut Editor, indices: *const u32, count: u32) -> bool {
    crate::ffi::guarded("pixmith_pe_remove", doc, || {
        let Some(e) = editor(doc) else { return false };
        let before = e.sprite.clone();
        if remove_entries(&mut e.sprite, &to_usize(index_list(indices, count))) {
            let after = std::mem::replace(&mut e.sprite, before);
            e.checkpoint();
            e.label_last("Delete Palette Entries");
            e.sprite = after;
            true
        } else {
            false
        }
    })
}

/// Sets an entry while the user drags a color picker. `begin` starts an edit session: it records
/// one undo step and fixes the pixel-to-entry assignment; later calls of the session pass false.
/// Returns whether the entry changed.
#[no_mangle]
pub unsafe extern "C" fn pixmith_pe_set_live(doc: *mut Editor, index: u32, rgba: u32, begin: bool) -> bool {
    crate::ffi::guarded("pixmith_pe_set_live", doc, || {
        let Some(e) = editor(doc) else { return false };
        let c = rgba.to_be_bytes();
        if e.sprite.palette.get(index as usize).is_none_or(|&old| old == c) {
            return false;
        }
        if begin {
            e.checkpoint();
            e.label_last("Edit Palette Entry");
            if e.sprite.color_mode == ColorMode::Indexed {
                ensure_hints(&mut e.sprite);
            }
        }
        set_palette_entry(&mut e.sprite, index as usize, c)
    })
}

/// Inserts `n` interpolated colors between entries `from` and `to` (HSV if `hsv`, else RGB),
/// after the later one. One undo step. Returns the index of the first new color, or -1.
#[no_mangle]
pub unsafe extern "C" fn pixmith_pe_ramp(doc: *mut Editor, from: u32, to: u32, n: u32, hsv: bool) -> i32 {
    crate::ffi::guarded("pixmith_pe_ramp", doc, || {
        let Some(e) = editor(doc) else { return -1 };
        let len = e.sprite.palette.len();
        if from as usize >= len || to as usize >= len || n == 0 {
            return -1;
        }
        if len >= MAX_COLORS {
            set_error("the palette is full (256 colors)");
            return -1;
        }
        e.checkpoint();
        e.label_last("Create Color Ramp");
        insert_ramp(&mut e.sprite, from as usize, to as usize, n as usize, hsv).map_or(-1, |i| i as i32)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::ase;

    const A: Color = [255, 0, 0, 255];
    const B: Color = [0, 255, 0, 255];
    const C: Color = [0, 0, 255, 255];
    const T: Color = [0, 0, 0, 0];

    fn px(s: &Sprite, x: usize) -> Color {
        let r = s.render(0);
        r[x * 4..x * 4 + 4].try_into().unwrap()
    }

    /// Indexed 4x1 sprite: palette [T, A, B, C], pixels [A, B, C, transparent].
    fn indexed() -> Sprite {
        let mut s = Sprite::new(4, 1);
        s.palette = vec![T, A, B, C];
        s.color_mode = ColorMode::Indexed;
        for (x, c) in [A, B, C].into_iter().enumerate() {
            s.draw_line(0, 0, x as i32, 0, x as i32, 0, c, 1);
        }
        s
    }

    fn saved_indices(s: &Sprite) -> Vec<u8> {
        // Re-read through a second sprite sharing the palette to expose written indices.
        let back = ase::read(&ase::write(s)).unwrap();
        back.cels[0][0].as_ref().unwrap().indices.as_ref().unwrap().to_vec()
    }

    #[test]
    fn move_order_semantics() {
        assert_eq!(move_order(5, &[3], 1), Some(vec![0, 3, 1, 2, 4]));
        assert_eq!(move_order(5, &[1], 4), Some(vec![0, 2, 3, 1, 4]));
        assert_eq!(move_order(5, &[1], 5), Some(vec![0, 2, 3, 4, 1]));
        assert_eq!(move_order(5, &[1, 3], 0), Some(vec![1, 3, 0, 2, 4]));
        assert_eq!(move_order(5, &[1, 3], 3), Some(vec![0, 2, 1, 3, 4]));
        assert_eq!(move_order(5, &[2], 2), None);
        assert_eq!(move_order(5, &[2], 3), None);
        assert_eq!(move_order(5, &[], 0), None);
        assert_eq!(move_order(5, &[9], 0), None);
    }

    #[test]
    fn reorder_keeps_the_image_and_remaps_indices() {
        let mut s = indexed();
        let before = s.render(0);
        ensure_hints(&mut s);
        // Move A to the end: [T, B, C, A].
        assert_eq!(move_entries(&mut s, &[1], 4), Some(3));
        assert_eq!(s.palette, vec![T, B, C, A]);
        assert_eq!(s.render(0), before);
        assert_eq!(s.transparent_index, 0);
        assert_eq!(saved_indices(&s), vec![3, 1, 2, 0]);
        // Moving the transparent entry carries the transparent index along.
        assert_eq!(move_entries(&mut s, &[0], 4), Some(3));
        assert_eq!(s.transparent_index, 3);
        assert_eq!(s.render(0), before);
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back.transparent_index, 3);
        assert_eq!(back.render(0), before);
    }

    #[test]
    fn reorder_works_without_remembered_indices() {
        let mut s = indexed();
        let before = s.render(0);
        assert!(reorder(&mut s, &[3, 2, 1, 0]));
        assert_eq!(s.transparent_index, 3);
        assert_eq!(s.render(0), before);
        assert_eq!(ase::read(&ase::write(&s)).unwrap().render(0), before);
        assert!(!reorder(&mut s, &[0, 0, 1, 2]));
        assert!(!reorder(&mut s, &[0, 1]));
    }

    #[test]
    fn reorder_keeps_duplicate_color_indices_distinct() {
        // Two identical entries; pixel 0 uses the second one.
        let mut s = Sprite::new(2, 1);
        s.palette = vec![T, A, A, B];
        s.color_mode = ColorMode::Indexed;
        let mut file = s.clone();
        file.draw_line(0, 0, 0, 0, 0, 0, A, 1);
        file.draw_line(0, 0, 1, 0, 1, 0, A, 1);
        let c = std::sync::Arc::make_mut(file.cels[0][0].as_mut().unwrap());
        c.indices = Some(std::sync::Arc::new(vec![2, 1]));
        assert_eq!(saved_indices(&file), vec![2, 1]);
        assert!(reorder(&mut file, &[0, 3, 2, 1]));
        assert_eq!(file.palette, vec![T, B, A, A]);
        assert_eq!(saved_indices(&file), vec![2, 3], "each pixel stays with its own entry");
    }

    #[test]
    fn insert_duplicate_remove_track_indices() {
        let mut s = indexed();
        let before = s.render(0);
        ensure_hints(&mut s);
        assert_eq!(insert_entries(&mut s, 2, &[[9, 9, 9, 255]]), Some(2));
        assert_eq!(s.palette, vec![T, A, [9, 9, 9, 255], B, C]);
        assert_eq!(s.render(0), before);
        assert_eq!(saved_indices(&s), vec![1, 3, 4, 0]);

        assert_eq!(duplicate_entries(&mut s, &[1, 4]), 2);
        assert_eq!(s.palette, vec![T, A, A, [9, 9, 9, 255], B, C, C]);
        assert_eq!(s.render(0), before);
        assert_eq!(saved_indices(&s), vec![1, 4, 5, 0], "originals keep their pixels");

        // Remove the filler, the duplicates, and an entry used by pixels (re-snapped).
        assert!(remove_entries(&mut s, &[2, 3, 6]));
        assert_eq!(s.palette, vec![T, A, B, C]);
        assert_eq!(s.render(0), before);
        assert!(remove_entries(&mut s, &[2]));
        assert_eq!(s.palette, vec![T, A, C]);
        assert!(s.palette.contains(&px(&s, 1)), "pixel re-snapped to a remaining color");
        assert!(!remove_entries(&mut s, &[0, 1, 2]), "cannot remove everything");
        assert!(!remove_entries(&mut s, &[]));
    }

    #[test]
    fn removing_the_transparent_entry_picks_a_new_one() {
        let mut s = indexed();
        assert!(remove_entries(&mut s, &[0]));
        assert_eq!(s.palette[s.transparent_index as usize][3], 0);
        let back = ase::read(&ase::write(&s)).unwrap();
        assert_eq!(back.render(0), s.render(0));
        assert_eq!(px(&back, 3), T);
    }

    #[test]
    fn insert_stops_at_256() {
        let mut s = Sprite::new(1, 1);
        s.palette = (0..255).map(|i| [i as u8, 0, 0, 255]).collect();
        assert_eq!(insert_entries(&mut s, 0, &[A, B, C]), Some(0));
        assert_eq!(s.palette.len(), 256);
        assert_eq!(insert_entries(&mut s, 0, &[A]), None);
        assert_eq!(duplicate_entries(&mut s, &[0]), 0);
    }

    #[test]
    fn sort_adoption_remaps_like_a_permutation() {
        let mut s = indexed();
        let before = s.render(0);
        ensure_hints(&mut s);
        let old = s.palette.clone();
        s.palette.sort(); // [T, C, B, A]: sorted by value
        adopt_permutation(&mut s, &old);
        assert_eq!(s.palette, vec![T, C, B, A]);
        assert_eq!(s.render(0), before);
        assert_eq!(saved_indices(&s), vec![3, 2, 1, 0]);
    }

    #[test]
    fn rgb_and_hsv_ramps() {
        let black = [0, 0, 0, 255];
        let white = [255, 255, 255, 255];
        assert_eq!(ramp(black, white, 3, false), vec![[64, 64, 64, 255], [128, 128, 128, 255], [191, 191, 191, 255]]);
        assert!(ramp(black, white, 0, false).is_empty());
        // RGB red -> green goes through dark olive, HSV stays bright via yellow.
        let mid_rgb = ramp(A, B, 1, false)[0];
        let mid_hsv = ramp(A, B, 1, true)[0];
        assert_eq!(mid_rgb, [128, 128, 0, 255]);
        assert_eq!(mid_hsv, [255, 255, 0, 255]);
        // Hue takes the short way round: red (0) -> magenta (300) passes through 330.
        let m = ramp(A, [255, 0, 255, 255], 1, true)[0];
        assert!(m[0] == 255 && m[1] == 0 && m[2] > 100 && m[2] < 160, "{m:?}");
        // Gray endpoint keeps the other hue.
        let g = ramp([128, 128, 128, 255], [255, 0, 0, 255], 1, true)[0];
        assert!(g[0] > g[1] && g[1] == g[2], "{g:?}");
        // Alpha interpolates.
        assert_eq!(ramp([0, 0, 0, 0], [0, 0, 0, 200], 1, false)[0][3], 100);
    }

    #[test]
    fn ramp_is_inserted_after_the_later_endpoint() {
        let mut s = Sprite::new(1, 1);
        s.palette = vec![[0, 0, 0, 255], [10, 10, 10, 255], [200, 200, 200, 255]];
        assert_eq!(insert_ramp(&mut s, 2, 0, 2, false), Some(3));
        assert_eq!(s.palette.len(), 5);
        assert_eq!(s.palette[3], [133, 133, 133, 255], "ramp runs from the first to the second entry");
        assert_eq!(s.palette[4], [67, 67, 67, 255]);
        assert_eq!(insert_ramp(&mut s, 9, 0, 2, false), None);
    }

    #[test]
    fn live_edit_is_one_undo_step_and_follows_only_its_entry() {
        // Duplicate entries A at 1 and 2; pixel 0 uses index 1, pixel 1 uses index 2.
        let mut s = Sprite::new(2, 1);
        s.palette = vec![T, A, A, B];
        s.color_mode = ColorMode::Indexed;
        s.draw_line(0, 0, 0, 0, 1, 0, A, 1);
        std::sync::Arc::make_mut(s.cels[0][0].as_mut().unwrap()).indices = Some(std::sync::Arc::new(vec![1, 2]));
        let mut e = Editor::new(s);
        let before = e.sprite.clone();
        unsafe {
            assert!(pixmith_pe_set_live(&mut e, 2, 0x112233FF, true));
            assert!(pixmith_pe_set_live(&mut e, 2, 0x445566FF, false));
            assert!(pixmith_pe_set_live(&mut e, 2, 0x778899FF, false));
            assert!(!pixmith_pe_set_live(&mut e, 2, 0x778899FF, false));
        }
        assert_eq!(px(&e.sprite, 0), A, "the other duplicate keeps its pixels");
        assert_eq!(px(&e.sprite, 1), [0x77, 0x88, 0x99, 255]);
        assert_eq!(saved_indices(&e.sprite), vec![1, 2]);
        assert!(e.undo());
        assert_eq!(e.sprite, before);
        assert!(!e.undo(), "one undo step per session");
    }

    #[test]
    fn ffi_operations_are_single_undo_steps() {
        let mut e = Editor::new(indexed());
        let before = e.sprite.clone();
        unsafe {
            let sel = [1u32, 2];
            assert_eq!(pixmith_pe_move(&mut e, sel.as_ptr(), 2, 4), 2);
            assert_eq!(e.sprite.palette, vec![T, C, A, B]);
            assert_eq!(pixmith_pe_move(&mut e, sel.as_ptr(), 0, 4), -1);
            assert!(pixmith_pe_insert(&mut e, 1, 0x010203FF));
            assert_eq!(pixmith_pe_duplicate(&mut e, [1u32].as_ptr(), 1), 1);
            assert_eq!(pixmith_pe_ramp(&mut e, 0, 3, 2, true), 4);
            assert!(pixmith_pe_remove(&mut e, [1u32].as_ptr(), 1));
            assert_eq!(pixmith_pe_ramp(&mut e, 0, 99, 2, true), -1);
        }
        for _ in 0..5 {
            assert!(e.undo());
        }
        assert!(!e.undo());
        assert_eq!(e.sprite, before);
    }
}
