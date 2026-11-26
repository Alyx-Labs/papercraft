//! Minimalist, plain-English control UI for PaperCraft Server.

use egui::{Color32, CornerRadius, Margin, Sense, Stroke, Vec2};
use papercraft_doc::UiTheme;

use crate::controller::{
    ControlTab, NoticeBanner, NoticeTone, ServerController, ServerStatus, StatsSnapshot,
    UsersViewMode, account_delete_confirmation_matches, account_delete_confirmation_phrase,
};
use crate::icons::{
    Icon, SwitchOption, icon_button, icon_button_width, paint_icon, segmented_switch,
    toggle_switch_row,
};
use crate::settings::{
    MAX_ACCESS_TTL_HOURS, MAX_MAX_NOTEBOOK_MB, MAX_REFRESH_TTL_DAYS, MIN_ACCESS_TTL_HOURS,
    MIN_MAX_NOTEBOOK_MB, MIN_REFRESH_TTL_DAYS, NetworkScope, format_bytes,
};
use crate::theme::Tokens;
use papercraft_server_cli::db::{
    ACCOUNT_TRASH_RETENTION_DAYS, PasswordResetOutcome, TrashedUserSummary, UserSummary,
};

const CARD_RADIUS: f32 = 10.0;
const CONTROL_RADIUS: f32 = 7.0;
const INPUT_HEIGHT: f32 = 30.0;

#[derive(Default)]
pub struct UiActions {
    pub hide_to_tray: bool,
    pub quit_app: bool,
    pub install_update: bool,
}

/// Renders the entire PaperCraft Server control window and returns requested window actions.
pub fn render_window(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) -> UiActions {
    let update_status = ctrl.updater.status().clone();
    render_window_with_update_status(ctrl, ui, t, &update_status)
}

fn render_window_with_update_status(
    ctrl: &mut ServerController,
    ui: &mut egui::Ui,
    t: &Tokens,
    update_status: &papercraft_updater::UpdateStatus,
) -> UiActions {
    let mut actions = UiActions::default();
    let ctx = ui.ctx().clone();

    egui::Panel::top("bnc_srv_header")
        .frame(
            egui::Frame::new()
                .fill(t.header_bg)
                .inner_margin(Margin::symmetric(16, 12))
                .stroke(Stroke::new(1.0, t.header_border)),
        )
        .show(ui, |ui| {
            render_header(ctrl, ui, t, &mut actions);
        });

    egui::Panel::bottom("bnc_srv_footer")
        .frame(
            egui::Frame::new()
                .fill(t.toolbar_bg)
                .inner_margin(Margin::symmetric(16, 10))
                .stroke(Stroke::new(1.0, t.border)),
        )
        .show(ui, |ui| {
            render_footer(ui, t, &mut actions);
        });

    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(t.canvas_bg)
                .inner_margin(Margin::symmetric(16, 12)),
        )
        .show(ui, |ui| {
            render_primary_tab_switch(ctrl, ui, t);
            ui.add_space(10.0);

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    match ctrl.active_tab {
                        ControlTab::Server => {
                            render_status_card(ctrl, ui, t);
                            render_banners(ctrl, ui, t);
                            render_essential_settings_card(ctrl, &ctx, ui, t);
                            render_advanced_card(ctrl, ui, t);
                            render_updates_card_with_status(
                                ctrl,
                                update_status,
                                ui,
                                t,
                                &mut actions,
                            );
                        }
                        ControlTab::Users => {
                            render_users_view(ctrl, ui, t);
                        }
                    }
                });
        });

    ctrl.auto_save_settings();
    actions
}

fn render_primary_tab_switch(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    let users_label = format!("Users ({})", ctrl.users_snapshot.len());
    let options = [
        SwitchOption::new(
            Some(Icon::Server),
            "Server",
            "Server status, connection addresses, and settings",
        ),
        SwitchOption::new(
            Some(Icon::Account),
            &users_label,
            "List all registered accounts stored on this server",
        ),
    ];
    let active_idx = match ctrl.active_tab {
        ControlTab::Server => 0,
        ControlTab::Users => 1,
    };
    if let Some(clicked) = segmented_switch(ui, "bnc_srv_primary_tabs", &options, active_idx, t) {
        let next = if clicked == 0 {
            ControlTab::Server
        } else {
            ControlTab::Users
        };
        ctrl.set_active_tab(next);
    }
}

fn render_header(
    ctrl: &mut ServerController,
    ui: &mut egui::Ui,
    t: &Tokens,
    actions: &mut UiActions,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;

        let (badge_rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
        let badge_border = if t.is_dark {
            t.accent.gamma_multiply(0.45)
        } else {
            t.header_active_bg
        };
        ui.painter().rect(
            badge_rect,
            CornerRadius::same(9),
            t.header_active_bg,
            Stroke::new(1.0, badge_border),
            egui::StrokeKind::Inside,
        );
        paint_icon(
            ui.painter(),
            badge_rect.shrink(8.0),
            Icon::Server,
            t.header_active_fg,
        );

        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(
                egui::RichText::new("PaperCraft Server")
                    .size(15.5)
                    .strong()
                    .color(t.header_fg),
            );
            ui.label(
                egui::RichText::new("Personal sync server in your system tray")
                    .size(11.5)
                    .color(t.header_fg_muted),
            );
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if crate::icons::header_icon_button(ui, Icon::ChevronDown, Some("Hide"), false, true, t)
                .on_hover_text("Hide this window to the system tray (server stays running)")
                .clicked()
            {
                actions.hide_to_tray = true;
            }

            let (theme_icon, next_theme, theme_tip) = match ctrl.draft_settings.theme {
                UiTheme::Light => (Icon::Moon, UiTheme::Dark, "Switch to Dark theme"),
                UiTheme::Dark => (Icon::Sun, UiTheme::Light, "Switch to Light theme"),
            };
            if crate::icons::header_icon_button(ui, theme_icon, None, false, true, t)
                .on_hover_text(theme_tip)
                .clicked()
            {
                ctrl.draft_settings.theme = next_theme;
            }
        });
    });
}

fn render_status_card(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    draw_card(ui, t, |ui| {
        ui.horizontal(|ui| {
            draw_status_pill(ui, &ctrl.status, t);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if ctrl.is_running() {
                    if danger_button(ui, Icon::Square, "Stop", true, t)
                        .on_hover_text("Gracefully save the database and stop accepting syncs")
                        .clicked()
                    {
                        ctrl.stop_server();
                    }
                } else if primary_button(ui, Icon::Server, "Start Server", true, t)
                    .on_hover_text("Start the PaperCraft sync server now")
                    .clicked()
                {
                    ctrl.start_server();
                }
            });
        });

        ui.add_space(10.0);
        render_connection_urls_box(ctrl, ui, t);

        if let Some(stats) = ctrl.stats_snapshot {
            ui.add_space(10.0);
            draw_hairline(ui, t);
            ui.add_space(8.0);
            render_stats_row(ctrl, ui, stats, t);
            ui.add_space(8.0);
            render_local_discovery_row(ctrl, ui, t);
        }

        if let ServerStatus::Error(ref err) = ctrl.status {
            ui.add_space(10.0);
            draw_banner(
                ui,
                &NoticeBanner {
                    tone: NoticeTone::Error,
                    text: err.clone(),
                },
                false,
                t,
            );
        }
    });
}

fn render_connection_urls_box(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    let urls = ctrl.connection_urls();
    let running = ctrl.is_running();

    egui::Frame::new()
        .fill(t.toolbar_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;

            let heading = if running {
                "How to connect PaperCraft"
            } else {
                "Server address (start the server to connect)"
            };
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                paint_icon(
                    ui.painter(),
                    ir,
                    Icon::Link,
                    if running { t.accent } else { t.text_muted },
                );
                ui.label(
                    egui::RichText::new(heading)
                        .size(12.0)
                        .strong()
                        .color(if running { t.text } else { t.text_muted }),
                );
            });

            let local_label = if urls.local_is_loopback {
                "On this computer"
            } else {
                "Server address"
            };
            if render_url_copy_row(
                ui,
                local_label,
                &urls.local_url,
                ctrl.was_recently_copied(&urls.local_url),
                t,
            ) {
                ctrl.copy_url(&urls.local_url);
            }

            if let Some(ref net_url) = urls.network_url {
                if render_url_copy_row(
                    ui,
                    "On other devices (Wi-Fi / VPN)",
                    net_url,
                    ctrl.was_recently_copied(net_url),
                    t,
                ) {
                    ctrl.copy_url(net_url);
                }
            } else if ctrl.draft_settings.network_scope == NetworkScope::LocalNetwork {
                ui.label(
                    egui::RichText::new(format!(
                        "Other devices can connect using http://<this-PC-IP>:{}",
                        ctrl.draft_settings.port
                    ))
                    .size(11.0)
                    .color(t.text_muted),
                );
            }

            ui.label(
                egui::RichText::new(
                    "Paste this address into PaperCraft -> Sign In -> Self-Hosted Server.",
                )
                .size(11.0)
                .color(t.text_muted),
            );
        });
}

fn render_url_copy_row(
    ui: &mut egui::Ui,
    label: &str,
    url: &str,
    copied: bool,
    t: &Tokens,
) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(egui::RichText::new(label).size(11.5).color(t.text_muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (icon, btn_lbl) = if copied {
                (Icon::Check, "Copied")
            } else {
                (Icon::Copy, "Copy")
            };
            if icon_button(ui, icon, Some(btn_lbl), copied, true, t)
                .on_hover_text("Copy server URL to clipboard")
                .clicked()
            {
                clicked = true;
            }
            ui.label(
                egui::RichText::new(url)
                    .size(12.0)
                    .monospace()
                    .strong()
                    .color(t.text),
            );
        });
    });
    clicked
}

fn render_stats_row(
    ctrl: &mut ServerController,
    ui: &mut egui::Ui,
    stats: StatsSnapshot,
    t: &Tokens,
) {
    let accounts_value = if stats.trashed_user_count > 0 {
        format!(
            "{} ({} in Trash)",
            stats.user_count, stats.trashed_user_count
        )
    } else {
        stats.user_count.to_string()
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        draw_metric_cell(ui, Some(Icon::Account), "Accounts", &accounts_value, t);
        draw_metric_cell(
            ui,
            Some(Icon::Notebook),
            "Notebooks",
            &stats.active_notebook_count.to_string(),
            t,
        );
        draw_metric_cell(
            ui,
            None,
            "Active devices",
            &stats.active_session_count.to_string(),
            t,
        );
        draw_metric_cell(
            ui,
            None,
            "Database size",
            &format_bytes(stats.db_size_bytes),
            t,
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, Icon::RotateCw, None, false, true, t)
                .on_hover_text("Refresh database counts")
                .clicked()
            {
                ctrl.refresh_stats();
            }
        });
    });
}

fn render_local_discovery_row(ctrl: &ServerController, ui: &mut egui::Ui, t: &Tokens) {
    let counts = ctrl.local_discovery;
    egui::Frame::new()
        .fill(t.toolbar_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 18.0;
                draw_metric_cell(
                    ui,
                    Some(Icon::Account),
                    "Unsynced local accounts",
                    &counts.unsynced_local_accounts.to_string(),
                    t,
                );
                draw_metric_cell(
                    ui,
                    Some(Icon::Notebook),
                    "Local notebooks",
                    &counts.total_local_notebooks.to_string(),
                    t,
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(
                            "Counts only (sync prompts stay hidden until signed in)",
                        )
                        .size(11.0)
                        .color(t.text_muted),
                    );
                });
            });
        });
}

fn render_banners(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    if ctrl.has_pending_restart() {
        egui::Frame::new()
            .fill(t.accent_soft)
            .stroke(Stroke::new(1.0, t.accent))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                    paint_icon(ui.painter(), ir, Icon::Info, t.accent);
                    ui.label(
                        egui::RichText::new("Settings changed — restart the server to apply.")
                            .size(12.0)
                            .strong()
                            .color(t.text),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if primary_button(ui, Icon::RotateCw, "Apply & Restart", true, t).clicked()
                        {
                            ctrl.restart_server();
                        }
                        if secondary_button(ui, "Revert", true, t).clicked() {
                            ctrl.revert_draft();
                        }
                    });
                });
            });
    }

    if let Some(banner) = ctrl.notice_banner.clone()
        && draw_banner(ui, &banner, true, t)
    {
        ctrl.notice_banner = None;
    }
}

fn render_update_available_box(
    ctrl: &mut ServerController,
    update: &papercraft_updater::AvailableUpdate,
    is_custom_feed: bool,
    ui: &mut egui::Ui,
    t: &Tokens,
) {
    use papercraft_updater::{InstallCapability, format_bytes as upd_format_bytes};

    egui::Frame::new()
        .fill(t.accent_soft)
        .stroke(Stroke::new(1.0, t.accent))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                paint_icon(ui.painter(), ir, Icon::MoveDown, t.accent);
                ui.label(
                    egui::RichText::new(format!(
                        "Update available: v{} (current: v{})",
                        update.version, update.current_version
                    ))
                    .size(12.5)
                    .strong()
                    .color(t.text),
                );
                if update.is_custom_feed && !is_custom_feed {
                    draw_compact_pill(ui, None, "Dev feed", t.field_bg, t.accent, t.accent);
                }
            });
            if !update.notes.trim().is_empty() {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(update.notes.trim())
                            .size(11.0)
                            .color(t.text_muted),
                    )
                    .wrap(),
                );
            }
            match &update.capability {
                InstallCapability::Supported { .. } => {
                    let size_info = if update.asset.size_bytes > 0 {
                        format!(
                            "Download size: {}",
                            upd_format_bytes(update.asset.size_bytes)
                        )
                    } else {
                        "Verified HTTPS + SHA-256 update".to_string()
                    };
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(size_info)
                                .size(11.0)
                                .color(t.text_muted),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if primary_button(ui, Icon::MoveDown, "Download Update", true, t)
                                .clicked()
                            {
                                ctrl.start_update_download();
                            }
                        });
                    });
                }
                InstallCapability::ReadOnly { reason, .. }
                | InstallCapability::UnsupportedLayout { reason } => {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(reason).size(11.0).color(t.warning_fg),
                        )
                        .wrap(),
                    );
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(
                                ui,
                                Icon::ExternalLink,
                                Some("Open Release Page"),
                                false,
                                true,
                                t,
                            )
                            .clicked()
                            {
                                ctrl.open_update_release_page();
                            }
                        });
                    });
                }
            }
        });
}

