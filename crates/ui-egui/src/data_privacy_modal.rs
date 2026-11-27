//! First-notebook light Data & Privacy modal explaining PaperCraft's local-first storage.

use egui::Color32;

use crate::PaperCraftApp;
use crate::icons::Icon;
use crate::modal::{ActiveModal, ModalMetrics};
use crate::welcome_modal::{
    FeatureItem, IllustratedModalAction, IllustratedModalSpec, IllustratedModalTheme,
    show_illustrated_modal,
};

const PARK_PRIVACY_PNG: &[u8] = include_bytes!("../assets/illustrations/welcome-park-privacy.png");

const TOP_BG: Color32 = Color32::from_rgb(0xD2, 0xEE, 0xDC);
const TOP_TITLE: Color32 = Color32::from_rgb(0x14, 0x4A, 0x35);
const TOP_CLOSE_HOVER: Color32 = Color32::from_rgb(0xBD, 0xE2, 0xCB);
const BOTTOM_BG: Color32 = Color32::from_rgb(0xF8, 0xFB, 0xF9);
const BOTTOM_TEXT: Color32 = Color32::from_rgb(0x18, 0x30, 0x26);
const BADGE_BG: Color32 = Color32::from_rgb(0xE2, 0xF4, 0xE8);
const CTA_BG: Color32 = Color32::from_rgb(0x1A, 0x7A, 0x52);
const CTA_HOVER: Color32 = Color32::from_rgb(0x21, 0x8F, 0x61);

const FEATURES: [FeatureItem; 4] = [
    FeatureItem {
        icon: Icon::Folder,
        label: "Saved locally on your device\nIn your app folder or browser",
    },
    FeatureItem {
        icon: Icon::Lock,
        label: "Private isolated workspaces\nLocal profiles never mix data",
    },
    FeatureItem {
        icon: Icon::Sync,
        label: "Optional self-hosted sync\nOnly connects to your server",
    },
    FeatureItem {
        icon: Icon::Export,
        label: "One-click .pcraftpkg backups\nMove or restore notes anytime",
    },
];

const DATA_PRIVACY_SPEC: IllustratedModalSpec = IllustratedModalSpec {
    modal: ActiveModal::FirstNotebookDataNotice,
    title: "Your notebooks stay private",
    body_heading: "How your data is saved and protected:",
    cta_label: "Continue to Create Notebook",
    illustration_cache_key: "papercraft-data-privacy-park-illustration",
    illustration_png: PARK_PRIVACY_PNG,
    fallback_icon: Icon::Lock,
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
        cta_focus_ring: TOP_TITLE,
    },
};

/// Marks the first-notebook data notice as seen and opens the New Notebook wizard.
pub(crate) fn dismiss_and_continue(app: &mut PaperCraftApp) {
    let _ = app.session.execute(
        "profile.dismissFirstNotebookDataNotice",
        &serde_json::json!({}),
    );
    crate::modal::open(app, ActiveModal::NewNotebook);
}

/// Marks the first-notebook data notice as seen and returns to the workspace.
pub(crate) fn dismiss_only(app: &mut PaperCraftApp) {
    let _ = app.session.execute(
        "profile.dismissFirstNotebookDataNotice",
        &serde_json::json!({}),
    );
    crate::modal::close(app);
}

