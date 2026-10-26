//! Tweening: keyframed layer transforms (position, rotation, scale, opacity) with easing.
//!
//! Design notes (see docs/SPEC-game.md section 3):
//! * Frames are frame indices, not times.
//! * The SOURCE of a tweened layer is its cel at the first key frame (the lowest key frame over all
//!   tracks). Every frame is rendered from that source, never from a previous result, so frames are
//!   independent and nothing degrades by chaining. Cels at other frames are ignored by the live
//!   render while the tracks exist, and only inside `first_key..=last_key`; outside that range the
//!   layer's own cels show, so the live preview matches what "Bake" writes.
//! * Transform order: scale, rotate around the pivot, translate. All rounding goes through
//!   `round_half_up` (`floor(x + 0.5)`).
//! * State lives in the layer's user data text as a versioned line appended after the user's own
//!   text, so it is saved in `.aseprite` files, follows undo snapshots and needs no `Sprite` field.
//! * Layers can have a tween parent (see `rig`): the layer's world transform is the parent's world
//!   transform composed with its own, and frames are rendered from the composed affine matrix.
//! * Opacity is 0..=1 and multiplies the cel opacity. Colors are never blended, so Indexed and
//!   Grayscale sprites stay on their palette; the result is still run through `Snapper`.

use super::rig;
use crate::document::color_mode::Snapper;
use crate::document::{Cel, Editor, Sprite};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The one rounding rule used everywhere (positions, opacity).
pub fn round_half_up(x: f64) -> f64 {
    (x + 0.5).floor()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prop {
    PosX,
    PosY,
    Rotation,
    ScaleX,
    ScaleY,
    Opacity,
}

impl Prop {
    pub const ALL: [Prop; 6] = [Prop::PosX, Prop::PosY, Prop::Rotation, Prop::ScaleX, Prop::ScaleY, Prop::Opacity];

    pub fn from_code(c: u32) -> Option<Prop> {
        Prop::ALL.get(c as usize).copied()
    }

    fn name(self) -> &'static str {
        ["px", "py", "rot", "sx", "sy", "op"][self as usize]
    }

    fn from_name(n: &str) -> Option<Prop> {
        Prop::ALL.into_iter().find(|p| p.name() == n)
    }

    /// Value when the property has no track. Opacity is 0..=1.
    pub fn default_value(self) -> f32 {
        match self {
            Prop::ScaleX | Prop::ScaleY | Prop::Opacity => 1.0,
            _ => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ease {
    Linear,
    Hold,
    InQuad,
    OutQuad,
    InOutQuad,
    InOutCubic,
    OutBack,
    OutBounce,
    Bezier(f32, f32, f32, f32),
}

const EASE_NAMES: [&str; 8] = ["lin", "hold", "iq", "oq", "ioq", "ioc", "ob", "obn"];

impl Ease {
    pub fn from_code(c: u32, b: [f32; 4]) -> Option<Ease> {
        Some(match c {
            0 => Ease::Linear,
            1 => Ease::Hold,
            2 => Ease::InQuad,
            3 => Ease::OutQuad,
            4 => Ease::InOutQuad,
            5 => Ease::InOutCubic,
            6 => Ease::OutBack,
            7 => Ease::OutBounce,
            8 => Ease::Bezier(b[0], b[1], b[2], b[3]),
            _ => return None,
        })
    }

    pub fn code(self) -> u32 {
        match self {
            Ease::Linear => 0,
            Ease::Hold => 1,
            Ease::InQuad => 2,
            Ease::OutQuad => 3,
            Ease::InOutQuad => 4,
            Ease::InOutCubic => 5,
            Ease::OutBack => 6,
            Ease::OutBounce => 7,
            Ease::Bezier(..) => 8,
        }
    }

    /// Maps linear progress `t` in 0..=1 to eased progress.
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Linear => t,
            Ease::Hold => {
                if t >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Ease::InQuad => t * t,
            Ease::OutQuad => 1.0 - (1.0 - t) * (1.0 - t),
            Ease::InOutQuad => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
            Ease::InOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Ease::OutBack => {
                let (c1, c3) = (1.70158, 2.70158);
                1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
            }
            Ease::OutBounce => {
                let (n1, d1) = (7.5625, 2.75);
                if t < 1.0 / d1 {
                    n1 * t * t
                } else if t < 2.0 / d1 {
                    let t = t - 1.5 / d1;
                    n1 * t * t + 0.75
                } else if t < 2.5 / d1 {
                    let t = t - 2.25 / d1;
                    n1 * t * t + 0.9375
                } else {
                    let t = t - 2.625 / d1;
                    n1 * t * t + 0.984375
                }
            }
            Ease::Bezier(x1, y1, x2, y2) => bezier(t, x1 as f64, y1 as f64, x2 as f64, y2 as f64),
        }
    }

    fn encode(self) -> String {
        match self {
            Ease::Bezier(a, b, c, d) => format!("bz/{a}/{b}/{c}/{d}"),
            e => EASE_NAMES[e.code() as usize].to_string(),
        }
    }

    fn decode(s: &str) -> Option<Ease> {
        if let Some(rest) = s.strip_prefix("bz/") {
            let v: Vec<f32> = rest.split('/').filter_map(|x| x.parse().ok()).collect();
            return (v.len() == 4 && v.iter().all(|x| x.is_finite())).then(|| Ease::Bezier(v[0], v[1], v[2], v[3]));
        }
        let i = EASE_NAMES.iter().position(|n| *n == s)?;
        Ease::from_code(i as u32, [0.0; 4])
    }
}

/// CSS-style cubic bezier through (0,0) and (1,1) with control points (x1,y1), (x2,y2): solves
/// x(u) = t with Newton's method, falling back to bisection, and returns y(u).
fn bezier(t: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let (x1, x2) = (x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0));
    let coord = |u: f64, a: f64, b: f64| 3.0 * a * u * (1.0 - u).powi(2) + 3.0 * b * u * u * (1.0 - u) + u * u * u;
    let slope = |u: f64| 3.0 * x1 * (1.0 - u).powi(2) + 6.0 * (x2 - x1) * u * (1.0 - u) + 3.0 * (1.0 - x2) * u * u;
    let mut u = t;
    let mut solved = false;
    for _ in 0..8 {
        let err = coord(u, x1, x2) - t;
        if err.abs() < 1e-9 {
            solved = true;
            break;
        }
        let d = slope(u);
        if d.abs() < 1e-6 {
            break;
        }
        u -= err / d;
    }
    if !solved || !(0.0..=1.0).contains(&u) {
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        for _ in 0..60 {
            u = (lo + hi) / 2.0;
            if coord(u, x1, x2) < t {
                lo = u;
            } else {
                hi = u;
            }
        }
    }
    coord(u, y1, y2)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub frame: u32,
    pub value: f32,
    /// Curve toward the next key.
    pub ease: Ease,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TweenTrack {
    pub layer: usize,
    pub property: Prop,
    /// Sorted by frame, one key per frame.
    pub keys: Vec<Key>,
}

/// Value of a track at `frame`: eased between keys, held before the first and after the last key.
pub fn value_at(track: &TweenTrack, frame: u32) -> f32 {
    let (Some(first), Some(last)) = (track.keys.first(), track.keys.last()) else {
        return track.property.default_value();
    };
    if frame <= first.frame {
        return first.value;
    }
    if frame >= last.frame {
        return last.value;
    }
    let i = track.keys.partition_point(|k| k.frame <= frame) - 1;
    let (a, b) = (track.keys[i], track.keys[i + 1]);
    let t = (frame - a.frame) as f64 / (b.frame - a.frame) as f64;
    (a.value as f64 + (b.value as f64 - a.value as f64) * a.ease.apply(t)) as f32
}

/// What a layer shows in the frames after its last key (the frames before the first key always show the layer's
/// own cels: the source image of a tween is the cel at the first key frame, so there is nothing to hold there).
/// Per layer, not per track, because it decides whether the layer is drawn at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Extend {
    /// The layer's own cels (the behaviour of files from before this option existed).
    Rest,
    /// Keep the pose of the last key until the end of the sprite (the default for new tweens).
    #[default]
    Hold,
    /// Repeat the key range: frame `last + 1` shows the pose of the first key frame, and so on.
    Loop,
}

impl Extend {
    pub fn code(self) -> u32 {
        self as u32
    }
    pub fn from_code(c: u32) -> Option<Extend> {
        Some(match c {
            0 => Extend::Rest,
            1 => Extend::Hold,
            2 => Extend::Loop,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        ["rest", "hold", "loop"][self as usize]
    }
    pub fn from_name(n: &str) -> Option<Extend> {
        (0..3).filter_map(Extend::from_code).find(|e| e.name() == n)
    }
}

/// All tweening state of one layer.
#[derive(Clone, Debug, PartialEq)]
pub struct TweenState {
    pub tracks: Vec<TweenTrack>,
    /// The frames after the last key (see [`Extend`]). A payload without the `ex` member reads as `Rest`.
    pub extend: Extend,
    /// Custom pivot in canvas coordinates (pixel corners are integers; .5 allowed).
    pub pivot: Option<(f32, f32)>,
    pub rotsprite: bool,
    /// Round scale values to integers (keeps pixel sizes even).
    pub snap_scale: bool,
    /// Ignore the opacity track (for Indexed/Grayscale sprites that cannot show partial opacity).
    pub opacity_off: bool,
    /// Rig identity: a number that is unique in the sprite, given to a layer when another layer is
    /// parented to it (`rig::set_parent`). It travels with the layer, so reordering, inserting and
    /// deleting layers never breaks a link.
    pub id: Option<u32>,
    /// The `id` of the tween parent layer. A link whose parent no longer exists is ignored.
    pub parent: Option<u32>,
}

impl TweenState {
    pub(crate) fn new() -> Self {
        TweenState {
            tracks: Vec::new(),
            extend: Extend::Hold,
            pivot: None,
            rotsprite: true,
            snap_scale: false,
            opacity_off: false,
            id: None,
            parent: None,
        }
    }

    pub(crate) fn is_default(&self) -> bool {
        self.tracks.is_empty()
            && self.pivot.is_none()
            && self.rotsprite
            && !self.snap_scale
            && !self.opacity_off
            && self.id.is_none()
            && self.parent.is_none()
    }

    pub fn track(&self, p: Prop) -> Option<&TweenTrack> {
        self.tracks.iter().find(|t| t.property == p)
    }

    /// Lowest and highest key frame over all tracks.
    pub fn range(&self) -> Option<(u32, u32)> {
        let lo = self.tracks.iter().filter_map(|t| t.keys.first()).map(|k| k.frame).min()?;
        let hi = self.tracks.iter().filter_map(|t| t.keys.last()).map(|k| k.frame).max()?;
        Some((lo, hi))
    }

    pub(crate) fn value(&self, p: Prop, frame: u32) -> f32 {
        self.track(p).map_or(p.default_value(), |t| value_at(t, frame))
    }

    /// The last frame the layer is drawn in, given the last key frame `hi` of its key range (its own keys joined
    /// with its ancestors'): `hi` itself for `Rest`, else the end of the sprite.
    pub(crate) fn driven_end(&self, frame_count: usize, hi: u32) -> u32 {
        match self.extend {
            Extend::Rest => hi,
            _ => (frame_count.saturating_sub(1) as u32).max(hi),
        }
    }

    /// The frame whose pose `frame` shows: itself, except past the key range `(lo, hi)` of a `Loop` layer.
    pub(crate) fn pose_frame(&self, (lo, hi): (u32, u32), frame: u32) -> u32 {
        if self.extend == Extend::Loop && frame > hi {
            lo + (frame - lo) % (hi - lo + 1)
        } else {
            frame
        }
    }
}

// MARK: Persistence in layer user data

pub(crate) const SEP: &str = "\u{1e}pixmith.tween.v1:";

/// Splits layer text into the user's own text and the tween payload.
fn split_text(text: &str) -> (&str, Option<&str>) {
    match text.find(SEP) {
        Some(i) => (text[..i].strip_suffix('\n').unwrap_or(&text[..i]), Some(&text[i + SEP.len()..])),
        None => (text, None),
    }
}

fn encode(st: &TweenState) -> String {
    let mut parts = vec![format!("rs={}", st.rotsprite as u8)];
    if st.snap_scale {
        parts.push("ss=1".into());
    }
    if st.opacity_off {
        parts.push("oo=1".into());
    }
    if st.extend != Extend::Rest {
        parts.push(format!("ex={}", st.extend.name()));
    }
    if let Some((x, y)) = st.pivot {
        parts.push(format!("pv={x},{y}"));
    }
    if let Some(id) = st.id {
        parts.push(format!("id={id}"));
    }
    if let Some(pa) = st.parent {
        parts.push(format!("pa={pa}"));
    }
    for t in st.tracks.iter().filter(|t| !t.keys.is_empty()) {
        let keys: Vec<String> = t.keys.iter().map(|k| format!("{},{},{}", k.frame, k.value, k.ease.encode())).collect();
        parts.push(format!("{}={}", t.property.name(), keys.join("~")));
    }
    parts.join(";")
}

fn decode(payload: &str, layer: usize) -> TweenState {
    let mut st = TweenState::new();
    st.extend = Extend::Rest; // files from before the option have no `ex`
    for part in payload.split(';') {
        let Some((k, v)) = part.split_once('=') else { continue };
        match k {
            "rs" => st.rotsprite = v == "1",
            "ss" => st.snap_scale = v == "1",
            "oo" => st.opacity_off = v == "1",
            "ex" => st.extend = Extend::from_name(v).unwrap_or(Extend::Rest),
            "id" => st.id = v.parse().ok().filter(|&n| n > 0),
            "pa" => st.parent = v.parse().ok().filter(|&n| n > 0),
            "pv" => {
                let n: Vec<f32> = v.split(',').filter_map(|x| x.parse().ok()).collect();
                if n.len() == 2 && n.iter().all(|x| x.is_finite()) {
                    st.pivot = Some((n[0], n[1]));
                }
            }
            _ => {
                let Some(property) = Prop::from_name(k) else { continue };
                let mut keys: Vec<Key> = v
                    .split('~')
                    .filter_map(|s| {
                        let mut f = s.splitn(3, ',');
                        let frame = f.next()?.parse().ok()?;
                        let value: f32 = f.next()?.parse().ok()?;
                        let ease = Ease::decode(f.next()?)?;
                        value.is_finite().then_some(Key { frame, value, ease })
                    })
                    .collect();
                keys.sort_by_key(|k| k.frame);
                keys.dedup_by_key(|k| k.frame);
                if !keys.is_empty() {
                    st.tracks.push(TweenTrack { layer, property, keys });
                }
            }
        }
    }
    st
}

/// Reads a layer's tween state, if it has one.
pub fn load(s: &Sprite, layer: usize) -> Option<TweenState> {
    let text = s.layers.get(layer)?.user_data.text.as_deref()?;
    split_text(text).1.map(|p| decode(p, layer))
}

/// Writes (or with `None` removes) a layer's tween state, keeping the user's own text.
pub fn store(s: &mut Sprite, layer: usize, st: Option<&TweenState>) {
    let Some(l) = s.layers.get_mut(layer) else { return };
    let user = l.user_data.text.as_deref().map_or("", |t| split_text(t).0).to_string();
    let text = match st {
        Some(st) => format!("{}{}{}{}", user, if user.is_empty() { "" } else { "\n" }, SEP, encode(st)),
        None => user,
    };
    l.user_data.text = (!text.is_empty()).then_some(text);
}

/// Removes the rig id from a layer's tween payload (a duplicated layer must not claim the id of the
/// layer it was copied from); its own parent link stays.
pub(crate) fn clear_identity(l: &mut crate::document::Layer) {
    let Some(text) = l.user_data.text.as_deref() else { return };
    let (user, Some(payload)) = split_text(text) else { return };
    let mut st = decode(payload, 0);
    if st.id.is_none() {
        return;
    }
    st.id = None;
    let user = user.to_string();
    let text = if st.is_default() {
        user
    } else {
        format!("{}{}{}{}", user, if user.is_empty() { "" } else { "\n" }, SEP, encode(&st))
    };
    l.user_data.text = (!text.is_empty()).then_some(text);
}

/// Stores the state, or removes it when it is back to defaults.
pub(crate) fn store_or_clear(s: &mut Sprite, layer: usize, st: &TweenState) {
    store(s, layer, (!st.is_default()).then_some(st));
}

// MARK: Geometry

#[derive(Clone, Copy, Debug)]
pub(crate) struct Params {
    pub(crate) pivot: (f64, f64),
    pub(crate) tx: f64,
    pub(crate) ty: f64,
    pub(crate) sx: f64,
    pub(crate) sy: f64,
    pub(crate) angle: f64,
}

/// Affine matrix `[a, b, c, d, e, f]`: `x' = a x + c y + e`, `y' = b x + d y + f` (canvas coordinates,
/// y down, positive angles clockwise).
pub type Matrix = [f64; 6];

impl Params {
    /// The forward transform of `warp`: scale about the pivot, rotate about it, then translate.
    pub(crate) fn matrix(&self) -> Matrix {
        let (c, s) = cos_sin(self.angle);
        let (a, b, cc, d) = (c * self.sx, s * self.sx, -s * self.sy, c * self.sy);
        let (px, py) = self.pivot;
        [a, b, cc, d, px + self.tx - (a * px + cc * py), py + self.ty - (b * px + d * py)]
    }

    fn is_identity(&self) -> bool {
        self.tx == 0.0 && self.ty == 0.0 && self.sx == 1.0 && self.sy == 1.0 && quarter_turns(self.angle) == Some(0)
    }

    fn bits(&self) -> [u64; 7] {
        [self.pivot.0, self.pivot.1, self.tx, self.ty, self.sx, self.sy, self.angle].map(f64::to_bits)
    }
}

/// `Some(n mod 4)` when the angle is a multiple of 90 degrees.
fn quarter_turns(angle: f64) -> Option<i32> {
    let q = angle / 90.0;
    ((q - q.round()).abs() < 1e-9).then(|| (q.round() as i64).rem_euclid(4) as i32)
}

/// Cosine and sine of the angle (degrees), exact for multiples of 90.
fn cos_sin(angle: f64) -> (f64, f64) {
    match quarter_turns(angle) {
        Some(0) => (1.0, 0.0),
        Some(1) => (0.0, 1.0),
        Some(2) => (-1.0, 0.0),
        Some(_) => (0.0, -1.0),
        None => {
            let (s, c) = angle.to_radians().sin_cos();
            (c, s)
        }
    }
}

/// Bounds center of the opaque pixels using pixel-corner math (a full 16x16 cel gives (8, 8));
/// the canvas center when the cel is empty.
pub fn content_center(px: &[u8], w: usize, h: usize) -> (f64, f64) {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if px[(y * w + x) * 4 + 3] != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 == usize::MAX {
        return (w as f64 / 2.0, h as f64 / 2.0);
    }
    ((x0 + x1) as f64 / 2.0, (y0 + y1) as f64 / 2.0)
}

fn pack(p: &[u8]) -> u32 {
    if p[3] == 0 {
        0
    } else {
        u32::from_be_bytes([p[0], p[1], p[2], p[3]])
    }
}

/// Largest canvas area the RotSprite resampler handles; bigger ones fall back to nearest.
const ROTSPRITE_MAX_AREA: usize = 1 << 18;

/// Scales, rotates around the pivot and translates a full-canvas RGBA buffer. With `up8` (the
/// source upscaled 8x by `transform::upscale8`) the result is resampled RotSprite-style, otherwise
/// nearest neighbor. Multiples of 90 degrees use exact trigonometry, so they are lossless.
fn warp(src: &[u8], w: usize, h: usize, p: &Params, up8: Option<&[u32]>) -> Vec<u8> {
    if p.sx == 0.0 || p.sy == 0.0 {
        return vec![0u8; w * h * 4];
    }
    let (c, s) = cos_sin(p.angle);
    let inverse = |qx: f64, qy: f64| -> (f64, f64) {
        let (dx, dy) = (qx - p.pivot.0 - p.tx, qy - p.pivot.1 - p.ty);
        ((c * dx + s * dy) / p.sx + p.pivot.0, (-s * dx + c * dy) / p.sy + p.pivot.1)
    };
    warp_with(src, w, h, &inverse, up8)
}

/// `warp` for a general affine matrix (a parented layer's composed transform).
fn warp_affine(src: &[u8], w: usize, h: usize, m: &Matrix, up8: Option<&[u32]>) -> Vec<u8> {
    let Some(inv) = invert(m) else { return vec![0u8; w * h * 4] };
    let inverse =
        |qx: f64, qy: f64| -> (f64, f64) { (inv[0] * qx + inv[2] * qy + inv[4], inv[1] * qx + inv[3] * qy + inv[5]) };
    warp_with(src, w, h, &inverse, up8)
}

/// `a` applied after `b` (`b` first).
pub(crate) fn mul(a: &Matrix, b: &Matrix) -> Matrix {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

pub(crate) const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

pub(crate) fn invert(m: &Matrix) -> Option<Matrix> {
    let det = m[0] * m[3] - m[1] * m[2];
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(a * m[4] + c * m[5]), -(b * m[4] + d * m[5])])
}

/// Whether the matrix only scales and turns by multiples of 90 degrees (nearest neighbor is then exact).
fn axis_aligned(m: &Matrix) -> bool {
    let tiny = |v: f64| v.abs() < 1e-9;
    (tiny(m[1]) && tiny(m[2])) || (tiny(m[0]) && tiny(m[3]))
}

fn warp_with(src: &[u8], w: usize, h: usize, inverse: &dyn Fn(f64, f64) -> (f64, f64), up8: Option<&[u32]>) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            match up8 {
                None => {
                    let (u, v) = inverse(x as f64 + 0.5, y as f64 + 0.5);
                    let (ux, uy) = (u.floor(), v.floor());
                    if ux >= 0.0 && uy >= 0.0 && ux < w as f64 && uy < h as f64 {
                        let i = (uy as usize * w + ux as usize) * 4;
                        out[o..o + 4].copy_from_slice(&src[i..i + 4]);
                    }
                }
                Some(up) => {
                    let (uw, uh) = (w * 8, h * 8);
                    let mut votes = [0u32; 64];
                    for j in 0..8 {
                        for i in 0..8 {
                            let (u, v) = inverse(x as f64 + (i as f64 + 0.5) / 8.0, y as f64 + (j as f64 + 0.5) / 8.0);
                            let (ux, uy) = ((u * 8.0).floor(), (v * 8.0).floor());
                            if ux >= 0.0 && uy >= 0.0 && ux < uw as f64 && uy < uh as f64 {
                                votes[j * 8 + i] = up[uy as usize * uw + ux as usize];
                            }
                        }
                    }
                    votes.sort_unstable();
                    let (mut best, mut best_n, mut run) = (0u32, 0, 0);
                    for k in 0..64 {
                        run = if k > 0 && votes[k] == votes[k - 1] { run + 1 } else { 1 };
                        if run > best_n {
                            best_n = run;
                            best = votes[k];
                        }
                    }
                    out[o..o + 4].copy_from_slice(&best.to_be_bytes());
                }
            }
        }
    }
    out
}

