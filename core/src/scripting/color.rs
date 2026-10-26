//! Aseprite's `Color` object and `app.pixelColor` for Lua scripts, and the color argument every
//! Pixmith API that takes a color shares.
//!
//! Three integer formats meet here, so be exact about which one a function speaks:
//!
//! * **Pixmith color integers** are `0xRRGGBBAA` (`0xFF0000FF` is opaque red). Every function that existed
//!   before `Color` did (`sprite:setPixel`, `sprite:getPixel`, `setPaletteColor`, `color.rgba`, `app.fgColor = n`,
//!   dialog colors, ...) keeps that meaning, and also accepts a `Color` object wherever it takes a color.
//! * **Aseprite pixel values** are `0xAABBGGRR` (`app.pixelColor.rgba(r, g, b, a)`). `Color(int)`, the `Image`
//!   pixel functions (`getPixel`, `putPixel`, `drawPixel`, `clear`, `isPlain`, `pixels()`) and `app.pixelColor`
//!   speak this one, exactly as in Aseprite.
//! * `Color` objects, which carry a color model (RGB, HSV, HSL, gray or palette index) like Aseprite's.
//!
//! `color.fromPixel(px)` and `color.toPixel(c)` convert between the first two.

use super::geometry::int_of;
use super::Ctx;
use mlua::{
    Error, FromLua, Lua, MetaMethod, MultiValue, Result as LuaResult, Table, UserData, UserDataFields, UserDataMethods,
    Value,
};
use std::cell::Cell;
use std::rc::Rc;

/// A color argument: a Pixmith `0xRRGGBBAA` integer or a `Color` object, as RGBA bytes.
#[derive(Clone, Copy, Debug)]
pub(super) struct PxColor(pub [u8; 4]);

impl FromLua for PxColor {
    fn from_lua(value: Value, _: &Lua) -> LuaResult<Self> {
        arg(&value).map(PxColor)
    }
}

/// RGBA bytes from a Pixmith color integer or a `Color` object.
pub(super) fn arg(v: &Value) -> LuaResult<[u8; 4]> {
    match v {
        Value::UserData(u) => {
            if let Ok(c) = u.borrow::<ColorH>() {
                return Ok(c.rgba());
            }
            Err(Error::runtime("expected a color (a Color or a 0xRRGGBBAA number)"))
        }
        other => match int_of(other) {
            Some(i) => super::color_arg(i),
            None => Err(Error::runtime("expected a color (a Color or a 0xRRGGBBAA number)")),
        },
    }
}

/// RGBA bytes from an Aseprite pixel value (`0xAABBGGRR`) or a `Color` object (`Image` pixel functions).
pub(super) fn pixel_arg(v: &Value) -> LuaResult<PixelArg> {
    match v {
        Value::UserData(u) => match u.borrow::<ColorH>() {
            Ok(c) => Ok(PixelArg::Rgba(c.rgba())),
            Err(_) => Err(Error::runtime("expected a Color or a pixel value")),
        },
        other => match int_of(other) {
            Some(i) => Ok(PixelArg::Value(i as u32)),
            None => Err(Error::runtime("expected a Color or a pixel value")),
        },
    }
}

/// What an `Image` pixel function was given.
#[derive(Clone, Copy, Debug)]
pub(super) enum PixelArg {
    /// An integer: a pixel value of the image's color mode (RGB: `0xAABBGGRR`).
    Value(u32),
    /// A `Color` object, resolved to RGBA.
    Rgba([u8; 4]),
}

pub(super) fn px_to_pixel(c: [u8; 4]) -> u32 {
    u32::from_le_bytes(c)
}

pub(super) fn pixel_to_px(p: u32) -> [u8; 4] {
    p.to_le_bytes()
}

// MARK: Model

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ColorV {
    Rgb([u8; 4]),
    /// Hue 0..360, saturation and value 0..1, alpha.
    Hsv(f64, f64, f64, u8),
    /// Hue 0..360, saturation and lightness 0..1, alpha.
    Hsl(f64, f64, f64, u8),
    Gray(u8, u8),
    Index(i32),
}

const CLEAR: ColorV = ColorV::Rgb([0, 0, 0, 0]);

