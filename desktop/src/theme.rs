//! Design tokens, one-to-one with `app/Sources/Pixmith/DesignSystem/DesignSystem.swift`.
//!
//! Keep the values identical to the Swift ones so both front ends look the same; when one changes, change
//! the other. Swift's points map to egui's points (logical pixels).

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke};
use std::sync::Arc;

/// Spacing on a 4/8 pt grid (`DS.Space`).
pub mod space {
    pub const XXS: f32 = 2.0;
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 24.0;
}

/// Corner radii (`DS.Radius`).
pub mod radius {
    pub const CHIP: u8 = 4;
    /// Inputs, segmented controls and popups.
    pub const FIELD: u8 = 5;
    /// Icon buttons and other controls.
    pub const CONTROL: u8 = 6;
    pub const PANEL: u8 = 8;
}

/// Fixed sizes (`DS.Metric`).
pub mod metric {
    /// Height of fields, popups and small icon buttons.
    pub const CONTROL: f32 = 24.0;
    /// Standard icon button and tool button hit target.
    pub const ICON_BUTTON: f32 = 28.0;
    pub const TOOL_BUTTON: f32 = 28.0;
    pub const TOOL_RAIL: f32 = 48.0;
    pub const INSPECTOR_WIDTH: f32 = 272.0;
    /// Title bar and context bar.
    pub const BAR_HEIGHT: f32 = 40.0;
    pub const TIMELINE_HEIGHT: f32 = 252.0;
    pub const TIMELINE_HEADER: f32 = 36.0;
    pub const TIMELINE_ROW: f32 = 24.0;
    pub const SECTION_HEADER: f32 = 36.0;
    /// Horizontal inset of inspector content and section titles.
    pub const INSET: f32 = 16.0;
}

/// Font sizes (`DS.Font`). Dense labels are 11 pt, body 12-13 pt.
pub mod font {
    pub const CAPTION: f32 = 11.0;
    pub const BODY: f32 = 12.0;
    pub const BODY_LARGE: f32 = 13.0;
    pub const TITLE: f32 = 13.0;
}

fn rgba(v: u32) -> Color32 {
    Color32::from_rgba_unmultiplied((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Semantic colors (`DSColor`), resolved for light or dark.
#[derive(Clone, Copy, Debug)]
pub struct Colors {
    /// The area around the sprite.
    pub workspace: Color32,
    /// Background of the bars, the tool rail, the inspector and the timeline.
    pub panel: Color32,
    /// A step lighter than the panel: the active document tab.
    pub panel2: Color32,
    /// Slightly recessed areas inside a panel.
    pub recessed: Color32,
    /// Popovers and menus.
    pub elevated: Color32,
    /// Lines between regions.
    pub separator: Color32,
    /// Quieter lines between sections and rows.
    pub separator_soft: Color32,
    /// Resting fill of inputs, segmented controls and popups.
    pub field: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub accent: Color32,
    /// Selected fills: the accent at 18%.
    pub selected: Color32,
    pub selected_hover: Color32,
    /// Outlines of selected things: the accent at 55%.
    pub accent_line: Color32,
    pub destructive: Color32,
    /// Faint frame-column guides in the timeline.
    pub guide: Color32,
    /// Status chips floating over the canvas.
    pub overlay: Color32,
    pub canvas_shadow: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub text_tertiary: Color32,
}

/// macOS system accent (blue), which the Swift app uses through `Color.accentColor`.
const ACCENT_DARK: u32 = 0x0A84FFFF;
const ACCENT_LIGHT: u32 = 0x007AFFFF;

fn with_alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a * 255.0).round() as u8)
}

impl Colors {
    pub fn dark() -> Colors {
        let accent = rgba(ACCENT_DARK);
        Colors {
            workspace: rgba(0x1B1B1DFF),
            panel: rgba(0x262628FF),
            panel2: rgba(0x2E2E31FF),
            recessed: rgba(0x00000033),
            elevated: rgba(0x2E2E31FF),
            separator: rgba(0x3A3A3EFF),
            separator_soft: rgba(0x303033FF),
            field: rgba(0x333336FF),
            hover: rgba(0x38383CFF),
            pressed: rgba(0x424247FF),
            accent,
            selected: with_alpha(accent, 0.18),
            selected_hover: with_alpha(accent, 0.26),
            accent_line: with_alpha(accent, 0.55),
            destructive: rgba(0xFF453AFF),
            guide: rgba(0xFFFFFF09),
            overlay: rgba(0x262628E6),
            canvas_shadow: rgba(0x00000059),
            text: rgba(0xECECEEFF),
            text_secondary: rgba(0xA3A3A8FF),
            text_tertiary: rgba(0x75757BFF),
        }
    }

