//! Software Update modal rendering release notes, download progress, and restart confirmation.

use egui::{CornerRadius, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2};
use papercraft_updater::{InstallCapability, UpdateStatus, current_platform_label, format_bytes};

use crate::PaperCraftApp;
use crate::icons::{Icon, paint_icon};
use crate::modal::{primary_action_button, secondary_action_button};
use crate::theme::Tokens;

/// Renders the Software Update dialog body. Returns `true` when the modal should close.
pub(crate) fn render_body(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    let status = app.updater.status().clone();
    let current_ver = app
        .updater
        .config()
        .map_or_else(crate::display_version_label, |c| {
            papercraft_doc::format_alpha_version(&c.current_version.to_string())
        });
    let is_custom_feed = app.updater.config().is_some_and(|c| c.is_custom_feed);

    match status {
        UpdateStatus::Idle | UpdateStatus::Checking { .. } => {
            draw_status_box(
                ui,
                t.panel_bg,
                t.border,
                t.accent,
                Icon::RotateCw,
                "Checking for updates...",
                &format!(
                    "Current version: {current_ver} · Platform: {}",
                    current_platform_label()
                ),
                true,
                t,
            );
        }
        UpdateStatus::UpToDate { latest_version } => {
            let is_ahead = app
                .updater
                .config()
                .is_some_and(|cfg| cfg.current_version > latest_version);
            let subtitle = if is_ahead {
                format!(
                    "Running {current_ver} — newer than the latest published release (v{latest_version})."
                )
            } else {
                format!("Running {current_ver} (latest published release: v{latest_version}).")
            };
            draw_status_box(
                ui,
                t.success_bg,
                t.success_border,
                t.success_fg,
                Icon::CheckCircle,
                "PaperCraft is up to date",
                &subtitle,
                false,
                t,
            );
        }
        UpdateStatus::Available(update) => {
            draw_release_header_card(
                ui,
                &papercraft_doc::format_alpha_version(&update.current_version.to_string()),
                &format!("v{}", update.version),
                update.asset.size_bytes,
                update.is_custom_feed || is_custom_feed,
                t,
            );
            ui.add_space(8.0);
            draw_release_notes_box(ui, &update.notes, t);
            match &update.capability {
                InstallCapability::Supported { .. } => {}
                InstallCapability::ReadOnly { reason, .. }
                | InstallCapability::UnsupportedLayout { reason } => {
                    ui.add_space(8.0);
                    draw_notice_box(ui, t.toolbar_bg, t.border, t.warning_fg, reason, t);
                }
            }
        }
        UpdateStatus::Downloading {
            update,
            downloaded_bytes,
            total_bytes,
        } => {
            draw_release_header_card(
                ui,
                &papercraft_doc::format_alpha_version(&update.current_version.to_string()),
                &format!("v{}", update.version),
                update.asset.size_bytes,
                update.is_custom_feed || is_custom_feed,
                t,
            );
            ui.add_space(8.0);
            draw_download_progress_box(ui, downloaded_bytes, total_bytes, t);
            if !update.notes.trim().is_empty() {
                ui.add_space(8.0);
                draw_release_notes_box(ui, &update.notes, t);
            }
        }
        UpdateStatus::ReadyToInstall { update, .. } => {
            draw_release_header_card(
                ui,
                &papercraft_doc::format_alpha_version(&update.current_version.to_string()),
                &format!("v{}", update.version),
                update.asset.size_bytes,
                update.is_custom_feed || is_custom_feed,
                t,
            );
            ui.add_space(8.0);
            draw_status_box(
                ui,
                t.success_bg,
                t.success_border,
                t.success_fg,
                Icon::CheckCircle,
                "Download verified (SHA-256)",
                "PaperCraft will save any open notebooks, install the new version, refresh your Desktop shortcut, and relaunch automatically.",
                false,
                t,
            );
            if !update.notes.trim().is_empty() {
                ui.add_space(8.0);
                draw_release_notes_box(ui, &update.notes, t);
            }
        }
        UpdateStatus::NoPlatformAsset {
            latest_version,
            platform,
            ..
        } => {
            draw_notice_box(
                ui,
                t.toolbar_bg,
                t.border,
                t.warning_fg,
                &format!(
                    "Release v{latest_version} was found, but no auto-update package is listed for `{platform}`. You can download it manually from GitHub Releases."
                ),
                t,
            );
        }
        UpdateStatus::Error { message, .. } => {
            draw_notice_box(ui, t.danger_bg, t.danger_border, t.danger_fg, &message, t);
        }
    }

    false
}

