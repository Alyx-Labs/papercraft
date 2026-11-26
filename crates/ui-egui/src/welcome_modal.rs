//! First-launch Welcome Banner modal introducing PaperCraft's core study workflow.

use egui::{
    Color32, CornerRadius, CursorIcon, FontFamily, FontId, Margin, Rect, RichText, Sense, Stroke,
    Vec2, pos2,
};

use crate::icons::{Icon, paint_icon};
use crate::modal::{ActiveModal, ModalMetrics};
use crate::{BRAND_DISPLAY_FAMILY, PaperCraftApp};

const WELCOME_SEA_PNG: &[u8] = include_bytes!("../assets/illustrations/welcome-sea.png");

const TOP_BG: Color32 = Color32::from_rgb(0xD4, 0xED, 0xFC);
const TOP_TITLE: Color32 = Color32::from_rgb(0x0D, 0x43, 0x63);
const TOP_CLOSE_HOVER: Color32 = Color32::from_rgb(0xBF, 0xE2, 0xF6);
const BOTTOM_BG: Color32 = Color32::from_rgb(0x27, 0x2B, 0x32);
const BOTTOM_TEXT: Color32 = Color32::from_rgb(0xEF, 0xF2, 0xF7);
const BADGE_BG: Color32 = Color32::from_rgb(0xF8, 0xFB, 0xFD);
const CTA_BG: Color32 = Color32::from_rgb(0x0C, 0x6F, 0xA6);
const CTA_HOVER: Color32 = Color32::from_rgb(0x0E, 0x80, 0xBF);

const CARD_RADIUS: u8 = 22;
const PREFERRED_WIDTH: f32 = 700.0;

#[derive(Clone, Copy)]
pub(crate) struct IllustratedModalTheme {
    pub(crate) top_bg: Color32,
    pub(crate) top_title: Color32,
    pub(crate) top_close_hover: Color32,
    pub(crate) bottom_bg: Color32,
    pub(crate) bottom_text: Color32,
    pub(crate) badge_bg: Color32,
    pub(crate) badge_icon: Color32,
    pub(crate) cta_bg: Color32,
    pub(crate) cta_hover: Color32,
    pub(crate) cta_focus_ring: Color32,
}

#[derive(Clone, Copy)]
pub(crate) struct FeatureItem {
    pub(crate) icon: Icon,
    pub(crate) label: &'static str,
}

