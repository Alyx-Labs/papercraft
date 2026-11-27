//! Dedicated full-window Profile, Account Switching, Sync, Identity & Backup screen.

use egui::{CornerRadius, Margin, Sense, Stroke, Vec2};
use papercraft_sync::{SyncConnectionState, SyncStatusSummary};

use crate::PaperCraftApp;
use crate::account_modal::{
    AuthTab, BannerTone, ProfileSection, draw_account_type_badge, draw_hairline, draw_section_card,
    draw_status_banner, draw_sync_connection_pill, format_notebook_count,
    handle_auth_submit_on_enter, probe_all_known_servers, render_account_email_card,
    render_accounts_switcher_section, render_auth_banners, render_auth_card_contents,
    render_change_password_card, render_expired_session_reauth_box,
    render_local_profile_identity_card, render_profile_identity_card, render_server_probe_section,
    render_signed_in_account_card, render_signed_in_server_card,
    render_unsynced_switch_confirmation, status_badge_style, summary_account_label,
    trigger_test_connection,
};
use crate::icons::{
    Icon, SidebarEdge, SidebarEdgeButtonSpec, SidebarEdgeSize, SwitchLayout, SwitchOption,
    TOOLBAR_CONTROL_HEIGHT, paint_icon, segmented_switch, sidebar_edge_button_sized,
    toolbar_icon_button,
};
use crate::modal::{danger_action_button, primary_action_button, secondary_action_button};
use crate::sync_worker::{complete_sign_out, trigger_manual_sync};
use crate::theme::Tokens;

const HEADER_ROW_HEIGHT: f32 = TOOLBAR_CONTROL_HEIGHT;
const HEADER_BADGE_SIZE: f32 = 28.0;
const MAX_CONTENT_WIDTH: f32 = 1080.0;

/// Renders the dedicated Profile & Cloud Sync screen.
pub fn show(app: &mut PaperCraftApp, ui: &mut egui::Ui) {
    if std::mem::take(&mut app.view.account.profile_probe_pending) {
        probe_all_known_servers(app, ui.ctx());
    }

    let t = Tokens::for_theme(app.session.prefs.theme);
    let summary = app.session.sync_status_summary();
    let cloud_pending = app.session.cloud_pending_changes_count();

    draw_profile_header(app, ui, &summary, cloud_pending, &t);
    crate::inspector::show(app, ui);

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(t.panel_bg))
        .show(ui, |ui| {
            let avail = ui.available_rect_before_wrap();
            let side_pad = if avail.width() < 420.0 { 12.0 } else { 24.0 };
            let content_w = (avail.width() - side_pad * 2.0)
                .clamp(160.0, MAX_CONTENT_WIDTH)
                .min(avail.width().max(1.0));
            let left = avail.left() + ((avail.width() - content_w) * 0.5).max(side_pad);
            let inner_rect = egui::Rect::from_min_max(
                egui::pos2(left, avail.top() + 18.0),
                egui::pos2(
                    left + content_w,
                    (avail.bottom() - 16.0).max(avail.top() + 60.0),
                ),
            );

            ui.scope_builder(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
                ui.set_width(content_w);
                draw_profile_workspace(app, ui, &summary, cloud_pending, content_w, &t);
            });
        });
}

fn draw_profile_workspace(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    cloud_pending: usize,
    content_w: f32,
    t: &Tokens,
) {
    ui.spacing_mut().item_spacing.y = 0.0;
    draw_active_account_summary_card(app, ui, summary, cloud_pending, t);
    ui.add_space(12.0);
    draw_profile_tab_bar(app, ui, t);
    ui.add_space(12.0);

    egui::ScrollArea::vertical()
        .id_salt("papercraft_profile_body_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(ui.available_width().min(content_w));
            draw_active_section_content(app, ui, summary, cloud_pending, t);
            ui.add_space(16.0);
        });
}

