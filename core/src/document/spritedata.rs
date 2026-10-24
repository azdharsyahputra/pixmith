//! Sprite-level data that the `.aseprite` format carries and Aseprite shows in its dialogs: the color
//! profile (chunk 0x2007), cel z-indexes and reference layers (layer flag 0x40), with their C ABI.
//!
//! None of it changes pixels. The color profile is kept and written back as read (Aseprite does not convert
//! pixels either), the z-index only changes the drawing order in `layers::drawing_order`, and reference layers
//! are left out of every render except the editor's (`Editor::reference_display`).

use crate::document::{Cel, Editor, Layer, Sprite, LAYER_EDITABLE, LAYER_REFERENCE};
use crate::ffi::{editor, path_arg, set_error, synced};
use std::ffi::{c_char, CString};
use std::ptr::null_mut;
use std::sync::Arc;

// MARK: Color profile

/// The color profile chunk of a sprite. `flags` and `gamma` (a 16.16 fixed point number, only meaningful when
/// `flags & 1`) are kept as read so that the chunk is written back unchanged.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ColorProfile {
    /// No chunk, or a chunk of type 0.
    #[default]
    None,
    Srgb {
        flags: u16,
        gamma: u32,
    },
    /// An embedded ICC profile; `data` is the raw profile.
    Icc {
        data: Arc<Vec<u8>>,
        flags: u16,
        gamma: u32,
    },
}

/// Largest ICC profile read from a file (real ones are a few KB; some are a few MB).
pub const MAX_ICC_BYTES: usize = 16 << 20;

impl ColorProfile {
    pub const SRGB: ColorProfile = ColorProfile::Srgb { flags: 0, gamma: 0 };

    /// 0 none, 1 sRGB, 2 embedded ICC.
    pub fn kind(&self) -> u32 {
        match self {
            ColorProfile::None => 0,
            ColorProfile::Srgb { .. } => 1,
            ColorProfile::Icc { .. } => 2,
        }
    }

    /// The text both front ends show: "None", "sRGB" or "Embedded ICC (name)".
    pub fn label(&self) -> String {
        match self {
            ColorProfile::None => "None".into(),
            ColorProfile::Srgb { .. } => "sRGB".into(),
            ColorProfile::Icc { data, .. } => {
                format!("Embedded ICC ({})", icc_description(data).unwrap_or_else(|| "unnamed".into()))
            }
        }
    }

    /// The body of a color profile chunk, or `None` when there is nothing to write.
    pub fn chunk_body(&self) -> Option<Vec<u8>> {
        let (kind, flags, gamma, icc) = match self {
            ColorProfile::None => return None,
            ColorProfile::Srgb { flags, gamma } => (1u16, *flags, *gamma, None),
            ColorProfile::Icc { data, flags, gamma } => (2u16, *flags, *gamma, Some(data)),
        };
        let mut b = Vec::new();
        b.extend(kind.to_le_bytes());
        b.extend(flags.to_le_bytes());
        b.extend(gamma.to_le_bytes());
        b.extend([0; 8]);
        if let Some(data) = icc {
            b.extend((data.len() as u32).to_le_bytes());
            b.extend(data.iter());
        }
        Some(b)
    }

    /// Parses a color profile chunk body. Unknown types and truncated or oversized ICC data read as `None`.
    pub fn from_chunk(body: &[u8]) -> ColorProfile {
        let u16_at = |at: usize| body.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
        let u32_at = |at: usize| body.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let (Some(kind), Some(flags), Some(gamma)) = (u16_at(0), u16_at(2), u32_at(4)) else {
            return ColorProfile::None;
        };
        match kind {
            1 => ColorProfile::Srgb { flags, gamma },
            2 => {
                let Some(len) = u32_at(16).map(|l| l as usize) else { return ColorProfile::None };
                match body.get(20..20usize.saturating_add(len)) {
                    Some(data) if len > 0 && len <= MAX_ICC_BYTES => {
                        ColorProfile::Icc { data: Arc::new(data.to_vec()), flags, gamma }
                    }
                    _ => ColorProfile::None,
                }
            }
            _ => ColorProfile::None,
        }
    }
}