fn render_update_downloading_box(
    ctrl: &mut ServerController,
    update: &papercraft_updater::AvailableUpdate,
    downloaded_bytes: u64,
    total_bytes: u64,
    ui: &mut egui::Ui,
    t: &Tokens,
) {
    use papercraft_updater::format_bytes as upd_format_bytes;

    let is_verifying = total_bytes > 0 && downloaded_bytes >= total_bytes;
    let title = if is_verifying {
        format!("Verifying v{} (SHA-256)...", update.version)
    } else {
        format!("Downloading v{}...", update.version)
    };
    let readout = if total_bytes > 0 {
        let frac = (downloaded_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0);
        let pct = (frac * 100.0).round() as u32;
        format!(
            "{} / {} · {pct}%",
            upd_format_bytes(downloaded_bytes),
            upd_format_bytes(total_bytes)
        )
    } else {
        upd_format_bytes(downloaded_bytes)
    };
    egui::Frame::new()
        .fill(t.accent_soft)
        .stroke(Stroke::new(1.0, t.accent))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(egui::RichText::new(title).size(12.0).strong().color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if secondary_button(ui, "Cancel", true, t).clicked() {
                        ctrl.cancel_update_download();
                    }
                    ui.label(
                        egui::RichText::new(readout)
                            .size(11.5)
                            .monospace()
                            .strong()
                            .color(t.accent),
                    );
                });
            });
            draw_update_progress_bar(ui, downloaded_bytes, total_bytes, t);
        });
}

fn render_update_ready_box(
    update: &papercraft_updater::AvailableUpdate,
    ui: &mut egui::Ui,
    t: &Tokens,
    actions: &mut UiActions,
) {
    egui::Frame::new()
        .fill(t.success_bg)
        .stroke(Stroke::new(1.0, t.success_border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                paint_icon(ui.painter(), ir, Icon::CheckCircle, t.success_fg);
                ui.label(
                    egui::RichText::new(format!(
                        "Update v{} verified and ready to install",
                        update.version
                    ))
                    .size(12.0)
                    .strong()
                    .color(t.success_fg),
                );
            });
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "Stops sync server safely, swaps the binary, refreshes your Desktop shortcut, and relaunches.",
                    )
                    .size(11.0)
                    .color(t.text_muted),
                )
                .wrap(),
            );
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(
                        ui,
                        Icon::RotateCw,
                        "Stop Server, Install & Restart",
                        true,
                        t,
                    )
                    .clicked()
                    {
                        actions.install_update = true;
                    }
                });
            });
        });
}

fn draw_update_progress_bar(
    ui: &mut egui::Ui,
    downloaded_bytes: u64,
    total_bytes: u64,
    t: &Tokens,
) {
    let track_h = 10.0_f32;
    let radius = 5_u8;
    let (bar_rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), track_h), Sense::hover());
    if !ui.is_rect_visible(bar_rect) {
        return;
    }

    let p = ui.painter();
    p.rect_filled(bar_rect, CornerRadius::same(radius), t.field_bg);
    p.rect_stroke(
        bar_rect,
        CornerRadius::same(radius),
        Stroke::new(1.0, t.border),
        egui::StrokeKind::Inside,
    );

    if total_bytes > 0 {
        let target_frac = (downloaded_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0);
        let anim_frac = ui
            .ctx()
            .animate_value_with_time(
                ui.make_persistent_id("papercraft_srv_update_progress"),
                target_frac,
                0.08,
            )
            .clamp(0.0, 1.0);
        let fill_w = bar_rect.width() * anim_frac;
        if fill_w > 0.5 {
            let fill_rect =
                egui::Rect::from_min_size(bar_rect.min, Vec2::new(fill_w, bar_rect.height()));
            let fill_radius = (fill_w * 0.5).min(f32::from(radius)).round() as u8;
            p.rect_filled(fill_rect, CornerRadius::same(fill_radius), t.accent);
        }
    } else {
        let time_s = ui.ctx().input(|i| i.time);
        let seg_w = (bar_rect.width() * 0.30).max(24.0).min(bar_rect.width());
        let travel = bar_rect.width() + seg_w;
        let phase = ((time_s * 0.85) % 1.0) as f32;
        let left = bar_rect.left() - seg_w + travel * phase;
        let seg_rect =
            egui::Rect::from_min_size(egui::pos2(left, bar_rect.top()), Vec2::new(seg_w, track_h));
        p.with_clip_rect(bar_rect)
            .rect_filled(seg_rect, CornerRadius::same(radius), t.accent);
        ui.ctx().request_repaint();
    }
}

fn render_updates_card_with_status(
    ctrl: &mut ServerController,
    status: &papercraft_updater::UpdateStatus,
    ui: &mut egui::Ui,
    t: &Tokens,
    actions: &mut UiActions,
) {
    use papercraft_updater::UpdateStatus;

    let current_ver = ctrl.updater.config().map_or_else(
        || env!("CARGO_PKG_VERSION").to_string(),
        |cfg| cfg.current_version.to_string(),
    );
    let is_custom_feed = ctrl.updater.config().is_some_and(|cfg| cfg.is_custom_feed);
    let busy = matches!(
        status,
        UpdateStatus::Checking { .. } | UpdateStatus::Downloading { .. }
    );

    draw_card(ui, t, |ui| {
        draw_section_header(
            ui,
            Icon::MoveDown,
            "App Updates & System",
            "Updates, Desktop shortcut, and automatic startup when you sign in.",
            t,
        );
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(
                egui::RichText::new(format!(
                    "Installed version: v{current_ver} · {}",
                    papercraft_updater::PUBLISHER_NAME
                ))
                .size(12.0)
                .strong()
                .color(t.text),
            );
            if is_custom_feed {
                draw_compact_pill(ui, None, "Dev feed", t.accent_soft, t.accent, t.accent);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let btn_lbl = if matches!(status, UpdateStatus::Checking { .. }) {
                    "Checking..."
                } else {
                    "Check for Updates"
                };
                if icon_button(ui, Icon::RotateCw, Some(btn_lbl), false, !busy, t)
                    .on_hover_text("Check GitHub Releases for a newer PaperCraft Server version")
                    .clicked()
                {
                    ctrl.check_for_updates();
                }
            });
        });

        render_update_status_details(ctrl, status, is_custom_feed, ui, t, actions);

        ui.add_space(10.0);
        draw_hairline(ui, t);
        ui.add_space(8.0);

        ui.checkbox(
            &mut ctrl.draft_settings.auto_check_updates,
            egui::RichText::new("Check for updates automatically on startup")
                .size(12.0)
                .color(t.text),
        );
        ui.label(
            egui::RichText::new(
                "Checks GitHub Releases quietly when PaperCraft Server starts. Turn off for completely offline use.",
            )
            .size(11.0)
            .color(t.text_muted),
        );

        render_desktop_shortcut_section(ctrl, ui, t);
        render_autostart_section(ctrl, ui, t);
    });
}

fn render_desktop_shortcut_section(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    ui.add_space(8.0);
    let mut shortcut_on = ctrl.draft_settings.auto_create_desktop_shortcut;
    let shortcut_resp = toggle_switch_row(
        ui,
        "srv_desktop_shortcut_switch",
        "Create Desktop shortcut automatically",
        "Keep a Desktop shortcut pointing to PaperCraft Server and refresh it automatically after updates.",
        &mut shortcut_on,
        t,
    );
    if shortcut_resp.changed() {
        ctrl.set_desktop_shortcut_enabled(shortcut_on);
    }
    ui.add(
        egui::Label::new(
            egui::RichText::new(format!(
                "{} Also refreshed automatically whenever PaperCraft Server updates. Keep the executable in the permanent folder where you want to use it.",
                papercraft_updater::shortcut_architecture_hint()
            ))
            .size(11.0)
            .color(t.text_muted),
        )
        .wrap(),
    );
    if let Some((true, ref msg)) = ctrl.desktop_shortcut_feedback {
        ui.add_space(4.0);
        ui.add(egui::Label::new(egui::RichText::new(msg).size(11.0).color(t.danger_fg)).wrap());
    }
}

fn render_autostart_section(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    ui.add_space(8.0);
    let mut autostart_on = ctrl.draft_settings.launch_at_login;
    let autostart_resp = toggle_switch_row(
        ui,
        "srv_autostart_switch",
        "Launch automatically at system startup",
        "Start PaperCraft Server automatically when you sign in to your computer.",
        &mut autostart_on,
        t,
    );
    if autostart_resp.changed() {
        ctrl.set_launch_at_login_enabled(autostart_on);
    }
    ui.add(
        egui::Label::new(
            egui::RichText::new(format!(
                "{} Turn off to launch PaperCraft Server only manually.",
                papercraft_updater::autostart_architecture_hint()
            ))
            .size(11.0)
            .color(t.text_muted),
        )
        .wrap(),
    );
    if let Some((true, ref msg)) = ctrl.autostart_feedback {
        ui.add_space(4.0);
        ui.add(egui::Label::new(egui::RichText::new(msg).size(11.0).color(t.danger_fg)).wrap());
    }

    ui.add_space(6.0);
    let autostart_enabled = ctrl.draft_settings.launch_at_login;
    let title_color = if autostart_enabled {
        t.text
    } else {
        t.text_muted
    };
    ui.indent("srv_autostart_window_indent", |ui| {
        ui.add_enabled_ui(autostart_enabled, |ui| {
            ui.checkbox(
                &mut ctrl.draft_settings.open_window_at_login,
                egui::RichText::new("Open control window at system startup")
                    .size(12.0)
                    .color(title_color),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "Leave unchecked to keep PaperCraft Server hidden in the system tray when your computer starts. Open it anytime from the tray icon.",
                    )
                    .size(11.0)
                    .color(t.text_muted),
                )
                .wrap(),
            );
        });
    });
}

fn render_update_status_details(
    ctrl: &mut ServerController,
    status: &papercraft_updater::UpdateStatus,
    is_custom_feed: bool,
    ui: &mut egui::Ui,
    t: &Tokens,
    actions: &mut UiActions,
) {
    use papercraft_updater::UpdateStatus;

    match status {
        UpdateStatus::Available(update) => {
            ui.add_space(10.0);
            render_update_available_box(ctrl, update, is_custom_feed, ui, t);
        }
        UpdateStatus::Downloading {
            update,
            downloaded_bytes,
            total_bytes,
        } => {
            ui.add_space(10.0);
            render_update_downloading_box(ctrl, update, *downloaded_bytes, *total_bytes, ui, t);
        }
        UpdateStatus::ReadyToInstall { update, .. } => {
            ui.add_space(10.0);
            render_update_ready_box(update, ui, t, actions);
        }
        UpdateStatus::UpToDate { latest_version } => {
            let msg = papercraft_updater::format_up_to_date_status(
                Some("PaperCraft Server"),
                ctrl.updater.config().map(|cfg| &cfg.current_version),
                latest_version,
            );
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(egui::RichText::new(msg).size(11.0).color(t.success_fg)).wrap(),
            );
        }
        UpdateStatus::NoPlatformAsset { latest_version, .. } => {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "Release v{latest_version} requires manual download for this platform."
                    ))
                    .size(11.0)
                    .color(t.warning_fg),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_button(
                        ui,
                        Icon::ExternalLink,
                        Some("Open Release Page"),
                        false,
                        true,
                        t,
                    )
                    .clicked()
                    {
                        ctrl.open_update_release_page();
                    }
                });
            });
        }
        UpdateStatus::Error {
            message,
            was_silent: false,
            ..
        } => {
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(egui::RichText::new(message).size(11.0).color(t.danger_fg)).wrap(),
            );
        }
        _ => {}
    }
}

fn render_essential_settings_card(
    ctrl: &mut ServerController,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    t: &Tokens,
) {
    draw_card(ui, t, |ui| {
        draw_section_header(
            ui,
            Icon::Settings,
            "Server Settings",
            "Changes save automatically. Choose where data lives and who can connect.",
            t,
        );
        ui.add_space(10.0);

        // 1. Storage folder (PAPERCRAFT_DATA_DIR)
        ui.label(
            egui::RichText::new("Where to save your notebooks")
                .size(12.5)
                .strong()
                .color(t.text),
        );
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(
                "Folder where accounts and synced notebooks are saved (papercraft.redb).",
            )
            .size(11.0)
            .color(t.text_muted),
        );
        ui.add_space(5.0);

        let picking = ctrl.is_picking_folder();
        let browse_lbl = if picking { "Opening..." } else { "Browse..." };
        let browse_w = icon_button_width(ui, Some(browse_lbl));
        let open_w = 60.0_f32;
        let gap = 6.0_f32;
        let input_w = (ui.available_width() - browse_w - open_w - gap * 2.0).max(120.0);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            draw_singleline_input(
                ui,
                "bnc_srv_data_dir",
                input_w,
                "Choose a storage folder...",
                &mut ctrl.draft_settings.data_dir,
                t,
            );
            if icon_button(ui, Icon::FolderOpen, Some(browse_lbl), false, !picking, t)
                .on_hover_text("Pick a folder using your file browser")
                .clicked()
            {
                ctrl.begin_folder_picker(ctx);
            }
            if fixed_secondary_button(ui, "Open", open_w, true, t)
                .on_hover_text("Open this folder in your file manager")
                .clicked()
            {
                ctrl.open_data_folder();
            }
        });

        ui.add_space(12.0);
        draw_hairline(ui, t);
        ui.add_space(10.0);

        // 2. Network scope + port (PAPERCRAFT_BIND)
        render_network_section(ctrl, ui, t);

        ui.add_space(12.0);
        draw_hairline(ui, t);
        ui.add_space(10.0);

        // 3. Registration toggle (!PAPERCRAFT_DISABLE_REGISTRATION)
        ui.checkbox(
            &mut ctrl.draft_settings.allow_registration,
            egui::RichText::new("Allow new account sign-ups")
                .size(12.5)
                .strong()
                .color(t.text),
        );
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(
                "Turn this off after creating your accounts so no one else can register.",
            )
            .size(11.0)
            .color(t.text_muted),
        );
    });
}