/// Renders the Software Update dialog footer actions. Returns `true` when closed.
pub(crate) fn render_footer(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    let status = app.updater.status().clone();
    let mut should_close = false;

    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            match status {
                UpdateStatus::Checking { .. } => {
                    if secondary_action_button(ui, "Close", true, t).clicked() {
                        should_close = true;
                    }
                }
                UpdateStatus::Available(update) => {
                    let can_install =
                        matches!(update.capability, InstallCapability::Supported { .. });
                    if can_install {
                        if primary_action_button(ui, "Download Update", true, t).clicked() {
                            app.start_update_download(ui.ctx());
                        }
                    } else if primary_action_button(ui, "Open Release Page", true, t).clicked() {
                        app.open_external_url(&update.release_url);
                    }
                    if secondary_action_button(ui, "Later", true, t).clicked() {
                        should_close = true;
                    }
                }
                UpdateStatus::Downloading { .. } => {
                    if secondary_action_button(ui, "Cancel Download", true, t).clicked() {
                        app.cancel_update_download();
                    }
                }
                UpdateStatus::ReadyToInstall { .. } => {
                    if primary_action_button(ui, "Install & Restart", true, t).clicked()
                        && app.apply_ready_update_and_restart(ui.ctx()).is_ok()
                    {
                        should_close = true;
                    }
                    if secondary_action_button(ui, "Later", true, t).clicked() {
                        should_close = true;
                    }
                }
                UpdateStatus::UpToDate { .. } | UpdateStatus::Idle => {
                    if primary_action_button(ui, "Check Again", true, t).clicked() {
                        app.trigger_update_check(false, ui.ctx());
                    }
                    if secondary_action_button(ui, "Close", true, t).clicked() {
                        should_close = true;
                    }
                }
                UpdateStatus::NoPlatformAsset { release_url, .. }
                | UpdateStatus::Error { release_url, .. } => {
                    if primary_action_button(ui, "Check Again", true, t).clicked() {
                        app.trigger_update_check(false, ui.ctx());
                    }
                    if secondary_action_button(ui, "Open Release Page", true, t).clicked() {
                        app.open_external_url(&release_url);
                    }
                    if secondary_action_button(ui, "Close", true, t).clicked() {
                        should_close = true;
                    }
                }
            }
        });
    });

    should_close
}

fn draw_release_header_card(
    ui: &mut egui::Ui,
    current_ver: &str,
    new_ver: &str,
    size_bytes: u64,
    is_custom_feed: bool,
    t: &Tokens,
) {
    egui::Frame::NONE
        .fill(t.panel_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                paint_icon(ui.painter(), icon_rect, Icon::MoveDown, t.accent);
                ui.label(
                    RichText::new(format!("{current_ver} -> {new_ver}"))
                        .size(14.0)
                        .strong()
                        .color(t.text),
                );
                if is_custom_feed {
                    egui::Frame::NONE
                        .fill(t.accent_soft)
                        .stroke(Stroke::new(1.0, t.accent))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(6, 2))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("Dev feed")
                                    .size(10.5)
                                    .strong()
                                    .color(t.accent),
                            );
                        });
                }
            });
            ui.add_space(4.0);
            let size_label = if size_bytes > 0 {
                format!(
                    "{} · {}",
                    current_platform_label(),
                    format_bytes(size_bytes)
                )
            } else {
                current_platform_label()
            };
            ui.label(RichText::new(size_label).size(11.5).color(t.text_muted));
        });
}

fn draw_release_notes_box(ui: &mut egui::Ui, notes: &str, t: &Tokens) {
    let clean = notes.trim();
    let display = if clean.is_empty() {
        "No release notes were included with this build."
    } else {
        clean
    };
    egui::Frame::NONE
        .fill(t.panel_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("What's new")
                    .size(12.0)
                    .strong()
                    .color(t.text),
            );
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .id_salt("papercraft_update_notes_scroll")
                .max_height(150.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(display).size(11.5).color(t.text_muted))
                            .wrap(),
                    );
                });
        });
}

fn draw_download_progress_box(
    ui: &mut egui::Ui,
    downloaded_bytes: u64,
    total_bytes: u64,
    t: &Tokens,
) {
    let is_verifying = total_bytes > 0 && downloaded_bytes >= total_bytes;
    let status_label = if is_verifying {
        "Verifying download (SHA-256)..."
    } else {
        "Downloading update..."
    };
    let readout = if total_bytes > 0 {
        let frac = (downloaded_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0);
        let pct = (frac * 100.0).round() as u32;
        format!(
            "{} / {} · {pct}%",
            format_bytes(downloaded_bytes),
            format_bytes(total_bytes)
        )
    } else {
        format_bytes(downloaded_bytes)
    };

    egui::Frame::NONE
        .fill(t.panel_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(status_label)
                        .size(12.0)
                        .strong()
                        .color(t.text),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(readout)
                            .size(11.5)
                            .monospace()
                            .strong()
                            .color(t.accent),
                    );
                });
            });
            ui.add_space(7.0);
            paint_progress_track(ui, downloaded_bytes, total_bytes, t);
        });
}

