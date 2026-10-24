//! Color-picker math shared by both front ends: HSV/HSL conversions, color-wheel geometry, harmony hues,
//! tangent-space normal encoding and the bitmaps of the wheel and the normal-map disc.
//!
//! Everything here is pure (no document). The macOS app calls it through the `pixmith_cp_*` C ABI declared in
//! `pixmith_colorpick.h`; the egui app calls the Rust functions directly. Colors are `0xRRGGBBAA`; color
//! components are `f64` with hue in degrees (0..360) and everything else in 0..1.
//!
//! # Disc coordinates
//!
//! The wheel and the normal disc are drawn in a square of side 2 centred on the origin: `x` grows to the right and
//! `y` grows **down** (screen orientation), so a caller maps a pointer position straight to `(x, y)` and back with
//! no flipping. On the color wheel hue 0 (red) is at the east and hue grows counter-clockwise; the radius is the
//! saturation. On the normal disc `(x, y)` is the tangent-space normal's X and (flipped) Y, the centre is the flat
//! normal `(0, 0, 1)` = `#8080FF` and the rim is the horizon (Z = 0).

/// Wheel model: which color space the wheel's radius and the bar next to it control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelModel {
    /// Radius is HSV saturation, the bar is value.
    Hsv,
    /// Radius is HSL saturation, the bar is lightness.
    Hsl,
}

impl WheelModel {
    pub fn from_u32(v: u32) -> WheelModel {
        if v == 1 {
            WheelModel::Hsl
        } else {
            WheelModel::Hsv
        }
    }
}

/// Which colors to mark on the wheel besides the picked one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Harmony {
    None,
    Complementary,
    Triadic,
    Analogous,
    SplitComplementary,
    Square,
}

impl Harmony {
    pub const ALL: [Harmony; 6] = [
        Harmony::None,
        Harmony::Complementary,
        Harmony::Triadic,
        Harmony::Analogous,
        Harmony::SplitComplementary,
        Harmony::Square,
    ];

    pub fn from_u32(v: u32) -> Harmony {
        Harmony::ALL.get(v as usize).copied().unwrap_or(Harmony::None)
    }

    /// Hue offsets (degrees) from the picked hue.
    pub fn offsets(self) -> &'static [f64] {
        match self {
            Harmony::None => &[],
            Harmony::Complementary => &[180.0],
            Harmony::Triadic => &[120.0, 240.0],
            Harmony::Analogous => &[-30.0, 30.0],
            Harmony::SplitComplementary => &[150.0, 210.0],
            Harmony::Square => &[90.0, 180.0, 270.0],
        }
    }
}

/// The hues of a harmony around `hue` (the picked hue itself is not included), in 0..360.
pub fn harmony_hues(hue: f64, harmony: Harmony) -> Vec<f64> {
    harmony.offsets().iter().map(|o| (hue + o).rem_euclid(360.0)).collect()
}

// MARK: Packing

/// Splits `0xRRGGBBAA` into `([r, g, b], a)`, all 0..1.
pub fn unpack(rgba: u32) -> ([f64; 3], f64) {
    let b = rgba.to_be_bytes();
    ([b[0] as f64 / 255.0, b[1] as f64 / 255.0, b[2] as f64 / 255.0], b[3] as f64 / 255.0)
}

fn byte(f: f64) -> u8 {
    (f * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Packs 0..1 components into `0xRRGGBBAA`.
pub fn pack(rgb: [f64; 3], a: f64) -> u32 {
    u32::from_be_bytes([byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), byte(a)])
}

/// Rec. 601 luma of a color, 0..1 (what the Gray selector shows).
pub fn luma(rgb: [f64; 3]) -> f64 {
    0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2]
}

// MARK: HSV / HSL

fn hue_of(rgb: [f64; 3], mx: f64, d: f64) -> f64 {
    let [r, g, b] = rgb;
    let h = if mx == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if mx == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    h.rem_euclid(360.0)
}

/// `[h, s, v]` of an RGB color. Gray has no hue, and black (HSV) has no saturation: `hint` (`(hue, saturation)` of
/// what the user was just editing) fills in what is undefined so a drag does not jump.
pub fn rgb_to_hsv(rgb: [f64; 3], hint: Option<(f64, f64)>) -> [f64; 3] {
    let mx = rgb[0].max(rgb[1]).max(rgb[2]);
    let mn = rgb[0].min(rgb[1]).min(rgb[2]);
    let d = mx - mn;
    let (hh, hs) = hint.unwrap_or((0.0, 0.0));
    let h = if d == 0.0 { hh } else { hue_of(rgb, mx, d) };
    let s = if mx == 0.0 { hs } else { d / mx };
    [h, s, mx]
}

