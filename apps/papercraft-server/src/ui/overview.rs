//! Server Overview dashboard: connection addresses, access & restrictions, and live activity.

use egui::{Margin, Sense, Stroke, Vec2};

use crate::controller::{ControlTab, ServerController, StatsSnapshot};
use crate::icons::{
    Icon, SwitchOption, icon_button, paint_icon, segmented_switch, toggle_switch_row,
};
use crate::settings::{NetworkScope, format_bytes};
use crate::theme::Tokens;
use crate::ui::TWO_COL_BREAKPOINT;
use crate::ui::widgets::{
    PageHeaderBadge, draw_compact_pill, draw_hairline, draw_metric_cell, draw_page_header,
    draw_section_header, draw_section_panel, draw_singleline_input, draw_soft_strip,
    fixed_secondary_button,
};

pub fn render_overview_page(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    let reg_open = ctrl.draft_settings.allow_registration;
    let scope_label = ctrl.draft_settings.network_scope.label();

    draw_page_header(
        ui,
        "Server Overview",
        None::<PageHeaderBadge<'_>>,
        "Monitor live sync activity, connection addresses, and access restrictions.",
        t,
        |ui| {
            draw_compact_pill(
                ui,
                Some(Icon::Globe),
                scope_label,
                t.toolbar_bg,
                t.border,
                t.text,
            );
            if reg_open {
                draw_compact_pill(
                    ui,
                    Some(Icon::LockOpen),
                    "New sign-ups open",
                    t.success_bg,
                    t.success_border,
                    t.success_fg,
                );
            } else {
                draw_compact_pill(
                    ui,
                    Some(Icon::Lock),
                    "New sign-ups closed",
                    t.warning_bg,
                    t.warning_border,
                    t.warning_fg,
                );
            }
        },
    );

    ui.add_space(14.0);

    let avail_w = ui.available_width();
    if avail_w >= TWO_COL_BREAKPOINT {
        let gap = 16.0_f32;
        let left_w = ((avail_w - gap) * 0.48).floor();
        let right_w = (avail_w - gap - left_w).max(200.0);

        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.allocate_ui_with_layout(
                Vec2::new(left_w, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(left_w);
                    ui.spacing_mut().item_spacing.y = 14.0;
                    render_connection_panel(ctrl, ui, t);
                    render_activity_panel(ctrl, ui, t);
                },
            );
            ui.allocate_ui_with_layout(
                Vec2::new(right_w, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(right_w);
                    ui.spacing_mut().item_spacing.y = 14.0;
                    render_access_and_restrictions_panel(ctrl, ui, t);
                },
            );
        });
    } else {
        ui.spacing_mut().item_spacing.y = 12.0;
        render_connection_panel(ctrl, ui, t);
        render_access_and_restrictions_panel(ctrl, ui, t);
        render_activity_panel(ctrl, ui, t);
    }
}

fn render_connection_panel(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    let urls = ctrl.connection_urls();
    let running = ctrl.is_running();

    draw_section_panel(ui, t, |ui| {
        let subtitle = if running {
            "Paste this address into PaperCraft -> Sign In -> Self-Hosted Server."
        } else {
            "Server address (start the server to connect)"
        };
        draw_section_header(ui, Icon::Link, "Connection Addresses", subtitle, t);
        ui.add_space(12.0);

        let local_label = if urls.local_is_loopback {
            "On this computer"
        } else {
            "Server address"
        };
        if render_url_copy_strip(
            ui,
            local_label,
            &urls.local_url,
            ctrl.was_recently_copied(&urls.local_url),
            t,
        ) {
            ctrl.copy_url(&urls.local_url);
        }

        if let Some(ref net_url) = urls.network_url {
            ui.add_space(8.0);
            if render_url_copy_strip(
                ui,
                "On other devices (Wi-Fi / VPN)",
                net_url,
                ctrl.was_recently_copied(net_url),
                t,
            ) {
                ctrl.copy_url(net_url);
            }
        } else if ctrl.draft_settings.network_scope == NetworkScope::LocalNetwork {
            ui.add_space(8.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "Other devices can connect using http://<this-PC-IP>:{}",
                        ctrl.draft_settings.port
                    ))
                    .size(11.5)
                    .color(t.text_muted),
                )
                .wrap(),
            );
        }
    });
}