fn render_network_section(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    ui.label(
        egui::RichText::new("Who can connect")
            .size(12.5)
            .strong()
            .color(t.text),
    );
    ui.add_space(5.0);

    let options = [
        SwitchOption::new(
            None,
            NetworkScope::LocalOnly.label(),
            NetworkScope::LocalOnly.description(),
        ),
        SwitchOption::new(
            None,
            NetworkScope::LocalNetwork.label(),
            NetworkScope::LocalNetwork.description(),
        ),
        SwitchOption::new(
            None,
            NetworkScope::Custom.label(),
            NetworkScope::Custom.description(),
        ),
    ];
    let active_idx = match ctrl.draft_settings.network_scope {
        NetworkScope::LocalOnly => 0,
        NetworkScope::LocalNetwork => 1,
        NetworkScope::Custom => 2,
    };
    if let Some(clicked) = segmented_switch(ui, "bnc_srv_network_scope", &options, active_idx, t) {
        ctrl.draft_settings.network_scope = match clicked {
            0 => NetworkScope::LocalOnly,
            1 => NetworkScope::LocalNetwork,
            _ => NetworkScope::Custom,
        };
    }

    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(ctrl.draft_settings.network_scope.description())
            .size(11.0)
            .color(t.text_muted),
    );
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(egui::RichText::new("Port").size(12.0).color(t.text));
        ui.add_sized(
            [88.0, 26.0],
            egui::DragValue::new(&mut ctrl.draft_settings.port)
                .range(1..=65535)
                .speed(1.0),
        );

        if ctrl.draft_settings.network_scope == NetworkScope::Custom {
            ui.add_space(6.0);
            ui.label(egui::RichText::new("IP address").size(12.0).color(t.text));
            let rem = ui.available_width().max(110.0);
            draw_singleline_input(
                ui,
                "bnc_srv_custom_ip",
                rem,
                "127.0.0.1",
                &mut ctrl.draft_settings.custom_ip,
                t,
            );
        }
    });

    if ctrl.draft_settings.network_scope == NetworkScope::LocalNetwork {
        ui.add_space(5.0);
        ui.label(
            egui::RichText::new(format!(
                "Tip: if other devices cannot reach port {}, allow inbound TCP port {} in your firewall.",
                ctrl.draft_settings.port, ctrl.draft_settings.port
            ))
            .size(11.0)
            .color(t.text_muted),
        );
    }
}

fn render_users_view(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    if let Some(banner) = ctrl.notice_banner.clone()
        && draw_banner(ui, &banner, true, t)
    {
        ctrl.notice_banner = None;
    }

    if let Some(outcome) = ctrl.last_password_reset.clone() {
        render_password_reset_card(ctrl, &outcome, ui, t);
    }

    draw_card(ui, t, |ui| {
        render_users_header(ctrl, ui, t);
        ui.add_space(10.0);
        render_users_view_switch(ctrl, ui, t);
        let has_items = match ctrl.users_view_mode {
            UsersViewMode::Active => !ctrl.users_snapshot.is_empty(),
            UsersViewMode::Trash => !ctrl.trashed_users_snapshot.is_empty(),
        };
        if has_items {
            ui.add_space(10.0);
            render_user_search_bar(ctrl, ui, t);
        }
        ui.add_space(10.0);
        match ctrl.users_view_mode {
            UsersViewMode::Active => render_users_list_body(ctrl, ui, t),
            UsersViewMode::Trash => render_trashed_users_list_body(ctrl, ui, t),
        }
    });
}

fn render_users_view_switch(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    let active_label = format!("Active ({})", ctrl.users_snapshot.len());
    let trash_label = format!("Trash ({})", ctrl.trashed_users_snapshot.len());
    let options = [
        SwitchOption::new(
            Some(Icon::Account),
            &active_label,
            "View active registered accounts stored on this server",
        ),
        SwitchOption::new(
            Some(Icon::Trash),
            &trash_label,
            "View deleted accounts held in server Trash for 30 days",
        ),
    ];
    let active_idx = match ctrl.users_view_mode {
        UsersViewMode::Active => 0,
        UsersViewMode::Trash => 1,
    };
    if let Some(clicked) = segmented_switch(ui, "bnc_srv_users_view_mode", &options, active_idx, t)
    {
        let next = if clicked == 0 {
            UsersViewMode::Active
        } else {
            UsersViewMode::Trash
        };
        ctrl.set_users_view_mode(next);
    }
}

fn render_password_reset_card(
    ctrl: &mut ServerController,
    outcome: &PasswordResetOutcome,
    ui: &mut egui::Ui,
    t: &Tokens,
) {
    egui::Frame::new()
        .fill(t.accent_soft)
        .stroke(Stroke::new(1.2, t.accent))
        .corner_radius(CARD_RADIUS)
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                paint_icon(ui.painter(), ir, Icon::Reset, t.accent);
                ui.label(
                    egui::RichText::new(format!(
                        "Temporary recovery code for {}",
                        outcome.account_label()
                    ))
                    .size(13.0)
                    .strong()
                    .color(t.text),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_button(ui, Icon::Close, None, false, true, t)
                        .on_hover_text("Dismiss recovery code card")
                        .clicked()
                    {
                        ctrl.dismiss_password_reset();
                    }
                });
            });

            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "Share this one-time code with the user. On their next sign-in, PaperCraft will prompt them to enter this recovery code and choose a new password.",
                    )
                    .size(11.0)
                    .color(t.text_muted),
                )
                .wrap(),
            );

            let copied = ctrl.was_recently_copied(&outcome.temporary_password);
            egui::Frame::new()
                .fill(t.field_bg)
                .stroke(Stroke::new(1.0, t.border))
                .corner_radius(CONTROL_RADIUS)
                .inner_margin(Margin::symmetric(10, 7))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(&outcome.temporary_password)
                                .size(15.0)
                                .monospace()
                                .strong()
                                .color(t.text),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (copy_icon, copy_lbl) = if copied {
                                (Icon::Check, "Copied")
                            } else {
                                (Icon::Copy, "Copy Code")
                            };
                            if icon_button(ui, copy_icon, Some(copy_lbl), copied, true, t)
                                .on_hover_text("Copy temporary recovery code to clipboard")
                                .clicked()
                            {
                                ctrl.copy_url(&outcome.temporary_password);
                            }
                        });
                    });
                });
        });
}

fn render_users_header(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    let total = ctrl.users_snapshot.len();
    let trashed_total = ctrl.trashed_users_snapshot.len();
    let count_text = if total == 1 {
        "1 active".to_string()
    } else {
        format!("{total} active")
    };

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        paint_icon(ui.painter(), ir, Icon::Account, t.accent);
        ui.label(
            egui::RichText::new("Registered Users")
                .size(13.5)
                .strong()
                .color(t.text),
        );
        draw_compact_pill(ui, None, &count_text, t.toolbar_bg, t.border, t.text_muted);
        if trashed_total > 0 {
            draw_compact_pill(
                ui,
                Some(Icon::Trash),
                &format!("{trashed_total} in Trash"),
                t.danger_bg,
                t.danger_border,
                t.danger_fg,
            );
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, Icon::RotateCw, Some("Refresh"), false, true, t)
                .on_hover_text("Reload active and trashed accounts from the database")
                .clicked()
            {
                ctrl.refresh_users();
                if ctrl.is_running() {
                    ctrl.refresh_stats();
                }
            }
        });
    });

    ui.add_space(3.0);
    let subtitle = match ctrl.users_view_mode {
        UsersViewMode::Active => {
            let data_path = ctrl.draft_settings.resolved_data_path();
            let folder_name = data_path
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|s| !s.is_empty())
                .map_or_else(|| data_path.display().to_string(), ToString::to_string);
            let reg_note = if ctrl.draft_settings.allow_registration {
                "New sign-ups open"
            } else {
                "New sign-ups closed"
            };
            format!("Accounts stored in `{folder_name}` (papercraft.redb) · {reg_note}.")
        }
        UsersViewMode::Trash => format!(
            "Deleted accounts stay in server Trash for {ACCOUNT_TRASH_RETENTION_DAYS} days and are permanently deleted automatically when they expire. Restore an account anytime before then."
        ),
    };
    ui.add(egui::Label::new(egui::RichText::new(subtitle).size(11.0).color(t.text_muted)).wrap());
}

fn render_user_search_bar(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    let has_query = !ctrl.user_search.is_empty();
    let clear_w = if has_query { 28.0 + 6.0 } else { 0.0 };
    let frame_w = (ui.available_width() - clear_w).max(140.0);
    let hint = match ctrl.users_view_mode {
        UsersViewMode::Active => "Search active accounts by name, email, or @username...",
        UsersViewMode::Trash => "Search trashed accounts by name, email, or @username...",
    };

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        egui::Frame::new()
            .fill(t.field_bg)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CONTROL_RADIUS)
            .inner_margin(Margin::symmetric(8, 0))
            .show(ui, |ui| {
                ui.set_width((frame_w - 16.0).max(60.0));
                ui.set_height(INPUT_HEIGHT);
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (sr, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                    paint_icon(ui.painter(), sr, Icon::Search, t.text_muted);
                    let edit_w = (ui.available_width() - 4.0).max(40.0);
                    ui.add_sized(
                        [edit_w, INPUT_HEIGHT - 2.0],
                        egui::TextEdit::singleline(&mut ctrl.user_search)
                            .id_salt("bnc_srv_user_search")
                            .hint_text(egui::RichText::new(hint).color(t.text_muted).size(12.0))
                            .frame(egui::Frame::NONE)
                            .font(egui::FontId::proportional(12.0))
                            .text_color(t.text),
                    );
                });
            });

        if has_query
            && icon_button(ui, Icon::Close, None, false, true, t)
                .on_hover_text("Clear search")
                .clicked()
        {
            ctrl.user_search.clear();
        }
    });
}

fn render_users_list_body(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    if let Some(ref err) = ctrl.users_error {
        draw_banner(
            ui,
            &NoticeBanner {
                tone: NoticeTone::Error,
                text: err.clone(),
            },
            false,
            t,
        );
        return;
    }

    if ctrl.users_snapshot.is_empty() {
        render_users_empty_state(ctrl.draft_settings.allow_registration, ui, t);
        return;
    }

    let query = ctrl.user_search.trim().to_ascii_lowercase();
    let matches: Vec<usize> = ctrl
        .users_snapshot
        .iter()
        .enumerate()
        .filter(|(_, u)| {
            query.is_empty()
                || u.email
                    .as_deref()
                    .is_some_and(|e| e.to_ascii_lowercase().contains(&query))
                || u.username.to_ascii_lowercase().contains(&query)
                || u.display_name.to_ascii_lowercase().contains(&query)
                || u.user_id.to_ascii_lowercase().contains(&query)
        })
        .map(|(idx, _)| idx)
        .collect();

    if matches.is_empty() {
        render_search_no_matches_box(&ctrl.user_search, "accounts", ui, t);
        return;
    }

    enum UserRowAction {
        CopyId(String),
        RequestReset(String),
        ConfirmReset(String),
        RequestDelete(String),
        ConfirmDelete(String),
        Cancel,
    }

    let mut action: Option<UserRowAction> = None;
    ui.spacing_mut().item_spacing.y = 7.0;
    for &idx in &matches {
        let user = ctrl.users_snapshot[idx].clone();
        let copied = ctrl.was_recently_copied(&user.user_id);
        let confirm_reset = ctrl.pending_reset_user.as_deref() == Some(user.user_id.as_str());
        let confirm_delete = ctrl.pending_delete_user.as_deref() == Some(user.user_id.as_str());
        let mut confirm_draft = DeleteConfirmDraft {
            input: &mut ctrl.delete_confirm_input,
            paste_blocked: &mut ctrl.delete_confirm_paste_blocked,
            focus_pending: &mut ctrl.delete_confirm_focus_pending,
        };
        match render_user_row(
            &user,
            copied,
            confirm_reset,
            confirm_delete,
            &mut confirm_draft,
            ui,
            t,
        ) {
            RowClick::None => {}
            RowClick::CopyId => action = Some(UserRowAction::CopyId(user.user_id)),
            RowClick::RequestReset => action = Some(UserRowAction::RequestReset(user.user_id)),
            RowClick::ConfirmReset => action = Some(UserRowAction::ConfirmReset(user.user_id)),
            RowClick::RequestDelete => action = Some(UserRowAction::RequestDelete(user.user_id)),
            RowClick::ConfirmDelete => action = Some(UserRowAction::ConfirmDelete(user.user_id)),
            RowClick::Cancel => action = Some(UserRowAction::Cancel),
        }
    }
    match action {
        Some(UserRowAction::CopyId(id)) => ctrl.copy_url(&id),
        Some(UserRowAction::RequestReset(id)) => ctrl.request_reset_password(&id),
        Some(UserRowAction::ConfirmReset(id)) => ctrl.confirm_reset_password(&id),
        Some(UserRowAction::RequestDelete(id)) => ctrl.request_delete_user(&id),
        Some(UserRowAction::ConfirmDelete(id)) => ctrl.confirm_delete_user(&id),
        Some(UserRowAction::Cancel) => ctrl.cancel_user_action(),
        None => {}
    }

    ui.add_space(4.0);
    let total_users = ctrl.users_snapshot.len();
    let total_notebooks: usize = ctrl.users_snapshot.iter().map(|u| u.notebook_count).sum();
    let nb_label = if total_notebooks == 1 {
        "notebook"
    } else {
        "notebooks"
    };
    let summary = if matches.len() == total_users {
        format!("{total_users} active · {total_notebooks} {nb_label} total")
    } else {
        format!(
            "Showing {} of {total_users} active accounts · {total_notebooks} {nb_label} total",
            matches.len()
        )
    };
    ui.label(egui::RichText::new(summary).size(11.0).color(t.text_muted));
}

