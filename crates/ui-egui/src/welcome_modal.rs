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

struct FeatureItem {
    icon: Icon,
    label: &'static str,
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

#[derive(Clone)]
enum CachedWelcomeArt {
    Ready(egui::TextureHandle),
    Failed,
}

fn welcome_sea_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let cache_id = egui::Id::unique("papercraft-welcome-sea-illustration");
    if let Some(cached) = ctx.data_mut(|d| d.get_temp::<CachedWelcomeArt>(cache_id)) {
        return match cached {
            CachedWelcomeArt::Ready(tex) => Some(tex),
            CachedWelcomeArt::Failed => None,
        };
    }
    let decoded = match image::load_from_memory(WELCOME_SEA_PNG) {
        Ok(img) => img.into_rgba8(),
        Err(err) => {
            log::warn!("Could not decode welcome sea illustration PNG: {err}");
            ctx.data_mut(|d| d.insert_temp(cache_id, CachedWelcomeArt::Failed));
            return None;
        }
    };
    let size = [decoded.width() as usize, decoded.height() as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
    let tex = ctx.load_texture(
        "papercraft-welcome-sea-illustration",
        color_image,
        egui::TextureOptions::LINEAR,
    );
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
    app.install_rich_fonts_if_needed(ui.ctx());
    let ctx = ui.ctx().clone();
    let viewport = ctx.content_rect();
    let dialog_width = PREFERRED_WIDTH.min((viewport.width() - 32.0).max(300.0));
    let is_dark_theme = app.session.prefs.theme == papercraft_doc::UiTheme::Dark;

    let backdrop_id = ui.make_persistent_id("papercraft_modal_backdrop");
    let modal_id = ui.make_persistent_id((
        "papercraft_modal_surface",
        ActiveModal::WelcomeBanner.id_tag(),
    ));

    let backdrop_resp = egui::Area::new(backdrop_id)
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

    let mut should_dismiss = false;
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
                        draw_top_header(ui, dialog_width, viewport.height());
                    close_btn_rect = x_rect;
                    if close_clicked {
                        should_dismiss = true;
                    }

                    let (cta_clicked, b_rect, f_rect) = draw_bottom_section(ui, dialog_width);
                    body_rect = b_rect;
                    footer_rect = f_rect;
                    if cta_clicked {
                        should_dismiss = true;
                    }
                });
        });

    let modal_rect = modal_resp.response.rect;
    let pointer_in_modal = backdrop_resp
        .interact_pointer_pos()
        .is_some_and(|pos| modal_rect.contains(pos));
    if interactive && backdrop_resp.clicked() && !pointer_in_modal {
        should_dismiss = true;
    }

    if interactive && should_dismiss {
        dismiss(app);
    }

    ModalMetrics {
        modal_rect,
        close_btn_rect,
        body_rect,
        footer_rect,
        inner_margin: 0.0,
    }
}

fn draw_top_header(ui: &mut egui::Ui, width: f32, viewport_h: f32) -> (bool, Rect) {
    let mut close_clicked = false;
    let mut close_rect = Rect::NOTHING;
    let top_pad: i8 = if viewport_h < 600.0 { 18 } else { 26 };
    let bottom_pad: i8 = if viewport_h < 600.0 { 12 } else { 18 };
    let side_pad: i8 = 24;
    let inner_w = (width - f32::from(side_pad) * 2.0).max(200.0);
    let art_w = if viewport_h < 600.0 { 164.0 } else { 224.0 };

    egui::Frame::NONE
        .fill(TOP_BG)
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
                                RichText::new("Study deeper with PaperCraft")
                                    .font(FontId::new(
                                        25.0,
                                        FontFamily::Name(BRAND_DISPLAY_FAMILY.into()),
                                    ))
                                    .color(TOP_TITLE),
                            )
                            .selectable(false)
                            .wrap(),
                        );
                    },
                );
                let (btn_rect, resp) = ui.allocate_exact_size(Vec2::splat(close_w), Sense::click());
                close_rect = btn_rect;
                if resp.hovered() {
                    ui.painter()
                        .circle_filled(btn_rect.center(), close_w * 0.5, TOP_CLOSE_HOVER);
                }
                paint_icon(ui.painter(), btn_rect.shrink(6.0), Icon::Close, TOP_TITLE);
                let resp = resp
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text("Close (Esc)");
                if resp.clicked() {
                    close_clicked = true;
                }
            });

            ui.add_space(if viewport_h < 600.0 { 4.0 } else { 8.0 });
            draw_sea_illustration(ui, inner_w, art_w);
        });

    (close_clicked, close_rect)
}

fn draw_sea_illustration(ui: &mut egui::Ui, inner_w: f32, art_w: f32) {
    let clamped_w = art_w.min(inner_w);
    if let Some(tex) = welcome_sea_texture(ui.ctx()) {
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
        ui.painter().circle_filled(badge.center(), 38.0, BADGE_BG);
        paint_icon(ui.painter(), badge.shrink(20.0), Icon::Notebook, TOP_TITLE);
    }
}