fn render_url_copy_strip(
    ui: &mut egui::Ui,
    label: &str,
    url: &str,
    copied: bool,
    t: &Tokens,
) -> bool {
    let mut clicked = false;
    draw_soft_strip(
        ui,
        t.toolbar_bg,
        Stroke::NONE,
        Margin::symmetric(12, 9),
        |ui| {
            let (icon, btn_lbl) = if copied {
                (Icon::Check, "Copied")
            } else {
                (Icon::Copy, "Copy")
            };
            let btn_w = crate::icons::icon_button_width(ui, Some(btn_lbl));
            let left_w = (ui.available_width() - btn_w - 10.0).max(110.0);

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.allocate_ui_with_layout(
                    Vec2::new(left_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(left_w);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(egui::RichText::new(label).size(11.0).color(t.text_muted));
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(url)
                                    .size(12.5)
                                    .monospace()
                                    .strong()
                                    .color(t.text),
                            )
                            .wrap(),
                        );
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_button(ui, icon, Some(btn_lbl), copied, true, t)
                        .on_hover_text("Copy server URL to clipboard")
                        .clicked()
                    {
                        clicked = true;
                    }
                });
            });
        },
    );
    clicked
}

fn render_access_and_restrictions_panel(
    ctrl: &mut ServerController,
    ui: &mut egui::Ui,
    t: &Tokens,
) {
    draw_section_panel(ui, t, |ui| {
        draw_section_header(
            ui,
            Icon::Globe,
            "Access & Restrictions",
            "Control which devices can reach this server and whether new accounts can sign up.",
            t,
        );
        ui.add_space(12.0);

        render_network_access_section(ctrl, ui, t);

        ui.add_space(12.0);
        draw_hairline(ui, t);
        ui.add_space(12.0);

        render_registration_section(ctrl, ui, t);

        ui.add_space(12.0);
        draw_hairline(ui, t);
        ui.add_space(10.0);

        render_storage_shortcut_row(ctrl, ui, t);
    });
}

fn render_network_access_section(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    ui.label(
        egui::RichText::new("Who can connect")
            .size(12.5)
            .strong()
            .color(t.text),
    );
    ui.add_space(6.0);

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

    ui.add_space(5.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new(ctrl.draft_settings.network_scope.description())
                .size(11.5)
                .color(t.text_muted),
        )
        .wrap(),
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
        ui.add_space(8.0);
        draw_soft_strip(
            ui,
            t.toolbar_bg,
            Stroke::NONE,
            Margin::symmetric(10, 8),
            |ui| {
                ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                paint_icon(ui.painter(), ir, Icon::Info, t.accent);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!(
                            "Tip: if other devices cannot reach port {}, allow inbound TCP port {} in your firewall.",
                            ctrl.draft_settings.port, ctrl.draft_settings.port
                        ))
                        .size(11.0)
                        .color(t.text_muted),
                    )
                    .wrap(),
                );
            });
            },
        );
    }
}

fn render_registration_section(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    let open = ctrl.draft_settings.allow_registration;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Account Sign-Ups")
                .size(12.5)
                .strong()
                .color(t.text),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if open {
                draw_compact_pill(
                    ui,
                    Some(Icon::LockOpen),
                    "Open",
                    t.success_bg,
                    t.success_border,
                    t.success_fg,
                );
            } else {
                draw_compact_pill(
                    ui,
                    Some(Icon::Lock),
                    "Closed",
                    t.warning_bg,
                    t.warning_border,
                    t.warning_fg,
                );
            }
        });
    });

    ui.add_space(6.0);
    toggle_switch_row(
        ui,
        "bnc_srv_allow_registration",
        "Allow new account sign-ups",
        "Control whether PaperCraft clients can register new accounts on this server.",
        &mut ctrl.draft_settings.allow_registration,
        t,
    );

    ui.add_space(4.0);
    let explanation = if ctrl.draft_settings.allow_registration {
        "Anyone who can reach this server can register a new account. Turn this off after creating your accounts so no one else can sign up."
    } else {
        "New account sign-ups are closed. Existing accounts can still sign in and sync normally."
    };
    ui.add(
        egui::Label::new(
            egui::RichText::new(explanation)
                .size(11.5)
                .color(t.text_muted),
        )
        .wrap(),
    );
}

