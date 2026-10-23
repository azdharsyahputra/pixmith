//! The C ABI must tolerate out-of-range indices, extreme coordinates, degenerate sizes, null
//! pointers and unusable paths without panicking (the release build aborts on panic), and must
//! leave the document internally consistent.

mod common;

use common::*;
use pixmith_core::document::layers::{self, PixmithTag};
use pixmith_core::document::{color_mode as cm, slices, tilemap as tm, Editor};
use pixmith_core::ffi::document::*;
use pixmith_core::ffi::pixmith_string_free;
use pixmith_core::formats::{animation, export, gif, import, sheet, sheet_import};
use pixmith_core::history;
use pixmith_core::paint::{curves, filters, font, ink, selection as sel, shapes, transform};
use pixmith_core::palette::{edit as palette_edit, io as palette_io, presets as palette_presets};
use pixmith_core::scripting as script;
use std::ffi::CString;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::{null, null_mut};
use std::sync::Mutex;

type Doc = *mut Editor;

/// Where the last panic happened, filled in by the panic hook.
static LAST_PANIC: Mutex<String> = Mutex::new(String::new());
static CURRENT: Mutex<String> = Mutex::new(String::new());

struct Failures(Vec<String>);

impl Failures {
    /// Runs one call, recording a panic together with its call text and source location.
    fn run(&mut self, what: &str, ctx: &str, f: impl FnOnce()) {
        // Panics inside `extern "C"` functions cannot unwind: they abort the process, exactly as
        // in the release app. So the hook names the offending call before the abort.
        *CURRENT.lock().unwrap() = format!("{what} [{ctx}]");
        LAST_PANIC.lock().unwrap().clear();
        let start = std::time::Instant::now();
        let failed = catch_unwind(AssertUnwindSafe(f)).is_err();
        if start.elapsed() > std::time::Duration::from_millis(700) {
            eprintln!("SLOW {:?}: {what} [{ctx}]", start.elapsed());
        }
        if failed {
            let at = LAST_PANIC.lock().unwrap().clone();
            self.0.push(format!("{what} [{ctx}] panicked: {at}"));
        }
    }
}

macro_rules! call {
    ($f:expr, $ctx:expr, $e:expr) => {
        $f.run(stringify!($e), &$ctx, || {
            #[allow(unused_unsafe)]
            let _ = unsafe { $e };
        })
    };
}

fn install_hook() {
    std::panic::set_hook(Box::new(|info| {
        let loc = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| info.payload().downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        if let Ok(mut g) = LAST_PANIC.lock() {
            *g = format!("{loc}: {msg}");
        }
        let current = CURRENT.lock().map(|c| c.clone()).unwrap_or_default();
        eprintln!("PANIC in {current}\n    at {loc}: {msg}");
    }));
}

/// A document that exercises most features: groups, a tilemap layer, linked cels, tags,
/// several frames and a selection.
fn populated() -> Doc {
    let mut s = rgba_sprite();
    let g = s.add_group(1);
    s.layers[g].name = "g".into();
    let t = tm::add_tilemap_layer(&mut s, 0, 4, 4);
    let ts = s.layers[t].tileset as usize;
    let id = s.tilesets[ts].add_blank();
    std::sync::Arc::make_mut(&mut s.tilesets[ts].tiles[id as usize])[..4].copy_from_slice(&RED);
    tm::stamp(&mut s, t, 0, 2, 2, id);
    let d = Box::into_raw(Box::new(Editor::new(s)));
    unsafe { sel::pixmith_sel_rect(d, 1, 1, 6, 5) };
    d
}

fn tiny() -> Doc {
    pixmith_new(1, 1)
}

/// Every cel must keep a consistent size, whatever the calls did.
fn check_doc(d: Doc, what: &str, f: &mut Failures) {
    let e = unsafe { &*d };
    if let Err(p) = catch_unwind(AssertUnwindSafe(|| check_sprite(&e.sprite))) {
        let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
        f.0.push(format!("{what}: document left inconsistent: {msg:?}"));
    }
}

const U: [u32; 7] = [0, 1, 2, 3, 7, 1000, u32::MAX];
const PTS: [(i32, i32); 8] =
    [(0, 0), (-1, -1), (3, 2), (11, 8), (12, 9), (100_000, 100_000), (-100_000, 5), (5, -100_000)];

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn far(v: i32) -> i32 {
    v.clamp(-300_000, 300_000)
}