fn render_trashed_users_list_body(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    if let Some(ref err) = ctrl.users_error {
        draw_banner(
            ui,
            &NoticeBanner {
                tone: NoticeTone::Error,
                text: err.clone(),
            },
            false,
            t,
        );
        return;
    }

    if ctrl.trashed_users_snapshot.is_empty() {
        render_trash_empty_state(ui, t);
        return;
    }

    let query = ctrl.user_search.trim().to_ascii_lowercase();
    let matches: Vec<usize> = ctrl
        .trashed_users_snapshot
        .iter()
        .enumerate()
        .filter(|(_, u)| {
            query.is_empty()
                || u.email
                    .as_deref()
                    .is_some_and(|e| e.to_ascii_lowercase().contains(&query))
                || u.username.to_ascii_lowercase().contains(&query)
                || u.display_name.to_ascii_lowercase().contains(&query)
                || u.user_id.to_ascii_lowercase().contains(&query)
        })
        .map(|(idx, _)| idx)
        .collect();

    if matches.is_empty() {
        render_search_no_matches_box(&ctrl.user_search, "trashed accounts", ui, t);
        return;
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    let mut restore_target: Option<String> = None;
    ui.spacing_mut().item_spacing.y = 7.0;
    for &idx in &matches {
        let user = &ctrl.trashed_users_snapshot[idx];
        if render_trashed_user_row(user, now_ms, ui, t) {
            restore_target = Some(user.user_id.clone());
        }
    }
    if let Some(user_id) = restore_target {
        ctrl.restore_trashed_user(&user_id);
    }

    ui.add_space(4.0);
    let total_trashed = ctrl.trashed_users_snapshot.len();
    let total_notebooks: usize = ctrl
        .trashed_users_snapshot
        .iter()
        .map(|u| u.notebook_count)
        .sum();
    let nb_label = if total_notebooks == 1 {
        "notebook"
    } else {
        "notebooks"
    };
    let summary = if matches.len() == total_trashed {
        format!(
            "{total_trashed} in Trash · {total_notebooks} {nb_label} kept ({ACCOUNT_TRASH_RETENTION_DAYS}-day retention)"
        )
    } else {
        format!(
            "Showing {} of {total_trashed} in Trash · {total_notebooks} {nb_label} kept",
            matches.len()
        )
    };
    ui.label(egui::RichText::new(summary).size(11.0).color(t.text_muted));
}

fn render_search_no_matches_box(query: &str, noun: &str, ui: &mut egui::Ui, t: &Tokens) {
    egui::Frame::new()
        .fill(t.toolbar_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new(format!("No {noun} match \"{}\".", query.trim()))
                        .size(12.0)
                        .color(t.text_muted),
                );
            });
        });
}

fn render_users_empty_state(registration_open: bool, ui: &mut egui::Ui, t: &Tokens) {
    egui::Frame::new()
        .fill(t.toolbar_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(16, 22))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let (badge_rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                ui.painter().rect(
                    badge_rect,
                    CornerRadius::same(10),
                    t.accent_soft,
                    Stroke::new(1.0, t.accent.gamma_multiply(0.35)),
                    egui::StrokeKind::Inside,
                );
                paint_icon(ui.painter(), badge_rect.shrink(9.0), Icon::Account, t.accent);

                ui.label(
                    egui::RichText::new("No registered users yet")
                        .size(13.0)
                        .strong()
                        .color(t.text),
                );
                let hint = if registration_open {
                    "When someone creates an account in PaperCraft Desktop or Web, it will appear here."
                } else {
                    "New account sign-ups are turned off, and no accounts exist in this database yet."
                };
                ui.add(
                    egui::Label::new(egui::RichText::new(hint).size(11.5).color(t.text_muted))
                        .wrap(),
                );
            });
        });
}

fn render_trash_empty_state(ui: &mut egui::Ui, t: &Tokens) {
    egui::Frame::new()
        .fill(t.toolbar_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(16, 22))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let (badge_rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                ui.painter().rect(
                    badge_rect,
                    CornerRadius::same(10),
                    t.field_bg,
                    Stroke::new(1.0, t.border),
                    egui::StrokeKind::Inside,
                );
                paint_icon(
                    ui.painter(),
                    badge_rect.shrink(9.0),
                    Icon::Trash,
                    t.text_muted,
                );

                ui.label(
                    egui::RichText::new("Server Trash is empty")
                        .size(13.0)
                        .strong()
                        .color(t.text),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!(
                            "Deleted accounts stay in server Trash for {ACCOUNT_TRASH_RETENTION_DAYS} days so you can restore them before automatic permanent deletion."
                        ))
                        .size(11.5)
                        .color(t.text_muted),
                    )
                    .wrap(),
                );
            });
        });
}

struct DeleteConfirmDraft<'a> {
    input: &'a mut String,
    paste_blocked: &'a mut bool,
    focus_pending: &'a mut bool,
}

enum RowClick {
    None,
    CopyId,
    RequestReset,
    ConfirmReset,
    RequestDelete,
    ConfirmDelete,
    Cancel,
}

fn measure_compact_pill_width(ui: &egui::Ui, has_icon: bool, label: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(11.0),
        Color32::WHITE,
    );
    let icon_w = if has_icon { 12.0 + 4.0 } else { 0.0 };
    16.0 + icon_w + galley.size().x + 2.0
}

fn measure_action_button_width(ui: &egui::Ui, label: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(12.2),
        Color32::WHITE,
    );
    (24.0 + 14.0 + 6.0 + galley.size().x).max(76.0)
}

fn render_user_row(
    user: &UserSummary,
    copied: bool,
    confirm_reset: bool,
    confirm_delete: bool,
    confirm_draft: &mut DeleteConfirmDraft<'_>,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> RowClick {
    let mut row_click = RowClick::None;
    let frame_border = if confirm_delete {
        t.danger_border
    } else if confirm_reset {
        t.accent
    } else {
        t.border
    };
    egui::Frame::new()
        .fill(t.toolbar_bg)
        .stroke(Stroke::new(1.0, frame_border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let nb_text = if user.notebook_count == 1 {
                "1 notebook".to_string()
            } else {
                format!("{} notebooks", user.notebook_count)
            };
            let nb_pill_w = measure_compact_pill_width(ui, true, &nb_text);
            let right_w = nb_pill_w + 5.0 * 3.0 + 28.0 * 3.0;
            let left_w = (ui.available_width() - 32.0 - 10.0 - right_w - 8.0).max(72.0);

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;

                let (avatar_rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
                ui.painter()
                    .circle_filled(avatar_rect.center(), 16.0, t.accent_soft);
                ui.painter().circle_stroke(
                    avatar_rect.center(),
                    16.0,
                    Stroke::new(1.0, t.accent.gamma_multiply(0.35)),
                );
                let initial = user_monogram(&user.display_name, &user.username);
                ui.painter().text(
                    avatar_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    initial,
                    egui::FontId::proportional(13.0),
                    t.accent,
                );

                ui.allocate_ui_with_layout(
                    Vec2::new(left_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(left_w);
                        ui.set_max_width(left_w);
                        ui.spacing_mut().item_spacing.y = 2.0;

                        let primary_name = if user.display_name.trim().is_empty() {
                            format!("@{}", user.username)
                        } else {
                            user.display_name.clone()
                        };
                        let at_handle = format!("@{}", user.username);
                        let show_handle = !primary_name.eq_ignore_ascii_case(&at_handle)
                            && !primary_name.eq_ignore_ascii_case(&user.username);
                        let reset_pill_w = if user.must_change_password {
                            measure_compact_pill_width(ui, false, "Reset pending") + 6.0
                        } else {
                            0.0
                        };

                        ui.horizontal(|ui| {
                            ui.set_max_width(left_w);
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let name_budget = if show_handle {
                                ((left_w - reset_pill_w) * 0.56).max(40.0)
                            } else {
                                (left_w - reset_pill_w).max(48.0)
                            };
                            ui.scope(|ui| {
                                ui.set_max_width(name_budget);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&primary_name)
                                            .size(12.5)
                                            .strong()
                                            .color(t.text),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(&primary_name);
                            });
                            if show_handle {
                                let handle_budget = (left_w - reset_pill_w - name_budget - 6.0).max(32.0);
                                ui.scope(|ui| {
                                    ui.set_max_width(handle_budget);
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&at_handle)
                                                .size(11.5)
                                                .monospace()
                                                .color(t.text_muted),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&at_handle);
                                });
                            }
                            if user.must_change_password {
                                draw_compact_pill(
                                    ui,
                                    None,
                                    "Reset pending",
                                    t.field_bg,
                                    t.warning_fg,
                                    t.warning_fg,
                                );
                            }
                        });

                        let joined = format_utc_date(user.created_at_ms);
                        let short_id = short_user_id(&user.user_id);
                        let subline = match user.email.as_deref().filter(|s| !s.trim().is_empty()) {
                            Some(email) => format!("{email} · Joined {joined} · ID {short_id}"),
                            None => format!("Joined {joined} · ID {short_id}"),
                        };
                        let hover_tip = match user.email.as_deref().filter(|s| !s.trim().is_empty()) {
                            Some(email) => format!("{email}\nUser ID: {}", user.user_id),
                            None => format!("User ID: {}", user.user_id),
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(subline)
                                    .size(11.0)
                                    .color(t.text_muted),
                            )
                            .truncate(),
                        )
                        .on_hover_text(hover_tip);
                    },
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    if danger_icon_button(ui, Icon::Trash, confirm_delete, t)
                        .on_hover_text("Move this user account to 30-day server Trash")
                        .clicked()
                    {
                        row_click = if confirm_delete {
                            RowClick::Cancel
                        } else {
                            RowClick::RequestDelete
                        };
                    }
                    if icon_button(ui, Icon::Reset, None, confirm_reset, true, t)
                        .on_hover_text("Reset password and generate a random recovery code")
                        .clicked()
                    {
                        row_click = if confirm_reset {
                            RowClick::Cancel
                        } else {
                            RowClick::RequestReset
                        };
                    }
                    let (copy_icon, copy_tip) = if copied {
                        (Icon::Check, "User ID copied")
                    } else {
                        (Icon::Copy, "Copy full user ID")
                    };
                    if icon_button(ui, copy_icon, None, copied, true, t)
                        .on_hover_text(copy_tip)
                        .clicked()
                    {
                        row_click = RowClick::CopyId;
                    }

                    draw_compact_pill(
                        ui,
                        Some(Icon::Notebook),
                        &nb_text,
                        t.field_bg,
                        t.border,
                        t.text,
                    );
                });
            });

            if confirm_reset {
                ui.add_space(8.0);
                draw_hairline(ui, t);
                ui.add_space(7.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Generate a random one-time recovery code? Their current password will stop working and active devices will be signed out until they log in with the code and set a new password.",
                        )
                        .size(11.0)
                        .color(t.text_muted),
                    )
                    .wrap(),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if primary_button(ui, Icon::Reset, "Generate Reset Code", true, t).clicked()
                        {
                            row_click = RowClick::ConfirmReset;
                        }
                        if secondary_button(ui, "Cancel", true, t).clicked() {
                            row_click = RowClick::Cancel;
                        }
                    });
                });
            } else if confirm_delete {
                ui.add_space(8.0);
                draw_hairline(ui, t);
                ui.add_space(7.0);
                let account_lbl = user
                    .email
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .map_or_else(|| format!("@{}", user.username), ToString::to_string);
                let expected = account_delete_confirmation_phrase(&user.username);
                let matched = account_delete_confirmation_matches(confirm_draft.input, &expected);

                egui::Frame::new()
                    .fill(t.field_bg)
                    .stroke(Stroke::new(1.0, t.danger_border))
                    .corner_radius(CONTROL_RADIUS)
                    .inner_margin(Margin::symmetric(10, 9))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 5.0;

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let (ir, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
                            paint_icon(ui.painter(), ir, Icon::AlertCircle, t.danger_fg);
                            ui.label(
                                egui::RichText::new("Confirm Account Deletion")
                                    .size(12.0)
                                    .strong()
                                    .color(t.danger_fg),
                            );
                        });

                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!(
                                    "Move `{account_lbl}` and its {} synced notebook(s) to server Trash? Active devices will be signed out immediately. The account stays recoverable for {ACCOUNT_TRASH_RETENTION_DAYS} days, then is permanently deleted automatically. Copy and paste is disabled.",
                                    user.notebook_count
                                ))
                                .size(11.0)
                                .color(t.text_muted),
                            )
                            .wrap(),
                        );

                        ui.label(
                            egui::RichText::new("Type this confirmation manually:")
                                .size(10.5)
                                .color(t.text_muted),
                        );

                        egui::Frame::new()
                            .fill(t.toolbar_bg)
                            .stroke(Stroke::new(1.0, t.border))
                            .corner_radius(CornerRadius::same(5))
                            .inner_margin(Margin::symmetric(8, 6))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&expected)
                                            .size(11.5)
                                            .strong()
                                            .monospace()
                                            .color(t.text),
                                    )
                                    .selectable(false)
                                    .wrap(),
                                );
                            });

                        let (hint_text, hint_color) =
                            if *confirm_draft.paste_blocked && !matched {
                                (
                                    "Copy/paste is disabled — type it manually".to_string(),
                                    t.warning_fg,
                                )
                            } else if confirm_draft.input.trim().is_empty() {
                                (
                                    "Type manually (no copy/paste)".to_string(),
                                    t.text_muted,
                                )
                            } else if matched {
                                ("Confirmation matched".to_string(), t.success_fg)
                            } else {
                                (format!("Must match: {expected}"), t.warning_fg)
                            };
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Manual Confirmation")
                                    .size(11.5)
                                    .strong()
                                    .color(t.text),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new(hint_text).size(10.5).color(hint_color));
                            });
                        });

                        let input_border = if matched {
                            t.success_border
                        } else {
                            t.danger_border
                        };
                        let input_res = draw_manual_confirm_input(
                            ui,
                            "bnc_srv_delete_user_confirm",
                            ui.available_width(),
                            &expected,
                            input_border,
                            matched,
                            confirm_draft.input,
                            t,
                        );
                        if std::mem::take(&mut *confirm_draft.focus_pending) {
                            ui.memory_mut(|m| m.request_focus(input_res.edit_id));
                        }
                        if input_res.paste_blocked {
                            *confirm_draft.paste_blocked = true;
                            ui.ctx().request_repaint();
                        } else if input_res.changed {
                            *confirm_draft.paste_blocked = false;
                        }
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            row_click = RowClick::Cancel;
                        }

                        ui.add_space(3.0);
                        ui.horizontal(|ui| {
                            if secondary_button(ui, "Cancel", true, t).clicked() {
                                row_click = RowClick::Cancel;
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if danger_button(ui, Icon::Trash, "Move to Trash", matched, t)
                                    .clicked()
                                    || (input_res.enter_pressed && matched)
                                {
                                    row_click = RowClick::ConfirmDelete;
                                }
                            });
                        });
                    });
            }
        });
    row_click
}

