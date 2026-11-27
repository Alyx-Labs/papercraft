//! UI design tokens and Light/Dark theme visual styling for PaperCraft.

use egui::Color32;
use papercraft_color::Color;
use papercraft_doc::{InfiniteThemeConfig, UiTheme, UserStudioConfig};

#[inline]
pub fn to_color32(c: Color) -> Color32 {
    let [r, g, b, a] = c.to_rgba8();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Color token set backing every PaperCraft shell, sidebar, dialog, and canvas backdrop.
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
    pub on_accent: Color32,
    pub field_bg: Color32,
    pub tab_strip_bg: Color32,
    pub tab_strip_fg: Color32,
    pub tab_strip_fg_muted: Color32,
    pub header_bg: Color32,
    pub header_border: Color32,
    pub header_fg: Color32,
    pub header_fg_muted: Color32,
    pub header_track_bg: Color32,
    pub header_control_hover: Color32,
    pub header_active_bg: Color32,
    pub header_active_fg: Color32,
    pub brand_craft: Color32,
    pub danger_fg: Color32,
    pub danger_border: Color32,
    pub danger_bg: Color32,
    pub success_fg: Color32,
    pub success_border: Color32,
    pub success_bg: Color32,
    pub warning_fg: Color32,
}

/// Applies W3C CSS `hue-rotate(hue_degrees) saturate(saturation_percent%)` to a UI color
/// while preserving alpha and leaving neutral grayscale values untouched.
pub(crate) fn apply_hue_saturation(
    color: Color32,
    hue_degrees: f32,
    saturation_percent: f32,
) -> Color32 {
    let clean_hue = InfiniteThemeConfig::sanitize_hue_degrees(hue_degrees);
    let clean_sat = InfiniteThemeConfig::sanitize_saturation_percent(saturation_percent);
    let hue_is_zero = (clean_hue - InfiniteThemeConfig::DEFAULT_HUE_DEGREES).abs() < 0.01
        || (clean_hue - InfiniteThemeConfig::MAX_HUE_DEGREES).abs() < 0.01;
    let sat_is_default = (clean_sat - InfiniteThemeConfig::DEFAULT_SATURATION_PERCENT).abs() < 0.01;
    if hue_is_zero && sat_is_default {
        return color;
    }

    let theta = clean_hue.to_radians();
    let cos = theta.cos();
    let sin = theta.sin();
    let sat = (clean_sat / 100.0).clamp(0.0, 2.5);

    let [r8, g8, b8, a8] = color.to_srgba_unmultiplied();
    let r = f32::from(r8) / 255.0;
    let g = f32::from(g8) / 255.0;
    let b = f32::from(b8) / 255.0;

    let hr = (0.213 + 0.787 * cos - 0.213 * sin) * r
        + (0.715 - 0.715 * cos - 0.715 * sin) * g
        + (0.072 - 0.072 * cos + 0.928 * sin) * b;
    let hg = (0.213 - 0.213 * cos + 0.143 * sin) * r
        + (0.715 + 0.285 * cos + 0.140 * sin) * g
        + (0.072 - 0.072 * cos - 0.283 * sin) * b;
    let hb = (0.213 - 0.213 * cos - 0.787 * sin) * r
        + (0.715 - 0.715 * cos + 0.715 * sin) * g
        + (0.072 + 0.928 * cos + 0.072 * sin) * b;

    let lum = 0.213 * hr + 0.715 * hg + 0.072 * hb;
    let out_r = ((lum + sat * (hr - lum)).clamp(0.0, 1.0) * 255.0).round() as u8;
    let out_g = ((lum + sat * (hg - lum)).clamp(0.0, 1.0) * 255.0).round() as u8;
    let out_b = ((lum + sat * (hb - lum)).clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(out_r, out_g, out_b, a8)
}

impl Tokens {
    /// Returns the token set for the active [`UiTheme`].
    pub fn for_theme(theme: UiTheme) -> Self {
        match theme {
            UiTheme::Light => Self::light(),
            UiTheme::Dark => Self::dark(),
        }
    }

    /// Returns the token set for `theme` with the optional [`InfiniteThemeConfig`] UI tint applied.
    pub fn for_theme_with_infinite(theme: UiTheme, infinite: InfiniteThemeConfig) -> Self {
        Self::for_theme(theme).with_infinite(infinite)
    }

    /// Returns the active UI token set derived from `prefs` (respecting Light/Dark + Infinite tint).
    pub fn for_prefs(prefs: &UserStudioConfig) -> Self {
        Self::for_theme_with_infinite(prefs.theme, prefs.infinite_theme)
    }

    /// Applies Infinite Theme hue rotation and saturation scaling to UI chrome tokens while
    /// keeping semantic status colors (`danger_*`, `success_*`, `warning_fg`) hue-consistent.
    #[must_use]
    pub fn with_infinite(self, infinite: InfiniteThemeConfig) -> Self {
        if infinite.is_identity() {
            return self;
        }
        let clean = infinite.sanitized();
        let shift =
            |c: Color32| apply_hue_saturation(c, clean.hue_degrees, clean.saturation_percent);
        let status_sat = clean.saturation_percent.min(100.0);
        let shift_status = |c: Color32| apply_hue_saturation(c, 0.0, status_sat);
        Self {
            is_dark: self.is_dark,
            toolbar_bg: shift(self.toolbar_bg),
            panel_bg: shift(self.panel_bg),
            canvas_bg: shift(self.canvas_bg),
            border: shift(self.border),
            text: shift(self.text),
            text_muted: shift(self.text_muted),
            accent: shift(self.accent),
            accent_hover: shift(self.accent_hover),
            accent_soft: shift(self.accent_soft),
            on_accent: shift(self.on_accent),
            field_bg: shift(self.field_bg),
            tab_strip_bg: shift(self.tab_strip_bg),
            tab_strip_fg: shift(self.tab_strip_fg),
            tab_strip_fg_muted: shift(self.tab_strip_fg_muted),
            header_bg: shift(self.header_bg),
            header_border: shift(self.header_border),
            header_fg: shift(self.header_fg),
            header_fg_muted: shift(self.header_fg_muted),
            header_track_bg: shift(self.header_track_bg),
            header_control_hover: shift(self.header_control_hover),
            header_active_bg: shift(self.header_active_bg),
            header_active_fg: shift(self.header_active_fg),
            brand_craft: shift(self.brand_craft),
            danger_fg: shift_status(self.danger_fg),
            danger_border: shift_status(self.danger_border),
            danger_bg: shift_status(self.danger_bg),
            success_fg: shift_status(self.success_fg),
            success_border: shift_status(self.success_border),
            success_bg: shift_status(self.success_bg),
            warning_fg: shift_status(self.warning_fg),
        }
    }

    /// Calm bright daylight studio palette.
    pub fn light() -> Self {
        Self {
            is_dark: false,
            toolbar_bg: Color32::from_rgb(0xF4, 0xF6, 0xF8),
            panel_bg: Color32::from_rgb(0xFA, 0xFB, 0xFC),
            canvas_bg: Color32::from_rgb(0xEC, 0xEF, 0xF3),
            border: Color32::from_rgb(0xD9, 0xE0, 0xE7),
            text: Color32::from_rgb(0x1B, 0x23, 0x2C),
            text_muted: Color32::from_rgb(0x5B, 0x66, 0x73),
            accent: Color32::from_rgb(0x35, 0x6B, 0x8F),
            accent_hover: Color32::from_rgb(0x2A, 0x58, 0x77),
            accent_soft: Color32::from_rgb(0xE8, 0xF1, 0xF6),
            on_accent: Color32::WHITE,
            field_bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            tab_strip_bg: Color32::from_rgb(0x82, 0xCB, 0xF3),
            tab_strip_fg: Color32::from_rgb(0x09, 0x1E, 0x2E),
            tab_strip_fg_muted: Color32::from_rgb(0x16, 0x3A, 0x52),
            header_bg: Color32::from_rgb(0x50, 0xA4, 0xD0),
            header_border: Color32::from_rgb(0x38, 0x84, 0xAD),
            header_fg: Color32::from_rgb(0x09, 0x1E, 0x2E),
            header_fg_muted: Color32::from_rgb(0x14, 0x31, 0x45),
            header_track_bg: Color32::from_rgb(0x6A, 0xB7, 0xE0),
            header_control_hover: Color32::from_rgb(0x8D, 0xD1, 0xF5),
            header_active_bg: Color32::from_rgb(0xE8, 0xF4, 0xFC),
            header_active_fg: Color32::from_rgb(0x0B, 0x2E, 0x46),
            brand_craft: Color32::from_rgb(0x0A, 0x46, 0x6B),
            danger_fg: Color32::from_rgb(0xB4, 0x28, 0x28),
            danger_border: Color32::from_rgb(0xE4, 0xBE, 0xBE),
            danger_bg: Color32::from_rgb(0xFD, 0xF5, 0xF5),
            success_fg: Color32::from_rgb(0x15, 0x80, 0x3D),
            success_border: Color32::from_rgb(0x9E, 0xDE, 0xB8),
            success_bg: Color32::from_rgb(0xEC, 0xFD, 0xF3),
            warning_fg: Color32::from_rgb(0xB4, 0x53, 0x09),
        }
    }

    /// Low-glare slate-charcoal dark studio palette.
    pub fn dark() -> Self {
        Self {
            is_dark: true,
            toolbar_bg: Color32::from_rgb(0x27, 0x2B, 0x32),
            panel_bg: Color32::from_rgb(0x20, 0x23, 0x29),
            canvas_bg: Color32::from_rgb(0x19, 0x1B, 0x20),
            border: Color32::from_rgb(0x3E, 0x45, 0x52),
            text: Color32::from_rgb(0xEF, 0xF2, 0xF7),
            text_muted: Color32::from_rgb(0xA3, 0xAC, 0xB9),
            accent: Color32::from_rgb(0x82, 0xCA, 0xF2),
            accent_hover: Color32::from_rgb(0x9F, 0xD7, 0xF7),
            accent_soft: Color32::from_rgb(0x1F, 0x3A, 0x4A),
            on_accent: Color32::from_rgb(0x09, 0x24, 0x34),
            field_bg: Color32::from_rgb(0x2F, 0x34, 0x3D),
            tab_strip_bg: Color32::from_rgb(0x19, 0x1B, 0x20),
            tab_strip_fg: Color32::from_rgb(0xEF, 0xF2, 0xF7),
            tab_strip_fg_muted: Color32::from_rgb(0xA3, 0xAC, 0xB9),
            header_bg: Color32::from_rgb(0x27, 0x2B, 0x32),
            header_border: Color32::from_rgb(0x3E, 0x45, 0x52),
            header_fg: Color32::from_rgb(0xEF, 0xF2, 0xF7),
            header_fg_muted: Color32::from_rgb(0xA3, 0xAC, 0xB9),
            header_track_bg: Color32::from_rgb(0x27, 0x2B, 0x32),
            header_control_hover: Color32::from_rgb(0x33, 0x38, 0x42),
            header_active_bg: Color32::from_rgb(0x1F, 0x3A, 0x4A),
            header_active_fg: Color32::from_rgb(0x82, 0xCA, 0xF2),
            brand_craft: Color32::from_rgb(0x82, 0xCA, 0xF2),
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

/// Applies PaperCraft's active studio palette (`Light` or `Dark`) to standard `egui` widgets.
pub fn apply_visuals(ctx: &egui::Context, theme: UiTheme) {
    apply_visuals_with_infinite(ctx, theme, InfiniteThemeConfig::default());
}

/// Applies PaperCraft's active studio palette and Infinite Theme modifier from `prefs` to `egui` widgets.
pub fn apply_prefs_visuals(ctx: &egui::Context, prefs: &UserStudioConfig) {
    apply_visuals_with_infinite(ctx, prefs.theme, prefs.infinite_theme);
}

/// Applies `theme` with an optional [`InfiniteThemeConfig`] modifier to standard `egui` widgets.
pub fn apply_visuals_with_infinite(
    ctx: &egui::Context,
    theme: UiTheme,
    infinite: InfiniteThemeConfig,
) {
    let t = Tokens::for_theme_with_infinite(theme, infinite);
    let egui_theme = match theme {
        UiTheme::Light => egui::Theme::Light,
        UiTheme::Dark => egui::Theme::Dark,
    };
    let active_style = ctx.style_of(egui_theme);
    let needs_apply = ctx.theme() != egui_theme
        || active_style.visuals.dark_mode != t.is_dark
        || active_style.visuals.panel_fill != t.panel_bg
        || active_style.visuals.extreme_bg_color != t.field_bg
        || active_style.visuals.faint_bg_color != t.toolbar_bg
        || active_style.visuals.override_text_color != Some(t.text)
        || active_style.visuals.selection.bg_fill != t.accent_soft
        || active_style.visuals.selection.stroke.color != t.accent
        || active_style.visuals.widgets.noninteractive.bg_stroke.color != t.border
        || active_style.interaction.selectable_labels
        || (active_style.animation_time - crate::motion::STANDARD_DURATION_S).abs() > 1e-4;
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
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, t.on_accent);
    visuals.widgets.active.corner_radius = control_radius;

    visuals.widgets.open.bg_fill = t.accent_soft;
    visuals.widgets.open.weak_bg_fill = t.accent_soft;
    visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0, t.accent);
    visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0, t.text);
    visuals.widgets.open.corner_radius = control_radius;

    ctx.set_visuals(visuals);
    ctx.all_styles_mut(|style| {
        crate::motion::apply_motion_style(style);
        style.interaction.selectable_labels = false;
    });
}

/// Renders a `ComboBox` matching PaperCraft's standard 28px control height,
/// 7px corner radius, crisp border, and pointing-hand hover cursor.
pub(crate) fn studio_combo_box<R>(
    ui: &mut egui::Ui,
    id_salt: impl egui::AsIdSalt,
    selected_text: impl Into<egui::WidgetText>,
    width: f32,
    menu_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    let selected_text: egui::WidgetText = selected_text.into();
    ui.scope(|ui| {
        let spacing = ui.spacing_mut();
        spacing.interact_size.y = 28.0;
        spacing.button_padding = egui::vec2(10.0, 4.0);
        let out = egui::ComboBox::from_id_salt(id_salt)
            .selected_text(selected_text)
            .width(width)
            .show_ui(ui, menu_contents);
        let resp = out.response.on_hover_cursor(egui::CursorIcon::PointingHand);
        egui::InnerResponse::new(out.inner, resp)
    })
    .inner
}

fn studio_row_label_galley(
    ui: &egui::Ui,
    text: impl Into<egui::WidgetText>,
) -> std::sync::Arc<egui::Galley> {
    let text: egui::WidgetText = text.into();
    text.into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    )
}