/// Calls taking a layer and/or frame index.
fn indexed_calls(d: Doc, l: u32, f: u32, g: &mut Failures) {
    let ctx = format!("layer={l} frame={f}");
    let name = cstr("x");
    let mut buf = vec![0u8; 12 * 9 * 4];
    let mut small = [0u8; 3];
    let mut tag = PixmithTag { from: 0, to: 0, direction: 0, repeat: 0, color: 0 };
    let mut reversed = layers::PlayState { reversed: false, passes: 0, tag: -1 };
    call!(g, ctx, pixmith_layer_name(d, l));
    call!(g, ctx, pixmith_layer_visible(d, l));
    call!(g, ctx, pixmith_layer_is_group(d, l));
    call!(g, ctx, pixmith_layer_level(d, l));
    call!(g, ctx, pixmith_layer_opacity(d, l));
    call!(g, ctx, pixmith_set_layer_visible(d, l, false));
    call!(g, ctx, pixmith_set_layer_opacity(d, l, 7));
    call!(g, ctx, pixmith_rename_layer(d, l, name.as_ptr()));
    call!(g, ctx, pixmith_rename_layer(d, l, null()));
    call!(g, ctx, pixmith_frame_duration(d, f));
    call!(g, ctx, pixmith_set_frame_duration(d, f, 0));
    call!(g, ctx, pixmith_render(d, f, buf.as_mut_ptr(), buf.len()));
    call!(g, ctx, pixmith_render(d, f, buf.as_mut_ptr(), 3));
    call!(g, ctx, pixmith_render(d, f, null_mut(), 0));
    call!(g, ctx, layers::pixmith_layer_move(d, l, true));
    call!(g, ctx, layers::pixmith_layer_move(d, l, false));
    call!(g, ctx, layers::pixmith_layer_duplicate(d, l));
    call!(g, ctx, layers::pixmith_layer_can_merge_down(d, l));
    call!(g, ctx, layers::pixmith_layer_merge_down(d, l));
    call!(g, ctx, layers::pixmith_layer_add_group(d, l));
    call!(g, ctx, layers::pixmith_layer_group(d, l));
    call!(g, ctx, layers::pixmith_layer_into_group(d, l, f));
    call!(g, ctx, layers::pixmith_layer_out_of_group(d, l));
    call!(g, ctx, layers::pixmith_layer_blend_mode(d, l));
    call!(g, ctx, layers::pixmith_layer_set_props(d, l, name.as_ptr(), 9, 99, true));
    call!(g, ctx, layers::pixmith_layer_set_props(d, l, null(), 9, 3, true));
    call!(g, ctx, layers::pixmith_frame_move(d, l, f));
    call!(g, ctx, layers::pixmith_cel_opacity(d, l, f));
    call!(g, ctx, layers::pixmith_cel_set_opacity(d, l, f, 9));
    call!(g, ctx, layers::pixmith_cel_is_linked(d, l, f));
    call!(g, ctx, layers::pixmith_cel_link(d, l, f, l));
    call!(g, ctx, layers::pixmith_cel_link(d, l, l, f));
    call!(g, ctx, layers::pixmith_cel_unlink(d, l, f));
    call!(g, ctx, layers::pixmith_tag_get(d, l, &mut tag));
    call!(g, ctx, layers::pixmith_tag_get(d, l, null_mut()));
    call!(g, ctx, layers::pixmith_tag_name(d, l));
    call!(
        g,
        ctx,
        layers::pixmith_tag_update(
            d,
            l,
            name.as_ptr(),
            PixmithTag { from: f, to: l, direction: 200, repeat: 9, color: u32::MAX }
        )
    );
    call!(
        g,
        ctx,
        layers::pixmith_tag_add(d, name.as_ptr(), PixmithTag { from: l, to: f, direction: 7, repeat: 0, color: 0 })
    );
    call!(g, ctx, layers::pixmith_tag_add(d, null(), PixmithTag { from: f, to: f, direction: 0, repeat: 0, color: 0 }));
    call!(g, ctx, layers::pixmith_tag_remove(d, l));
    call!(g, ctx, layers::pixmith_cels_has_clip());
    call!(g, ctx, layers::pixmith_play_next(d, f, &mut reversed, true));
    call!(g, ctx, layers::pixmith_play_next(d, f, null_mut(), false));
    call!(g, ctx, tm::pixmith_layer_is_tilemap(d, l));
    call!(g, ctx, tm::pixmith_layer_tileset(d, l));
    call!(g, ctx, tm::pixmith_tileset_tile_count(d, l));
    call!(g, ctx, tm::pixmith_tileset_tile_width(d, l));
    call!(g, ctx, tm::pixmith_tileset_tile_height(d, l));
    call!(g, ctx, tm::pixmith_tileset_base_index(d, l));
    call!(g, ctx, tm::pixmith_tileset_set_base_index(d, l, i32::MIN));
    call!(g, ctx, tm::pixmith_tileset_name(d, l));
    call!(g, ctx, tm::pixmith_tileset_tile_pixels(d, l, f, small.as_mut_ptr(), 3));
    call!(g, ctx, tm::pixmith_tileset_tile_pixels(d, l, f, buf.as_mut_ptr(), buf.len()));
    call!(g, ctx, tm::pixmith_tileset_add_tile(d, l));
    call!(g, ctx, tm::pixmith_add_tilemap_layer(d, l, f, f));
    call!(g, ctx, tm::pixmith_add_tilemap_layer(d, l, 0, 0));
    call!(g, ctx, tm::pixmith_convert_to_tilemap(d, l, f, f));
    call!(g, ctx, tm::pixmith_convert_to_tilemap(d, l, 0, 0));
    call!(g, ctx, tm::pixmith_convert_to_normal(d, l));
    call!(g, ctx, pixmith_palette_color(d, l));
    call!(g, ctx, pixmith_remove_layer(d, l));
    call!(g, ctx, pixmith_add_layer(d, l));
    call!(g, ctx, pixmith_add_frame(d, f, true));
    call!(g, ctx, pixmith_add_frame(d, f, false));
    call!(g, ctx, pixmith_remove_frame(d, f));
    call!(g, ctx, sel::pixmith_sel_move_begin(d, l, f));
    call!(g, ctx, sel::pixmith_sel_move_to(d, 3, 3));
    call!(g, ctx, sel::pixmith_sel_move_end(d));
    call!(g, ctx, sel::pixmith_sel_nudge(d, l, f, 1, 1));
    call!(g, ctx, sel::pixmith_sel_delete(d, l, f));
    call!(g, ctx, sel::pixmith_sel_copy(d, l, f, buf.as_mut_ptr(), buf.len()));
    call!(g, ctx, sel::pixmith_sel_copy(d, l, f, buf.as_mut_ptr(), 0));
    call!(g, ctx, sel::pixmith_sel_copy(d, l, f, null_mut(), 5));
    for op in [0, 1, 2, 3, 99] {
        call!(g, ctx, sel::pixmith_sel_transform(d, l, f, op));
    }
    call!(g, ctx, ink::pixmith_brush_from_selection(d, l, f));
    for target in 0..4 {
        call!(g, ctx, filters::pixmith_filter_apply(d, 3, target, l, f, 0, 0, 0, 0, [10, 10, 10].as_ptr(), 3));
    }
    call!(g, ctx, filters::pixmith_filter_apply(d, 6, 2, l, f, -5, -5, u32::MAX, u32::MAX, [0, 1, 1].as_ptr(), 3));
    call!(g, ctx, filters::pixmith_filter_apply(d, 9, 0, l, f, i32::MAX, i32::MAX, 10, 10, null(), 0));
    call!(g, ctx, filters::pixmith_filter_preview(d, 9, 1, l, f, 0, 0, 0, 0, null(), 0, buf.as_mut_ptr(), buf.len()));
    call!(g, ctx, filters::pixmith_filter_preview(d, 9, 1, l, f, 0, 0, 0, 0, null(), 0, buf.as_mut_ptr(), 4));
    call!(g, ctx, pixmith_begin_stroke(d));
}