/// The human readable name of an ICC profile: its `desc` tag, in the version 2 (`desc`) or version 4
/// (`mluc`) layout.
pub fn icc_description(icc: &[u8]) -> Option<String> {
    let be32 = |at: usize| icc.get(at..at.checked_add(4)?).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let count = be32(128)? as usize;
    for i in 0..count.min(256) {
        let entry = 132 + i * 12;
        if icc.get(entry..entry + 4)? != b"desc" {
            continue;
        }
        let (offset, size) = (be32(entry + 4)? as usize, be32(entry + 8)? as usize);
        let tag = icc.get(offset..offset.checked_add(size)?)?;
        let name = match tag.get(..4)? {
            b"desc" => {
                let len = u32::from_be_bytes(tag.get(8..12)?.try_into().ok()?) as usize;
                let text = tag.get(12..12usize.checked_add(len)?)?;
                String::from_utf8_lossy(text.split(|&c| c == 0).next()?).into_owned()
            }
            b"mluc" => {
                // First record: language, country, length, offset (from the tag start); UTF-16BE text.
                let len = u32::from_be_bytes(tag.get(20..24)?.try_into().ok()?) as usize;
                let at = u32::from_be_bytes(tag.get(24..28)?.try_into().ok()?) as usize;
                let text = tag.get(at..at.checked_add(len)?)?;
                let units: Vec<u16> = text.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                String::from_utf16_lossy(&units)
            }
            _ => return None,
        };
        let name = name.trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string();
        return (!name.is_empty()).then_some(name);
    }
    None
}

// MARK: Cel z-index and reference layers on the sprite

impl Sprite {
    /// The z-index of a cel, or `None` when the cel is empty.
    pub fn cel_z_index(&self, layer: usize, frame: usize) -> Option<i16> {
        self.cels.get(frame)?.get(layer)?.as_ref().map(|c| c.z_index)
    }

    /// Sets a cel's z-index, clamped to what the file format can store (shared by linked cels). Returns false
    /// if the cel is empty.
    pub fn set_cel_z_index(&mut self, layer: usize, frame: usize, z: i32) -> bool {
        if !matches!(self.cels.get(frame).and_then(|r| r.get(layer)), Some(Some(_))) {
            return false;
        }
        match self.cel_mut_any(layer, frame) {
            Some(c) => c.z_index = z.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            None => return false,
        }
        self.flush_links();
        true
    }

    /// Turns a layer into a reference layer, or back into a normal one. A new reference layer is locked (it
    /// cannot be drawn on until unlocked). Groups cannot be reference layers.
    pub fn set_layer_reference(&mut self, layer: usize, on: bool) -> bool {
        let Some(l) = self.layers.get_mut(layer).filter(|l| !l.is_group()) else { return false };
        if l.is_reference() != on {
            l.flags = if on {
                (l.flags | LAYER_REFERENCE) & !LAYER_EDITABLE
            } else {
                (l.flags & !LAYER_REFERENCE) | LAYER_EDITABLE
            };
        }
        true
    }

    /// Locks or unlocks a layer (the "editable" layer flag). Only reference layers refuse drawing while locked.
    pub fn set_layer_locked(&mut self, layer: usize, locked: bool) -> bool {
        let Some(l) = self.layers.get_mut(layer).filter(|l| !l.is_group()) else { return false };
        l.flags = if locked { l.flags & !LAYER_EDITABLE } else { l.flags | LAYER_EDITABLE };
        true
    }
}

impl Layer {
    /// A locked, reference layer named `name`.
    pub fn new_reference(name: impl Into<String>) -> Layer {
        Layer { flags: LAYER_REFERENCE, ..Layer::new(name) }
    }
}

// MARK: Reference layer from an image

/// Scales straight-alpha RGBA `src` (`sw` x `sh`) to fit inside `cw` x `ch` keeping its aspect ratio, centered
/// on a transparent canvas-sized buffer. Shrinking averages the covered pixels (weighted by alpha), growing
/// repeats pixels so that pixel art stays sharp.
pub fn fit_to_canvas(src: &[u8], sw: usize, sh: usize, cw: usize, ch: usize) -> Vec<u8> {
    let mut out = vec![0u8; cw * ch * 4];
    if sw == 0 || sh == 0 || cw == 0 || ch == 0 || src.len() < sw * sh * 4 {
        return out;
    }
    // Scale factor as a ratio: the largest that fits (can be above 1).
    let (dw, dh) = if sw * ch >= sh * cw {
        (cw, ((sh * cw + sw / 2) / sw).clamp(1, ch))
    } else {
        (((sw * ch + sh / 2) / sh).clamp(1, cw), ch)
    };
    let (ox, oy) = ((cw - dw) / 2, (ch - dh) / 2);
    for dy in 0..dh {
        let (y0, y1) = (dy * sh / dh, ((dy + 1) * sh).div_ceil(dh).clamp(dy * sh / dh + 1, sh));
        for dx in 0..dw {
            let (x0, x1) = (dx * sw / dw, ((dx + 1) * sw).div_ceil(dw).clamp(dx * sw / dw + 1, sw));
            let (mut r, mut g, mut b, mut a, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = &src[(y * sw + x) * 4..][..4];
                    let alpha = p[3] as u64;
                    r += p[0] as u64 * alpha;
                    g += p[1] as u64 * alpha;
                    b += p[2] as u64 * alpha;
                    a += alpha;
                    n += 1;
                }
            }
            if let (Some(r), Some(g), Some(b), Some(alpha)) =
                (r.checked_div(a), g.checked_div(a), b.checked_div(a), a.checked_div(n))
            {
                let d = &mut out[((oy + dy) * cw + ox + dx) * 4..][..4];
                d.copy_from_slice(&[r as u8, g as u8, b as u8, alpha as u8]);
            }
        }
    }
    out
}

