//! Built-in palette presets and their C ABI.

use crate::document::Editor;
use crate::ffi::{editor, set_error};
use crate::palette::edit::replace_palette;
use std::ffi::{c_char, CString};
use std::ptr::null_mut;

/// 0xRRGGBB colors.
const PICO8: &[u32] = &[
    0x000000, 0x1D2B53, 0x7E2553, 0x008751, 0xAB5236, 0x5F574F, 0xC2C3C7, 0xFFF1E8, 0xFF004D, 0xFFA300, 0xFFEC27,
    0x00E436, 0x29ADFF, 0x83769C, 0xFF77A8, 0xFFCCAA,
];

/// DawnBringer's 16 colors.
const DB16: &[u32] = &[
    0x140C1C, 0x442434, 0x30346D, 0x4E4A4E, 0x854C30, 0x346524, 0xD04648, 0x757161, 0x597DCE, 0xD27D2C, 0x8595A1,
    0x6DAA2C, 0xD2AA99, 0x6DC2CA, 0xDAD45E, 0xDEEED6,
];

/// DawnBringer's 32 colors.
const DB32: &[u32] = &[
    0x000000, 0x222034, 0x45283C, 0x663931, 0x8F563B, 0xDF7126, 0xD9A066, 0xEEC39A, 0xFBF236, 0x99E550, 0x6ABE30,
    0x37946E, 0x4B692F, 0x524B24, 0x323C39, 0x3F3F74, 0x306082, 0x5B6EE1, 0x639BFF, 0x5FCDE4, 0xCBDBFC, 0xFFFFFF,
    0x9BADB7, 0x847E87, 0x696A6A, 0x595652, 0x76428A, 0xAC3232, 0xD95763, 0xD77BBA, 0x8F974A, 0x8A6F30,
];

const ENDESGA32: &[u32] = &[
    0xBE4A2F, 0xD77643, 0xEAD4AA, 0xE4A672, 0xB86F50, 0x733E39, 0x3E2731, 0xA22633, 0xE43B44, 0xF77622, 0xFEAE34,
    0xFEE761, 0x63C74D, 0x3E8948, 0x265C42, 0x193C3E, 0x124E89, 0x0099DB, 0x2CE8F5, 0xFFFFFF, 0xC0CBDC, 0x8B9BB4,
    0x5A6988, 0x3A4466, 0x262B44, 0x181425, 0xFF0044, 0x68386C, 0xB55088, 0xF6757A, 0xE8B796, 0xC28569,
];

/// The four shades of the original Game Boy screen, darkest first.
const GAME_BOY: &[u32] = &[0x0F380F, 0x306230, 0x8BAC0F, 0x9BBC0F];

/// Four rows of 14 NES colors (duplicate blacks and whites are dropped when loading: 54 left).
const NES: &[u32] = &[
    0x626262, 0x001FB2, 0x2404C8, 0x5200B2, 0x730076, 0x800024, 0x730B00, 0x522800, 0x244400, 0x005700, 0x005C00,
    0x005324, 0x003C76, 0x000000, //
    0xABABAB, 0x0D57FF, 0x4B30FF, 0x8A13FF, 0xBC08D6, 0xD21269, 0xC72E00, 0x9D5400, 0x607B00, 0x209800, 0x00A300,
    0x009942, 0x007DB4, 0x000000, //
    0xFFFFFF, 0x53AEFF, 0x9085FF, 0xD365FF, 0xFF57FF, 0xFF5DCF, 0xFF7757, 0xFA9E00, 0xBDC700, 0x7AE700, 0x43F611,
    0x26EF7E, 0x2CD5F6, 0x4E4E4E, //
    0xFFFFFF, 0xB6E1FF, 0xCED1FF, 0xE9C3FF, 0xFFBCFF, 0xFFBDF4, 0xFFC6C3, 0xFFD59A, 0xE9E681, 0xCEF481, 0xB6FB9A,
    0xA9FAC3, 0xA9F0F4, 0xB8B8B8,
];

/// `n` evenly spaced grays from black to white.
fn grays(n: u32) -> Vec<u32> {
    (0..n)
        .map(|i| {
            let v = (i * 255 + (n - 1) / 2) / (n - 1);
            v << 16 | v << 8 | v
        })
        .collect()
}

/// Preset names, in menu order.
pub const NAMES: [&str; 12] = [
    "PICO-8",
    "DB16",
    "DB32",
    "Endesga 32",
    "Game Boy (4)",
    "NES (54)",
    "Grayscale 2",
    "Grayscale 4",
    "Grayscale 8",
    "Grayscale 16",
    "Grayscale 32",
    "Grayscale 256",
];

/// Colors of preset `index`, opaque, without duplicates.
pub fn preset(index: usize) -> Option<Vec<[u8; 4]>> {
    let rgb: Vec<u32> = match index {
        0 => PICO8.to_vec(),
        1 => DB16.to_vec(),
        2 => DB32.to_vec(),
        3 => ENDESGA32.to_vec(),
        4 => GAME_BOY.to_vec(),
        5 => NES.to_vec(),
        6 => grays(2),
        7 => grays(4),
        8 => grays(8),
        9 => grays(16),
        10 => grays(32),
        11 => grays(256),
        _ => return None,
    };
    let mut out: Vec<[u8; 4]> = Vec::with_capacity(rgb.len());
    for c in rgb {
        let [_, r, g, b] = c.to_be_bytes();
        if !out.contains(&[r, g, b, 255]) {
            out.push([r, g, b, 255]);
        }
    }
    Some(out)
}