/// Calls taking a layer, a frame and a canvas position.
fn point_calls(d: Doc, l: u32, f: u32, x: i32, y: i32, g: &mut Failures) {
    let ctx = format!("layer={l} frame={f} at ({x},{y})");
    let (x1, y1) = (y, x);
    call!(g, ctx, pixmith_draw_line(d, l, f, x, y, far(x1), far(y1), 0xFF0000FF, 5));
    call!(g, ctx, pixmith_draw_line(d, l, f, x, y, x, y, 0xFF0000FF, u32::MAX));
    call!(g, ctx, pixmith_fill(d, l, f, x, y, 0x00FF00FF));
    call!(g, ctx, ink::pixmith_fill_pattern(d, l, f, x, y, 1, 2, 7, true, 3));
    call!(g, ctx, ink::pixmith_fill_pattern(d, l, f, x, y, 1, 2, u32::MAX, false, u32::MAX));
    call!(g, ctx, shapes::pixmith_fill_mode(d, l, f, x, y, 0x123456FF, false));
    call!(g, ctx, shapes::pixmith_fill_mode(d, l, f, x, y, 0, true));
    call!(g, ctx, ink::pixmith_gradient(d, l, f, x, y, x1, y1, 0xFF0000FF, 0x0000FFFF, 1, 3, false));
    call!(g, ctx, ink::pixmith_gradient(d, l, f, x, y, x, y, 0xFF0000FF, 0x0000FFFF, 0, 0, true));
    for kind in 0..3 {
        call!(g, ctx, shapes::pixmith_shape(d, kind, l, f, x, y, far(x1), far(y1), 0xFF, 3, 7, false));
        call!(g, ctx, shapes::pixmith_shape(d, kind, l, f, x, y, x, y, 0xFF, 0, 0, true));
    }
    call!(g, ctx, shapes::pixmith_stroke_begin(d, l, f, x, y, 0xFF0000FF, 3, 0xFF));
    call!(g, ctx, shapes::pixmith_stroke_to(d, far(x1), far(y1)));
    call!(g, ctx, shapes::pixmith_stroke_end());
    call!(g, ctx, shapes::pixmith_spray(d, l, f, x, y, 0xFF, 20, 100, 42, 3));
    call!(g, ctx, shapes::pixmith_spray(d, l, f, x, y, 0xFF, 5000, u32::MAX, 0, 0));
    call!(g, ctx, tm::pixmith_tile_stamp(d, l, f, x, y, 1));
    call!(g, ctx, tm::pixmith_tile_stamp(d, l, f, x, y, u32::MAX));
    call!(g, ctx, tm::pixmith_tile_stamp(d, l, f, x, y, 0));
    call!(g, ctx, tm::pixmith_tile_at(d, l, f, x, y));
    call!(g, ctx, sel::pixmith_sel_wand(d, l, f, x, y, 40, true, 0));
    call!(g, ctx, sel::pixmith_sel_wand(d, l, f, x, y, u32::MAX, false, u32::MAX));
    call!(g, ctx, sel::pixmith_sel_contains(d, x, y));
    call!(g, ctx, sel::pixmith_sel_rect(d, x, y, x1, y1));
    call!(g, ctx, sel::pixmith_sel_paste(d, l, f, 2, 2, [1u8; 16].as_ptr(), x, y));
    call!(g, ctx, sel::pixmith_sel_paste(d, l, f, 0, 0, [1u8; 16].as_ptr(), x, y));
    call!(g, ctx, sel::pixmith_sel_paste(d, l, f, 2, 2, null(), x, y));
    call!(g, ctx, sel::pixmith_sel_poly(d, [x, y, x1, y1, 5, 5].as_ptr(), 3));
    call!(g, ctx, sel::pixmith_sel_poly(d, null(), 3));
    call!(g, ctx, sel::pixmith_sel_poly(d, [0, 0].as_ptr(), 0));
    call!(g, ctx, sel::pixmith_sel_move_begin(d, l, f));
    call!(g, ctx, sel::pixmith_sel_move_to(d, x, y));
    call!(g, ctx, sel::pixmith_sel_move_end(d));
    call!(g, ctx, sel::pixmith_sel_nudge(d, l, f, x, y));
    call!(g, ctx, pixmith_undo(d));
    call!(g, ctx, pixmith_redo(d));
}