/// Adds a reference layer above `above` holding the image scaled to fit the canvas (undoable). The image is the
/// cel of every frame (the frames share one buffer, like linked cels). Returns the new layer's index.
pub fn add_reference_layer(
    e: &mut Editor,
    above: usize,
    name: &str,
    rgba: &[u8],
    (w, h): (usize, usize),
) -> Result<usize, String> {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return Err("the image is empty".into());
    }
    e.checkpoint();
    let s = &mut e.sprite;
    let at = s.add_layer(above);
    s.layers[at] = Layer { name: name.to_string(), child_level: s.layers[at].child_level, ..Layer::new_reference("") };
    let pixels = fit_to_canvas(rgba, w, h, s.width as usize, s.height as usize);
    let cel = Arc::new(Cel { opacity: 255, pixels, ..Default::default() });
    for row in &mut s.cels {
        row[at] = Some(cel.clone());
    }
    Ok(at)
}

// MARK: C ABI

fn cstring(text: String) -> *mut c_char {
    CString::new(text.replace('\0', "")).map_or(null_mut(), CString::into_raw)
}

/// 0 none, 1 sRGB, 2 embedded ICC; -1 for an invalid document.
#[no_mangle]
pub unsafe extern "C" fn pixmith_color_profile_kind(doc: *const Editor) -> i32 {
    crate::ffi::guarded("pixmith_color_profile_kind", std::ptr::null_mut(), || {
        synced(doc).map_or(-1, |e| e.sprite.color_profile.kind() as i32)
    })
}

/// "None", "sRGB" or "Embedded ICC (name)". Free with `pixmith_string_free`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_color_profile_label(doc: *const Editor) -> *mut c_char {
    crate::ffi::guarded("pixmith_color_profile_label", std::ptr::null_mut(), || {
        synced(doc).map_or(null_mut(), |e| cstring(e.sprite.color_profile.label()))
    })
}

/// Size of the embedded ICC profile in bytes (0 when there is none).
#[no_mangle]
pub unsafe extern "C" fn pixmith_color_profile_icc_size(doc: *const Editor) -> u32 {
    crate::ffi::guarded("pixmith_color_profile_icc_size", std::ptr::null_mut(), || match synced(doc) {
        Some(Editor { sprite: Sprite { color_profile: ColorProfile::Icc { data, .. }, .. }, .. }) => data.len() as u32,
        _ => 0,
    })
}

/// Assigns a color profile as one undo step: 0 removes it, 1 assigns sRGB. Pixels are not converted. An embedded
/// ICC profile cannot be assigned, only kept. Returns false for another kind or when nothing changes.
#[no_mangle]
pub unsafe extern "C" fn pixmith_color_profile_set(doc: *mut Editor, kind: u32) -> bool {
    crate::ffi::guarded("pixmith_color_profile_set", doc, || {
        let Some(e) = editor(doc) else { return false };
        let new = match kind {
            0 => ColorProfile::None,
            1 => ColorProfile::SRGB,
            _ => return false,
        };
        if e.sprite.color_profile == new {
            return false;
        }
        e.checkpoint();
        e.sprite.color_profile = new;
        true
    })
}

/// The z-index of a cel. Returns false (and leaves `out` alone) when the cel is empty.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cel_z_index(doc: *const Editor, layer: u32, frame: u32, out: *mut i32) -> bool {
    crate::ffi::guarded("pixmith_cel_z_index", std::ptr::null_mut(), || {
        let z = synced(doc).and_then(|e| e.sprite.cel_z_index(layer as usize, frame as usize));
        match (z, out.is_null()) {
            (Some(z), false) => {
                *out = z as i32;
                true
            }
            (Some(_), true) => true,
            _ => false,
        }
    })
}