/// Renders the light two-tone first-notebook Data & Privacy modal.
pub(crate) fn show(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    eased: f32,
    backdrop_eased: f32,
    interactive: bool,
) -> ModalMetrics {
    let (metrics, action) = show_illustrated_modal(
        app,
        ui,
        &DATA_PRIVACY_SPEC,
        eased,
        backdrop_eased,
        interactive,
    );
    if interactive {
        match action {
            Some(IllustratedModalAction::Cta) => dismiss_and_continue(app),
            Some(IllustratedModalAction::Close) => dismiss_only(app),
            None => {}
        }
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
        assert!(rendered_text.contains("Your notebooks stay private"));
        assert!(rendered_text.contains("How your data is saved and protected:"));
        assert!(rendered_text.contains("Saved locally on your device"));
        assert!(rendered_text.contains("Private isolated workspaces"));
        assert!(rendered_text.contains("Optional self-hosted sync"));
        assert!(rendered_text.contains("One-click .pcraftpkg backups"));
        assert!(rendered_text.contains("Continue to Create Notebook"));

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
    fn data_privacy_modal_shows_on_first_notebook_click_and_proceeds_to_new_notebook() {
        let dir = std::env::temp_dir().join(format!(
            "papercraft_data_privacy_ui_{}_{}",
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

        let mut session = Session::open_desktop_library(dir);
        session
            .execute("profile.dismissWelcomeBanner", &serde_json::json!({}))
            .unwrap();
        session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "Alyx" }),
            )
            .unwrap();
        session
            .execute(
                "profile.setStartupPicker",
                &serde_json::json!({ "skip": true }),
            )
            .unwrap();

        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        assert_eq!(app.view.active_modal, None);
        assert!(app.session.should_show_first_notebook_data_notice());

        // First click to create a notebook opens FirstNotebookDataNotice instead of NewNotebook,
        // while still preserving the active folder scope for when the user continues.
        app.view.library_folder_filter = Some("Study".to_string());
        crate::modal::open(&mut app, ActiveModal::NewNotebook);
        assert_eq!(
            app.view.active_modal,
            Some(ActiveModal::FirstNotebookDataNotice)
        );
        assert_eq!(app.view.new_notebook_folder, "Study");

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;
        let cta_rect = settle_and_assert_layout(&ctx, &mut app, viewport, &mut time);

        // Outside clicks and Escape must not dismiss the notice accidentally.
        click_pos(&ctx, &mut app, viewport, &mut time, pos2(16.0, 16.0));
        assert_eq!(
            app.view.active_modal,
            Some(ActiveModal::FirstNotebookDataNotice)
        );
        assert!(app.session.should_show_first_notebook_data_notice());

        press_escape(&ctx, &mut app, viewport, &mut time);
        assert_eq!(
            app.view.active_modal,
            Some(ActiveModal::FirstNotebookDataNotice)
        );

        // Clicking CTA marks notice seen and immediately opens the New Notebook wizard.
        click_pos(&ctx, &mut app, viewport, &mut time, cta_rect.center());
        assert!(!app.session.should_show_first_notebook_data_notice());
        assert_eq!(app.view.active_modal, Some(ActiveModal::NewNotebook));
        assert_eq!(app.view.new_notebook_folder, "Study");

        // Subsequent New Notebook clicks open NewNotebook directly.
        crate::modal::close(&mut app);
        crate::modal::open(&mut app, ActiveModal::NewNotebook);
        assert_eq!(app.view.active_modal, Some(ActiveModal::NewNotebook));
    }

    #[test]
    fn data_privacy_modal_close_button_dismisses_without_opening_wizard() {
        let mut session = Session::open_web_cache(None);
        session
            .execute("profile.dismissWelcomeBanner", &serde_json::json!({}))
            .unwrap();
        session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "WebUser" }),
            )
            .unwrap();
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());

        crate::modal::open(&mut app, ActiveModal::NewNotebook);
        assert_eq!(
            app.view.active_modal,
            Some(ActiveModal::FirstNotebookDataNotice)
        );

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;
        let mut close_center = Pos2::ZERO;
        for _ in 0..8 {
            time += 0.08;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                time: Some(time),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                let (metrics, _) =
                    show_illustrated_modal(&mut app, ui, &DATA_PRIVACY_SPEC, 1.0, 1.0, true);
                close_center = metrics.close_btn_rect.center();
            });
            out.drop_without_applying_deltas();
        }
        assert!(close_center.x > 0.0);

        click_pos(&ctx, &mut app, viewport, &mut time, close_center);
        assert!(!app.session.should_show_first_notebook_data_notice());
        assert_eq!(app.view.active_modal, None);
    }
}