/// Calls that don't take layer or frame indices.
fn global_calls(d: Doc, g: &mut Failures) {
    let ctx = String::new();
    let bad = cstr("/nonexistent-dir/pixmith/x.png");
    let mut buf = vec![0u8; 12 * 9 * 4];
    let mut out4 = [0i32; 4];
    let mut outline = vec![0i32; 64];
    let mut brush = [0i32; 3];
    for v in [0, 1, 2, 3, 4, 5, 6, 9, 1000, u32::MAX] {
        call!(g, ctx, cm::pixmith_cm_convert(d, v, 2));
        call!(g, ctx, cm::pixmith_cm_convert(d, 1, v));
        call!(g, ctx, cm::pixmith_cm_palette_set(d, v, 0x112233FF));
        call!(g, ctx, cm::pixmith_cm_palette_from_sprite(d, v));
        call!(g, ctx, cm::pixmith_cm_snap_color(d, v));
        call!(g, ctx, cm::pixmith_cm_palette_index_of(d, v));
        call!(g, ctx, pixmith_palette_color(d, v));
        call!(g, ctx, palette_io::pixmith_palette_remove(d, v));
        call!(g, ctx, palette_io::pixmith_palette_sort(d, v));
        call!(g, ctx, sel::pixmith_sel_modify(d, v, v.min(50)));
        call!(g, ctx, sel::pixmith_sel_modify(d, 0, v.min(50)));
        call!(g, ctx, sel::pixmith_sel_begin(d, v));
        call!(g, ctx, sel::pixmith_sel_end(d));
        call!(g, ctx, ink::pixmith_ink_set(v, true, [0xFF0000FFu32, 0x00FF00FF].as_ptr(), 2));
        call!(g, ctx, ink::pixmith_ink_set(v, false, null(), 5));
        call!(g, ctx, ink::pixmith_ink_set(v, false, null(), 0));
        call!(g, ctx, ink::pixmith_ink_set(v, false, [0u32; 4].as_ptr(), 0));
        call!(g, ctx, ink::pixmith_brush_circle(v % 2 == 0));
        call!(g, ctx, ink::pixmith_brush_reset());
    }
    call!(g, ctx, sel::pixmith_crop_to_selection(d));
    call!(g, ctx, sel::pixmith_sel_all(d));
    call!(g, ctx, sel::pixmith_crop_to_selection(d));
    call!(g, ctx, sel::pixmith_sel_invert(d));
    call!(g, ctx, sel::pixmith_sel_clear(d));
    call!(g, ctx, sel::pixmith_sel_bounds(d, out4.as_mut_ptr()));
    call!(g, ctx, sel::pixmith_sel_bounds(d, null_mut()));
    call!(g, ctx, sel::pixmith_sel_copy_bounds(d, out4.as_mut_ptr()));
    call!(g, ctx, sel::pixmith_sel_copy_bounds(d, null_mut()));
    call!(g, ctx, sel::pixmith_sel_outline(d, outline.as_mut_ptr(), outline.len()));
    call!(g, ctx, sel::pixmith_sel_outline(d, outline.as_mut_ptr(), 0));
    call!(g, ctx, sel::pixmith_sel_outline(d, outline.as_mut_ptr(), 3));
    call!(g, ctx, sel::pixmith_sel_outline(d, null_mut(), 64));
    call!(g, ctx, sel::pixmith_sel_active(d));
    call!(g, ctx, sel::pixmith_float_active(d));
    call!(g, ctx, sel::pixmith_float_commit(d));
    call!(g, ctx, sel::pixmith_float_cancel(d));
    call!(g, ctx, ink::pixmith_brush_info(brush.as_mut_ptr()));
    call!(g, ctx, ink::pixmith_brush_info(null_mut()));
    call!(g, ctx, history::pixmith_is_dirty(d));
    call!(g, ctx, history::pixmith_mark_saved(d));
    call!(g, ctx, history::pixmith_touch(d));
    call!(g, ctx, pixmith_palette_add(d, 0x01020304));
    call!(g, ctx, pixmith_render(d, 0, buf.as_mut_ptr(), buf.len()));
    // File operations with unusable paths.
    for scale in [0, 1, 64, 1000, u32::MAX] {
        call!(g, ctx, pixmith_export_png(d, 0, scale, bad.as_ptr()));
        call!(g, ctx, pixmith_export_png(d, u32::MAX, scale, bad.as_ptr()));
        call!(g, ctx, pixmith_export_sheet(d, scale, bad.as_ptr()));
        call!(g, ctx, gif::pixmith_export_gif(d, scale, bad.as_ptr()));
        call!(g, ctx, gif::pixmith_export_png_sequence(d, scale, bad.as_ptr()));
        for layout in [0, 2, 9] {
            call!(g, ctx, sheet::pixmith_export_sheet_ex(d, layout, 0, scale % 9, scale, true, bad.as_ptr()));
            call!(
                g,
                ctx,
                sheet::pixmith_export_sheet_ex(
                    d,
                    layout,
                    u32::MAX,
                    if scale == 1 { u32::MAX } else { 1 },
                    scale,
                    false,
                    bad.as_ptr()
                )
            );
        }
    }
    for p in [null(), bad.as_ptr()] {
        call!(g, ctx, pixmith_save(d, p));
        call!(g, ctx, pixmith_export_png(d, 0, 1, p));
        call!(g, ctx, palette_io::pixmith_palette_load(d, p));
        call!(g, ctx, palette_io::pixmith_palette_save(d, p));
        call!(g, ctx, import::pixmith_import_png_layer(d, 0, 0, p));
        call!(g, ctx, import::pixmith_import_png(p));
        call!(g, ctx, import::pixmith_import_sheet(p, 0, 0));
        call!(g, ctx, pixmith_open(p));
    }
    // Resizing last: it can make the canvas big, which slows down everything after it.
    for (w, h) in [
        (0, 0),
        (1, 1),
        (5, 5),
        (13, 4),
        (100_000, 100_000),
        (u32::MAX, u32::MAX),
        (16384, 16384),
        (0, 20_000),
        (2048, 2048),
    ] {
        for (ox, oy) in [(0, 0), (-3, 4), (i32::MAX, i32::MIN), (1_000_000, -1_000_000), (i32::MIN, i32::MIN)] {
            call!(g, format!("{w}x{h} at {ox},{oy}"), sel::pixmith_resize_canvas(d, w, h, ox, oy));
        }
        call!(g, format!("{w}x{h}"), filters::pixmith_scale_sprite(d, w, h));
    }
}