    pub fn light() -> Colors {
        let accent = rgba(ACCENT_LIGHT);
        Colors {
            workspace: rgba(0xE9E9EBFF),
            panel: rgba(0xF6F6F7FF),
            panel2: rgba(0xFFFFFFFF),
            recessed: rgba(0x0000000A),
            elevated: rgba(0xFFFFFFFF),
            separator: rgba(0xCFCFD4FF),
            separator_soft: rgba(0xE0E0E4FF),
            field: rgba(0xE8E8EBFF),
            hover: rgba(0xDEDEE2FF),
            pressed: rgba(0xD3D3D8FF),
            accent,
            selected: with_alpha(accent, 0.18),
            selected_hover: with_alpha(accent, 0.26),
            accent_line: with_alpha(accent, 0.55),
            destructive: rgba(0xFF3B30FF),
            guide: rgba(0x00000009),
            overlay: rgba(0xF6F6F7E6),
            canvas_shadow: rgba(0x00000040),
            text: rgba(0x1D1D1FFF),
            text_secondary: rgba(0x65656BFF),
            text_tertiary: rgba(0x9A9AA0FF),
        }
    }

    /// The colors for the current egui theme.
    pub fn of(ctx: &egui::Context) -> Colors {
        if ctx.theme() == egui::Theme::Dark { Colors::dark() } else { Colors::light() }
    }
}

/// `FontId` for a body-family font of `size`.
pub fn ui_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

/// `FontId` for a medium/semibold label (section titles, tab names).
pub fn strong_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("strong".into()))
}

/// The system UI font of the host, so text looks native next to the macOS app: San Francisco on macOS,
/// Segoe UI on Windows; on Linux egui's bundled font is used.
fn system_fonts() -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let candidates: &[(&str, &str)] = if cfg!(target_os = "macos") {
        &[("/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/SFNS.ttf")]
    } else if cfg!(windows) {
        &[(r"C:\Windows\Fonts\segoeui.ttf", r"C:\Windows\Fonts\seguisb.ttf")]
    } else {
        &[]
    };
    for (regular, strong) in candidates {
        if let Ok(r) = std::fs::read(regular) {
            return (Some(r), std::fs::read(strong).ok());
        }
    }
    (None, None)
}

/// Installs fonts (system UI font plus Phosphor icons) and the Pixmith look on `ctx`.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let (regular, strong) = system_fonts();
    if let Some(r) = regular {
        fonts.font_data.insert("system".into(), Arc::new(FontData::from_owned(r)));
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "system".into());
    }
    let strong_family = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut strong_list = strong_family.clone();
    if let Some(s) = strong {
        fonts.font_data.insert("system-strong".into(), Arc::new(FontData::from_owned(s)));
        strong_list.insert(0, "system-strong".into());
    }
    fonts.families.insert(FontFamily::Name("strong".into()), strong_list);
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    // Icons are also reachable from the strong family.
    if let Some(icons) = fonts.families.get(&FontFamily::Proportional).and_then(|f| f.last().cloned()) {
        fonts.families.entry(FontFamily::Name("strong".into())).or_default().push(icons);
    }
    ctx.set_fonts(fonts);
    apply(ctx, true);
}

/// Applies the light or dark Pixmith style.
pub fn apply(ctx: &egui::Context, dark: bool) {
    let c = if dark { Colors::dark() } else { Colors::light() };
    let mut visuals = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    visuals.panel_fill = c.panel;
    visuals.window_fill = c.elevated;
    visuals.extreme_bg_color = c.field;
    visuals.faint_bg_color = c.recessed;
    visuals.override_text_color = Some(c.text);
    visuals.selection.bg_fill = c.selected;
    visuals.selection.stroke = Stroke::new(1.0, c.accent);
    visuals.hyperlink_color = c.accent;
    visuals.window_corner_radius = CornerRadius::same(radius::PANEL);
    visuals.menu_corner_radius = CornerRadius::same(radius::PANEL);
    visuals.window_stroke = Stroke::new(1.0, c.separator);
    for (w, fill) in [
        (&mut visuals.widgets.noninteractive, c.panel),
        (&mut visuals.widgets.inactive, c.field),
        (&mut visuals.widgets.hovered, c.hover),
        (&mut visuals.widgets.active, c.pressed),
        (&mut visuals.widgets.open, c.hover),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.corner_radius = CornerRadius::same(radius::CONTROL);
        w.bg_stroke = Stroke::NONE;
        w.fg_stroke = Stroke::new(1.0, c.text);
        w.expansion = 0.0;
    }
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, c.separator_soft);
    let theme = if dark { egui::Theme::Dark } else { egui::Theme::Light };
    ctx.set_theme(theme);
    ctx.set_visuals_of(theme, visuals);
    ctx.style_mut_of(theme, |s| {
        s.spacing.item_spacing = egui::vec2(space::XS, space::XS);
        s.spacing.button_padding = egui::vec2(space::SM, space::XS);
        s.spacing.interact_size = egui::vec2(metric::CONTROL, metric::CONTROL);
        s.text_styles.insert(egui::TextStyle::Body, ui_font(font::BODY));
        s.text_styles.insert(egui::TextStyle::Button, ui_font(font::BODY));
        s.text_styles.insert(egui::TextStyle::Small, ui_font(font::CAPTION));
        s.text_styles.insert(egui::TextStyle::Heading, strong_font(font::TITLE));
        s.text_styles.insert(egui::TextStyle::Monospace, FontId::monospace(font::CAPTION));
    });
}
