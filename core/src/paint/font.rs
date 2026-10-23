//! A built-in 5x8 bitmap font for printable ASCII, drawn from glyph data in this file, plus text
//! rasterization to a mask and stamping masks into a cel.

use crate::document::{Editor, Sprite};
use crate::paint::shapes::{reset_to_checkpoint, tracking, Painter, Pt};
use std::ffi::{c_char, CStr};

pub const GLYPH_W: usize = 5;
pub const GLYPH_H: usize = 8;
const ADVANCE: usize = GLYPH_W + 1;
const LINE_HEIGHT: usize = GLYPH_H + 1;

/// Glyphs for ' ' through '~'. Eight rows of five pixels, '#' is ink; rows 0-6 hold capitals and
/// the x-height sits in rows 2-6, with row 7 reserved for descenders.
#[rustfmt::skip]
const GLYPHS: [&str; 95] = [
    "...../...../...../...../...../...../...../.....",
    "..#../..#../..#../..#../..#../...../..#../.....",
    ".#.#./.#.#./.#.#./...../...../...../...../.....",
    ".#.#./.#.#./#####/.#.#./#####/.#.#./.#.#./.....",
    "..#../.####/#.#../.###./..#.#/####./..#../.....",
    "##..#/##..#/...#./..#../.#.../#..##/#..##/.....",
    ".##../#..#./#.#../.#.../#.#.#/#..#./.##.#/.....",
    "..#../..#../.#.../...../...../...../...../.....",
    "...#./..#../.#.../.#.../.#.../..#../...#./.....",
    ".#.../..#../...#./...#./...#./..#../.#.../.....",
    "...../..#../#.#.#/.###./#.#.#/..#../...../.....",
    "...../..#../..#../#####/..#../..#../...../.....",
    "...../...../...../...../...../..##./..#../.#...",
    "...../...../...../#####/...../...../...../.....",
    "...../...../...../...../...../...../..#../.....",
    "....#/....#/...#./..#../.#.../#..../#..../.....",
    ".###./#...#/#..##/#.#.#/##..#/#...#/.###./.....",
    "..#../.##../..#../..#../..#../..#../.###./.....",
    ".###./#...#/....#/...#./..#../.#.../#####/.....",
    "#####/...#./..#../...#./....#/#...#/.###./.....",
    "...#./..##./.#.#./#..#./#####/...#./...#./.....",
    "#####/#..../####./....#/....#/#...#/.###./.....",
    "..##./.#.../#..../####./#...#/#...#/.###./.....",
    "#####/....#/...#./..#../.#.../.#.../.#.../.....",
    ".###./#...#/#...#/.###./#...#/#...#/.###./.....",
    ".###./#...#/#...#/.####/....#/...#./.##../.....",
    "...../..#../..#../...../..#../..#../...../.....",
    "...../..#../..#../...../..#../..#../.#.../.....",
    "...#./..#../.#.../#..../.#.../..#../...#./.....",
    "...../...../#####/...../#####/...../...../.....",
    ".#.../..#../...#./....#/...#./..#../.#.../.....",
    ".###./#...#/....#/...#./..#../...../..#../.....",
    ".###./#...#/#.###/#.#.#/#.###/#..../.###./.....",
    ".###./#...#/#...#/#####/#...#/#...#/#...#/.....",
    "####./#...#/#...#/####./#...#/#...#/####./.....",
    ".###./#...#/#..../#..../#..../#...#/.###./.....",
    "####./#...#/#...#/#...#/#...#/#...#/####./.....",
    "#####/#..../#..../####./#..../#..../#####/.....",
    "#####/#..../#..../####./#..../#..../#..../.....",
    ".###./#...#/#..../#.###/#...#/#...#/.####/.....",
    "#...#/#...#/#...#/#####/#...#/#...#/#...#/.....",
    ".###./..#../..#../..#../..#../..#../.###./.....",
    "..###/...#./...#./...#./...#./#..#./.##../.....",
    "#...#/#..#./#.#../##.../#.#../#..#./#...#/.....",
    "#..../#..../#..../#..../#..../#..../#####/.....",
    "#...#/##.##/#.#.#/#.#.#/#...#/#...#/#...#/.....",
    "#...#/##..#/#.#.#/#..##/#...#/#...#/#...#/.....",
    ".###./#...#/#...#/#...#/#...#/#...#/.###./.....",
    "####./#...#/#...#/####./#..../#..../#..../.....",
    ".###./#...#/#...#/#...#/#.#.#/#..#./.##.#/.....",
    "####./#...#/#...#/####./#.#../#..#./#...#/.....",
    ".####/#..../#..../.###./....#/....#/####./.....",
    "#####/..#../..#../..#../..#../..#../..#../.....",
    "#...#/#...#/#...#/#...#/#...#/#...#/.###./.....",
    "#...#/#...#/#...#/#...#/#...#/.#.#./..#../.....",
    "#...#/#...#/#...#/#.#.#/#.#.#/##.##/#...#/.....",
    "#...#/#...#/.#.#./..#../.#.#./#...#/#...#/.....",
    "#...#/#...#/.#.#./..#../..#../..#../..#../.....",
    "#####/....#/...#./..#../.#.../#..../#####/.....",
    ".###./.#.../.#.../.#.../.#.../.#.../.###./.....",
    "#..../#..../.#.../..#../...#./....#/....#/.....",
    ".###./...#./...#./...#./...#./...#./.###./.....",
    "..#../.#.#./#...#/...../...../...../...../.....",
    "...../...../...../...../...../...../...../#####",
    ".#.../..#../...#./...../...../...../...../.....",
    "...../...../.###./....#/.####/#...#/.####/.....",
    "#..../#..../#.##./##..#/#...#/##..#/#.##./.....",
    "...../...../.###./#..../#..../#...#/.###./.....",
    "....#/....#/.##.#/#..##/#...#/#..##/.##.#/.....",
    "...../...../.###./#...#/#####/#..../.###./.....",
    "..##./.#..#/.#.../###../.#.../.#.../.#.../.....",
    "...../...../.####/#...#/#...#/.####/....#/.###.",
    "#..../#..../#.##./##..#/#...#/#...#/#...#/.....",
    "..#../...../.##../..#../..#../..#../.###./.....",
    "...#./...../..##./...#./...#./...#./#..#./.##..",
    "#..../#..../#..#./#.#../##.../#.#../#..#./.....",
    ".##../..#../..#../..#../..#../..#../.###./.....",
    "...../...../##.#./#.#.#/#.#.#/#...#/#...#/.....",
    "...../...../#.##./##..#/#...#/#...#/#...#/.....",
    "...../...../.###./#...#/#...#/#...#/.###./.....",
    "...../...../####./#...#/#...#/####./#..../#....",
    "...../...../.####/#...#/#...#/.####/....#/....#",
    "...../...../#.##./##..#/#..../#..../#..../.....",
    "...../...../.####/#..../.###./....#/####./.....",
    ".#.../.#.../###../.#.../.#.../.#..#/..##./.....",
    "...../...../#...#/#...#/#...#/#..##/.##.#/.....",
    "...../...../#...#/#...#/#...#/.#.#./..#../.....",
    "...../...../#...#/#...#/#.#.#/#.#.#/.#.#./.....",
    "...../...../#...#/.#.#./..#../.#.#./#...#/.....",
    "...../...../#...#/#...#/#...#/.####/....#/.###.",
    "...../...../#####/...#./..#../.#.../#####/.....",
    "..##./.#.../.#.../#..../.#.../.#.../..##./.....",
    "..#../..#../..#../..#../..#../..#../..#../..#..",
    ".##../...#./...#./....#/...#./...#./.##../.....",
    "...../...../.#.../#.#.#/...#./...../...../.....",
];

