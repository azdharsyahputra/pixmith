//! Aseprite's `Palette` for Lua scripts: `sprite.palettes[1]`, `Palette(n)`, `Palette{ fromFile = path }`,
//! `palette:getColor(i)` (a `Color`), `setColor`, `resize`, `#palette`, `saveAs`, and `sprite:setPalette` /
//! `sprite:loadPalette`. Palette entries are 0-based, as in Aseprite and Pixmith's `sprite:getPaletteColor`.

use super::access::{self, Access};
use super::color::{self, ColorH};
use super::{Ctx, SpriteH};
use crate::document::color_mode::{self, ColorMode};
use mlua::{Error, Lua, MetaMethod, MultiValue, Result as LuaResult, UserData, UserDataFields, UserDataMethods, Value};
use std::cell::RefCell;
use std::rc::Rc;

const MAX_COLORS: usize = 256;

enum Kind {
    /// The palette of a sprite: edits change the sprite.
    Sprite(Rc<Ctx>),
    /// A free-standing palette (`Palette(n)`, `Palette{ fromFile }`, `palette:clone()`).
    Own(RefCell<Vec<[u8; 4]>>),
}

pub(super) struct PaletteH {
    /// Any context of the run; colors are created through it.
    ctx: Rc<Ctx>,
    kind: Kind,
}

impl PaletteH {
    fn colors(&self) -> Vec<[u8; 4]> {
        match &self.kind {
            Kind::Sprite(c) => c.sprite().palette.clone(),
            Kind::Own(v) => v.borrow().clone(),
        }
    }

    fn len(&self) -> usize {
        match &self.kind {
            Kind::Sprite(c) => c.sprite().palette.len(),
            Kind::Own(v) => v.borrow().len(),
        }
    }

    fn index(&self, i: i64) -> LuaResult<usize> {
        usize::try_from(i).ok().filter(|&i| i < self.len()).ok_or_else(|| {
            Error::runtime(format!("palette index {i} is out of bounds (0..{})", self.len().saturating_sub(1)))
        })
    }
}

fn read_palette_file(path: &std::path::Path) -> Result<Vec<[u8; 4]>, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let bytes = std::fs::read(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    match ext.as_str() {
        "png" | "aseprite" | "ase" | "pxmt" | "gif" | "bmp" => {
            let s = crate::formats::raster::read_any(&bytes, &path.to_string_lossy())?;
            if matches!(ext.as_str(), "aseprite" | "ase" | "pxmt") {
                return Ok(s.palette.into_iter().take(MAX_COLORS).collect());
            }
            // An image: its distinct colors, in reading order.
            let px = s.render(0);
            let mut out: Vec<[u8; 4]> = Vec::new();
            for c in px.chunks_exact(4) {
                let c = [c[0], c[1], c[2], c[3]];
                if !out.contains(&c) {
                    out.push(c);
                    if out.len() >= MAX_COLORS {
                        break;
                    }
                }
            }
            Ok(out)
        }
        _ => crate::palette::io::parse(&String::from_utf8_lossy(&bytes)),
    }
}

fn save_palette_file(ctx: &Rc<Ctx>, raw: &str, colors: &[[u8; 4]]) -> LuaResult<()> {
    let path = access::check(ctx, raw, Access::Write)?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let data: Vec<u8> = match ext.as_str() {
        "gpl" => crate::palette::io::to_gpl(colors).into_bytes(),
        "hex" => crate::palette::io::to_hex(colors).into_bytes(),
        "png" => {
            let px: Vec<u8> = colors.iter().flatten().copied().collect();
            crate::formats::export::encode_png(&px, colors.len().max(1), 1).map_err(Error::runtime)?
        }
        other => {
            return Err(Error::runtime(format!("cannot save a palette as \".{other}\" (use .gpl, .hex or .png)")));
        }
    };
    crate::formats::durable::write_file(&path.to_string_lossy(), &data).map_err(Error::runtime)
}

fn assign(ctx: &Rc<Ctx>, colors: &[[u8; 4]]) -> LuaResult<()> {
    if colors.is_empty() || colors.len() > MAX_COLORS {
        return Err(Error::runtime(format!("a palette has 1..{MAX_COLORS} colors")));
    }
    let s = ctx.sprite();
    if s.color_mode == ColorMode::Indexed {
        // Pixels follow their entry: recolor in place for the shared part, then append or drop the rest.
        let common = s.palette.len().min(colors.len());
        for (i, c) in colors.iter().enumerate().take(common) {
            color_mode::set_palette_entry(s, i, *c);
        }
        s.palette.truncate(colors.len());
        for c in &colors[common..] {
            s.palette.push(*c);
        }
        color_mode::conform_all(s);
    } else {
        s.palette = colors.to_vec();
    }
    Ok(())
}