fn draw_profile_header(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    cloud_pending: usize,
    t: &Tokens,
) {
    let frame = egui::Frame::new()
        .fill(t.header_bg)
        .inner_margin(Margin {
            left: 12,
            right: 0,
            top: 8,
            bottom: 8,
        })
        .stroke(Stroke::new(1.0, t.header_border));

    egui::Panel::top("papercraft_profile_header")
        .frame(frame)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(HEADER_ROW_HEIGHT);
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);

                let (badge_rect, _) =
                    ui.allocate_exact_size(Vec2::splat(HEADER_BADGE_SIZE), Sense::hover());
                ui.painter()
                    .rect_filled(badge_rect, CornerRadius::same(7), t.header_active_bg);
                paint_icon(
                    ui.painter(),
                    badge_rect.shrink(6.0),
                    Icon::Account,
                    t.header_active_fg,
                );

                ui.label(
                    egui::RichText::new("Profile & Cloud Sync")
                        .size(14.5)
                        .strong()
                        .color(t.header_fg),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.set_height(HEADER_ROW_HEIGHT);
                    ui.spacing_mut().item_spacing.x = 6.0;

                    if sidebar_edge_button_sized(
                        ui,
                        SidebarEdgeButtonSpec {
                            edge: SidebarEdge::Right,
                            icon: Icon::Settings,
                            label: "",
                            max_width: None,
                            open: app.view.inspector_open,
                            enabled: true,
                        },
                        SidebarEdgeSize::Toolbar,
                        t,
                    )
                    .on_hover_text(
                        "Settings (Shortcut: S)\nStudio theme, app scale, and pen presets.",
                    )
                    .clicked()
                    {
                        if !app.view.inspector_open {
                            app.view.settings_tab = crate::SettingsTab::Appearance;
                        }
                        app.view.inspector_open = !app.view.inspector_open;
                    }

                    let in_flight = app.session.is_sync_in_flight();
                    let (state_label, _) = status_badge_style(summary.connection, t);
                    let dot_color = if summary.viewing_cloud {
                        crate::account_modal::header_status_dot_color(summary.connection, t)
                    } else if app
                        .view
                        .account
                        .probe_for(app.session.preferred_server_url())
                        .is_connected()
                    {
                        t.success_fg
                    } else {
                        t.header_fg_muted
                    };
                    let sync_tip = if summary.viewing_cloud {
                        if in_flight {
                            format!("Syncing with server...\nServer: {}", summary.server_url)
                        } else if summary.connection == SyncConnectionState::Offline {
                            format!(
                                "Offline — changes saved locally ({cloud_pending} queued for reconnect)\nServer: {}",
                                summary.server_url
                            )
                        } else {
                            format!(
                                "Sync Now ({state_label}, {cloud_pending} pending)\nServer: {}",
                                summary.server_url
                            )
                        }
                    } else {
                        format!(
                            "Local Account Active (Saved on this device)\nClick to check sync server status ({}).",
                            app.session.preferred_server_url()
                        )
                    };
                    let sync_resp =
                        toolbar_icon_button(ui, Icon::Sync, None, in_flight, !in_flight, t);
                    if ui.is_rect_visible(sync_resp.rect) {
                        let dot_c =
                            egui::pos2(sync_resp.rect.right() - 6.5, sync_resp.rect.bottom() - 6.5);
                        ui.painter().circle_filled(dot_c, 4.2, t.header_bg);
                        ui.painter().circle_filled(dot_c, 3.0, dot_color);
                    }
                    if sync_resp.on_hover_text(sync_tip).clicked() {
                        app.view.account.error_message = None;
                        app.view.account.status_message = None;
                        if summary.viewing_cloud {
                            trigger_manual_sync(app, ui.ctx());
                        } else {
                            probe_all_known_servers(app, ui.ctx());
                        }
                    }
                });
            });
        });
}

fn draw_active_account_summary_card(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    cloud_pending: usize,
    t: &Tokens,
) {
    let identity_key = if summary.signed_in {
        format!(
            "cloud:{}:{}:{}:{}",
            summary.server_url,
            summary.username.as_deref().unwrap_or(""),
            summary.display_name.as_deref().unwrap_or(""),
            summary.email.as_deref().unwrap_or("")
        )
    } else {
        format!(
            "local:{}:{}",
            summary.active_local_profile_id.as_deref().unwrap_or(""),
            summary.active_local_profile_name.as_deref().unwrap_or("")
        )
    };
    let dt = ui.ctx().input(|i| i.stable_dt);
    if app.view.motion.profile_identity.advance(identity_key, dt) {
        ui.ctx().request_repaint();
    }
    let identity_opacity = app.view.motion.profile_identity.opacity();
    let (primary, handle_or_mode, monogram) = resolve_summary_identity(summary);

    egui::Frame::new()
        .fill(t.toolbar_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.multiply_opacity(identity_opacity);

            draw_summary_card_header_row(
                app,
                ui,
                summary,
                (&primary, &handle_or_mode, &monogram),
                t,
            );
            draw_summary_card_meta_row(app, ui, summary, cloud_pending, t);

            if summary.signed_in
                && app.session.is_session_expired()
                && let Some(acct) = app.session.active_account_session().cloned()
            {
                ui.add_space(4.0);
                render_expired_session_reauth_box(
                    app,
                    ui,
                    &acct,
                    cloud_pending,
                    "summary_reauth_pw",
                    t,
                );
            }
        });
}

fn resolve_summary_identity(summary: &SyncStatusSummary) -> (String, String, String) {
    if summary.signed_in {
        let disp = summary
            .display_name
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(summary.username.as_deref())
            .unwrap_or("Account")
            .to_string();
        let email_or_handle = match (
            summary.email.as_deref().filter(|s| !s.trim().is_empty()),
            summary.username.as_deref().filter(|s| !s.trim().is_empty()),
        ) {
            (Some(e), Some(u)) => format!("{e} (@{u})"),
            (Some(e), None) => e.to_string(),
            (None, Some(u)) => format!("@{u}"),
            (None, None) => "Server Account".to_string(),
        };
        let mono = disp
            .chars()
            .find(|c| c.is_alphanumeric())
            .map_or_else(|| "U".to_string(), |c| c.to_uppercase().collect());
        (disp, email_or_handle, mono)
    } else {
        let nick = summary
            .active_local_profile_name
            .clone()
            .unwrap_or_else(|| "Personal".to_string());
        let mono = nick
            .chars()
            .find(|c| c.is_alphanumeric())
            .map_or_else(|| "L".to_string(), |c| c.to_uppercase().collect());
        (nick, "Offline workspace on this device".to_string(), mono)
    }
}

