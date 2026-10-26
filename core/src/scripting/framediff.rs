//! `app.frameDiff{ frame=, layer=, wrap=, mask= }` (`wrap` = "tag" (default), "document", "none", true or false): which pixels a frame changed compared with the frame before
//! it (see `render::diff`). Read-only; works on the active document.

use super::geometry::RectV;
use super::image::ImageH;
use super::sprites::active_ctx;
use super::Ctx;
use crate::render::diff::{self, DiffSource, DiffWrap, DEFAULT_COLORS};
use mlua::{Error, Lua, Result as LuaResult, Table, Value};
use std::rc::Rc;

fn frame_diff(lua: &Lua, root: &Rc<Ctx>, p: Option<Table>) -> LuaResult<Table> {
    let act = active_ctx(root);
    let get = |k: &str| p.as_ref().and_then(|t| t.raw_get::<Value>(k).ok()).unwrap_or(Value::Nil);
    let frame = match get("frame") {
        Value::Nil => act.frame_index(None)?,
        v => act.frame_of(&v)?,
    };
    let (source, layer) = match get("layer") {
        Value::Nil => (DiffSource::Composite, None),
        v => {
            let l = act.layer_of(&v)?;
            if act.sprite().layers[l].is_group() {
                return Err(Error::runtime("groups have no pixels"));
            }
            (DiffSource::Layer(l), Some(l))
        }
    };
    let wrap = match get("wrap") {
        Value::Nil => DiffWrap::Tag,
        Value::Boolean(b) => b.into(),
        Value::String(s) => match s.to_str()?.to_ascii_lowercase().as_str() {
            "tag" => DiffWrap::Tag,
            "document" => DiffWrap::Document,
            "none" => DiffWrap::None,
            _ => return Err(Error::runtime("frameDiff: wrap must be \"tag\", \"document\", \"none\", true or false")),
        },
        _ => return Err(Error::runtime("frameDiff: wrap must be \"tag\", \"document\", \"none\", true or false")),
    };
    let want_mask = matches!(get("mask"), Value::Boolean(true));
    act.sprite().flush_links();
    let d = diff::frame_diff(act.sprite(), frame, source, wrap)
        .ok_or_else(|| Error::runtime("frameDiff: the frame or layer does not exist"))?;

    let out = lua.create_table()?;
    out.set("frame", frame + 1)?;
    out.set("previous", d.reference.map(|f| f + 1))?;
    out.set("layer", layer.map(|l| l + 1))?;
    out.set("changed", d.changed)?;
    out.set("appeared", d.appeared)?;
    out.set("disappeared", d.disappeared)?;
    out.set("total", d.total())?;
    out.set("bounds", d.bounds.map(|(x, y, w, h)| RectV::new(x as i64, y as i64, w as i64, h as i64)))?;
    if want_mask {
        // Opaque magenta (changed), green (appeared), red (disappeared) on a transparent image.
        let px = d.to_rgba(DEFAULT_COLORS, 255);
        out.set("mask", ImageH::own(root, d.width, d.height, Some(px), 0)?)?;
    }
    Ok(out)
}

pub(super) fn install(lua: &Lua, ctx: &Rc<Ctx>) -> LuaResult<()> {
    let app: Table = lua.globals().get("app")?;
    let c = ctx.clone();
    app.set("frameDiff", lua.create_function(move |lua, p: Option<Table>| frame_diff(lua, &c, p))?)?;
    Ok(())
}
