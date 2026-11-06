//! The egui look: Instrument Sans, an amber accent, and light/dark palettes
//! that follow the system appearance.

use egui::{
    Color32, CornerRadius, FontData, FontFamily, FontId, Shadow, Stroke,
    vec2,
};
use std::sync::Arc;

pub static INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
pub static INTER_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/Inter-Medium.ttf");
pub static INTER_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
pub static INSTRUMENT_REGULAR: &[u8] = include_bytes!("../../assets/fonts/InstrumentSans-Regular.ttf");
pub static INSTRUMENT_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/InstrumentSans-Medium.ttf");
pub static INSTRUMENT_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/InstrumentSans-SemiBold.ttf");
/// The timecode's face: tabular, so the digits don't jiggle as they change.
pub static JETBRAINS_MONO: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Medium.ttf");

/// The one working colour: selection, focus and on/off states (amber, like a
/// VU meter's needle lamp). Text on it uses [`ON_ACCENT`].
pub const AMBER: Color32 = Color32::from_rgb(0xf2, 0xb4, 0x45);
/// Text and marks drawn on an amber fill.
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x1b, 0x1e, 0x25);
/// The tally light: kept for recording and going live, nothing else.
pub const TALLY: Color32 = Color32::from_rgb(0xff, 0x4d, 0x3d);
/// Paused recording.
pub const PAUSED: Color32 = Color32::from_rgb(0xff, 0x9f, 0x0a);
/// A healthy signal: the low part of the meters, "ready" dots.
pub const SIGNAL: Color32 = Color32::from_rgb(0x4f, 0xd1, 0xb5);

/// Body text (13pt) and secondary text (11pt), as in AppKit.
pub const BODY: f32 = 13.0;
pub const SECONDARY: f32 = 11.0;
/// Height of the unified toolbar.
pub const TOOLBAR_H: f32 = 46.0;
/// Traffic lights: left inset and the height of the bar they are centred in (points).
pub const TRAFFIC_LIGHTS: (f64, f64) = (16.0, TOOLBAR_H as f64);
/// Horizontal room the traffic lights need from the window's left edge.
pub const TRAFFIC_LIGHTS_W: f32 = 78.0;

pub fn accent() -> Color32 {
    AMBER
}

/// Colours for one appearance, loosely after AppKit's semantic colours.
#[derive(Clone, Copy)]
pub struct Palette {
    pub window: Color32,
    pub sidebar: Color32,
    pub inspector: Color32,
    pub control: Color32,
    pub control_border: Color32,
    pub field: Color32,
    pub text: Color32,
    pub secondary: Color32,
    pub tertiary: Color32,
    pub separator: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub popover: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        // Graphite with a little blue in it, like a control room at night.
        Palette {
            window: Color32::from_rgb(0x1b, 0x1e, 0x25),
            sidebar: Color32::from_rgb(0x21, 0x25, 0x2e),
            inspector: Color32::from_rgb(0x21, 0x25, 0x2e),
            control: Color32::from_rgb(0x2c, 0x31, 0x3c),
            control_border: Color32::from_white_alpha(14),
            field: Color32::from_rgb(0x17, 0x1a, 0x20),
            text: Color32::from_rgb(0xec, 0xee, 0xf2),
            secondary: Color32::from_rgb(0xa3, 0xaa, 0xb8),
            tertiary: Color32::from_rgb(0x7b, 0x83, 0x94),
            separator: Color32::from_rgb(0x2e, 0x33, 0x3e),
            hover: Color32::from_white_alpha(14),
            pressed: Color32::from_white_alpha(28),
            popover: Color32::from_rgb(0x26, 0x2b, 0x35),
        }
    } else {
        Palette {
            window: Color32::from_rgb(0xf1, 0xf3, 0xf6),
            sidebar: Color32::from_rgb(0xfa, 0xfb, 0xfc),
            inspector: Color32::from_rgb(0xfa, 0xfb, 0xfc),
            control: Color32::from_rgb(0xe8, 0xeb, 0xf0),
            control_border: Color32::from_black_alpha(18),
            field: Color32::WHITE,
            text: Color32::from_rgb(0x1b, 0x1e, 0x25),
            secondary: Color32::from_rgb(0x5b, 0x62, 0x72),
            tertiary: Color32::from_rgb(0x8a, 0x91, 0x9f),
            separator: Color32::from_rgb(0xe1, 0xe4, 0xea),
            hover: Color32::from_black_alpha(10),
            pressed: Color32::from_black_alpha(20),
            popover: Color32::WHITE,
        }
    }
}

pub fn pal(ui: &egui::Ui) -> Palette {
    palette(ui.visuals().dark_mode)
}