fn draw_summary_card_header_row(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    identity: (&str, &str, &str),
    t: &Tokens,
) {
    let (primary, subtitle, monogram) = identity;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(10.0, 6.0);
        let (avatar_rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
        ui.painter()
            .circle_filled(avatar_rect.center(), 20.0, t.accent_soft);
        ui.painter().circle_stroke(
            avatar_rect.center(),
            20.0,
            Stroke::new(1.2, t.accent.gamma_multiply(0.45)),
        );
        ui.painter().text(
            avatar_rect.center(),
            egui::Align2::CENTER_CENTER,
            monogram,
            egui::FontId::proportional(16.0),
            t.accent,
        );

        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(
                    egui::RichText::new(primary)
                        .size(14.5)
                        .strong()
                        .color(t.text),
                );
                draw_account_type_badge(ui, summary.signed_in, t);
                if summary.signed_in {
                    draw_sync_connection_pill(ui, summary.connection, t);
                }
            });
            ui.label(egui::RichText::new(subtitle).size(11.5).color(t.text_muted));
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if summary.signed_in {
                if danger_action_button(ui, "Sign Out", true, t).clicked() {
                    complete_sign_out(app, ui.ctx());
                }
                if secondary_action_button(ui, "Switch Account", true, t).clicked() {
                    crate::modal::open(app, crate::modal::ActiveModal::SelectLocalAccount);
                }
                let in_flight = app.session.is_sync_in_flight();
                let can_sync = !in_flight && app.session.is_viewing_cloud();
                let sync_lbl = if in_flight { "Syncing..." } else { "Sync Now" };
                if primary_action_button(ui, sync_lbl, can_sync, t).clicked() {
                    app.view.account.error_message = None;
                    app.view.account.status_message = None;
                    trigger_manual_sync(app, ui.ctx());
                }
            } else {
                if secondary_action_button(ui, "Switch Account", true, t).clicked() {
                    crate::modal::open(app, crate::modal::ActiveModal::SelectLocalAccount);
                }
                if primary_action_button(ui, "Connect to Server", true, t).clicked() {
                    app.view.account.profile_section = ProfileSection::ServerSync;
                    app.view.account.tab = AuthTab::SignIn;
                    app.view.account.profile_probe_pending = true;
                    app.view.account.error_message = None;
                }
            }
        });
    });
}

fn draw_summary_card_meta_row(
    app: &PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    cloud_pending: usize,
    t: &Tokens,
) {
    let nb_label = format_notebook_count(app.session.notebooks.len());
    let detail = if summary.signed_in {
        let queue_txt = match cloud_pending {
            0 => "all changes synced".to_string(),
            1 => "1 pending sync".to_string(),
            n => format!("{n} pending syncs"),
        };
        format!(
            "Server: {} · Device: {} · {nb_label} · {queue_txt}",
            summary.server_url, summary.device_name
        )
    } else {
        format!(
            "{nb_label} in this local workspace · Stored only on this device unless synced via Server Sync"
        )
    };
    ui.add(egui::Label::new(egui::RichText::new(detail).size(11.0).color(t.text_muted)).wrap());
}

fn draw_profile_tab_bar(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let options = [
        SwitchOption::new(
            Some(Icon::Inbox),
            Some("Accounts"),
            "Switch, create, rename, delete, or forget accounts",
            true,
        ),
        SwitchOption::new(
            Some(Icon::Server),
            Some("Server Sync"),
            "Configure server URL, inspect capabilities, and sign in or switch accounts",
            true,
        ),
        SwitchOption::new(
            Some(Icon::Account),
            Some("Profile"),
            "Edit account name, username, email, and password",
            true,
        ),
        SwitchOption::new(
            Some(Icon::Export),
            Some("Backup"),
            "Export or import .pcraftpkg archives and inspect device storage",
            true,
        ),
    ];
    let active = match app.view.account.profile_section {
        ProfileSection::Accounts => 0,
        ProfileSection::ServerSync => 1,
        ProfileSection::Profile => 2,
        ProfileSection::Backup => 3,
    };
    let (clicked, _) = segmented_switch(
        ui,
        "profile_main_tabs",
        &options,
        Some(active),
        SwitchLayout::EqualWidth,
        t,
    );
    if let Some(idx) = clicked {
        app.view.account.profile_section = match idx {
            0 => ProfileSection::Accounts,
            1 => ProfileSection::ServerSync,
            2 => ProfileSection::Profile,
            _ => ProfileSection::Backup,
        };
    }
}

fn draw_active_section_content(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    cloud_pending: usize,
    t: &Tokens,
) {
    ui.spacing_mut().item_spacing.y = 0.0;

    let section_order: isize = match app.view.account.profile_section {
        ProfileSection::Accounts => 0,
        ProfileSection::ServerSync => 1,
        ProfileSection::Profile => 2,
        ProfileSection::Backup => 3,
    };
    let section_key = (summary.signed_in, section_order as u8);
    let dt = ui.ctx().input(|i| i.stable_dt);
    if app
        .view
        .motion
        .profile_section
        .advance(section_key, section_order, dt)
    {
        ui.ctx().request_repaint();
    }
    let flow_opacity = app.view.motion.profile_section.opacity();
    let flow_offset_x = app.view.motion.profile_section.offset_x();

    crate::motion::show_flow_content(ui, flow_opacity, flow_offset_x, |ui| {
        match app.view.account.profile_section {
            ProfileSection::Accounts => {
                render_accounts_switcher_section(app, ui, summary, cloud_pending, t);
            }
            ProfileSection::ServerSync => {
                draw_server_sync_tab(app, ui, summary, cloud_pending, t);
            }
            ProfileSection::Profile => {
                draw_profile_tab(app, ui, summary, t);
            }
            ProfileSection::Backup => {
                draw_backup_tab(app, ui, t);
            }
        }
    });

    if let Some(ref err) = summary.last_error
        && app.view.account.error_message.is_none()
        && summary.connection == SyncConnectionState::Error
    {
        ui.add_space(8.0);
        draw_status_banner(ui, BannerTone::Error, err, t);
    }
    render_auth_banners(app, ui, t);
}