/// Measures the horizontal width of a [`studio_row_label`].
pub(crate) fn studio_row_label_width(ui: &egui::Ui, text: impl Into<egui::WidgetText>) -> f32 {
    studio_row_label_galley(ui, text).size().x
}

/// Measures the minimum/rendered width of a [`studio_combo_box`].
pub(crate) fn studio_combo_box_width(ui: &egui::Ui, selected_text: &str, min_width: f32) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        selected_text.to_string(),
        egui::TextStyle::Button.resolve(ui.style()),
        Color32::WHITE,
    );
    let chrome = 20.0 + ui.spacing().icon_width + ui.spacing().icon_spacing + 4.0;
    min_width.max(galley.size().x + chrome)
}

/// Renders a single-line label vertically centered inside `row_height` so it aligns
/// with adjacent fixed-height controls (`studio_combo_box`, `DragValue`, buttons).
pub(crate) fn studio_row_label(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    row_height: f32,
) -> egui::Response {
    let galley = studio_row_label_galley(ui, text);
    let desired_size = egui::vec2(galley.size().x, row_height.max(galley.size().y));
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let pos = egui::pos2(rect.min.x, rect.center().y - galley.size().y * 0.5);
        ui.painter().galley(pos, galley, ui.visuals().text_color());
    }
    response
}

