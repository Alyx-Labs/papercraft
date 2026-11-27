//! Server Settings view: storage folder, sync limits, startup behavior, and app updates.

use egui::{CornerRadius, Margin, Sense, Stroke, Vec2};
use papercraft_updater::{
    AvailableUpdate, InstallCapability, UpdateStatus, format_bytes as upd_format_bytes,
};

use crate::controller::ServerController;
use crate::icons::{
    Icon, icon_button, icon_button_width, paint_icon, toggle_switch_row, toggle_switch_row_enabled,
};
use crate::settings::{
    MAX_ACCESS_TTL_HOURS, MAX_MAX_NOTEBOOK_MB, MAX_REFRESH_TTL_DAYS, MIN_ACCESS_TTL_HOURS,
    MIN_MAX_NOTEBOOK_MB, MIN_REFRESH_TTL_DAYS,
};
use crate::theme::Tokens;
use crate::ui::widgets::{
    PageHeaderBadge, draw_compact_pill, draw_hairline, draw_page_header, draw_section_header,
    draw_section_panel, draw_singleline_input, draw_soft_strip, fixed_secondary_button,
    primary_button, secondary_button,
};
use crate::ui::{TWO_COL_BREAKPOINT, UiActions};

pub fn render_settings_page(
    ctrl: &mut ServerController,
    ctx: &egui::Context,
    update_status: &UpdateStatus,
    ui: &mut egui::Ui,
    t: &Tokens,
    actions: &mut UiActions,
) {
    draw_page_header(
        ui,
        "Settings",
        None::<PageHeaderBadge<'_>>,
        "Changes save automatically. Network access and sign-up rules live on Overview.",
        t,
        |ui| {
            if icon_button(ui, Icon::Reset, Some("Reset to defaults"), false, true, t)
                .on_hover_text("Restore all server settings to recommended defaults")
                .clicked()
            {
                ctrl.reset_defaults();
            }
        },
    );

    ui.add_space(14.0);

    let avail_w = ui.available_width();
    if avail_w >= TWO_COL_BREAKPOINT {
        let gap = 16.0_f32;
        let col_w = ((avail_w - gap) * 0.5).floor();
        let right_w = (avail_w - gap - col_w).max(200.0);

        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.allocate_ui_with_layout(
                Vec2::new(col_w, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(col_w);
                    ui.spacing_mut().item_spacing.y = 14.0;
                    render_storage_panel(ctrl, ctx, ui, t);
                    render_limits_panel(ctrl, ui, t);
                },
            );
            ui.allocate_ui_with_layout(
                Vec2::new(right_w, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(right_w);
                    ui.spacing_mut().item_spacing.y = 14.0;
                    render_updates_panel(ctrl, update_status, ui, t, actions);
                    render_startup_panel(ctrl, ui, t);
                },
            );
        });
    } else {
        ui.spacing_mut().item_spacing.y = 12.0;
        render_storage_panel(ctrl, ctx, ui, t);
        render_limits_panel(ctrl, ui, t);
        render_updates_panel(ctrl, update_status, ui, t, actions);
        render_startup_panel(ctrl, ui, t);
    }
}

fn render_storage_panel(
    ctrl: &mut ServerController,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    t: &Tokens,
) {
    draw_section_panel(ui, t, |ui| {
        draw_section_header(
            ui,
            Icon::FolderOpen,
            "Storage Folder",
            "Folder where accounts and synced notebooks are saved (papercraft.redb).",
            t,
        );
        ui.add_space(12.0);

        ui.label(
            egui::RichText::new("Where to save your notebooks")
                .size(12.5)
                .strong()
                .color(t.text),
        );
        ui.add_space(6.0);

        let picking = ctrl.is_picking_folder();
        let browse_lbl = if picking { "Opening..." } else { "Browse..." };
        let browse_w = icon_button_width(ui, Some(browse_lbl));
        let open_w = 60.0_f32;
        let gap = 6.0_f32;
        let input_w = (ui.available_width() - browse_w - open_w - gap * 2.0).max(110.0);

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
    });
}

fn render_limits_panel(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    draw_section_panel(ui, t, |ui| {
        draw_section_header(
            ui,
            Icon::Settings,
            "Sync & Session Limits",
            "Upload size limits and how long devices stay signed in.",
            t,
        );
        ui.add_space(12.0);

        render_numeric_setting_row(
            ui,
            "Max notebook upload size (MiB)",
            "Largest single notebook file accepted during sync.",
            &mut ctrl.draft_settings.max_notebook_mb,
            MIN_MAX_NOTEBOOK_MB..=MAX_MAX_NOTEBOOK_MB,
            t,
        );

        ui.add_space(10.0);
        draw_hairline(ui, t);
        ui.add_space(10.0);

        render_numeric_setting_row(
            ui,
            "Stay signed in (days)",
            "How long devices stay signed in without re-entering a password.",
            &mut ctrl.draft_settings.refresh_ttl_days,
            MIN_REFRESH_TTL_DAYS..=MAX_REFRESH_TTL_DAYS,
            t,
        );

        ui.add_space(10.0);
        draw_hairline(ui, t);
        ui.add_space(10.0);

        render_numeric_setting_row(
            ui,
            "Background login refresh (hours)",
            "How often active devices silently refresh their login token.",
            &mut ctrl.draft_settings.access_ttl_hours,
            MIN_ACCESS_TTL_HOURS..=MAX_ACCESS_TTL_HOURS,
            t,
        );
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
                ui.spacing_mut().item_spacing.y = 2.0;
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

fn render_startup_panel(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    draw_section_panel(ui, t, |ui| {
        draw_section_header(
            ui,
            Icon::Server,
            "Startup & System",
            "Automatic server start, tray behavior, Desktop shortcut, and login startup.",
            t,
        );
        ui.add_space(12.0);

        toggle_switch_row(
            ui,
            "srv_auto_start_switch",
            "Start server automatically when this app opens",
            "Start accepting sync connections immediately when PaperCraft Server launches.",
            &mut ctrl.draft_settings.auto_start_server,
            t,
        );
        ui.add_space(2.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new("Recommended if you want sync ready right after launch.")
                    .size(11.0)
                    .color(t.text_muted),
            )
            .wrap(),
        );

        ui.add_space(10.0);
        draw_hairline(ui, t);
        ui.add_space(10.0);

        toggle_switch_row(
            ui,
            "srv_start_in_tray_switch",
            "Start minimized in the system tray",
            "Keep the window hidden when launched manually.",
            &mut ctrl.draft_settings.start_in_tray,
            t,
        );
        ui.add_space(2.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "Hides this window on manual launches too; sign-in launches use the switch below.",
                )
                .size(11.0)
                .color(t.text_muted),
            )
            .wrap(),
        );

        ui.add_space(10.0);
        draw_hairline(ui, t);
        ui.add_space(10.0);

        render_desktop_shortcut_section(ctrl, ui, t);

        ui.add_space(10.0);
        draw_hairline(ui, t);
        ui.add_space(10.0);

        render_autostart_section(ctrl, ui, t);
    });
}