fn draw_server_sync_tab(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    cloud_pending: usize,
    t: &Tokens,
) {
    if summary.signed_in {
        let mut test_on_enter = false;
        draw_section_card(ui, t, |ui| {
            test_on_enter = render_signed_in_server_card(app, ui, summary, t);
        });
        if test_on_enter && !app.view.account.is_testing() {
            trigger_test_connection(app, ui.ctx());
        }

        ui.add_space(10.0);
        draw_section_card(ui, t, |ui| {
            render_signed_in_account_card(app, ui, summary, cloud_pending, t);
        });

        ui.add_space(10.0);
        draw_sign_in_another_account_card(app, ui, summary, t);

        ui.add_space(10.0);
        draw_self_hosted_download_disclaimer(app, ui, t);
    } else {
        let mut test_on_enter = false;
        draw_section_card(ui, t, |ui| {
            test_on_enter = render_server_probe_section(app, ui, t, true);
        });
        ui.add_space(10.0);
        let mut submit_on_enter = false;
        draw_section_card(ui, t, |ui| {
            submit_on_enter = render_auth_card_contents(app, ui, t, true);
        });
        render_unsynced_switch_confirmation(app, ui, t);
        ui.add_space(10.0);
        draw_self_hosted_download_disclaimer(app, ui, t);
        if test_on_enter && !app.view.account.is_testing() {
            trigger_test_connection(app, ui.ctx());
        }
        if submit_on_enter {
            handle_auth_submit_on_enter(app, ui.ctx());
        }
    }
}

fn draw_sign_in_another_account_card(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    t: &Tokens,
) {
    let open = app.view.account.server_switch_form_open || app.view.account.password_reset.active;
    let mut submit_on_enter = false;
    draw_section_card(ui, t, |ui| {
        ui.horizontal(|ui| {
            let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            paint_icon(ui.painter(), ir, Icon::Account, t.accent);
            ui.label(
                egui::RichText::new("Sign In with Another Server Account")
                    .size(13.5)
                    .strong()
                    .color(t.text),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let lbl = if open {
                    "Hide Form"
                } else {
                    "Switch / Sign In Another"
                };
                if crate::account_modal::fixed_secondary_button(ui, lbl, 160.0, true, t).clicked() {
                    if open {
                        app.view.account.server_switch_form_open = false;
                        app.view.account.clear_pending_unsynced_switch();
                        app.view.account.exit_password_reset_flow();
                    } else {
                        app.view.account.server_switch_form_open = true;
                        app.view.account.tab = AuthTab::SignIn;
                        app.view.account.clear_auth_form_credentials();
                    }
                }
            });
        });
        ui.add_space(3.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!(
                    "Currently signed in as {}. Enter credentials for a different account on {} (or test another server URL above) to switch safely while preserving this account's isolated local cache.",
                    summary_account_label(summary),
                    app.view.account.server_url
                ))
                .size(11.0)
                .color(t.text_muted),
            )
            .wrap(),
        );

        if open {
            ui.add_space(10.0);
            draw_hairline(ui, t);
            ui.add_space(6.0);
            render_unsynced_switch_confirmation(app, ui, t);
            ui.add_space(6.0);
            if render_auth_card_contents(app, ui, t, true) {
                submit_on_enter = true;
            }
        }
    });

    if submit_on_enter {
        handle_auth_submit_on_enter(app, ui.ctx());
    }
}

fn draw_profile_tab(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    t: &Tokens,
) {
    if summary.signed_in {
        draw_section_card(ui, t, |ui| {
            render_profile_identity_card(app, ui, t);
        });
        ui.add_space(10.0);
        draw_section_card(ui, t, |ui| {
            render_account_email_card(app, ui, summary, t);
        });
        ui.add_space(10.0);
        draw_section_card(ui, t, |ui| {
            render_change_password_card(app, ui, t);
        });
    } else {
        draw_section_card(ui, t, |ui| {
            render_local_profile_identity_card(app, ui, t);
        });
        ui.add_space(10.0);
        draw_section_card(ui, t, |ui| {
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                paint_icon(ui.painter(), ir, Icon::Reset, t.accent);
                ui.label(
                    egui::RichText::new("Server Email & Password")
                        .size(13.5)
                        .strong()
                        .color(t.text),
                );
            });
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "Local offline accounts do not require an email or password. Sign in on the Server Sync tab or upload this local account to manage cloud credentials.",
                    )
                    .size(11.0)
                    .color(t.text_muted),
                )
                .wrap(),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if secondary_action_button(ui, "Open Server Sync", true, t).clicked() {
                    app.view.account.profile_section = ProfileSection::ServerSync;
                    app.view.account.tab = AuthTab::SignIn;
                    app.view.account.profile_probe_pending = true;
                }
            });
        });
    }
}

fn draw_backup_tab(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    draw_section_card(ui, t, |ui| {
        ui.horizontal(|ui| {
            let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            paint_icon(ui.painter(), ir, Icon::Export, t.accent);
            ui.label(
                egui::RichText::new("Workspace Backup (.pcraftpkg)")
                    .size(13.5)
                    .strong()
                    .color(t.text),
            );
        });
        ui.add_space(3.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "Create a single portable backup archive containing all notebooks, Trash items, folders, and preferences from the active account, or restore a .pcraftpkg archive.",
                )
                .size(11.0)
                .color(t.text_muted),
            )
            .wrap(),
        );
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
            if primary_action_button(ui, "Export Backup (.pcraftpkg)", true, t).clicked() {
                app.trigger_export_package_dialog();
            }
            if secondary_action_button(ui, "Import Backup (.pcraftpkg)", true, t).clicked() {
                app.trigger_import_package_dialog();
            }
        });
        if let Some(ref status) = app.session.last_export_status {
            ui.add_space(8.0);
            draw_status_banner(ui, BannerTone::Info, status, t);
        }
    });

    ui.add_space(10.0);
    draw_storage_overview_card(app, ui, t);
}