fn to_u8(v: f64) -> u8 {
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

fn clamp_u8(v: i64) -> u8 {
    v.clamp(0, 255) as u8
}

fn clamp_f(v: f64) -> u8 {
    if v.is_nan() {
        0
    } else {
        v.round().clamp(0.0, 255.0) as u8
    }
}

pub(super) fn hsv_to_rgb(h: f64, s: f64, v: f64) -> [u8; 3] {
    let (s, v) = (s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    let h = h.rem_euclid(360.0);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [to_u8(r + m), to_u8(g + m), to_u8(b + m)]
}

pub(super) fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [u8; 3] {
    let (s, l) = (s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    let h = h.rem_euclid(360.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [to_u8(r + m), to_u8(g + m), to_u8(b + m)]
}

fn hue_of(r: f64, g: f64, b: f64, max: f64, min: f64) -> f64 {
    let d = max - min;
    if d <= 0.0 {
        return 0.0;
    }
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0).rem_euclid(360.0)
}

pub(super) fn rgb_to_hsv(c: [u8; 4]) -> (f64, f64, f64) {
    let (r, g, b) = (c[0] as f64 / 255.0, c[1] as f64 / 255.0, c[2] as f64 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let s = if max <= 0.0 { 0.0 } else { (max - min) / max };
    (hue_of(r, g, b, max, min), s, max)
}

pub(super) fn rgb_to_hsl(c: [u8; 4]) -> (f64, f64, f64) {
    let (r, g, b) = (c[0] as f64 / 255.0, c[1] as f64 / 255.0, c[2] as f64 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    let s = if d <= 0.0 { 0.0 } else { d / (1.0 - (2.0 * l - 1.0).abs()) };
    (hue_of(r, g, b, max, min), s.clamp(0.0, 1.0), l)
}

/// The nearest palette entry (exact matches win; ties go to the lower index).
pub(super) fn nearest_index(palette: &[[u8; 4]], c: [u8; 4]) -> i32 {
    let mut best = (u64::MAX, 0usize);
    for (i, p) in palette.iter().enumerate() {
        let d: u64 = (0..4)
            .map(|k| {
                let d = p[k] as i64 - c[k] as i64;
                (d * d) as u64
            })
            .sum();
        if d < best.0 {
            best = (d, i);
            if d == 0 {
                break;
            }
        }
    }
    best.1 as i32
}

/// The color object. Mutable (its properties can be assigned), like Aseprite's.
pub(super) struct ColorH(pub Cell<ColorV>, pub Rc<Ctx>);

impl ColorH {
    pub(super) fn new(ctx: &Rc<Ctx>, v: ColorV) -> ColorH {
        ColorH(Cell::new(v), ctx.clone())
    }

    pub(super) fn from_rgba(ctx: &Rc<Ctx>, c: [u8; 4]) -> ColorH {
        ColorH::new(ctx, ColorV::Rgb(c))
    }

    /// The palette colors an `Index` color refers to: the active sprite's.
    fn palette(&self) -> Vec<[u8; 4]> {
        super::sprites::active_ctx(&self.1).sprite().palette.clone()
    }

    pub(super) fn rgba(&self) -> [u8; 4] {
        match self.0.get() {
            ColorV::Index(i) => super::sprites::active_ctx(&self.1)
                .sprite()
                .palette
                .get(usize::try_from(i).unwrap_or(usize::MAX))
                .copied()
                .unwrap_or([0, 0, 0, 255]),
            v => resolve(v),
        }
    }

    fn alpha(&self) -> u8 {
        match self.0.get() {
            ColorV::Rgb(c) => c[3],
            ColorV::Hsv(.., a) | ColorV::Hsl(.., a) | ColorV::Gray(_, a) => a,
            ColorV::Index(_) => self.rgba()[3],
        }
    }

    fn set_alpha(&self, a: u8) {
        self.0.set(match self.0.get() {
            ColorV::Rgb(c) => ColorV::Rgb([c[0], c[1], c[2], a]),
            ColorV::Hsv(h, s, v, _) => ColorV::Hsv(h, s, v, a),
            ColorV::Hsl(h, s, l, _) => ColorV::Hsl(h, s, l, a),
            ColorV::Gray(g, _) => ColorV::Gray(g, a),
            ColorV::Index(i) => {
                let c = self.rgba();
                let _ = i;
                ColorV::Rgb([c[0], c[1], c[2], a])
            }
        });
    }

    fn hsv(&self) -> (f64, f64, f64) {
        match self.0.get() {
            ColorV::Hsv(h, s, v, _) => (h, s, v),
            _ => rgb_to_hsv(self.rgba()),
        }
    }

    fn hsl(&self) -> (f64, f64, f64) {
        match self.0.get() {
            ColorV::Hsl(h, s, l, _) => (h, s, l),
            _ => rgb_to_hsl(self.rgba()),
        }
    }

    fn gray(&self) -> u8 {
        match self.0.get() {
            ColorV::Gray(g, _) => g,
            _ => crate::document::color_mode::luma(self.rgba()),
        }
    }

    fn index(&self) -> i32 {
        match self.0.get() {
            ColorV::Index(i) => i,
            _ => nearest_index(&self.palette(), self.rgba()),
        }
    }

    fn describe(&self) -> String {
        match self.0.get() {
            ColorV::Rgb(c) => format!("Color{{ r={}, g={}, b={}, a={} }}", c[0], c[1], c[2], c[3]),
            ColorV::Hsv(h, s, v, a) => format!("Color{{ h={}, s={}, v={}, a={a} }}", g4(h), g4(s), g4(v)),
            ColorV::Hsl(h, s, l, a) => format!("Color{{ h={}, s={}, l={}, a={a} }}", g4(h), g4(s), g4(l)),
            ColorV::Gray(g, a) => format!("Color{{ gray={g}, a={a} }}"),
            ColorV::Index(i) => format!("Color{{ index={i} }}"),
        }
    }

    /// The Pixmith `0xRRGGBBAA` integer.
    fn pixmith(&self) -> i64 {
        u32::from_be_bytes(self.rgba()) as i64
    }
}

/// `%.4g`-style formatting (4 significant digits, no trailing zeros).
fn g4(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let digits = 3 - v.abs().log10().floor() as i32;
    let p = digits.clamp(0, 12) as usize;
    let s = format!("{v:.p$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

fn resolve(v: ColorV) -> [u8; 4] {
    match v {
        ColorV::Rgb(c) => c,
        ColorV::Hsv(h, s, v, a) => {
            let [r, g, b] = hsv_to_rgb(h, s, v);
            [r, g, b, a]
        }
        ColorV::Hsl(h, s, l, a) => {
            let [r, g, b] = hsl_to_rgb(h, s, l);
            [r, g, b, a]
        }
        ColorV::Gray(g, a) => [g, g, g, a],
        ColorV::Index(_) => [0, 0, 0, 255],
    }
}

// MARK: Constructor

fn num_field(t: &Table, names: &[&str]) -> Option<f64> {
    names.iter().find_map(|n| match t.raw_get::<Value>(*n) {
        Ok(Value::Integer(i)) => Some(i as f64),
        Ok(Value::Number(n)) if n.is_finite() => Some(n),
        _ => None,
    })
}

fn int_field(t: &Table, names: &[&str]) -> Option<i64> {
    num_field(t, names).map(|n| n.round() as i64)
}

fn alpha_field(t: &Table) -> u8 {
    int_field(t, &["alpha", "a"]).map_or(255, clamp_u8)
}

fn parse_hex(s: &str) -> Option<[u8; 4]> {
    let h = s.trim().trim_start_matches('#');
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let d = |i: usize| u8::from_str_radix(&h[i..i + 1].repeat(2), 16).ok();
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    match h.len() {
        3 => Some([d(0)?, d(1)?, d(2)?, 255]),
        4 => Some([d(0)?, d(1)?, d(2)?, d(3)?]),
        6 => Some([p(0)?, p(2)?, p(4)?, 255]),
        8 => Some([p(0)?, p(2)?, p(4)?, p(6)?]),
        _ => None,
    }
}

fn from_table(t: &Table) -> LuaResult<ColorV> {
    if let Some(i) = int_field(t, &["index"]) {
        return Ok(ColorV::Index(i.clamp(0, i32::MAX as i64) as i32));
    }
    if let Some(g) = int_field(t, &["gray", "grey"]) {
        return Ok(ColorV::Gray(clamp_u8(g), alpha_field(t)));
    }
    let a = alpha_field(t);
    let hue = num_field(t, &["hue", "h"]);
    if let Some(h) = hue {
        let s = num_field(t, &["saturation", "s"]).unwrap_or(0.0);
        if let Some(v) = num_field(t, &["value", "v"]) {
            return Ok(ColorV::Hsv(h.rem_euclid(360.0), s.clamp(0.0, 1.0), v.clamp(0.0, 1.0), a));
        }
        if let Some(l) = num_field(t, &["lightness", "l"]) {
            return Ok(ColorV::Hsl(h.rem_euclid(360.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0), a));
        }
        return Err(Error::runtime("Color{ hue=... } needs value= (HSV) or lightness= (HSL)"));
    }
    let ch = |long: &str, short: &str| int_field(t, &[long, short]).map_or(0, clamp_u8);
    Ok(ColorV::Rgb([ch("red", "r"), ch("green", "g"), ch("blue", "b"), a]))
}

fn construct(a: MultiValue) -> LuaResult<ColorV> {
    let v: Vec<Value> = a.into_iter().collect();
    // Color(...) is called as a function; `Color{...}` passes one table.
    match v.as_slice() {
        [] => Ok(CLEAR),
        [Value::Table(t)] => from_table(t),
        [Value::String(s)] => parse_hex(&s.to_str()?)
            .map(ColorV::Rgb)
            .ok_or_else(|| Error::runtime("Color: expected \"#rrggbb\", \"#rrggbbaa\", \"#rgb\" or \"#rgba\"")),
        [x] => match int_of(x) {
            // Color(pixelValue): Aseprite's 0xAABBGGRR.
            Some(i) => Ok(ColorV::Rgb(pixel_to_px(i as u32))),
            None => Err(Error::runtime("Color: expected (r, g, b [, a]), a table or a pixel value")),
        },
        rest if (3..=4).contains(&rest.len()) => {
            let mut ch = [0u8, 0, 0, 255];
            for (i, x) in rest.iter().enumerate() {
                ch[i] = clamp_u8(
                    int_of(x).ok_or_else(|| Error::runtime("Color(r, g, b [, a]): the channels must be numbers"))?,
                );
            }
            Ok(ColorV::Rgb(ch))
        }
        _ => Err(Error::runtime("Color: expected (r, g, b [, a]), a table or a pixel value")),
    }
}

/// A `Color` object for RGBA bytes, made from inside a Lua callback that has no `Ctx` at hand.
pub(super) fn new_color(lua: &Lua, c: [u8; 4]) -> LuaResult<mlua::AnyUserData> {
    let ctx = lua
        .app_data_ref::<Rc<Ctx>>()
        .map(|c| c.clone())
        .ok_or_else(|| Error::runtime("internal error: no script context"))?;
    lua.create_userdata(ColorH::from_rgba(&ctx, c))
}

// MARK: Lua API

fn other_color(v: &Value) -> Option<ColorV> {
    match v {
        Value::UserData(u) => u.borrow::<ColorH>().ok().map(|c| c.0.get()),
        _ => None,
    }
}

impl UserData for ColorH {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        macro_rules! rgb_channel {
            ($long:literal, $short:literal, $i:literal) => {
                f.add_field_method_get($long, |_, t| Ok(t.rgba()[$i]));
                f.add_field_method_get($short, |_, t| Ok(t.rgba()[$i]));
                f.add_field_method_set($long, |_, t, v: f64| {
                    let mut c = t.rgba();
                    c[$i] = clamp_f(v);
                    t.0.set(ColorV::Rgb(c));
                    Ok(())
                });
                f.add_field_method_set($short, |_, t, v: f64| {
                    let mut c = t.rgba();
                    c[$i] = clamp_f(v);
                    t.0.set(ColorV::Rgb(c));
                    Ok(())
                });
            };
        }
        rgb_channel!("red", "r", 0);
        rgb_channel!("green", "g", 1);
        rgb_channel!("blue", "b", 2);
        f.add_field_method_get("alpha", |_, t| Ok(t.alpha()));
        f.add_field_method_get("a", |_, t| Ok(t.alpha()));
        f.add_field_method_set("alpha", |_, t, v: f64| {
            t.set_alpha(clamp_f(v));
            Ok(())
        });
        f.add_field_method_set("a", |_, t, v: f64| {
            t.set_alpha(clamp_f(v));
            Ok(())
        });

        // `hue`, `saturation`, `value` are the HSV components, `lightness` the HSL one (as in Aseprite).
        f.add_field_method_get("hue", |_, t| Ok(t.hsv().0));
        f.add_field_method_get("hsvHue", |_, t| Ok(t.hsv().0));
        f.add_field_method_get("saturation", |_, t| Ok(t.hsv().1));
        f.add_field_method_get("hsvSaturation", |_, t| Ok(t.hsv().1));
        f.add_field_method_get("value", |_, t| Ok(t.hsv().2));
        f.add_field_method_get("hsvValue", |_, t| Ok(t.hsv().2));
        f.add_field_method_get("hslHue", |_, t| Ok(t.hsl().0));
        f.add_field_method_get("hslSaturation", |_, t| Ok(t.hsl().1));
        f.add_field_method_get("lightness", |_, t| Ok(t.hsl().2));
        f.add_field_method_get("hslLightness", |_, t| Ok(t.hsl().2));
        for name in ["hue", "hsvHue"] {
            f.add_field_method_set(name, |_, t, v: f64| {
                let (_, s, val) = t.hsv();
                t.0.set(ColorV::Hsv(v.rem_euclid(360.0), s, val, t.alpha()));
                Ok(())
            });
        }
        for name in ["saturation", "hsvSaturation"] {
            f.add_field_method_set(name, |_, t, v: f64| {
                let (h, _, val) = t.hsv();
                t.0.set(ColorV::Hsv(h, v.clamp(0.0, 1.0), val, t.alpha()));
                Ok(())
            });
        }
        for name in ["value", "hsvValue"] {
            f.add_field_method_set(name, |_, t, v: f64| {
                let (h, s, _) = t.hsv();
                t.0.set(ColorV::Hsv(h, s, v.clamp(0.0, 1.0), t.alpha()));
                Ok(())
            });
        }
        f.add_field_method_set("hslHue", |_, t, v: f64| {
            let (_, s, l) = t.hsl();
            t.0.set(ColorV::Hsl(v.rem_euclid(360.0), s, l, t.alpha()));
            Ok(())
        });
        f.add_field_method_set("hslSaturation", |_, t, v: f64| {
            let (h, _, l) = t.hsl();
            t.0.set(ColorV::Hsl(h, v.clamp(0.0, 1.0), l, t.alpha()));
            Ok(())
        });
        for name in ["lightness", "hslLightness"] {
            f.add_field_method_set(name, |_, t, v: f64| {
                let (h, s, _) = t.hsl();
                t.0.set(ColorV::Hsl(h, s, v.clamp(0.0, 1.0), t.alpha()));
                Ok(())
            });
        }
        f.add_field_method_get("gray", |_, t| Ok(t.gray()));
        f.add_field_method_set("gray", |_, t, v: f64| {
            t.0.set(ColorV::Gray(clamp_f(v), t.alpha()));
            Ok(())
        });
        f.add_field_method_get("index", |_, t| Ok(t.index()));
        f.add_field_method_set("index", |_, t, v: f64| {
            t.0.set(ColorV::Index(v.clamp(0.0, i32::MAX as f64) as i32));
            Ok(())
        });
        // 0xAABBGGRR, like `app.pixelColor.rgba(...)`.
        f.add_field_method_get("rgbaPixel", |_, t| Ok(px_to_pixel(t.rgba()) as i64));
        // gray | alpha << 8, like `app.pixelColor.graya(...)`.
        f.add_field_method_get("grayPixel", |_, t| Ok(t.gray() as i64 | (t.alpha() as i64) << 8));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, t, ()| Ok(t.describe()));
        m.add_meta_method(MetaMethod::Eq, |_, t, o: Value| {
            let Some(o) = other_color(&o) else { return Ok(false) };
            Ok(match (t.0.get(), o) {
                (ColorV::Index(a), ColorV::Index(b)) => a == b,
                (a, b) => {
                    let ra = if matches!(a, ColorV::Index(_)) { t.rgba() } else { resolve(a) };
                    let rb = if matches!(b, ColorV::Index(_)) { t.rgba() } else { resolve(b) };
                    ra == rb
                }
            })
        });
        // A Color converts to its Pixmith 0xRRGGBBAA integer in bit operations, so older scripts that did
        // `(c >> 24) & 0xFF` on `app.fgColor` keep working.
        m.add_meta_method(MetaMethod::Shr, |_, t, n: i64| Ok(shift(t.pixmith(), -n)));
        m.add_meta_method(MetaMethod::Shl, |_, t, n: i64| Ok(shift(t.pixmith(), n)));
        m.add_meta_method(MetaMethod::BAnd, |_, t, n: i64| Ok(t.pixmith() & n));
        m.add_meta_method(MetaMethod::BOr, |_, t, n: i64| Ok(t.pixmith() | n));
        m.add_meta_method(MetaMethod::BXor, |_, t, n: i64| Ok(t.pixmith() ^ n));
        m.add_meta_method(MetaMethod::BNot, |_, t, _: Value| Ok(!t.pixmith()));
        m.add_meta_method(MetaMethod::Concat, |_, t, o: Value| {
            let me = t.describe();
            Ok(match o {
                Value::String(s) => format!("{me}{}", s.to_string_lossy()),
                Value::Integer(i) => format!("{me}{i}"),
                Value::Number(n) => format!("{me}{n}"),
                _ => me,
            })
        });
    }
}

fn shift(v: i64, by: i64) -> i64 {
    if by >= 64 || by <= -64 {
        0
    } else if by >= 0 {
        ((v as u64) << by) as i64
    } else {
        ((v as u64) >> -by) as i64
    }
}

// MARK: Installation

pub(super) fn install(lua: &Lua, ctx: &Rc<Ctx>) -> LuaResult<()> {
    let g = lua.globals();
    lua.set_app_data(ctx.clone());
    let c = ctx.clone();
    g.set("Color", lua.create_function(move |_, a: MultiValue| Ok(ColorH::new(&c, construct(a)?)))?)?;

    // app.pixelColor: Aseprite's pixel packing for RGB (0xAABBGGRR) and grayscale (value | alpha << 8) images.
    let pc = lua.create_table()?;
    let ch = |v: i64| (v & 0xFF) as u32;
    pc.set(
        "rgba",
        lua.create_function(move |_, (r, g, b, a): (i64, i64, i64, Option<i64>)| {
            Ok((ch(r) | ch(g) << 8 | ch(b) << 16 | ch(a.unwrap_or(255)) << 24) as i64)
        })?,
    )?;
    let px = |p: i64| p as u32;
    pc.set("rgbaR", lua.create_function(move |_, p: i64| Ok(px(p) & 0xFF))?)?;
    pc.set("rgbaG", lua.create_function(move |_, p: i64| Ok((px(p) >> 8) & 0xFF))?)?;
    pc.set("rgbaB", lua.create_function(move |_, p: i64| Ok((px(p) >> 16) & 0xFF))?)?;
    pc.set("rgbaA", lua.create_function(move |_, p: i64| Ok((px(p) >> 24) & 0xFF))?)?;
    pc.set("graya", lua.create_function(move |_, (v, a): (i64, Option<i64>)| Ok(ch(v) | ch(a.unwrap_or(255)) << 8))?)?;
    pc.set("grayaV", lua.create_function(move |_, p: i64| Ok(px(p) & 0xFF))?)?;
    pc.set("grayaA", lua.create_function(move |_, p: i64| Ok((px(p) >> 8) & 0xFF))?)?;
    let app: Table = g.get("app")?;
    app.set("pixelColor", pc)?;

    // The Pixmith `color` helper table: accepts Color objects too, and bridges to Aseprite pixel values.
    let color: Table = g.get("color")?;
    color.set(
        "split",
        lua.create_function(|_, v: Value| {
            let [r, g, b, a] = arg(&v)?;
            Ok((r, g, b, a))
        })?,
    )?;
    color.set(
        "rgba",
        lua.create_function(|_, (r, g, b, a): (i64, i64, i64, Option<i64>)| {
            let ch = |v: i64| v.clamp(0, 255) as u32;
            Ok((ch(r) << 24 | ch(g) << 16 | ch(b) << 8 | ch(a.unwrap_or(255))) as i64)
        })?,
    )?;
    // Aseprite pixel value (0xAABBGGRR) -> Pixmith integer (0xRRGGBBAA)
    color.set("fromPixel", lua.create_function(|_, p: i64| Ok(u32::from_be_bytes(pixel_to_px(p as u32)) as i64))?)?;
    // Pixmith integer or Color -> Aseprite pixel value
    color.set("toPixel", lua.create_function(|_, v: Value| Ok(px_to_pixel(arg(&v)?) as i64))?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_roundtrip() {
        for c in [[255u8, 0, 0, 255], [10, 200, 90, 255], [128, 128, 128, 255], [0, 0, 0, 255], [255, 255, 255, 255]] {
            let (h, s, v) = rgb_to_hsv(c);
            let [r, g, b] = hsv_to_rgb(h, s, v);
            assert_eq!([r, g, b], [c[0], c[1], c[2]]);
            let (h, s, l) = rgb_to_hsl(c);
            let [r, g, b] = hsl_to_rgb(h, s, l);
            assert_eq!([r, g, b], [c[0], c[1], c[2]]);
        }
    }

    #[test]
    fn g4_formats_like_printf() {
        assert_eq!(g4(120.0), "120");
        assert_eq!(g4(0.5), "0.5");
        assert_eq!(g4(0.123456), "0.1235");
        assert_eq!(g4(359.99), "360");
    }
}