fn paint_progress_track(ui: &mut egui::Ui, downloaded_bytes: u64, total_bytes: u64, t: &Tokens) {
    let track_h = 10.0_f32;
    let radius = 5_u8;
    let (bar_rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), track_h), Sense::hover());
    if !ui.is_rect_visible(bar_rect) {
        return;
    }

    let p = ui.painter();
    p.rect_filled(bar_rect, CornerRadius::same(radius), t.toolbar_bg);
    p.rect_stroke(
        bar_rect,
        CornerRadius::same(radius),
        Stroke::new(1.0, t.border),
        StrokeKind::Inside,
    );

    if total_bytes > 0 {
        let target_frac = (downloaded_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0);
        let anim_frac = ui
            .ctx()
            .animate_value_with_time(
                ui.make_persistent_id("papercraft_update_progress"),
                target_frac,
                0.08,
            )
            .clamp(0.0, 1.0);
        let fill_w = bar_rect.width() * anim_frac;
        if fill_w > 0.5 {
            let fill_rect = Rect::from_min_size(bar_rect.min, Vec2::new(fill_w, bar_rect.height()));
            let fill_radius = (fill_w * 0.5).min(f32::from(radius)).round() as u8;
            p.rect_filled(fill_rect, CornerRadius::same(fill_radius), t.accent);
        }
    } else {
        let time_s = ui.ctx().input(|i| i.time);
        let seg_w = (bar_rect.width() * 0.30).max(24.0).min(bar_rect.width());
        let travel = bar_rect.width() + seg_w;
        let phase = ((time_s * 0.85) % 1.0) as f32;
        let left = bar_rect.left() - seg_w + travel * phase;
        let seg_rect = Rect::from_min_size(pos2(left, bar_rect.top()), Vec2::new(seg_w, track_h));
        p.with_clip_rect(bar_rect)
            .rect_filled(seg_rect, CornerRadius::same(radius), t.accent);
        ui.ctx().request_repaint();
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_status_box(
    ui: &mut egui::Ui,
    bg: egui::Color32,
    border: egui::Color32,
    fg: egui::Color32,
    icon: Icon,
    title: &str,
    body: &str,
    spinner: bool,
    t: &Tokens,
) {
    egui::Frame::NONE
        .fill(bg)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if spinner {
                    ui.add(egui::Spinner::new().size(15.0).color(fg));
                } else {
                    let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                    paint_icon(ui.painter(), ir, icon, fg);
                }
                ui.label(RichText::new(title).size(12.5).strong().color(fg));
            });
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(body).size(11.5).color(t.text_muted)).wrap());
        });
}

fn draw_notice_box(
    ui: &mut egui::Ui,
    bg: egui::Color32,
    border: egui::Color32,
    fg: egui::Color32,
    text: &str,
    _t: &Tokens,
) {
    egui::Frame::NONE
        .fill(bg)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
                paint_icon(ui.painter(), ir, Icon::AlertCircle, fg);
                ui.add(egui::Label::new(RichText::new(text).size(11.5).color(fg)).wrap());
            });
        });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use egui::{Pos2, RawInput, Shape};
    use papercraft_doc::UiTheme;

    fn collect_text(shapes: &[egui::epaint::ClippedShape]) -> String {
        let mut out = String::new();
        fn visit(shape: &Shape, buf: &mut String) {
            match shape {
                Shape::Text(t) => {
                    buf.push_str(t.galley.text());
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
        for c in shapes {
            visit(&c.shape, &mut out);
        }
        out
    }

    #[test]
    fn download_progress_box_renders_determinate_and_indeterminate_states_cleanly() {
        let cases = [
            (0_u64, 1000_u64, "Downloading update...", "0%"),
            (500_u64, 1000_u64, "Downloading update...", "50%"),
            (
                1000_u64,
                1000_u64,
                "Verifying download (SHA-256)...",
                "100%",
            ),
            (2048_u64, 0_u64, "Downloading update...", "2 KB"),
        ];

        for theme in [UiTheme::Light, UiTheme::Dark] {
            let tokens = Tokens::for_theme(theme);
            let ctx = egui::Context::default();
            for (done, total, expected_title, expected_readout) in cases {
                let raw = RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(480.0, 260.0))),
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| {
                    draw_download_progress_box(ui, done, total, &tokens);
                });
                let text = collect_text(&out.shapes);
                out.drop_without_applying_deltas();
                assert!(
                    text.contains(expected_title),
                    "missing `{expected_title}` in `{text}`"
                );
                assert!(
                    text.contains(expected_readout),
                    "missing `{expected_readout}` in `{text}`"
                );
            }
        }
    }
}