// MARK: Cache

#[derive(Hash, PartialEq, Eq)]
struct CacheKey {
    cel: usize,
    w: usize,
    h: usize,
    params: [u64; 7],
    affine: bool,
    rotsprite: bool,
}

struct Cache {
    /// The `Arc<Cel>` (when the source is a cel) is held so the pointer in the key cannot be reused
    /// while the entry lives; it also makes later edits of that cel copy-on-write, which changes the
    /// pointer, so an edited source can never hit a stale entry.
    results: HashMap<CacheKey, (Option<Arc<Cel>>, Arc<Vec<u8>>)>,
    up8: HashMap<usize, (Option<Arc<Cel>>, Arc<Vec<u32>>)>,
}

/// Identity of a source image for the cache: a cel pointer, or a content hash (groups).
#[derive(Clone)]
enum Source<'a> {
    Cel(&'a Arc<Cel>),
    Buffer(&'a [u8]),
}

impl Source<'_> {
    fn id(&self) -> usize {
        match self {
            Source::Cel(c) => Arc::as_ptr(c) as usize,
            Source::Buffer(b) => {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                b.hash(&mut h);
                (h.finish() as usize) | 1 // cel pointers are aligned, so the low bit tells them apart
            }
        }
    }

    fn pixels(&self) -> &[u8] {
        match self {
            Source::Cel(c) => &c.pixels,
            Source::Buffer(b) => b,
        }
    }

    fn keep(&self) -> Option<Arc<Cel>> {
        match self {
            Source::Cel(c) => Some((*c).clone()),
            Source::Buffer(_) => None,
        }
    }
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
const CACHE_MAX: usize = 64;

/// Number of warps computed (not served from the cache); lets tests observe caching.
pub static WARPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The transform of a layer: its own parameters, or (below a tween parent) a composed matrix.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Xf {
    Own(Params),
    World(Matrix),
}

fn transformed(src: Source, w: usize, h: usize, xf: &Xf, rotsprite: bool) -> Arc<Vec<u8>> {
    let (identity, quarter, bits) = match xf {
        Xf::Own(p) => (p.is_identity(), quarter_turns(p.angle).is_some(), p.bits()),
        Xf::World(m) => (*m == IDENTITY, axis_aligned(m), [m[0], m[1], m[2], m[3], m[4], m[5], 0.0].map(f64::to_bits)),
    };
    if identity {
        return Arc::new(src.pixels().to_vec());
    }
    let use_rs = rotsprite && !quarter && w * h <= ROTSPRITE_MAX_AREA;
    let id = src.id();
    let key = CacheKey { cel: id, w, h, params: bits, affine: matches!(xf, Xf::World(_)), rotsprite: use_rs };
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(|| Cache { results: HashMap::new(), up8: HashMap::new() });
    if let Some((_, r)) = cache.results.get(&key) {
        return r.clone();
    }
    let up = use_rs.then(|| {
        if cache.up8.len() >= 8 && !cache.up8.contains_key(&id) {
            cache.up8.clear();
        }
        cache
            .up8
            .entry(id)
            .or_insert_with(|| {
                let packed: Vec<u32> = src.pixels().chunks_exact(4).map(pack).collect();
                (src.keep(), Arc::new(crate::paint::transform::upscale8(&packed, w, h)))
            })
            .1
            .clone()
    });
    WARPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let up = up.as_deref().map(|v| v.as_slice());
    let result = Arc::new(match xf {
        Xf::Own(p) => warp(src.pixels(), w, h, p, up),
        Xf::World(m) => warp_affine(src.pixels(), w, h, m, up),
    });
    if cache.results.len() >= CACHE_MAX {
        cache.results.clear();
    }
    cache.results.insert(key, (src.keep(), result.clone()));
    result
}

// MARK: Rendering

/// Transform parameters at `frame` with the pivot resolved, plus opacity (0..=1).
fn params_at(st: &TweenState, frame: u32, src: &[u8], w: usize, h: usize) -> (Params, f64) {
    let scale = |v: f64| if st.snap_scale { round_half_up(v) } else { v };
    let pivot = st.pivot.map_or_else(|| content_center(src, w, h), |(x, y)| (x as f64, y as f64));
    let p = Params {
        pivot,
        tx: round_half_up(st.value(Prop::PosX, frame) as f64),
        ty: round_half_up(st.value(Prop::PosY, frame) as f64),
        sx: scale(st.value(Prop::ScaleX, frame) as f64),
        sy: scale(st.value(Prop::ScaleY, frame) as f64),
        angle: st.value(Prop::Rotation, frame) as f64,
    };
    let op = if st.opacity_off { 1.0 } else { (st.value(Prop::Opacity, frame) as f64).clamp(0.0, 1.0) };
    (p, op)
}

/// The local transform of a layer at a frame: identity when it has no tracks (a plain rig parent),
/// else scale, rotation about its own pivot and translation. Used for the ancestors of a layer.
pub(crate) fn local_matrix(s: &Sprite, layer: usize, st: &TweenState, frame: u32) -> Matrix {
    let Some((lo, _)) = st.range() else { return IDENTITY };
    let (w, h) = (s.width as usize, s.height as usize);
    let px = if st.pivot.is_some() { Vec::new() } else { source_pixels(s, layer, lo as usize).unwrap_or_default() };
    let px = if px.len() == w * h * 4 || st.pivot.is_some() { px } else { vec![0; w * h * 4] };
    params_at(st, frame, &px, w, h).0.matrix()
}

/// Renders the tweened cel of `layer` at `frame` from the source (the cel at the first key frame, or
/// for a group its composite at that frame), or `None` outside the key range. For a group the
/// result is the transformed composite and its tween opacity as cel opacity. Below a tween parent the
/// transform is the parent's world transform composed with the layer's own, and the key range is the
/// union of the ranges of the layer and its ancestors.
fn render_state(s: &Sprite, layer: usize, frame: usize, st: &TweenState) -> Option<Arc<Cel>> {
    let anc = rig::ancestors(s, layer, st);
    let (lo, hi) = rig::hull(st, &anc)?;
    if (frame as u32) < lo || frame as u32 > st.driven_end(s.frame_count(), hi) {
        return None;
    }
    let pose = st.pose_frame((lo, hi), frame as u32);
    let src_lo = st.range().map_or(lo, |r| r.0);
    let parent_world = rig::ancestors_world(s, &anc, pose);
    let xf = |p: Params| if parent_world == IDENTITY { Xf::Own(p) } else { Xf::World(mul(&parent_world, &p.matrix())) };
    let (w, h) = (s.width as usize, s.height as usize);
    let is_group = s.layers.get(layer)?.is_group();
    let (pixels, base_opacity) = if is_group {
        let mut buf = vec![0u8; w * h * 4];
        crate::document::layers::composite_range(s, src_lo as usize, layer + 1, s.subtree_end(layer), &mut buf, 0);
        let (p, op) = params_at(st, pose, &buf, w, h);
        (transformed(Source::Buffer(&buf), w, h, &xf(p), st.rotsprite).as_ref().clone(), (255u8, op))
    } else {
        let src = s.cels.get(src_lo as usize)?.get(layer)?.clone();
        let Some(src) = src else {
            return Some(Arc::new(Cel { opacity: 255, pixels: vec![0; w * h * 4], ..Default::default() }));
        };
        let (p, op) = params_at(st, pose, &src.pixels, w, h);
        (transformed(Source::Cel(&src), w, h, &xf(p), st.rotsprite).as_ref().clone(), (src.opacity, op))
    };
    let mut pixels = pixels;
    let mut sn = Snapper::new(s);
    if sn.active() {
        sn.snap_rgba(&mut pixels, false);
    }
    let opacity = round_half_up(base_opacity.0 as f64 * base_opacity.1).clamp(0.0, 255.0) as u8;
    Some(Arc::new(Cel { opacity, pixels, ..Default::default() }))
}

/// Whether any layer has tween tracks (an edit to one cel then changes what other frames show).
pub(crate) fn any_tween(s: &Sprite) -> bool {
    s.layers.iter().any(|l| l.user_data.text.as_deref().is_some_and(|t| t.contains(SEP)))
}

/// Hook for `layers::composite_range`: the tweened cel of a layer at a frame, or `None` when the
/// layer has no tracks (the common case, which costs one `Option` check per layer).
pub(crate) fn tweened(s: &Sprite, frame: usize, layer: usize) -> Option<Arc<Cel>> {
    let l = s.layers.get(layer)?;
    if l.kind > 1 || !l.user_data.text.as_deref().is_some_and(|t| t.contains(SEP)) {
        return None;
    }
    render_state(s, layer, frame, &load(s, layer)?)
}

/// Bit 0: a scale value between the first and last key is not an integer (uneven pixel sizes);
/// not reported while "snap scale" is on. Bit 1: an opacity track in an Indexed/Grayscale sprite
/// (opacity is a cel property there, colors stay on the palette); not reported when opacity is off.
pub fn warnings(s: &Sprite, layer: usize) -> u32 {
    let Some(st) = load(s, layer) else { return 0 };
    let Some((lo, hi)) = st.range() else { return 0 };
    let uneven = !st.snap_scale
        && (lo..=hi).any(|f| {
            [Prop::ScaleX, Prop::ScaleY].iter().any(|&p| {
                let v = st.value(p, f) as f64;
                (v - v.round()).abs() > 1e-3
            })
        });
    let opacity = !st.opacity_off
        && s.color_mode != crate::document::color_mode::ColorMode::Rgba
        && st.track(Prop::Opacity).is_some();
    uneven as u32 | (opacity as u32) << 1
}

/// Human-readable text for a warnings bit mask, one line per warning.
pub fn warning_text(bits: u32) -> String {
    let mut out = Vec::new();
    if bits & 1 != 0 {
        out.push("Non-integer scale gives uneven pixel sizes. Turn on 'Snap scale to integer' to avoid it.");
    }
    if bits & 2 != 0 {
        out.push("Indexed/Grayscale mode: opacity fades the whole cel (pixels stay on the palette, no new colors). Turn on 'Ignore opacity' to disable it.");
    }
    out.join("\n")
}

// MARK: Editing

/// Normal and group layers can be tweened (tilemap layers cannot).
pub(crate) fn editable(s: &Sprite, layer: usize) -> bool {
    s.layers.get(layer).is_some_and(|l| l.kind <= 1)
}

/// Adds or replaces the key of `prop` at `frame`. One undo step.
pub fn set_key(e: &mut Editor, layer: usize, prop: Prop, frame: u32, value: f32, ease: Ease) -> bool {
    if !key_ok(&e.sprite, layer, frame, value) {
        return false;
    }
    e.checkpoint();
    set_key_raw(&mut e.sprite, layer, prop, frame, value, ease)
}

fn key_ok(s: &Sprite, layer: usize, frame: u32, value: f32) -> bool {
    editable(s, layer) && (frame as usize) < s.frame_count() && value.is_finite()
}

/// `set_key` on a bare sprite, without an undo step (scripts own their undo step).
pub(crate) fn set_key_raw(s: &mut Sprite, layer: usize, prop: Prop, frame: u32, value: f32, ease: Ease) -> bool {
    if !key_ok(s, layer, frame, value) {
        return false;
    }
    let mut st = load(s, layer).unwrap_or_else(TweenState::new);
    let i = match st.tracks.iter().position(|t| t.property == prop) {
        Some(i) => i,
        None => {
            st.tracks.push(TweenTrack { layer, property: prop, keys: Vec::new() });
            st.tracks.len() - 1
        }
    };
    let keys = &mut st.tracks[i].keys;
    match keys.binary_search_by_key(&frame, |k| k.frame) {
        Ok(j) => keys[j] = Key { frame, value, ease },
        Err(j) => keys.insert(j, Key { frame, value, ease }),
    }
    store_or_clear(s, layer, &st);
    true
}

/// Removes the key of `prop` at `frame`. One undo step.
pub fn remove_key(e: &mut Editor, layer: usize, prop: Prop, frame: u32) -> bool {
    let Some(mut st) = load(&e.sprite, layer) else { return false };
    let Some(t) = st.tracks.iter().find(|t| t.property == prop) else { return false };
    if !t.keys.iter().any(|k| k.frame == frame) {
        return false;
    }
    e.checkpoint();
    if let Some(t) = st.tracks.iter_mut().find(|t| t.property == prop) {
        t.keys.retain(|k| k.frame != frame);
    }
    st.tracks.retain(|t| !t.keys.is_empty());
    store_or_clear(&mut e.sprite, layer, &st);
    true
}

/// Sets (`Some`) or resets (`None`, content center) the layer's pivot. One undo step.
pub fn set_pivot(e: &mut Editor, layer: usize, pivot: Option<(f32, f32)>) -> bool {
    if !pivot_ok(&e.sprite, layer, pivot) {
        return false;
    }
    e.checkpoint();
    set_pivot_raw(&mut e.sprite, layer, pivot)
}

fn pivot_ok(s: &Sprite, layer: usize, pivot: Option<(f32, f32)>) -> bool {
    editable(s, layer) && !pivot.is_some_and(|(x, y)| !x.is_finite() || !y.is_finite())
}

/// `set_pivot` on a bare sprite, without an undo step.
pub(crate) fn set_pivot_raw(s: &mut Sprite, layer: usize, pivot: Option<(f32, f32)>) -> bool {
    if !pivot_ok(s, layer, pivot) {
        return false;
    }
    let mut st = load(s, layer).unwrap_or_else(TweenState::new);
    st.pivot = pivot;
    store_or_clear(s, layer, &st);
    true
}

/// Sets what the layer shows after its last key. One undo step.
pub fn set_extend(e: &mut Editor, layer: usize, extend: Extend) -> bool {
    if !editable(&e.sprite, layer) {
        return false;
    }
    e.checkpoint();
    set_extend_raw(&mut e.sprite, layer, extend)
}

/// `set_extend` on a bare sprite, without an undo step (scripts own their undo step).
pub(crate) fn set_extend_raw(s: &mut Sprite, layer: usize, extend: Extend) -> bool {
    if !editable(s, layer) {
        return false;
    }
    let mut st = load(s, layer).unwrap_or_else(TweenState::new);
    st.extend = extend;
    store_or_clear(s, layer, &st);
    true
}

/// What a layer shows after its last key (`Hold` for a layer without tween state).
pub fn extend_of(s: &Sprite, layer: usize) -> Extend {
    load(s, layer).map_or(Extend::Hold, |st| st.extend)
}

/// Chooses RotSprite (true) or nearest neighbor (false) for non-90-degree rotation.
pub fn set_rotsprite(e: &mut Editor, layer: usize, on: bool) -> bool {
    set_flag(e, layer, 0, on)
}

/// Source pixels of a layer at its tween source frame (cel, or the group's composite).
fn source_pixels(s: &Sprite, layer: usize, frame: usize) -> Option<Vec<u8>> {
    let (w, h) = (s.width as usize, s.height as usize);
    if s.layers.get(layer)?.is_group() {
        let mut buf = vec![0u8; w * h * 4];
        crate::document::layers::composite_range(s, frame, layer + 1, s.subtree_end(layer), &mut buf, 0);
        return Some(buf);
    }
    s.cels.get(frame)?.get(layer)?.as_ref().map(|c| c.pixels.clone())
}

/// The frame the layer's tween renders from: its first own key, else (a keyless child) the start of
/// the range it inherits, else frame 0.
fn source_frame(s: &Sprite, layer: usize, st: Option<&TweenState>) -> u32 {
    let Some(st) = st else { return 0 };
    st.range().map(|r| r.0).or_else(|| rig::hull(st, &rig::ancestors(s, layer, st)).map(|r| r.0)).unwrap_or(0)
}

/// The pivot in effect: custom, else the content center of the source.
pub fn effective_pivot(s: &Sprite, layer: usize) -> Option<(f64, f64)> {
    let st = load(s, layer);
    if let Some((x, y)) = st.as_ref().and_then(|st| st.pivot) {
        return Some((x as f64, y as f64));
    }
    let frame = source_frame(s, layer, st.as_ref()) as usize;
    let px = source_pixels(s, layer, frame)?;
    Some(content_center(&px, s.width as usize, s.height as usize))
}

/// Bounds (x0, y0, x1, y1; pixel corners) of the opaque pixels of the layer's tween source, or the
/// whole canvas when it is empty. Canvas handles are drawn around this box.
pub fn source_bounds(s: &Sprite, layer: usize) -> Option<(f64, f64, f64, f64)> {
    let st = load(s, layer);
    let frame = source_frame(s, layer, st.as_ref()) as usize;
    let px = source_pixels(s, layer, frame)?;
    let (w, h) = (s.width as usize, s.height as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if px[(y * w + x) * 4 + 3] != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 == usize::MAX {
        return Some((0.0, 0.0, w as f64, h as f64));
    }
    Some((x0 as f64, y0 as f64, x1 as f64, y1 as f64))
}

/// Which frames a bake covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BakeRange {
    /// First key to the last frame the layer is drawn in (the last key for `Rest`, else the end of the sprite).
    All,
    /// An inclusive frame range (must lie inside the range `All` covers).
    Frames(u32, u32),
    Current(u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BakeOptions {
    pub range: BakeRange,
    /// Write into a new layer above instead of the layer itself (always the case for groups).
    pub new_layer: bool,
    /// Keep the tracks and the source (the source cel is never overwritten), so the layer can be
    /// baked again. Without it a full-range bake removes the tracks.
    pub keep_source: bool,
}

impl BakeOptions {
    pub fn all(keep_source: bool) -> Self {
        BakeOptions { range: BakeRange::All, new_layer: false, keep_source }
    }
}

/// Transform parameters at a frame: x, y (rounded), rotation, scale x, scale y, opacity, pivot x, y.
pub fn transform_at(s: &Sprite, layer: usize, frame: u32) -> Option<[f32; 8]> {
    let st = load(s, layer)?;
    let hull = rig::hull(&st, &rig::ancestors(s, layer, &st))?;
    let lo = st.range().map_or(hull.0, |r| r.0);
    let (w, h) = (s.width as usize, s.height as usize);
    let px = source_pixels(s, layer, lo as usize).unwrap_or_else(|| vec![0; w * h * 4]);
    let (p, op) = params_at(&st, st.pose_frame(hull, frame), &px, w, h);
    Some([
        p.tx as f32,
        p.ty as f32,
        p.angle as f32,
        p.sx as f32,
        p.sy as f32,
        op as f32,
        p.pivot.0 as f32,
        p.pivot.1 as f32,
    ])
}

/// Everything a bake needs, rendered from the untouched document.
struct BakePlan {
    rendered: Vec<(usize, Arc<Cel>)>,
    new_layer: bool,
    /// Covers the whole key range, so the tracks (and the rig link) go away.
    full: bool,
}

/// Renders the frames a bake of `layer` writes. The sprite must have its links flushed.
fn plan_bake(s: &Sprite, layer: usize, opts: &BakeOptions) -> Option<BakePlan> {
    let st = load(s, layer)?;
    let anc = rig::ancestors(s, layer, &st);
    let (lo, hi) = rig::hull(&st, &anc)?;
    if !editable(s, layer) || hi as usize >= s.frame_count() {
        return None;
    }
    let src_lo = st.range().map_or(lo, |r| r.0);
    // The frames after the last key are part of the animation too (held or looped), so a bake that removes the
    // tracks writes them; otherwise they would jump back to the layer's own cels.
    let end = st.driven_end(s.frame_count(), hi);
    let (from, to) = match opts.range {
        BakeRange::All => (lo, end),
        BakeRange::Frames(a, b) => (a.min(b), a.max(b)),
        BakeRange::Current(f) => (f, f),
    };
    if from < lo || to > end {
        return None;
    }
    let new_layer = opts.new_layer || s.layers[layer].is_group();
    let rendered = (from..=to)
        .filter(|&f| new_layer || !(opts.keep_source && f == src_lo))
        .filter_map(|f| render_state(s, layer, f as usize, &st).map(|c| (f as usize, c)))
        .collect();
    Some(BakePlan { rendered, new_layer, full: from == lo && to == end })
}

/// Writes a plan into the document (the caller made the undo checkpoint). Returns the layer the cels
/// went to and, when a layer was inserted, its index.
fn apply_bake(e: &mut Editor, layer: usize, opts: &BakeOptions, plan: &BakePlan) -> (usize, Option<usize>) {
    let is_group = e.sprite.layers[layer].is_group();
    let (target, inserted) = if plan.new_layer {
        let above = if is_group { e.sprite.subtree_end(layer) - 1 } else { layer };
        let t = e.sprite.add_layer(above);
        e.sprite.layers[t].name = format!("{} (baked)", e.sprite.layers[layer].name);
        e.sprite.layers[t].child_level = e.sprite.layers[layer].child_level;
        (t, Some(t))
    } else {
        (layer, None)
    };
    // `add_layer` inserts above `layer`, so `layer` keeps its index.
    for (f, cel) in &plan.rendered {
        e.sprite.cels[*f][target] = Some(cel.clone());
    }
    if plan.new_layer {
        e.sprite.layers[layer].visible = false;
    }
    if !opts.keep_source && plan.full {
        store(&mut e.sprite, layer, None);
        rig::forget_unused_ids(&mut e.sprite);
    }
    (target, inserted)
}

/// Writes the tweened result into real cels as one undo step, per `opts`. Every frame is rendered
/// from the source before any cel is touched. Returns the layer the cels went to.
///
/// In place, the source cel at the first key frame is overwritten only when the tracks are removed
/// (a plain bake); with `keep_source` it is left alone. A new layer leaves the original untouched
/// and hides it. Groups always bake into a new layer.
///
/// A layer with a tween parent is baked with its composed transform; baking it removes its rig link
/// (the motion of the parent is part of its cels now). A layer that has tween children cannot lose its
/// tracks this way (the children would stop moving): use `bake_all`, or `keep_source`.
pub fn bake_with(e: &mut Editor, layer: usize, opts: BakeOptions) -> Option<usize> {
    e.sprite.flush_links();
    let plan = plan_bake(&e.sprite, layer, &opts)?;
    if !opts.keep_source && plan.full && !rig::children_of(&e.sprite, layer).is_empty() {
        return None;
    }
    e.checkpoint();
    Some(apply_bake(e, layer, &opts, &plan).0)
}

/// Plain bake: in place, first to last key; `keep_source` keeps tracks and source.
pub fn bake(e: &mut Editor, layer: usize, keep_source: bool) -> bool {
    bake_with(e, layer, BakeOptions::all(keep_source)).is_some()
}

/// Bakes every tweened layer (one undo step). All frames of all layers are rendered from the
/// document as it is first, so the parents' motion reaches their children whatever the order;
/// the results are then written parents before children. Returns how many layers were baked.
pub fn bake_all(e: &mut Editor, keep_source: bool, new_layer: bool) -> usize {
    let opts = BakeOptions { range: BakeRange::All, new_layer, keep_source };
    e.sprite.flush_links();
    let mut plans: Vec<(usize, usize, BakePlan)> = (0..e.sprite.layers.len())
        .filter_map(|l| {
            let depth = load(&e.sprite, l).map(|st| rig::ancestors(&e.sprite, l, &st).len())?;
            plan_bake(&e.sprite, l, &opts).map(|p| (depth, l, p))
        })
        .collect();
    if plans.is_empty() {
        return 0;
    }
    // Parents first; among equals the higher layer first (its insertion shifts fewer pending layers).
    plans.sort_by_key(|(depth, l, _)| (*depth, std::cmp::Reverse(*l)));
    let mut index: Vec<usize> = plans.iter().map(|p| p.1).collect();
    e.begin_transaction();
    e.checkpoint();
    for k in 0..plans.len() {
        if let (_, Some(at)) = apply_bake(e, index[k], &opts, &plans[k].2) {
            for j in index.iter_mut().skip(k + 1).filter(|j| **j >= at) {
                *j += 1;
            }
        }
    }
    e.end_transaction();
    plans.len()
}

/// A copy of the sprite with every tween baked in place (tracks removed), for exporters that need
/// real cels. The document itself is not touched, so exporting stays non-destructive. (All the
/// built-in exporters go through `Sprite::render`, which already shows live tweens.)
pub fn baked_copy(s: &Sprite) -> Sprite {
    let mut e = Editor::new(s.clone());
    bake_all(&mut e, false, false);
    e.sprite
}

// MARK: Key editing helpers

/// Moves a key to another frame (replacing a key already there). One undo step.
pub fn move_key(e: &mut Editor, layer: usize, prop: Prop, from: u32, to: u32) -> bool {
    let Some(mut st) = load(&e.sprite, layer) else { return false };
    if to as usize >= e.sprite.frame_count() {
        return false;
    }
    let Some(t) = st.tracks.iter_mut().find(|t| t.property == prop) else { return false };
    let Some(j) = t.keys.iter().position(|k| k.frame == from) else { return false };
    if from == to {
        return true;
    }
    e.checkpoint();
    let mut k = t.keys.remove(j);
    k.frame = to;
    t.keys.retain(|x| x.frame != to);
    let at = t.keys.partition_point(|x| x.frame < to);
    t.keys.insert(at, k);
    store_or_clear(&mut e.sprite, layer, &st);
    true
}

/// Changes the curve of an existing key (value unchanged). One undo step.
pub fn set_ease(e: &mut Editor, layer: usize, prop: Prop, frame: u32, ease: Ease) -> bool {
    let Some(v) = load(&e.sprite, layer)
        .and_then(|st| st.track(prop).and_then(|t| t.keys.iter().find(|k| k.frame == frame).map(|k| k.value)))
    else {
        return false;
    };
    set_key(e, layer, prop, frame, v, ease)
}

/// A key copied relative to the first copied frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipKey {
    pub prop: Prop,
    pub offset: u32,
    pub value: f32,
    pub ease: Ease,
}

static CLIPBOARD: Mutex<Vec<ClipKey>> = Mutex::new(Vec::new());

/// Copies every key of the layer in frames `from..=to` (all properties) to the clipboard.
pub fn copy_keys(s: &Sprite, layer: usize, from: u32, to: u32) -> usize {
    let mut clip = Vec::new();
    if let Some(st) = load(s, layer) {
        for t in &st.tracks {
            for k in t.keys.iter().filter(|k| k.frame >= from && k.frame <= to) {
                clip.push(ClipKey { prop: t.property, offset: k.frame - from, value: k.value, ease: k.ease });
            }
        }
    }
    let n = clip.len();
    *CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner()) = clip;
    n
}

/// Pastes the clipboard so its first frame lands on `at` (keys past the last frame are dropped).
/// One undo step. Returns the number of keys pasted.
pub fn paste_keys(e: &mut Editor, layer: usize, at: u32) -> usize {
    let clip = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let n_frames = e.sprite.frame_count() as u32;
    let clip: Vec<ClipKey> = clip.into_iter().filter(|k| at + k.offset < n_frames).collect();
    if clip.is_empty() || !editable(&e.sprite, layer) {
        return 0;
    }
    e.begin_transaction();
    for k in &clip {
        set_key(e, layer, k.prop, at + k.offset, k.value, k.ease);
    }
    e.end_transaction();
    clip.len()
}

/// Sets keys for the properties in `mask` (bit per `Prop`) at one frame, one undo step (canvas
/// dragging, free-transform handles, auto-key).
pub fn set_keys_at(e: &mut Editor, layer: usize, frame: u32, mask: u32, values: &[f32; 6], ease: Ease) -> bool {
    if !editable(&e.sprite, layer) || frame as usize >= e.sprite.frame_count() {
        return false;
    }
    e.begin_transaction();
    let mut any = false;
    for p in Prop::ALL {
        if mask & (1 << p as u32) != 0 {
            any |= set_key(e, layer, p, frame, values[p as usize], ease);
        }
    }
    e.end_transaction();
    any
}

pub fn set_flag(e: &mut Editor, layer: usize, flag: u32, on: bool) -> bool {
    if !editable(&e.sprite, layer) {
        return false;
    }
    e.checkpoint();
    let mut st = load(&e.sprite, layer).unwrap_or_else(TweenState::new);
    match flag {
        0 => st.rotsprite = on,
        1 => st.snap_scale = on,
        2 => st.opacity_off = on,
        _ => return false,
    }
    store_or_clear(&mut e.sprite, layer, &st);
    true
}

// MARK: C ABI

unsafe fn ed<'a>(doc: *mut Editor) -> Option<&'a mut Editor> {
    doc.as_mut()
}

/// `ease`: 0 linear, 1 hold, 2 in-quad, 3 out-quad, 4 in-out-quad, 5 in-out-cubic, 6 out-back,
/// 7 out-bounce, 8 bezier (x1, y1, x2, y2). `prop`: 0 x, 1 y, 2 rotation (degrees), 3 scale x,
/// 4 scale y, 5 opacity (0..1).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_tween_set_key(
    doc: *mut Editor,
    layer: u32,
    prop: u32,
    frame: u32,
    value: f32,
    ease: u32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
) -> bool {
    crate::ffi::guarded("pixmith_tween_set_key", doc, || {
        let (Some(e), Some(p), Some(ease)) = (ed(doc), Prop::from_code(prop), Ease::from_code(ease, [x1, y1, x2, y2]))
        else {
            return false;
        };
        if let Ease::Bezier(a, b, c, d) = ease {
            if ![a, b, c, d].iter().all(|v| v.is_finite()) {
                return false;
            }
        }
        set_key(e, layer as usize, p, frame, value, ease)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_remove_key(doc: *mut Editor, layer: u32, prop: u32, frame: u32) -> bool {
    crate::ffi::guarded("pixmith_tween_remove_key", doc, || {
        let (Some(e), Some(p)) = (ed(doc), Prop::from_code(prop)) else { return false };
        remove_key(e, layer as usize, p, frame)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_key_count(doc: *const Editor, layer: u32, prop: u32) -> u32 {
    crate::ffi::guarded("pixmith_tween_key_count", std::ptr::null_mut(), || {
        let (Some(e), Some(p)) = (doc.as_ref(), Prop::from_code(prop)) else { return 0 };
        load(&e.sprite, layer as usize).and_then(|st| st.track(p).map(|t| t.keys.len() as u32)).unwrap_or(0)
    })
}

/// Writes 7 floats: frame, value, ease code, x1, y1, x2, y2.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_key_at(
    doc: *const Editor,
    layer: u32,
    prop: u32,
    index: u32,
    out: *mut f32,
) -> bool {
    crate::ffi::guarded("pixmith_tween_key_at", std::ptr::null_mut(), || {
        let (Some(e), Some(p), false) = (doc.as_ref(), Prop::from_code(prop), out.is_null()) else { return false };
        let Some(k) = load(&e.sprite, layer as usize)
            .and_then(|st| st.track(p).and_then(|t| t.keys.get(index as usize).copied()))
        else {
            return false;
        };
        let b = if let Ease::Bezier(a, b, c, d) = k.ease { [a, b, c, d] } else { [0.0; 4] };
        std::slice::from_raw_parts_mut(out, 7).copy_from_slice(&[
            k.frame as f32,
            k.value,
            k.ease.code() as f32,
            b[0],
            b[1],
            b[2],
            b[3],
        ]);
        true
    })
}

/// Value of a property at a frame (the default when it has no track).
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_value_at(doc: *const Editor, layer: u32, prop: u32, frame: u32) -> f32 {
    crate::ffi::guarded("pixmith_tween_value_at", std::ptr::null_mut(), || {
        let (Some(e), Some(p)) = (doc.as_ref(), Prop::from_code(prop)) else { return 0.0 };
        load(&e.sprite, layer as usize).map_or(p.default_value(), |st| st.value(p, frame))
    })
}

/// `custom` false resets the pivot to the content center.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_set_pivot(doc: *mut Editor, layer: u32, x: f32, y: f32, custom: bool) -> bool {
    crate::ffi::guarded("pixmith_tween_set_pivot", doc, || {
        ed(doc).is_some_and(|e| set_pivot(e, layer as usize, custom.then_some((x, y))))
    })
}

/// Writes the effective pivot (x, y).
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_get_pivot(doc: *const Editor, layer: u32, out: *mut f32) -> bool {
    crate::ffi::guarded("pixmith_tween_get_pivot", std::ptr::null_mut(), || {
        let (Some(e), false) = (doc.as_ref(), out.is_null()) else { return false };
        let Some(p) = effective_pivot(&e.sprite, layer as usize) else { return false };
        std::slice::from_raw_parts_mut(out, 2).copy_from_slice(&[p.0 as f32, p.1 as f32]);
        true
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_set_rotsprite(doc: *mut Editor, layer: u32, on: bool) -> bool {
    crate::ffi::guarded("pixmith_tween_set_rotsprite", doc, || {
        ed(doc).is_some_and(|e| set_rotsprite(e, layer as usize, on))
    })
}

/// Bit 0: layer has keys. Bit 1: RotSprite on. Bit 2: custom pivot. Bit 3: snap scale. Bit 4: opacity ignored.
/// Bit 5: the layer has a tween parent link (it may point at a deleted layer; see `pixmith_tween_get_parent`).
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_options(doc: *const Editor, layer: u32) -> u32 {
    crate::ffi::guarded("pixmith_tween_options", std::ptr::null_mut(), || {
        let Some(e) = doc.as_ref() else { return 0 };
        match load(&e.sprite, layer as usize) {
            Some(st) => {
                (!st.tracks.is_empty()) as u32
                    | (st.rotsprite as u32) << 1
                    | (st.pivot.is_some() as u32) << 2
                    | (st.snap_scale as u32) << 3
                    | (st.opacity_off as u32) << 4
                    | (st.parent.is_some() as u32) << 5
            }
            None => 2,
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_bake(doc: *mut Editor, layer: u32, keep_source: bool) -> bool {
    crate::ffi::guarded("pixmith_tween_bake", doc, || ed(doc).is_some_and(|e| bake(e, layer as usize, keep_source)))
}

/// Bit 0: non-integer scale (uneven pixel sizes).
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_warnings(doc: *const Editor, layer: u32) -> u32 {
    crate::ffi::guarded("pixmith_tween_warnings", std::ptr::null_mut(), || {
        doc.as_ref().map_or(0, |e| warnings(&e.sprite, layer as usize))
    })
}

/// Flag 0 RotSprite, 1 snap scale to integer, 2 ignore opacity.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_set_flag(doc: *mut Editor, layer: u32, flag: u32, on: bool) -> bool {
    crate::ffi::guarded("pixmith_tween_set_flag", doc, || {
        ed(doc).is_some_and(|e| set_flag(e, layer as usize, flag, on))
    })
}

/// What the layer shows after its last key: 0 rest (its own cels), 1 hold (the default), 2 loop (repeat the key
/// range). One undo step.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_set_extend(doc: *mut Editor, layer: u32, mode: u32) -> bool {
    crate::ffi::guarded("pixmith_tween_set_extend", doc, || {
        let Some(x) = Extend::from_code(mode) else { return false };
        ed(doc).is_some_and(|e| set_extend(e, layer as usize, x))
    })
}

/// The mode `pixmith_tween_set_extend` takes (1, hold, for a layer without tween state).
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_extend(doc: *const Editor, layer: u32) -> u32 {
    crate::ffi::guarded("pixmith_tween_extend", std::ptr::null_mut(), || {
        doc.as_ref().map_or(1, |e| extend_of(&e.sprite, layer as usize).code())
    })
}

/// `mode`: 0 all keys, 1 frames `from..=to`, 2 current frame (`from`). Returns the layer index the
/// cels went to, or -1 on failure.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_bake_ex(
    doc: *mut Editor,
    layer: u32,
    mode: u32,
    from: u32,
    to: u32,
    new_layer: bool,
    keep_source: bool,
) -> i32 {
    crate::ffi::guarded("pixmith_tween_bake_ex", doc, || {
        let Some(e) = ed(doc) else { return -1 };
        let range = match mode {
            0 => BakeRange::All,
            1 => BakeRange::Frames(from, to),
            2 => BakeRange::Current(from),
            _ => return -1,
        };
        bake_with(e, layer as usize, BakeOptions { range, new_layer, keep_source }).map_or(-1, |l| l as i32)
    })
}

/// Bakes every tweened layer as one undo step; returns how many were baked.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_bake_all(doc: *mut Editor, keep_source: bool, new_layer: bool) -> u32 {
    crate::ffi::guarded("pixmith_tween_bake_all", doc, || {
        ed(doc).map_or(0, |e| bake_all(e, keep_source, new_layer) as u32)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_move_key(doc: *mut Editor, layer: u32, prop: u32, from: u32, to: u32) -> bool {
    crate::ffi::guarded("pixmith_tween_move_key", doc, || {
        let (Some(e), Some(p)) = (ed(doc), Prop::from_code(prop)) else { return false };
        move_key(e, layer as usize, p, from, to)
    })
}

/// Changes only the curve of an existing key (`ease` codes as in `pixmith_tween_set_key`).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_tween_set_ease(
    doc: *mut Editor,
    layer: u32,
    prop: u32,
    frame: u32,
    ease: u32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
) -> bool {
    crate::ffi::guarded("pixmith_tween_set_ease", doc, || {
        let (Some(e), Some(p), Some(ease)) = (ed(doc), Prop::from_code(prop), Ease::from_code(ease, [x1, y1, x2, y2]))
        else {
            return false;
        };
        set_ease(e, layer as usize, p, frame, ease)
    })
}

/// Copies the keys of frames `from..=to` (all properties); returns how many.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_copy_keys(doc: *const Editor, layer: u32, from: u32, to: u32) -> u32 {
    crate::ffi::guarded("pixmith_tween_copy_keys", std::ptr::null_mut(), || {
        doc.as_ref().map_or(0, |e| copy_keys(&e.sprite, layer as usize, from.min(to), from.max(to)) as u32)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_paste_keys(doc: *mut Editor, layer: u32, at: u32) -> u32 {
    crate::ffi::guarded("pixmith_tween_paste_keys", doc, || {
        ed(doc).map_or(0, |e| paste_keys(e, layer as usize, at) as u32)
    })
}

/// Sets several properties' keys at one frame: bit i of `mask` selects property i, `values` holds 6
/// floats indexed by property. One undo step.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_set_keys_at(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    mask: u32,
    values: *const f32,
    ease: u32,
) -> bool {
    crate::ffi::guarded("pixmith_tween_set_keys_at", doc, || {
        let (Some(e), false) = (ed(doc), values.is_null()) else { return false };
        let Some(ease) = Ease::from_code(ease, [0.0; 4]).filter(|e| !matches!(e, Ease::Bezier(..))) else {
            return false;
        };
        let mut v = [0f32; 6];
        v.copy_from_slice(std::slice::from_raw_parts(values, 6));
        v.iter().all(|x| x.is_finite()) && set_keys_at(e, layer as usize, frame, mask, &v, ease)
    })
}

/// Writes 8 floats: x, y (rounded), rotation, scale x, scale y, opacity, pivot x, pivot y.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_transform_at(doc: *const Editor, layer: u32, frame: u32, out: *mut f32) -> bool {
    crate::ffi::guarded("pixmith_tween_transform_at", std::ptr::null_mut(), || {
        let (Some(e), false) = (doc.as_ref(), out.is_null()) else { return false };
        let Some(t) = transform_at(&e.sprite, layer as usize, frame) else { return false };
        std::slice::from_raw_parts_mut(out, 8).copy_from_slice(&t);
        true
    })
}

/// Writes 4 floats: x0, y0, x1, y1 of the source content.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_source_bounds(doc: *const Editor, layer: u32, out: *mut f32) -> bool {
    crate::ffi::guarded("pixmith_tween_source_bounds", std::ptr::null_mut(), || {
        let (Some(e), false) = (doc.as_ref(), out.is_null()) else { return false };
        let Some(b) = source_bounds(&e.sprite, layer as usize) else { return false };
        std::slice::from_raw_parts_mut(out, 4).copy_from_slice(&[b.0 as f32, b.1 as f32, b.2 as f32, b.3 as f32]);
        true
    })
}

/// Writes the first and last key frame.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_range(doc: *const Editor, layer: u32, out: *mut u32) -> bool {
    crate::ffi::guarded("pixmith_tween_range", std::ptr::null_mut(), || {
        let (Some(e), false) = (doc.as_ref(), out.is_null()) else { return false };
        let Some((a, b)) = load(&e.sprite, layer as usize).and_then(|st| st.range()) else { return false };
        std::slice::from_raw_parts_mut(out, 2).copy_from_slice(&[a, b]);
        true
    })
}

/// The warning text for a layer (free with `pixmith_string_free`), or NULL when there is none.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_warning_text(doc: *const Editor, layer: u32) -> *mut std::ffi::c_char {
    crate::ffi::guarded("pixmith_tween_warning_text", std::ptr::null_mut(), || {
        let Some(e) = doc.as_ref() else { return std::ptr::null_mut() };
        let t = warning_text(warnings(&e.sprite, layer as usize));
        if t.is_empty() {
            return std::ptr::null_mut();
        }
        std::ffi::CString::new(t).map_or(std::ptr::null_mut(), |c| c.into_raw())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];

    fn sprite16() -> Editor {
        let mut s = Sprite::new(16, 16);
        // Asymmetric shape filling the full canvas.
        for y in 0..16 {
            for x in 0..16 {
                let c = [(x * 16) as u8, (y * 16) as u8, ((x ^ y) * 9) as u8, 255];
                s.draw_line(0, 0, x, y, x, y, c, 1);
            }
        }
        while s.frame_count() < 6 {
            s.add_frame(s.frame_count() - 1, false);
        }
        Editor::new(s)
    }

    fn lin() -> Ease {
        Ease::Linear
    }

    fn frame_px(e: &Editor, f: usize) -> Vec<u8> {
        e.sprite.render(f)
    }

    fn cel_px(e: &Editor, f: usize) -> Option<Vec<u8>> {
        e.sprite.cels[f][0].as_ref().map(|c| c.pixels.clone())
    }

    #[test]
    fn identity_is_pixel_exact() {
        let mut e = sprite16();
        let before = frame_px(&e, 0);
        assert!(set_key(&mut e, 0, Prop::Rotation, 0, 0.0, lin()));
        assert!(set_key(&mut e, 0, Prop::Rotation, 4, 0.0, lin()));
        assert!(set_key(&mut e, 0, Prop::ScaleX, 0, 1.0, lin()));
        for f in 0..=4 {
            assert_eq!(frame_px(&e, f), before, "frame {f}");
        }
        // Even through the full warp path (RotSprite off, nearest) an identity warp is exact.
        let cel = e.sprite.cels[0][0].clone().unwrap();
        let p = Params { pivot: (8.0, 8.0), tx: 0.0, ty: 0.0, sx: 1.0, sy: 1.0, angle: 0.0 };
        assert_eq!(warp(&cel.pixels, 16, 16, &p, None), cel.pixels);
    }

    #[test]
    fn eases_at_0_half_1() {
        let cases = [
            (Ease::Linear, 0.5),
            (Ease::InQuad, 0.25),
            (Ease::OutQuad, 0.75),
            (Ease::InOutQuad, 0.5),
            (Ease::InOutCubic, 0.5),
            (Ease::OutBounce, 0.765625),
        ];
        for (ease, half) in cases {
            assert!(ease.apply(0.0).abs() < 1e-12, "{ease:?} at 0");
            assert!((ease.apply(0.5) - half).abs() < 1e-9, "{ease:?} at .5");
            assert!((ease.apply(1.0) - 1.0).abs() < 1e-9, "{ease:?} at 1");
        }
        let b = Ease::OutBack;
        assert!(b.apply(0.0).abs() < 1e-9 && (b.apply(1.0) - 1.0).abs() < 1e-9);
        assert!(b.apply(0.5) > 1.0, "back overshoots");
        assert_eq!(Ease::Hold.apply(0.0), 0.0);
        assert_eq!(Ease::Hold.apply(0.5), 0.0);
        assert_eq!(Ease::Hold.apply(1.0), 1.0);
    }

    #[test]
    fn bezier_sanity() {
        // (0,0,1,1) is the identity curve; CSS "ease" is monotonic and below the diagonal early on.
        let id = Ease::Bezier(0.0, 0.0, 1.0, 1.0);
        for i in 0..=10 {
            let t = i as f64 / 10.0;
            assert!((id.apply(t) - t).abs() < 1e-6);
        }
        let ease = Ease::Bezier(0.25, 0.1, 0.25, 1.0);
        assert!(ease.apply(0.0).abs() < 1e-9 && (ease.apply(1.0) - 1.0).abs() < 1e-9);
        let mut last = 0.0;
        for i in 0..=100 {
            let v = ease.apply(i as f64 / 100.0);
            assert!(v >= last - 1e-9);
            last = v;
        }
        assert!((ease.apply(0.5) - 0.8024).abs() < 2e-3, "{}", ease.apply(0.5));
    }

    #[test]
    fn value_at_holds_and_eases() {
        let t = TweenTrack {
            layer: 0,
            property: Prop::PosX,
            keys: vec![
                Key { frame: 2, value: 10.0, ease: Ease::Linear },
                Key { frame: 6, value: 30.0, ease: Ease::Hold },
                Key { frame: 8, value: 0.0, ease: Ease::Linear },
            ],
        };
        assert_eq!(value_at(&t, 0), 10.0);
        assert_eq!(value_at(&t, 4), 20.0);
        assert_eq!(value_at(&t, 7), 30.0);
        assert_eq!(value_at(&t, 99), 0.0);
    }

    #[test]
    fn pivot_half_pixel_math_90_degrees() {
        let mut e = sprite16();
        let src = cel_px(&e, 0).unwrap();
        assert_eq!(effective_pivot(&e.sprite, 0), Some((8.0, 8.0)));
        set_key(&mut e, 0, Prop::Rotation, 0, 90.0, lin());
        set_key(&mut e, 0, Prop::Rotation, 3, 90.0, lin());
        let out = frame_px(&e, 1);
        // Clockwise 90 degrees about (8,8): source (x, y) lands on (15 - y, x).
        for y in 0..16usize {
            for x in 0..16usize {
                let d = ((x) * 16 + (15 - y)) * 4;
                let s = (y * 16 + x) * 4;
                assert_eq!(out[d..d + 4], src[s..s + 4], "src ({x},{y})");
            }
        }
    }

    #[test]
    fn rotation_90_is_lossless_and_no_new_colors() {
        let mut e = sprite16();
        let src = cel_px(&e, 0).unwrap();
        set_key(&mut e, 0, Prop::Rotation, 0, 0.0, lin());
        set_key(&mut e, 0, Prop::Rotation, 4, 360.0, lin());
        // 4 steps of 90 degrees land exactly on 90, 180, 270, 360.
        let r1 = frame_px(&e, 1);
        let rotated_back = {
            let p = Params { pivot: (8.0, 8.0), tx: 0.0, ty: 0.0, sx: 1.0, sy: 1.0, angle: 270.0 };
            warp(&r1, 16, 16, &p, None)
        };
        assert_eq!(rotated_back, src);
        assert_eq!(frame_px(&e, 4), src);
        let colors: std::collections::HashSet<_> = r1.chunks(4).collect();
        let orig: std::collections::HashSet<_> = src.chunks(4).collect();
        assert_eq!(colors, orig);
    }

    #[test]
    fn frames_do_not_chain() {
        let mut e = sprite16();
        set_key(&mut e, 0, Prop::Rotation, 0, 0.0, lin());
        set_key(&mut e, 0, Prop::Rotation, 5, 37.0, lin());
        set_key(&mut e, 0, Prop::PosX, 0, 0.0, lin());
        set_key(&mut e, 0, Prop::PosX, 5, 3.0, lin());
        let direct = frame_px(&e, 3);
        for f in [5, 1, 4, 2, 0, 3] {
            let _ = frame_px(&e, f);
        }
        assert_eq!(frame_px(&e, 3), direct);
        assert_eq!(frame_px(&e, 3), direct);
        // The source cel is untouched.
        assert!(e.sprite.cels[0][0].is_some());
    }

    #[test]
    fn translation_rounds_half_up() {
        assert_eq!(round_half_up(0.5), 1.0);
        assert_eq!(round_half_up(-0.5), 0.0);
        assert_eq!(round_half_up(-1.5), -1.0);
        let mut e = sprite16();
        set_key(&mut e, 0, Prop::PosX, 0, 0.0, lin());
        set_key(&mut e, 0, Prop::PosX, 2, 1.0, lin());
        let src = cel_px(&e, 0).unwrap();
        let out = frame_px(&e, 1); // 0.5 -> 1
        for y in 0..16usize {
            for x in 1..16usize {
                assert_eq!(out[(y * 16 + x) * 4..][..4], src[(y * 16 + x - 1) * 4..][..4]);
            }
            assert_eq!(out[(y * 16) * 4 + 3], 0);
        }
    }

    #[test]
    fn rotsprite_is_cached_and_nearest_option_persists() {
        let mut e = sprite16();
        set_key(&mut e, 0, Prop::Rotation, 0, 0.0, lin());
        set_key(&mut e, 0, Prop::Rotation, 4, 40.0, lin());
        let a = frame_px(&e, 2);
        let b = frame_px(&e, 2);
        assert_eq!(a, b);
        // Same source cel + params: the cached buffer itself is returned.
        let cel = e.sprite.cels[0][0].clone().unwrap();
        let p = Xf::Own(Params { pivot: (8.0, 8.0), tx: 0.0, ty: 0.0, sx: 1.0, sy: 1.0, angle: 20.0 });
        let (r1, r2) =
            (transformed(Source::Cel(&cel), 16, 16, &p, true), transformed(Source::Cel(&cel), 16, 16, &p, true));
        assert!(Arc::ptr_eq(&r1, &r2), "second request served from cache");
        assert!(WARPS.load(std::sync::atomic::Ordering::Relaxed) > 0);
        set_rotsprite(&mut e, 0, false);
        assert_eq!(pixmith_tween_options_for_test(&e), 1);
        let c = frame_px(&e, 2);
        assert_eq!(c.len(), a.len());
    }

    fn pixmith_tween_options_for_test(e: &Editor) -> u32 {
        unsafe { pixmith_tween_options(e, 0) & 3 }
    }

    #[test]
    fn non_integer_scale_warns() {
        let mut e = sprite16();
        set_key(&mut e, 0, Prop::ScaleX, 0, 1.0, lin());
        set_key(&mut e, 0, Prop::ScaleX, 2, 2.0, Ease::Hold);
        assert_eq!(warnings(&e.sprite, 0), 1); // 1.5 at frame 1
        let mut e = sprite16();
        set_key(&mut e, 0, Prop::ScaleX, 0, 1.0, Ease::Hold);
        set_key(&mut e, 0, Prop::ScaleX, 2, 2.0, lin());
        assert_eq!(warnings(&e.sprite, 0), 0);
    }

    #[test]
    fn opacity_multiplies_cel_opacity() {
        let mut e = sprite16();
        set_key(&mut e, 0, Prop::Opacity, 0, 1.0, lin());
        set_key(&mut e, 0, Prop::Opacity, 2, 0.0, lin());
        let cel = tweened(&e.sprite, 1, 0).unwrap();
        assert_eq!(cel.opacity, 128); // 255 * 0.5 rounded half up
    }

    #[test]
    fn bake_writes_cels_and_undoes() {
        let mut e = sprite16();
        set_key(&mut e, 0, Prop::PosX, 0, 0.0, lin());
        set_key(&mut e, 0, Prop::PosX, 4, 4.0, lin());
        let before: Vec<_> = (0..6).map(|f| cel_px(&e, f)).collect();
        let shown: Vec<_> = (0..6).map(|f| frame_px(&e, f)).collect();
        assert!(bake(&mut e, 0, false));
        assert!(load(&e.sprite, 0).is_none());
        assert!(e.sprite.layers[0].user_data.text.is_none());
        for f in 0..=4 {
            assert_eq!(frame_px(&e, f), shown[f], "baked frame {f} matches preview");
        }
        e.undo();
        let after: Vec<_> = (0..6).map(|f| cel_px(&e, f)).collect();
        assert_eq!(before, after);
        assert!(load(&e.sprite, 0).is_some(), "undo restores the tracks");
    }

    #[test]
    fn bake_keep_source_keeps_tracks_and_source() {
        let mut e = sprite16();
        set_key(&mut e, 0, Prop::PosX, 0, 3.0, lin());
        set_key(&mut e, 0, Prop::PosX, 4, 6.0, lin());
        let source = cel_px(&e, 0);
        assert!(bake(&mut e, 0, true));
        assert_eq!(cel_px(&e, 0), source);
        assert!(load(&e.sprite, 0).is_some());
        assert!(cel_px(&e, 2).is_some());
        let shown = frame_px(&e, 2);
        assert!(bake(&mut e, 0, true));
        assert_eq!(frame_px(&e, 2), shown, "re-bake is stable");
    }

    #[test]
    fn persistence_round_trips_through_aseprite() {
        let mut e = sprite16();
        e.sprite.layers[0].user_data.text = Some("my note".into());
        set_key(&mut e, 0, Prop::Rotation, 0, 0.0, Ease::Bezier(0.25, 0.1, 0.25, 1.0));
        set_key(&mut e, 0, Prop::Rotation, 4, 45.5, Ease::OutBack);
        set_key(&mut e, 0, Prop::PosY, 1, -2.25, Ease::Hold);
        set_pivot(&mut e, 0, Some((7.5, 8.0)));
        set_rotsprite(&mut e, 0, false);
        let want = load(&e.sprite, 0).unwrap();
        let shown = frame_px(&e, 2);
        let bytes = crate::formats::ase::write(&e.sprite);
        let back = crate::formats::ase::read(&bytes).unwrap();
        assert_eq!(load(&back, 0).unwrap(), want);
        assert_eq!(split_text(back.layers[0].user_data.text.as_deref().unwrap()).0, "my note");
        assert_eq!(back.render(2), shown);
    }

    #[test]
    fn no_tracks_render_is_unchanged() {
        let e = sprite16();
        assert!(tweened(&e.sprite, 0, 0).is_none());
        let mut plain = e.sprite.clone();
        plain.layers[0].user_data.text = Some("just a note".into());
        assert!(tweened(&plain, 0, 0).is_none());
        assert_eq!(plain.render(0), e.sprite.render(0));
        // A pivot-only state does not change rendering either.
        let mut e2 = sprite16();
        set_pivot(&mut e2, 0, Some((3.0, 3.0)));
        assert_eq!(e2.sprite.render(1), e.sprite.render(1));
        let _ = RED;
    }

    #[test]
    fn remove_key_clears_state() {
        let mut e = sprite16();
        set_key(&mut e, 0, Prop::PosX, 1, 2.0, lin());
        assert!(remove_key(&mut e, 0, Prop::PosX, 1));
        assert!(load(&e.sprite, 0).is_none());
        assert!(e.sprite.layers[0].user_data.text.is_none());
    }

    // MARK: Extended features

    fn two_layers() -> Editor {
        let mut e = sprite16();
        let top = e.sprite.add_layer(0);
        e.sprite.draw_line(top, 0, 2, 2, 5, 2, [255, 255, 255, 255], 1);
        e
    }

    #[test]
    fn group_tween_transforms_the_composite() {
        let mut s = Sprite::new(8, 8);
        s.layers[0].child_level = 1;
        let mut group = crate::document::Layer::new("Group");
        group.kind = 1;
        s.layers.insert(0, group);
        s.cels[0].insert(0, None);
        s.draw_line(1, 0, 1, 1, 1, 1, RED, 1);
        s.add_frame(0, false);
        s.add_frame(1, false);
        let mut e = Editor::new(s);
        assert!(set_key(&mut e, 0, Prop::PosX, 0, 0.0, lin()));
        assert!(set_key(&mut e, 0, Prop::PosX, 2, 4.0, lin()));
        let at = |e: &Editor, f: usize, x: usize, y: usize| e.sprite.render(f)[(y * 8 + x) * 4..][..4].to_vec();
        assert_eq!(at(&e, 0, 1, 1), RED);
        assert_eq!(at(&e, 1, 3, 1), RED);
        assert_eq!(at(&e, 1, 1, 1)[3], 0);
        assert_eq!(at(&e, 2, 5, 1), RED);
        // Baking a group goes into a new layer and hides the group.
        let before = e.sprite.layers.len();
        let shown: Vec<_> = (0..3).map(|f| e.sprite.render(f)).collect();
        let t = bake_with(&mut e, 0, BakeOptions::all(false)).unwrap();
        assert_eq!(e.sprite.layers.len(), before + 1);
        assert!(!e.sprite.layers[0].visible);
        assert_eq!(e.sprite.layers[t].name, "Group (baked)");
        for f in 0..3 {
            assert_eq!(e.sprite.render(f), shown[f], "baked group frame {f}");
        }
        e.undo();
        assert_eq!(e.sprite.layers.len(), before);
        assert!(load(&e.sprite, 0).is_some());
    }

    #[test]
    fn bake_ranges_and_new_layer() {
        let mut e = sprite16();
        set_key(&mut e, 0, Prop::PosX, 0, 0.0, lin());
        set_key(&mut e, 0, Prop::PosX, 4, 4.0, lin());
        let shown: Vec<_> = (0..6).map(|f| frame_px(&e, f)).collect();
        // Current frame only.
        assert!(bake_with(
            &mut e,
            0,
            BakeOptions { range: BakeRange::Current(2), new_layer: false, keep_source: true }
        )
        .is_some());
        assert!(load(&e.sprite, 0).is_some());
        assert!(cel_px(&e, 2).is_some() && cel_px(&e, 1).is_none() && cel_px(&e, 3).is_none());
        // Range 1..=3, tracks kept even without keep_source because the range is partial.
        assert!(bake_with(
            &mut e,
            0,
            BakeOptions { range: BakeRange::Frames(1, 3), new_layer: false, keep_source: false }
        )
        .is_some());
        assert!(load(&e.sprite, 0).is_some());
        assert!(cel_px(&e, 1).is_some() && cel_px(&e, 3).is_some() && cel_px(&e, 4).is_none());
        // Past the last key is refused when the layer shows its own cels there (`Rest`), allowed when it holds.
        assert!(set_extend(&mut e, 0, Extend::Rest));
        assert!(bake_with(
            &mut e,
            0,
            BakeOptions { range: BakeRange::Frames(3, 5), new_layer: false, keep_source: true }
        )
        .is_none());
        assert!(set_extend(&mut e, 0, Extend::Hold));
        assert!(
            bake_with(&mut e, 0, BakeOptions { range: BakeRange::Frames(3, 9), new_layer: false, keep_source: true })
                .is_none(),
            "the sprite has 6 frames"
        );
        assert!(set_extend(&mut e, 0, Extend::Rest));
        // New layer: original hidden and untouched, copy shows the same frames.
        let n = e.sprite.layers.len();
        let t =
            bake_with(&mut e, 0, BakeOptions { range: BakeRange::All, new_layer: true, keep_source: true }).unwrap();
        assert_eq!(e.sprite.layers.len(), n + 1);
        assert_eq!(t, 1);
        assert!(!e.sprite.layers[0].visible);
        for f in 0..=4 {
            assert_eq!(frame_px(&e, f), shown[f], "frame {f}");
        }
    }

    /// 16x16 sprite with 8 frames and a rotation track: 0 degrees at frame 0, 90 at frame 4.
    fn bent_cycle() -> Editor {
        let mut e = sprite16();
        while e.sprite.frame_count() < 8 {
            let n = e.sprite.frame_count();
            e.sprite.add_frame(n - 1, false);
        }
        assert!(set_key(&mut e, 0, Prop::Rotation, 0, 0.0, lin()));
        assert!(set_key(&mut e, 0, Prop::Rotation, 4, 90.0, lin()));
        e
    }

    #[test]
    fn frames_after_the_last_key_hold_by_default() {
        let e = bent_cycle();
        assert_eq!(extend_of(&e.sprite, 0), Extend::Hold);
        let (first, bent) = (frame_px(&e, 0), frame_px(&e, 4));
        assert_ne!(first, bent);
        for f in 5..8 {
            assert_eq!(frame_px(&e, f), bent, "frame {f} keeps the pose of the last key");
        }
        assert_eq!(e.sprite.render(3).len(), bent.len());
        assert_eq!(transform_at(&e.sprite, 0, 7).unwrap()[2], 90.0);
        // Before the first key the layer's own cels still show (the source image is the cel at the first key).
        let mut e = sprite16();
        let own = frame_px(&e, 0);
        set_key(&mut e, 0, Prop::Rotation, 2, 0.0, lin());
        set_key(&mut e, 0, Prop::Rotation, 4, 90.0, lin());
        assert_eq!(frame_px(&e, 0), own);
        assert!(tweened(&e.sprite, 1, 0).is_none() && tweened(&e.sprite, 5, 0).is_some());
    }

    #[test]
    fn rest_option_keeps_the_old_behaviour_and_old_files_read_as_rest() {
        let mut e = bent_cycle();
        let own = frame_px(&e, 0);
        assert!(set_extend(&mut e, 0, Extend::Rest));
        for f in 5..8 {
            assert_ne!(frame_px(&e, f), frame_px(&e, 4));
        }
        assert!(tweened(&e.sprite, 5, 0).is_none());
        let _ = own;
        // Rest is the absence of `ex`: a payload written before the option existed has none and reads as Rest.
        let text = e.sprite.layers[0].user_data.text.clone().unwrap();
        assert!(!text.contains("ex="), "{text}");
        assert_eq!(load(&e.sprite, 0).unwrap().extend, Extend::Rest);
        // Hold and loop are stored and survive a file round trip.
        for x in [Extend::Hold, Extend::Loop] {
            assert!(set_extend(&mut e, 0, x));
            assert!(e.sprite.layers[0].user_data.text.as_deref().unwrap().contains(&format!("ex={}", x.name())));
            let back = crate::formats::pxmt::read(&crate::formats::pxmt::write(&e.sprite).unwrap()).unwrap();
            assert_eq!(extend_of(&back, 0), x);
        }
        // The setting is one undo step, and the ABI takes the same codes.
        assert!(set_extend(&mut e, 0, Extend::Rest));
        e.undo();
        assert_eq!(extend_of(&e.sprite, 0), Extend::Loop);
        unsafe {
            let p: *mut Editor = &mut e;
            assert!(pixmith_tween_set_extend(p, 0, 1));
            assert_eq!(pixmith_tween_extend(p, 0), 1);
            assert!(!pixmith_tween_set_extend(p, 0, 3));
        }
        assert_eq!(Extend::from_name("loop"), Some(Extend::Loop));
        assert_eq!(Extend::from_name("nope"), None);
    }

    #[test]
    fn loop_repeats_the_key_range() {
        let mut e = bent_cycle();
        assert!(set_extend(&mut e, 0, Extend::Loop));
        // The key range 0..=4 repeats: frame 5 shows frame 0, 6 shows 1, 7 shows 2.
        for (f, same) in [(5, 0), (6, 1), (7, 2)] {
            assert_eq!(frame_px(&e, f), frame_px(&e, same), "frame {f}");
        }
        assert_eq!(transform_at(&e.sprite, 0, 5).unwrap()[2], 0.0);
        assert_eq!(transform_at(&e.sprite, 0, 6).unwrap()[2], 22.5);
        // A single key has nothing to repeat.
        let mut one = sprite16();
        set_key(&mut one, 0, Prop::Rotation, 1, 90.0, lin());
        set_extend(&mut one, 0, Extend::Loop);
        assert_eq!(frame_px(&one, 3), frame_px(&one, 1));
    }

    #[test]
    fn bake_writes_the_held_and_looped_frames_too() {
        for x in [Extend::Hold, Extend::Loop] {
            let mut e = bent_cycle();
            set_extend(&mut e, 0, x);
            let shown: Vec<_> = (0..8).map(|f| frame_px(&e, f)).collect();
            assert!(bake(&mut e, 0, false));
            assert!(load(&e.sprite, 0).is_none(), "{x:?}: the tracks went away");
            for f in 0..8 {
                assert_eq!(frame_px(&e, f), shown[f], "{x:?}: frame {f} looks the same after the bake");
            }
        }
        // Rest only bakes the key range, as before.
        let mut e = bent_cycle();
        set_extend(&mut e, 0, Extend::Rest);
        let own = cel_px(&e, 6);
        assert!(bake(&mut e, 0, false));
        assert_eq!(cel_px(&e, 6), own);
    }

    #[test]
    fn rig_children_use_the_same_rule() {
        use super::rig::{set_parent, world_matrix};
        let mut e = bent_cycle();
        let child = e.sprite.add_layer(0);
        e.sprite.draw_line(child, 0, 2, 2, 9, 2, [255, 255, 255, 255], 1);
        assert!(set_parent(&mut e, child, Some(0)));
        let at = |e: &Editor, f| world_matrix(&e.sprite, child, f).unwrap();
        // The keyless child follows its parent, and holds the pose of the parent's last key like the parent.
        assert_eq!(extend_of(&e.sprite, child), Extend::Hold);
        assert_ne!(at(&e, 0), at(&e, 4));
        assert_eq!(at(&e, 7), at(&e, 4));
        assert_eq!(frame_px(&e, 7), frame_px(&e, 4));
        assert!(tweened(&e.sprite, 7, child).is_some());
        // Loop: frame 6 poses like frame 1, for the ancestors too.
        set_extend(&mut e, child, Extend::Loop);
        assert_eq!(at(&e, 6), at(&e, 1));
        // Rest: the child shows its own cels after the parent's last key.
        set_extend(&mut e, child, Extend::Rest);
        assert!(tweened(&e.sprite, 7, child).is_none());
        // Baking the child agrees with what is shown.
        set_extend(&mut e, child, Extend::Hold);
        let shown: Vec<_> = (0..8).map(|f| frame_px(&e, f)).collect();
        assert!(bake_all(&mut e, false, false) >= 2);
        for f in 0..8 {
            assert_eq!(frame_px(&e, f), shown[f], "frame {f}");
        }
    }

    #[test]
    fn bake_all_is_one_undo_step() {
        let mut e = two_layers();
        for l in 0..2 {
            set_key(&mut e, l, Prop::PosX, 0, 0.0, lin());
            set_key(&mut e, l, Prop::PosX, 2, 2.0, lin());
        }
        let shown: Vec<_> = (0..3).map(|f| frame_px(&e, f)).collect();
        let (undo_before, _) = e.history_len();
        assert_eq!(bake_all(&mut e, false, false), 2);
        assert_eq!(e.history_len().0, undo_before + 1);
        for f in 0..3 {
            assert_eq!(frame_px(&e, f), shown[f]);
        }
        assert!(load(&e.sprite, 0).is_none() && load(&e.sprite, 1).is_none());
        e.undo();
        assert!(load(&e.sprite, 0).is_some() && load(&e.sprite, 1).is_some());
    }

    #[test]
    fn baked_copy_leaves_the_document_alone() {
        let mut e = sprite16();
        set_key(&mut e, 0, Prop::PosX, 0, 0.0, lin());
        set_key(&mut e, 0, Prop::PosX, 4, 4.0, lin());
        let copy = baked_copy(&e.sprite);
        assert!(load(&copy, 0).is_none());
        assert!(load(&e.sprite, 0).is_some());
        for f in 0..6 {
            assert_eq!(copy.render(f), e.sprite.render(f));
        }
    }

    #[test]
    fn exports_use_tweened_frames() {
        let mut e = sprite16();
        set_key(&mut e, 0, Prop::PosX, 0, 0.0, lin());
        set_key(&mut e, 0, Prop::PosX, 4, 4.0, lin());
        let tweened = crate::formats::export::frame_png(&e.sprite, 2, 1).unwrap();
        let baked = crate::formats::export::frame_png(&baked_copy(&e.sprite), 2, 1).unwrap();
        assert_eq!(tweened, baked);
        let mut plain = e.sprite.clone();
        store(&mut plain, 0, None);
        assert_ne!(tweened, crate::formats::export::frame_png(&plain, 2, 1).unwrap(), "the tween changes the export");
        assert_eq!(
            crate::formats::export::sheet_png(&e.sprite, 1).unwrap(),
            crate::formats::export::sheet_png(&baked_copy(&e.sprite), 1).unwrap()
        );
    }

    #[test]
    fn keys_move_copy_paste_and_set_ease() {
        let mut e = sprite16();
        set_key(&mut e, 0, Prop::PosX, 0, 1.0, lin());
        set_key(&mut e, 0, Prop::PosX, 2, 5.0, Ease::OutQuad);
        set_key(&mut e, 0, Prop::Rotation, 2, 90.0, lin());
        assert!(move_key(&mut e, 0, Prop::PosX, 2, 4));
        let t = load(&e.sprite, 0).unwrap();
        assert_eq!(t.track(Prop::PosX).unwrap().keys.iter().map(|k| k.frame).collect::<Vec<_>>(), vec![0, 4]);
        assert_eq!(t.track(Prop::PosX).unwrap().keys[1].ease, Ease::OutQuad);
        assert!(set_ease(&mut e, 0, Prop::PosX, 4, Ease::Hold));
        assert_eq!(load(&e.sprite, 0).unwrap().track(Prop::PosX).unwrap().keys[1].ease, Ease::Hold);
        // Moving onto an existing key replaces it.
        set_key(&mut e, 0, Prop::PosX, 3, 9.0, lin());
        assert!(move_key(&mut e, 0, Prop::PosX, 3, 4));
        assert_eq!(load(&e.sprite, 0).unwrap().track(Prop::PosX).unwrap().keys.len(), 2);
        // Copy frames 0..=2 (PosX@0, Rotation@2) and paste at frame 3.
        assert_eq!(copy_keys(&e.sprite, 0, 0, 2), 2);
        assert_eq!(paste_keys(&mut e, 0, 3), 2);
        let t = load(&e.sprite, 0).unwrap();
        assert!(t.track(Prop::PosX).unwrap().keys.iter().any(|k| k.frame == 3 && k.value == 1.0));
        assert!(t.track(Prop::Rotation).unwrap().keys.iter().any(|k| k.frame == 5 && k.value == 90.0));
        e.undo();
        assert!(!load(&e.sprite, 0).unwrap().track(Prop::PosX).unwrap().keys.iter().any(|k| k.frame == 3));
    }

    #[test]
    fn set_keys_at_is_one_step_and_path_reports_positions() {
        let mut e = sprite16();
        let (u0, _) = e.history_len();
        assert!(set_keys_at(&mut e, 0, 0, 0b11, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], lin()));
        assert!(set_keys_at(&mut e, 0, 4, 0b11, &[8.0, -4.0, 0.0, 1.0, 1.0, 1.0], lin()));
        assert_eq!(e.history_len().0, u0 + 2);
        let t = transform_at(&e.sprite, 0, 2).unwrap();
        assert_eq!((t[0], t[1]), (4.0, -2.0));
        assert_eq!((t[6], t[7]), (8.0, 8.0));
    }

    #[test]
    fn snap_scale_option_rounds_and_silences_the_warning() {
        let mut e = sprite16();
        set_key(&mut e, 0, Prop::ScaleX, 0, 1.0, lin());
        set_key(&mut e, 0, Prop::ScaleX, 4, 2.0, lin());
        assert_eq!(warnings(&e.sprite, 0) & 1, 1);
        assert!(warning_text(1).contains("Snap scale"));
        set_flag(&mut e, 0, 1, true);
        assert_eq!(warnings(&e.sprite, 0) & 1, 0);
        assert_eq!(transform_at(&e.sprite, 0, 1).unwrap()[3], 1.0); // 1.25 -> 1
        assert_eq!(transform_at(&e.sprite, 0, 2).unwrap()[3], 2.0); // 1.5 -> 2
        let back = crate::formats::ase::read(&crate::formats::ase::write(&e.sprite)).unwrap();
        assert!(load(&back, 0).unwrap().snap_scale);
    }

    #[test]
    fn rotsprite_cache_invalidates_when_the_source_changes() {
        let mut e = sprite16();
        set_key(&mut e, 0, Prop::Rotation, 0, 0.0, lin());
        set_key(&mut e, 0, Prop::Rotation, 4, 40.0, lin());
        let before = frame_px(&e, 2);
        assert_eq!(frame_px(&e, 2), before);
        e.checkpoint();
        e.sprite.draw_line(0, 0, 8, 8, 8, 8, [1, 2, 3, 255], 1);
        let after = frame_px(&e, 2);
        assert_ne!(before, after, "editing the source must not hit a stale cache entry");
        // Changing rotation params gives a different image than the cached one.
        set_key(&mut e, 0, Prop::Rotation, 4, 50.0, lin());
        assert_ne!(frame_px(&e, 2), after);
        // RotSprite and nearest differ on a diagonal-rich image but both keep 16x16 RGBA.
        set_rotsprite(&mut e, 0, false);
        assert_eq!(frame_px(&e, 2).len(), after.len());
    }

    #[test]
    fn indexed_mode_stays_on_palette_and_opacity_warns() {
        let mut s = Sprite::new(8, 8);
        s.color_mode = crate::document::color_mode::ColorMode::Indexed;
        s.palette = vec![[0, 0, 0, 255], [255, 0, 0, 255], [0, 255, 0, 255]];
        for (x, c) in [(1, [255, 0, 0, 255]), (2, [0, 255, 0, 255]), (3, [255, 0, 0, 255])] {
            s.draw_line(0, 0, x, 3, x, 4, c, 1);
        }
        s.add_frame(0, false);
        s.add_frame(1, false);
        let mut e = Editor::new(s);
        set_key(&mut e, 0, Prop::Rotation, 0, 0.0, lin());
        set_key(&mut e, 0, Prop::Rotation, 2, 33.0, lin());
        set_key(&mut e, 0, Prop::Opacity, 0, 1.0, lin());
        set_key(&mut e, 0, Prop::Opacity, 2, 0.5, lin());
        let allowed: std::collections::HashSet<[u8; 3]> = e.sprite.palette.iter().map(|c| [c[0], c[1], c[2]]).collect();
        for f in 0..3 {
            for p in e.sprite.render(f).chunks(4).filter(|p| p[3] != 0) {
                assert!(allowed.contains(&[p[0], p[1], p[2]]), "frame {f}: {p:?} not in palette");
            }
        }
        assert_eq!(warnings(&e.sprite, 0) & 2, 2);
        assert!(warning_text(2).contains("Indexed"));
        // Ignoring opacity makes the cel fully opaque and clears the warning.
        set_flag(&mut e, 0, 2, true);
        assert_eq!(warnings(&e.sprite, 0) & 2, 0);
        assert_eq!(tweened(&e.sprite, 2, 0).unwrap().opacity, 255);
        // An RGBA sprite never warns about opacity.
        let mut rgba = sprite16();
        set_key(&mut rgba, 0, Prop::Opacity, 0, 1.0, lin());
        assert_eq!(warnings(&rgba.sprite, 0) & 2, 0);
    }

    #[test]
    fn source_bounds_use_pixel_corners() {
        let mut e = sprite16();
        assert_eq!(source_bounds(&e.sprite, 0), Some((0.0, 0.0, 16.0, 16.0)));
        let mut s = Sprite::new(16, 16);
        s.draw_line(0, 0, 2, 3, 5, 3, RED, 1);
        e = Editor::new(s);
        assert_eq!(source_bounds(&e.sprite, 0), Some((2.0, 3.0, 6.0, 4.0)));
        assert_eq!(effective_pivot(&e.sprite, 0), Some((4.0, 3.5)));
    }

    #[test]
    fn one_undo_step_per_edit() {
        let mut e = sprite16();
        let (u0, _) = e.history_len();
        set_key(&mut e, 0, Prop::PosX, 1, 3.0, lin());
        set_pivot(&mut e, 0, Some((4.0, 4.5)));
        set_rotsprite(&mut e, 0, false);
        assert_eq!(e.history_len().0, u0 + 3);
        e.undo();
        assert!(load(&e.sprite, 0).unwrap().rotsprite);
        e.undo();
        assert_eq!(load(&e.sprite, 0).unwrap().pivot, None);
        e.undo();
        assert!(load(&e.sprite, 0).is_none());
    }

    #[test]
    fn user_text_survives_edits_and_removal() {
        let mut e = sprite16();
        e.sprite.layers[0].user_data.text = Some("note\nline two".into());
        set_key(&mut e, 0, Prop::PosX, 1, 3.0, lin());
        set_key(&mut e, 0, Prop::PosX, 2, 4.0, lin());
        assert!(remove_key(&mut e, 0, Prop::PosX, 1));
        assert!(remove_key(&mut e, 0, Prop::PosX, 2));
        assert_eq!(e.sprite.layers[0].user_data.text.as_deref(), Some("note\nline two"));
    }
}