/// Instrument Sans (Regular/Medium/SemiBold) as the proportional family, with
/// Inter and then egui's fonts as fallbacks for symbols it lacks; JetBrains
/// Mono for the `timecode` family.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("Instrument", INSTRUMENT_REGULAR),
        ("Instrument-Medium", INSTRUMENT_MEDIUM),
        ("Instrument-SemiBold", INSTRUMENT_SEMIBOLD),
        ("Inter", INTER_REGULAR),
        ("Inter-Medium", INTER_MEDIUM),
        ("Inter-SemiBold", INTER_SEMIBOLD),
        ("JetBrainsMono", JETBRAINS_MONO),
    ] {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    let fallbacks: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let with = |first: &str, inter: &str| {
        let mut v = vec![first.to_string(), inter.to_string()];
        v.extend(fallbacks.iter().cloned());
        v
    };
    fonts
        .families
        .insert(FontFamily::Proportional, with("Instrument", "Inter"));
    fonts
        .families
        .insert(FontFamily::Name("medium".into()), with("Instrument-Medium", "Inter-Medium"));
    fonts
        .families
        .insert(FontFamily::Name("semibold".into()), with("Instrument-SemiBold", "Inter-SemiBold"));
    fonts
        .families
        .insert(FontFamily::Name("timecode".into()), with("JetBrainsMono", "Inter-Medium"));
    if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
        mono.push("Inter".into());
    }
    ctx.set_fonts(fonts);
}

/// The recording timecode (JetBrains Mono).
pub fn timecode(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("timecode".into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

fn style_for(s: &mut egui::Style, dark: bool) {
    use egui::TextStyle as T;
    let p = palette(dark);
    let acc = accent();
    s.visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    s.text_styles = [
        (T::Small, FontId::proportional(SECONDARY)),
        (T::Body, FontId::proportional(BODY)),
        (T::Button, FontId::proportional(BODY)),
        (T::Heading, semibold(BODY)),
        (T::Monospace, FontId::monospace(12.0)),
    ]
    .into();
    s.spacing.item_spacing = vec2(6.0, 4.0);
    s.spacing.button_padding = vec2(10.0, 4.0);
    s.spacing.interact_size = vec2(24.0, 22.0);
    s.spacing.menu_margin = egui::Margin::same(5);
    s.spacing.window_margin = egui::Margin::same(12);
    s.spacing.scroll = egui::style::ScrollStyle {
        bar_width: 7.0,
        handle_min_length: 24.0,
        ..egui::style::ScrollStyle::floating()
    };
    s.interaction.selectable_labels = false;

    let v = &mut s.visuals;
    v.override_text_color = None;
    v.weak_text_color = Some(p.secondary);
    // Amber text is too light on the light theme: a darker shade there.
    v.hyperlink_color = if dark { acc } else { Color32::from_rgb(0x9a, 0x62, 0x00) };
    v.panel_fill = p.inspector;
    v.window_fill = p.popover;
    v.faint_bg_color = if dark {
        Color32::from_white_alpha(6)
    } else {
        Color32::from_black_alpha(6)
    };
    v.extreme_bg_color = p.field;
    v.text_edit_bg_color = Some(p.field);
    v.window_stroke = Stroke::new(1.0, p.control_border);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 110 } else { 40 }),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 90 } else { 30 }),
    };
    v.selection.bg_fill = acc;
    v.selection.stroke = Stroke::new(1.0, ON_ACCENT);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.indent_has_left_vline = false;
    v.striped = false;
    v.interact_cursor = None;
    v.collapsing_header_frame = false;

    let r6 = CornerRadius::same(7);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.inspector;
    w.noninteractive.weak_bg_fill = p.inspector;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.separator);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r6;
    w.inactive.bg_fill = p.control;
    w.inactive.weak_bg_fill = p.control;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r6;
    w.inactive.expansion = 0.0;
    w.hovered.bg_fill = p.control;
    w.hovered.weak_bg_fill = blend(p.control, p.hover);
    w.hovered.bg_stroke = Stroke::NONE;
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.corner_radius = r6;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = blend(p.control, p.pressed);
    w.active.weak_bg_fill = blend(p.control, p.pressed);
    w.active.bg_stroke = Stroke::NONE;
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.active.corner_radius = r6;
    w.active.expansion = 0.0;
    w.open = w.active;
}

/// `over` composited on `base`.
pub fn blend(base: Color32, over: Color32) -> Color32 {
    let a = over.a() as f32 / 255.0;
    let [or, og, ob, _] = over.to_srgba_unmultiplied();
    let mix = |b: u8, o: u8| (b as f32 * (1.0 - a) + o as f32 * a).round() as u8;
    Color32::from_rgb(mix(base.r(), or), mix(base.g(), og), mix(base.b(), ob))
}

/// Fonts plus light and dark styles. egui picks the one matching the system appearance
/// (reported by egui-winit); headless rendering falls back to light.
pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.options_mut(|o| {
        o.theme_preference = egui::ThemePreference::System;
        o.fallback_theme = egui::Theme::Light;
    });
    ctx.style_mut_of(egui::Theme::Light, |s| style_for(s, false));
    ctx.style_mut_of(egui::Theme::Dark, |s| style_for(s, true));
}