/// Names of every preset a user can pick: the built-in ones, then the palettes of enabled extensions
/// ("Name (Extension)"), in the same order as [`colors`].
pub fn names() -> Vec<String> {
    let mut out: Vec<String> = NAMES.iter().map(|n| n.to_string()).collect();
    out.extend(crate::extensions::palettes().into_iter().map(|(ext, p)| format!("{} ({ext})", p.name)));
    out
}

/// Colors of preset `index` of [`names`]: a built-in one or a palette contributed by an extension.
pub fn colors(index: usize) -> Option<Vec<[u8; 4]>> {
    if index < NAMES.len() {
        return preset(index);
    }
    crate::extensions::palettes().into_iter().nth(index - NAMES.len()).map(|(_, p)| p.colors)
}

#[no_mangle]
pub extern "C" fn pixmith_pe_preset_count() -> u32 {
    crate::ffi::guarded("pixmith_pe_preset_count", std::ptr::null_mut(), || names().len() as u32)
}

/// Name of a preset; release with `pixmith_string_free`. Null if out of range.
#[no_mangle]
pub extern "C" fn pixmith_pe_preset_name(index: u32) -> *mut c_char {
    crate::ffi::guarded("pixmith_pe_preset_name", std::ptr::null_mut(), || {
        names().into_iter().nth(index as usize).and_then(|n| CString::new(n).ok()).map_or(null_mut(), CString::into_raw)
    })
}

/// Replaces the palette with a preset (one undo step). In Indexed mode pixels are re-snapped
/// to the new palette.
#[no_mangle]
pub unsafe extern "C" fn pixmith_pe_preset_load(doc: *mut Editor, index: u32) -> bool {
    crate::ffi::guarded("pixmith_pe_preset_load", doc, || {
        let Some(e) = editor(doc) else { return false };
        let (Some(colors), Some(name)) = (colors(index as usize), names().into_iter().nth(index as usize)) else {
            set_error("unknown palette preset");
            return false;
        };
        e.checkpoint();
        e.label_last(&format!("Load Palette Preset: {name}"));
        replace_palette(&mut e.sprite, colors);
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::color_mode::ColorMode;
    use crate::document::Sprite;

    #[test]
    fn preset_sizes() {
        let sizes: Vec<usize> = (0..NAMES.len()).map(|i| preset(i).unwrap().len()).collect();
        assert_eq!(sizes, vec![16, 16, 32, 32, 4, 54, 2, 4, 8, 16, 32, 256]);
        assert!(preset(NAMES.len()).is_none());
    }

    #[test]
    fn presets_have_no_duplicates_and_are_opaque() {
        for i in 0..NAMES.len() {
            let p = preset(i).unwrap();
            for (j, c) in p.iter().enumerate() {
                assert_eq!(c[3], 255);
                assert!(!p[..j].contains(c), "{} repeats {c:?}", NAMES[i]);
            }
        }
    }

    #[test]
    fn known_colors() {
        assert_eq!(preset(0).unwrap()[8], [255, 0, 77, 255]);
        assert_eq!(preset(4).unwrap()[0], [0x0F, 0x38, 0x0F, 255]);
        assert_eq!(preset(6).unwrap(), vec![[0, 0, 0, 255], [255, 255, 255, 255]]);
        assert_eq!(preset(7).unwrap()[1], [85, 85, 85, 255]);
        assert_eq!(preset(11).unwrap()[200], [200, 200, 200, 255]);
    }

    #[test]
    fn pico8_matches_the_default_palette() {
        assert_eq!(preset(0).unwrap(), crate::document::default_palette());
    }

    #[test]
    fn load_is_undoable_and_resnaps_indexed_pixels() {
        let mut s = Sprite::new(2, 1);
        s.draw_line(0, 0, 0, 0, 0, 0, [200, 200, 200, 255], 1);
        s.draw_line(0, 0, 1, 0, 1, 0, [10, 10, 10, 255], 1);
        let mut e = Editor::new(s);
        let before = e.sprite.clone();
        unsafe {
            assert!(!pixmith_pe_preset_load(&mut e, 100_000));
            assert!(pixmith_pe_preset_load(&mut e, 7));
        }
        assert_eq!(e.sprite.palette.len(), 4, "RGB sprites just get the palette");
        assert_eq!(e.sprite.render(0)[0..4], [200, 200, 200, 255]);
        assert!(e.undo());
        assert_eq!(e.sprite, before);

        e.sprite.color_mode = ColorMode::Indexed;
        unsafe { assert!(pixmith_pe_preset_load(&mut e, 7)) };
        // 4 grays + the transparent entry added for indexed sprites.
        assert_eq!(e.sprite.palette.len(), 5);
        assert_eq!(e.sprite.palette[e.sprite.transparent_index as usize][3], 0);
        let r = e.sprite.render(0);
        assert_eq!(r[0..4], [170, 170, 170, 255]);
        assert_eq!(r[4..8], [0, 0, 0, 255]);
        let back = crate::formats::ase::read(&crate::formats::ase::write(&e.sprite)).unwrap();
        assert_eq!(back.render(0), r);
    }

    #[test]
    fn names_through_the_c_abi() {
        assert!(pixmith_pe_preset_count() as usize >= NAMES.len());
        let p = pixmith_pe_preset_name(2);
        assert_eq!(unsafe { std::ffi::CStr::from_ptr(p) }.to_str().unwrap(), "DB32");
        unsafe { drop(CString::from_raw(p)) };
        assert!(pixmith_pe_preset_name(100_000).is_null());
    }
}