/// Rows of a glyph as bit masks, bit 4 being the leftmost pixel. Unknown characters draw as '?'.
pub fn glyph(c: char) -> [u8; GLYPH_H] {
    let code = c as u32;
    let i = if (32..127).contains(&code) { code as usize - 32 } else { '?' as usize - 32 };
    let mut rows = [0u8; GLYPH_H];
    for (r, row) in GLYPHS[i].split('/').enumerate() {
        rows[r] = row.bytes().fold(0, |acc, b| acc << 1 | (b == b'#') as u8);
    }
    rows
}

/// Largest text mask `render` builds; longer or larger text renders as nothing.
const MAX_MASK_PIXELS: u64 = 1 << 26;

/// Coverage mask of rendered text: 255 where ink goes.
#[derive(Debug, PartialEq)]
pub struct Mask {
    pub w: i32,
    pub h: i32,
    pub data: Vec<u8>,
}

/// Size in pixels of `text` at an integer scale.
pub fn text_size(text: &str, scale: u32) -> (i32, i32) {
    let scale = scale.clamp(1, 32) as usize;
    let lines = text.split('\n');
    let count = lines.clone().count();
    let widest = lines.map(|l| l.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum::<usize>()).max().unwrap_or(0);
    let w = if widest == 0 { 0 } else { (widest * ADVANCE - 1) * scale };
    (w.min(i32::MAX as usize) as i32, ((count * LINE_HEIGHT - 1) * scale).min(i32::MAX as usize) as i32)
}