/// RGB of `[h, s, v]` (hue wraps).
pub fn hsv_to_rgb(hsv: [f64; 3]) -> [f64; 3] {
    let [h, s, v] = hsv;
    let c = v * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - ((hp % 2.0) - 1.0).abs());
    let (r1, g1, b1) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r1 + m, g1 + m, b1 + m]
}

/// `[h, s, l]` of an RGB color; `hint` fills in an undefined hue (gray) and saturation (black and white).
pub fn rgb_to_hsl(rgb: [f64; 3], hint: Option<(f64, f64)>) -> [f64; 3] {
    let mx = rgb[0].max(rgb[1]).max(rgb[2]);
    let mn = rgb[0].min(rgb[1]).min(rgb[2]);
    let d = mx - mn;
    let l = (mx + mn) / 2.0;
    let (hh, hs) = hint.unwrap_or((0.0, 0.0));
    let h = if d == 0.0 { hh } else { hue_of(rgb, mx, d) };
    let s = if d == 0.0 {
        if l == 0.0 || l == 1.0 {
            hs
        } else {
            0.0
        }
    } else {
        d / (1.0 - (2.0 * l - 1.0).abs())
    };
    [h, s.clamp(0.0, 1.0), l]
}

/// RGB of `[h, s, l]` (hue wraps).
pub fn hsl_to_rgb(hsl: [f64; 3]) -> [f64; 3] {
    let [h, s, l] = hsl;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - ((hp % 2.0) - 1.0).abs());
    let (r1, g1, b1) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r1 + m, g1 + m, b1 + m]
}

// MARK: Wheel geometry

/// Hue (degrees) and saturation of a point of the wheel disc. Beyond the rim the saturation is 1 (the pointer is
/// clamped to the edge). At the exact centre the hue is `hint`.
pub fn wheel_hit(x: f64, y: f64, hint: f64) -> (f64, f64) {
    let r = x.hypot(y);
    let hue = if r < 1e-9 { hint } else { (-y).atan2(x).to_degrees().rem_euclid(360.0) };
    (hue, r.min(1.0))
}

/// Where hue `hue` (degrees) at saturation `sat` sits on the wheel disc (the inverse of [`wheel_hit`]).
pub fn wheel_point(hue: f64, sat: f64) -> (f64, f64) {
    let a = hue.to_radians();
    (a.cos() * sat, -a.sin() * sat)
}

/// RGB of a point of the wheel at `value` (HSV value or HSL lightness, per the model).
pub fn wheel_color(model: WheelModel, hue: f64, sat: f64, value: f64) -> [f64; 3] {
    match model {
        WheelModel::Hsv => hsv_to_rgb([hue, sat, value]),
        WheelModel::Hsl => hsl_to_rgb([hue, sat, value]),
    }
}

/// Fills `out` (`size * size * 4` bytes, straight-alpha RGBA) with the color wheel at `value`. The disc touches the
/// bitmap's edges; its rim is anti-aliased and everything outside is transparent.
pub fn render_wheel(out: &mut [u8], size: usize, model: WheelModel, value: f64) {
    render_disc(out, size, |x, y| {
        let (h, s) = wheel_hit(x, y, 0.0);
        wheel_color(model, h, s, value)
    });
}

fn render_disc(out: &mut [u8], size: usize, color: impl Fn(f64, f64) -> [f64; 3]) {
    if size == 0 || out.len() < size * size * 4 {
        return;
    }
    let half = size as f64 / 2.0;
    for j in 0..size {
        for i in 0..size {
            let x = (i as f64 + 0.5) / half - 1.0;
            let y = (j as f64 + 0.5) / half - 1.0;
            let r = x.hypot(y);
            // One pixel of coverage across the rim.
            let coverage = ((1.0 - r) * half + 0.5).clamp(0.0, 1.0);
            let px = &mut out[(j * size + i) * 4..][..4];
            if coverage <= 0.0 {
                px.copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let rgb = color(x, y);
            px.copy_from_slice(&[byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), byte(coverage)]);
        }
    }
}