fn render_trashed_user_row(
    user: &TrashedUserSummary,
    now_ms: i64,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let mut restore_clicked = false;
    let (expiry_label, urgent) = format_trash_remaining(user.expires_at_ms, now_ms);
    let nb_text = if user.notebook_count == 1 {
        "1 notebook".to_string()
    } else {
        format!("{} notebooks", user.notebook_count)
    };

    egui::Frame::new()
        .fill(t.toolbar_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let nb_pill_w = measure_compact_pill_width(ui, true, &nb_text);
            let restore_w = measure_action_button_width(ui, "Restore");
            let right_w = nb_pill_w + 6.0 + restore_w;
            let left_w = (ui.available_width() - 32.0 - 10.0 - right_w - 8.0).max(72.0);

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;

                let (avatar_rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
                ui.painter()
                    .circle_filled(avatar_rect.center(), 16.0, t.danger_bg);
                ui.painter().circle_stroke(
                    avatar_rect.center(),
                    16.0,
                    Stroke::new(1.0, t.danger_border),
                );
                let initial = user_monogram(&user.display_name, &user.username);
                ui.painter().text(
                    avatar_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    initial,
                    egui::FontId::proportional(13.0),
                    t.danger_fg,
                );

                ui.allocate_ui_with_layout(
                    Vec2::new(left_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(left_w);
                        ui.set_max_width(left_w);
                        ui.spacing_mut().item_spacing.y = 2.0;

                        let primary_name = if user.display_name.trim().is_empty() {
                            format!("@{}", user.username)
                        } else {
                            user.display_name.clone()
                        };
                        let at_handle = format!("@{}", user.username);
                        let show_handle = !primary_name.eq_ignore_ascii_case(&at_handle)
                            && !primary_name.eq_ignore_ascii_case(&user.username);
                        let expiry_pill_w = measure_compact_pill_width(ui, false, &expiry_label) + 6.0;

                        ui.horizontal(|ui| {
                            ui.set_max_width(left_w);
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let name_budget = if show_handle {
                                ((left_w - expiry_pill_w) * 0.56).max(40.0)
                            } else {
                                (left_w - expiry_pill_w).max(48.0)
                            };
                            ui.scope(|ui| {
                                ui.set_max_width(name_budget);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&primary_name)
                                            .size(12.5)
                                            .strong()
                                            .color(t.text),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(&primary_name);
                            });
                            if show_handle {
                                let handle_budget = (left_w - expiry_pill_w - name_budget - 6.0).max(32.0);
                                ui.scope(|ui| {
                                    ui.set_max_width(handle_budget);
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&at_handle)
                                                .size(11.5)
                                                .monospace()
                                                .color(t.text_muted),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&at_handle);
                                });
                            }
                            let (pill_fill, pill_border, pill_fg) = if urgent {
                                (t.danger_bg, t.danger_border, t.danger_fg)
                            } else {
                                (t.field_bg, t.warning_fg, t.warning_fg)
                            };
                            draw_compact_pill(ui, None, &expiry_label, pill_fill, pill_border, pill_fg);
                        });

                        let deleted_date = format_utc_date(user.deleted_at_ms);
                        let expires_date = format_utc_date(user.expires_at_ms);
                        let short_id = short_user_id(&user.user_id);
                        let subline = match user.email.as_deref().filter(|s| !s.trim().is_empty()) {
                            Some(email) => format!(
                                "{email} · Deleted {deleted_date} · Expires {expires_date} · ID {short_id}"
                            ),
                            None => {
                                format!("Deleted {deleted_date} · Expires {expires_date} · ID {short_id}")
                            }
                        };
                        let hover_tip = match user.email.as_deref().filter(|s| !s.trim().is_empty()) {
                            Some(email) => format!("{email}\nUser ID: {}", user.user_id),
                            None => format!("User ID: {}", user.user_id),
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(subline)
                                    .size(11.0)
                                    .color(t.text_muted),
                            )
                            .truncate(),
                        )
                        .on_hover_text(hover_tip);
                    },
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if primary_button(ui, Icon::Reset, "Restore", true, t)
                        .on_hover_text(
                            "Restore this account and all synced notebooks from Trash",
                        )
                        .clicked()
                    {
                        restore_clicked = true;
                    }
                    draw_compact_pill(
                        ui,
                        Some(Icon::Notebook),
                        &nb_text,
                        t.field_bg,
                        t.border,
                        t.text,
                    );
                });
            });
        });

    restore_clicked
}

fn format_trash_remaining(expires_at_ms: i64, now_ms: i64) -> (String, bool) {
    const DAY_MS: i64 = 24 * 60 * 60 * 1000;
    let remaining_ms = expires_at_ms.saturating_sub(now_ms).max(0);
    let days_left = if remaining_ms == 0 {
        0
    } else {
        remaining_ms.saturating_add(DAY_MS - 1) / DAY_MS
    };
    if days_left <= 0 {
        ("Expires today".to_string(), true)
    } else if days_left == 1 {
        ("1 day left to restore".to_string(), true)
    } else {
        (format!("{days_left} days left to restore"), days_left <= 3)
    }
}

struct ManualConfirmInputResult {
    edit_id: egui::Id,
    changed: bool,
    enter_pressed: bool,
    paste_blocked: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClipboardEventAction {
    Keep,
    Drop,
    BlockPaste,
}

fn classify_clipboard_event(
    event: &egui::Event,
    has_focus: bool,
    pointer_inside: bool,
    box_rect: egui::Rect,
    target_accesskit_id: egui::accesskit::NodeId,
) -> ClipboardEventAction {
    match event {
        egui::Event::Paste(_) if has_focus || pointer_inside => ClipboardEventAction::BlockPaste,
        egui::Event::MiddleClickPaste { pos, .. } if has_focus || box_rect.contains(*pos) => {
            ClipboardEventAction::BlockPaste
        }
        egui::Event::Copy | egui::Event::Cut if has_focus => ClipboardEventAction::Drop,
        egui::Event::AccessKitActionRequest(req)
            if req.target_node == target_accesskit_id
                && matches!(
                    req.action,
                    egui::accesskit::Action::ReplaceSelectedText
                        | egui::accesskit::Action::SetValue
                ) =>
        {
            ClipboardEventAction::BlockPaste
        }
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } if has_focus => {
            let cmd = modifiers.command || modifiers.ctrl;
            if (cmd && *key == egui::Key::V) || (modifiers.shift && *key == egui::Key::Insert) {
                ClipboardEventAction::BlockPaste
            } else if cmd && matches!(*key, egui::Key::C | egui::Key::X | egui::Key::Insert) {
                ClipboardEventAction::Drop
            } else {
                ClipboardEventAction::Keep
            }
        }
        _ => ClipboardEventAction::Keep,
    }
}

fn suppress_clipboard_events(ui: &egui::Ui, edit_id: egui::Id, box_rect: egui::Rect) -> bool {
    let has_focus = ui.ctx().memory(|m| m.has_focus(edit_id));
    let pointer_inside = ui.rect_contains_pointer(box_rect);
    if !has_focus && !pointer_inside {
        return false;
    }
    let target_accesskit_id = edit_id.accesskit_id();
    let mut paste_blocked = false;
    ui.ctx().input_mut(|i| {
        i.events.retain(|event| {
            match classify_clipboard_event(
                event,
                has_focus,
                pointer_inside,
                box_rect,
                target_accesskit_id,
            ) {
                ClipboardEventAction::Keep => true,
                ClipboardEventAction::Drop => false,
                ClipboardEventAction::BlockPaste => {
                    paste_blocked = true;
                    false
                }
            }
        });
    });
    paste_blocked
}

#[allow(clippy::too_many_arguments)]
fn draw_manual_confirm_input(
    ui: &mut egui::Ui,
    id_salt: &str,
    width: f32,
    hint: &str,
    border_color: Color32,
    matched: bool,
    value: &mut String,
    t: &Tokens,
) -> ManualConfirmInputResult {
    let input_width = width.max(0.0).min(ui.available_width().max(0.0));
    let (box_rect, box_resp) =
        ui.allocate_exact_size(Vec2::new(input_width, INPUT_HEIGHT), Sense::hover());
    let edit_id = ui.make_persistent_id(id_salt);
    let has_focus = ui.ctx().memory(|m| m.has_focus(edit_id));
    let stroke = if has_focus {
        Stroke::new(1.5, t.accent)
    } else if box_resp.hovered() {
        Stroke::new(1.0, t.text_muted.gamma_multiply(0.55))
    } else {
        Stroke::new(1.2, border_color)
    };
    let p = ui.painter();
    p.rect_filled(
        box_rect,
        CornerRadius::same(CONTROL_RADIUS as u8),
        t.field_bg,
    );
    p.rect_stroke(
        box_rect,
        CornerRadius::same(CONTROL_RADIUS as u8),
        stroke,
        egui::StrokeKind::Inside,
    );

    let paste_blocked = suppress_clipboard_events(ui, edit_id, box_rect);
    let inner_rect = box_rect.shrink2(Vec2::new(10.0, 2.0));
    let trailing_w = if matched { 20.0 } else { 0.0 };
    let text_w = (inner_rect.width() - trailing_w).max(40.0);

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 6.0;
    child.set_width(inner_rect.width());

    let resp = child.add_sized(
        Vec2::new(text_w, inner_rect.height()),
        egui::TextEdit::singleline(value)
            .id(edit_id)
            .desired_width(text_w)
            .font(egui::FontId::proportional(12.2))
            .margin(egui::Margin::ZERO)
            .align(egui::Align2::LEFT_CENTER)
            .frame(egui::Frame::NONE)
            .text_color(t.text)
            .hint_text(egui::RichText::new(hint).color(t.text_muted).size(12.0)),
    );
    let enter_pressed = resp.lost_focus() && child.input(|i| i.key_pressed(egui::Key::Enter));
    if matched {
        let (ir, _) = child.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        paint_icon(child.painter(), ir, Icon::Check, t.success_fg);
    }

    ManualConfirmInputResult {
        edit_id,
        changed: resp.changed(),
        enter_pressed,
        paste_blocked,
    }
}

fn danger_icon_button(ui: &mut egui::Ui, icon: Icon, selected: bool, t: &Tokens) -> egui::Response {
    let size = Vec2::splat(28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let fill = if selected || resp.hovered() {
            t.danger_bg
        } else {
            t.field_bg
        };
        let border = if selected || resp.hovered() {
            t.danger_border
        } else {
            t.border
        };
        let fg = if selected || resp.hovered() {
            t.danger_fg
        } else {
            t.text_muted
        };
        p.rect_filled(rect, CornerRadius::same(7), fill);
        p.rect_stroke(
            rect,
            CornerRadius::same(7),
            Stroke::new(if selected { 1.4 } else { 1.0 }, border),
            egui::StrokeKind::Inside,
        );
        paint_icon(
            p,
            egui::Rect::from_center_size(rect.center(), Vec2::splat(15.0)),
            icon,
            fg,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn draw_compact_pill(
    ui: &mut egui::Ui,
    icon: Option<Icon>,
    label: &str,
    fill: Color32,
    border: Color32,
    fg: Color32,
) {
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if let Some(ic) = icon {
                    let (ir, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                    paint_icon(ui.painter(), ir, ic, fg);
                }
                ui.label(egui::RichText::new(label).size(11.0).color(fg));
            });
        });
}

fn user_monogram(display_name: &str, username: &str) -> String {
    display_name
        .chars()
        .chain(username.chars())
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| "U".to_string(), |c| c.to_uppercase().collect::<String>())
}

fn short_user_id(user_id: &str) -> String {
    let prefix: String = user_id.chars().take(8).collect();
    if user_id.chars().count() > 8 {
        format!("{prefix}...")
    } else {
        prefix
    }
}

fn format_utc_date(created_at_ms: i64) -> String {
    if created_at_ms <= 0 {
        return "unknown date".to_string();
    }
    let secs = created_at_ms.div_euclid(1000);
    let days = secs.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + i64::from(m <= 2);
    format!("{year:04}-{m:02}-{d:02}")
}

fn render_advanced_card(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    draw_card(ui, t, |ui| {
        let header_resp = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let arrow = if ctrl.advanced_open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                };
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                paint_icon(ui.painter(), ir, arrow, t.text_muted);
                ui.label(
                    egui::RichText::new("Advanced settings")
                        .size(13.0)
                        .strong()
                        .color(t.text),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("Limits, sessions & startup")
                            .size(11.0)
                            .color(t.text_muted),
                    );
                });
            })
            .response
            .interact(Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);

        if header_resp.clicked() {
            ctrl.advanced_open = !ctrl.advanced_open;
        }

        if !ctrl.advanced_open {
            return;
        }

        ui.add_space(10.0);
        draw_hairline(ui, t);
        ui.add_space(10.0);

        render_numeric_setting_row(
            ui,
            "Max notebook upload size (MiB)",
            "Largest single notebook file accepted during sync.",
            &mut ctrl.draft_settings.max_notebook_mb,
            MIN_MAX_NOTEBOOK_MB..=MAX_MAX_NOTEBOOK_MB,
            t,
        );
        ui.add_space(8.0);

        render_numeric_setting_row(
            ui,
            "Stay signed in (days)",
            "How long devices stay signed in without re-entering a password.",
            &mut ctrl.draft_settings.refresh_ttl_days,
            MIN_REFRESH_TTL_DAYS..=MAX_REFRESH_TTL_DAYS,
            t,
        );
        ui.add_space(8.0);

        render_numeric_setting_row(
            ui,
            "Background login refresh (hours)",
            "How often active devices silently refresh their login token.",
            &mut ctrl.draft_settings.access_ttl_hours,
            MIN_ACCESS_TTL_HOURS..=MAX_ACCESS_TTL_HOURS,
            t,
        );

        ui.add_space(10.0);
        draw_hairline(ui, t);
        ui.add_space(8.0);

        ui.checkbox(
            &mut ctrl.draft_settings.auto_start_server,
            egui::RichText::new("Start server automatically when this app opens")
                .size(12.0)
                .color(t.text),
        );
        ui.label(
            egui::RichText::new("Recommended if you want sync ready right after launch.")
                .size(11.0)
                .color(t.text_muted),
        );

        ui.add_space(6.0);
        ui.checkbox(
            &mut ctrl.draft_settings.start_in_tray,
            egui::RichText::new("Start minimized in the system tray")
                .size(12.0)
                .color(t.text),
        );
        ui.label(
            egui::RichText::new(
                "Hides this window on manual launches too (use \"Open control window at system startup\" below to control sign-in launches).",
            )
            .size(11.0)
            .color(t.text_muted),
        );

        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, Icon::Reset, Some("Reset to defaults"), false, true, t).clicked() {
                ctrl.reset_defaults();
            }
        });
    });
}