/// Renders text with the built-in font. Newlines start a new line; tabs are four spaces.
pub fn render(text: &str, scale: u32) -> Mask {
    let scale = scale.clamp(1, 32) as usize;
    let (w, h) = text_size(text, scale as u32);
    if w < 0 || h < 0 || w as u64 * h as u64 > MAX_MASK_PIXELS {
        return Mask { w: 0, h: 0, data: Vec::new() };
    }
    let mut data = vec![0u8; (w * h) as usize];
    for (line, content) in text.split('\n').enumerate() {
        let mut col = 0;
        for c in content.chars() {
            if c == '\t' {
                col += 4;
                continue;
            }
            for (r, bits) in glyph(c).iter().enumerate() {
                for x in 0..GLYPH_W {
                    if bits >> (GLYPH_W - 1 - x) & 1 == 1 {
                        let (px, py) = ((col * ADVANCE + x) * scale, (line * LINE_HEIGHT + r) * scale);
                        for dy in 0..scale {
                            let o = (py + dy) * w as usize + px;
                            data[o..o + scale].fill(255);
                        }
                    }
                }
            }
            col += 1;
        }
    }
    Mask { w, h, data }
}

impl Sprite {
    /// Paints `color` through a coverage mask (values above 127) with its top-left corner at `at`.
    #[allow(clippy::too_many_arguments)]
    pub fn stamp_mask(
        &mut self,
        layer: usize,
        frame: usize,
        at: Pt,
        w: i32,
        h: i32,
        mask: &[u8],
        color: [u8; 4],
        flags: u32,
    ) {
        if w <= 0 || h <= 0 || mask.len() != w as usize * h as usize {
            return;
        }
        let dims = self.dims();
        let Some(cel) = self.cel_mut(layer, frame) else { return };
        let mut touched = tracking(dims);
        let mut p = Painter::new(cel, dims, color, 1, flags).plain().tracked(touched.as_deref_mut());
        for y in 0..h {
            let mut x = 0;
            while x < w {
                if mask[(y * w + x) as usize] <= 127 {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < w && mask[(y * w + x) as usize] > 127 {
                    x += 1;
                }
                p.rect(at.0 + start, at.1 + y, at.0 + x - 1, at.1 + y);
            }
        }
    }
}

unsafe fn text_arg(p: *const c_char) -> Option<String> {
    (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
}

/// Draws UTF-8 text with the built-in font, top-left at `(x, y)`, each font pixel `scale` pixels
/// square. With `restore`, rewinds to the `pixmith_begin_stroke` checkpoint first.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_text(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    text: *const c_char,
    scale: u32,
    color: u32,
    flags: u32,
    restore: bool,
) {
    crate::ffi::guarded("pixmith_text", doc, || {
        let (Some(e), Some(text)) = (crate::ffi::editor(doc), text_arg(text)) else { return };
        if restore {
            reset_to_checkpoint(e);
        }
        let m = render(&text, scale);
        let (x, y) = (crate::ffi::clampc(x), crate::ffi::clampc(y));
        e.sprite.stamp_mask(layer as usize, frame as usize, (x, y), m.w, m.h, &m.data, color.to_be_bytes(), flags);
        crate::paint::selection::clip_to_selection(e, layer as usize, frame as usize);
    })
}

/// Writes the pixel size of `text` at `scale` to `out[0..2]`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_text_size(text: *const c_char, scale: u32, out: *mut i32) {
    crate::ffi::guarded("pixmith_text_size", std::ptr::null_mut(), || {
        if let (Some(t), false) = (text_arg(text), out.is_null()) {
            let (w, h) = text_size(&t, scale);
            std::slice::from_raw_parts_mut(out, 2).copy_from_slice(&[w, h]);
        }
    })
}