pub(crate) struct IllustratedModalSpec {
    pub(crate) modal: ActiveModal,
    pub(crate) title: &'static str,
    pub(crate) body_heading: &'static str,
    pub(crate) cta_label: &'static str,
    pub(crate) illustration_cache_key: &'static str,
    pub(crate) illustration_png: &'static [u8],
    pub(crate) fallback_icon: Icon,
    pub(crate) features: [FeatureItem; 4],
    pub(crate) theme: IllustratedModalTheme,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IllustratedModalAction {
    Close,
    Cta,
}

const FEATURES: [FeatureItem; 4] = [
    FeatureItem {
        icon: Icon::Notebook,
        label: "Load huge PDFs smoothly\nfor daily study",
    },
    FeatureItem {
        icon: Icon::FountainPen,
        label: "Natural stylus ink with\nper-pen smoothing",
    },
    FeatureItem {
        icon: Icon::Export,
        label: "Clean PDF, PNG & SVG\nexport, no watermarks",
    },
    FeatureItem {
        icon: Icon::Sync,
        label: "100% offline-first with\nprivate self-hosted sync",
    },
];

const WELCOME_SPEC: IllustratedModalSpec = IllustratedModalSpec {
    modal: ActiveModal::WelcomeBanner,
    title: "Study deeper with PaperCraft",
    body_heading: "Built for calm, distraction-free daily study:",
    cta_label: "Start Studying",
    illustration_cache_key: "papercraft-welcome-sea-illustration",
    illustration_png: WELCOME_SEA_PNG,
    fallback_icon: Icon::Notebook,
    features: FEATURES,
    theme: IllustratedModalTheme {
        top_bg: TOP_BG,
        top_title: TOP_TITLE,
        top_close_hover: TOP_CLOSE_HOVER,
        bottom_bg: BOTTOM_BG,
        bottom_text: BOTTOM_TEXT,
        badge_bg: BADGE_BG,
        badge_icon: TOP_TITLE,
        cta_bg: CTA_BG,
        cta_hover: CTA_HOVER,
        cta_focus_ring: Color32::WHITE,
    },
};

#[derive(Clone)]
enum CachedWelcomeArt {
    Ready(egui::TextureHandle),
    Failed,
}

fn load_illustration_texture(
    ctx: &egui::Context,
    cache_key: &'static str,
    png_bytes: &'static [u8],
) -> Option<egui::TextureHandle> {
    let cache_id = egui::Id::unique(cache_key);
    if let Some(cached) = ctx.data_mut(|d| d.get_temp::<CachedWelcomeArt>(cache_id)) {
        return match cached {
            CachedWelcomeArt::Ready(tex) => Some(tex),
            CachedWelcomeArt::Failed => None,
        };
    }
    let decoded = match image::load_from_memory(png_bytes) {
        Ok(img) => img.into_rgba8(),
        Err(err) => {
            log::warn!("Could not decode illustration PNG `{cache_key}`: {err}");
            ctx.data_mut(|d| d.insert_temp(cache_id, CachedWelcomeArt::Failed));
            return None;
        }
    };
    let size = [decoded.width() as usize, decoded.height() as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
    let tex = ctx.load_texture(cache_key, color_image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(cache_id, CachedWelcomeArt::Ready(tex.clone())));
    Some(tex)
}

/// Marks the welcome banner as seen and advances to initial nickname setup if required.
pub(crate) fn dismiss(app: &mut PaperCraftApp) {
    let _ = app
        .session
        .execute("profile.dismissWelcomeBanner", &serde_json::json!({}));
    if app.session.needs_initial_nickname() {
        app.view.first_access_nickname.clear();
        app.view.first_access_error = None;
        app.view.first_access_focus_pending = true;
        crate::modal::open(app, ActiveModal::FirstAccessNickname);
    } else {
        crate::modal::close(app);
    }
}

/// Renders the two-tone first-launch welcome banner modal.
pub(crate) fn show(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    eased: f32,
    backdrop_eased: f32,
    interactive: bool,
) -> ModalMetrics {
    let (metrics, action) =
        show_illustrated_modal(app, ui, &WELCOME_SPEC, eased, backdrop_eased, interactive);
    if interactive && action.is_some() {
        dismiss(app);
    }
    metrics
}

/// Renders a two-tone illustrated onboarding modal and returns which action button was clicked.
pub(crate) fn show_illustrated_modal(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    spec: &IllustratedModalSpec,
    eased: f32,
    backdrop_eased: f32,
    interactive: bool,
) -> (ModalMetrics, Option<IllustratedModalAction>) {
    app.install_rich_fonts_if_needed(ui.ctx());
    let ctx = ui.ctx().clone();
    let viewport = ctx.content_rect();
    let dialog_width = PREFERRED_WIDTH.min((viewport.width() - 32.0).max(300.0));
    let is_dark_theme = app.session.prefs.theme == papercraft_doc::UiTheme::Dark;

    let backdrop_id = ui.make_persistent_id("papercraft_modal_backdrop");
    let modal_id = ui.make_persistent_id(("papercraft_modal_surface", spec.modal.id_tag()));

    let _backdrop_resp = egui::Area::new(backdrop_id)
        .order(egui::Order::Middle)
        .fixed_pos(viewport.min)
        .fade_in(false)
        .interactable(interactive)
        .show(&ctx, |ui| {
            let (_rect, resp) = ui.allocate_exact_size(viewport.size(), Sense::click());
            let backdrop_alpha = if is_dark_theme { 155 } else { 96 };
            ui.painter().rect_filled(
                viewport,
                0.0,
                Color32::from_black_alpha(backdrop_alpha).gamma_multiply(backdrop_eased),
            );
            resp
        })
        .inner;

    let mut action = None;
    let mut close_btn_rect = Rect::NOTHING;
    let mut body_rect = Rect::NOTHING;
    let mut footer_rect = Rect::NOTHING;

    let shadow = ctx.style_of(ctx.theme()).visuals.window_shadow;
    let settle_offset = egui::vec2(0.0, (1.0 - eased) * crate::motion::MODAL_SETTLE_OFFSET_Y);

    let modal_resp = egui::Area::new(modal_id)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, settle_offset)
        .default_size(egui::vec2(
            dialog_width,
            540.0_f32.min((viewport.height() - 32.0).max(260.0)),
        ))
        .constrain(true)
        .fade_in(false)
        .interactable(interactive)
        .show(&ctx, |ui| {
            ui.multiply_opacity(eased);
            egui::Frame::NONE
                .corner_radius(CornerRadius::same(CARD_RADIUS))
                .shadow(shadow)
                .show(ui, |ui| {
                    ui.set_min_width(dialog_width);
                    ui.set_max_width(dialog_width);
                    ui.spacing_mut().item_spacing = Vec2::ZERO;

                    let (close_clicked, x_rect) =
                        draw_top_header(ui, spec, dialog_width, viewport.height());
                    close_btn_rect = x_rect;
                    if close_clicked {
                        action = Some(IllustratedModalAction::Close);
                    }

                    let (cta_clicked, b_rect, f_rect) = draw_bottom_section(ui, spec, dialog_width);
                    body_rect = b_rect;
                    footer_rect = f_rect;
                    if cta_clicked {
                        action = Some(IllustratedModalAction::Cta);
                    }
                });
        });

    let modal_rect = modal_resp.response.rect;
    // Outside clicks are absorbed by the backdrop and must not dismiss illustrated onboarding modals;
    // only an explicit modal button click closes them.
    let metrics = ModalMetrics {
        modal_rect,
        close_btn_rect,
        body_rect,
        footer_rect,
        inner_margin: 0.0,
    };
    (metrics, action)
}

fn draw_top_header(
    ui: &mut egui::Ui,
    spec: &IllustratedModalSpec,
    width: f32,
    viewport_h: f32,
) -> (bool, Rect) {
    let mut close_clicked = false;
    let mut close_rect = Rect::NOTHING;
    let top_pad: i8 = if viewport_h < 600.0 { 18 } else { 26 };
    let bottom_pad: i8 = if viewport_h < 600.0 { 12 } else { 18 };
    let side_pad: i8 = 24;
    let inner_w = (width - f32::from(side_pad) * 2.0).max(200.0);
    let art_w = if viewport_h < 600.0 { 164.0 } else { 224.0 };
    let theme = &spec.theme;

    egui::Frame::NONE
        .fill(theme.top_bg)
        .corner_radius(CornerRadius {
            nw: CARD_RADIUS,
            ne: CARD_RADIUS,
            sw: 0,
            se: 0,
        })
        .inner_margin(Margin {
            left: side_pad,
            right: side_pad,
            top: top_pad,
            bottom: bottom_pad,
        })
        .show(ui, |ui| {
            ui.set_min_width(inner_w);
            ui.set_max_width(inner_w);

            ui.horizontal(|ui| {
                let close_w = 28.0_f32;
                let title_w = (inner_w - close_w * 2.0).max(120.0);
                let _ = ui.allocate_exact_size(Vec2::splat(close_w), Sense::hover());
                ui.allocate_ui_with_layout(
                    Vec2::new(title_w, 0.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(spec.title)
                                    .font(FontId::new(
                                        25.0,
                                        FontFamily::Name(BRAND_DISPLAY_FAMILY.into()),
                                    ))
                                    .color(theme.top_title),
                            )
                            .selectable(false)
                            .wrap(),
                        );
                    },
                );
                let (btn_rect, resp) = ui.allocate_exact_size(Vec2::splat(close_w), Sense::click());
                close_rect = btn_rect;
                if resp.hovered() {
                    ui.painter().circle_filled(
                        btn_rect.center(),
                        close_w * 0.5,
                        theme.top_close_hover,
                    );
                }
                paint_icon(
                    ui.painter(),
                    btn_rect.shrink(6.0),
                    Icon::Close,
                    theme.top_title,
                );
                let resp = resp
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text("Close");
                if resp.clicked() {
                    close_clicked = true;
                }
            });

            ui.add_space(if viewport_h < 600.0 { 4.0 } else { 8.0 });
            draw_illustration(ui, spec, inner_w, art_w);
        });

    (close_clicked, close_rect)
}