fn render_numeric_setting_row<Num: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: &str,
    value: &mut Num,
    range: std::ops::RangeInclusive<Num>,
    t: &Tokens,
) {
    let input_w = 84.0_f32;
    let label_w = (ui.available_width() - input_w - 12.0).max(140.0);
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(label_w, 0.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(egui::RichText::new(title).size(12.0).strong().color(t.text));
                ui.add(
                    egui::Label::new(egui::RichText::new(subtitle).size(11.0).color(t.text_muted))
                        .wrap(),
                );
            },
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_sized(
                [input_w, 26.0],
                egui::DragValue::new(value).range(range).speed(1.0),
            );
        });
    });
}

fn render_footer(ui: &mut egui::Ui, t: &Tokens, actions: &mut UiActions) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Closing hides to tray; server keeps running.")
                .size(11.0)
                .color(t.text_muted),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if danger_button(ui, Icon::Close, "Quit", true, t)
                .on_hover_text("Stop the server and exit completely")
                .clicked()
            {
                actions.quit_app = true;
            }
            if secondary_button(ui, "Hide to Tray", true, t)
                .on_hover_text("Hide this window and keep running in the system tray")
                .clicked()
            {
                actions.hide_to_tray = true;
            }
        });
    });
}

fn draw_card(ui: &mut egui::Ui, t: &Tokens, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(t.panel_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CARD_RADIUS)
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            body(ui);
        });
}

fn draw_section_header(ui: &mut egui::Ui, icon: Icon, title: &str, subtitle: &str, t: &Tokens) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        paint_icon(ui.painter(), ir, icon, t.accent);
        ui.label(egui::RichText::new(title).size(13.5).strong().color(t.text));
    });
    ui.add_space(2.0);
    ui.label(egui::RichText::new(subtitle).size(11.0).color(t.text_muted));
}

fn draw_status_pill(ui: &mut egui::Ui, status: &ServerStatus, t: &Tokens) {
    let (fill, border, fg, icon, label) = match status {
        ServerStatus::Running => (
            t.success_bg,
            t.success_border,
            t.success_fg,
            Icon::CheckCircle,
            "Running",
        ),
        ServerStatus::Stopped => (
            t.toolbar_bg,
            t.border,
            t.text_muted,
            Icon::Server,
            "Stopped",
        ),
        ServerStatus::Error(_) => (
            t.danger_bg,
            t.danger_border,
            t.danger_fg,
            Icon::AlertCircle,
            "Needs attention",
        ),
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                paint_icon(ui.painter(), ir, icon, fg);
                ui.label(egui::RichText::new(label).size(12.0).strong().color(fg));
            });
        });
}

fn draw_banner(ui: &mut egui::Ui, banner: &NoticeBanner, dismissible: bool, t: &Tokens) -> bool {
    let mut dismissed = false;
    let (bg, border, fg, icon) = match banner.tone {
        NoticeTone::Info => (t.accent_soft, t.accent, t.text, Icon::Info),
        NoticeTone::Success => (
            t.success_bg,
            t.success_border,
            t.success_fg,
            Icon::CheckCircle,
        ),
        NoticeTone::Warning => (t.toolbar_bg, t.border, t.warning_fg, Icon::AlertCircle),
        NoticeTone::Error => (t.danger_bg, t.danger_border, t.danger_fg, Icon::AlertCircle),
    };
    egui::Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
                paint_icon(ui.painter(), ir, icon, fg);
                ui.add(
                    egui::Label::new(egui::RichText::new(&banner.text).size(11.5).color(fg)).wrap(),
                );
                if dismissible {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(ui, Icon::Close, None, false, true, t)
                            .on_hover_text("Dismiss message")
                            .clicked()
                        {
                            dismissed = true;
                        }
                    });
                }
            });
        });
    dismissed
}

fn draw_singleline_input(
    ui: &mut egui::Ui,
    id_salt: &str,
    width: f32,
    hint: &str,
    value: &mut String,
    t: &Tokens,
) {
    egui::Frame::new()
        .fill(t.field_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CONTROL_RADIUS)
        .inner_margin(Margin::symmetric(8, 0))
        .show(ui, |ui| {
            ui.set_width((width - 16.0).max(40.0));
            ui.set_height(INPUT_HEIGHT);
            ui.horizontal_centered(|ui| {
                ui.add_sized(
                    [ui.available_width(), INPUT_HEIGHT - 2.0],
                    egui::TextEdit::singleline(value)
                        .id_salt(id_salt)
                        .hint_text(egui::RichText::new(hint).color(t.text_muted).size(12.0))
                        .frame(egui::Frame::NONE)
                        .font(egui::FontId::proportional(12.0))
                        .text_color(t.text),
                );
            });
        });
}

fn draw_metric_cell(ui: &mut egui::Ui, icon: Option<Icon>, label: &str, value: &str, t: &Tokens) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if let Some(ic) = icon {
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                paint_icon(ui.painter(), ir, ic, t.text_muted);
            }
            ui.label(egui::RichText::new(label).size(10.5).color(t.text_muted));
        });
        ui.label(egui::RichText::new(value).size(12.5).strong().color(t.text));
    });
}

fn draw_hairline(ui: &mut egui::Ui, t: &Tokens) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, t.border));
}

struct ActionButtonStyle {
    fill: Color32,
    hover_fill: Color32,
    border: Color32,
    fg: Color32,
}

fn primary_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    enabled: bool,
    t: &Tokens,
) -> egui::Response {
    styled_action_button(
        ui,
        icon,
        label,
        ActionButtonStyle {
            fill: if enabled { t.accent } else { t.toolbar_bg },
            hover_fill: if enabled {
                t.accent_hover
            } else {
                t.toolbar_bg
            },
            border: if enabled { t.accent } else { t.border },
            fg: if enabled { t.on_accent } else { t.text_muted },
        },
        enabled,
    )
}