fn render_desktop_shortcut_section(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
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
    ui.add_space(2.0);
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
    ui.add_space(2.0);
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

    ui.add_space(8.0);
    let autostart_enabled = ctrl.draft_settings.launch_at_login;
    ui.indent("srv_autostart_window_indent", |ui| {
        toggle_switch_row_enabled(
            ui,
            "srv_autostart_window_switch",
            "Open control window at system startup",
            "Open the PaperCraft Server control window automatically when you sign in instead of keeping it hidden in the system tray.",
            &mut ctrl.draft_settings.open_window_at_login,
            autostart_enabled,
            t,
        );
        ui.add_space(2.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "Leave off to keep PaperCraft Server hidden in the system tray when your computer starts. Open it anytime from the tray icon.",
                )
                .size(11.0)
                .color(t.text_muted),
            )
            .wrap(),
        );
    });
}

fn render_updates_panel(
    ctrl: &mut ServerController,
    status: &UpdateStatus,
    ui: &mut egui::Ui,
    t: &Tokens,
    actions: &mut UiActions,
) {
    let current_ver = ctrl.updater.config().map_or_else(
        || env!("CARGO_PKG_VERSION").to_string(),
        |cfg| cfg.current_version.to_string(),
    );
    let is_custom_feed = ctrl.updater.config().is_some_and(|cfg| cfg.is_custom_feed);
    let busy = matches!(
        status,
        UpdateStatus::Checking { .. } | UpdateStatus::Downloading { .. }
    );

    draw_section_panel(ui, t, |ui| {
        draw_section_header(
            ui,
            Icon::MoveDown,
            "App Updates",
            "Check GitHub Releases for newer PaperCraft Server builds.",
            t,
        );
        ui.add_space(12.0);

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(
                egui::RichText::new(format!(
                    "Installed version: {} · {}",
                    papercraft_doc::format_alpha_version(&current_ver),
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
        ui.add_space(10.0);

        toggle_switch_row(
            ui,
            "srv_auto_check_updates_switch",
            "Check for updates automatically on startup",
            "Quietly check GitHub Releases for new versions when PaperCraft Server starts.",
            &mut ctrl.draft_settings.auto_check_updates,
            t,
        );
        ui.add_space(2.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "Checks GitHub Releases quietly when PaperCraft Server starts. Turn off for completely offline use.",
                )
                .size(11.0)
                .color(t.text_muted),
            )
            .wrap(),
        );
    });
}

fn render_update_status_details(
    ctrl: &mut ServerController,
    status: &UpdateStatus,
    is_custom_feed: bool,
    ui: &mut egui::Ui,
    t: &Tokens,
    actions: &mut UiActions,
) {
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
            ui.add_space(8.0);
            ui.add(
                egui::Label::new(egui::RichText::new(msg).size(11.0).color(t.success_fg)).wrap(),
            );
        }
        UpdateStatus::NoPlatformAsset { latest_version, .. } => {
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
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
            ui.add_space(8.0);
            ui.add(
                egui::Label::new(egui::RichText::new(message).size(11.0).color(t.danger_fg)).wrap(),
            );
        }
        _ => {}
    }
}

fn render_update_available_box(
    ctrl: &mut ServerController,
    update: &AvailableUpdate,
    is_custom_feed: bool,
    ui: &mut egui::Ui,
    t: &Tokens,
) {
    draw_soft_strip(
        ui,
        t.accent_soft,
        Stroke::new(1.0, t.accent),
        Margin::symmetric(12, 10),
        |ui| {
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
        },
    );
}

fn render_update_downloading_box(
    ctrl: &mut ServerController,
    update: &AvailableUpdate,
    downloaded_bytes: u64,
    total_bytes: u64,
    ui: &mut egui::Ui,
    t: &Tokens,
) {
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

    draw_soft_strip(
        ui,
        t.accent_soft,
        Stroke::new(1.0, t.accent),
        Margin::symmetric(12, 10),
        |ui| {
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
        },
    );
}

fn render_update_ready_box(
    update: &AvailableUpdate,
    ui: &mut egui::Ui,
    t: &Tokens,
    actions: &mut UiActions,
) {
    draw_soft_strip(
        ui,
        t.success_bg,
        Stroke::new(1.0, t.success_border),
        Margin::symmetric(12, 10),
        |ui| {
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
        },
    );
}

pub fn draw_update_progress_bar(
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