/// Sets a cel's z-index (clamped to -32768..=32767; linked cels share it) as one undo step.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cel_set_z_index(doc: *mut Editor, layer: u32, frame: u32, z: i32) -> bool {
    crate::ffi::guarded("pixmith_cel_set_z_index", doc, || {
        let Some(e) = editor(doc) else { return false };
        crate::document::layers::undoable(e, |s| s.set_cel_z_index(layer as usize, frame as usize, z).then_some(()))
            .is_some()
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_is_reference(doc: *const Editor, layer: u32) -> bool {
    crate::ffi::guarded("pixmith_layer_is_reference", std::ptr::null_mut(), || {
        synced(doc).and_then(|e| e.sprite.layers.get(layer as usize)).is_some_and(|l| l.is_reference())
    })
}

/// Turns a layer into a reference layer (locked) or back into a normal one, as one undo step.
#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_set_reference(doc: *mut Editor, layer: u32, on: bool) -> bool {
    crate::ffi::guarded("pixmith_layer_set_reference", doc, || {
        let Some(e) = editor(doc) else { return false };
        crate::document::layers::undoable(e, |s| s.set_layer_reference(layer as usize, on).then_some(())).is_some()
    })
}

/// Whether the layer's editable flag is clear. Drawing is refused only on locked reference layers.
#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_is_locked(doc: *const Editor, layer: u32) -> bool {
    crate::ffi::guarded("pixmith_layer_is_locked", std::ptr::null_mut(), || {
        synced(doc).and_then(|e| e.sprite.layers.get(layer as usize)).is_some_and(|l| l.is_locked())
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_set_locked(doc: *mut Editor, layer: u32, locked: bool) -> bool {
    crate::ffi::guarded("pixmith_layer_set_locked", doc, || {
        let Some(e) = editor(doc) else { return false };
        crate::document::layers::undoable(e, |s| s.set_layer_locked(layer as usize, locked).then_some(())).is_some()
    })
}

/// New Reference Layer from File: imports the image at `path`, scaled to fit the canvas, into a new locked
/// reference layer above `above` (undoable). Returns the layer index or -1.
#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_new_reference(doc: *mut Editor, above: u32, path: *const c_char) -> i32 {
    crate::ffi::guarded("pixmith_layer_new_reference", doc, || {
        let Some(e) = editor(doc) else { return -1 };
        let result = (|| {
            let path = path_arg(path).ok_or("invalid path")?;
            let bytes = std::fs::read(&path).map_err(|m| format!("{path}: {m}"))?;
            let (w, h, rgba) = crate::formats::import::decode_png(&bytes).map_err(|m| format!("{path}: {m}"))?;
            let name = std::path::Path::new(&path)
                .file_stem()
                .map_or("Reference".to_string(), |n| n.to_string_lossy().into_owned());
            add_reference_layer(e, above as usize, &name, &rgba, (w, h))
        })();
        match result {
            Ok(i) => i as i32,
            Err(m) => {
                set_error(m);
                -1
            }
        }
    })
}

/// Like `pixmith_layer_new_reference` for an image the caller has already decoded: `rgba` is `w * h * 4` bytes of
/// straight-alpha RGBA, `name` the layer's name.
#[no_mangle]
pub unsafe extern "C" fn pixmith_layer_new_reference_rgba(
    doc: *mut Editor,
    above: u32,
    name: *const c_char,
    rgba: *const u8,
    w: u32,
    h: u32,
) -> i32 {
    crate::ffi::guarded("pixmith_layer_new_reference_rgba", doc, || {
        let Some(e) = editor(doc) else { return -1 };
        let len = (w as usize).saturating_mul(h as usize).saturating_mul(4);
        if rgba.is_null() || w == 0 || h == 0 || w > u16::MAX as u32 || h > u16::MAX as u32 || len > 1 << 28 {
            set_error("the image is empty or too large");
            return -1;
        }
        let name = path_arg(name).filter(|n| !n.is_empty()).unwrap_or_else(|| "Reference".into());
        let pixels = std::slice::from_raw_parts(rgba, len);
        match add_reference_layer(e, above as usize, &name, pixels, (w as usize, h as usize)) {
            Ok(i) => i as i32,
            Err(m) => {
                set_error(m);
                -1
            }
        }
    })
}

/// How opaque the editor draws reference layers, in percent (0 hides them, default 100). A view setting.
#[no_mangle]
pub unsafe extern "C" fn pixmith_reference_display(doc: *const Editor) -> u32 {
    crate::ffi::guarded("pixmith_reference_display", std::ptr::null_mut(), || {
        doc.as_ref().map_or(0, |e| e.reference_display as u32)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_set_reference_display(doc: *mut Editor, percent: u32) {
    crate::ffi::guarded("pixmith_set_reference_display", doc, || {
        if let Some(e) = doc.as_mut() {
            e.reference_display = percent.min(100) as u8;
            e.dirty.mark_all();
        }
    })
}