// MARK: Normal map

/// The unit tangent-space normal `(x, y, z)` at a point of the normal disc: the hemisphere seen from above, `x` to
/// the right and `y` up on screen. Points beyond the rim land on it (Z = 0).
pub fn normal_from_disc(x: f64, y: f64) -> [f64; 3] {
    let r = x.hypot(y);
    let (x, y) = if r > 1.0 { (x / r, y / r) } else { (x, y) };
    [x, -y, (1.0 - x * x - y * y).max(0.0).sqrt()]
}

/// The disc point of a normal (the inverse of [`normal_from_disc`]; the normal is normalized first).
pub fn normal_to_disc(n: [f64; 3]) -> (f64, f64) {
    let n = normal_normalized(n);
    (n[0], -n[1])
}

/// The normal scaled to unit length with Z clamped to the upper hemisphere; a zero vector becomes `(0, 0, 1)`.
pub fn normal_normalized(n: [f64; 3]) -> [f64; 3] {
    let n = [n[0], n[1], n[2].max(0.0)];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len < 1e-9 {
        [0.0, 0.0, 1.0]
    } else {
        [n[0] / len, n[1] / len, n[2] / len]
    }
}

/// The color that stores a normal: each component mapped from -1...1 to 0...1.
pub fn normal_to_rgb(n: [f64; 3]) -> [f64; 3] {
    [n[0] * 0.5 + 0.5, n[1] * 0.5 + 0.5, n[2] * 0.5 + 0.5]
}

/// The normal a color stores, normalized (any color maps to some normal; colors pointing below the surface are
/// lifted onto the horizon).
pub fn normal_from_rgb(rgb: [f64; 3]) -> [f64; 3] {
    normal_normalized([rgb[0] * 2.0 - 1.0, rgb[1] * 2.0 - 1.0, rgb[2] * 2.0 - 1.0])
}

/// The normal `n` with its Z replaced by `z` (0...1): X and Y keep their direction and shrink or grow to fit the
/// unit sphere. When X and Y are both zero the tilt goes toward +X.
pub fn normal_with_z(n: [f64; 3], z: f64) -> [f64; 3] {
    let z = z.clamp(0.0, 1.0);
    let t = (1.0 - z * z).sqrt();
    let len = n[0].hypot(n[1]);
    if len < 1e-9 {
        [t, 0.0, z]
    } else {
        [n[0] / len * t, n[1] / len * t, z]
    }
}

/// The normal `n` with its X (`axis` 0) or Y (`axis` 1) replaced by `v` (-1...1), the other kept as far as the
/// unit disc allows; Z follows.
pub fn normal_with_xy(n: [f64; 3], axis: usize, v: f64) -> [f64; 3] {
    let v = v.clamp(-1.0, 1.0);
    let (mut x, mut y) = (n[0], n[1]);
    if axis == 0 {
        x = v;
        let room = (1.0 - x * x).sqrt();
        y = y.clamp(-room, room);
    } else {
        y = v;
        let room = (1.0 - y * y).sqrt();
        x = x.clamp(-room, room);
    }
    [x, y, (1.0 - x * x - y * y).max(0.0).sqrt()]
}

/// Fills `out` (`size * size * 4` bytes, straight-alpha RGBA) with the normal-map disc: every point shows the color
/// of its normal. The rim is anti-aliased and the outside transparent.
pub fn render_normal_disc(out: &mut [u8], size: usize) {
    render_disc(out, size, |x, y| normal_to_rgb(normal_from_disc(x, y)));
}

// MARK: C ABI

unsafe fn read3(p: *const f64) -> Option<[f64; 3]> {
    (!p.is_null()).then(|| [*p, *p.add(1), *p.add(2)])
}

unsafe fn write3(p: *mut f64, v: [f64; 3]) {
    if !p.is_null() {
        for (i, x) in v.iter().enumerate() {
            *p.add(i) = *x;
        }
    }
}

unsafe fn hint2(p: *const f64) -> Option<(f64, f64)> {
    (!p.is_null()).then(|| (*p, *p.add(1)))
}

/// `[h, s, v]` of a `0xRRGGBBAA` color into `out3`. `hint` is null or `[hue, saturation]` (see [`rgb_to_hsv`]).
#[no_mangle]
pub unsafe extern "C" fn pixmith_cp_rgba_to_hsv(rgba: u32, hint: *const f64, out3: *mut f64) {
    crate::ffi::guarded("pixmith_cp_rgba_to_hsv", std::ptr::null_mut(), || {
        write3(out3, rgb_to_hsv(unpack(rgba).0, hint2(hint)));
    })
}

