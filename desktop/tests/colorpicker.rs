//! The Color section's selectors (`ColorSelectors.swift`): switching through the header menu, and driving every
//! selector with synthetic input. The screenshot test at the bottom needs a GPU and is ignored by default:
//!
//! ```sh
//! cargo test --test colorpicker -- --ignored     # writes target/screens/color-<selector>-<theme>.png
//! ```

mod common;

use eframe::egui::{self, Pos2, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pixmith_core::colorpick::{self, Harmony, WheelModel};
use pixmith_desktop::PixmithApp;
use pixmith_desktop::ui::inspector::selectors::Selector;

type H = Harness<'static, PixmithApp>;

fn open(selector: Selector, primary: u32) -> H {
    let mut app = PixmithApp { primary, ..Default::default() };
    app.inspector.picker.selector = selector;
    common::harness(app)
}

fn press(h: &mut H, pos: Pos2) {
    h.hover_at(pos);
    h.run();
    h.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    h.run();
}

fn release(h: &mut H, pos: Pos2) {
    h.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    h.run();
}

fn click(h: &mut H, pos: Pos2) {
    press(h, pos);
    release(h, pos);
}

fn rect_of(h: &H, label: &str) -> Rect {
    h.get_by_label(label).rect()
}

/// The point of a horizontal track (thumb travel 7 pt in from each end) for `f` in 0...1.
fn on_track(r: Rect, f: f32) -> Pos2 {
    Pos2::new(r.left() + 7.0 + (r.width() - 14.0) * f, r.center().y)
}

/// Clicks the track named `label` at `f` and releases.
fn slide(h: &mut H, label: &str, f: f32) {
    let r = rect_of(h, label);
    click(h, on_track(r, f));
}

fn disc_point(r: Rect, x: f64, y: f64) -> Pos2 {
    let half = r.width() / 2.0;
    r.center() + egui::vec2(x as f32 * half, y as f32 * half)
}

#[test]
fn the_header_menu_switches_every_selector() {
    let mut h = open(Selector::Spectrum, 0xF2A65AFF);
    assert!(h.query_by_label("Saturation and value").is_some(), "Spectrum is the default");
    for (selector, marker) in [
        (Selector::Wheel, "Color wheel"),
        (Selector::Rgb, "Red"),
        (Selector::Hsv, "Saturation"),
        (Selector::Hsl, "Lightness"),
        (Selector::Gray, "Gray level (luma)"),
        (Selector::Normal, "Normal map disc"),
        (Selector::Spectrum, "Saturation and value"),
    ] {
        h.get_by_label("Color selector").click();
        h.run();
        h.get_by_label(selector.title()).click();
        h.run();
        assert_eq!(h.state().inspector.picker.selector, selector);
        assert!(h.query_by_label(marker).is_some(), "{} should show '{marker}'", selector.title());
    }
}

#[test]
fn every_selector_has_alpha() {
    for selector in Selector::ALL {
        let h = open(selector, 0xF2A65AFF);
        assert!(h.query_by_label("Alpha").is_some(), "{} has no alpha control", selector.title());
        assert_eq!(h.state().primary, 0xF2A65AFF, "{} must not touch the color", selector.title());
    }
}

#[test]
fn alpha_slider_edits_alpha_only() {
    for selector in Selector::ALL {
        let mut h = open(selector, 0x336699FF);
        slide(&mut h, "Alpha", 0.0);
        assert_eq!(h.state().primary, 0x33669900, "{}", selector.title());
        slide(&mut h, "Alpha", 1.0);
        assert_eq!(h.state().primary, 0x336699FF, "{}", selector.title());
    }
}

#[test]
fn rgb_sliders_set_their_channel() {
    let mut h = open(Selector::Rgb, 0x336699FF);
    slide(&mut h, "Red", 1.0);
    assert_eq!(h.state().primary, 0xFF6699FF);
    slide(&mut h, "Green", 0.0);
    assert_eq!(h.state().primary, 0xFF0099FF);
    slide(&mut h, "Blue", 1.0);
    assert_eq!(h.state().primary, 0xFF00FFFF);
}

#[test]
fn hsv_sliders_set_their_channel() {
    let mut h = open(Selector::Hsv, 0xFF0000FF);
    slide(&mut h, "Hue", 1.0 / 3.0);
    let hsv = colorpick::rgb_to_hsv(colorpick::unpack(h.state().primary).0, None);
    assert!((hsv[0] - 120.0).abs() < 2.0 && hsv[1] > 0.99 && hsv[2] > 0.99, "{hsv:?}");
    slide(&mut h, "Value", 0.5);
    let hsv = colorpick::rgb_to_hsv(colorpick::unpack(h.state().primary).0, None);
    assert!((hsv[2] - 0.5).abs() < 0.03 && (hsv[0] - 120.0).abs() < 2.0, "{hsv:?}");
    slide(&mut h, "Saturation", 0.0);
    let primary = h.state().primary;
    let [r, g, b, _] = primary.to_be_bytes();
    assert!(r == g && g == b, "no saturation is gray: {primary:08X}");
    // The hue survives the trip through gray.
    slide(&mut h, "Saturation", 1.0);
    let hsv = colorpick::rgb_to_hsv(colorpick::unpack(h.state().primary).0, None);
    assert!((hsv[0] - 120.0).abs() < 2.0, "{hsv:?}");
}

#[test]
fn hsl_sliders_set_their_channel() {
    let mut h = open(Selector::Hsl, 0xFF0000FF);
    slide(&mut h, "Lightness", 1.0);
    assert_eq!(h.state().primary, 0xFFFFFFFF);
    slide(&mut h, "Lightness", 0.5);
    let hsl = colorpick::rgb_to_hsl(colorpick::unpack(h.state().primary).0, None);
    assert!((hsl[2] - 0.5).abs() < 0.03 && hsl[1] > 0.95, "{hsl:?}");
    slide(&mut h, "Saturation", 0.0);
    let [r, g, b, _] = h.state().primary.to_be_bytes();
    assert!(r == g && g == b);
    slide(&mut h, "Hue", 0.5);
    slide(&mut h, "Saturation", 1.0);
    let hsl = colorpick::rgb_to_hsl(colorpick::unpack(h.state().primary).0, None);
    assert!((hsl[0] - 180.0).abs() < 3.0, "{hsl:?}");
}

#[test]
fn gray_slider_sets_a_gray() {
    let mut h = open(Selector::Gray, 0x336699FF);
    slide(&mut h, "Gray level (luma)", 0.0);
    assert_eq!(h.state().primary, 0x000000FF);
    slide(&mut h, "Gray level (luma)", 1.0);
    assert_eq!(h.state().primary, 0xFFFFFFFF);
    slide(&mut h, "Gray level (luma)", 0.5);
    let [r, g, b, a] = h.state().primary.to_be_bytes();
    assert!(r == g && g == b && (120..=135).contains(&r) && a == 255);
}

#[test]
fn wheel_picks_hue_and_saturation_and_the_bar_picks_value() {
    let mut h = open(Selector::Wheel, 0x808080FF);
    let wheel = rect_of(&h, "Color wheel");
    // The east edge is red at full saturation; the gray's value (50%) is kept.
    click(&mut h, disc_point(wheel, 0.98, 0.0));
    let hsv = colorpick::rgb_to_hsv(colorpick::unpack(h.state().primary).0, None);
    assert!(hsv[0] < 3.0 || hsv[0] > 357.0, "red at the east: {hsv:?}");
    assert!(hsv[1] > 0.95 && (hsv[2] - 0.5).abs() < 0.02, "{hsv:?}");
    // Straight up on screen is hue 90, halfway out is half saturation.
    click(&mut h, disc_point(wheel, 0.0, -0.5));
    let hsv = colorpick::rgb_to_hsv(colorpick::unpack(h.state().primary).0, None);
    assert!((hsv[0] - 90.0).abs() < 3.0 && (hsv[1] - 0.5).abs() < 0.03, "{hsv:?}");
    // Dragging past the rim clamps to the rim.
    press(&mut h, disc_point(wheel, 0.0, 0.5));
    h.hover_at(disc_point(wheel, 0.0, 1.5));
    h.run();
    release(&mut h, disc_point(wheel, 0.0, 1.5));
    let hsv = colorpick::rgb_to_hsv(colorpick::unpack(h.state().primary).0, None);
    assert!((hsv[0] - 270.0).abs() < 3.0 && hsv[1] > 0.99, "{hsv:?}");
    // The bar: bottom is black, top is full value.
    let bar = rect_of(&h, "Value");
    click(&mut h, Pos2::new(bar.center().x, bar.bottom() - 1.0));
    assert_eq!(h.state().primary, 0x000000FF);
    click(&mut h, Pos2::new(bar.center().x, bar.top() + 1.0));
    let hsv = colorpick::rgb_to_hsv(colorpick::unpack(h.state().primary).0, None);
    assert!(hsv[2] > 0.98 && (hsv[0] - 270.0).abs() < 3.0, "the hue survives black: {hsv:?}");
}

#[test]
fn hsl_wheel_uses_lightness() {
    let mut app = PixmithApp { primary: 0xFF0000FF, ..Default::default() };
    app.inspector.picker.selector = Selector::Wheel;
    app.inspector.picker.wheel_model = WheelModel::Hsl;
    app.inspector.picker.harmony = Harmony::Triadic;
    let mut h = common::harness(app);
    let bar = rect_of(&h, "Lightness");
    click(&mut h, Pos2::new(bar.center().x, bar.top() + 1.0));
    assert_eq!(h.state().primary, 0xFFFFFFFF, "lightness 1 is white");
    let wheel = rect_of(&h, "Color wheel");
    click(&mut h, disc_point(wheel, 0.98, 0.0));
    click(&mut h, Pos2::new(bar.center().x, bar.center().y));
    let hsl = colorpick::rgb_to_hsl(colorpick::unpack(h.state().primary).0, None);
    assert!((hsl[2] - 0.5).abs() < 0.05 && hsl[1] > 0.9, "mid lightness at the rim is a pure color: {hsl:?}");
}

#[test]
fn normal_disc_picks_tangent_space_normals() {
    let mut h = open(Selector::Normal, 0x336699FF);
    let disc = rect_of(&h, "Normal map disc");
    click(&mut h, disc_point(disc, 0.0, 0.0));
    let [r, g, b, a] = h.state().primary.to_be_bytes();
    assert!(r.abs_diff(128) <= 2 && g.abs_diff(128) <= 2 && b >= 253 && a == 255, "flat: {:08X}", h.state().primary);
    // Right: +X. Up on the screen: +Y (green high).
    click(&mut h, disc_point(disc, 0.9, 0.0));
    let n = colorpick::normal_from_rgb(colorpick::unpack(h.state().primary).0);
    assert!((n[0] - 0.9).abs() < 0.03 && n[1].abs() < 0.03, "{n:?}");
    click(&mut h, disc_point(disc, 0.0, -0.6));
    let n = colorpick::normal_from_rgb(colorpick::unpack(h.state().primary).0);
    assert!(n[0].abs() < 0.03 && (n[1] - 0.6).abs() < 0.03 && (n[2] - 0.8).abs() < 0.03, "{n:?}");
    // Past the rim: the horizon.
    press(&mut h, disc_point(disc, -0.9, 0.0));
    h.hover_at(disc_point(disc, -1.4, 0.0));
    h.run();
    release(&mut h, disc_point(disc, -1.4, 0.0));
    let n = colorpick::normal_from_rgb(colorpick::unpack(h.state().primary).0);
    assert!(n[0] < -0.98 && n[2] < 0.1, "{n:?}");
    // The flat button.
    h.get_by_label("Flat").click();
    h.run();
    let [r, g, b, _] = h.state().primary.to_be_bytes();
    assert!(r.abs_diff(128) <= 2 && g.abs_diff(128) <= 2 && b >= 253);
}

#[test]
fn indexed_mode_snaps_the_primary_but_not_the_secondary() {
    let mut app = PixmithApp { primary: 0x000000FF, ..Default::default() };
    app.inspector.picker.selector = Selector::Rgb;
    assert!(app.tab_mut().doc.convert_color_mode(2).is_ok());
    let palette = app.tab().doc.palette();
    assert!(!palette.is_empty());
    let mut h = common::harness(app);
    slide(&mut h, "Red", 1.0);
    slide(&mut h, "Green", 0.3);
    assert!(h.state().tab().doc.palette().contains(&h.state().primary), "{:08X} is a palette color", h.state().primary);
    // The secondary color is edited as typed.
    h.state_mut().inspector.secondary = true;
    h.state_mut().secondary = 0x123456FF;
    h.run();
    slide(&mut h, "Blue", 1.0);
    assert_eq!(h.state().secondary, 0x1234FFFF);
}

#[test]
fn the_selector_choice_is_stored() {
    let mut app = PixmithApp::default();
    app.inspector.picker.selector = Selector::Normal;
    app.inspector.picker.wheel_model = WheelModel::Hsl;
    app.inspector.picker.harmony = Harmony::Square;
    let text = app.inspector.picker.encode();
    let mut restored = PixmithApp::default();
    restored.inspector.picker.apply_text(&text);
    assert_eq!(restored.inspector.picker.selector, Selector::Normal);
    assert_eq!(restored.inspector.picker.wheel_model, WheelModel::Hsl);
    assert_eq!(restored.inspector.picker.harmony, Harmony::Square);
    restored.inspector.picker.apply_text("selector=bogus\nharmony=99\nnonsense");
    assert_eq!(restored.inspector.picker.selector, Selector::Normal, "unknown values keep the current one");
    assert_eq!(restored.inspector.picker.harmony, Harmony::None);
}

/// Every selector in both themes at 2x.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn screenshots_of_every_selector() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/screens");
    std::fs::create_dir_all(&dir).unwrap();
    for dark in [true, false] {
        for selector in Selector::ALL {
            for (suffix, harmony) in [("", Harmony::None), ("-harmony", Harmony::Triadic)] {
                if suffix == "-harmony" && selector != Selector::Wheel {
                    continue;
                }
                let color = if selector == Selector::Normal { 0xA05CF0C0 } else { 0xF2A65AC0 };
                let mut app = PixmithApp { dark, primary: color, ..Default::default() };
                app.inspector.picker.selector = selector;
                app.inspector.picker.harmony = harmony;
                let mut h = common::harness_sized(app, [1280.0, 800.0], 2.0);
                let image = h.render().expect("render");
                let name = format!("color-{}{suffix}-{}.png", selector.key(), if dark { "dark" } else { "light" });
                image.save(dir.join(&name)).unwrap();
                println!("wrote {name}");
            }
        }
    }
    // The HSL wheel.
    for dark in [true, false] {
        let mut app = PixmithApp { dark, primary: 0x4F8FE0FF, ..Default::default() };
        app.inspector.picker.selector = Selector::Wheel;
        app.inspector.picker.wheel_model = WheelModel::Hsl;
        app.inspector.picker.harmony = Harmony::Analogous;
        let mut h = common::harness_sized(app, [1280.0, 800.0], 2.0);
        let image = h.render().expect("render");
        image.save(dir.join(format!("color-wheel-hsl-{}.png", if dark { "dark" } else { "light" }))).unwrap();
    }
}