impl UserData for PaletteH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("frame", |_, t| Ok(matches!(t.kind, Kind::Sprite(_)).then_some(1)));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::Len, |_, t, ()| Ok(t.len()));
        m.add_method("getColor", |_, t, i: i64| {
            let i = t.index(i)?;
            Ok(ColorH::from_rgba(&t.ctx, t.colors()[i]))
        });
        m.add_method("setColor", |_, t, (i, c): (i64, color::PxColor)| {
            let i = t.index(i)?;
            match &t.kind {
                Kind::Sprite(ctx) => {
                    color_mode::set_palette_entry(ctx.sprite(), i, c.0);
                }
                Kind::Own(v) => v.borrow_mut()[i] = c.0,
            }
            Ok(())
        });
        m.add_method("resize", |_, t, n: i64| {
            if n < 1 || n as usize > MAX_COLORS {
                return Err(Error::runtime(format!("a palette has 1..{MAX_COLORS} colors")));
            }
            let n = n as usize;
            match &t.kind {
                Kind::Sprite(ctx) => {
                    let s = ctx.sprite();
                    s.palette.resize(n, [0, 0, 0, 255]);
                    if s.color_mode == ColorMode::Indexed {
                        color_mode::conform_all(s);
                    }
                }
                Kind::Own(v) => v.borrow_mut().resize(n, [0, 0, 0, 255]),
            }
            Ok(())
        });
        m.add_method("clone", |_, t, ()| {
            Ok(PaletteH { ctx: t.ctx.clone(), kind: Kind::Own(RefCell::new(t.colors())) })
        });
        m.add_method("saveAs", |_, t, path: String| save_palette_file(&t.ctx, &path, &t.colors()));
    }
}

pub(super) fn install(lua: &Lua, ctx: &Rc<Ctx>) -> LuaResult<()> {
    let c = ctx.clone();
    lua.globals().set(
        "Palette",
        lua.create_function(move |_, a: MultiValue| -> LuaResult<PaletteH> {
            let bad = || Error::runtime("Palette: expected (ncolors), {fromFile=path} or a Palette");
            let v: Vec<Value> = a.into_iter().collect();
            let colors = match v.as_slice() {
                [] => vec![[0, 0, 0, 255]; 256],
                [Value::Table(t)] => {
                    let Ok(Value::String(p)) = t.raw_get::<Value>("fromFile") else { return Err(bad()) };
                    let path = access::check(&c, &p.to_str()?, Access::Read)?;
                    let colors = read_palette_file(&path).map_err(Error::runtime)?;
                    if colors.is_empty() {
                        return Err(Error::runtime("the palette file has no colors"));
                    }
                    colors
                }
                [Value::UserData(u)] => u.borrow::<PaletteH>().map_err(|_| bad())?.colors(),
                [n] => {
                    let n = super::geometry::int_of(n).ok_or_else(bad)?;
                    if n < 1 || n as usize > MAX_COLORS {
                        return Err(Error::runtime(format!("a palette has 1..{MAX_COLORS} colors")));
                    }
                    vec![[0, 0, 0, 255]; n as usize]
                }
                _ => return Err(bad()),
            };
            Ok(PaletteH { ctx: c.clone(), kind: Kind::Own(RefCell::new(colors)) })
        })?,
    )
}

pub(super) fn add_fields<F: UserDataFields<SpriteH>>(f: &mut F) {
    // Pixmith sprites have one palette for all frames.
    f.add_field_method_get("palettes", |lua, t| {
        let p = PaletteH { ctx: t.0.clone(), kind: Kind::Sprite(t.0.clone()) };
        lua.create_sequence_from([p])
    });
}

pub(super) fn add_methods<M: UserDataMethods<SpriteH>>(m: &mut M) {
    m.add_method("setPalette", |_, t, p: Value| {
        let Value::UserData(u) = p else { return Err(Error::runtime("setPalette expects a Palette")) };
        let pal = u.borrow::<PaletteH>().map_err(|_| Error::runtime("setPalette expects a Palette"))?;
        assign(&t.0, &pal.colors())
    });
    m.add_method("loadPalette", |_, t, path: String| {
        let p = access::check(&t.0, &path, Access::Read)?;
        let colors = read_palette_file(&p).map_err(Error::runtime)?;
        assign(&t.0, &colors)
    });
}