/// `0xRRGGBBAA` of hue (degrees), saturation, value and alpha (0...1).
#[no_mangle]
pub extern "C" fn pixmith_cp_hsv_to_rgba(h: f64, s: f64, v: f64, a: f64) -> u32 {
    crate::ffi::guarded("pixmith_cp_hsv_to_rgba", std::ptr::null_mut(), || pack(hsv_to_rgb([h, s, v]), a))
}

/// `[h, s, l]` of a `0xRRGGBBAA` color into `out3`. `hint` is null or `[hue, saturation]`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cp_rgba_to_hsl(rgba: u32, hint: *const f64, out3: *mut f64) {
    crate::ffi::guarded("pixmith_cp_rgba_to_hsl", std::ptr::null_mut(), || {
        write3(out3, rgb_to_hsl(unpack(rgba).0, hint2(hint)));
    })
}

/// `0xRRGGBBAA` of hue (degrees), saturation, lightness and alpha (0...1).
#[no_mangle]
pub extern "C" fn pixmith_cp_hsl_to_rgba(h: f64, s: f64, l: f64, a: f64) -> u32 {
    crate::ffi::guarded("pixmith_cp_hsl_to_rgba", std::ptr::null_mut(), || pack(hsl_to_rgb([h, s, l]), a))
}

/// Rec. 601 luma of a color, 0...1.
#[no_mangle]
pub extern "C" fn pixmith_cp_luma(rgba: u32) -> f64 {
    crate::ffi::guarded("pixmith_cp_luma", std::ptr::null_mut(), || luma(unpack(rgba).0))
}

/// Hue (degrees) and saturation of a wheel-disc point into `out2`; `hint_hue` is used at the exact centre.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cp_wheel_hit(x: f64, y: f64, hint_hue: f64, out2: *mut f64) {
    crate::ffi::guarded("pixmith_cp_wheel_hit", std::ptr::null_mut(), || {
        let (h, s) = wheel_hit(x, y, hint_hue);
        if !out2.is_null() {
            *out2 = h;
            *out2.add(1) = s;
        }
    })
}

/// The wheel-disc point `(x, y)` of a hue and saturation into `out2`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cp_wheel_point(hue: f64, sat: f64, out2: *mut f64) {
    crate::ffi::guarded("pixmith_cp_wheel_point", std::ptr::null_mut(), || {
        let (x, y) = wheel_point(hue, sat);
        if !out2.is_null() {
            *out2 = x;
            *out2.add(1) = y;
        }
    })
}

/// Writes the hues of `harmony` (0 none, 1 complementary, 2 triadic, 3 analogous, 4 split complementary, 5 square)
/// around `hue` into `out` (room for 4) and returns how many.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cp_harmony(hue: f64, harmony: u32, out: *mut f64) -> u32 {
    crate::ffi::guarded("pixmith_cp_harmony", std::ptr::null_mut(), || {
        let hues = harmony_hues(hue, Harmony::from_u32(harmony));
        if !out.is_null() {
            for (i, h) in hues.iter().enumerate() {
                *out.add(i) = *h;
            }
        }
        hues.len() as u32
    })
}

/// Renders the wheel (`model` 0 = HSV, 1 = HSL) into `out`, `size * size * 4` bytes of straight-alpha RGBA.
/// Returns false when the buffer is null or `size` is 0 or above 4096.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cp_wheel_render(out: *mut u8, size: u32, model: u32, value: f64) -> bool {
    crate::ffi::guarded("pixmith_cp_wheel_render", std::ptr::null_mut(), || {
        if out.is_null() || size == 0 || size > 4096 {
            return false;
        }
        let n = size as usize;
        render_wheel(std::slice::from_raw_parts_mut(out, n * n * 4), n, WheelModel::from_u32(model), value);
        true
    })
}

/// Renders the normal-map disc into `out` (`size * size * 4` bytes). Same rules as `pixmith_cp_wheel_render`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cp_normal_render(out: *mut u8, size: u32) -> bool {
    crate::ffi::guarded("pixmith_cp_normal_render", std::ptr::null_mut(), || {
        if out.is_null() || size == 0 || size > 4096 {
            return false;
        }
        let n = size as usize;
        render_normal_disc(std::slice::from_raw_parts_mut(out, n * n * 4), n);
        true
    })
}