fn secondary_button(ui: &mut egui::Ui, label: &str, enabled: bool, t: &Tokens) -> egui::Response {
    let fg = if enabled { t.text } else { t.text_muted };
    let btn = egui::Button::new(egui::RichText::new(label).size(12.0).color(fg))
        .fill(t.field_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CONTROL_RADIUS)
        .min_size(Vec2::new(72.0, INPUT_HEIGHT));
    let resp = ui.add_enabled(enabled, btn);
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

fn fixed_secondary_button(
    ui: &mut egui::Ui,
    label: &str,
    width: f32,
    enabled: bool,
    t: &Tokens,
) -> egui::Response {
    let fg = if enabled { t.text } else { t.text_muted };
    let btn = egui::Button::new(egui::RichText::new(label).size(12.0).color(fg))
        .fill(t.field_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CONTROL_RADIUS)
        .min_size(Vec2::new(width, INPUT_HEIGHT));
    let resp = ui.add_enabled(enabled, btn);
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

fn danger_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    enabled: bool,
    t: &Tokens,
) -> egui::Response {
    styled_action_button(
        ui,
        icon,
        label,
        ActionButtonStyle {
            fill: t.danger_bg,
            hover_fill: t.danger_bg.gamma_multiply(0.92),
            border: t.danger_border,
            fg: if enabled { t.danger_fg } else { t.text_muted },
        },
        enabled,
    )
}

fn styled_action_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    style: ActionButtonStyle,
    enabled: bool,
) -> egui::Response {
    let font = egui::FontId::proportional(12.2);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font, style.fg);
    let pad_x = 12.0_f32;
    let icon_dim = 14.0_f32;
    let gap = 6.0_f32;
    let width = (pad_x * 2.0 + icon_dim + gap + galley.size().x).max(76.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, INPUT_HEIGHT), sense);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let bg = if enabled && resp.hovered() {
            style.hover_fill
        } else {
            style.fill
        };
        p.rect_filled(rect, CornerRadius::same(7), bg);
        p.rect_stroke(
            rect,
            CornerRadius::same(7),
            Stroke::new(1.0, style.border),
            egui::StrokeKind::Inside,
        );
        let ir = egui::Rect::from_center_size(
            egui::pos2(rect.left() + pad_x + icon_dim * 0.5, rect.center().y),
            Vec2::splat(icon_dim),
        );
        paint_icon(p, ir, icon, style.fg);
        let text_pos = egui::pos2(ir.right() + gap, rect.center().y - galley.size().y * 0.5);
        p.galley(text_pos, galley, style.fg);
    }
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::settings::CONFIG_FILE_NAME;
    use crate::theme::apply_visuals;
    use egui::{Pos2, RawInput, Rect, Shape};
    use std::path::PathBuf;

    struct TempDirGuard(PathBuf);
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir(label: &str) -> TempDirGuard {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let p = std::env::temp_dir().join(format!("bnc_ui_test_{label}_{nanos}"));
        let _ = std::fs::remove_dir_all(&p);
        TempDirGuard(p)
    }

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
    fn ui_renders_light_dark_stopped_running_and_advanced_states_cleanly() {
        let dir = temp_dir("render");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = crate::settings::ServerControlSettings {
            data_dir: dir.0.join("vault").to_string_lossy().into_owned(),
            port: 18944,
            auto_start_server: false,
            ..Default::default()
        };
        settings.save_atomic(&cfg_path).unwrap();

        let mut ctrl = ServerController::new(cfg_path, false);
        let ctx = egui::Context::default();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(540.0, 680.0))),
            ..Default::default()
        };

        // 1. Stopped + Light theme + Advanced collapsed
        ctrl.draft_settings.theme = UiTheme::Light;
        apply_visuals(&ctx, ctrl.theme());
        let light_tokens = Tokens::for_theme(ctrl.theme());
        let out1 = ctx.run_ui(raw, |ui| {
            render_window(&mut ctrl, ui, &light_tokens);
        });
        let text1 = collect_text(&out1.shapes);
        out1.drop_without_applying_deltas();
        assert!(text1.contains("PaperCraft Server"));
        assert!(text1.contains("Stopped"));
        assert!(text1.contains("Start Server"));
        assert!(text1.contains("Where to save your notebooks"));
        assert!(!text1.contains("Stay signed in (days)"));

        // 2. Running + Dark theme + Advanced expanded
        ctrl.draft_settings.theme = UiTheme::Dark;
        ctrl.advanced_open = true;
        ctrl.start_server();
        if !ctrl.is_running() {
            ctrl.draft_settings.port = 28944;
            ctrl.start_server();
        }
        assert!(ctrl.is_running());

        let raw_tall = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(560.0, 960.0))),
            ..Default::default()
        };
        apply_visuals(&ctx, ctrl.theme());
        let dark_tokens = Tokens::for_theme(ctrl.theme());
        let out2 = ctx.run_ui(raw_tall.clone(), |ui| {
            render_window(&mut ctrl, ui, &dark_tokens);
        });
        let text2 = collect_text(&out2.shapes);
        out2.drop_without_applying_deltas();
        assert!(text2.contains("Running"));
        assert!(text2.contains("Accounts"));
        assert!(text2.contains("Unsynced local accounts"));
        assert!(text2.contains("Local notebooks"));
        assert!(text2.contains("Stay signed in (days)"));
        assert!(text2.contains("Users (0)"));
        ctrl.stop_server();

        // 3. Tall viewport shows the App Updates card below Advanced settings
        ctrl.advanced_open = false;
        let raw_full = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(560.0, 1400.0))),
            ..Default::default()
        };
        let out3 = ctx.run_ui(raw_full, |ui| {
            render_window(&mut ctrl, ui, &dark_tokens);
        });
        let text3 = collect_text(&out3.shapes);
        out3.drop_without_applying_deltas();
        assert!(text3.contains("App Updates"));
        assert!(text3.contains("Check for updates automatically on startup"));
        assert!(text3.contains("Check for Updates"));
    }

    #[test]
    fn ui_renders_users_tab_and_reset_delete_states_cleanly() {
        let dir = temp_dir("render-users");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = crate::settings::ServerControlSettings {
            data_dir: dir.0.join("vault").to_string_lossy().into_owned(),
            port: 18945,
            auto_start_server: false,
            theme: UiTheme::Dark,
            ..Default::default()
        };
        settings.save_atomic(&cfg_path).unwrap();

        let mut ctrl = ServerController::new(cfg_path, false);
        ctrl.set_active_tab(ControlTab::Users);
        let ctx = egui::Context::default();
        let raw_tall = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(560.0, 960.0))),
            ..Default::default()
        };
        apply_visuals(&ctx, ctrl.theme());
        let dark_tokens = Tokens::for_theme(ctrl.theme());

        // 1. Empty Users tab state
        let out_empty = ctx.run_ui(raw_tall.clone(), |ui| {
            render_window(&mut ctrl, ui, &dark_tokens);
        });
        let text_empty = collect_text(&out_empty.shapes);
        out_empty.drop_without_applying_deltas();
        assert!(text_empty.contains("Registered Users"));
        assert!(text_empty.contains("No registered users yet"));
        assert!(!text_empty.contains("Where to save your notebooks"));

        // 2. Populated user card
        ctrl.users_snapshot = vec![UserSummary {
            user_id: "12345678-abcd-efgh-ijkl-999999999999".to_string(),
            email: None,
            username: "maria_notes".to_string(),
            display_name: "Maria Clara".to_string(),
            notebook_count: 3,
            created_at_ms: 1_763_424_000_000,
            must_change_password: false,
        }];
        let out_populated = ctx.run_ui(raw_tall.clone(), |ui| {
            render_window(&mut ctrl, ui, &dark_tokens);
        });
        let text_populated = collect_text(&out_populated.shapes);
        out_populated.drop_without_applying_deltas();
        assert!(text_populated.contains("Users (1)"));
        assert!(text_populated.contains("Maria Clara"));
        assert!(text_populated.contains("@maria_notes"));
        assert!(text_populated.contains("3 notebooks"));
        assert!(text_populated.contains("Joined 2025-11-18"));

        // 3. Reset confirmation, generated recovery code card, and delete confirmation
        ctrl.request_reset_password("12345678-abcd-efgh-ijkl-999999999999");
        let out_reset = ctx.run_ui(raw_tall.clone(), |ui| {
            render_window(&mut ctrl, ui, &dark_tokens);
        });
        let text_reset = collect_text(&out_reset.shapes);
        out_reset.drop_without_applying_deltas();
        assert!(text_reset.contains("Generate Reset Code"));

        ctrl.cancel_user_action();
        ctrl.last_password_reset = Some(PasswordResetOutcome {
            user_id: "12345678-abcd-efgh-ijkl-999999999999".to_string(),
            username: "maria_notes".to_string(),
            email: None,
            display_name: "Maria Clara".to_string(),
            temporary_password: "ABCD-EFGH-JKLM".to_string(),
        });
        ctrl.users_snapshot[0].must_change_password = true;
        ctrl.request_delete_user("12345678-abcd-efgh-ijkl-999999999999");
        let out_delete = ctx.run_ui(raw_tall, |ui| {
            render_window(&mut ctrl, ui, &dark_tokens);
        });
        let text_delete = collect_text(&out_delete.shapes);
        out_delete.drop_without_applying_deltas();
        assert!(text_delete.contains("ABCD-EFGH-JKLM"));
        assert!(text_delete.contains("Copy Code"));
        assert!(text_delete.contains("Reset pending"));
        assert!(text_delete.contains("Confirm Account Deletion"));
        assert!(text_delete.contains("delete maria_notes"));
        assert!(text_delete.contains("Move to Trash"));
    }

    #[test]
    fn users_view_requires_manual_typed_confirmation_blocks_paste_and_restores_from_trash() {
        use papercraft_server_cli::config::ensure_data_dir_and_meta;
        use papercraft_server_cli::db::{ServerDb, TokenTtls};

        fn find_text_center(
            shapes: &[egui::epaint::ClippedShape],
            target: &str,
        ) -> Option<egui::Pos2> {
            fn visit(shape: &Shape, target: &str, out: &mut Option<egui::Pos2>) {
                if out.is_some() {
                    return;
                }
                match shape {
                    Shape::Text(t) if t.galley.text() == target => {
                        *out = Some(t.visual_bounding_rect().center());
                    }
                    Shape::Vec(items) => {
                        for item in items {
                            visit(item, target, out);
                        }
                    }
                    _ => {}
                }
            }
            for c in shapes {
                let mut found = None;
                visit(&c.shape, target, &mut found);
                if let Some(pos) = found
                    && c.clip_rect.contains(pos)
                {
                    return Some(pos);
                }
            }
            None
        }

        let dir = temp_dir("users-trash-restore-ui");
        let vault_dir = dir.0.join("vault");
        let _ = ensure_data_dir_and_meta(&vault_dir).unwrap();
        let alice_id = {
            let db = ServerDb::open(&vault_dir.join("papercraft.redb")).unwrap();
            let ttls = TokenTtls {
                access_ttl_ms: 3_600_000,
                refresh_ttl_ms: 86_400_000,
            };
            db.register_user_with_email(
                Some("alice.very.long.email.address.for.clipping.check@example.com"),
                "alice_notes",
                "strong-password-1",
                Some("Alice Wonderfully Long Name"),
                Some("Desk"),
                true,
                ttls,
            )
            .unwrap()
            .user
            .user_id
        };

        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = crate::settings::ServerControlSettings {
            data_dir: vault_dir.to_string_lossy().into_owned(),
            port: 18951,
            auto_start_server: false,
            theme: UiTheme::Dark,
            ..Default::default()
        };
        settings.save_atomic(&cfg_path).unwrap();

        let mut ctrl = ServerController::new(cfg_path, false);
        ctrl.set_active_tab(ControlTab::Users);
        assert_eq!(ctrl.users_snapshot.len(), 1);

        let ctx = egui::Context::default();
        apply_visuals(&ctx, ctrl.theme());
        let dark_tokens = Tokens::for_theme(ctrl.theme());
        let mut t = 1.0_f64;

        let run_ui =
            |ctrl: &mut ServerController, time: &mut f64, width: f32, events: Vec<egui::Event>| {
                *time += 0.05;
                let out = ctx.run_ui(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 960.0))),
                        time: Some(*time),
                        predicted_dt: 0.05,
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let _ = render_window_with_update_status(
                            ctrl,
                            ui,
                            &dark_tokens,
                            &papercraft_updater::UpdateStatus::Idle,
                        );
                    },
                );
                let text = collect_text(&out.shapes);
                let shapes = out.shapes.clone();
                out.drop_without_applying_deltas();
                (text, shapes)
            };

        // 1. Narrow 460px width renders row and Active/Trash switch cleanly without clipping.
        let (narrow_text, _) = run_ui(&mut ctrl, &mut t, 460.0, Vec::new());
        assert!(narrow_text.contains("Active (1)"));
        assert!(narrow_text.contains("Trash (0)"));

        // 2. Request delete -> focus goes to manual input -> Paste / Ctrl+V is blocked.
        ctrl.request_delete_user(&alice_id);
        let _ = run_ui(&mut ctrl, &mut t, 560.0, Vec::new());
        let _ = run_ui(
            &mut ctrl,
            &mut t,
            560.0,
            vec![
                egui::Event::Paste("delete alice_notes".to_string()),
                egui::Event::Key {
                    key: egui::Key::V,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL,
                },
            ],
        );
        let (paste_text, paste_shapes) = run_ui(&mut ctrl, &mut t, 560.0, Vec::new());
        assert!(ctrl.delete_confirm_input.is_empty());
        assert!(ctrl.delete_confirm_paste_blocked);
        assert!(paste_text.contains("Copy/paste is disabled — type it manually"));

        // Clicking Move to Trash while empty does not trash the account.
        let move_btn = find_text_center(&paste_shapes, "Move to Trash").unwrap();
        for pressed in [true, false] {
            let _ = run_ui(
                &mut ctrl,
                &mut t,
                560.0,
                vec![
                    egui::Event::PointerMoved(move_btn),
                    egui::Event::PointerButton {
                        pos: move_btn,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(ctrl.users_snapshot.len(), 1);
        assert!(ctrl.trashed_users_snapshot.is_empty());

        // 3. Click the manual confirmation input and type `delete alice_notes` to enable Move to Trash.
        let manual_lbl = find_text_center(&paste_shapes, "Manual Confirmation").unwrap();
        let input_pos = egui::pos2(manual_lbl.x + 12.0, manual_lbl.y + 24.0);
        for pressed in [true, false] {
            let _ = run_ui(
                &mut ctrl,
                &mut t,
                560.0,
                vec![
                    egui::Event::PointerMoved(input_pos),
                    egui::Event::PointerButton {
                        pos: input_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        let _ = run_ui(
            &mut ctrl,
            &mut t,
            560.0,
            vec![egui::Event::Text("delete alice_notes".to_string())],
        );
        let (matched_text, matched_shapes) = run_ui(&mut ctrl, &mut t, 560.0, Vec::new());
        assert!(matched_text.contains("Confirmation matched"));
        let enabled_move_btn = find_text_center(&matched_shapes, "Move to Trash").unwrap();
        for pressed in [true, false] {
            let _ = run_ui(
                &mut ctrl,
                &mut t,
                560.0,
                vec![
                    egui::Event::PointerMoved(enabled_move_btn),
                    egui::Event::PointerButton {
                        pos: enabled_move_btn,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(ctrl.users_snapshot.is_empty());
        assert_eq!(ctrl.trashed_users_snapshot.len(), 1);

        // 4. Switch renders even when Active is empty; clicking Trash (1) shows 30-day retention & Restore.
        let (empty_active_text, empty_active_shapes) = run_ui(&mut ctrl, &mut t, 560.0, Vec::new());
        assert!(empty_active_text.contains("Active (0)"));
        assert!(empty_active_text.contains("Trash (1)"));
        let trash_tab = find_text_center(&empty_active_shapes, "Trash (1)").unwrap();
        for pressed in [true, false] {
            let _ = run_ui(
                &mut ctrl,
                &mut t,
                560.0,
                vec![
                    egui::Event::PointerMoved(trash_tab),
                    egui::Event::PointerButton {
                        pos: trash_tab,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(ctrl.users_view_mode, UsersViewMode::Trash);

        let (trash_text, trash_shapes) = run_ui(&mut ctrl, &mut t, 560.0, Vec::new());
        assert!(trash_text.contains("30 days left to restore"));
        assert!(trash_text.contains("Restore"));

        // 5. Clicking Restore brings Alice back to Active.
        let restore_btn = find_text_center(&trash_shapes, "Restore").unwrap();
        for pressed in [true, false] {
            let _ = run_ui(
                &mut ctrl,
                &mut t,
                560.0,
                vec![
                    egui::Event::PointerMoved(restore_btn),
                    egui::Event::PointerButton {
                        pos: restore_btn,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(ctrl.users_snapshot.len(), 1);
        assert!(ctrl.trashed_users_snapshot.is_empty());
        assert_eq!(ctrl.users_view_mode, UsersViewMode::Active);
    }

    #[test]
    fn utc_date_formatter_handles_epoch_and_invalid_values() {
        assert_eq!(format_utc_date(0), "unknown date");
        assert_eq!(format_utc_date(1_000), "1970-01-01");
        assert_eq!(format_utc_date(1_763_424_000_000), "2025-11-18");
    }

    #[test]
    fn server_update_progress_bar_renders_determinate_and_indeterminate_cleanly() {
        let ctx = egui::Context::default();
        for theme in [UiTheme::Light, UiTheme::Dark] {
            let tokens = Tokens::for_theme(theme);
            for (done, total) in [(0, 1000), (500, 1000), (1000, 1000), (2048, 0)] {
                let raw = RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(420.0, 80.0))),
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| {
                    draw_update_progress_bar(ui, done, total, &tokens);
                });
                assert!(!out.shapes.is_empty());
                out.drop_without_applying_deltas();
            }
        }
    }

    fn find_text_y(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<f32> {
        fn visit(shape: &Shape, needle: &str) -> Option<f32> {
            match shape {
                Shape::Text(t) if t.galley.text().contains(needle) => Some(t.pos.y),
                Shape::Vec(items) => items.iter().find_map(|s| visit(s, needle)),
                _ => None,
            }
        }
        shapes.iter().find_map(|c| visit(&c.shape, needle))
    }

    #[test]
    fn server_update_states_render_inside_app_updates_card_in_consistent_order() {
        use papercraft_updater::{
            AppId, AssetFormat, AvailableUpdate, InstallCapability, PlatformAsset, SemVer,
            StagedUpdate, UpdateStatus,
        };

        let dir = temp_dir("update-order");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = crate::settings::ServerControlSettings {
            data_dir: dir.0.join("vault").to_string_lossy().into_owned(),
            port: 18947,
            auto_start_server: false,
            theme: UiTheme::Dark,
            ..Default::default()
        };
        settings.save_atomic(&cfg_path).unwrap();
        let mut ctrl = ServerController::new(cfg_path, false);

        let update = AvailableUpdate {
            app: AppId::PaperCraftServer,
            current_version: SemVer::parse("0.1.2").unwrap(),
            version: SemVer::parse("0.2.0").unwrap(),
            tag: "v0.2.0".to_string(),
            release_url: "https://github.com/Alyx-Labs/papercraft/releases/tag/v0.2.0".to_string(),
            published_at: "2026-11-24T16:00:00Z".to_string(),
            notes: "- feat: release notes line".to_string(),
            platform: "windows-x86_64".to_string(),
            asset: PlatformAsset {
                file_name: "papercraft-server-windows-x86_64.exe".to_string(),
                url: "https://github.com/Alyx-Labs/papercraft/releases/download/v0.2.0/papercraft-server-windows-x86_64.exe".to_string(),
                sha256: "0".repeat(64),
                size_bytes: 24_117_248,
                format: AssetFormat::Exe,
            },
            capability: InstallCapability::Supported {
                target_path: dir.0.join("papercraft-server.exe"),
                staging_dir: dir.0.clone(),
            },
            is_custom_feed: false,
        };

        let ctx = egui::Context::default();
        let raw_tall = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(620.0, 1400.0))),
            ..Default::default()
        };
        apply_visuals(&ctx, ctrl.theme());
        let dark_tokens = Tokens::for_theme(ctrl.theme());

        // 1. Available state renders inside App Updates (below Server Settings, never above it).
        let available_status = UpdateStatus::Available(Box::new(update.clone()));
        let out_avail = ctx.run_ui(raw_tall.clone(), |ui| {
            let _ =
                render_window_with_update_status(&mut ctrl, ui, &dark_tokens, &available_status);
        });
        let text_avail = collect_text(&out_avail.shapes);
        let y_connect = find_text_y(&out_avail.shapes, "Server address").unwrap();
        let y_settings = find_text_y(&out_avail.shapes, "Where to save your notebooks").unwrap();
        let y_updates_hdr = find_text_y(&out_avail.shapes, "App Updates").unwrap();
        let y_avail = find_text_y(&out_avail.shapes, "Update available: v0.2.0").unwrap();
        let y_auto_chk = find_text_y(
            &out_avail.shapes,
            "Check for updates automatically on startup",
        )
        .unwrap();
        out_avail.drop_without_applying_deltas();

        assert!(text_avail.contains("Download Update"));
        assert!(y_connect < y_settings);
        assert!(y_settings < y_updates_hdr);
        assert!(y_updates_hdr < y_avail);
        assert!(y_avail < y_auto_chk);

        // 2. Downloading and ReadyToInstall states also render inside App Updates cleanly.
        let downloading_status = UpdateStatus::Downloading {
            update: Box::new(update.clone()),
            downloaded_bytes: 12_000_000,
            total_bytes: 24_000_000,
        };
        let out_dl = ctx.run_ui(raw_tall.clone(), |ui| {
            let _ =
                render_window_with_update_status(&mut ctrl, ui, &dark_tokens, &downloading_status);
        });
        let text_dl = collect_text(&out_dl.shapes);
        let y_dl = find_text_y(&out_dl.shapes, "Downloading v0.2.0...").unwrap();
        let y_dl_hdr = find_text_y(&out_dl.shapes, "App Updates").unwrap();
        let y_dl_chk =
            find_text_y(&out_dl.shapes, "Check for updates automatically on startup").unwrap();
        out_dl.drop_without_applying_deltas();
        assert!(text_dl.contains("Cancel"));
        assert!(y_dl_hdr < y_dl && y_dl < y_dl_chk);

        let ready_status = UpdateStatus::ReadyToInstall {
            update: Box::new(update.clone()),
            staged: StagedUpdate {
                app: AppId::PaperCraftServer,
                version: SemVer::parse("0.2.0").unwrap(),
                staged_path: dir.0.join("staged.exe"),
                target_path: dir.0.join("papercraft-server.exe"),
                format: AssetFormat::Exe,
                sha256: "0".repeat(64),
            },
        };
        let out_ready = ctx.run_ui(raw_tall, |ui| {
            let _ = render_window_with_update_status(&mut ctrl, ui, &dark_tokens, &ready_status);
        });
        let text_ready = collect_text(&out_ready.shapes);
        let y_ready = find_text_y(
            &out_ready.shapes,
            "Update v0.2.0 verified and ready to install",
        )
        .unwrap();
        let y_ready_hdr = find_text_y(&out_ready.shapes, "App Updates").unwrap();
        let y_ready_chk = find_text_y(
            &out_ready.shapes,
            "Check for updates automatically on startup",
        )
        .unwrap();
        out_ready.drop_without_applying_deltas();
        assert!(text_ready.contains("Stop Server, Install & Restart"));
        assert!(y_ready_hdr < y_ready && y_ready < y_ready_chk);
    }

    #[test]
    fn desktop_shortcut_switch_renders_below_auto_check_and_invokes_hook() {
        use std::sync::{Arc, Mutex};

        let dir = temp_dir("desktop-shortcut-switch");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = crate::settings::ServerControlSettings {
            data_dir: dir.0.join("vault").to_string_lossy().into_owned(),
            port: 18948,
            auto_start_server: false,
            theme: UiTheme::Dark,
            ..Default::default()
        };
        settings.save_atomic(&cfg_path).unwrap();

        let calls = Arc::new(Mutex::new(Vec::<bool>::new()));
        let calls_hook = Arc::clone(&calls);
        let mut ctrl = ServerController::new(cfg_path.clone(), false);
        ctrl.set_desktop_shortcut_sync(Box::new(move |enabled| {
            calls_hook
                .lock()
                .map_err(|err| err.to_string())?
                .push(enabled);
            Ok(if enabled {
                "Server shortcut ready (mock).".to_string()
            } else {
                "Server shortcut removed (mock).".to_string()
            })
        }));

        let find_rect = |shapes: &[egui::epaint::ClippedShape], needle: &str| -> Option<Rect> {
            fn visit(shape: &Shape, needle: &str) -> Option<Rect> {
                match shape {
                    Shape::Text(t) if t.galley.text().contains(needle) => {
                        Some(Rect::from_min_size(t.pos, t.galley.size()))
                    }
                    Shape::Vec(items) => items.iter().find_map(|s| visit(s, needle)),
                    _ => None,
                }
            }
            shapes.iter().find_map(|c| visit(&c.shape, needle))
        };

        let ctx = egui::Context::default();
        apply_visuals(&ctx, ctrl.theme());
        let tokens = Tokens::for_theme(ctrl.theme());
        let idle = papercraft_updater::UpdateStatus::Idle;
        let run_frame = |ctrl: &mut ServerController, t: f64, events: Vec<egui::Event>| {
            let out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(620.0, 1450.0))),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let _ = render_window_with_update_status(ctrl, ui, &tokens, &idle);
                },
            );
            let shapes = out.shapes.clone();
            out.drop_without_applying_deltas();
            shapes
        };

        let _ = run_frame(&mut ctrl, 1.0, Vec::new());
        let shapes = run_frame(&mut ctrl, 1.05, Vec::new());
        let y_auto_chk =
            find_text_y(&shapes, "Check for updates automatically on startup").unwrap();
        let shortcut_rect = find_rect(&shapes, "Create Desktop shortcut automatically").unwrap();
        assert!(y_auto_chk < shortcut_rect.min.y);

        let pos = shortcut_rect.center();
        let _ = run_frame(
            &mut ctrl,
            1.10,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let shapes_off = run_frame(
            &mut ctrl,
            1.14,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(!ctrl.draft_settings.auto_create_desktop_shortcut);
        assert!(find_rect(&shapes_off, "Server shortcut removed (mock).").is_none());

        let pos_on = find_rect(&shapes_off, "Create Desktop shortcut automatically")
            .unwrap()
            .center();
        let _ = run_frame(
            &mut ctrl,
            1.20,
            vec![
                egui::Event::PointerMoved(pos_on),
                egui::Event::PointerButton {
                    pos: pos_on,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let shapes_on = run_frame(
            &mut ctrl,
            1.24,
            vec![egui::Event::PointerButton {
                pos: pos_on,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(ctrl.draft_settings.auto_create_desktop_shortcut);
        assert!(find_rect(&shapes_on, "Server shortcut ready (mock).").is_none());
        assert_eq!(*calls.lock().unwrap(), vec![false, true]);

        let (reloaded, warning) = crate::settings::ServerControlSettings::load_or_init(&cfg_path);
        assert!(warning.is_none());
        assert!(reloaded.auto_create_desktop_shortcut);
    }

    #[test]
    fn autostart_switch_renders_below_desktop_shortcut_and_invokes_hook() {
        use std::sync::{Arc, Mutex};

        let dir = temp_dir("autostart-switch");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = crate::settings::ServerControlSettings {
            data_dir: dir.0.join("vault").to_string_lossy().into_owned(),
            port: 18949,
            auto_start_server: false,
            theme: UiTheme::Dark,
            ..Default::default()
        };
        settings.save_atomic(&cfg_path).unwrap();

        let calls = Arc::new(Mutex::new(Vec::<(bool, bool)>::new()));
        let calls_hook = Arc::clone(&calls);
        let mut ctrl = ServerController::new(cfg_path.clone(), false);
        ctrl.set_autostart_sync(Box::new(move |enabled, clear_override| {
            calls_hook
                .lock()
                .map_err(|err| err.to_string())?
                .push((enabled, clear_override));
            Ok(if enabled {
                "Startup launch ready (mock).".to_string()
            } else {
                "Startup launch removed (mock).".to_string()
            })
        }));

        let find_rect = |shapes: &[egui::epaint::ClippedShape], needle: &str| -> Option<Rect> {
            fn visit(shape: &Shape, needle: &str) -> Option<Rect> {
                match shape {
                    Shape::Text(t) if t.galley.text().contains(needle) => {
                        Some(Rect::from_min_size(t.pos, t.galley.size()))
                    }
                    Shape::Vec(items) => items.iter().find_map(|s| visit(s, needle)),
                    _ => None,
                }
            }
            shapes.iter().find_map(|c| visit(&c.shape, needle))
        };

        let ctx = egui::Context::default();
        apply_visuals(&ctx, ctrl.theme());
        let tokens = Tokens::for_theme(ctrl.theme());
        let idle = papercraft_updater::UpdateStatus::Idle;
        let run_frame = |ctrl: &mut ServerController, t: f64, events: Vec<egui::Event>| {
            let out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(620.0, 1650.0))),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let _ = render_window_with_update_status(ctrl, ui, &tokens, &idle);
                },
            );
            let shapes = out.shapes.clone();
            out.drop_without_applying_deltas();
            shapes
        };

        let _ = run_frame(&mut ctrl, 1.0, Vec::new());
        let shapes = run_frame(&mut ctrl, 1.05, Vec::new());
        let y_shortcut = find_text_y(&shapes, "Create Desktop shortcut automatically").unwrap();
        let autostart_rect = find_rect(&shapes, "Launch automatically at system startup").unwrap();
        assert!(y_shortcut < autostart_rect.min.y);

        let pos = autostart_rect.center();
        let _ = run_frame(
            &mut ctrl,
            1.10,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let shapes_off = run_frame(
            &mut ctrl,
            1.14,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(!ctrl.draft_settings.launch_at_login);
        assert!(find_rect(&shapes_off, "Startup launch removed (mock).").is_none());

        let pos_on = find_rect(&shapes_off, "Launch automatically at system startup")
            .unwrap()
            .center();
        let _ = run_frame(
            &mut ctrl,
            1.20,
            vec![
                egui::Event::PointerMoved(pos_on),
                egui::Event::PointerButton {
                    pos: pos_on,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let shapes_on = run_frame(
            &mut ctrl,
            1.24,
            vec![egui::Event::PointerButton {
                pos: pos_on,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(ctrl.draft_settings.launch_at_login);
        assert!(find_rect(&shapes_on, "Startup launch ready (mock).").is_none());
        assert_eq!(*calls.lock().unwrap(), vec![(false, true), (true, true)]);

        // Sync failures still render visibly in error styling.
        ctrl.set_autostart_sync(Box::new(|_, _| Err("Startup registry locked".to_string())));
        let autostart_err_pos = find_rect(&shapes_on, "Launch automatically at system startup")
            .unwrap()
            .center();
        let _ = run_frame(
            &mut ctrl,
            1.30,
            vec![
                egui::Event::PointerMoved(autostart_err_pos),
                egui::Event::PointerButton {
                    pos: autostart_err_pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let shapes_err = run_frame(
            &mut ctrl,
            1.34,
            vec![egui::Event::PointerButton {
                pos: autostart_err_pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(find_rect(&shapes_err, "Startup registry locked").is_some());

        ctrl.set_launch_at_login_enabled(true);
        let (reloaded, warning) = crate::settings::ServerControlSettings::load_or_init(&cfg_path);
        assert!(warning.is_none());
        assert!(reloaded.launch_at_login);
    }

    #[test]
    fn autostart_window_checkbox_renders_below_autostart_and_persists_when_enabled() {
        let dir = temp_dir("autostart-window-checkbox");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = crate::settings::ServerControlSettings {
            data_dir: dir.0.join("vault").to_string_lossy().into_owned(),
            port: 18950,
            auto_start_server: false,
            theme: UiTheme::Dark,
            ..Default::default()
        };
        settings.save_atomic(&cfg_path).unwrap();

        let mut ctrl = ServerController::new(cfg_path.clone(), false);
        assert!(ctrl.draft_settings.launch_at_login);
        assert!(!ctrl.draft_settings.open_window_at_login);

        let find_rect = |shapes: &[egui::epaint::ClippedShape], needle: &str| -> Option<Rect> {
            fn visit(shape: &Shape, needle: &str) -> Option<Rect> {
                match shape {
                    Shape::Text(t) if t.galley.text().contains(needle) => {
                        Some(Rect::from_min_size(t.pos, t.galley.size()))
                    }
                    Shape::Vec(items) => items.iter().find_map(|s| visit(s, needle)),
                    _ => None,
                }
            }
            shapes.iter().find_map(|c| visit(&c.shape, needle))
        };

        let ctx = egui::Context::default();
        apply_visuals(&ctx, ctrl.theme());
        let tokens = Tokens::for_theme(ctrl.theme());
        let idle = papercraft_updater::UpdateStatus::Idle;
        let run_frame = |ctrl: &mut ServerController, t: f64, events: Vec<egui::Event>| {
            let out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(620.0, 1750.0))),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let _ = render_window_with_update_status(ctrl, ui, &tokens, &idle);
                },
            );
            let shapes = out.shapes.clone();
            out.drop_without_applying_deltas();
            shapes
        };

        let _ = run_frame(&mut ctrl, 1.0, Vec::new());
        let shapes = run_frame(&mut ctrl, 1.05, Vec::new());
        let y_autostart = find_text_y(&shapes, "Launch automatically at system startup").unwrap();
        let chk_rect = find_rect(&shapes, "Open control window at system startup").unwrap();
        assert!(y_autostart < chk_rect.min.y);

        // Clicking when `launch_at_login` is enabled toggles and saves `open_window_at_login`.
        let pos = chk_rect.center();
        let _ = run_frame(
            &mut ctrl,
            1.10,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let shapes_checked = run_frame(
            &mut ctrl,
            1.14,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(ctrl.draft_settings.open_window_at_login);

        let (reloaded, warning) = crate::settings::ServerControlSettings::load_or_init(&cfg_path);
        assert!(warning.is_none());
        assert!(reloaded.open_window_at_login);

        // When `launch_at_login` is disabled, clicking the checkbox does not toggle it.
        ctrl.set_launch_at_login_enabled(false);
        let shapes_disabled = run_frame(&mut ctrl, 1.20, Vec::new());
        let pos_disabled = find_rect(&shapes_disabled, "Open control window at system startup")
            .unwrap()
            .center();
        let _ = run_frame(
            &mut ctrl,
            1.25,
            vec![
                egui::Event::PointerMoved(pos_disabled),
                egui::Event::PointerButton {
                    pos: pos_disabled,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let _ = run_frame(
            &mut ctrl,
            1.29,
            vec![egui::Event::PointerButton {
                pos: pos_disabled,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(ctrl.draft_settings.open_window_at_login);
        let _ = shapes_checked;
    }
}
