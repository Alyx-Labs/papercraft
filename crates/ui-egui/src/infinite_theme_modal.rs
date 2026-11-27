//! Two-tone illustrated modal explaining PaperCraft's Infinite theme hue & saturation matrix.

use egui::Color32;

use crate::icons::Icon;
use crate::modal::{ActiveModal, ModalMetrics};
use crate::welcome_modal::{
    FeatureItem, IllustratedModalSpec, IllustratedModalTheme, show_illustrated_modal,
};
use crate::{PaperCraftApp, SettingsTab};

const INFINITE_THEME_PNG: &[u8] = include_bytes!("../assets/illustrations/infinite-theme.png");

const TOP_BG: Color32 = Color32::from_rgb(0xEA, 0xE3, 0xFA);
const TOP_TITLE: Color32 = Color32::from_rgb(0x2E, 0x1D, 0x59);
const TOP_CLOSE_HOVER: Color32 = Color32::from_rgb(0xDD, 0xD2, 0xF7);
const BOTTOM_BG: Color32 = Color32::from_rgb(0x26, 0x24, 0x30);
const BOTTOM_TEXT: Color32 = Color32::from_rgb(0xF1, 0xEF, 0xF8);
const BADGE_BG: Color32 = Color32::from_rgb(0xF7, 0xF4, 0xFD);
const CTA_BG: Color32 = Color32::from_rgb(0x5E, 0x3A, 0xC2);
const CTA_HOVER: Color32 = Color32::from_rgb(0x6D, 0x47, 0xD4);

const FEATURES: [FeatureItem; 4] = [
    FeatureItem {
        icon: Icon::Palette,
        label: "Rotate hue 0°–360° &\nscale saturation 0%–250%",
    },
    FeatureItem {
        icon: Icon::Sun,
        label: "Layers smoothly atop\nLight and Dark themes",
    },
    FeatureItem {
        icon: Icon::Eye,
        label: "Protects readable WCAG\ncontrast at every tint",
    },
    FeatureItem {
        icon: Icon::Notebook,
        label: "UI only — paper, ink,\nand exports stay untouched",
    },
];

const fn build_spec(cta_label: &'static str) -> IllustratedModalSpec {
    IllustratedModalSpec {
        modal: ActiveModal::InfiniteTheme,
        title: "Color your studio with Infinite",
        body_heading: "How the hue & saturation matrix adapts your UI:",
        cta_label,
        illustration_cache_key: "papercraft-infinite-theme-illustration",
        illustration_png: INFINITE_THEME_PNG,
        fallback_icon: Icon::Palette,
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
    }
}

fn dismiss_notice(app: &mut PaperCraftApp) {
    let _ = app
        .session
        .execute("profile.dismissInfiniteThemeNotice", &serde_json::json!({}));
    app.flush_web_storage(true);
}

/// Opens the Infinite theme modal automatically when entering a notebook if unseen in this release.
pub(crate) fn maybe_open_release_notice(app: &mut PaperCraftApp) {
    if app.view.active_modal.is_none()
        && app.view.screen == crate::AppScreen::Editor
        && app.session.should_show_infinite_theme_notice()
    {
        crate::modal::open(app, ActiveModal::InfiniteTheme);
    }
}

/// Enables Infinite theme if needed, marks the release notice seen, opens the Appearance sidebar, and closes.
pub(crate) fn activate_and_close(app: &mut PaperCraftApp) {
    if !app.session.prefs.infinite_theme.enabled {
        let _ = app.session.execute(
            "theme.set",
            &serde_json::json!({ "infinite_enabled": true }),
        );
    }
    dismiss_notice(app);
    app.view.inspector_open = true;
    app.view.settings_tab = SettingsTab::Appearance;
    if app.view.screen == crate::AppScreen::Home {
        app.view.account.profile_section = crate::account_modal::ProfileSection::Appearance;
        let order = crate::account_modal::ProfileSection::Appearance.flow_order();
        app.view
            .motion
            .profile_section
            .sync((app.session.is_signed_in(), order as u8), order);
    }
    crate::modal::close(app);
}

