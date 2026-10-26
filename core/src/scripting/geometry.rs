//! Aseprite's `Point`, `Size` and `Rectangle` value types for Lua scripts, and the argument
//! parsing every API that takes a point, size or rectangle shares.
//!
//! The objects are copies: changing a field of one does not change the object it was built from.
//! Every function that takes one also takes a plain table (`{x=, y=}`, `{x=, y=, w=, h=}`,
//! `{width=, height=}` or the array forms `{x, y}` / `{x, y, w, h}`), and the geometry methods that
//! used to take loose numbers (`selectRect(x, y, w, h)`, `setBounds(x, y, w, h)`) still do.

use mlua::{
    Error, Lua, MetaMethod, MultiValue, Result as LuaResult, Table, UserData, UserDataFields, UserDataMethods, Value,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PointV {
    pub x: i64,
    pub y: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SizeV {
    pub w: i64,
    pub h: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RectV {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

impl RectV {
    pub fn new(x: i64, y: i64, w: i64, h: i64) -> RectV {
        RectV { x, y, w, h }
    }

    /// Aseprite: a rectangle with no area.
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    fn right(&self) -> i64 {
        self.x.saturating_add(self.w)
    }

    fn bottom(&self) -> i64 {
        self.y.saturating_add(self.h)
    }

    pub fn contains_point(&self, x: i64, y: i64) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    pub fn contains_rect(&self, r: &RectV) -> bool {
        r.x >= self.x && r.y >= self.y && r.right() <= self.right() && r.bottom() <= self.bottom()
    }

    /// The overlap, or `0,0,0,0` when there is none.
    pub fn intersection(&self, r: &RectV) -> RectV {
        let (x1, y1) = (self.x.max(r.x), self.y.max(r.y));
        let (x2, y2) = (self.right().min(r.right()), self.bottom().min(r.bottom()));
        if x2 <= x1 || y2 <= y1 {
            RectV::default()
        } else {
            RectV::new(x1, y1, x2 - x1, y2 - y1)
        }
    }

    /// The smallest rectangle holding both; an empty one does not count.
    pub fn union(&self, r: &RectV) -> RectV {
        if self.is_empty() {
            return *r;
        }
        if r.is_empty() {
            return *self;
        }
        let (x1, y1) = (self.x.min(r.x), self.y.min(r.y));
        let (x2, y2) = (self.right().max(r.right()), self.bottom().max(r.bottom()));
        RectV::new(x1, y1, x2 - x1, y2 - y1)
    }
}

// MARK: Argument parsing

/// A Lua number as an integer (fractions are dropped); `None` for anything else.
pub fn int_of(v: &Value) -> Option<i64> {
    match v {
        Value::Integer(i) => Some(*i),
        Value::Number(n) if n.is_finite() => Some(n.trunc() as i64),
        _ => None,
    }
}

fn is_number(v: &Value) -> bool {
    matches!(v, Value::Integer(_) | Value::Number(_))
}

/// The first of `names` present in the table as a number, else element `idx`.
fn field(t: &Table, names: &[&str], idx: i64) -> Option<i64> {
    names
        .iter()
        .find_map(|n| t.raw_get::<Value>(*n).ok().as_ref().and_then(int_of))
        .or_else(|| t.raw_get::<Value>(idx).ok().as_ref().and_then(int_of))
}

pub fn point_of(v: &Value) -> Option<(i64, i64)> {
    match v {
        Value::UserData(u) => u.borrow::<PointV>().ok().map(|p| (p.x, p.y)),
        Value::Table(t) => Some((field(t, &["x"], 1)?, field(t, &["y"], 2)?)),
        _ => None,
    }
}

pub fn size_of(v: &Value) -> Option<(i64, i64)> {
    match v {
        Value::UserData(u) => u.borrow::<SizeV>().ok().map(|s| (s.w, s.h)),
        Value::Table(t) => Some((field(t, &["width", "w"], 1)?, field(t, &["height", "h"], 2)?)),
        _ => None,
    }
}

pub fn rect_of(v: &Value) -> Option<RectV> {
    match v {
        Value::UserData(u) => u.borrow::<RectV>().ok().map(|r| *r),
        Value::Table(t) => Some(RectV::new(
            field(t, &["x"], 1)?,
            field(t, &["y"], 2)?,
            field(t, &["width", "w"], 3)?,
            field(t, &["height", "h"], 4)?,
        )),
        _ => None,
    }
}

fn pop_int(a: &mut MultiValue, what: &str) -> LuaResult<i64> {
    let v = a.pop_front().unwrap_or(Value::Nil);
    int_of(&v).ok_or_else(|| Error::runtime(format!("{what}: expected a number")))
}

/// Takes a rectangle off the front of the arguments: one Rectangle (or table) or four numbers.
pub fn take_rect(a: &mut MultiValue, what: &str) -> LuaResult<RectV> {
    match a.front() {
        Some(v) if is_number(v) => {
            if a.len() < 4 {
                return Err(Error::runtime(format!("{what}: expected a Rectangle or x, y, width, height")));
            }
            let (x, y) = (pop_int(a, what)?, pop_int(a, what)?);
            Ok(RectV::new(x, y, pop_int(a, what)?, pop_int(a, what)?))
        }
        Some(_) => {
            let v = a.pop_front().unwrap_or(Value::Nil);
            rect_of(&v).ok_or_else(|| Error::runtime(format!("{what}: expected a Rectangle or x, y, width, height")))
        }
        None => Err(Error::runtime(format!("{what}: expected a Rectangle or x, y, width, height"))),
    }
}

/// Takes a point off the front of the arguments: one Point (or table) or two numbers.
pub fn take_point(a: &mut MultiValue, what: &str) -> LuaResult<PointV> {
    match a.front() {
        Some(v) if is_number(v) => {
            if a.len() < 2 {
                return Err(Error::runtime(format!("{what}: expected a Point or x, y")));
            }
            let x = pop_int(a, what)?;
            Ok(PointV { x, y: pop_int(a, what)? })
        }
        Some(_) => {
            let v = a.pop_front().unwrap_or(Value::Nil);
            point_of(&v)
                .map(|(x, y)| PointV { x, y })
                .ok_or_else(|| Error::runtime(format!("{what}: expected a Point or x, y")))
        }
        None => Err(Error::runtime(format!("{what}: expected a Point or x, y"))),
    }
}

/// Takes an optional integer: nothing or nil gives `None`.
pub fn take_opt_int(a: &mut MultiValue, what: &str) -> LuaResult<Option<i64>> {
    match a.pop_front() {
        None | Some(Value::Nil) => Ok(None),
        Some(v) => int_of(&v).map(Some).ok_or_else(|| Error::runtime(format!("{what}: expected a number"))),
    }
}

fn int_arg(v: &Value, what: &str) -> LuaResult<i64> {
    int_of(v).ok_or_else(|| Error::runtime(format!("{what} must be a number")))
}

// MARK: Constructors

fn point_ctor(mut a: MultiValue) -> LuaResult<PointV> {
    match a.front() {
        None | Some(Value::Nil) => Ok(PointV::default()),
        Some(v) if is_number(v) => {
            let x = pop_int(&mut a, "Point")?;
            let y = if a.is_empty() { 0 } else { pop_int(&mut a, "Point")? };
            Ok(PointV { x, y })
        }
        Some(v) => point_of(v)
            .map(|(x, y)| PointV { x, y })
            .ok_or_else(|| Error::runtime("Point: expected (x, y), {x=, y=} or a Point")),
    }
}

fn size_ctor(mut a: MultiValue) -> LuaResult<SizeV> {
    match a.front() {
        None | Some(Value::Nil) => Ok(SizeV::default()),
        Some(v) if is_number(v) => {
            let w = pop_int(&mut a, "Size")?;
            let h = if a.is_empty() { 0 } else { pop_int(&mut a, "Size")? };
            Ok(SizeV { w, h })
        }
        Some(v) => size_of(v)
            .map(|(w, h)| SizeV { w, h })
            .ok_or_else(|| Error::runtime("Size: expected (width, height), {width=, height=} or a Size")),
    }
}

fn rect_ctor(mut a: MultiValue) -> LuaResult<RectV> {
    match a.front() {
        None | Some(Value::Nil) => Ok(RectV::default()),
        Some(v) if is_number(v) => {
            let mut n = [0i64; 4];
            for slot in n.iter_mut() {
                if a.is_empty() {
                    break;
                }
                *slot = pop_int(&mut a, "Rectangle")?;
            }
            Ok(RectV::new(n[0], n[1], n[2], n[3]))
        }
        Some(Value::UserData(u)) if u.is::<PointV>() => {
            let p = take_point(&mut a, "Rectangle")?;
            let (w, h) =
                a.front().and_then(size_of).ok_or_else(|| Error::runtime("Rectangle: expected (Point, Size)"))?;
            Ok(RectV::new(p.x, p.y, w, h))
        }
        Some(v) => rect_of(v).ok_or_else(|| {
            Error::runtime("Rectangle: expected (x, y, width, height), {x=, y=, width=, height=} or a Rectangle")
        }),
    }
}

pub fn install(lua: &Lua) -> LuaResult<()> {
    let g = lua.globals();
    g.set("Point", lua.create_function(|_, a: MultiValue| point_ctor(a))?)?;
    g.set("Size", lua.create_function(|_, a: MultiValue| size_ctor(a))?)?;
    g.set("Rectangle", lua.create_function(|_, a: MultiValue| rect_ctor(a))?)?;
    Ok(())
}

// MARK: Userdata

impl UserData for PointV {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("x", |_, t| Ok(t.x));
        f.add_field_method_set("x", |_, t, v: Value| {
            t.x = int_arg(&v, "x")?;
            Ok(())
        });
        f.add_field_method_get("y", |_, t| Ok(t.y));
        f.add_field_method_set("y", |_, t, v: Value| {
            t.y = int_arg(&v, "y")?;
            Ok(())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::Eq, |_, t, o: Value| {
            Ok(matches!(&o, Value::UserData(u) if u.borrow::<PointV>().is_ok_and(|p| *p == *t)))
        });
        m.add_meta_method(MetaMethod::ToString, |_, t, ()| Ok(format!("Point{{ x={}, y={} }}", t.x, t.y)));
        m.add_meta_method(MetaMethod::Add, |_, t, o: Value| {
            let (x, y) = point_of(&o).ok_or_else(|| Error::runtime("Point + expects a Point"))?;
            Ok(PointV { x: t.x.saturating_add(x), y: t.y.saturating_add(y) })
        });
        m.add_meta_method(MetaMethod::Sub, |_, t, o: Value| {
            let (x, y) = point_of(&o).ok_or_else(|| Error::runtime("Point - expects a Point"))?;
            Ok(PointV { x: t.x.saturating_sub(x), y: t.y.saturating_sub(y) })
        });
        m.add_meta_method(MetaMethod::Unm, |_, t, _: Value| {
            Ok(PointV { x: t.x.saturating_neg(), y: t.y.saturating_neg() })
        });
    }
}

impl UserData for SizeV {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        for name in ["width", "w"] {
            f.add_field_method_get(name, |_, t| Ok(t.w));
            f.add_field_method_set(name, |_, t, v: Value| {
                t.w = int_arg(&v, "width")?;
                Ok(())
            });
        }
        for name in ["height", "h"] {
            f.add_field_method_get(name, |_, t| Ok(t.h));
            f.add_field_method_set(name, |_, t, v: Value| {
                t.h = int_arg(&v, "height")?;
                Ok(())
            });
        }
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::Eq, |_, t, o: Value| {
            Ok(matches!(&o, Value::UserData(u) if u.borrow::<SizeV>().is_ok_and(|p| *p == *t)))
        });
        m.add_meta_method(MetaMethod::ToString, |_, t, ()| Ok(format!("Size{{ width={}, height={} }}", t.w, t.h)));
        m.add_method("union", |_, t, o: Value| {
            let (w, h) = size_of(&o).ok_or_else(|| Error::runtime("union expects a Size"))?;
            Ok(SizeV { w: t.w.max(w), h: t.h.max(h) })
        });
    }
}

impl UserData for RectV {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("x", |_, t| Ok(t.x));
        f.add_field_method_set("x", |_, t, v: Value| {
            t.x = int_arg(&v, "x")?;
            Ok(())
        });
        f.add_field_method_get("y", |_, t| Ok(t.y));
        f.add_field_method_set("y", |_, t, v: Value| {
            t.y = int_arg(&v, "y")?;
            Ok(())
        });
        for name in ["width", "w"] {
            f.add_field_method_get(name, |_, t| Ok(t.w));
            f.add_field_method_set(name, |_, t, v: Value| {
                t.w = int_arg(&v, "width")?;
                Ok(())
            });
        }
        for name in ["height", "h"] {
            f.add_field_method_get(name, |_, t| Ok(t.h));
            f.add_field_method_set(name, |_, t, v: Value| {
                t.h = int_arg(&v, "height")?;
                Ok(())
            });
        }
        f.add_field_method_get("origin", |_, t| Ok(PointV { x: t.x, y: t.y }));
        f.add_field_method_set("origin", |_, t, v: Value| {
            let (x, y) = point_of(&v).ok_or_else(|| Error::runtime("origin must be a Point"))?;
            (t.x, t.y) = (x, y);
            Ok(())
        });
        f.add_field_method_get("size", |_, t| Ok(SizeV { w: t.w, h: t.h }));
        f.add_field_method_set("size", |_, t, v: Value| {
            let (w, h) = size_of(&v).ok_or_else(|| Error::runtime("size must be a Size"))?;
            (t.w, t.h) = (w, h);
            Ok(())
        });
        f.add_field_method_get("isEmpty", |_, t| Ok(t.is_empty()));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::Eq, |_, t, o: Value| {
            Ok(matches!(&o, Value::UserData(u) if u.borrow::<RectV>().is_ok_and(|p| *p == *t)))
        });
        m.add_meta_method(MetaMethod::ToString, |_, t, ()| {
            Ok(format!("Rectangle{{ x={}, y={}, width={}, height={} }}", t.x, t.y, t.w, t.h))
        });
        // A rectangle, a point, or loose numbers (x, y) / (x, y, w, h).
        m.add_method("contains", |_, t, mut a: MultiValue| {
            let by_point = match a.front() {
                Some(v) if is_number(v) => a.len() < 4,
                Some(Value::UserData(u)) => u.is::<PointV>(),
                Some(Value::Table(tb)) => rect_of(&Value::Table(tb.clone())).is_none(),
                _ => false,
            };
            if by_point {
                let p = take_point(&mut a, "contains")?;
                Ok(t.contains_point(p.x, p.y))
            } else {
                Ok(t.contains_rect(&take_rect(&mut a, "contains")?))
            }
        });
        m.add_method("intersects", |_, t, mut a: MultiValue| {
            let r = take_rect(&mut a, "intersects")?;
            Ok(!t.intersection(&r).is_empty())
        });
        m.add_method("intersect", |_, t, mut a: MultiValue| Ok(t.intersection(&take_rect(&mut a, "intersect")?)));
        m.add_method("union", |_, t, mut a: MultiValue| Ok(t.union(&take_rect(&mut a, "union")?)));
    }
}