/// Newer entry points taking a layer and/or frame index (cels, tilesets, slices, user data,
/// palette editing, history, scripts, transforms).
fn indexed_calls2(d: Doc, l: u32, f: u32, g: &mut Failures) {
    let ctx = format!("layer={l} frame={f}");
    let name = cstr("name");
    let mut buf = vec![0u8; 12 * 9 * 4];
    let mut out13 = [0i32; 13];
    let mut out4 = [0i32; 4];
    let mut color = 0u32;
    let mut msgs: *mut std::ffi::c_char = null_mut();
    let idx = [l, f, 0, u32::MAX, 3];
    let center = [1i32, 1, 2, 2];
    let pivot = [1i32, 1];
    let mask = [255u8; 16];
    call!(g, ctx, layers::pixmith_cel_grid(d, buf.as_mut_ptr(), buf.len()));
    call!(g, ctx, layers::pixmith_cel_grid(d, buf.as_mut_ptr(), 1));
    call!(g, ctx, layers::pixmith_cel_grid(d, null_mut(), 5));
    call!(g, ctx, layers::pixmith_cels_clear(d, l, f, f, l));
    call!(g, ctx, layers::pixmith_cels_clear(d, f, l, l, f));
    call!(g, ctx, layers::pixmith_cels_copy(d, l, f, l, f));
    call!(g, ctx, layers::pixmith_cels_copy(d, f, l, f, l));
    call!(g, ctx, layers::pixmith_cels_paste(d, l, f));
    for (dl, df) in [(1, 1), (-1, -1), (i32::MAX, i32::MIN), (0, 0), (-1000, 1000)] {
        call!(g, ctx, layers::pixmith_cels_move(d, l, f, l, f, dl, df, false));
        call!(g, ctx, layers::pixmith_cels_move(d, 0, l, 0, f, dl, df, true));
    }
    call!(g, ctx, tm::pixmith_tileset_duplicate_tile(d, l, f));
    call!(g, ctx, tm::pixmith_tileset_delete_tile(d, l, f));
    call!(g, ctx, tm::pixmith_layer_assign_tileset(d, l, f));
    call!(g, ctx, tm::pixmith_tileset_rename(d, l, name.as_ptr()));
    call!(g, ctx, tm::pixmith_tileset_rename(d, l, null()));
    call!(g, ctx, tm::pixmith_tile_grid_origin(d, l, f, out4.as_mut_ptr()));
    call!(g, ctx, tm::pixmith_tile_grid_origin(d, l, f, null_mut()));
    call!(g, ctx, tm::pixmith_convert_to_tilemap_ex(d, l, f, f, true));
    call!(g, ctx, tm::pixmith_convert_to_tilemap_ex(d, l, 0, 0, false));
    call!(g, ctx, slices::pixmith_slice_name(d, l));
    call!(g, ctx, slices::pixmith_slice_get(d, l, f, out13.as_mut_ptr()));
    call!(g, ctx, slices::pixmith_slice_get(d, l, f, null_mut()));
    call!(g, ctx, slices::pixmith_slice_add(d, 1, 1, l, f));
    call!(g, ctx, slices::pixmith_slice_add(d, i32::MAX, i32::MIN, u32::MAX, u32::MAX));
    call!(g, ctx, slices::pixmith_slice_set_bounds(d, l, f, -3, 4, l, f));
    call!(g, ctx, slices::pixmith_slice_set_bounds(d, l, f, i32::MAX, i32::MAX, u32::MAX, u32::MAX));
    call!(g, ctx, slices::pixmith_slice_set_props(d, l, f, name.as_ptr(), center.as_ptr(), pivot.as_ptr()));
    call!(g, ctx, slices::pixmith_slice_set_props(d, l, f, null(), null(), null()));
    call!(g, ctx, slices::pixmith_slice_remove(d, l));
    for kind in [0u32, 1, 2, 3, 4, 5, 99, u32::MAX] {
        call!(g, ctx, slices::pixmith_userdata_text(d, kind, l, f));
        call!(g, ctx, slices::pixmith_userdata_color(d, kind, l, f, &mut color));
        call!(g, ctx, slices::pixmith_userdata_color(d, kind, l, f, null_mut()));
        call!(g, ctx, slices::pixmith_userdata_set(d, kind, l, f, name.as_ptr(), true, 0xAABBCCDD));
        call!(g, ctx, slices::pixmith_userdata_set(d, kind, l, f, null(), false, 0));
    }
    call!(g, ctx, slices::pixmith_ud_tag_name(d, l));
    call!(g, ctx, palette_edit::pixmith_pe_move(d, idx.as_ptr(), idx.len() as u32, l));
    call!(g, ctx, palette_edit::pixmith_pe_move(d, idx.as_ptr(), 5, f));
    call!(g, ctx, palette_edit::pixmith_pe_move(d, null(), 3, f));
    call!(g, ctx, palette_edit::pixmith_pe_insert(d, l, 0x11223344));
    call!(g, ctx, palette_edit::pixmith_pe_duplicate(d, idx.as_ptr(), idx.len() as u32));
    call!(g, ctx, palette_edit::pixmith_pe_duplicate(d, null(), 3));
    call!(g, ctx, palette_edit::pixmith_pe_remove(d, idx.as_ptr(), idx.len() as u32));
    call!(g, ctx, palette_edit::pixmith_pe_remove(d, null(), 4));
    call!(g, ctx, palette_edit::pixmith_pe_set_live(d, l, 0x01020304, true));
    call!(g, ctx, palette_edit::pixmith_pe_set_live(d, l, 0x01020304, false));
    call!(g, ctx, palette_edit::pixmith_pe_ramp(d, l, f, l.min(300), true));
    call!(g, ctx, palette_edit::pixmith_pe_ramp(d, l, f, f.min(300), false));
    call!(g, ctx, palette_presets::pixmith_pe_preset_name(l));
    call!(g, ctx, palette_presets::pixmith_pe_preset_load(d, l));
    call!(g, ctx, history::pixmith_history_label(d, l));
    call!(g, ctx, history::pixmith_history_goto(d, l));
    call!(g, ctx, font::pixmith_text(d, l, f, 1, 1, name.as_ptr(), 2, 0xFF0000FF, 0, false));
    call!(g, ctx, font::pixmith_text(d, l, f, 1, 1, null(), 2, 0xFF0000FF, 0, true));
    call!(g, ctx, font::pixmith_stamp_mask(d, l, f, 1, 1, 4, 4, mask.as_ptr(), 0xFF, 3, false));
    call!(g, ctx, font::pixmith_stamp_mask(d, l, f, 1, 1, 0, 4, mask.as_ptr(), 0xFF, 0, false));
    call!(g, ctx, font::pixmith_stamp_mask(d, l, f, 1, 1, 4, 4, null(), 0xFF, 0, false));
    call!(g, ctx, transform::pixmith_xf_begin(d, l, f, true));
    call!(g, ctx, transform::pixmith_xf_commit(d));
    call!(g, ctx, transform::pixmith_xf_begin(d, l, f, false));
    call!(g, ctx, transform::pixmith_xf_cancel(d));
    call!(g, ctx, script::pixmith_run_script_at(d, cstr("print('hi')").as_ptr(), l, f, &mut msgs));
    call!(g, ctx, script::pixmith_run_script_at(d, null(), l, f, null_mut()));
    if !msgs.is_null() {
        unsafe { pixmith_string_free(msgs) };
    }
}