/// The normal `(x, y, z)` at a normal-disc point into `out3`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cp_normal_from_disc(x: f64, y: f64, out3: *mut f64) {
    crate::ffi::guarded("pixmith_cp_normal_from_disc", std::ptr::null_mut(), || write3(out3, normal_from_disc(x, y)))
}

/// The disc point `(x, y)` of the normal stored in a color into `out2`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cp_normal_disc_point(rgba: u32, out2: *mut f64) {
    crate::ffi::guarded("pixmith_cp_normal_disc_point", std::ptr::null_mut(), || {
        let (x, y) = normal_to_disc(normal_from_rgb(unpack(rgba).0));
        if !out2.is_null() {
            *out2 = x;
            *out2.add(1) = y;
        }
    })
}

/// The normal stored in a color into `out3` (normalized, lifted onto the upper hemisphere).
#[no_mangle]
pub unsafe extern "C" fn pixmith_cp_normal_decode(rgba: u32, out3: *mut f64) {
    crate::ffi::guarded("pixmith_cp_normal_decode", std::ptr::null_mut(), || {
        write3(out3, normal_from_rgb(unpack(rgba).0))
    })
}

/// The color storing the normal `n3` (normalized first), with alpha `a` (0...1).
#[no_mangle]
pub unsafe extern "C" fn pixmith_cp_normal_encode(n3: *const f64, a: f64) -> u32 {
    crate::ffi::guarded("pixmith_cp_normal_encode", std::ptr::null_mut(), || {
        pack(normal_to_rgb(normal_normalized(read3(n3).unwrap_or([0.0, 0.0, 1.0]))), a)
    })
}