#[cfg(test)]
mod tests {
    use crate::document::{Editor, Sprite};
    use crate::scripting::{run, Limits};

    fn ok(src: &str) {
        let mut e = Editor::new(Sprite::new(8, 8));
        let o = run(&mut e, src, (0, 0), Limits::default());
        assert_eq!(o.error, None, "script failed:\n{src}");
    }

    #[test]
    fn point_constructors_fields_and_operators() {
        ok(r#"
            local p = Point(3, 4)
            assert(p.x == 3 and p.y == 4)
            local q = Point{ x = 1, y = 2 }
            assert(q.x == 1 and q.y == 2)
            local r = Point(p)
            r.x = 10
            assert(p.x == 3 and r.x == 10)               -- Point(other) is a copy
            assert(Point{ 5, 6 }.y == 6)                 -- array form
            assert(Point().x == 0 and Point().y == 0)
            assert(Point(1, 2) == Point(1, 2) and Point(1, 2) ~= Point(2, 1))
            assert(Point(1, 2) ~= Size(1, 2))
            local s = Point(1, 2) + Point(10, 20)
            assert(s == Point(11, 22))
            assert(Point(5, 5) - Point(2, 1) == Point(3, 4))
            assert(-Point(1, -2) == Point(-1, 2))
            assert(tostring(Point(1, 2)) == "Point{ x=1, y=2 }")
            assert(Point(1.9, 2.0).x == 1)               -- fractions are dropped
            assert(Point{ x = 1, y = 2 } + { x = 1, y = 1 } == Point(2, 3))   -- plain tables work as operands
            assert(not pcall(Point, "a"))
            assert(not pcall(function() p.x = "z" end))
        "#);
    }

    #[test]
    fn size_constructors_and_fields() {
        ok(r#"
            local s = Size(3, 4)
            assert(s.width == 3 and s.height == 4 and s.w == 3 and s.h == 4)
            local t = Size{ width = 5, height = 6 }
            assert(t.width == 5 and t.height == 6)
            assert(Size{ w = 7, h = 8 }.height == 8)
            assert(Size(t) == t and Size(1, 2) ~= Size(2, 1))
            t.width = 9
            assert(t.w == 9)
            assert(Size(t).width == 9)
            assert(tostring(Size(1, 2)) == "Size{ width=1, height=2 }")
            assert(Size(1, 5):union(Size(3, 2)) == Size(3, 5))
            assert(Size().width == 0)
        "#);
    }

    #[test]
    fn rectangle_constructors_fields_and_methods() {
        ok(r#"
            local r = Rectangle(1, 2, 3, 4)
            assert(r.x == 1 and r.y == 2 and r.width == 3 and r.height == 4 and r.w == 3 and r.h == 4)
            local t = Rectangle{ x = 1, y = 2, width = 3, height = 4 }
            assert(t == r)
            assert(Rectangle{ x = 1, y = 2, w = 3, h = 4 } == r)
            assert(Rectangle{ 1, 2, 3, 4 } == r)
            assert(Rectangle(r) == r)
            assert(Rectangle(Point(1, 2), Size(3, 4)) == r)
            assert(Rectangle().isEmpty and Rectangle(0, 0, 0, 5).isEmpty and not r.isEmpty)
            assert(r.origin == Point(1, 2) and r.size == Size(3, 4))
            local c = Rectangle(r)
            c.w = 10 c.origin = Point(7, 8)
            assert(c.width == 10 and c.x == 7 and c.y == 8 and r.width == 3)
            assert(tostring(r) == "Rectangle{ x=1, y=2, width=3, height=4 }")

            -- contains: points and rectangles
            assert(r:contains(Point(1, 2)) and r:contains(3, 5) and not r:contains(4, 2) and not r:contains(Point(0, 2)))
            assert(r:contains(Rectangle(2, 3, 1, 1)) and not r:contains(Rectangle(2, 3, 5, 1)))
            assert(r:contains{ x = 2, y = 3, w = 1, h = 1 })
            -- intersects / intersect / union
            local a, b = Rectangle(0, 0, 10, 10), Rectangle(5, 5, 10, 10)
            assert(a:intersects(b) and a:intersect(b) == Rectangle(5, 5, 5, 5))
            assert(not a:intersects(Rectangle(10, 0, 5, 5)) and a:intersect(Rectangle(20, 20, 1, 1)).isEmpty)
            assert(a:union(b) == Rectangle(0, 0, 15, 15))
            assert(Rectangle():union(b) == b and a:union(Rectangle()) == a)
            assert(a:intersects(5, 5, 1, 1))
            assert(a:union{ x = -1, y = 0, w = 1, h = 1 } == Rectangle(-1, 0, 11, 10))
            assert(not pcall(Rectangle, "x"))
            assert(not pcall(function() return a:intersects() end))
        "#);
    }

    #[test]
    fn new_slice_takes_rectangles_and_keeps_the_pixmith_form() {
        let mut e = Editor::new(Sprite::new(16, 16));
        let o = run(
            &mut e,
            r#"
            local a = sprite:newSlice(Rectangle(1, 2, 3, 4))
            assert(a.name == "Slice 1" and a.bounds == Rectangle(1, 2, 3, 4))
            a.name = "hit"
            local b = sprite:newSlice(Rectangle{ x = 5, y = 6, width = 7, height = 8 })
            assert(b.name == "Slice 2" and b.bounds.w == 7)
            local c = sprite:newSlice(0, 1, 2, 3)                         -- loose numbers
            assert(c.bounds == Rectangle(0, 1, 2, 3))
            local d = sprite:newSlice("named", 9, 9, 2, 2)              -- the Pixmith form
            assert(d.name == "named" and d:getBounds() == Rectangle(9, 9, 2, 2))
            local e = sprite:newSlice("n2", Rectangle(1, 1, 1, 1))
            assert(e.bounds == Rectangle(1, 1, 1, 1))
            local f = sprite:newSlice({ x = 2, y = 2, w = 3, h = 3 })    -- a plain table
            assert(f.bounds == Rectangle(2, 2, 3, 3))
            local whole = sprite:newSlice()
            assert(whole.bounds == sprite.bounds)
            assert(not pcall(sprite.newSlice, sprite, Rectangle(0, 0, -1, 2)))
            assert(not pcall(sprite.newSlice, sprite, "x", 1, 2))

            -- bounds / center / pivot properties
            a.bounds = Rectangle(10, 10, 4, 4)
            assert(a.bounds == Rectangle(10, 10, 4, 4) and a:getBounds(1) == Rectangle(10, 10, 4, 4))
            assert(a.center == nil and a.pivot == nil)
            a.center = Rectangle(1, 1, 2, 2)
            a.pivot = Point(2, 3)
            assert(a.hasCenter and a.hasPivot and a.center == Rectangle(1, 1, 2, 2) and a.pivot == Point(2, 3))
            assert(select(1, a:getPivot()) == 2 and select(2, a:getPivot()) == 3)    -- the Pixmith getter
            a.center = nil
            a.pivot = nil
            assert(not a.hasCenter and not a.hasPivot and a.center == nil)
            -- methods take rectangles and points too, plus a frame
            sprite:newFrame()
            a:setBounds(Rectangle(0, 0, 2, 2), 2)
            assert(a:getBounds(2) == Rectangle(0, 0, 2, 2) and a:getBounds(1) == Rectangle(10, 10, 4, 4))
            a:setCenter(Rectangle(0, 0, 1, 1), sprite.frames[2])
            a:setPivot(Point(1, 1), 2)
            assert(a:getCenter(2) == Rectangle(0, 0, 1, 1))
            a:setBounds(3, 3, 3, 3)                                      -- numbers still work
            assert(a:getBounds() == Rectangle(3, 3, 3, 3))
            assert(not pcall(function() a.bounds = 5 end))
        "#,
            (0, 0),
            Limits::default(),
        );
        assert_eq!(o.error, None);
        assert_eq!(e.sprite.extra.slices.len(), 7);
    }

    #[test]
    fn selection_and_pixel_calls_accept_rectangles_and_points() {
        let mut e = Editor::new(Sprite::new(16, 16));
        let o = run(
            &mut e,
            r#"
            local sel = sprite.selection
            sel:selectRect(Rectangle(1, 1, 4, 4))
            assert(sel.bounds == Rectangle(1, 1, 4, 4) and sel.bounds.w == 4)
            sel:selectRect(Rectangle(0, 0, 2, 2), "add")
            assert(sel.bounds == Rectangle(0, 0, 5, 5))
            sel:selectRect(0, 0, 1, 1, "subtract")                       -- the old form
            assert(not sel:contains(0, 0) and sel:contains(Point(1, 1)) and sel:contains{ x = 1, y = 1 })
            sel:selectRect{ x = 8, y = 8, w = 2, h = 2 }
            assert(sel.bounds == Rectangle(8, 8, 2, 2))
            sel:select(Rectangle(-5, -5, 7, 7))                         -- clipped to the canvas
            assert(sel.bounds == Rectangle(0, 0, 2, 2))
            sel:deselect()
            assert(sel.bounds == nil and sel.origin == Point(0, 0))
            assert(sprite.bounds == Rectangle(0, 0, 16, 16) and sprite.size == Size(16, 16))
            assert(not pcall(sel.selectRect, sel, 1, 2))
            assert(not pcall(sel.select, sel))
            sel:select(Rectangle(0, 0, 2, 2))
            sel.origin = Point(100, 100)                                 -- moved off the canvas
            assert(sel.isEmpty)

            -- dialog bounds
            local d = Dialog("t")
            d.bounds = Rectangle(1, 2, 3, 4)
            assert(d.bounds == Rectangle(1, 2, 3, 4))
            d.bounds = { x = 5, y = 6, width = 7, height = 8 }
            assert(d.bounds.x == 5 and d.bounds.w == 7)
        "#,
            (0, 0),
            Limits::default(),
        );
        assert_eq!(o.error, None);
    }

    #[test]
    fn plain_tables_still_work_wherever_a_point_or_rectangle_is_taken() {
        let mut e = Editor::new(Sprite::new(8, 8));
        let o = run(
            &mut e,
            r#"
            sprite:setPixel({ x = 2, y = 3 }, 0xFF0000FF)
            assert(sprite:getPixel({ x = 2, y = 3 }) == 0xFF0000FF and sprite:getPixel{ 2, 3 } == 0xFF0000FF)
            local px = sprite:getPixels(1, 1, { x = 2, y = 3, w = 1, h = 1 })
            assert(px[1] == 0xFF0000FF)
            sprite:newLayer("t")
            local cel = sprite:newCel(2, 1, Image(1, 1), { x = 1, y = 1 })
            assert(cel.isEmpty)
            local img = Image(2, 2) img:clear(0xFFFFFFFF)
            sprite:newCel(2, 1, img, { x = 1, y = 1 })
            assert(sprite:cel(2, 1).position == Point(1, 1))
            sprite:cel(2, 1).position = { x = 3, y = 3 }
            assert(sprite:cel(2, 1).bounds == Rectangle(3, 3, 2, 2))
            assert(Rectangle(0, 0, 8, 8):contains{ x = 1, y = 1, w = 2, h = 2 })
        "#,
            (0, 0),
            Limits::default(),
        );
        assert_eq!(o.error, None);
    }
}