fn draw_illustration(ui: &mut egui::Ui, spec: &IllustratedModalSpec, inner_w: f32, art_w: f32) {
    let clamped_w = art_w.min(inner_w);
    if let Some(tex) =
        load_illustration_texture(ui.ctx(), spec.illustration_cache_key, spec.illustration_png)
    {
        let tex_size = tex.size_vec2();
        let aspect = if tex_size.x > 0.0 {
            tex_size.y / tex_size.x
        } else {
            451.0 / 520.0
        };
        let draw_size = Vec2::new(clamped_w, clamped_w * aspect);
        let (art_rect, _) = ui.allocate_exact_size(Vec2::new(inner_w, draw_size.y), Sense::hover());
        let centered_rect = Rect::from_center_size(art_rect.center(), draw_size);
        egui::Image::new(egui::load::SizedTexture::new(tex.id(), draw_size))
            .paint_at(ui, centered_rect);
    } else {
        let (fallback_rect, _) = ui.allocate_exact_size(Vec2::new(inner_w, 120.0), Sense::hover());
        let badge = Rect::from_center_size(fallback_rect.center(), Vec2::splat(76.0));
        ui.painter()
            .circle_filled(badge.center(), 38.0, spec.theme.badge_bg);
        paint_icon(
            ui.painter(),
            badge.shrink(20.0),
            spec.fallback_icon,
            spec.theme.top_title,
        );
    }
}