fn render_storage_shortcut_row(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    let display_path = {
        let full_path = ctrl.draft_settings.data_dir.trim();
        if full_path.is_empty() {
            "Default storage folder".to_string()
        } else {
            full_path.to_string()
        }
    };
    let open_w = 56.0_f32;
    let settings_w = crate::icons::icon_button_width(ui, Some("Settings"));
    let path_w = (ui.available_width() - 22.0 - open_w - settings_w - 18.0).max(60.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        paint_icon(ui.painter(), ir, Icon::FolderOpen, t.text_muted);

        ui.allocate_ui_with_layout(
            Vec2::new(path_w, 0.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_max_width(path_w);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&display_path)
                            .size(11.5)
                            .color(t.text_muted),
                    )
                    .truncate(),
                )
                .on_hover_text(&display_path);
            },
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if icon_button(ui, Icon::Settings, Some("Settings"), false, true, t)
                .on_hover_text("Open storage, limits, and startup settings")
                .clicked()
            {
                ctrl.set_active_tab(ControlTab::Settings);
            }
            if fixed_secondary_button(ui, "Open", open_w, true, t)
                .on_hover_text("Open the server data folder in your file manager")
                .clicked()
            {
                ctrl.open_data_folder();
            }
        });
    });
}

fn render_activity_panel(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    draw_section_panel(ui, t, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            paint_icon(ui.painter(), ir, Icon::Server, t.accent);
            ui.label(
                egui::RichText::new("Activity & Storage")
                    .size(14.0)
                    .strong()
                    .color(t.text),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, Icon::RotateCw, None, false, true, t)
                    .on_hover_text("Refresh database counts")
                    .clicked()
                {
                    ctrl.refresh_stats();
                    ctrl.refresh_users();
                }
            });
        });

        ui.add_space(10.0);
        if let Some(stats) = ctrl.stats_snapshot {
            render_live_stats_grid(stats, ui, t);
        } else {
            let active_count = ctrl.users_snapshot.len();
            let trash_count = ctrl.trashed_users_snapshot.len();
            let accounts_summary = if trash_count > 0 {
                format!("{active_count} active ({trash_count} in Trash)")
            } else {
                format!("{active_count} active")
            };
            draw_soft_strip(
                ui,
                t.toolbar_bg,
                Stroke::NONE,
                Margin::symmetric(12, 10),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.label(
                        egui::RichText::new(format!("Stored accounts: {accounts_summary}"))
                            .size(12.0)
                            .strong()
                            .color(t.text),
                    );
                    ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Start the server to view live synced notebooks, active device sessions, and database size.",
                        )
                        .size(11.5)
                        .color(t.text_muted),
                    )
                    .wrap(),
                );
                },
            );
        }

        ui.add_space(12.0);
        draw_hairline(ui, t);
        ui.add_space(10.0);

        render_local_discovery_section(ctrl, ui, t);
    });
}

fn render_live_stats_grid(stats: StatsSnapshot, ui: &mut egui::Ui, t: &Tokens) {
    let accounts_value = if stats.trashed_user_count > 0 {
        format!(
            "{} ({} in Trash)",
            stats.user_count, stats.trashed_user_count
        )
    } else {
        stats.user_count.to_string()
    };

    ui.columns(2, |cols| {
        cols[0].spacing_mut().item_spacing.y = 10.0;
        cols[1].spacing_mut().item_spacing.y = 10.0;
        draw_metric_cell(
            &mut cols[0],
            Some(Icon::Account),
            "Accounts",
            &accounts_value,
            15.5,
            t,
        );
        draw_metric_cell(
            &mut cols[1],
            Some(Icon::Notebook),
            "Notebooks",
            &stats.active_notebook_count.to_string(),
            15.5,
            t,
        );
        draw_metric_cell(
            &mut cols[0],
            None,
            "Active devices",
            &stats.active_session_count.to_string(),
            15.5,
            t,
        );
        draw_metric_cell(
            &mut cols[1],
            None,
            "Database size",
            &format_bytes(stats.db_size_bytes),
            15.5,
            t,
        );
    });
}

fn render_local_discovery_section(ctrl: &ServerController, ui: &mut egui::Ui, t: &Tokens) {
    let counts = ctrl.local_discovery;
    ui.label(
        egui::RichText::new("On-device discovery")
            .size(12.0)
            .strong()
            .color(t.text),
    );
    ui.add_space(6.0);

    ui.columns(2, |cols| {
        draw_metric_cell(
            &mut cols[0],
            Some(Icon::Account),
            "Unsynced local accounts",
            &counts.unsynced_local_accounts.to_string(),
            14.5,
            t,
        );
        draw_metric_cell(
            &mut cols[1],
            Some(Icon::Notebook),
            "Local notebooks",
            &counts.total_local_notebooks.to_string(),
            14.5,
            t,
        );
    });

    ui.add_space(6.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new("Counts only (sync prompts stay hidden until signed in)")
                .size(11.0)
                .color(t.text_muted),
        )
        .wrap(),
    );
}