fn nasty_f64(i: usize, x: i32) -> f64 {
    [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300, -1e300, 0.0, x as f64, 0.5, f64::MIN_POSITIVE][i % 9]
}

/// Newer entry points taking a canvas position (text, curves, polygons, transform drags).
fn point_calls2(d: Doc, l: u32, f: u32, x: i32, y: i32, g: &mut Failures) {
    let ctx = format!("layer={l} frame={f} at ({x},{y})");
    let name = cstr("Hi!");
    let long = cstr(&"w".repeat(2000));
    let mask = [255u8; 16];
    let curve = [x, y, y, x, 5, 5, far(x), far(y)];
    let poly = [x, y, y, x, 5, 5, 1, far(y), far(x), 2];
    call!(g, ctx, font::pixmith_text(d, l, f, x, y, name.as_ptr(), 1, 0xFF, 0, false));
    call!(g, ctx, font::pixmith_text(d, l, f, x, y, long.as_ptr(), 8, 0xFF, 7, false));
    call!(g, ctx, font::pixmith_text(d, l, f, x, y, name.as_ptr(), 64, 0xFF, u32::MAX, true));
    call!(g, ctx, font::pixmith_text(d, l, f, x, y, name.as_ptr(), 0, 0xFF, 0, false));
    call!(g, ctx, font::pixmith_stamp_mask(d, l, f, x, y, 4, 4, mask.as_ptr(), 0x123456FF, 1, false));
    call!(g, ctx, curves::pixmith_curve(d, l, f, curve.as_ptr(), 0xFF0000FF, 3, 1, false));
    call!(g, ctx, curves::pixmith_curve(d, l, f, curve.as_ptr(), 0xFF0000FF, u32::MAX, u32::MAX, true));
    call!(g, ctx, curves::pixmith_curve(d, l, f, null(), 0xFF0000FF, 3, 1, false));
    call!(g, ctx, curves::pixmith_polygon(d, l, f, poly.as_ptr(), 5, 0xFF0000FF, 2, 3, false));
    call!(g, ctx, curves::pixmith_polygon(d, l, f, poly.as_ptr(), 0, 0xFF0000FF, 2, 3, true));
    call!(g, ctx, curves::pixmith_polygon(d, l, f, null(), 5, 0xFF0000FF, 2, 3, false));
    call!(g, ctx, transform::pixmith_xf_begin(d, l, f, x % 2 == 0));
    for i in 0..9 {
        let (a, b) = (nasty_f64(i, x), nasty_f64(i + 3, y));
        for kind in [0u32, 1, 2, 3, 4, 9] {
            call!(g, ctx, transform::pixmith_xf_drag_begin(d));
            call!(g, ctx, transform::pixmith_xf_drag(d, kind, x.signum(), y.signum(), i % 2 == 0, a, b, b, a));
        }
        for field in 0..9u32 {
            call!(g, ctx, transform::pixmith_xf_set_field(d, field, a));
        }
    }
    call!(g, ctx, transform::pixmith_xf_set_anchor(d, x, y));
    call!(g, ctx, transform::pixmith_xf_set_rotsprite(d, true));
    let mut geo = [0f64; 18];
    call!(g, ctx, transform::pixmith_xf_geometry(d, geo.as_mut_ptr()));
    call!(g, ctx, transform::pixmith_xf_fields(d, geo.as_mut_ptr()));
    call!(g, ctx, transform::pixmith_xf_fields(d, null_mut()));
    call!(g, ctx, transform::pixmith_xf_active(d));
    if x % 2 == 0 {
        call!(g, ctx, transform::pixmith_xf_commit(d));
    } else {
        call!(g, ctx, transform::pixmith_xf_cancel(d));
    }
}