/// Runs `add_contents` inside a wrapping horizontal row with vertical center
/// alignment and a minimum row height so leading labels never start shifted high.
pub(crate) fn studio_wrapped_row<R>(
    ui: &mut egui::Ui,
    spacing: egui::Vec2,
    row_height: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let initial_size = egui::vec2(
        ui.available_size_before_wrap().x,
        ui.spacing().interact_size.y.max(row_height),
    );
    ui.allocate_ui_with_layout(
        initial_size,
        egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
        |ui| {
            ui.spacing_mut().item_spacing = spacing;
            add_contents(ui)
        },
    )
}

/// Brand color used for the `"Craft"` accent in the `"Paper Craft"` header wordmark.
pub(crate) fn brand_craft_color(t: &Tokens) -> Color32 {
    t.brand_craft
}

/// Builds the single-line `"Paper Craft"` header wordmark in the display font (`Bricolage Grotesque`).
pub(crate) fn brand_wordmark_job(font_size: f32, t: &Tokens) -> egui::text::LayoutJob {
    let family = egui::FontFamily::Name(crate::BRAND_DISPLAY_FAMILY.into());
    let base = egui::TextFormat {
        font_id: egui::FontId::new(font_size, family),
        color: t.header_fg,
        extra_letter_spacing: -0.25,
        ..Default::default()
    };
    let mut craft = base.clone();
    craft.color = brand_craft_color(t);

    let mut job = egui::text::LayoutJob::default();
    job.append("Paper ", 0.0, base);
    job.append("Craft", 0.0, craft);
    job
}