fn draw_bottom_section(
    ui: &mut egui::Ui,
    spec: &IllustratedModalSpec,
    width: f32,
) -> (bool, Rect, Rect) {
    let mut cta_clicked = false;
    let mut body_rect = Rect::NOTHING;
    let mut footer_rect = Rect::NOTHING;
    let side_pad_f = if width >= 620.0 { 68.0 } else { 24.0 };
    let side_pad = side_pad_f as i8;
    let inner_w = (width - side_pad_f * 2.0).max(200.0);
    let theme = &spec.theme;

    egui::Frame::NONE
        .fill(theme.bottom_bg)
        .corner_radius(CornerRadius {
            nw: 0,
            ne: 0,
            sw: CARD_RADIUS,
            se: CARD_RADIUS,
        })
        .inner_margin(Margin {
            left: side_pad,
            right: side_pad,
            top: 26,
            bottom: 30,
        })
        .show(ui, |ui| {
            ui.set_min_width(inner_w);
            ui.set_max_width(inner_w);

            let body_resp = ui.allocate_ui_with_layout(
                Vec2::new(inner_w, 0.0),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    ui.set_min_width(inner_w);
                    ui.set_max_width(inner_w);
                    ui.add(
                        egui::Label::new(
                            RichText::new(spec.body_heading)
                                .size(16.0)
                                .strong()
                                .color(theme.bottom_text),
                        )
                        .selectable(false)
                        .wrap(),
                    );
                    ui.add_space(22.0);
                    draw_feature_grid(ui, &spec.features, theme, inner_w);
                },
            );
            body_rect = body_resp.response.rect;

            ui.add_space(26.0);
            let (clicked, btn_rect) = draw_cta_button(ui, spec.cta_label, theme, inner_w);
            footer_rect = btn_rect;
            if clicked {
                cta_clicked = true;
            }
        });

    (cta_clicked, body_rect, footer_rect)
}