/// Newer entry points without indices.
fn global_calls2(d: Doc, g: &mut Failures) {
    let ctx = String::new();
    let bad = cstr("/nonexistent-dir/pixmith/x.png");
    let name = cstr("label");
    let mut msgs: *mut std::ffi::c_char = null_mut();
    let (mut undo, mut redo) = (0u32, 0u32);
    let mut out3 = [0i32; 3];
    let mut lut = [0u8; 256];
    let mut state = layers::PlayState { reversed: true, passes: u32::MAX, tag: i32::MAX };
    call!(g, ctx, history::pixmith_begin_transaction(d));
    call!(g, ctx, history::pixmith_begin_transaction(d));
    call!(g, ctx, history::pixmith_checkpoint_label(d, name.as_ptr()));
    call!(g, ctx, history::pixmith_checkpoint_label(d, null()));
    call!(g, ctx, history::pixmith_end_transaction(d));
    call!(g, ctx, history::pixmith_end_transaction(d));
    call!(g, ctx, history::pixmith_end_transaction(d));
    call!(g, ctx, history::pixmith_history_counts(d, &mut undo, &mut redo));
    call!(g, ctx, history::pixmith_history_counts(d, null_mut(), null_mut()));
    call!(g, ctx, history::pixmith_history_goto(d, 0));
    call!(g, ctx, history::pixmith_history_goto(d, u32::MAX));
    call!(g, ctx, shapes::pixmith_stroke_style(d, 0xFF, u32::MAX));
    call!(g, ctx, shapes::pixmith_stroke_end_doc(d));
    call!(g, ctx, shapes::pixmith_rollback(d));
    call!(g, ctx, cm::pixmith_cm_conform_all(d));
    call!(g, ctx, layers::pixmith_play_start(&mut state));
    call!(g, ctx, layers::pixmith_play_start(null_mut()));
    call!(g, ctx, layers::pixmith_play_next(d, 0, &mut state, true));
    call!(g, ctx, layers::pixmith_play_next(d, u32::MAX, &mut state, true));
    call!(g, ctx, layers::pixmith_tag_count(d));
    call!(g, ctx, slices::pixmith_slice_count(d));
    call!(g, ctx, slices::pixmith_ud_tag_count(d));
    call!(g, ctx, palette_presets::pixmith_pe_preset_count());
    call!(g, ctx, font::pixmith_text_size(null(), 1, out3.as_mut_ptr()));
    call!(g, ctx, font::pixmith_text_size(name.as_ptr(), 64, out3.as_mut_ptr()));
    call!(g, ctx, font::pixmith_text_size(name.as_ptr(), 1, null_mut()));
    call!(g, ctx, filters::pixmith_curve_lut(null(), 0, lut.as_mut_ptr()));
    call!(g, ctx, filters::pixmith_curve_lut(null(), 5, lut.as_mut_ptr()));
    call!(g, ctx, filters::pixmith_curve_lut([0i32, 0, 255, 255, 128, 300, -5, 40].as_ptr(), 4, lut.as_mut_ptr()));
    call!(g, ctx, filters::pixmith_curve_lut([0i32, 0].as_ptr(), 1, null_mut()));
    // Scripts: every bundled example, plus broken sources and API misuse.
    let n = script::pixmith_example_script_count();
    for i in (0..n).chain([n, u32::MAX]) {
        call!(g, format!("example {i}"), script::pixmith_example_script_name(i));
        let src = script::pixmith_example_script_source(i);
        if !src.is_null() {
            call!(g, format!("example {i}"), script::pixmith_run_script(d, src, &mut msgs));
            unsafe { pixmith_string_free(src) };
            if !msgs.is_null() {
                unsafe { pixmith_string_free(msgs) };
                msgs = null_mut();
            }
        }
    }
    for src in [
        "",
        "(",
        "error('boom')",
        "local s = app.activeSprite; s:newLayer(); s:deleteLayer(99)",
        "local i = Image(70000, 70000)",
        "local i = Image(0, 0)",
        "local s = app.activeSprite; s:resize(0, 0)",
        "local s = app.activeSprite; s:resize(100000, 100000)",
        "local s = app.activeSprite; s:newFrame(); s:deleteFrame(-5); s:deleteFrame(1000)",
        "return string.rep('x', 1 << 40)",
    ] {
        let c = cstr(src);
        call!(g, format!("script {src:?}"), script::pixmith_run_script(d, c.as_ptr(), &mut msgs));
        call!(g, format!("script {src:?}"), script::pixmith_run_script(d, c.as_ptr(), null_mut()));
        if !msgs.is_null() {
            unsafe { pixmith_string_free(msgs) };
            msgs = null_mut();
        }
    }
    call!(g, ctx, script::pixmith_run_script(d, null(), &mut msgs));
    // Files.
    for scale in [0, 1, 64, u32::MAX] {
        call!(g, ctx, animation::pixmith_export_apng(d, scale, bad.as_ptr()));
        call!(g, ctx, animation::pixmith_export_apng(d, scale, null()));
    }
    for p in [null(), bad.as_ptr()] {
        call!(g, ctx, animation::pixmith_import_gif(p));
        call!(g, ctx, sheet_import::pixmith_import_sheet_json(p, p));
        call!(g, ctx, sheet_import::pixmith_import_sheet_json(bad.as_ptr(), p));
    }
}