/// Renders the two-tone illustrated Infinite theme modal.
pub(crate) fn show(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    eased: f32,
    backdrop_eased: f32,
    interactive: bool,
) -> ModalMetrics {
    let cta_label = if app.session.prefs.infinite_theme.enabled {
        "Customize Hue & Saturation"
    } else {
        "Enable Infinite Theme"
    };
    let spec = build_spec(cta_label);
    let (metrics, action) =
        show_illustrated_modal(app, ui, &spec, eased, backdrop_eased, interactive);
    if interactive && action.is_some() {
        activate_and_close(app);
    }
    metrics
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect, Vec2, pos2};
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

    fn scan_shapes(
        shape: &egui::Shape,
        circles: &mut Vec<Pos2>,
        texts: &mut Vec<(String, Rect)>,
        cta: &mut Rect,
        bottom_fill: &mut Option<Color32>,
    ) {
        match shape {
            egui::Shape::Circle(c) if (c.radius - 21.0).abs() < 0.5 => {
                circles.push(c.center);
            }
            egui::Shape::Rect(r) if (r.rect.height() - 44.0).abs() < 0.5 => {
                *cta = r.rect;
            }
            egui::Shape::Rect(r) if r.fill == BOTTOM_BG && r.rect.width() > 400.0 => {
                *bottom_fill = Some(r.fill);
            }
            egui::Shape::Text(t) => {
                let rect = Rect::from_min_size(t.pos, t.galley.size());
                texts.push((t.galley.text().to_string(), rect));
            }
            egui::Shape::Vec(items) => {
                for s in items {
                    scan_shapes(s, circles, texts, cta, bottom_fill);
                }
            }
            _ => {}
        }
    }

    fn click_pos(
        ctx: &egui::Context,
        app: &mut PaperCraftApp,
        viewport: Vec2,
        time: &mut f64,
        pos: Pos2,
    ) {
        for pressed in [true, false] {
            *time += 0.04;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                time: Some(*time),
                predicted_dt: 0.04,
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
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
    }

    fn press_escape(ctx: &egui::Context, app: &mut PaperCraftApp, viewport: Vec2, time: &mut f64) {
        *time += 0.04;
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                time: Some(*time),
                predicted_dt: 0.04,
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| app.show_ui(ui),
        );
        out.drop_without_applying_deltas();
    }

    fn settle_and_assert_layout(
        ctx: &egui::Context,
        app: &mut PaperCraftApp,
        viewport: Vec2,
        time: &mut f64,
        expected_cta: &str,
    ) -> Rect {
        let mut rendered_text = String::new();
        let mut circle_centers = Vec::new();
        let mut feature_text_boxes = Vec::new();
        let mut cta_rect = Rect::NOTHING;
        let mut bottom_fill = None;
        for _ in 0..8 {
            *time += 0.08;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                time: Some(*time),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
            rendered_text = collect_text(&out.shapes);
            circle_centers.clear();
            feature_text_boxes.clear();
            cta_rect = Rect::NOTHING;
            bottom_fill = None;
            for cs in &out.shapes {
                scan_shapes(
                    &cs.shape,
                    &mut circle_centers,
                    &mut feature_text_boxes,
                    &mut cta_rect,
                    &mut bottom_fill,
                );
            }
            out.drop_without_applying_deltas();
        }

        assert_eq!(bottom_fill, Some(BOTTOM_BG));
        assert!(rendered_text.contains("Color your studio with Infinite"));
        assert!(rendered_text.contains("How the hue & saturation matrix adapts your UI:"));
        assert!(rendered_text.contains("Rotate hue 0°–360° &"));
        assert!(rendered_text.contains("Layers smoothly atop"));
        assert!(rendered_text.contains("Protects readable WCAG"));
        assert!(rendered_text.contains("UI only — paper, ink,"));
        assert!(rendered_text.contains(expected_cta));

        assert_eq!(circle_centers.len(), 4, "expected 4 feature icon badges");
        assert!(cta_rect.width() > 400.0, "expected full-width CTA pill");
        assert!((circle_centers[0].x - 21.0 - cta_rect.left()).abs() <= 1.0);
        assert!((circle_centers[2].x - 21.0 - cta_rect.left()).abs() <= 1.0);
        assert!((circle_centers[1].x - circle_centers[3].x).abs() <= 0.5);
        assert!(circle_centers[1].x > cta_rect.center().x);

        let mut right_col_max_x = f32::NEG_INFINITY;
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
            assert!(dy <= 1.5, "feature {idx} text misaligned vertically");
        }
        assert!((right_col_max_x - cta_rect.right()).abs() <= 1.5);
        cta_rect
    }

    #[test]
    fn infinite_theme_modal_opens_from_appearance_and_enables_via_cta() {
        let mut app = PaperCraftApp::new(Session::default(), crate::PlatformHooks::default());
        app.view.screen = crate::AppScreen::Editor;
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
        app.view.inspector_open = true;
        app.view.settings_tab = SettingsTab::Appearance;
        assert!(!app.session.prefs.infinite_theme.enabled);

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;
        let mut info_btn_center = Pos2::ZERO;
        for _ in 0..8 {
            time += 0.08;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                time: Some(time),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
            let mut texts = Vec::new();
            let mut circles = Vec::new();
            let mut cta = Rect::NOTHING;
            let mut bottom = None;
            for cs in &out.shapes {
                scan_shapes(&cs.shape, &mut circles, &mut texts, &mut cta, &mut bottom);
            }
            if let Some((_, rect)) = texts
                .iter()
                .find(|(txt, _)| txt.contains("How Infinite works"))
            {
                info_btn_center = rect.center();
            }
            out.drop_without_applying_deltas();
        }
        assert!(
            info_btn_center.x > 0.0,
            "expected 'How Infinite works' button in Appearance settings"
        );

        click_pos(&ctx, &mut app, viewport, &mut time, info_btn_center);
        assert_eq!(app.view.active_modal, Some(ActiveModal::InfiniteTheme));

        let _ =
            settle_and_assert_layout(&ctx, &mut app, viewport, &mut time, "Enable Infinite Theme");

        // Outside clicks keep the modal open, while Escape closes it without changing prefs.
        click_pos(&ctx, &mut app, viewport, &mut time, pos2(16.0, 16.0));
        assert_eq!(app.view.active_modal, Some(ActiveModal::InfiniteTheme));

        press_escape(&ctx, &mut app, viewport, &mut time);
        assert_eq!(app.view.active_modal, None);
        assert!(!app.session.prefs.infinite_theme.enabled);

        // Re-open and click CTA -> enables Infinite theme, keeps Appearance open, closes modal.
        crate::modal::open(&mut app, ActiveModal::InfiniteTheme);
        let cta_rect =
            settle_and_assert_layout(&ctx, &mut app, viewport, &mut time, "Enable Infinite Theme");
        click_pos(&ctx, &mut app, viewport, &mut time, cta_rect.center());
        assert!(app.session.prefs.infinite_theme.enabled);
        assert!(app.view.inspector_open);
        assert_eq!(app.view.settings_tab, SettingsTab::Appearance);
        assert_eq!(app.view.active_modal, None);

        // When already enabled, CTA reflects customization and Close X dismisses cleanly.
        crate::modal::open(&mut app, ActiveModal::InfiniteTheme);
        let _ = settle_and_assert_layout(
            &ctx,
            &mut app,
            viewport,
            &mut time,
            "Customize Hue & Saturation",
        );
        let spec = build_spec("Customize Hue & Saturation");
        let mut close_center = Pos2::ZERO;
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                time: Some(time + 0.08),
                predicted_dt: 0.08,
                ..Default::default()
            },
            |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                let (metrics, _) = show_illustrated_modal(&mut app, ui, &spec, 1.0, 1.0, true);
                close_center = metrics.close_btn_rect.center();
            },
        );
        out.drop_without_applying_deltas();
        click_pos(&ctx, &mut app, viewport, &mut time, close_center);
        assert_eq!(app.view.active_modal, None);
        assert!(app.session.prefs.infinite_theme.enabled);
    }

    fn dismiss_startup_onboarding(app: &mut PaperCraftApp) {
        assert_eq!(app.view.active_modal, Some(ActiveModal::WelcomeBanner));
        crate::welcome_modal::dismiss(app);
        app.session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "Alyx" }),
            )
            .unwrap();
        app.session
            .execute(
                "profile.setStartupPicker",
                &serde_json::json!({ "skip": true }),
            )
            .unwrap();
        crate::modal::close(app);
        assert_eq!(app.view.active_modal, None);
    }

    fn close_button_center(
        ctx: &egui::Context,
        app: &mut PaperCraftApp,
        viewport: Vec2,
        time: &mut f64,
        cta_label: &'static str,
    ) -> Pos2 {
        let spec = build_spec(cta_label);
        let mut close_center = Pos2::ZERO;
        *time += 0.08;
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                time: Some(*time),
                predicted_dt: 0.08,
                ..Default::default()
            },
            |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                let (metrics, _) = show_illustrated_modal(app, ui, &spec, 1.0, 1.0, true);
                close_center = metrics.close_btn_rect.center();
            },
        );
        out.drop_without_applying_deltas();
        close_center
    }

    fn assert_appearance_sliders_visible(
        ctx: &egui::Context,
        app: &mut PaperCraftApp,
        viewport: Vec2,
        time: &mut f64,
    ) {
        let mut sidebar_text = String::new();
        for _ in 0..6 {
            *time += 0.08;
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                    time: Some(*time),
                    predicted_dt: 0.08,
                    ..Default::default()
                },
                |ui| app.show_ui(ui),
            );
            sidebar_text = collect_text(&out.shapes);
            out.drop_without_applying_deltas();
        }
        assert!(sidebar_text.contains("Infinite theme"));
        assert!(sidebar_text.contains("Hue"));
        assert!(sidebar_text.contains("Saturation"));
    }

    fn verify_infinite_theme_reappears_after_upgrade(dir: &std::path::Path, viewport: Vec2) {
        let mut upgraded = papercraft_sync::load_global_state(dir);
        upgraded.infinite_theme_notice_seen_version = Some("0.7.3".to_string());
        papercraft_sync::save_global_state(dir, &upgraded).unwrap();

        let upgraded_session = Session::open_desktop_library(dir.to_path_buf());
        let mut upgraded_app =
            PaperCraftApp::new(upgraded_session, crate::PlatformHooks::default());
        upgraded_app
            .session
            .execute(
                "theme.set",
                &serde_json::json!({ "infinite_enabled": false }),
            )
            .unwrap();
        assert_eq!(upgraded_app.view.active_modal, None);

        // Also verify show_ui triggers it cleanly after transitioning into Editor (e.g. post-wizard).
        upgraded_app.view.screen = crate::AppScreen::Editor;
        upgraded_app.view.motion.screen =
            crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
        let upgrade_ctx = egui::Context::default();
        let mut upgrade_time = 0.0_f64;
        let cta_rect = settle_and_assert_layout(
            &upgrade_ctx,
            &mut upgraded_app,
            viewport,
            &mut upgrade_time,
            "Enable Infinite Theme",
        );
        assert_eq!(
            upgraded_app.view.active_modal,
            Some(ActiveModal::InfiniteTheme)
        );
        click_pos(
            &upgrade_ctx,
            &mut upgraded_app,
            viewport,
            &mut upgrade_time,
            cta_rect.center(),
        );
        assert_eq!(upgraded_app.view.active_modal, None);
        assert!(!upgraded_app.session.should_show_infinite_theme_notice());
        assert!(upgraded_app.session.prefs.infinite_theme.enabled);
        assert!(upgraded_app.view.inspector_open);
        assert_eq!(upgraded_app.view.settings_tab, SettingsTab::Appearance);
    }

    #[test]
    fn infinite_theme_modal_appears_on_notebook_open_once_per_release() {
        let dir = std::env::temp_dir().join(format!(
            "papercraft_infinite_release_ui_{}_{}",
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

        // 1. First install: opening a notebook opens InfiniteTheme automatically once dismissed.
        let session = Session::open_desktop_library(dir.clone());
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        dismiss_startup_onboarding(&mut app);
        assert!(app.session.should_show_infinite_theme_notice());
        app.session
            .execute(
                "notebook.create",
                &serde_json::json!({ "title": "Study Notes" }),
            )
            .unwrap();

        crate::open_notebook_in_editor(&mut app, 0);
        assert_eq!(app.view.screen, crate::AppScreen::Editor);
        assert_eq!(app.view.active_modal, Some(ActiveModal::InfiniteTheme));

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;
        let _ =
            settle_and_assert_layout(&ctx, &mut app, viewport, &mut time, "Enable Infinite Theme");

        // During first release display, outside clicks and Escape do not skip the modal.
        click_pos(&ctx, &mut app, viewport, &mut time, pos2(16.0, 16.0));
        assert_eq!(app.view.active_modal, Some(ActiveModal::InfiniteTheme));
        press_escape(&ctx, &mut app, viewport, &mut time);
        assert_eq!(app.view.active_modal, Some(ActiveModal::InfiniteTheme));

        let close_center =
            close_button_center(&ctx, &mut app, viewport, &mut time, "Enable Infinite Theme");
        click_pos(&ctx, &mut app, viewport, &mut time, close_center);
        assert_eq!(app.view.active_modal, None);
        assert!(!app.session.should_show_infinite_theme_notice());
        assert!(app.session.prefs.infinite_theme.enabled);
        assert!(app.view.inspector_open);
        assert_eq!(app.view.settings_tab, SettingsTab::Appearance);
        assert_appearance_sliders_visible(&ctx, &mut app, viewport, &mut time);

        // 2. Reopening within the same release does not show InfiniteTheme when opening a notebook.
        let same_session = Session::open_desktop_library(dir.clone());
        let mut same_app = PaperCraftApp::new(same_session, crate::PlatformHooks::default());
        assert_eq!(same_app.view.active_modal, None);
        crate::open_notebook_in_editor(&mut same_app, 0);
        assert_eq!(same_app.view.screen, crate::AppScreen::Editor);
        assert_eq!(same_app.view.active_modal, None);
        assert!(!same_app.view.inspector_open);

        // 3. After upgrading to a newer release version, opening a notebook shows it again.
        verify_infinite_theme_reappears_after_upgrade(&dir, viewport);
    }
}
