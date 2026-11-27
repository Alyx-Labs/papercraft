//! UI design tokens and Light/Dark theme visual styling for PaperCraft.

use egui::Color32;
use papercraft_color::{
    Color, adjust_oklch_lightness_for_contrast, contrast_ratio, oklch_lightness,
    relative_luminance, with_oklch_lightness,
};
use papercraft_doc::{InfiniteThemeConfig, UiTheme, UserStudioConfig};

#[inline]
pub fn to_color32(c: Color) -> Color32 {
    let [r, g, b, a] = c.to_rgba8();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

#[inline]
pub(crate) fn to_doc_color(c: Color32) -> Color {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Color::from_rgba8(r, g, b, a)
}

#[inline]
pub(crate) fn wcag_contrast(a: Color32, b: Color32) -> f32 {
    contrast_ratio(to_doc_color(a), to_doc_color(b))
}

/// Adjusts `fg` along its perceptual OKLCH lightness so it achieves at least `min_ratio` WCAG
/// contrast against every color in `bgs` (or the highest achievable contrast).
pub(crate) fn ensure_contrast_against(
    fg: Color32,
    bgs: &[Color32],
    min_ratio: f32,
    prefer_lighter: Option<bool>,
) -> Color32 {
    if bgs.iter().all(|&bg| wcag_contrast(fg, bg) >= min_ratio) {
        return fg;
    }
    let bg_docs: Vec<Color> = bgs.iter().copied().map(to_doc_color).collect();
    to_color32(adjust_oklch_lightness_for_contrast(
        to_doc_color(fg),
        &bg_docs,
        min_ratio + 0.08,
        prefer_lighter,
    ))
}

#[inline]
fn with_oklch_l(c: Color32, target_ok_l: f32) -> Color32 {
    to_color32(with_oklch_lightness(to_doc_color(c), target_ok_l))
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
    /// enforcing perceptual WCAG contrast across toolbar, tab strip, header, and body controls
    /// (especially keeping Light-theme toolbar icons and text crisp on any hue/saturation).
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

        let mut out = Self {
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
        };
        out.enforce_infinite_contrast();
        out
    }

    fn enforce_infinite_contrast(&mut self) {
        if !self.is_dark {
            self.enforce_light_infinite_contrast();
            return;
        }
        let hdr_doc = to_doc_color(self.header_bg);
        let hdr_lum = relative_luminance(hdr_doc);
        let hdr_ok_l = oklch_lightness(hdr_doc);
        let hdr_fg_probe = adjust_oklch_lightness_for_contrast(
            to_doc_color(self.header_fg),
            &[hdr_doc],
            6.0,
            None,
        );
        let header_fg_is_light = relative_luminance(hdr_fg_probe) > hdr_lum;

        if header_fg_is_light {
            let track_lum = relative_luminance(to_doc_color(self.header_track_bg));
            if track_lum > hdr_lum + 0.03 || track_lum + 0.03 < hdr_lum {
                self.header_track_bg =
                    with_oklch_l(self.header_track_bg, (hdr_ok_l + 0.02).clamp(0.0, 0.44));
            }
            self.header_track_bg =
                ensure_contrast_against(self.header_track_bg, &[Color32::WHITE], 4.8, Some(false));

            let hover_ok_l = oklch_lightness(to_doc_color(self.header_control_hover));
            let hover_lum = relative_luminance(to_doc_color(self.header_control_hover));
            if hover_lum > hdr_lum + 0.06 || hover_ok_l < hdr_ok_l + 0.04 {
                self.header_control_hover = with_oklch_l(
                    self.header_control_hover,
                    (hdr_ok_l + 0.07).clamp(0.0, 0.48),
                );
            }
            self.header_control_hover = ensure_contrast_against(
                self.header_control_hover,
                &[Color32::WHITE],
                4.8,
                Some(false),
            );

            if wcag_contrast(self.header_border, self.header_bg) < 1.28
                || relative_luminance(to_doc_color(self.header_border)) < hdr_lum
            {
                self.header_border = ensure_contrast_against(
                    with_oklch_l(self.header_border, (hdr_ok_l + 0.11).clamp(0.0, 0.70)),
                    &[self.header_bg],
                    1.32,
                    Some(true),
                );
            }
        } else {
            if oklch_lightness(to_doc_color(self.header_track_bg)) < hdr_ok_l {
                self.header_track_bg =
                    with_oklch_l(self.header_track_bg, (hdr_ok_l + 0.05).min(0.96));
            }
            if oklch_lightness(to_doc_color(self.header_control_hover)) < hdr_ok_l + 0.04 {
                self.header_control_hover =
                    with_oklch_l(self.header_control_hover, (hdr_ok_l + 0.10).min(0.98));
            }
            self.header_border =
                ensure_contrast_against(self.header_border, &[self.header_bg], 1.32, Some(false));
        }

        let header_bgs = [
            self.header_bg,
            self.header_track_bg,
            self.header_control_hover,
        ];
        self.header_fg =
            ensure_contrast_against(self.header_fg, &header_bgs, 6.0, Some(header_fg_is_light));
        self.header_fg_muted = ensure_contrast_against(
            self.header_fg_muted,
            &header_bgs,
            4.5,
            Some(header_fg_is_light),
        );
        self.brand_craft = ensure_contrast_against(
            self.brand_craft,
            &[self.header_bg],
            4.5,
            Some(header_fg_is_light),
        );
        let min_active_ratio = if self.is_dark { 5.5 } else { 7.0 };
        self.header_active_fg = ensure_contrast_against(
            self.header_active_fg,
            &[self.header_active_bg],
            min_active_ratio,
            None,
        );

        let tab_probe = ensure_contrast_against(self.tab_strip_fg, &[self.tab_strip_bg], 7.0, None);
        let tab_fg_is_light = relative_luminance(to_doc_color(tab_probe))
            > relative_luminance(to_doc_color(self.tab_strip_bg));
        self.tab_strip_fg = tab_probe;
        self.tab_strip_fg_muted = ensure_contrast_against(
            self.tab_strip_fg_muted,
            &[self.tab_strip_bg],
            4.5,
            Some(tab_fg_is_light),
        );

        let body_bgs = [
            self.panel_bg,
            self.toolbar_bg,
            self.field_bg,
            self.accent_soft,
        ];
        let accent_bgs = [
            self.panel_bg,
            self.toolbar_bg,
            self.field_bg,
            self.accent_soft,
            self.header_active_bg,
        ];
        let body_fg_is_light = self.is_dark;
        self.text = ensure_contrast_against(self.text, &body_bgs, 7.0, Some(body_fg_is_light));
        self.text_muted =
            ensure_contrast_against(self.text_muted, &body_bgs, 4.5, Some(body_fg_is_light));
        self.accent =
            ensure_contrast_against(self.accent, &accent_bgs, 4.5, Some(body_fg_is_light));
        self.accent_hover =
            ensure_contrast_against(self.accent_hover, &body_bgs, 4.5, Some(body_fg_is_light));
        self.on_accent = ensure_contrast_against(self.on_accent, &[self.accent], 4.5, None);
        self.border = ensure_contrast_against(
            self.border,
            &[self.panel_bg, self.field_bg],
            1.28,
            Some(body_fg_is_light),
        );
    }

    fn enforce_light_infinite_contrast(&mut self) {
        const LIGHT_CHROME_WHITE: Color32 = Color32::WHITE;
        const LIGHT_CHROME_WHITE_MUTED: Color32 = Color32::from_rgb(0xEC, 0xEE, 0xF2);
        const LIGHT_CHROME_WHITE_CRAFT: Color32 = Color32::from_rgb(0xF0, 0xF2, 0xF6);
        const LIGHT_CHROME_DARK_GRAY: Color32 = Color32::from_rgb(0x24, 0x25, 0x28);
        const LIGHT_CHROME_DARK_GRAY_MUTED: Color32 = Color32::from_rgb(0x34, 0x36, 0x3A);
        const LIGHT_CHROME_DARK_GRAY_CRAFT: Color32 = Color32::from_rgb(0x2C, 0x2D, 0x31);

        let hdr_doc = to_doc_color(self.header_bg);
        let hdr_lum = relative_luminance(hdr_doc);
        let hdr_ok_l = oklch_lightness(hdr_doc);
        // Prefer pure crisp white whenever it achieves AA (>= 4.5:1) against `header_bg`;
        // otherwise adapt to clean dark charcoal gray (never muddy hue-tinted near-black).
        let header_fg_is_light = wcag_contrast(LIGHT_CHROME_WHITE, self.header_bg) >= 4.5;

        if header_fg_is_light {
            let track_lum = relative_luminance(to_doc_color(self.header_track_bg));
            if track_lum > hdr_lum + 0.03 || track_lum + 0.03 < hdr_lum {
                self.header_track_bg =
                    with_oklch_l(self.header_track_bg, (hdr_ok_l + 0.02).clamp(0.0, 0.44));
            }
            self.header_track_bg =
                ensure_contrast_against(self.header_track_bg, &[Color32::WHITE], 4.8, Some(false));

            let hover_ok_l = oklch_lightness(to_doc_color(self.header_control_hover));
            let hover_lum = relative_luminance(to_doc_color(self.header_control_hover));
            if hover_lum > hdr_lum + 0.06 || hover_ok_l < hdr_ok_l + 0.04 {
                self.header_control_hover = with_oklch_l(
                    self.header_control_hover,
                    (hdr_ok_l + 0.07).clamp(0.0, 0.48),
                );
            }
            self.header_control_hover = ensure_contrast_against(
                self.header_control_hover,
                &[Color32::WHITE],
                4.8,
                Some(false),
            );

            if wcag_contrast(self.header_border, self.header_bg) < 1.28
                || relative_luminance(to_doc_color(self.header_border)) < hdr_lum
            {
                self.header_border = ensure_contrast_against(
                    with_oklch_l(self.header_border, (hdr_ok_l + 0.11).clamp(0.0, 0.70)),
                    &[self.header_bg],
                    1.32,
                    Some(true),
                );
            }
        } else {
            if oklch_lightness(to_doc_color(self.header_track_bg)) < hdr_ok_l {
                self.header_track_bg =
                    with_oklch_l(self.header_track_bg, (hdr_ok_l + 0.05).min(0.96));
            }
            if oklch_lightness(to_doc_color(self.header_control_hover)) < hdr_ok_l + 0.04 {
                self.header_control_hover =
                    with_oklch_l(self.header_control_hover, (hdr_ok_l + 0.10).min(0.98));
            }
            self.header_border =
                ensure_contrast_against(self.header_border, &[self.header_bg], 1.32, Some(false));
        }

        let header_bgs = [
            self.header_bg,
            self.header_track_bg,
            self.header_control_hover,
        ];
        if header_fg_is_light {
            self.header_fg = LIGHT_CHROME_WHITE;
            self.header_fg_muted =
                ensure_contrast_against(LIGHT_CHROME_WHITE_MUTED, &header_bgs, 4.7, Some(true));
            self.brand_craft = ensure_contrast_against(
                LIGHT_CHROME_WHITE_CRAFT,
                &[self.header_bg],
                4.7,
                Some(true),
            );
        } else {
            self.header_fg =
                ensure_contrast_against(LIGHT_CHROME_DARK_GRAY, &header_bgs, 4.8, Some(false));
            self.header_fg_muted = ensure_contrast_against(
                LIGHT_CHROME_DARK_GRAY_MUTED,
                &header_bgs,
                4.6,
                Some(false),
            );
            self.brand_craft = ensure_contrast_against(
                LIGHT_CHROME_DARK_GRAY_CRAFT,
                &[self.header_bg],
                4.6,
                Some(false),
            );
        }

        let active_fg_is_light = wcag_contrast(LIGHT_CHROME_WHITE, self.header_active_bg) >= 4.5;
        self.header_active_fg = if active_fg_is_light {
            LIGHT_CHROME_WHITE
        } else {
            ensure_contrast_against(
                LIGHT_CHROME_DARK_GRAY,
                &[self.header_active_bg],
                7.0,
                Some(false),
            )
        };

        let tab_fg_is_light = wcag_contrast(LIGHT_CHROME_WHITE, self.tab_strip_bg) >= 4.5;
        if tab_fg_is_light {
            self.tab_strip_fg = LIGHT_CHROME_WHITE;
            self.tab_strip_fg_muted = ensure_contrast_against(
                LIGHT_CHROME_WHITE_MUTED,
                &[self.tab_strip_bg],
                4.7,
                Some(true),
            );
        } else {
            self.tab_strip_fg = ensure_contrast_against(
                LIGHT_CHROME_DARK_GRAY,
                &[self.tab_strip_bg],
                4.8,
                Some(false),
            );
            self.tab_strip_fg_muted = ensure_contrast_against(
                LIGHT_CHROME_DARK_GRAY_MUTED,
                &[self.tab_strip_bg],
                4.6,
                Some(false),
            );
        }

        let body_bgs = [
            self.panel_bg,
            self.toolbar_bg,
            self.field_bg,
            self.accent_soft,
        ];
        let accent_bgs = [
            self.panel_bg,
            self.toolbar_bg,
            self.field_bg,
            self.accent_soft,
            self.header_active_bg,
        ];
        self.text = ensure_contrast_against(self.text, &body_bgs, 7.0, Some(false));
        self.text_muted = ensure_contrast_against(self.text_muted, &body_bgs, 4.5, Some(false));
        self.accent = ensure_contrast_against(self.accent, &accent_bgs, 4.5, Some(false));
        self.accent_hover = ensure_contrast_against(self.accent_hover, &body_bgs, 4.5, Some(false));
        self.on_accent = if wcag_contrast(LIGHT_CHROME_WHITE, self.accent) >= 4.5 {
            LIGHT_CHROME_WHITE
        } else {
            ensure_contrast_against(LIGHT_CHROME_DARK_GRAY, &[self.accent], 4.5, Some(false))
        };
        self.border = ensure_contrast_against(
            self.border,
            &[self.panel_bg, self.field_bg],
            1.28,
            Some(false),
        );
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

    #[test]
    fn infinite_theme_maintains_toolbar_header_and_accent_contrast_across_all_hues_and_saturations()
    {
        use papercraft_doc::InfiniteThemeConfig;

        for theme in [UiTheme::Light, UiTheme::Dark] {
            for hue_step in 0..=24 {
                let hue = hue_step as f32 * 15.0;
                for sat in [0.0_f32, 50.0, 100.0, 150.0, 200.0, 250.0] {
                    let cfg = InfiniteThemeConfig {
                        enabled: true,
                        hue_degrees: hue,
                        saturation_percent: sat,
                    };
                    if cfg.is_identity() {
                        continue;
                    }
                    let t = Tokens::for_theme_with_infinite(theme, cfg);
                    for bg in [t.header_bg, t.header_track_bg, t.header_control_hover] {
                        let cr_fg = contrast_ratio(t.header_fg, bg);
                        let cr_muted = contrast_ratio(t.header_fg_muted, bg);
                        assert!(
                            cr_fg >= 4.5,
                            "{theme:?} hue={hue} sat={sat}: header_fg contrast {cr_fg:.2} too low"
                        );
                        assert!(
                            cr_muted >= 4.45,
                            "{theme:?} hue={hue} sat={sat}: header_fg_muted contrast {cr_muted:.2} too low"
                        );
                    }
                    let cr_craft = contrast_ratio(t.brand_craft, t.header_bg);
                    assert!(
                        cr_craft >= 4.45,
                        "{theme:?} hue={hue} sat={sat}: brand_craft contrast {cr_craft:.2} too low"
                    );
                    let min_active_cr = if theme == UiTheme::Dark { 5.5 } else { 7.0 };
                    assert!(
                        contrast_ratio(t.header_active_fg, t.header_active_bg) >= min_active_cr,
                        "{theme:?} hue={hue} sat={sat}: header_active_fg contrast too low"
                    );
                    assert!(
                        contrast_ratio(t.tab_strip_fg, t.tab_strip_bg) >= 4.5,
                        "{theme:?} hue={hue} sat={sat}: tab_strip_fg contrast too low"
                    );
                    assert!(
                        contrast_ratio(t.tab_strip_fg_muted, t.tab_strip_bg) >= 4.45,
                        "{theme:?} hue={hue} sat={sat}: tab_strip_fg_muted contrast too low"
                    );
                    for bg in [t.panel_bg, t.toolbar_bg, t.field_bg, t.accent_soft] {
                        assert!(
                            contrast_ratio(t.text, bg) >= 7.0,
                            "{theme:?} hue={hue} sat={sat}: text contrast too low"
                        );
                        assert!(
                            contrast_ratio(t.text_muted, bg) >= 4.45,
                            "{theme:?} hue={hue} sat={sat}: text_muted contrast too low"
                        );
                        assert!(
                            contrast_ratio(t.accent, bg) >= 4.45,
                            "{theme:?} hue={hue} sat={sat}: accent contrast too low"
                        );
                    }
                    assert!(
                        contrast_ratio(t.accent, t.header_active_bg) >= 4.45,
                        "{theme:?} hue={hue} sat={sat}: accent vs header_active_bg contrast too low"
                    );
                    assert!(
                        contrast_ratio(t.on_accent, t.accent) >= 4.45,
                        "{theme:?} hue={hue} sat={sat}: on_accent vs accent contrast too low"
                    );
                }
            }
        }
    }

    #[test]
    fn light_infinite_theme_adapts_chrome_foregrounds_to_crisp_white_or_neutral_dark_gray() {
        use papercraft_doc::InfiniteThemeConfig;

        let is_neutral_gray_or_white = |c: Color32| {
            let max_ch = c.r().max(c.g()).max(c.b());
            let min_ch = c.r().min(c.g()).min(c.b());
            max_ch.saturating_sub(min_ch) <= 10
        };

        let mut saw_white_accent = false;
        let mut saw_dark_gray_header = false;
        for hue_step in 0..=24 {
            let hue = hue_step as f32 * 15.0;
            for sat in [0.0_f32, 50.0, 100.0, 150.0, 200.0, 250.0] {
                let cfg = InfiniteThemeConfig {
                    enabled: true,
                    hue_degrees: hue,
                    saturation_percent: sat,
                };
                if cfg.is_identity() {
                    continue;
                }
                let t = Tokens::for_theme_with_infinite(UiTheme::Light, cfg);
                assert!(
                    is_neutral_gray_or_white(t.header_fg),
                    "expected neutral white/dark-gray header_fg at hue={hue} sat={sat}, got {:?}",
                    t.header_fg
                );
                assert!(
                    is_neutral_gray_or_white(t.header_fg_muted),
                    "expected neutral white/dark-gray header_fg_muted at hue={hue} sat={sat}, got {:?}",
                    t.header_fg_muted
                );
                assert!(
                    is_neutral_gray_or_white(t.tab_strip_fg),
                    "expected neutral white/dark-gray tab_strip_fg at hue={hue} sat={sat}, got {:?}",
                    t.tab_strip_fg
                );
                assert!(
                    is_neutral_gray_or_white(t.tab_strip_fg_muted),
                    "expected neutral white/dark-gray tab_strip_fg_muted at hue={hue} sat={sat}, got {:?}",
                    t.tab_strip_fg_muted
                );
                assert!(
                    is_neutral_gray_or_white(t.brand_craft),
                    "expected neutral white/dark-gray brand_craft at hue={hue} sat={sat}, got {:?}",
                    t.brand_craft
                );
                assert!(
                    is_neutral_gray_or_white(t.on_accent),
                    "expected neutral white/dark-gray on_accent at hue={hue} sat={sat}, got {:?}",
                    t.on_accent
                );
                if t.on_accent == Color32::WHITE {
                    saw_white_accent = true;
                }
                if t.header_fg != Color32::WHITE {
                    saw_dark_gray_header = true;
                    assert!(
                        t.header_fg.r() <= 0x30,
                        "expected dark gray header_fg at hue={hue} sat={sat}, got {:?}",
                        t.header_fg
                    );
                }
            }
        }
        assert!(
            saw_white_accent && saw_dark_gray_header,
            "expected Light Infinite sweep to exercise white on_accent ({saw_white_accent}) and dark-gray header_fg ({saw_dark_gray_header})"
        );

        // When Light chrome bars reach darker luminance (<= ~0.183), the contrast solver
        // switches header and tab strip text/icons cleanly to pure crisp white.
        let mut dark_header_light_mode = Tokens::light();
        dark_header_light_mode.header_bg = Color32::from_rgb(0x2B, 0x32, 0x8F);
        dark_header_light_mode.tab_strip_bg = Color32::from_rgb(0x1E, 0x26, 0x6E);
        dark_header_light_mode.enforce_infinite_contrast();
        assert_eq!(dark_header_light_mode.header_fg, Color32::WHITE);
        assert_eq!(dark_header_light_mode.tab_strip_fg, Color32::WHITE);
        assert!(
            contrast_ratio(
                dark_header_light_mode.header_fg_muted,
                dark_header_light_mode.header_bg
            ) >= 4.5
        );
        assert!(
            contrast_ratio(
                dark_header_light_mode.brand_craft,
                dark_header_light_mode.header_bg
            ) >= 4.5
        );
        assert!(
            contrast_ratio(
                dark_header_light_mode.tab_strip_fg_muted,
                dark_header_light_mode.tab_strip_bg
            ) >= 4.5
        );
    }
}