/// Applies the Light studio palette cleanly (used by theme-neutral unit tests).
#[cfg(test)]
pub fn apply_light_visuals(ctx: &egui::Context) {
    apply_visuals(ctx, UiTheme::Light);
}

#[cfg(test)]
mod tests {
    use super::{Tokens, apply_visuals};
    use crate::{AppScreen, PaperCraftApp, SettingsTab, modal, motion, open_notebook_in_editor};
    use egui::{Color32, Pos2, RawInput, Rect, Shape, Vec2};
    use papercraft_doc::UiTheme;

    fn channel_sum(c: Color32) -> u16 {
        u16::from(c.r()) + u16::from(c.g()) + u16::from(c.b())
    }

    fn collect_text_shapes(shapes: &[egui::epaint::ClippedShape]) -> String {
        let mut out = String::new();
        fn visit(shape: &Shape, buf: &mut String) {
            match shape {
                Shape::Text(text) => {
                    buf.push_str(text.galley.text());
                    buf.push('\n');
                }
                Shape::Vec(items) => {
                    for s in items {
                        visit(s, buf);
                    }
                }
                _ => {}
            }
        }
        for clipped in shapes {
            visit(&clipped.shape, &mut out);
        }
        out
    }

    fn run_app_frame(ctx: &egui::Context, app: &mut PaperCraftApp) -> String {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0))),
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
        let texts = collect_text_shapes(&out.shapes);
        assert!(!out.shapes.is_empty(), "rendered frame must emit shapes");
        out.drop_without_applying_deltas();
        texts
    }

    #[test]
    fn tokens_for_theme_distinguish_light_and_dark_palettes() {
        let light = Tokens::for_theme(UiTheme::Light);
        let dark = Tokens::for_theme(UiTheme::Dark);

        assert!(!light.is_dark);
        assert!(dark.is_dark);
        assert_ne!(light.canvas_bg, dark.canvas_bg);
        assert_ne!(light.panel_bg, dark.panel_bg);
        assert_ne!(light.toolbar_bg, dark.toolbar_bg);
        assert_ne!(light.text, dark.text);
        assert_ne!(light.danger_fg, dark.danger_fg);
        assert_ne!(light.success_fg, dark.success_fg);
        assert_ne!(light.warning_fg, dark.warning_fg);

        assert!(channel_sum(light.panel_bg) > channel_sum(light.text));
        assert!(channel_sum(light.canvas_bg) > channel_sum(light.text));
        assert!(channel_sum(dark.panel_bg) < channel_sum(dark.text));
        assert!(channel_sum(dark.canvas_bg) < channel_sum(dark.text));
    }

    fn relative_luminance(c: Color32) -> f32 {
        fn channel(v: u8) -> f32 {
            let s = f32::from(v) / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }

    fn contrast_ratio(a: Color32, b: Color32) -> f32 {
        let la = relative_luminance(a);
        let lb = relative_luminance(b);
        let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn assert_light_cyan_contrast(light: &Tokens) {
        let light_brand_blue = Color32::from_rgb(0x0A, 0x46, 0x6B);
        assert_eq!(super::brand_craft_color(light), light_brand_blue);
        assert_eq!(light.tab_strip_bg, Color32::from_rgb(0x82, 0xCB, 0xF3));
        assert_eq!(light.header_bg, Color32::from_rgb(0x50, 0xA4, 0xD0));
        assert_eq!(light.header_active_bg, Color32::from_rgb(0xE8, 0xF4, 0xFC));

        // Two-tier Light header hierarchy: bright sky tab strip (#82CBF3) -> deeper cerulean active header (#50A4D0).
        assert!(relative_luminance(light.tab_strip_bg) > relative_luminance(light.header_bg));
        let tier_contrast = contrast_ratio(light.tab_strip_bg, light.header_bg);
        assert!(
            tier_contrast >= 1.4,
            "expected clear two-tier header contrast in Light theme, got {tier_contrast:.3}"
        );
        assert!(contrast_ratio(light.tab_strip_fg, light.tab_strip_bg) >= 7.0);
        assert!(contrast_ratio(light.tab_strip_fg_muted, light.tab_strip_bg) >= 4.5);
        assert!(contrast_ratio(light.header_fg, light.header_bg) >= 6.0);
        assert!(contrast_ratio(light.header_fg_muted, light.header_bg) >= 4.5);
        assert!(contrast_ratio(light.header_fg, light.header_track_bg) >= 7.0);
        assert!(contrast_ratio(light.header_fg_muted, light.header_track_bg) >= 4.5);
        assert!(contrast_ratio(light.header_fg, light.header_control_hover) >= 7.0);
        assert!(contrast_ratio(light.header_active_fg, light.header_active_bg) >= 7.0);
        assert!(contrast_ratio(super::brand_craft_color(light), light.header_bg) >= 3.0);

        // Light body cyan scale readability.
        assert!(contrast_ratio(light.accent, light.field_bg) >= 4.5);
        assert!(contrast_ratio(light.accent, light.toolbar_bg) >= 4.5);
        assert!(contrast_ratio(light.accent, light.accent_soft) >= 4.5);
        assert!(contrast_ratio(light.on_accent, light.accent) >= 4.5);
        assert!(contrast_ratio(light.text, light.panel_bg) >= 7.0);
    }

    fn assert_dark_cyan_contrast(dark: &Tokens) {
        let brand_cyan = Color32::from_rgb(0x82, 0xCA, 0xF2);
        assert_eq!(super::brand_craft_color(dark), brand_cyan);
        assert_eq!(dark.accent, brand_cyan);

        // Dark charcoal surfaces and brand cyan accents.
        assert!(contrast_ratio(dark.text, dark.panel_bg) >= 7.0);
        assert!(contrast_ratio(dark.text_muted, dark.panel_bg) >= 4.5);
        assert!(contrast_ratio(dark.accent, dark.toolbar_bg) >= 7.0);
        assert!(contrast_ratio(dark.on_accent, dark.accent) >= 7.0);
        assert!(contrast_ratio(dark.header_active_fg, dark.header_active_bg) >= 4.5);
    }

    #[test]
    fn cyan_scale_and_header_hierarchy_meet_contrast_targets() {
        let light = Tokens::for_theme(UiTheme::Light);
        let dark = Tokens::for_theme(UiTheme::Dark);
        assert_light_cyan_contrast(&light);
        assert_dark_cyan_contrast(&dark);
    }

    #[test]
    fn theme_switching_updates_egui_visuals_and_renders_home_editor_and_modal() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        assert_eq!(app.session.prefs.theme, UiTheme::Dark);

        // Dark Home frame (initial default).
        let home_dark_init = run_app_frame(&ctx, &mut app);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        assert!(ctx.style_of(ctx.theme()).visuals.dark_mode);
        assert!(home_dark_init.contains("New Notebook"));

        // Switch to Light via engine command and verify Home updates.
        assert!(
            app.session
                .execute("theme.set", &serde_json::json!({ "theme": "Light" }))
                .is_ok()
        );
        assert_eq!(app.session.prefs.theme, UiTheme::Light);
        let home_light = run_app_frame(&ctx, &mut app);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        assert!(!ctx.style_of(ctx.theme()).visuals.dark_mode);
        assert!(home_light.contains("New Notebook"));

        // Enter Editor on Appearance tab and switch to Dark mode.
        assert!(
            app.session
                .execute("theme.set", &serde_json::json!({ "theme": "Dark" }))
                .is_ok()
        );
        open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = motion::ScreenTransition::new(AppScreen::Editor);
        app.view.inspector_open = true;
        app.view.settings_tab = SettingsTab::Appearance;
        let _ = run_app_frame(&ctx, &mut app);
        let editor_dark = run_app_frame(&ctx, &mut app);
        assert!(editor_dark.contains("Appearance"));
        assert!(editor_dark.contains("Dark"));
        assert!(editor_dark.contains("Light"));

        // Open Export modal in Dark mode, then toggle back to Light and re-render cleanly.
        modal::open(&mut app, modal::ActiveModal::Export);
        let _ = run_app_frame(&ctx, &mut app);
        let modal_dark = run_app_frame(&ctx, &mut app);
        assert!(modal_dark.contains("Export Notebook"));

        apply_visuals(&ctx, UiTheme::Light);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        assert!(
            app.session
                .execute("theme.set", &serde_json::json!({ "theme": "Light" }))
                .is_ok()
        );
        let modal_light = run_app_frame(&ctx, &mut app);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        assert!(!ctx.style_of(ctx.theme()).visuals.dark_mode);
        assert!(modal_light.contains("Export Notebook"));
    }

    #[test]
    fn apply_visuals_disables_ui_label_text_selection_in_light_and_dark() {
        let ctx = egui::Context::default();
        assert!(
            ctx.style_of(egui::Theme::Light)
                .interaction
                .selectable_labels
        );
        assert!(
            ctx.style_of(egui::Theme::Dark)
                .interaction
                .selectable_labels
        );

        apply_visuals(&ctx, UiTheme::Light);
        assert!(
            !ctx.style_of(egui::Theme::Light)
                .interaction
                .selectable_labels
        );
        assert!(
            !ctx.style_of(egui::Theme::Dark)
                .interaction
                .selectable_labels
        );

        apply_visuals(&ctx, UiTheme::Dark);
        assert!(
            !ctx.style_of(egui::Theme::Light)
                .interaction
                .selectable_labels
        );
        assert!(
            !ctx.style_of(egui::Theme::Dark)
                .interaction
                .selectable_labels
        );
    }

    #[test]
    fn infinite_theme_identity_and_shifts_apply_to_light_and_dark_preserving_contrast() {
        use papercraft_doc::InfiniteThemeConfig;

        for theme in [UiTheme::Light, UiTheme::Dark] {
            let base = Tokens::for_theme(theme);
            let disabled = Tokens::for_theme_with_infinite(
                theme,
                InfiniteThemeConfig {
                    enabled: false,
                    hue_degrees: 140.0,
                    saturation_percent: 200.0,
                },
            );
            assert_eq!(disabled.panel_bg, base.panel_bg);
            assert_eq!(disabled.accent, base.accent);
            assert_eq!(disabled.header_bg, base.header_bg);

            let identity = Tokens::for_theme_with_infinite(
                theme,
                InfiniteThemeConfig {
                    enabled: true,
                    hue_degrees: 0.0,
                    saturation_percent: 100.0,
                },
            );
            assert_eq!(identity.accent, base.accent);
            assert_eq!(identity.brand_craft, base.brand_craft);

            let shifted = Tokens::for_theme_with_infinite(
                theme,
                InfiniteThemeConfig {
                    enabled: true,
                    hue_degrees: 150.0,
                    saturation_percent: 180.0,
                },
            );
            assert_eq!(shifted.is_dark, base.is_dark);
            assert_ne!(shifted.accent, base.accent);
            assert_ne!(shifted.brand_craft, base.brand_craft);
            // Semantic danger/success hues stay intact when only hue rotates.
            assert_eq!(shifted.danger_fg, base.danger_fg);
            assert_eq!(shifted.success_fg, base.success_fg);
            assert!(
                contrast_ratio(shifted.text, shifted.panel_bg) >= 7.0,
                "expected high text contrast in {theme:?} shifted theme"
            );
            assert!(
                contrast_ratio(shifted.on_accent, shifted.accent) >= 4.0,
                "expected clear accent button contrast in {theme:?} shifted theme"
            );

            // 0% saturation produces clean monochrome neutral RGB across UI tokens.
            let mono = Tokens::for_theme_with_infinite(
                theme,
                InfiniteThemeConfig {
                    enabled: true,
                    hue_degrees: 90.0,
                    saturation_percent: 0.0,
                },
            );
            assert_eq!(mono.accent.r(), mono.accent.g());
            assert_eq!(mono.accent.g(), mono.accent.b());
            assert_eq!(mono.panel_bg.r(), mono.panel_bg.g());
            assert_eq!(mono.danger_fg.r(), mono.danger_fg.g());
        }
    }

    #[test]
    fn apply_prefs_visuals_updates_egui_visuals_when_infinite_theme_changes() {
        use papercraft_doc::{InfiniteThemeConfig, UserStudioConfig};

        let ctx = egui::Context::default();
        let mut prefs = UserStudioConfig {
            theme: UiTheme::Dark,
            ..UserStudioConfig::default()
        };
        super::apply_prefs_visuals(&ctx, &prefs);
        let base_accent = ctx
            .style_of(egui::Theme::Dark)
            .visuals
            .selection
            .stroke
            .color;
        assert_eq!(base_accent, Tokens::dark().accent);

        prefs.infinite_theme = InfiniteThemeConfig {
            enabled: true,
            hue_degrees: 135.0,
            saturation_percent: 175.0,
        };
        super::apply_prefs_visuals(&ctx, &prefs);
        let expected = Tokens::for_prefs(&prefs);
        let style = ctx.style_of(egui::Theme::Dark);
        assert_eq!(style.visuals.selection.stroke.color, expected.accent);
        assert_eq!(style.visuals.selection.bg_fill, expected.accent_soft);
        assert_eq!(style.visuals.panel_fill, expected.panel_bg);
        assert_ne!(style.visuals.selection.stroke.color, base_accent);
    }
}