#[test]
fn indexed_calls_with_bad_indices() {
    let _g = serial();
    install_hook();
    let mut g = Failures(Vec::new());
    for make in [populated as fn() -> Doc, tiny] {
        for l in U {
            for f in U {
                let d = make();
                indexed_calls(d, l, f, &mut g);
                indexed_calls2(d, l, f, &mut g);
                check_doc(d, &format!("indexed layer={l} frame={f}"), &mut g);
                unsafe { pixmith_core::ffi::document::pixmith_free(d) };
            }
        }
    }
    let _ = std::panic::take_hook();
    assert_modest_allocations("indexed calls");
    report(g.0);
}

#[test]
fn point_calls_with_bad_coordinates() {
    let _g = serial();
    install_hook();
    let mut g = Failures(Vec::new());
    for make in [populated as fn() -> Doc, tiny] {
        for l in [0u32, 1, 2, 3, 99, u32::MAX] {
            for f in [0u32, 1, 2, 50, u32::MAX] {
                for (x, y) in PTS {
                    let d = make();
                    point_calls(d, l, f, x, y, &mut g);
                    point_calls2(d, l, f, x, y, &mut g);
                    check_doc(d, &format!("points layer={l} frame={f} ({x},{y})"), &mut g);
                    unsafe { pixmith_core::ffi::document::pixmith_free(d) };
                }
            }
        }
    }
    let _ = std::panic::take_hook();
    assert_modest_allocations("point calls");
    report(g.0);
}

#[test]
fn global_calls_with_bad_arguments() {
    let _g = serial();
    install_hook();
    let mut g = Failures(Vec::new());
    for make in [populated as fn() -> Doc, tiny] {
        let d = make();
        global_calls(d, &mut g);
        global_calls2(d, &mut g);
        check_doc(d, "global calls", &mut g);
        unsafe { pixmith_core::ffi::document::pixmith_free(d) };
    }
    let _ = std::panic::take_hook();
    assert_modest_allocations("global calls");
    report(g.0);
}

#[test]
fn null_document_everywhere() {
    let _g = serial();
    install_hook();
    let mut g = Failures(Vec::new());
    let d: Doc = null_mut();
    indexed_calls(d, 0, 0, &mut g);
    point_calls(d, 0, 0, 1, 1, &mut g);
    global_calls(d, &mut g);
    global_calls2(d, &mut g);
    indexed_calls2(d, 0, 0, &mut g);
    point_calls2(d, 0, 0, 1, 1, &mut g);
    let _ = std::panic::take_hook();
    report(g.0);
}

#[test]
fn new_with_degenerate_sizes() {
    let _g = serial();
    for (w, h) in [(0, 0), (0, 5), (5, 0), (u32::MAX, u32::MAX), (65535, 65535), (65535, 1)] {
        let d = pixmith_new(w, h);
        assert!(!d.is_null());
        let (sw, sh) =
            unsafe { (pixmith_core::ffi::document::pixmith_width(d), pixmith_core::ffi::document::pixmith_height(d)) };
        assert!(
            sw >= 1 && sh >= 1 && sw as usize * sh as usize <= pixmith_core::document::MAX_CANVAS_PIXELS,
            "{w}x{h} -> {sw}x{sh}"
        );
        unsafe { pixmith_core::ffi::document::pixmith_free(d) };
    }
    assert_modest_allocations("pixmith_new");
}

#[test]
fn opening_garbage_files_reports_errors() {
    let _g = serial();
    let dir = std::env::temp_dir().join(format!("pixmith-robustness-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut rng = Rng(77);
    let mut bytes = pixmith_core::formats::ase::write(&rgba_sprite());
    for i in 0..40 {
        let at = rng.below(bytes.len());
        bytes[at] = rng.next() as u8;
        let path = dir.join(format!("f{i}.aseprite"));
        std::fs::write(&path, &bytes[..bytes.len() - i]).unwrap();
        let c = cstr(path.to_str().unwrap());
        let d = unsafe { pixmith_core::ffi::document::pixmith_open(c.as_ptr()) };
        if d.is_null() {
            let err = unsafe { std::ffi::CStr::from_ptr(pixmith_core::ffi::pixmith_last_error()) };
            assert!(!err.to_bytes().is_empty());
        } else {
            unsafe { pixmith_core::ffi::document::pixmith_free(d) };
        }
    }
    // PNG import of garbage and of a truncated real PNG.
    let png = export::frame_png(&rgba_sprite(), 0, 1).unwrap();
    for (i, data) in [vec![], vec![0x89, b'P', b'N', b'G'], png[..png.len() / 2].to_vec(), png].into_iter().enumerate()
    {
        let path = dir.join(format!("p{i}.png"));
        std::fs::write(&path, data).unwrap();
        let c = cstr(path.to_str().unwrap());
        let d = unsafe { import::pixmith_import_png(c.as_ptr()) };
        if !d.is_null() {
            unsafe { pixmith_core::ffi::document::pixmith_free(d) };
        }
        let d = unsafe { import::pixmith_import_sheet(c.as_ptr(), 3, 3) };
        if !d.is_null() {
            unsafe { pixmith_core::ffi::document::pixmith_free(d) };
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    assert_modest_allocations("garbage files");
}