fn draw_feature_grid(
    ui: &mut egui::Ui,
    features: &[FeatureItem; 4],
    theme: &IllustratedModalTheme,
    inner_w: f32,
) {
    let badge_dim = 42.0_f32;
    let icon_gap = 14.0_f32;
    let two_cols = inner_w >= 420.0;
    if two_cols {
        let min_col_gap = 24.0_f32;
        let max_text_w = ((inner_w - min_col_gap) * 0.5 - badge_dim - icon_gap).max(80.0);
        let galleys: Vec<_> = features
            .iter()
            .map(|item| {
                ui.painter().layout(
                    item.label.to_string(),
                    FontId::proportional(13.5),
                    theme.bottom_text,
                    max_text_w,
                )
            })
            .collect();
        let col1_w = badge_dim + icon_gap + galleys[0].size().x.max(galleys[2].size().x);
        let col2_w = badge_dim + icon_gap + galleys[1].size().x.max(galleys[3].size().x);
        let row_pairs = [(0usize, 1usize), (2usize, 3usize)];
        for (row_idx, &(left_idx, right_idx)) in row_pairs.iter().enumerate() {
            let row_h = badge_dim
                .max(galleys[left_idx].size().y)
                .max(galleys[right_idx].size().y);
            let (row_rect, _) = ui.allocate_exact_size(Vec2::new(inner_w, row_h), Sense::hover());
            let left_rect = Rect::from_min_size(row_rect.min, Vec2::new(col1_w, row_h));
            let right_rect = Rect::from_min_size(
                pos2(row_rect.right() - col2_w, row_rect.top()),
                Vec2::new(col2_w, row_h),
            );
            paint_feature_tile(
                ui,
                left_rect,
                features[left_idx].icon,
                std::sync::Arc::clone(&galleys[left_idx]),
                theme,
            );
            paint_feature_tile(
                ui,
                right_rect,
                features[right_idx].icon,
                std::sync::Arc::clone(&galleys[right_idx]),
                theme,
            );
            if row_idx + 1 < row_pairs.len() {
                ui.add_space(18.0);
            }
        }
    } else {
        let text_w = (inner_w - badge_dim - icon_gap).max(80.0);
        for (idx, item) in features.iter().enumerate() {
            let galley = ui.painter().layout(
                item.label.to_string(),
                FontId::proportional(13.5),
                theme.bottom_text,
                text_w,
            );
            let row_h = badge_dim.max(galley.size().y);
            let (row_rect, _) = ui.allocate_exact_size(Vec2::new(inner_w, row_h), Sense::hover());
            paint_feature_tile(ui, row_rect, item.icon, galley, theme);
            if idx + 1 < features.len() {
                ui.add_space(14.0);
            }
        }
    }
}

fn paint_feature_tile(
    ui: &mut egui::Ui,
    tile_rect: Rect,
    icon: Icon,
    galley: std::sync::Arc<egui::Galley>,
    theme: &IllustratedModalTheme,
) {
    let badge_dim = 42.0_f32;
    let icon_gap = 14.0_f32;
    let badge_center = pos2(tile_rect.left() + badge_dim * 0.5, tile_rect.center().y);
    let badge_rect = Rect::from_center_size(badge_center, Vec2::splat(badge_dim));
    ui.painter()
        .circle_filled(badge_center, badge_dim * 0.5, theme.badge_bg);
    paint_icon(
        ui.painter(),
        badge_rect.shrink(11.0),
        icon,
        theme.badge_icon,
    );

    let text_pos = pos2(
        badge_rect.right() + icon_gap,
        tile_rect.center().y - galley.size().y * 0.5,
    );
    ui.painter().galley(text_pos, galley, theme.bottom_text);
}

