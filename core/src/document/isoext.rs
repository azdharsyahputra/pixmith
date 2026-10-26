//! How isometric tilemap data is stored in `.aseprite` / `.pxmt` files.
//!
//! Aseprite has no isometric tilemaps, so everything it cannot express travels as one text line in the
//! sprite's user data, after the user's own text, and is lifted out again when the file is read (the user
//! never sees the line, and a file without isometric data has none):
//!
//! ```text
//! pixmith.iso.v1:{"v":1,"tilesets":[{"i":0,"tw":96,"th":96,"cw":96,"ch":48,"layout":"diamond_down",
//!                 "pivots":[[3,48,72]]}],"layers":[{"i":2,"x":592,"y":360}]}
//! ```
//!
//! * `tilesets[]`: tileset `i` (its index, which is also its Tileset chunk ID) is isometric; `tw`/`th` are its tile
//!   image size (a check that the entry still belongs to that tileset), `cw`/`ch` the diamond cell, `layout`
//!   the cell layout and `pivots` the non-default `[tile, x, y]` pivots.
//! * `layers[]`: tilemap layer `i` has the pixel offset `x`, `y` of cell (0, 0)'s bounding box. Layers at
//!   offset (0, 0) are left out.
//!
//! In the file, an isometric layer's tilemap cels are ordinary Aseprite tilemap cels whose "position" is the
//! cell coordinate (i, j) of the grid's first cell, so Aseprite shows them as a square grid.
//! Entries that do not match the file are ignored; unknown members are skipped. Version 1 is described by
//! `docs/PXMT_FORMAT.md`.

use crate::document::tilemap::{IsoLayout, IsoSpec, KIND_TILEMAP};
use crate::document::{Layer, Sprite};
use crate::formats::json::{self, Value};

pub const PREFIX: &str = "pixmith.iso.v1:";
pub const VERSION: u32 = 1;

/// Whether the sprite has anything that needs the extension line.
pub fn needed(s: &Sprite) -> bool {
    s.tilesets.iter().any(|t| t.iso.is_some())
}

/// The extension line for a sprite (`None` when it has no isometric tileset).
pub fn line(s: &Sprite) -> Option<String> {
    if !needed(s) {
        return None;
    }
    let mut tilesets = Vec::new();
    for (i, t) in s.tilesets.iter().enumerate() {
        let Some(spec) = &t.iso else { continue };
        let pivots: Vec<String> = spec.pivots.iter().map(|(k, &(x, y))| format!("[{k},{x},{y}]")).collect();
        tilesets.push(format!(
            "{{\"i\":{i},\"tw\":{},\"th\":{},\"cw\":{},\"ch\":{},\"layout\":\"diamond_down\",\"pivots\":[{}]}}",
            t.tile_w,
            t.tile_h,
            spec.cell_w,
            spec.cell_h,
            pivots.join(",")
        ));
    }
    let mut layers = Vec::new();
    for (i, l) in s.layers.iter().enumerate() {
        let iso = l.kind == KIND_TILEMAP && s.tilesets.get(l.tileset as usize).is_some_and(|t| t.iso.is_some());
        if iso && l.offset != (0, 0) {
            layers.push(format!("{{\"i\":{i},\"x\":{},\"y\":{}}}", l.offset.0, l.offset.1));
        }
    }
    Some(format!("{PREFIX}{{\"v\":{VERSION},\"tilesets\":[{}],\"layers\":[{}]}}", tilesets.join(","), layers.join(",")))
}

/// `text` with the extension line added (or replaced) after the user's own lines.
pub fn with_line(text: Option<&str>, line: &str) -> String {
    let mut lines: Vec<&str> =
        text.map(|t| t.split('\n').filter(|l| !l.starts_with(PREFIX)).collect()).unwrap_or_default();
    lines.push(line);
    lines.join("\n")
}

/// The extension line in a sprite user data text, if any.
pub fn find(text: &str) -> Option<&str> {
    text.split('\n').find_map(|l| l.strip_prefix(PREFIX))
}

/// `text` without the extension line; `None` when nothing else is left.
pub fn without_line(text: &str) -> Option<String> {
    let rest: Vec<&str> = text.split('\n').filter(|l| !l.starts_with(PREFIX)).collect();
    let joined = rest.join("\n");
    (!joined.is_empty()).then_some(joined)
}

fn int(v: Option<&Value>) -> Option<i64> {
    let f = v?.as_f64()?;
    (f.fract() == 0.0 && f.abs() < 1e9).then_some(f as i64)
}

/// Applies an extension payload (the JSON after the prefix) to freshly read tilesets and layers.
/// Entries that do not fit are skipped. Returns whether anything was applied.
pub fn apply(payload: &str, tilesets: &mut [crate::document::tilemap::Tileset], layers: &mut [Layer]) -> bool {
    let Ok(v) = json::parse(payload) else { return false };
    if int(v.get("v")).is_none_or(|n| n < 1 || n > VERSION as i64) {
        return false;
    }
    let mut any = false;
    for e in v.get("tilesets").and_then(Value::as_array).unwrap_or(&[]) {
        let (Some(i), Some(tw), Some(th), Some(cw), Some(ch)) =
            (int(e.get("i")), int(e.get("tw")), int(e.get("th")), int(e.get("cw")), int(e.get("ch")))
        else {
            continue;
        };
        let Some(t) = usize::try_from(i).ok().and_then(|i| tilesets.get_mut(i)) else { continue };
        if (t.tile_w as i64, t.tile_h as i64) != (tw, th) || !(1..=4096).contains(&cw) || !(1..=4096).contains(&ch) {
            continue;
        }
        let mut spec = IsoSpec::new(cw as u32, ch as u32);
        spec.layout = IsoLayout::DiamondDown;
        for p in e.get("pivots").and_then(Value::as_array).unwrap_or(&[]) {
            let Some(a) = p.as_array() else { continue };
            if let [k, x, y] = a {
                if let (Some(k), Some(x), Some(y)) = (int(Some(k)), int(Some(x)), int(Some(y))) {
                    if (0..t.tiles.len() as i64).contains(&k) && x.abs() <= 1 << 14 && y.abs() <= 1 << 14 {
                        spec.pivots.insert(k as u32, (x as i32, y as i32));
                    }
                }
            }
        }
        t.iso = Some(spec);
        any = true;
    }
    for e in v.get("layers").and_then(Value::as_array).unwrap_or(&[]) {
        let (Some(i), Some(x), Some(y)) = (int(e.get("i")), int(e.get("x")), int(e.get("y"))) else { continue };
        let Some(l) = usize::try_from(i).ok().and_then(|i| layers.get_mut(i)) else { continue };
        let iso = l.kind == KIND_TILEMAP && tilesets.get(l.tileset as usize).is_some_and(|t| t.iso.is_some());
        if iso && x.abs() <= 1 << 24 && y.abs() <= 1 << 24 {
            l.offset = (x as i32, y as i32);
        }
    }
    any
}