fn draw_storage_overview_card(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let active_notebooks = format_notebook_count(app.session.notebooks.len());
    let trash_count = format_notebook_count(app.session.trashed_notebooks.len());
    let local_count = app.session.saved_local_profiles().len();
    let server_count = app.session.remembered_server_accounts().len();
    let desktop_dir = app.session.library_dir().map(|p| p.display().to_string());
    let web_status = app.web_storage_status.clone();

    draw_section_card(ui, t, |ui| {
        ui.horizontal(|ui| {
            let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            paint_icon(ui.painter(), ir, Icon::Library, t.accent);
            ui.label(
                egui::RichText::new("Device & Browser Storage")
                    .size(13.5)
                    .strong()
                    .color(t.text),
            );
        });
        ui.add_space(3.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "Each account on this device keeps its own isolated workspace storage so switching accounts never mixes notebooks.",
                )
                .size(11.0)
                .color(t.text_muted),
            )
            .wrap(),
        );
        ui.add_space(8.0);
        draw_hairline(ui, t);
        ui.add_space(8.0);

        ui.label(
            egui::RichText::new(format!(
                "Active workspace: {active_notebooks} · Trash: {trash_count} · Saved accounts: {local_count} local, {server_count} server"
            ))
            .size(11.5)
            .color(t.text),
        );
        ui.add_space(6.0);

        if let Some(status) = web_status {
            let (info_text, tip) = crate::inspector::web_storage_info_from_status(&status);
            let resp = ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("Browser storage: {info_text}"))
                        .size(11.5)
                        .color(t.text_muted),
                )
                .wrap(),
            );
            if let Some(tip) = tip {
                let _ = resp.on_hover_text(tip);
            }
            if let Some((usage_text, _)) = crate::inspector::web_storage_usage_row(&status) {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!("Usage & Quota: {usage_text}"))
                        .size(11.0)
                        .color(t.text_muted),
                );
                if let (Some(used), Some(quota)) = (status.usage_bytes, status.quota_bytes)
                    && quota > 0
                {
                    ui.add_space(4.0);
                    let frac = (used as f32 / quota as f32).clamp(0.02, 1.0);
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .desired_width(ui.available_width())
                            .desired_height(6.0)
                            .fill(t.accent),
                    );
                }
            }
            if status.has_unsaved_error {
                ui.add_space(8.0);
                if let Some(ref err) = status.last_error {
                    draw_status_banner(ui, BannerTone::Error, err, t);
                    ui.add_space(6.0);
                }
                if app.hooks.retry_web_storage_write.is_some()
                    && primary_action_button(ui, "Retry Save Now", true, t).clicked()
                    && let Some(retry) = app.hooks.retry_web_storage_write.as_mut()
                {
                    retry();
                }
            }
        } else if let Some(dir) = desktop_dir {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("Library directory: {dir}"))
                        .size(11.0)
                        .monospace()
                        .color(t.text_muted),
                )
                .wrap(),
            );
        } else {
            ui.label(
                egui::RichText::new(
                    "Storage mode: In-memory session (changes are kept until the app closes unless exported).",
                )
                .size(11.0)
                .color(t.text_muted),
            );
        }
    });
}