fn draw_cta_button(
    ui: &mut egui::Ui,
    label: &str,
    theme: &IllustratedModalTheme,
    width: f32,
) -> (bool, Rect) {
    let height = 44.0_f32;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let fill = if resp.hovered() {
        theme.cta_hover
    } else {
        theme.cta_bg
    };
    ui.painter().rect_filled(rect, CornerRadius::same(22), fill);
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(22),
            Stroke::new(2.0, theme.cta_focus_ring),
            egui::StrokeKind::Outside,
        );
    }
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        FontId::proportional(15.0),
        Color32::WHITE,
    );
    let text_pos = pos2(
        rect.center().x - galley.size().x * 0.5,
        rect.center().y - galley.size().y * 0.5,
    );
    ui.painter().galley(text_pos, galley, Color32::WHITE);
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    (resp.clicked(), rect)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use papercraft_engine::Session;

    fn collect_text(shapes: &[egui::epaint::ClippedShape]) -> String {
        let mut out = String::new();
        fn visit(shape: &egui::Shape, out: &mut String) {
            match shape {
                egui::Shape::Text(text) => {
                    out.push_str(text.galley.text());
                    out.push('\n');
                }
                egui::Shape::Vec(items) => {
                    for s in items {
                        visit(s, out);
                    }
                }
                _ => {}
            }
        }
        for cs in shapes {
            visit(&cs.shape, &mut out);
        }
        out
    }

    #[test]
    fn welcome_banner_renders_and_dismisses_into_first_access_nickname() {
        let dir = std::env::temp_dir().join(format!(
            "papercraft_welcome_ui_{}_{}",
            std::process::id(),
            papercraft_sync_protocol::now_unix_ms()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        struct Guard(std::path::PathBuf);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _guard = Guard(dir.clone());

        let session = Session::open_desktop_library(dir);
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        assert_eq!(app.view.active_modal, Some(ActiveModal::WelcomeBanner));

        let ctx = egui::Context::default();
        let mut rendered_text = String::new();
        let mut circle_centers: Vec<egui::Pos2> = Vec::new();
        let mut feature_text_boxes: Vec<(String, Rect)> = Vec::new();
        let mut cta_rect = Rect::NOTHING;
        for step in 0..6 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    pos2(0.0, 0.0),
                    Vec2::new(1280.0, 800.0),
                )),
                time: Some(f64::from(step) * 0.08),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                crate::modal::show(&mut app, ui);
            });
            rendered_text = collect_text(&out.shapes);
            circle_centers.clear();
            feature_text_boxes.clear();
            cta_rect = Rect::NOTHING;
            fn scan_shapes(
                shape: &egui::Shape,
                circles: &mut Vec<egui::Pos2>,
                texts: &mut Vec<(String, Rect)>,
                cta: &mut Rect,
            ) {
                match shape {
                    egui::Shape::Circle(c) if (c.radius - 21.0).abs() < 0.5 => {
                        circles.push(c.center);
                    }
                    egui::Shape::Rect(r) if (r.rect.height() - 44.0).abs() < 0.5 => {
                        *cta = r.rect;
                    }
                    egui::Shape::Text(t) => {
                        let rect = Rect::from_min_size(t.pos, t.galley.size());
                        texts.push((t.galley.text().to_string(), rect));
                    }
                    egui::Shape::Vec(items) => {
                        for s in items {
                            scan_shapes(s, circles, texts, cta);
                        }
                    }
                    _ => {}
                }
            }
            for cs in &out.shapes {
                scan_shapes(
                    &cs.shape,
                    &mut circle_centers,
                    &mut feature_text_boxes,
                    &mut cta_rect,
                );
            }
            out.drop_without_applying_deltas();
        }
        assert!(
            rendered_text.contains("Study deeper with PaperCraft"),
            "rendered text:\n{rendered_text}"
        );
        assert!(rendered_text.contains("Load huge PDFs smoothly"));
        assert!(rendered_text.contains("Natural stylus ink with"));
        assert!(rendered_text.contains("Clean PDF, PNG & SVG"));
        assert!(rendered_text.contains("100% offline-first with"));
        assert!(rendered_text.contains("Start Studying"));

        assert_eq!(circle_centers.len(), 4, "expected 4 feature icon badges");
        assert!(cta_rect.width() > 400.0, "expected full-width CTA pill");
        // Left-column icon badges must align flush with CTA button's left edge.
        assert!((circle_centers[0].x - 21.0 - cta_rect.left()).abs() <= 1.0);
        assert!((circle_centers[2].x - 21.0 - cta_rect.left()).abs() <= 1.0);
        // Right-column icon badges must share the exact same x and sit cleanly in the right half.
        assert!((circle_centers[1].x - circle_centers[3].x).abs() <= 0.5);
        assert!(circle_centers[1].x > cta_rect.center().x);

        let mut right_col_max_x = f32::NEG_INFINITY;
        // Each feature's text block must be vertically centered on its icon badge.
        for (idx, item) in FEATURES.iter().enumerate() {
            let first_line = item.label.lines().next().unwrap_or("");
            let (_, text_rect) = feature_text_boxes
                .iter()
                .find(|(txt, _)| txt.contains(first_line))
                .expect("feature text shape missing");
            if idx % 2 == 1 {
                right_col_max_x = right_col_max_x.max(text_rect.right());
            }
            let dy = (text_rect.center().y - circle_centers[idx].y).abs();
            assert!(
                dy <= 1.5,
                "feature {idx} text center y ({}) misaligned with icon badge center y ({})",
                text_rect.center().y,
                circle_centers[idx].y
            );
        }
        // Right-column widest text must end flush with CTA button's right edge.
        assert!(
            (right_col_max_x - cta_rect.right()).abs() <= 1.5,
            "right column right edge ({right_col_max_x}) != CTA right ({})",
            cta_rect.right()
        );

        dismiss(&mut app);
        assert!(!app.session.should_show_welcome_banner());
        assert_eq!(
            app.view.active_modal,
            Some(ActiveModal::FirstAccessNickname)
        );

        app.session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "Alyx" }),
            )
            .unwrap();
        crate::modal::open(&mut app, ActiveModal::WelcomeBanner);
        dismiss(&mut app);
        assert_eq!(app.view.active_modal, None);
    }

    #[test]
    fn welcome_banner_ignores_outside_clicks_and_escape_until_button_clicked() {
        let dir = std::env::temp_dir().join(format!(
            "papercraft_welcome_button_only_{}_{}",
            std::process::id(),
            papercraft_sync_protocol::now_unix_ms()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        struct Guard(std::path::PathBuf);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _guard = Guard(dir.clone());

        let session = Session::open_desktop_library(dir);
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        assert_eq!(app.view.active_modal, Some(ActiveModal::WelcomeBanner));

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;
        let mut cta_center = pos2(0.0, 0.0);
        for _ in 0..8 {
            time += 0.08;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), viewport)),
                time: Some(time),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
            fn find_cta(shape: &egui::Shape, cta: &mut Rect) {
                match shape {
                    egui::Shape::Rect(r) if (r.rect.height() - 44.0).abs() < 0.5 => {
                        *cta = r.rect;
                    }
                    egui::Shape::Vec(items) => {
                        for s in items {
                            find_cta(s, cta);
                        }
                    }
                    _ => {}
                }
            }
            let mut cta = Rect::NOTHING;
            for cs in &out.shapes {
                find_cta(&cs.shape, &mut cta);
            }
            if cta.is_positive() {
                cta_center = cta.center();
            }
            out.drop_without_applying_deltas();
        }
        assert!(cta_center.x > 0.0);

        // 1. Clicking outside the modal on the backdrop keeps WelcomeBanner open.
        let outside = pos2(16.0, 16.0);
        for pressed in [true, false] {
            time += 0.04;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), viewport)),
                time: Some(time),
                predicted_dt: 0.04,
                events: vec![
                    egui::Event::PointerMoved(outside),
                    egui::Event::PointerButton {
                        pos: outside,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
            out.drop_without_applying_deltas();
        }
        assert_eq!(app.view.active_modal, Some(ActiveModal::WelcomeBanner));
        assert!(app.session.should_show_welcome_banner());

        // 2. Pressing Escape keeps WelcomeBanner open.
        time += 0.04;
        let esc_raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), viewport)),
            time: Some(time),
            predicted_dt: 0.04,
            events: vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let esc_out = ctx.run_ui(esc_raw, |ui| app.show_ui(ui));
        esc_out.drop_without_applying_deltas();
        assert_eq!(app.view.active_modal, Some(ActiveModal::WelcomeBanner));
        assert!(app.session.should_show_welcome_banner());

        // 3. Clicking the CTA button dismisses WelcomeBanner cleanly.
        for pressed in [true, false] {
            time += 0.04;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), viewport)),
                time: Some(time),
                predicted_dt: 0.04,
                events: vec![
                    egui::Event::PointerMoved(cta_center),
                    egui::Event::PointerButton {
                        pos: cta_center,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
            out.drop_without_applying_deltas();
        }
        assert!(!app.session.should_show_welcome_banner());
        assert_eq!(
            app.view.active_modal,
            Some(ActiveModal::FirstAccessNickname)
        );
    }
}
