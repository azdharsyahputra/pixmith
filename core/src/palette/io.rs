//! Palette files (GIMP `.gpl`, plain `.hex`), sorting, and their C ABI.

use crate::document::Editor;
use std::ffi::c_char;

pub type Color = [u8; 4];

const MAX_COLORS: usize = 256;

/// Parses a GIMP palette (`.gpl`) or a list of hex colors (`.hex`), detected from the content.
pub fn parse(text: &str) -> Result<Vec<Color>, String> {
    let text = text.trim_start_matches('\u{feff}');
    let colors = if text.trim_start().starts_with("GIMP Palette") { parse_gpl(text)? } else { parse_hex(text)? };
    if colors.is_empty() {
        return Err("palette has no colors".into());
    }
    Ok(colors.into_iter().take(MAX_COLORS).collect())
}

pub fn parse_gpl(text: &str) -> Result<Vec<Color>, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("GIMP Palette") {
        return Err("not a GIMP palette".into());
    }
    let mut out = Vec::new();
    for (n, line) in lines.enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("Name:") || line.starts_with("Columns:") {
            continue;
        }
        let mut c = [0, 0, 0, 255];
        let mut parts = line.split_whitespace();
        for v in c.iter_mut().take(3) {
            *v = parts
                .next()
                .and_then(|p| p.parse::<u8>().ok())
                .ok_or_else(|| format!("line {}: expected three values 0-255", n + 2))?;
        }
        out.push(c);
    }
    Ok(out)
}

pub fn parse_hex(text: &str) -> Result<Vec<Color>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim().trim_start_matches('#');
        if line.is_empty() {
            continue;
        }
        let bad = || format!("line {}: invalid hex color", n + 1);
        if !line.is_ascii() || (line.len() != 6 && line.len() != 8) {
            return Err(bad());
        }
        let mut c = [0, 0, 0, 255];
        for (i, v) in c.iter_mut().enumerate().take(line.len() / 2) {
            *v = u8::from_str_radix(&line[i * 2..i * 2 + 2], 16).map_err(|_| bad())?;
        }
        out.push(c);
    }
    Ok(out)
}

pub fn to_gpl(colors: &[Color]) -> String {
    let mut s = String::from("GIMP Palette\nName: Pixmith\nColumns: 8\n#\n");
    for (i, c) in colors.iter().enumerate() {
        s.push_str(&format!("{:3} {:3} {:3}\tIndex {}\n", c[0], c[1], c[2], i));
    }
    s
}

/// One lowercase hex color per line; colors with alpha get eight digits.
pub fn to_hex(colors: &[Color]) -> String {
    colors
        .iter()
        .map(|c| {
            if c[3] == 255 {
                format!("{:02x}{:02x}{:02x}\n", c[0], c[1], c[2])
            } else {
                format!("{:02x}{:02x}{:02x}{:02x}\n", c[0], c[1], c[2], c[3])
            }
        })
        .collect()
}

pub fn luminance(c: Color) -> f32 {
    0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32
}

/// Hue in degrees and saturation (0-1) of the HSV model.
pub fn hue_sat(c: Color) -> (f32, f32) {
    let [r, g, b] = [c[0] as f32, c[1] as f32, c[2] as f32];
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0);
    }
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, d / max)
}

/// Grays first (dark to light), then colors by hue and brightness.
pub fn sort_by_hue(colors: &mut [Color]) {
    colors.sort_by(|a, b| {
        let key = |c: &Color| {
            let (h, s) = hue_sat(*c);
            (s > 0.0, h, luminance(*c))
        };
        key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal)
    });
}

pub fn sort_by_luminance(colors: &mut [Color]) {
    colors.sort_by(|a, b| luminance(*a).total_cmp(&luminance(*b)));
}

unsafe fn path_str(p: *const c_char) -> Option<String> {
    (!p.is_null()).then(|| std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
}

/// Replaces the palette with the contents of a `.gpl` or `.hex` file (one undo step).
#[no_mangle]
pub unsafe extern "C" fn pixmith_palette_load(doc: *mut Editor, path: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_palette_load", doc, || {
        let (Some(e), Some(path)) = (crate::ffi::editor(doc), path_str(path)) else { return false };
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| parse(&t)) {
            Ok(colors) => {
                e.checkpoint();
                e.label_last("Load Palette");
                crate::palette::edit::replace_palette(&mut e.sprite, colors);
                true
            }
            Err(msg) => {
                crate::ffi::set_error(format!("{path}: {msg}"));
                false
            }
        }
    })
}

/// Writes the palette as `.hex` when the path ends in ".hex", otherwise as `.gpl`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_palette_save(doc: *const Editor, path: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_palette_save", std::ptr::null_mut(), || {
        let Some(s) = crate::ffi::sprite(doc) else { return false };
        let hex = path_str(path).is_some_and(|p| p.to_ascii_lowercase().ends_with(".hex"));
        let text = if hex { to_hex(&s.palette) } else { to_gpl(&s.palette) };
        crate::ffi::save_to(path, Ok(text.into_bytes()))
    })
}