fn draw_self_hosted_download_disclaimer(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let mut open_github = false;
    let frame_resp = egui::Frame::new()
        .fill(t.toolbar_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin {
            left: 16,
            right: 12,
            top: 9,
            bottom: 9,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                paint_icon(
                    ui.painter(),
                    icon_rect.translate(Vec2::new(0.0, 1.5)),
                    Icon::Info,
                    t.text_muted,
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(
                                "Note: Connecting to a local server requires downloading and running PaperCraft Server. If you want the standalone local desktop app as well, download it from GitHub.",
                            )
                            .size(11.0)
                            .color(t.text_muted),
                        )
                        .wrap(),
                    );
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        let (link_icon_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                        paint_icon(ui.painter(), link_icon_rect, Icon::Link, t.text_muted);
                        let link_resp = ui
                            .add(
                                egui::Label::new(
                                    egui::RichText::new("github.com/Alyx-Labs/papercraft")
                                        .size(11.0)
                                        .color(t.accent),
                                )
                                .sense(Sense::click()),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("Open GitHub to download PaperCraft Server or Desktop");
                        if link_resp.hovered() {
                            ui.painter().hline(
                                link_resp.rect.x_range(),
                                link_resp.rect.bottom() - 1.0,
                                Stroke::new(1.0, t.accent),
                            );
                        }
                        if link_resp.clicked() {
                            open_github = true;
                        }
                    });
                });
            });
        });

    let bar_rect = egui::Rect::from_min_max(
        egui::pos2(
            frame_resp.response.rect.left() + 6.0,
            frame_resp.response.rect.top() + 8.0,
        ),
        egui::pos2(
            frame_resp.response.rect.left() + 8.5,
            frame_resp.response.rect.bottom() - 8.0,
        ),
    );
    if bar_rect.height() > 2.0 {
        ui.painter().rect_filled(
            bar_rect,
            CornerRadius::same(2),
            t.text_muted.gamma_multiply(0.45),
        );
    }

    if open_github {
        let _ = crate::open_external_url(app, ui.ctx(), crate::GITHUB_REPO_URL);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::AppScreen;
    use crate::account_modal::AuthTab;
    use crate::sync_worker::{SyncWorkerEvent, apply_worker_event};
    use egui::{Shape, epaint::ClippedShape};
    use papercraft_sync::ActiveAccountSession;
    use papercraft_sync_protocol::{DEFAULT_SERVER_URL, SyncPullResponse, UserProfile};

    fn collect_text(shapes: &[ClippedShape]) -> String {
        let mut out = String::new();
        fn visit(shape: &Shape, buf: &mut String) {
            match shape {
                Shape::Text(text) => {
                    buf.push_str(text.galley.text());
                    buf.push('\n');
                }
                Shape::Vec(items) => {
                    for item in items {
                        visit(item, buf);
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

    fn render_profile_text(ctx: &egui::Context, app: &mut PaperCraftApp) -> String {
        let mut text = String::new();
        for step in 0..4 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(f64::from(step) * 0.05),
                predicted_dt: 0.05,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                show(app, ui);
            });
            text = collect_text(&out.shapes);
            out.drop_without_applying_deltas();
        }
        text
    }

    #[test]
    fn signed_out_profile_screen_transitions_into_password_reset_flow() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        crate::open_profile_screen(&mut app);
        assert_eq!(app.view.screen, AppScreen::Profile);
        assert_eq!(app.view.account.profile_section, ProfileSection::Accounts);

        app.view.account.profile_section = ProfileSection::ServerSync;
        app.view.account.tab = AuthTab::CreateAccount;
        let signup_text = render_profile_text(&ctx, &mut app);
        assert!(signup_text.contains("Profile & Cloud Sync"));
        assert!(signup_text.contains("Username"));
        assert!(signup_text.contains("Display Name"));

        app.view.account.tab = AuthTab::SignIn;
        let session_gen = app.sync_worker.current_session_generation();
        apply_worker_event(
            &mut app,
            &ctx,
            SyncWorkerEvent::PasswordResetRequired {
                session_generation: session_gen,
                identifier: "maria@example.com".to_string(),
                prefill_code: Some("ABCD-EFGH-JKLM".to_string()),
                message: "Temporary password verified. Choose a new password.".to_string(),
            },
        );
        assert!(app.view.account.password_reset.active);
        assert_eq!(
            app.view.account.password_reset.recovery_code,
            "ABCD-EFGH-JKLM"
        );
        let reset_text = render_profile_text(&ctx, &mut app);
        assert!(reset_text.contains("Complete Password Reset"));
        assert!(reset_text.contains("Temporary Recovery Code"));
        assert!(reset_text.contains("Set New Password & Sign In"));
    }

    #[test]
    fn signed_in_profile_screen_renders_all_sections_in_light_and_dark() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        let _ = app
            .session
            .execute("profile.create", &serde_json::json!({ "nickname": "Ana" }));
        let acct = ActiveAccountSession {
            server_url: DEFAULT_SERVER_URL.to_string(),
            user: UserProfile {
                user_id: "u-prof-1".to_string(),
                email: Some("alex@example.com".to_string()),
                username: "alex_notes".to_string(),
                display_name: "Alex Silva".to_string(),
                created_at_ms: 1000,
                must_change_password: false,
            },
            access_token: "acc".to_string(),
            refresh_token: "ref".to_string(),
            access_expires_at_ms: 99_999,
            refresh_expires_at_ms: 99_999,
            device_name: "Desk".to_string(),
            last_auth_ms: 1000,
        };
        let empty_pull = SyncPullResponse {
            server_cursor: 0,
            user: acct.user.clone(),
            notebooks: Vec::new(),
            folders: Vec::new(),
            studio_prefs: None,
        };
        app.session.enter_online_server_session(acct, empty_pull);
        crate::open_profile_screen(&mut app);
        assert_eq!(app.view.account.profile_section, ProfileSection::Accounts);

        let accts_text = render_profile_text(&ctx, &mut app);
        assert!(accts_text.contains("Switch Account"), "{accts_text}");
        assert!(accts_text.contains("Connected to Server"), "{accts_text}");
        assert!(accts_text.contains("Local Accounts (1)"), "{accts_text}");
        assert!(
            accts_text.contains("Server Accounts (Offline-First)"),
            "{accts_text}"
        );
        assert!(accts_text.contains("Ana"));
        assert!(accts_text.contains("0 notebooks"));

        app.view.account.profile_section = ProfileSection::ServerSync;
        let sync_text = render_profile_text(&ctx, &mut app);
        assert!(
            sync_text.contains("Server Address & Connection"),
            "{sync_text}"
        );
        assert!(sync_text.contains("SYNC QUEUE"), "{sync_text}");
        assert!(
            sync_text.contains("Sign In with Another Server Account"),
            "{sync_text}"
        );
        app.view.account.server_switch_form_open = true;
        let switch_form_text = render_profile_text(&ctx, &mut app);
        assert!(
            switch_form_text.contains("Create Account") && switch_form_text.contains("Email"),
            "{switch_form_text}"
        );

        app.view.account.profile_section = ProfileSection::Profile;
        let profile_text = render_profile_text(&ctx, &mut app);
        assert!(profile_text.contains("Profile Details"));
        assert!(profile_text.contains("Save Profile"));
        assert!(profile_text.contains("Email Address"));
        assert!(profile_text.contains("Change Password"));
        assert!(profile_text.contains("@alex_notes"));

        app.view.account.profile_section = ProfileSection::Backup;
        let _ = app
            .session
            .execute("theme.set", &serde_json::json!({ "theme": "Dark" }));
        let backup_text = render_profile_text(&ctx, &mut app);
        assert!(
            backup_text.contains("Workspace Backup (.pcraftpkg)"),
            "{backup_text}"
        );
        assert!(
            backup_text.contains("Export Backup (.pcraftpkg)"),
            "{backup_text}"
        );
        assert!(
            backup_text.contains("Device & Browser Storage"),
            "{backup_text}"
        );

        crate::close_profile_screen(&mut app);
        assert_eq!(app.view.screen, AppScreen::Home);
    }

    #[test]
    fn signed_out_profile_exposes_all_four_tabs_in_light_and_dark() {
        let ctx = egui::Context::default();
        let mut session = papercraft_engine::Session::open_web_cache(None);
        session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "Mia" }),
            )
            .unwrap();
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        app.view.active_modal = None;
        app.web_storage_status = Some(crate::WebStorageStatus {
            backend: crate::WebStorageBackend::IndexedDb,
            persisted: Some(true),
            usage_bytes: Some(2 * 1024 * 1024),
            quota_bytes: Some(128 * 1024 * 1024),
            in_flight: false,
            has_unsaved_error: false,
            quota_exceeded: false,
            last_error: None,
            other_tab_active: false,
            other_tab_wrote: false,
        });
        crate::open_profile_screen(&mut app);
        let acct_text = render_profile_text(&ctx, &mut app);
        assert!(acct_text.contains("Switch Account"), "{acct_text}");
        assert!(acct_text.contains("Connect to Server"), "{acct_text}");
        assert!(acct_text.contains("Mia"), "{acct_text}");
        assert!(acct_text.contains("Backup"), "{acct_text}");

        app.view.account.profile_section = ProfileSection::Profile;
        let prof_text = render_profile_text(&ctx, &mut app);
        assert!(prof_text.contains("Local Account Profile"), "{prof_text}");
        assert!(prof_text.contains("Upload to Server"), "{prof_text}");
        assert!(prof_text.contains("Server Email & Password"), "{prof_text}");

        app.view.account.profile_section = ProfileSection::Backup;
        let _ = app
            .session
            .execute("theme.set", &serde_json::json!({ "theme": "Dark" }));
        let backup_text = render_profile_text(&ctx, &mut app);
        assert!(backup_text.contains("IndexedDB"), "{backup_text}");
        assert!(backup_text.contains("Usage & Quota"), "{backup_text}");
    }

    #[test]
    fn signed_out_cloud_account_disclaimer_renders_and_opens_github_url() {
        use std::sync::{Arc, Mutex};

        let opened: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let opened_hook = Arc::clone(&opened);
        let hooks = crate::PlatformHooks {
            open_url: Some(Box::new(move |url| {
                opened_hook.lock().unwrap().push(url.to_string());
            })),
            ..crate::PlatformHooks::default()
        };

        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::new(papercraft_engine::Session::default(), hooks);
        crate::open_profile_screen(&mut app);
        app.view.account.profile_section = ProfileSection::ServerSync;

        let find_text_center = |shapes: &[ClippedShape], needle: &str| -> Option<egui::Pos2> {
            fn visit(shape: &Shape, needle: &str) -> Option<egui::Pos2> {
                match shape {
                    Shape::Text(text) if text.galley.text() == needle => {
                        Some(text.pos + text.galley.rect.center().to_vec2())
                    }
                    Shape::Vec(items) => items.iter().find_map(|s| visit(s, needle)),
                    _ => None,
                }
            }
            shapes.iter().find_map(|c| visit(&c.shape, needle))
        };

        let mut time = 0.0_f64;
        let run_settled = |ctx: &egui::Context,
                           app: &mut PaperCraftApp,
                           time: &mut f64|
         -> (String, Vec<ClippedShape>) {
            let mut text = String::new();
            let mut out_shapes = Vec::new();
            for _ in 0..4 {
                *time += 0.05;
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(*time),
                    predicted_dt: 0.05,
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| {
                    crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                    show(app, ui);
                });
                text = collect_text(&out.shapes);
                out_shapes = out.shapes.clone();
                out.drop_without_applying_deltas();
            }
            (text, out_shapes)
        };

        let (light_text, shapes) = run_settled(&ctx, &mut app, &mut time);
        assert!(light_text.contains("PaperCraft Server"));
        assert!(light_text.contains("standalone local desktop app"));
        assert!(light_text.contains("github.com/Alyx-Labs/papercraft"));

        let link_pos = find_text_center(&shapes, "github.com/Alyx-Labs/papercraft")
            .expect("GitHub disclaimer link should render");
        for pressed in [true, false] {
            time += 0.05;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(time),
                predicted_dt: 0.05,
                events: vec![
                    egui::Event::PointerMoved(link_pos),
                    egui::Event::PointerButton {
                        pos: link_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                show(&mut app, ui);
            });
            out.drop_without_applying_deltas();
        }
        assert_eq!(
            *opened.lock().unwrap(),
            vec!["https://github.com/Alyx-Labs/papercraft".to_string()]
        );

        let _ = app
            .session
            .execute("theme.set", &serde_json::json!({ "theme": "Dark" }));
        let (dark_text, _) = run_settled(&ctx, &mut app, &mut time);
        assert!(dark_text.contains("PaperCraft Server"));
        assert!(dark_text.contains("standalone local desktop app"));
        assert!(dark_text.contains("github.com/Alyx-Labs/papercraft"));
    }

    #[test]
    fn profile_header_matches_toolbar_height_omits_back_button_and_uses_icon_only_controls() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        crate::open_profile_screen(&mut app);

        let mut header_rect = egui::Rect::NOTHING;
        let mut header_texts: Vec<String> = Vec::new();
        let mut settings_tab = None;
        let mut sync_btn = None;
        for step in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(f64::from(step) * 0.05),
                predicted_dt: 0.05,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                show(&mut app, ui);
            });
            header_rect = egui::Rect::NOTHING;
            header_texts.clear();
            settings_tab = None;
            sync_btn = None;
            fn find_header(shape: &Shape, out: &mut egui::Rect) {
                match shape {
                    Shape::Rect(rs)
                        if rs.rect.top() <= 1.0
                            && (rs.rect.width() - 1280.0).abs() <= 1.0
                            && rs.rect.height() >= 40.0
                            && rs.rect.height() <= 64.0 =>
                    {
                        *out = rs.rect;
                    }
                    Shape::Vec(items) => {
                        for item in items {
                            find_header(item, out);
                        }
                    }
                    _ => {}
                }
            }
            for clipped in &out.shapes {
                find_header(&clipped.shape, &mut header_rect);
            }
            fn scan(
                shape: &Shape,
                header: egui::Rect,
                texts: &mut Vec<String>,
                edge_tab: &mut Option<egui::Rect>,
                sync_rect: &mut Option<egui::Rect>,
            ) {
                match shape {
                    Shape::Text(text) if header.contains(text.visual_bounding_rect().center()) => {
                        texts.push(text.galley.text().to_string());
                    }
                    Shape::Rect(rs)
                        if header.contains(rs.rect.center())
                            && (rs.rect.height() - HEADER_ROW_HEIGHT).abs() <= 0.5
                            && rs.corner_radius.nw >= 14 =>
                    {
                        *edge_tab = Some(rs.rect);
                    }
                    Shape::Rect(rs)
                        if header.contains(rs.rect.center())
                            && (rs.rect.height() - HEADER_ROW_HEIGHT).abs() <= 0.5
                            && (rs.rect.width() - HEADER_ROW_HEIGHT).abs() <= 0.5 =>
                    {
                        *sync_rect = Some(rs.rect);
                    }
                    Shape::Vec(items) => {
                        for item in items {
                            scan(item, header, texts, edge_tab, sync_rect);
                        }
                    }
                    _ => {}
                }
            }
            for clipped in &out.shapes {
                scan(
                    &clipped.shape,
                    header_rect,
                    &mut header_texts,
                    &mut settings_tab,
                    &mut sync_btn,
                );
            }
            out.drop_without_applying_deltas();
        }

        assert!(
            header_rect.height() >= 48.0 && header_rect.height() <= 56.0,
            "expected 52px profile header matching toolbar/home, got {header_rect:?}"
        );
        assert!(
            header_texts.iter().any(|t| t == "Profile & Cloud Sync"),
            "expected screen title in profile header, got {header_texts:?}"
        );
        for forbidden in [
            "Back to Notebook",
            "Back to Library",
            "Settings",
            "Local Only",
            "Viewing Local",
            "Sync Now",
        ] {
            assert!(
                !header_texts.iter().any(|t| t == forbidden),
                "unexpected button text {forbidden:?} in profile header: {header_texts:?}"
            );
        }
        assert!(sync_btn.is_some(), "expected 36x36 sync icon button");
        let settings_pos = settings_tab
            .expect("expected Settings edge tab in profile header")
            .center();
        for (pressed, time) in [(true, 0.25), (false, 0.30)] {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(time),
                events: vec![
                    egui::Event::PointerMoved(settings_pos),
                    egui::Event::PointerButton {
                        pos: settings_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                show(&mut app, ui);
            });
            out.drop_without_applying_deltas();
        }
        assert!(app.view.inspector_open);
    }

    #[test]
    fn profile_section_and_auth_tab_transitions_animate_smoothly() {
        fn settle_profile(ctx: &egui::Context, app: &mut PaperCraftApp, time: &mut f64) {
            for _ in 0..12 {
                *time += 0.04;
                let out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280.0, 800.0),
                        )),
                        time: Some(*time),
                        predicted_dt: 0.04,
                        ..Default::default()
                    },
                    |ui| {
                        crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                        show(app, ui);
                    },
                );
                out.drop_without_applying_deltas();
            }
        }

        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        crate::open_profile_screen(&mut app);

        let mut time = 0.0_f64;
        settle_profile(&ctx, &mut app, &mut time);
        assert!(
            (app.view.motion.profile_section.opacity() - 1.0).abs() <= f32::EPSILON,
            "expected settled initial profile section"
        );

        // Switching ProfileSection starts a directional FlowTransition that settles smoothly.
        app.view.account.profile_section = ProfileSection::ServerSync;
        time += 0.02;
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(time),
                predicted_dt: 0.02,
                ..Default::default()
            },
            |ui| {
                crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                show(&mut app, ui);
            },
        );
        out.drop_without_applying_deltas();
        assert!(
            app.view.motion.profile_section.opacity() < 1.0,
            "expected active profile_section transition opacity < 1.0, got {}",
            app.view.motion.profile_section.opacity()
        );
        settle_profile(&ctx, &mut app, &mut time);
        assert!(
            (app.view.motion.profile_section.opacity() - 1.0).abs() <= f32::EPSILON,
            "expected settled profile_section after animation"
        );

        // Switching AuthTab on ServerSync starts auth_flow FlowTransition that settles smoothly.
        app.view.account.tab = crate::account_modal::AuthTab::CreateAccount;
        time += 0.02;
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(time),
                predicted_dt: 0.02,
                ..Default::default()
            },
            |ui| {
                crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                show(&mut app, ui);
            },
        );
        out.drop_without_applying_deltas();
        assert!(
            app.view.motion.auth_flow.opacity() < 1.0,
            "expected active auth_flow transition opacity < 1.0, got {}",
            app.view.motion.auth_flow.opacity()
        );
        settle_profile(&ctx, &mut app, &mut time);
        assert!(
            (app.view.motion.auth_flow.opacity() - 1.0).abs() <= f32::EPSILON,
            "expected settled auth_flow after animation"
        );
    }
}