/// The normal in `n3` with X (`axis` 0), Y (`axis` 1) or Z (`axis` 2) replaced by `v`, written back to `n3`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_cp_normal_set(n3: *mut f64, axis: u32, v: f64) {
    crate::ffi::guarded("pixmith_cp_normal_set", std::ptr::null_mut(), || {
        let Some(n) = read3(n3) else { return };
        let n = normal_normalized(n);
        write3(n3, if axis == 2 { normal_with_z(n, v) } else { normal_with_xy(n, axis as usize, v) });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn rgb8(r: u8, g: u8, b: u8) -> [f64; 3] {
        [r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0]
    }

    fn grid() -> impl Iterator<Item = (u8, u8, u8)> {
        let steps: Vec<u8> = (0..=255u16).step_by(3).map(|v| v as u8).chain([1, 2, 254, 255]).collect();
        steps.clone().into_iter().flat_map(move |r| {
            let steps = steps.clone();
            steps.clone().into_iter().flat_map(move |g| steps.clone().into_iter().map(move |b| (r, g, b)))
        })
    }

    #[test]
    fn hsv_roundtrips_every_sampled_byte_color() {
        for (r, g, b) in grid() {
            let c = rgb8(r, g, b);
            let back = pack(hsv_to_rgb(rgb_to_hsv(c, None)), 1.0);
            assert_eq!(back, pack(c, 1.0), "hsv {r},{g},{b}");
        }
    }

    #[test]
    fn hsl_roundtrips_every_sampled_byte_color() {
        for (r, g, b) in grid() {
            let c = rgb8(r, g, b);
            let back = pack(hsl_to_rgb(rgb_to_hsl(c, None)), 1.0);
            assert_eq!(back, pack(c, 1.0), "hsl {r},{g},{b}");
        }
    }

    #[test]
    fn known_conversions() {
        let hsv = rgb_to_hsv(rgb8(0xF2, 0xA6, 0x5A), None);
        assert_eq!((hsv[0].round(), (hsv[1] * 100.0).round(), (hsv[2] * 100.0).round()), (30.0, 63.0, 95.0));
        let hsl = rgb_to_hsl(rgb8(0xF2, 0xA6, 0x5A), None);
        assert_eq!((hsl[0].round(), (hsl[1] * 100.0).round(), (hsl[2] * 100.0).round()), (30.0, 85.0, 65.0));
        assert_eq!(pack(hsv_to_rgb([120.0, 1.0, 1.0]), 1.0), 0x00FF00FF);
        assert_eq!(pack(hsl_to_rgb([240.0, 1.0, 0.5]), 0.5), 0x0000FF80);
        assert_eq!(pack(hsl_to_rgb([0.0, 1.0, 1.0]), 1.0), 0xFFFFFFFF);
        assert_eq!(pack(hsv_to_rgb([360.0, 1.0, 1.0]), 1.0), 0xFF0000FF, "hue 360 wraps to red");
        assert_eq!(pack(hsv_to_rgb([-120.0, 1.0, 1.0]), 1.0), 0x0000FFFF, "negative hue wraps");
    }

    #[test]
    fn undefined_components_follow_the_hint() {
        // Gray has no hue; black has no saturation (HSV), black and white have none (HSL).
        assert_eq!(rgb_to_hsv(rgb8(128, 128, 128), Some((200.0, 0.7))), [200.0, 0.0, 128.0 / 255.0]);
        assert_eq!(rgb_to_hsv([0.0; 3], Some((200.0, 0.7))), [200.0, 0.7, 0.0]);
        assert_eq!(rgb_to_hsl([1.0; 3], Some((10.0, 0.4))), [10.0, 0.4, 1.0]);
        assert_eq!(rgb_to_hsl(rgb8(128, 128, 128), Some((10.0, 0.4)))[1], 0.0);
        assert_eq!(rgb_to_hsv([0.0; 3], None), [0.0; 3]);
    }

    #[test]
    fn luma_matches_rec601() {
        assert!(close(luma([1.0; 3]), 1.0));
        assert!(close(luma([1.0, 0.0, 0.0]), 0.299));
        assert!(close(luma(unpack(0x808080FF).0), 128.0 / 255.0));
    }

    #[test]
    fn wheel_geometry_roundtrips() {
        for hue in [0.0, 30.0, 90.0, 180.0, 271.5, 359.0] {
            for sat in [0.1, 0.5, 1.0] {
                let (x, y) = wheel_point(hue, sat);
                let (h, s) = wheel_hit(x, y, 0.0);
                assert!(close(h, hue) && close(s, sat), "{hue} {sat} -> {h} {s}");
            }
        }
    }

    #[test]
    fn wheel_orientation() {
        // Red at the east, yellow up and to the right (counter-clockwise), cyan at the west.
        assert_eq!(wheel_hit(1.0, 0.0, 0.0), (0.0, 1.0));
        assert!(close(wheel_hit(0.0, -0.5, 0.0).0, 90.0), "up on screen is hue 90");
        assert!(close(wheel_hit(-1.0, 0.0, 0.0).0, 180.0));
        assert!(close(wheel_hit(0.0, 1.0, 0.0).0, 270.0));
    }

    #[test]
    fn wheel_clamps_to_the_rim_and_keeps_the_hint_at_the_center() {
        let (h, s) = wheel_hit(3.0, 0.0, 0.0);
        assert_eq!((h, s), (0.0, 1.0));
        assert_eq!(wheel_hit(0.0, 0.0, 77.0), (77.0, 0.0));
    }

    #[test]
    fn harmonies() {
        assert!(harmony_hues(10.0, Harmony::None).is_empty());
        assert_eq!(harmony_hues(10.0, Harmony::Complementary), vec![190.0]);
        assert_eq!(harmony_hues(10.0, Harmony::Triadic), vec![130.0, 250.0]);
        assert_eq!(harmony_hues(10.0, Harmony::Analogous), vec![340.0, 40.0]);
        assert_eq!(harmony_hues(350.0, Harmony::SplitComplementary), vec![140.0, 200.0]);
        assert_eq!(harmony_hues(0.0, Harmony::Square), vec![90.0, 180.0, 270.0]);
        assert_eq!(Harmony::from_u32(99), Harmony::None);
    }

    #[test]
    fn rendered_wheel_matches_the_math() {
        let n = 64;
        let mut buf = vec![0u8; n * n * 4];
        render_wheel(&mut buf, n, WheelModel::Hsv, 1.0);
        fn at(buf: &[u8], n: usize, x: usize, y: usize) -> [u8; 4] {
            buf[(y * n + x) * 4..][..4].try_into().unwrap()
        }
        let px = |buf: &[u8], x: usize, y: usize| at(buf, n, x, y);
        assert_eq!(px(&buf, 0, 0)[3], 0, "outside the disc is transparent");
        assert_eq!(px(&buf, n - 1, n - 1)[3], 0);
        let east = px(&buf, n - 2, n / 2);
        assert!(east[0] > 240 && east[1] < 40 && east[3] == 255, "red at the east: {east:?}");
        let west = px(&buf, 1, n / 2);
        assert!(west[0] < 40 && west[1] > 220 && west[2] > 220, "cyan at the west: {west:?}");
        let center = px(&buf, n / 2, n / 2);
        assert!(center[0] > 245 && center[1] > 245 && center[2] > 245, "white at the center at V=1: {center:?}");
        // The HSL wheel at L=0.5 has a pure red rim and a mid-gray center.
        render_wheel(&mut buf, n, WheelModel::Hsl, 0.5);
        let c = px(&buf, n / 2, n / 2);
        assert!(c[..3].iter().all(|v| (118..=138).contains(v)), "gray center: {c:?}");
        // Value 0 is black everywhere inside.
        render_wheel(&mut buf, n, WheelModel::Hsv, 0.0);
        assert_eq!(&px(&buf, n - 2, n / 2)[..3], &[0, 0, 0]);
    }

    #[test]
    fn normal_disc_geometry() {
        let n = normal_from_disc(0.0, 0.0);
        assert_eq!(n, [0.0, 0.0, 1.0]);
        assert_eq!(pack(normal_to_rgb(n), 1.0), 0x8080FFFF, "the flat normal is #8080FF");
        let right = normal_from_disc(1.0, 0.0);
        assert!(close(right[0], 1.0) && close(right[2], 0.0));
        let up = normal_from_disc(0.0, -0.6); // up on screen is +Y
        assert!(close(up[1], 0.6) && close(up[2], 0.8));
        let beyond = normal_from_disc(5.0, 0.0);
        assert!(close(beyond[0], 1.0) && close(beyond[2], 0.0), "clamped to the horizon");
        for (x, y) in [(0.3, -0.2), (-0.7, 0.1), (0.0, 0.9)] {
            let n = normal_from_disc(x, y);
            assert!(close(n.iter().map(|v| v * v).sum::<f64>(), 1.0), "unit length");
            let (dx, dy) = normal_to_disc(n);
            assert!(close(dx, x) && close(dy, y));
        }
    }

    #[test]
    fn normal_encoding_roundtrips_through_a_byte_color() {
        for (x, y) in [(0.0, 0.0), (0.5, -0.5), (-0.8, 0.3), (0.0, -1.0)] {
            let n = normal_from_disc(x, y);
            let rgba = pack(normal_to_rgb(n), 0.5);
            assert_eq!(rgba & 0xFF, 0x80);
            let back = normal_from_rgb(unpack(rgba).0);
            for i in 0..3 {
                assert!((back[i] - n[i]).abs() < 0.012, "{x},{y} comp {i}: {back:?} vs {n:?}");
            }
        }
        // Any color decodes to a valid unit normal; a color below the surface is lifted to the horizon.
        let n = normal_from_rgb(rgb8(255, 128, 0));
        assert!(close(n.iter().map(|v| v * v).sum::<f64>(), 1.0) && n[2] == 0.0);
    }

    #[test]
    fn editing_one_normal_component_keeps_the_rest_valid() {
        let n = normal_from_disc(0.3, -0.4);
        let z = normal_with_z(n, 0.5);
        assert!(close(z[2], 0.5) && close(z.iter().map(|v| v * v).sum::<f64>(), 1.0));
        assert!(z[0] > 0.0 && z[1] > 0.0, "direction kept");
        assert!(close(z[1] / z[0], n[1] / n[0]));
        let flat = normal_with_z([0.0, 0.0, 1.0], 0.0);
        assert!(close(flat[0], 1.0) && close(flat[2], 0.0), "a flat normal tilts toward +X");
        let x = normal_with_xy([0.0, 0.9, 0.436], 0, 0.8);
        assert!(close(x[0], 0.8) && close(x[1], 0.6) && close(x[2], 0.0), "Y shrinks to stay on the sphere: {x:?}");
        let y = normal_with_xy([0.5, 0.0, 0.866], 1, -0.25);
        assert!(close(y[1], -0.25) && close(y[0], 0.5) && close(y.iter().map(|v| v * v).sum::<f64>(), 1.0));
    }

    #[test]
    fn rendered_normal_disc_shows_the_normals() {
        let n = 64;
        let mut buf = vec![0u8; n * n * 4];
        render_normal_disc(&mut buf, n);
        let px = |x: usize, y: usize| -> [u8; 4] { buf[(y * n + x) * 4..][..4].try_into().unwrap() };
        assert_eq!(px(0, 0)[3], 0);
        let c = px(n / 2, n / 2);
        assert!(c[0].abs_diff(128) < 6 && c[1].abs_diff(128) < 6 && c[2] > 250, "flat blue at the center: {c:?}");
        let east = px(n - 2, n / 2);
        assert!(east[0] > 240 && east[1].abs_diff(128) < 12, "red-ish at the east edge: {east:?}");
        let north = px(n / 2, 1);
        assert!(north[1] > 240, "+Y (green) at the top: {north:?}");
    }

    #[test]
    fn render_ignores_bad_buffers() {
        let mut small = [0u8; 8];
        render_wheel(&mut small, 64, WheelModel::Hsv, 1.0);
        render_normal_disc(&mut small, 0);
        assert_eq!(small, [0u8; 8]);
    }

    #[test]
    fn c_abi_roundtrip() {
        unsafe {
            let mut hsv = [0.0; 3];
            pixmith_cp_rgba_to_hsv(0xF2A65AFF, std::ptr::null(), hsv.as_mut_ptr());
            assert_eq!(hsv[0].round(), 30.0);
            assert_eq!(pixmith_cp_hsv_to_rgba(hsv[0], hsv[1], hsv[2], 1.0), 0xF2A65AFF);
            let hint = [200.0, 0.5];
            pixmith_cp_rgba_to_hsl(0x808080FF, hint.as_ptr(), hsv.as_mut_ptr());
            assert_eq!((hsv[0], hsv[1]), (200.0, 0.0));
            assert_eq!(pixmith_cp_hsl_to_rgba(hsv[0], 1.0, 0.5, 1.0), 0x00AAFFFF);
            pixmith_cp_rgba_to_hsv(1, std::ptr::null(), std::ptr::null_mut());

            let mut p = [0.0; 2];
            pixmith_cp_wheel_point(90.0, 0.5, p.as_mut_ptr());
            let mut hs = [0.0; 2];
            pixmith_cp_wheel_hit(p[0], p[1], 0.0, hs.as_mut_ptr());
            assert!(close(hs[0], 90.0) && close(hs[1], 0.5));

            let mut hues = [0.0; 4];
            assert_eq!(pixmith_cp_harmony(10.0, 2, hues.as_mut_ptr()), 2);
            assert_eq!(&hues[..2], &[130.0, 250.0]);
            assert_eq!(pixmith_cp_harmony(10.0, 0, std::ptr::null_mut()), 0);

            let mut buf = vec![0u8; 16 * 16 * 4];
            assert!(pixmith_cp_wheel_render(buf.as_mut_ptr(), 16, 0, 1.0));
            assert!(buf.iter().any(|b| *b != 0));
            assert!(!pixmith_cp_wheel_render(std::ptr::null_mut(), 16, 0, 1.0));
            assert!(!pixmith_cp_wheel_render(buf.as_mut_ptr(), 0, 0, 1.0));
            assert!(pixmith_cp_normal_render(buf.as_mut_ptr(), 16));
            assert!(!pixmith_cp_normal_render(buf.as_mut_ptr(), 100_000));

            let mut n = [0.0; 3];
            pixmith_cp_normal_from_disc(0.0, 0.0, n.as_mut_ptr());
            let rgba = pixmith_cp_normal_encode(n.as_ptr(), 1.0);
            assert_eq!(rgba, 0x8080FFFF);
            pixmith_cp_normal_decode(rgba, n.as_mut_ptr());
            assert!(n[2] > 0.99);
            n = [0.0, 0.0, 1.0];
            pixmith_cp_normal_set(n.as_mut_ptr(), 2, 0.0);
            assert!(close(n[0], 1.0) && close(n[2], 0.0));
            let mut d = [9.0; 2];
            pixmith_cp_normal_disc_point(0x8080FFFF, d.as_mut_ptr());
            assert!(d[0].abs() < 0.01 && d[1].abs() < 0.01);
            assert_eq!(pixmith_cp_normal_encode(std::ptr::null(), 1.0), 0x8080FFFF);
            assert!(close(pixmith_cp_luma(0xFFFFFFFF), 1.0));
        }
    }
}
