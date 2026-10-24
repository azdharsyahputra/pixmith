//! `pixmith_*` entry points for the document as a whole: create, open, save, export, layer and frame
//! basics, rendering, undo and the palette.

use super::{clampc, editor, editor_quiet, path_arg, save_to, set_error, sprite};
use crate::document::{Editor, Sprite};
use crate::formats::{ase, export, pxmt};
use crate::paint::{selection, shapes};
use std::ffi::{c_char, CString};
use std::ptr::null_mut;

#[no_mangle]
pub extern "C" fn pixmith_new(width: u32, height: u32) -> *mut Editor {
    crate::ffi::guarded("pixmith_new", std::ptr::null_mut(), || {
        Box::into_raw(Box::new(Editor::new(Sprite::new(width, height))))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_open(path: *const c_char) -> *mut Editor {
    crate::ffi::guarded("pixmith_open", std::ptr::null_mut(), || {
        let Some(path) = path_arg(path) else {
            set_error("invalid path");
            return null_mut();
        };
        // .pxmt is recognized by its magic bytes, anything else is read as .aseprite.
        match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| pxmt::read_document(&b)) {
            Ok(s) => Box::into_raw(Box::new(Editor::new(s))),
            Err(e) => {
                set_error(format!("{path}: {e}"));
                null_mut()
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_free(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_free", doc, || {
        if !doc.is_null() {
            shapes::forget_stroke(doc);
            drop(Box::from_raw(doc));
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_save(doc: *const Editor, path: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_save", std::ptr::null_mut(), || {
        let Some(s) = editor_quiet(doc as *mut Editor).map(|e| &e.sprite) else { return false };
        let Some(p) = path_arg(path) else {
            set_error("invalid path");
            return false;
        };
        // .pxmt when the path ends in ".pxmt", .aseprite otherwise.
        save_to(path, pxmt::encode_for_path(&p, s))
    })
}

/// Writes the document as .aseprite whatever the extension says.
#[no_mangle]
pub unsafe extern "C" fn pixmith_save_aseprite(doc: *const Editor, path: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_save_aseprite", std::ptr::null_mut(), || {
        let Some(s) = editor_quiet(doc as *mut Editor).map(|e| &e.sprite) else { return false };
        save_to(path, Ok(ase::write(s)))
    })
}

/// A preview of a document file fitted inside `max_w` x `max_h`; `out` holds `max_w * max_h * 4` bytes.
#[no_mangle]
pub unsafe extern "C" fn pixmith_file_thumbnail(
    path: *const c_char,
    max_w: u32,
    max_h: u32,
    out: *mut u8,
    len: usize,
    w: *mut u32,
    h: *mut u32,
) -> bool {
    crate::ffi::guarded("pixmith_file_thumbnail", std::ptr::null_mut(), || {
        let Some(path) = path_arg(path) else {
            set_error("invalid path");
            return false;
        };
        if out.is_null() {
            set_error("no output buffer");
            return false;
        }
        match pxmt::file_thumbnail(&path, max_w, max_h) {
            Ok(t) if t.rgba.len() <= len => {
                std::ptr::copy_nonoverlapping(t.rgba.as_ptr(), out, t.rgba.len());
                if !w.is_null() {
                    *w = t.width;
                }
                if !h.is_null() {
                    *h = t.height;
                }
                true
            }
            Ok(_) => {
                set_error("the output buffer is too small for the thumbnail");
                false
            }
            Err(e) => {
                set_error(e);
                false
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_export_png(doc: *const Editor, frame: u32, scale: u32, path: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_export_png", std::ptr::null_mut(), || {
        let Some(s) = sprite(doc) else { return false };
        save_to(path, export::frame_png(s, frame as usize, scale as usize))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_export_sheet(doc: *const Editor, scale: u32, path: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_export_sheet", std::ptr::null_mut(), || {
        let Some(s) = sprite(doc) else { return false };
        save_to(path, export::sheet_png(s, scale as usize))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_width(doc: *const Editor) -> u32 {
    crate::ffi::guarded("pixmith_width", std::ptr::null_mut(), || sprite(doc).map_or(0, |s| s.width))
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_height(doc: *const Editor) -> u32 {
    crate::ffi::guarded("pixmith_height", std::ptr::null_mut(), || sprite(doc).map_or(0, |s| s.height))
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_count(doc: *const Editor) -> u32 {
    crate::ffi::guarded("pixmith_layer_count", std::ptr::null_mut(), || {
        sprite(doc).map_or(0, |s| s.layers.len() as u32)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_frame_count(doc: *const Editor) -> u32 {
    crate::ffi::guarded("pixmith_frame_count", std::ptr::null_mut(), || {
        sprite(doc).map_or(0, |s| s.frame_count() as u32)
    })
}

/// Returns a string to release with `pixmith_string_free`, or null.
#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_name(doc: *const Editor, layer: u32) -> *mut c_char {
    crate::ffi::guarded("pixmith_layer_name", std::ptr::null_mut(), || {
        sprite(doc)
            .and_then(|s| s.layers.get(layer as usize))
            .and_then(|l| CString::new(l.name.replace('\0', "")).ok())
            .map_or(null_mut(), CString::into_raw)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_visible(doc: *const Editor, layer: u32) -> bool {
    crate::ffi::guarded("pixmith_layer_visible", std::ptr::null_mut(), || {
        sprite(doc).and_then(|s| s.layers.get(layer as usize)).is_some_and(|l| l.visible)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_is_group(doc: *const Editor, layer: u32) -> bool {
    crate::ffi::guarded("pixmith_layer_is_group", std::ptr::null_mut(), || {
        sprite(doc).and_then(|s| s.layers.get(layer as usize)).is_some_and(|l| l.is_group())
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_level(doc: *const Editor, layer: u32) -> u32 {
    crate::ffi::guarded("pixmith_layer_level", std::ptr::null_mut(), || {
        sprite(doc).and_then(|s| s.layers.get(layer as usize)).map_or(0, |l| l.child_level as u32)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_opacity(doc: *const Editor, layer: u32) -> u8 {
    crate::ffi::guarded("pixmith_layer_opacity", std::ptr::null_mut(), || {
        sprite(doc).and_then(|s| s.layers.get(layer as usize)).map_or(0, |l| l.opacity)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_set_layer_visible(doc: *mut Editor, layer: u32, visible: bool) {
    crate::ffi::guarded("pixmith_set_layer_visible", doc, || {
        if let Some(l) = editor(doc).and_then(|e| e.sprite.layers.get_mut(layer as usize)) {
            l.visible = visible;
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_set_layer_opacity(doc: *mut Editor, layer: u32, opacity: u8) {
    crate::ffi::guarded("pixmith_set_layer_opacity", doc, || {
        if let Some(l) = editor(doc).and_then(|e| e.sprite.layers.get_mut(layer as usize)) {
            l.opacity = opacity;
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_rename_layer(doc: *mut Editor, layer: u32, name: *const c_char) {
    crate::ffi::guarded("pixmith_rename_layer", doc, || {
        let (Some(e), Some(name)) = (editor(doc), path_arg(name)) else { return };
        if e.sprite.layers.get(layer as usize).is_some_and(|l| l.name != name) {
            e.checkpoint();
            e.sprite.layers[layer as usize].name = name;
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_add_layer(doc: *mut Editor, above: u32) -> u32 {
    crate::ffi::guarded("pixmith_add_layer", doc, || {
        let Some(e) = editor(doc) else { return 0 };
        e.checkpoint();
        e.sprite.add_layer(above as usize) as u32
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_remove_layer(doc: *mut Editor, layer: u32) -> bool {
    crate::ffi::guarded("pixmith_remove_layer", doc, || {
        let Some(e) = editor(doc) else { return false };
        let before = e.sprite.clone();
        if e.sprite.remove_layer(layer as usize) {
            let after = std::mem::replace(&mut e.sprite, before);
            e.checkpoint();
            e.sprite = after;
            return true;
        }
        false
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_add_frame(doc: *mut Editor, after: u32, duplicate: bool) -> u32 {
    crate::ffi::guarded("pixmith_add_frame", doc, || {
        let Some(e) = editor(doc) else { return 0 };
        e.checkpoint();
        e.sprite.add_frame(after as usize, duplicate) as u32
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_remove_frame(doc: *mut Editor, frame: u32) -> bool {
    crate::ffi::guarded("pixmith_remove_frame", doc, || {
        let Some(e) = editor(doc) else { return false };
        if e.sprite.frame_count() <= 1 || frame as usize >= e.sprite.frame_count() {
            return false;
        }
        e.checkpoint();
        e.sprite.remove_frame(frame as usize)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_frame_duration(doc: *const Editor, frame: u32) -> u16 {
    crate::ffi::guarded("pixmith_frame_duration", std::ptr::null_mut(), || {
        sprite(doc).and_then(|s| s.durations.get(frame as usize).copied()).unwrap_or(0)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_set_frame_duration(doc: *mut Editor, frame: u32, ms: u16) {
    crate::ffi::guarded("pixmith_set_frame_duration", doc, || {
        if let Some(d) = editor(doc).and_then(|e| e.sprite.durations.get_mut(frame as usize)) {
            *d = ms.max(1);
        }
    })
}

/// Composites a frame into `out` (width * height * 4 bytes, straight-alpha RGBA), as the editor shows it:
/// reference layers are drawn at `pixmith_reference_display` percent.
#[no_mangle]
pub unsafe extern "C" fn pixmith_render(doc: *const Editor, frame: u32, out: *mut u8, len: usize) -> bool {
    crate::ffi::guarded("pixmith_render", std::ptr::null_mut(), || {
        let Some(e) = crate::ffi::synced(doc) else { return false };
        // The editor's view: reference layers show at the document's reference opacity (exports never do).
        let s = &e.sprite;
        let rect = (0, 0, s.width as usize, s.height as usize);
        let px = crate::document::layers::render_frame_rect_with(s, frame as usize, rect, e.reference_display);
        if out.is_null() || len < px.len() {
            return false;
        }
        std::ptr::copy_nonoverlapping(px.as_ptr(), out, px.len());
        true
    })
}

/// Starts an undoable edit; call once before a stroke or fill.
#[no_mangle]
pub unsafe extern "C" fn pixmith_begin_stroke(doc: *mut Editor) {
    crate::ffi::guarded("pixmith_begin_stroke", doc, || {
        if let Some(e) = editor_quiet(doc) {
            e.begin_preview();
        }
    })
}

/// Colors are 0xRRGGBBAA. A zero-alpha color erases.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_draw_line(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    rgba: u32,
    size: u32,
) {
    crate::ffi::guarded("pixmith_draw_line", doc, || {
        if let Some(e) = editor_quiet(doc) {
            let shared = e.cel_shared(layer as usize, frame as usize);
            selection::draw_line(
                e,
                layer as usize,
                frame as usize,
                (clampc(x0), clampc(y0)),
                (clampc(x1), clampc(y1)),
                rgba.to_be_bytes(),
                size,
            );
            let (x0, y0, x1, y1) = (clampc(x0), clampc(y0), clampc(x1), clampc(y1));
            let pad = size.clamp(1, 64) as i32;
            let r = (x0.min(x1) - pad, y0.min(y1) - pad, x0.max(x1) + pad + 1, y0.max(y1) + pad + 1);
            e.mark_cel_rect(layer as usize, frame as usize, r, shared);
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_fill(doc: *mut Editor, layer: u32, frame: u32, x: i32, y: i32, rgba: u32) {
    crate::ffi::guarded("pixmith_fill", doc, || {
        if let Some(e) = editor(doc) {
            selection::fill(e, layer as usize, frame as usize, x, y, rgba.to_be_bytes());
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_undo(doc: *mut Editor) -> bool {
    crate::ffi::guarded("pixmith_undo", doc, || editor(doc).is_some_and(|e| e.undo()))
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_redo(doc: *mut Editor) -> bool {
    crate::ffi::guarded("pixmith_redo", doc, || editor(doc).is_some_and(|e| e.redo()))
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_palette_count(doc: *const Editor) -> u32 {
    crate::ffi::guarded("pixmith_palette_count", std::ptr::null_mut(), || {
        sprite(doc).map_or(0, |s| s.palette.len() as u32)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_palette_color(doc: *const Editor, index: u32) -> u32 {
    crate::ffi::guarded("pixmith_palette_color", std::ptr::null_mut(), || {
        sprite(doc).and_then(|s| s.palette.get(index as usize)).map_or(0, |c| u32::from_be_bytes(*c))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_palette_add(doc: *mut Editor, rgba: u32) {
    crate::ffi::guarded("pixmith_palette_add", doc, || {
        if let Some(e) = editor(doc) {
            let c = rgba.to_be_bytes();
            if !e.sprite.palette.contains(&c) && e.sprite.palette.len() < 256 {
                e.checkpoint();
                e.sprite.palette.push(c);
            }
        }
    })
}