/// Removes one color; the last remaining color cannot be removed.
#[no_mangle]
pub unsafe extern "C" fn pixmith_palette_remove(doc: *mut Editor, index: u32) -> bool {
    crate::ffi::guarded("pixmith_palette_remove", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return false };
        if e.sprite.palette.len() <= 1 || index as usize >= e.sprite.palette.len() {
            return false;
        }
        let before = e.sprite.clone();
        if !crate::palette::edit::remove_entries(&mut e.sprite, &[index as usize]) {
            return false;
        }
        let after = std::mem::replace(&mut e.sprite, before);
        e.checkpoint();
        e.label_last("Delete Palette Color");
        e.sprite = after;
        true
    })
}

/// Sorts the palette: mode 0 by hue, 1 by luminance.
#[no_mangle]
pub unsafe extern "C" fn pixmith_palette_sort(doc: *mut Editor, mode: u32) {
    crate::ffi::guarded("pixmith_palette_sort", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return };
        e.checkpoint();
        e.label_last("Sort Palette");
        let before = e.sprite.palette.clone();
        if mode == 0 {
            sort_by_hue(&mut e.sprite.palette);
        } else {
            sort_by_luminance(&mut e.sprite.palette);
        }
        crate::palette::edit::adopt_permutation(&mut e.sprite, &before);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Sprite;
    use std::ffi::CString;

    #[test]
    fn parses_gpl_with_header_comments_and_names() {
        let text = "GIMP Palette\nName: Test\nColumns: 4\n# note\n  0   0   0\tBlack\n255 128   0 Orange Peel\n\n";
        assert_eq!(parse(text).unwrap(), vec![[0, 0, 0, 255], [255, 128, 0, 255]]);
    }

    #[test]
    fn rejects_bad_gpl() {
        assert!(parse("GIMP Palette\n1 2\n").is_err());
        assert!(parse("GIMP Palette\n300 0 0\n").is_err());
        assert!(parse("GIMP Palette\n# only comments\n").is_err());
        assert!(parse_gpl("nope\n1 2 3\n").is_err());
    }

    #[test]
    fn parses_hex_variants() {
        let text = "\u{feff}FF0000\n#00ff00\r\n\n0000ff80\n";
        assert_eq!(parse(text).unwrap(), vec![[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 128]]);
        assert!(parse("12345\n").is_err());
        assert!(parse("gggggg\n").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn round_trips() {
        let colors = vec![[1, 2, 3, 255], [255, 255, 255, 255], [10, 20, 30, 40]];
        assert_eq!(parse(&to_hex(&colors)).unwrap(), colors);
        let opaque = &colors[..2];
        assert_eq!(parse(&to_gpl(opaque)).unwrap(), opaque);
    }

    #[test]
    fn caps_at_256_colors() {
        let text: String = (0..300).map(|i| format!("{:02x}{:02x}00\n", i % 256, i / 256)).collect();
        assert_eq!(parse(&text).unwrap().len(), 256);
    }

    #[test]
    fn hue_sorting_orders_grays_then_hues() {
        let mut c = vec![[0, 0, 255, 255], [255, 255, 255, 255], [0, 255, 0, 255], [255, 0, 0, 255], [0, 0, 0, 255]];
        sort_by_hue(&mut c);
        assert_eq!(c, vec![[0, 0, 0, 255], [255, 255, 255, 255], [255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]);
        assert!((hue_sat([255, 255, 0, 255]).0 - 60.0).abs() < 1e-3);
        assert!((hue_sat([255, 0, 255, 255]).0 - 300.0).abs() < 1e-3);
    }

    #[test]
    fn luminance_sorting() {
        let mut c = vec![[255, 255, 255, 255], [255, 0, 0, 255], [0, 0, 255, 255], [0, 255, 0, 255]];
        sort_by_luminance(&mut c);
        assert_eq!(c, vec![[0, 0, 255, 255], [255, 0, 0, 255], [0, 255, 0, 255], [255, 255, 255, 255]]);
    }

    #[test]
    fn ffi_load_save_remove_sort_are_undoable() {
        let dir = std::env::temp_dir().join(format!("pixmith-pal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("in.hex");
        std::fs::write(&src, "ffffff\n000000\n808080\n").unwrap();
        let cstr = |p: &std::path::Path| CString::new(p.to_str().unwrap()).unwrap();
        let mut e = Editor::new(Sprite::new(1, 1));
        let original = e.sprite.palette.clone();
        unsafe {
            assert!(pixmith_palette_load(&mut e, cstr(&src).as_ptr()));
            assert_eq!(e.sprite.palette.len(), 3);
            pixmith_palette_sort(&mut e, 1);
            assert_eq!(e.sprite.palette[0], [0, 0, 0, 255]);
            assert!(pixmith_palette_remove(&mut e, 0));
            assert_eq!(e.sprite.palette.len(), 2);
            for p in ["out.gpl", "out.hex"] {
                let out = dir.join(p);
                assert!(pixmith_palette_save(&e, cstr(&out).as_ptr()));
                assert_eq!(parse(&std::fs::read_to_string(&out).unwrap()).unwrap(), e.sprite.palette);
            }
            assert!(!pixmith_palette_load(&mut e, cstr(&dir.join("missing.gpl")).as_ptr()));
        }
        assert!(e.undo() && e.undo() && e.undo());
        assert_eq!(e.sprite.palette, original);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn last_color_is_kept() {
        let mut e = Editor::new(Sprite::new(1, 1));
        e.sprite.palette = vec![[1, 1, 1, 255]];
        assert!(!unsafe { pixmith_palette_remove(&mut e, 0) });
    }
}
