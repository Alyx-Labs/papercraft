//! Clean Light and Dark studio design tokens matching PaperCraft.

use egui::Color32;
use papercraft_doc::UiTheme;

pub const MICRO_DURATION_S: f32 = 0.14;
pub const STANDARD_DURATION_S: f32 = 0.20;

/// Color tokens for the PaperCraft Server control window.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub is_dark: bool,
    pub toolbar_bg: Color32,
    pub panel_bg: Color32,
    pub canvas_bg: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_soft: Color32,
    pub field_bg: Color32,
    pub danger_fg: Color32,
    pub danger_border: Color32,
    pub danger_bg: Color32,
    pub success_fg: Color32,
    pub success_border: Color32,
    pub success_bg: Color32,
    pub warning_fg: Color32,
}

impl Tokens {
    pub fn for_theme(theme: UiTheme) -> Self {
        match theme {
            UiTheme::Light => Self::light(),
            UiTheme::Dark => Self::dark(),
        }
    }

    pub fn light() -> Self {
        Self {
            is_dark: false,
            toolbar_bg: Color32::from_rgb(0xF6, 0xF7, 0xFA),
            panel_bg: Color32::from_rgb(0xFA, 0xFB, 0xFD),
            canvas_bg: Color32::from_rgb(0xE4, 0xE7, 0xEE),
            border: Color32::from_rgb(0xD8, 0xDD, 0xE6),
            text: Color32::from_rgb(0x1E, 0x22, 0x2B),
            text_muted: Color32::from_rgb(0x62, 0x6A, 0x79),
            accent: Color32::from_rgb(0x28, 0x66, 0xE8),
            accent_hover: Color32::from_rgb(0x1E, 0x57, 0xD4),
            accent_soft: Color32::from_rgb(0xE3, 0xEC, 0xFC),
            field_bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            danger_fg: Color32::from_rgb(0xB4, 0x28, 0x28),
            danger_border: Color32::from_rgb(0xE4, 0xBE, 0xBE),
            danger_bg: Color32::from_rgb(0xFD, 0xF5, 0xF5),
            success_fg: Color32::from_rgb(0x15, 0x80, 0x3D),
            success_border: Color32::from_rgb(0x9E, 0xDE, 0xB8),
            success_bg: Color32::from_rgb(0xEC, 0xFD, 0xF3),
            warning_fg: Color32::from_rgb(0xB4, 0x53, 0x09),
        }
    }

    pub fn dark() -> Self {
        Self {
            is_dark: true,
            toolbar_bg: Color32::from_rgb(0x1B, 0x1E, 0x26),
            panel_bg: Color32::from_rgb(0x15, 0x18, 0x1F),
            canvas_bg: Color32::from_rgb(0x0F, 0x11, 0x17),
            border: Color32::from_rgb(0x2E, 0x34, 0x40),
            text: Color32::from_rgb(0xED, 0xF0, 0xF6),
            text_muted: Color32::from_rgb(0x9A, 0xA3, 0xB5),
            accent: Color32::from_rgb(0x3B, 0x76, 0xEC),
            accent_hover: Color32::from_rgb(0x52, 0x89, 0xF5),
            accent_soft: Color32::from_rgb(0x1F, 0x32, 0x54),
            field_bg: Color32::from_rgb(0x22, 0x26, 0x31),
            danger_fg: Color32::from_rgb(0xF0, 0x7A, 0x7A),
            danger_border: Color32::from_rgb(0x5E, 0x31, 0x36),
            danger_bg: Color32::from_rgb(0x2D, 0x1C, 0x20),
            success_fg: Color32::from_rgb(0x3E, 0xD0, 0x76),
            success_border: Color32::from_rgb(0x23, 0x5B, 0x3B),
            success_bg: Color32::from_rgb(0x14, 0x2B, 0x1E),
            warning_fg: Color32::from_rgb(0xF5, 0x9E, 0x0B),
        }
    }
}

/// Applies the selected studio palette to standard `egui` widgets.
pub fn apply_visuals(ctx: &egui::Context, theme: UiTheme) {
    let t = Tokens::for_theme(theme);
    let egui_theme = match theme {
        UiTheme::Light => egui::Theme::Light,
        UiTheme::Dark => egui::Theme::Dark,
    };
    let active_style = ctx.style_of(egui_theme);
    let needs_apply = ctx.theme() != egui_theme
        || active_style.visuals.dark_mode != t.is_dark
        || active_style.visuals.panel_fill != t.panel_bg
        || active_style.visuals.extreme_bg_color != t.field_bg
        || (active_style.animation_time - STANDARD_DURATION_S).abs() > 1e-4;
    if !needs_apply {
        return;
    }

    ctx.set_theme(egui_theme);
    let mut visuals = if t.is_dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = Some(t.text);
    visuals.panel_fill = t.panel_bg;
    visuals.window_fill = t.field_bg;
    visuals.window_stroke = egui::Stroke::new(1.0, t.border);
    visuals.faint_bg_color = t.toolbar_bg;
    visuals.extreme_bg_color = t.field_bg;
    visuals.selection.bg_fill = t.accent_soft;
    visuals.selection.stroke = egui::Stroke::new(1.5, t.accent);

    let control_radius = egui::CornerRadius::same(7);
    visuals.widgets.noninteractive.bg_fill = t.panel_bg;
    visuals.widgets.noninteractive.weak_bg_fill = t.toolbar_bg;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, t.border);
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, t.text);
    visuals.widgets.noninteractive.corner_radius = control_radius;

    visuals.widgets.inactive.bg_fill = t.field_bg;
    visuals.widgets.inactive.weak_bg_fill = t.field_bg;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, t.border);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, t.text);
    visuals.widgets.inactive.corner_radius = control_radius;

    visuals.widgets.hovered.bg_fill = t.accent_soft;
    visuals.widgets.hovered.weak_bg_fill = t.accent_soft;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, t.accent);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, t.text);
    visuals.widgets.hovered.corner_radius = control_radius;

    visuals.widgets.active.bg_fill = t.accent;
    visuals.widgets.active.weak_bg_fill = t.accent;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, t.accent);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.active.corner_radius = control_radius;

    visuals.widgets.open.bg_fill = t.accent_soft;
    visuals.widgets.open.weak_bg_fill = t.accent_soft;
    visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0, t.accent);
    visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0, t.text);
    visuals.widgets.open.corner_radius = control_radius;

    ctx.set_visuals(visuals);
    ctx.all_styles_mut(|style| {
        style.animation_time = STANDARD_DURATION_S;
    });
}

#[inline]
pub fn hover_factor(ctx: &egui::Context, id: egui::Id, active: bool) -> f32 {
    ctx.animate_bool_with_time(id, active, MICRO_DURATION_S)
}

#[inline]
pub fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    let u = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    a + (b - a) * u
}

#[inline]
pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let u = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    Color32::from_rgba_unmultiplied(
        (f32::from(a.r()) + (f32::from(b.r()) - f32::from(a.r())) * u).round() as u8,
        (f32::from(a.g()) + (f32::from(b.g()) - f32::from(a.g())) * u).round() as u8,
        (f32::from(a.b()) + (f32::from(b.b()) - f32::from(a.b())) * u).round() as u8,
        (f32::from(a.a()) + (f32::from(b.a()) - f32::from(a.a())) * u).round() as u8,
    )
}