/// Paints `color` through a w*h coverage mask (e.g. text rasterized by the app).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_stamp_mask(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    mask: *const u8,
    color: u32,
    flags: u32,
    restore: bool,
) {
    crate::ffi::guarded("pixmith_stamp_mask", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return };
        if mask.is_null() || w == 0 || h == 0 || w as u64 * h as u64 > 1 << 26 {
            return;
        }
        if restore {
            reset_to_checkpoint(e);
        }
        let data = std::slice::from_raw_parts(mask, w as usize * h as usize);
        let (x, y) = (crate::ffi::clampc(x), crate::ffi::clampc(y));
        e.sprite.stamp_mask(
            layer as usize,
            frame as usize,
            (x, y),
            w as i32,
            h as i32,
            data,
            color.to_be_bytes(),
            flags,
        );
        crate::paint::selection::clip_to_selection(e, layer as usize, frame as usize);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::shapes::SYM_X;
    use std::collections::HashSet;

    #[test]
    fn glyph_table_is_well_formed() {
        for (i, g) in GLYPHS.iter().enumerate() {
            let rows: Vec<&str> = g.split('/').collect();
            let c = (32 + i as u8) as char;
            assert_eq!(rows.len(), GLYPH_H, "rows of {c:?}");
            for r in rows {
                assert_eq!(r.chars().count(), GLYPH_W, "width of {c:?}: {r}");
                assert!(r.chars().all(|ch| ch == '#' || ch == '.'), "chars of {c:?}");
            }
        }
    }

    #[test]
    fn glyphs_are_distinct_and_space_is_blank() {
        assert_eq!(glyph(' '), [0; GLYPH_H]);
        let mut seen = HashSet::new();
        for c in '!'..='~' {
            let g = glyph(c);
            assert!(g.iter().any(|&r| r != 0), "{c:?} is blank");
            assert!(seen.insert(g), "{c:?} duplicates another glyph");
        }
    }

    #[test]
    fn capitals_stay_above_the_descender_row_and_descenders_use_it() {
        for c in ('A'..='Z').chain('0'..='9') {
            assert_eq!(glyph(c)[7], 0, "{c}");
        }
        for c in "gjpqy".chars() {
            assert_ne!(glyph(c)[7], 0, "{c}");
        }
        assert_eq!(glyph('a')[0] | glyph('a')[1], 0);
    }

    #[test]
    fn known_shapes() {
        assert_eq!(glyph('I'), [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110, 0]);
        assert_eq!(glyph('-')[3], 0b11111);
        assert_eq!(glyph('é'), glyph('?'));
    }

    #[test]
    fn size_and_render_agree() {
        assert_eq!(text_size("", 1), (0, 8));
        assert_eq!(text_size("A", 1), (5, 8));
        assert_eq!(text_size("AB", 2), (22, 16));
        assert_eq!(text_size("AB\nC", 1), (11, 17));
        assert_eq!(text_size("\tA", 1), (29, 8));
        let m = render("Hi!", 3);
        assert_eq!((m.w, m.h), text_size("Hi!", 3));
        assert_eq!(m.data.len(), (m.w * m.h) as usize);
    }

    #[test]
    fn scaling_replicates_pixels() {
        let one = render("A", 1);
        let two = render("A", 2);
        for y in 0..one.h {
            for x in 0..one.w {
                let v = one.data[(y * one.w + x) as usize];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    assert_eq!(two.data[((2 * y + dy) * two.w + 2 * x + dx) as usize], v);
                }
            }
        }
    }

    #[test]
    fn newline_moves_down_a_line() {
        let m = render("I\nI", 1);
        let row = |y: i32| m.data[(y * m.w) as usize..((y + 1) * m.w) as usize].iter().filter(|&&v| v > 0).count();
        assert_eq!(row(0), 3);
        assert_eq!(row(9), 3);
        assert_eq!(row(8), 0);
    }

    #[test]
    fn stamping_paints_clips_and_mirrors() {
        let red = [255, 0, 0, 255];
        let mut s = Sprite::new(20, 10);
        let m = render("I", 1);
        s.stamp_mask(0, 0, (1, 1), m.w, m.h, &m.data, red, 0);
        let px = s.render(0);
        let lit = |x: usize, y: usize| px[(y * 20 + x) * 4 + 3] != 0;
        assert!(lit(2, 1) && lit(3, 1) && lit(4, 1) && !lit(1, 1) && lit(3, 4));
        let mut s = Sprite::new(20, 10);
        s.stamp_mask(0, 0, (-2, -3), m.w, m.h, &m.data, red, SYM_X);
        assert!(s.render(0).chunks(4).any(|c| c[3] != 0));
        let mut s = Sprite::new(20, 10);
        s.stamp_mask(0, 0, (0, 0), 2, 2, &[255; 3], red, 0);
        assert!(s.render(0).chunks(4).all(|c| c[3] == 0));
    }

    #[test]
    fn ffi_text_previews_in_one_undo_step() {
        let mut e = Editor::new(Sprite::new(30, 12));
        e.begin_preview();
        let a = std::ffi::CString::new("A").unwrap();
        let b = std::ffi::CString::new("BB").unwrap();
        let mut size = [0; 2];
        unsafe {
            pixmith_text_size(b.as_ptr(), 1, size.as_mut_ptr());
            assert_eq!(size, [11, 8]);
            pixmith_text(&mut e, 0, 0, 1, 1, a.as_ptr(), 1, 0xFF0000FF, 0, true);
            pixmith_text(&mut e, 0, 0, 1, 1, b.as_ptr(), 1, 0xFF0000FF, 0, true);
            pixmith_text(&mut e, 0, 0, 0, 0, std::ptr::null(), 1, 0, 0, false);
        }
        let px = e.sprite.render(0);
        assert!((0..30 * 12).any(|i| px[i * 4 + 3] != 0 && i % 30 >= 7));
        assert!(e.undo());
        assert!(e.sprite.render(0).chunks(4).all(|c| c[3] == 0));
    }
}