fn draw_bottom_section(ui: &mut egui::Ui, width: f32) -> (bool, Rect, Rect) {
    let mut cta_clicked = false;
    let mut body_rect = Rect::NOTHING;
    let mut footer_rect = Rect::NOTHING;
    let side_pad_f = if width >= 620.0 { 52.0 } else { 24.0 };
    let side_pad = side_pad_f as i8;
    let inner_w = (width - side_pad_f * 2.0).max(200.0);

    egui::Frame::NONE
        .fill(BOTTOM_BG)
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
                            RichText::new("Built for calm, distraction-free daily study:")
                                .size(16.0)
                                .strong()
                                .color(BOTTOM_TEXT),
                        )
                        .selectable(false)
                        .wrap(),
                    );
                    ui.add_space(22.0);
                    draw_feature_grid(ui, inner_w);
                },
            );
            body_rect = body_resp.response.rect;

            ui.add_space(26.0);
            let (clicked, btn_rect) = draw_cta_button(ui, inner_w);
            footer_rect = btn_rect;
            if clicked {
                cta_clicked = true;
            }
        });

    (cta_clicked, body_rect, footer_rect)
}

fn draw_feature_grid(ui: &mut egui::Ui, inner_w: f32) {
    let two_cols = inner_w >= 420.0;
    if two_cols {
        let gap = 24.0_f32;
        let col_w = ((inner_w - gap) * 0.5).max(140.0);
        let row_count = FEATURES.len().div_ceil(2);
        for (row_idx, row) in FEATURES.chunks(2).enumerate() {
            ui.allocate_ui_with_layout(
                Vec2::new(inner_w, 0.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(inner_w);
                    ui.set_max_width(inner_w);
                    if let Some(left) = row.first() {
                        let _ = draw_feature_tile(ui, left, col_w);
                    }
                    ui.add_space(gap);
                    if let Some(right) = row.get(1) {
                        let _ = draw_feature_tile(ui, right, col_w);
                    }
                },
            );
            if row_idx + 1 < row_count {
                ui.add_space(18.0);
            }
        }
    } else {
        for (idx, item) in FEATURES.iter().enumerate() {
            let _ = draw_feature_tile(ui, item, inner_w);
            if idx + 1 < FEATURES.len() {
                ui.add_space(14.0);
            }
        }
    }
}

fn draw_feature_tile(ui: &mut egui::Ui, item: &FeatureItem, col_w: f32) -> (Rect, Rect) {
    let badge_dim = 42.0_f32;
    let gap = 14.0_f32;
    let text_w = (col_w - badge_dim - gap).max(80.0);
    let galley = ui.painter().layout(
        item.label.to_string(),
        FontId::proportional(13.5),
        BOTTOM_TEXT,
        text_w,
    );
    let row_h = badge_dim.max(galley.size().y);
    let (tile_rect, _) = ui.allocate_exact_size(Vec2::new(col_w, row_h), Sense::hover());
    let badge_center = pos2(tile_rect.left() + badge_dim * 0.5, tile_rect.center().y);
    let badge_rect = Rect::from_center_size(badge_center, Vec2::splat(badge_dim));
    ui.painter()
        .circle_filled(badge_center, badge_dim * 0.5, BADGE_BG);
    paint_icon(ui.painter(), badge_rect.shrink(11.0), item.icon, TOP_TITLE);

    let text_pos = pos2(
        badge_rect.right() + gap,
        tile_rect.center().y - galley.size().y * 0.5,
    );
    let text_rect = Rect::from_min_size(text_pos, galley.size());
    ui.painter().galley(text_pos, galley, BOTTOM_TEXT);
    (badge_rect, text_rect)
}

fn draw_cta_button(ui: &mut egui::Ui, width: f32) -> (bool, Rect) {
    let height = 44.0_f32;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let fill = if resp.hovered() { CTA_HOVER } else { CTA_BG };
    ui.painter().rect_filled(rect, CornerRadius::same(22), fill);
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(22),
            Stroke::new(2.0, Color32::WHITE),
            egui::StrokeKind::Outside,
        );
    }
    let galley = ui.painter().layout_no_wrap(
        "Start Studying".to_string(),
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
            fn scan_shapes(
                shape: &egui::Shape,
                circles: &mut Vec<egui::Pos2>,
                texts: &mut Vec<(String, Rect)>,
            ) {
                match shape {
                    egui::Shape::Circle(c) if (c.radius - 21.0).abs() < 0.5 => {
                        circles.push(c.center);
                    }
                    egui::Shape::Text(t) => {
                        let rect = Rect::from_min_size(t.pos, t.galley.size());
                        texts.push((t.galley.text().to_string(), rect));
                    }
                    egui::Shape::Vec(items) => {
                        for s in items {
                            scan_shapes(s, circles, texts);
                        }
                    }
                    _ => {}
                }
            }
            for cs in &out.shapes {
                scan_shapes(&cs.shape, &mut circle_centers, &mut feature_text_boxes);
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
        // Right-column badges must start cleanly past the modal midpoint (x = 640).
        assert!(
            circle_centers[1].x > 640.0 && circle_centers[3].x > 640.0,
            "right column badges drifted left: {:?}",
            circle_centers
        );
        // Each feature's text block must be vertically centered on its icon badge.
        for (idx, item) in FEATURES.iter().enumerate() {
            let first_line = item.label.lines().next().unwrap_or("");
            let (_, text_rect) = feature_text_boxes
                .iter()
                .find(|(txt, _)| txt.contains(first_line))
                .expect("feature text shape missing");
            let dy = (text_rect.center().y - circle_centers[idx].y).abs();
            assert!(
                dy <= 1.5,
                "feature {idx} text center y ({}) misaligned with icon badge center y ({})",
                text_rect.center().y,
                circle_centers[idx].y
            );
        }

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
}
