//! Right-hand Profile, Account Switching, Identity, Sync & Backup drawer rendered on Home.

use egui::{Color32, CornerRadius, Margin, Rect, Sense, Stroke, StrokeKind, Vec2};
use papercraft_sync::{SyncConnectionState, SyncStatusSummary};

use crate::PaperCraftApp;
use crate::account_modal::{
    AddAccountMode, BannerTone, ProfileSection, ServerProbeState, account_icon_and_colors,
    draw_account_type_badge, draw_active_pill, draw_hairline, draw_section_card,
    draw_status_banner, draw_sync_connection_pill, fixed_secondary_button, format_notebook_count,
    header_status_dot_color, norm_probe_url_key, probe_all_known_servers,
    render_account_email_card, render_accounts_switcher_section, render_auth_banners,
    render_change_password_card, render_expired_session_reauth_box,
    render_forget_server_account_confirmation, render_local_profile_identity_card,
    render_profile_identity_card, render_server_probe_section, render_signed_in_server_card,
    render_unsynced_switch_confirmation, sync_draft_from_session, trigger_test_connection,
};
use crate::icons::{
    Icon, SwitchLayout, SwitchOption, compact_icon_button, paint_icon, segmented_switch,
};
use crate::modal::{danger_action_button, primary_action_button};
use crate::sync_worker::{complete_sign_out, trigger_manual_sync};
use crate::theme::Tokens;

/// Renders the Profile & Sync header, active account hero card, section switches, and scrollable body.
pub(crate) fn show_drawer_contents(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    if std::mem::take(&mut app.view.account.profile_probe_pending) {
        probe_all_known_servers(app, ui.ctx());
    }

    let summary = app.session.sync_status_summary();
    let cloud_pending = app.session.cloud_pending_changes_count();

    draw_profile_drawer_header(app, ui, &summary, t);
    ui.separator();

    let dt = ui.ctx().input(|i| i.stable_dt);
    let section_order: isize = match app.view.account.profile_section {
        ProfileSection::Accounts => 0,
        ProfileSection::Profile => 1,
        ProfileSection::Sync => 2,
    };
    let section_key = (summary.signed_in, section_order as u8);
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

    egui::ScrollArea::vertical()
        .id_salt("papercraft_inspector_profile_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;

            crate::motion::show_flow_content(ui, flow_opacity, flow_offset_x, |ui| {
                match app.view.account.profile_section {
                    ProfileSection::Accounts => {
                        draw_active_account_summary_card(app, ui, &summary, cloud_pending, t);
                        ui.add_space(10.0);
                        render_accounts_switcher_section(app, ui, &summary, t);
                    }
                    ProfileSection::Profile => {
                        draw_profile_details_tab(app, ui, &summary, t);
                    }
                    ProfileSection::Sync => {
                        draw_sync_and_storage_tab(app, ui, &summary, t);
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
            ui.add_space(12.0);
        });
}

fn draw_profile_drawer_header(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    t: &Tokens,
) {
    match app.view.account.profile_section {
        ProfileSection::Accounts | ProfileSection::Sync => {
            draw_profile_main_header(app, ui, t);
        }
        ProfileSection::Profile => {
            let subtitle = if summary.signed_in {
                "Update your display name, @username, email, and password."
            } else {
                "Rename your local account on this device."
            };
            draw_edit_account_header(app, ui, subtitle, t);
        }
    }
}

fn draw_profile_main_header(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let header_w = ui.available_width();
    ui.horizontal(|ui| {
        ui.set_height(28.0);
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
        paint_icon(ui.painter(), ir, Icon::Account, t.accent);
        ui.label(
            egui::RichText::new("Profile & Sync")
                .size(17.0)
                .strong()
                .color(t.text),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let add_lbl = if header_w >= 255.0 {
                Some("Add Account")
            } else if header_w >= 200.0 {
                Some("Add")
            } else {
                None
            };
            if compact_icon_button(ui, Icon::Plus, add_lbl, false, true, t)
                .on_hover_text("Sign in to a server or create another local account")
                .clicked()
            {
                app.view.account.add_account_mode = AddAccountMode::Server;
                crate::modal::open(app, crate::modal::ActiveModal::AddAccount);
            }
        });
    });

    ui.add(
        egui::Label::new(
            egui::RichText::new("Switch accounts, edit your profile, or manage sync.")
                .size(11.5)
                .color(t.text_muted),
        )
        .wrap(),
    );

    ui.add_space(8.0);
    draw_profile_subtabs(app, ui, t);
}

fn draw_profile_subtabs(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let options = [
        SwitchOption::new(
            None,
            Some("Profile"),
            "Active account, saved local and server accounts",
            true,
        ),
        SwitchOption::new(
            None,
            Some("Sync"),
            "Configure sync server and device storage",
            true,
        ),
    ];
    let active = match app.view.account.profile_section {
        ProfileSection::Sync => 1,
        ProfileSection::Accounts | ProfileSection::Profile => 0,
    };
    let (clicked, _) = segmented_switch(
        ui,
        "inspector_profile_subtabs",
        &options,
        Some(active),
        SwitchLayout::EqualWidth,
        t,
    );
    if let Some(idx) = clicked {
        app.view.account.error_message = None;
        app.view.account.status_message = None;
        app.view.account.profile_section = match idx {
            1 => ProfileSection::Sync,
            _ => ProfileSection::Accounts,
        };
    }
}

fn draw_edit_account_header(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    subtitle: &str,
    t: &Tokens,
) {
    ui.horizontal(|ui| {
        ui.set_height(28.0);
        if compact_icon_button(ui, Icon::PrevPage, None, false, true, t)
            .on_hover_text("Back to profile")
            .clicked()
        {
            app.view.account.error_message = None;
            app.view.account.status_message = None;
            app.view.account.profile_section = ProfileSection::Accounts;
        }
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
        paint_icon(ui.painter(), ir, Icon::Edit, t.accent);
        ui.label(
            egui::RichText::new("Edit Account")
                .size(17.0)
                .strong()
                .color(t.text),
        );
    });

    ui.add(egui::Label::new(egui::RichText::new(subtitle).size(11.5).color(t.text_muted)).wrap());
}

/// Renders the icon-only Sync status & action button in the Home/Editor top bar.
pub(crate) fn draw_header_sync_button(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> egui::Response {
    let summary = app.session.sync_status_summary();
    let in_flight = app.session.is_sync_in_flight();
    let cloud_pending = app.session.cloud_pending_changes_count();
    let size = Vec2::splat(crate::icons::TOOLBAR_CONTROL_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    if ui.is_rect_visible(rect) {
        let hover_t =
            crate::motion::hover_factor(ui.ctx(), response.id.with("hov"), response.hovered());
        let (fill, stroke_color, icon_fg) = if !summary.signed_in {
            let base_stroke = if t.is_dark {
                t.header_border
            } else {
                Color32::TRANSPARENT
            };
            (
                crate::motion::lerp_color(Color32::TRANSPARENT, t.header_control_hover, hover_t),
                crate::motion::lerp_color(base_stroke, t.header_border, hover_t),
                crate::motion::lerp_color(
                    t.header_fg_muted.gamma_multiply(0.50),
                    t.header_fg_muted.gamma_multiply(0.90),
                    hover_t,
                ),
            )
        } else {
            let effective_state = if in_flight {
                SyncConnectionState::Syncing
            } else if summary.connection == SyncConnectionState::Synced && cloud_pending > 0 {
                SyncConnectionState::Offline
            } else {
                summary.connection
            };
            let status_color = header_status_dot_color(effective_state, t);
            (
                crate::motion::lerp_color(
                    status_color.gamma_multiply(0.14),
                    status_color.gamma_multiply(0.24),
                    hover_t,
                ),
                crate::motion::lerp_color(
                    status_color.gamma_multiply(0.50),
                    status_color.gamma_multiply(0.85),
                    hover_t,
                ),
                status_color,
            )
        };
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(9), fill);
        p.rect_stroke(
            rect,
            CornerRadius::same(9),
            Stroke::new(1.0, stroke_color),
            StrokeKind::Inside,
        );
        let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(18.0));
        paint_icon(p, icon_rect, Icon::Sync, icon_fg);
    }

    let tip = if !summary.signed_in {
        "Not linked to a sync server — click to link this local account with email and password"
            .to_string()
    } else if app.session.is_session_expired() {
        "Session expired — click to sign in and resume sync".to_string()
    } else if in_flight {
        "Syncing changes with server...".to_string()
    } else {
        match summary.connection {
            SyncConnectionState::Error => "Sync issue — click to retry sync".to_string(),
            SyncConnectionState::Offline => "Offline — click to retry sync".to_string(),
            SyncConnectionState::Syncing => "Syncing changes with server...".to_string(),
            SyncConnectionState::Synced if cloud_pending > 0 => {
                format!("{cloud_pending} pending change(s) — click to sync now")
            }
            _ => "Synced with server — click to sync now".to_string(),
        }
    };

    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip);
    if response.clicked() {
        if !summary.signed_in {
            if let Some(active_local) = app.session.active_local_profile().cloned() {
                app.view.account.convert_source_profile_id = active_local.local_profile_id;
                app.view.account.convert_source_nickname = active_local.nickname;
                app.view.account.convert_source_notebook_count = app.session.notebooks.len();
                app.view.account.convert_email.clear();
                app.view.account.convert_password.clear();
                app.view.account.convert_show_password = false;
                crate::modal::open(app, crate::modal::ActiveModal::ConvertLocalAccount);
            } else {
                app.view.account.add_account_mode = AddAccountMode::Server;
                crate::modal::open(app, crate::modal::ActiveModal::AddAccount);
            }
        } else if app.session.is_session_expired() {
            crate::open_profile_drawer(app);
        } else if !in_flight {
            app.view.account.error_message = None;
            app.view.account.status_message = None;
            trigger_manual_sync(app, ui.ctx());
        }
    }
    response
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
    let (primary, subtitle) = resolve_summary_identity(summary);
    let server_online = if summary.signed_in {
        summary.connection != SyncConnectionState::Offline
            && !matches!(
                app.view.account.probe_for(&summary.server_url),
                ServerProbeState::Failed { .. }
            )
    } else {
        false
    };
    let active_acct = summary
        .signed_in
        .then(|| app.session.active_account_session().cloned())
        .flatten();
    let is_confirming_forget = active_acct.as_ref().is_some_and(|acct| {
        let key = (
            norm_probe_url_key(&acct.server_url),
            acct.user.user_id.clone(),
        );
        app.view.account.pending_forget_server_key.as_ref() == Some(&key)
    });
    let hero_fill = if is_confirming_forget {
        t.danger_bg
    } else {
        crate::motion::lerp_color(
            t.field_bg,
            t.accent_soft,
            if t.is_dark { 0.42 } else { 0.55 },
        )
    };
    let hero_stroke = if is_confirming_forget {
        Stroke::new(1.25, t.danger_border)
    } else {
        Stroke::new(
            1.25,
            t.accent.gamma_multiply(if t.is_dark { 0.55 } else { 0.65 }),
        )
    };

    egui::Frame::new()
        .fill(hero_fill)
        .stroke(hero_stroke)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(11, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.multiply_opacity(identity_opacity);

            draw_active_account_header(ui, summary, server_online, &primary, &subtitle, t);
            draw_active_account_meta_row(
                ui,
                summary,
                server_online,
                app.session.notebooks.len(),
                cloud_pending,
                t,
            );

            if summary.signed_in && !summary.server_url.trim().is_empty() {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&summary.server_url)
                            .size(11.0)
                            .color(t.text_muted),
                    )
                    .wrap(),
                );
            }

            draw_active_account_actions(
                app,
                ui,
                summary,
                active_acct.as_ref(),
                is_confirming_forget,
                t,
            );
            draw_active_account_alerts(
                app,
                ui,
                active_acct.as_ref(),
                is_confirming_forget,
                &primary,
                cloud_pending,
                t,
            );
        });
}

fn draw_active_account_header(
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    server_online: bool,
    primary: &str,
    subtitle: &str,
    t: &Tokens,
) {
    let (acct_icon, icon_fg, icon_bg, icon_border) =
        account_icon_and_colors(summary.signed_in, server_online, t);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (badge_rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
        ui.painter()
            .rect_filled(badge_rect, CornerRadius::same(8), icon_bg);
        ui.painter().rect_stroke(
            badge_rect,
            CornerRadius::same(8),
            Stroke::new(1.0, icon_border),
            egui::StrokeKind::Inside,
        );
        paint_icon(ui.painter(), badge_rect.shrink(7.0), acct_icon, icon_fg);

        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                let name_max_w = (ui.available_width() - 78.0).max(48.0);
                ui.scope(|ui| {
                    ui.set_max_width(name_max_w);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(primary)
                                .size(14.0)
                                .strong()
                                .color(t.text),
                        )
                        .truncate(),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    draw_active_pill(ui, t);
                });
            });
            if !subtitle.is_empty() {
                ui.add(
                    egui::Label::new(egui::RichText::new(subtitle).size(11.0).color(t.text_muted))
                        .wrap(),
                );
            }
        });
    });
}

fn draw_active_account_meta_row(
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    server_online: bool,
    notebook_count: usize,
    cloud_pending: usize,
    t: &Tokens,
) {
    let nb_label = format_notebook_count(notebook_count);
    let meta = if summary.signed_in {
        match cloud_pending {
            0 => nb_label,
            1 => format!("{nb_label} · 1 pending"),
            n => format!("{nb_label} · {n} pending"),
        }
    } else {
        format!("{nb_label} · Saved on this device")
    };

    let pills_reserve_w = if summary.signed_in {
        if summary.connection == SyncConnectionState::Error {
            178.0
        } else {
            162.0
        }
    } else {
        74.0
    };

    ui.horizontal(|ui| {
        let left_max_w = (ui.available_width() - pills_reserve_w).max(48.0);
        ui.scope(|ui| {
            ui.set_max_width(left_max_w);
            ui.add(
                egui::Label::new(egui::RichText::new(meta).size(11.0).color(t.text_muted)).wrap(),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            draw_account_type_badge(ui, summary.signed_in, server_online, t);
            if summary.signed_in {
                draw_sync_connection_pill(ui, summary.connection, t);
            }
        });
    });
}

fn draw_active_account_actions(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    active_acct: Option<&papercraft_sync::ActiveAccountSession>,
    is_confirming_forget: bool,
    t: &Tokens,
) {
    if !summary.signed_in && app.session.active_local_profile().is_none() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        if summary.signed_in {
            let in_flight = app.session.is_sync_in_flight();
            let can_sync = !in_flight && app.session.is_viewing_cloud();
            let sync_lbl = if in_flight { "Syncing..." } else { "Sync Now" };
            if primary_action_button(ui, sync_lbl, can_sync, t).clicked() {
                app.view.account.error_message = None;
                app.view.account.status_message = None;
                trigger_manual_sync(app, ui.ctx());
            }
            if fixed_secondary_button(ui, "Edit", 56.0, true, t)
                .on_hover_text("Edit display name, @username, email, or password")
                .clicked()
            {
                sync_draft_from_session(app);
                app.view.account.profile_section = ProfileSection::Profile;
            }
            if danger_action_button(ui, "Sign Out", true, t).clicked() {
                complete_sign_out(app, ui.ctx());
            }
            if !is_confirming_forget
                && let Some(acct) = active_acct
                && fixed_secondary_button(ui, "Forget Account", 104.0, true, t)
                    .on_hover_text(
                        "Remove this account shortcut and delete ONLY its local cache on this device",
                    )
                    .clicked()
            {
                app.view.account.pending_forget_server_key = Some((
                    norm_probe_url_key(&acct.server_url),
                    acct.user.user_id.clone(),
                ));
                app.view.account.clear_quick_login_draft();
                app.view.account.clear_pending_delete_profile();
            }
        } else if app.session.active_local_profile().is_some()
            && fixed_secondary_button(ui, "Edit", 56.0, true, t)
                .on_hover_text("Edit this local account nickname")
                .clicked()
        {
            sync_draft_from_session(app);
            app.view.account.profile_section = ProfileSection::Profile;
        }
    });
}

fn draw_active_account_alerts(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    active_acct: Option<&papercraft_sync::ActiveAccountSession>,
    is_confirming_forget: bool,
    primary: &str,
    cloud_pending: usize,
    t: &Tokens,
) {
    let Some(acct) = active_acct else {
        return;
    };
    crate::motion::animated_collapse_with_duration(
        ui,
        ui.make_persistent_id((
            "summary_forget_confirm",
            &acct.server_url,
            &acct.user.user_id,
        )),
        is_confirming_forget,
        152.0,
        crate::motion::MICRO_DURATION_S,
        |ui| {
            ui.add_space(4.0);
            render_forget_server_account_confirmation(
                app,
                ui,
                &acct.server_url,
                &acct.user.user_id,
                primary,
                cloud_pending,
                t,
            );
        },
    );
    if app.session.is_session_expired() {
        ui.add_space(4.0);
        render_expired_session_reauth_box(app, ui, acct, cloud_pending, "summary_reauth_pw", t);
    }
}

fn resolve_summary_identity(summary: &SyncStatusSummary) -> (String, String) {
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
            (None, None) => String::new(),
        };
        (disp, email_or_handle)
    } else {
        let nick = summary
            .active_local_profile_name
            .clone()
            .unwrap_or_else(|| "Personal".to_string());
        (nick, String::new())
    }
}

fn draw_profile_details_tab(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    t: &Tokens,
) {
    if summary.signed_in {
        draw_section_card(ui, t, |ui| {
            render_profile_identity_card(app, ui, t);
        });
        ui.add_space(8.0);
        draw_section_card(ui, t, |ui| {
            render_account_email_card(app, ui, summary, t);
        });
        ui.add_space(8.0);
        draw_section_card(ui, t, |ui| {
            render_change_password_card(app, ui, t);
        });
    } else {
        draw_section_card(ui, t, |ui| {
            render_local_profile_identity_card(app, ui, t);
        });
    }
}

fn draw_sync_and_storage_tab(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    t: &Tokens,
) {
    let mut test_on_enter = false;
    draw_section_card(ui, t, |ui| {
        if summary.signed_in {
            test_on_enter = render_signed_in_server_card(app, ui, summary, t);
        } else {
            test_on_enter = render_server_probe_section(app, ui, t, true);
        }
    });
    if test_on_enter && !app.view.account.is_testing() {
        trigger_test_connection(app, ui.ctx());
    }
    render_unsynced_switch_confirmation(app, ui, t);

    ui.add_space(8.0);
    draw_storage_overview_card(app, ui, t);
    ui.add_space(8.0);
    draw_server_download_note(app, ui, t);
}

fn draw_storage_overview_card(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let active_notebooks = format_notebook_count(app.session.notebooks.len());
    let trash_count = format_notebook_count(app.session.trashed_notebooks.len());
    let desktop_dir = app.session.library_dir().map(|p| p.display().to_string());
    let web_status = app.web_storage_status.clone();

    draw_section_card(ui, t, |ui| {
        ui.horizontal(|ui| {
            let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            paint_icon(ui.painter(), ir, Icon::Library, t.accent);
            ui.label(
                egui::RichText::new("Storage")
                    .size(13.5)
                    .strong()
                    .color(t.text),
            );
        });
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!("{active_notebooks} · {trash_count} in Trash"))
                .size(11.5)
                .color(t.text),
        );
        ui.add_space(4.0);
        draw_hairline(ui, t);
        ui.add_space(6.0);

        if let Some(status) = web_status {
            let (info_text, tip) = crate::inspector::web_storage_info_from_status(&status);
            let resp = ui.add(
                egui::Label::new(
                    egui::RichText::new(info_text)
                        .size(11.0)
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
                    egui::RichText::new(usage_text)
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
                    egui::RichText::new(dir)
                        .size(11.0)
                        .monospace()
                        .color(t.text_muted),
                )
                .wrap(),
            );
        } else {
            ui.label(
                egui::RichText::new("In-memory session")
                    .size(11.0)
                    .color(t.text_muted),
            );
        }
    });
}

fn draw_server_download_note(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let mut open_github = false;
    draw_section_card(ui, t, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(5.0, 3.0);
            let (link_icon_rect, _) = ui.allocate_exact_size(Vec2::splat(13.0), Sense::hover());
            paint_icon(ui.painter(), link_icon_rect, Icon::Link, t.text_muted);
            ui.label(
                egui::RichText::new("Self-hosted server:")
                    .size(11.0)
                    .color(t.text_muted),
            );
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

    if open_github {
        let _ = crate::open_external_url(app, ui.ctx(), crate::GITHUB_REPO_URL);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
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

    fn render_drawer_text(ctx: &egui::Context, app: &mut PaperCraftApp) -> String {
        let mut text = String::new();
        for step in 0..8 {
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
                crate::inspector::show(app, ui);
            });
            text = collect_text(&out.shapes);
            out.drop_without_applying_deltas();
        }
        text
    }

    #[test]
    fn signed_out_add_account_modal_transitions_into_password_reset_flow() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        app.view.account.add_account_mode = AddAccountMode::Server;
        crate::modal::open(&mut app, crate::modal::ActiveModal::AddAccount);
        app.view.account.tab = AuthTab::CreateAccount;

        let mut signup_text = String::new();
        for step in 0..6 {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(f64::from(step) * 0.05),
                    predicted_dt: 0.05,
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                    crate::modal::show(&mut app, ui);
                },
            );
            signup_text = collect_text(&out.shapes);
            out.drop_without_applying_deltas();
        }
        assert!(signup_text.contains("Add Account"));
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
        let mut reset_text = String::new();
        for step in 0..6 {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(0.5 + f64::from(step) * 0.05),
                    predicted_dt: 0.05,
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                    crate::modal::show(&mut app, ui);
                },
            );
            reset_text = collect_text(&out.shapes);
            out.drop_without_applying_deltas();
        }
        assert!(reset_text.contains("Complete Password Reset"));
        assert!(reset_text.contains("Temporary Recovery Code"));
        assert!(reset_text.contains("Set New Password & Sign In"));
    }

    #[test]
    fn signed_in_profile_drawer_renders_all_sections_in_light_and_dark() {
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
        crate::open_profile_drawer(&mut app);
        assert!(app.view.inspector_open);
        assert_eq!(app.view.account.profile_section, ProfileSection::Accounts);

        fn assert_contains_all(haystack: &str, needles: &[&str]) {
            for needle in needles {
                assert!(
                    haystack.contains(needle),
                    "missing {needle:?} in {haystack}"
                );
            }
        }

        let accts_text = render_drawer_text(&ctx, &mut app);
        assert!(
            !accts_text.lines().any(|line| line == "Settings"),
            "Home Profile drawer must not show Settings switch: {accts_text}"
        );
        assert!(
            !accts_text.lines().any(|line| line == "Details"),
            "Home Profile drawer must not show segmented Details switch: {accts_text}"
        );
        assert_contains_all(
            &accts_text,
            &[
                "Profile & Sync",
                "Active",
                "Alex Silva",
                "@alex_notes",
                "Edit",
                "Forget Account",
                "Local Accounts (1)",
                "Server Accounts (0)",
                "No other server accounts saved on this device.",
                "Add Account",
                "Ana",
            ],
        );

        app.view.account.profile_section = ProfileSection::Profile;
        let profile_text = render_drawer_text(&ctx, &mut app);
        assert_contains_all(
            &profile_text,
            &[
                "Edit Account",
                "Profile Details",
                "Save Profile",
                "Email Address",
                "Change Password",
                "alex_notes",
            ],
        );

        app.view.account.profile_section = ProfileSection::Sync;
        let _ = app
            .session
            .execute("theme.set", &serde_json::json!({ "theme": "Dark" }));
        let sync_text = render_drawer_text(&ctx, &mut app);
        assert!(
            !sync_text.contains("Backup (.pcraftpkg)") && !sync_text.contains("Export Backup"),
            "Sync tab must not show redundant Backup (.pcraftpkg) card: {sync_text}"
        );
        assert_contains_all(
            &sync_text,
            &[
                "Profile",
                "Sync",
                "Server Address & Connection",
                "Storage",
                "github.com/Alyx-Labs/papercraft",
            ],
        );
    }

    #[test]
    fn signed_out_profile_drawer_exposes_accounts_details_and_sync_in_light_and_dark() {
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
            cross_tab_syncing: false,
            cross_tab_error: None,
        });
        crate::open_profile_drawer(&mut app);
        let acct_text = render_drawer_text(&ctx, &mut app);
        assert!(acct_text.contains("Active"), "{acct_text}");
        assert!(!acct_text.contains("Connect Server"), "{acct_text}");
        assert!(!acct_text.contains("Upload to Server"), "{acct_text}");
        assert!(acct_text.contains("Edit"), "{acct_text}");
        assert!(
            !acct_text.lines().any(|line| line == "Details"),
            "Home Profile drawer must not show segmented Details switch: {acct_text}"
        );
        assert!(acct_text.contains("Local Accounts (0)"), "{acct_text}");
        assert!(
            acct_text.contains("No other local accounts on this device."),
            "{acct_text}"
        );
        assert!(acct_text.contains("Add Account"), "{acct_text}");
        assert!(acct_text.contains("Mia"), "{acct_text}");

        app.view.account.profile_section = ProfileSection::Profile;
        let prof_text = render_drawer_text(&ctx, &mut app);
        assert!(prof_text.contains("Edit Account"), "{prof_text}");
        assert!(prof_text.contains("Local Account Profile"), "{prof_text}");
        assert!(prof_text.contains("Save Nickname"), "{prof_text}");

        app.view.account.profile_section = ProfileSection::Sync;
        let _ = app
            .session
            .execute("theme.set", &serde_json::json!({ "theme": "Dark" }));
        let sync_text = render_drawer_text(&ctx, &mut app);
        assert!(sync_text.contains("IndexedDB"), "{sync_text}");
        assert!(sync_text.contains("2.0 MB / 128.0 MB"), "{sync_text}");
    }

    #[test]
    fn sync_tab_github_link_opens_repo_url() {
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
        crate::open_profile_drawer(&mut app);
        app.view.account.profile_section = ProfileSection::Sync;

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
        let mut out_shapes = Vec::new();
        for _ in 0..8 {
            time += 0.05;
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(time),
                    predicted_dt: 0.05,
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                    crate::inspector::show(&mut app, ui);
                },
            );
            out_shapes = out.shapes.clone();
            out.drop_without_applying_deltas();
        }

        let link_pos = find_text_center(&out_shapes, "github.com/Alyx-Labs/papercraft")
            .expect("GitHub link should render");
        for pressed in [true, false] {
            time += 0.05;
            let out = ctx.run_ui(
                egui::RawInput {
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
                },
                |ui| {
                    crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                    crate::inspector::show(&mut app, ui);
                },
            );
            out.drop_without_applying_deltas();
        }
        assert_eq!(
            *opened.lock().unwrap(),
            vec![crate::GITHUB_REPO_URL.to_string()]
        );
    }

    #[test]
    fn profile_section_transitions_animate_smoothly_in_drawer() {
        fn settle_drawer(ctx: &egui::Context, app: &mut PaperCraftApp, time: &mut f64) {
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
                        crate::inspector::show(app, ui);
                    },
                );
                out.drop_without_applying_deltas();
            }
        }

        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        crate::open_profile_drawer(&mut app);

        let mut time = 0.0_f64;
        settle_drawer(&ctx, &mut app, &mut time);
        assert!(
            (app.view.motion.profile_section.opacity() - 1.0).abs() <= f32::EPSILON,
            "expected settled initial profile section"
        );

        app.view.account.profile_section = ProfileSection::Sync;
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
                crate::inspector::show(&mut app, ui);
            },
        );
        out.drop_without_applying_deltas();
        assert!(
            app.view.motion.profile_section.opacity() < 1.0,
            "expected active profile_section transition opacity < 1.0, got {}",
            app.view.motion.profile_section.opacity()
        );
        settle_drawer(&ctx, &mut app, &mut time);
        assert!(
            (app.view.motion.profile_section.opacity() - 1.0).abs() <= f32::EPSILON,
            "expected settled profile_section after animation"
        );
    }

    #[test]
    fn editor_inspector_hides_profile_switch_and_profile_controls() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        app.view.active_modal = None;
        crate::open_profile_drawer(&mut app);
        app.view.screen = crate::AppScreen::Editor;

        let mut text = String::new();
        for step in 0..8 {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(f64::from(step) * 0.04),
                    predicted_dt: 0.04,
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    crate::inspector::show(&mut app, ui);
                },
            );
            text = collect_text(&out.shapes);
            out.drop_without_applying_deltas();
        }
        assert!(text.contains("Settings"), "{text}");
        assert!(text.contains("Pen"), "{text}");
        assert!(text.contains("Appearance"), "{text}");
        assert!(text.contains("System"), "{text}");
        assert!(!text.contains("Profile & Sync"), "{text}");
        assert!(!text.contains("Add Account"), "{text}");
    }

    #[test]
    fn active_account_card_sits_above_subtabs_and_excludes_active_from_lists() {
        fn render_text_ys(
            ctx: &egui::Context,
            app: &mut PaperCraftApp,
            start_t: f64,
        ) -> Vec<(String, f32)> {
            fn collect(shape: &Shape, out: &mut Vec<(String, f32)>) {
                match shape {
                    Shape::Text(text) => {
                        out.push((
                            text.galley.text().to_string(),
                            text.visual_bounding_rect().center().y,
                        ));
                    }
                    Shape::Vec(items) => {
                        for item in items {
                            collect(item, out);
                        }
                    }
                    _ => {}
                }
            }

            let mut out_items = Vec::new();
            for step in 0..8 {
                let out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280.0, 900.0),
                        )),
                        time: Some(start_t + f64::from(step) * 0.04),
                        predicted_dt: 0.04,
                        ..Default::default()
                    },
                    |ui| {
                        crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                        crate::inspector::show(app, ui);
                    },
                );
                out_items.clear();
                for clipped in &out.shapes {
                    collect(&clipped.shape, &mut out_items);
                }
                out.drop_without_applying_deltas();
            }
            out_items
        }

        let ctx = egui::Context::default();
        let mut session = papercraft_engine::Session::open_web_cache(None);
        session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "Alyx" }),
            )
            .unwrap();
        let _ = session
            .execute(
                "profile.create",
                &serde_json::json!({ "nickname": "Work", "switch_now": false }),
            )
            .unwrap();
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        app.view.active_modal = None;
        crate::open_profile_drawer(&mut app);

        let local_items = render_text_ys(&ctx, &mut app, 0.0);
        let alyx_ys: Vec<f32> = local_items
            .iter()
            .filter(|(label, _)| label == "Alyx")
            .map(|(_, y)| *y)
            .collect();
        let add_acct_y = local_items
            .iter()
            .find(|(label, _)| label == "Add Account")
            .map(|(_, y)| *y)
            .expect("Add Account header button");
        let local_header_y = local_items
            .iter()
            .find(|(label, _)| label == "Local Accounts (1)")
            .map(|(_, y)| *y)
            .expect("Local Accounts (1) header");
        let work_y = local_items
            .iter()
            .find(|(label, _)| label == "Work")
            .map(|(_, y)| *y)
            .expect("inactive Work local profile");
        assert_eq!(
            alyx_ys.len(),
            1,
            "active local account must appear only in hero card: {local_items:?}"
        );
        assert!(
            add_acct_y < alyx_ys[0] && alyx_ys[0] < local_header_y && local_header_y < work_y,
            "unexpected local drawer hierarchy: {local_items:?}"
        );

        // Now add a remembered server account "Bob" and sign in as active server account "Alex Silva".
        let bob = ActiveAccountSession {
            server_url: DEFAULT_SERVER_URL.to_string(),
            user: UserProfile {
                user_id: "u-bob".to_string(),
                email: Some("bob@example.com".to_string()),
                username: "bob_notes".to_string(),
                display_name: "Bob".to_string(),
                created_at_ms: 1000,
                must_change_password: false,
            },
            access_token: "acc-b".to_string(),
            refresh_token: "ref-b".to_string(),
            access_expires_at_ms: 99_999,
            refresh_expires_at_ms: 99_999,
            device_name: "Desk".to_string(),
            last_auth_ms: 1000,
        };
        app.session.enter_online_server_session(
            bob.clone(),
            SyncPullResponse {
                server_cursor: 0,
                user: bob.user,
                notebooks: Vec::new(),
                folders: Vec::new(),
                studio_prefs: None,
            },
        );
        assert!(app.session.sign_out_from_server_session());

        let alex = ActiveAccountSession {
            server_url: DEFAULT_SERVER_URL.to_string(),
            user: UserProfile {
                user_id: "u-alex".to_string(),
                email: Some("alex@example.com".to_string()),
                username: "alex_notes".to_string(),
                display_name: "Alex Silva".to_string(),
                created_at_ms: 2000,
                must_change_password: false,
            },
            access_token: "acc-a".to_string(),
            refresh_token: "ref-a".to_string(),
            access_expires_at_ms: 99_999,
            refresh_expires_at_ms: 99_999,
            device_name: "Desk".to_string(),
            last_auth_ms: 2000,
        };
        app.session.enter_online_server_session(
            alex.clone(),
            SyncPullResponse {
                server_cursor: 0,
                user: alex.user,
                notebooks: Vec::new(),
                folders: Vec::new(),
                studio_prefs: None,
            },
        );

        let server_items = render_text_ys(&ctx, &mut app, 1.0);
        let alex_ys: Vec<f32> = server_items
            .iter()
            .filter(|(label, _)| label == "Alex Silva")
            .map(|(_, y)| *y)
            .collect();
        let s_local_hdr_y = server_items
            .iter()
            .find(|(label, _)| label == "Local Accounts (2)")
            .map(|(_, y)| *y)
            .expect("Local Accounts (2) header");
        let s_server_hdr_y = server_items
            .iter()
            .find(|(label, _)| label == "Server Accounts (1)")
            .map(|(_, y)| *y)
            .expect("Server Accounts (1) header");
        let bob_y = server_items
            .iter()
            .find(|(label, _)| label == "Bob")
            .map(|(_, y)| *y)
            .expect("inactive remembered server account Bob");
        assert_eq!(
            alex_ys.len(),
            1,
            "active server account must appear only in hero card: {server_items:?}"
        );
        assert!(
            alex_ys[0] < s_local_hdr_y && s_local_hdr_y < s_server_hdr_y && s_server_hdr_y < bob_y,
            "unexpected signed-in drawer hierarchy: {server_items:?}"
        );
    }

    #[test]
    fn active_account_card_places_type_badge_right_below_active_without_overlap() {
        fn collect_text_rects(
            ctx: &egui::Context,
            app: &mut PaperCraftApp,
            start_t: f64,
        ) -> Vec<(String, egui::Rect)> {
            fn visit(shape: &Shape, out: &mut Vec<(String, egui::Rect)>) {
                match shape {
                    Shape::Text(text) => {
                        out.push((text.galley.text().to_string(), text.visual_bounding_rect()));
                    }
                    Shape::Vec(items) => {
                        for item in items {
                            visit(item, out);
                        }
                    }
                    _ => {}
                }
            }

            let mut out_rects = Vec::new();
            for step in 0..8 {
                let out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280.0, 900.0),
                        )),
                        time: Some(start_t + f64::from(step) * 0.04),
                        predicted_dt: 0.04,
                        ..Default::default()
                    },
                    |ui| {
                        crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                        crate::inspector::show(app, ui);
                    },
                );
                out_rects.clear();
                for clipped in &out.shapes {
                    visit(&clipped.shape, &mut out_rects);
                }
                out.drop_without_applying_deltas();
            }
            out_rects
        }

        fn find_rect(rects: &[(String, egui::Rect)], needle: &str) -> egui::Rect {
            rects
                .iter()
                .find(|(text, _)| text == needle)
                .map(|(_, rect)| *rect)
                .unwrap_or_else(|| panic!("missing text {needle:?} in {rects:?}"))
        }

        let ctx = egui::Context::default();
        let mut session = papercraft_engine::Session::open_web_cache(None);
        session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "alyx" }),
            )
            .unwrap();
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        app.view.active_modal = None;
        crate::open_profile_drawer(&mut app);

        let local_rects = collect_text_rects(&ctx, &mut app, 0.0);
        let name_r = find_rect(&local_rects, "alyx");
        let active_r = find_rect(&local_rects, "Active");
        let local_r = find_rect(&local_rects, "Local");
        let meta_r = local_rects
            .iter()
            .find(|(text, _)| text.contains("Saved on this device"))
            .map(|(_, r)| *r)
            .unwrap_or_else(|| panic!("missing local meta row in {local_rects:?}"));
        let edit_r = find_rect(&local_rects, "Edit");
        assert!(
            !local_rects.iter().any(|(text, _)| text == "Connect Server"),
            "Local card should not show Connect Server: {local_rects:?}"
        );
        assert!(
            !local_rects
                .iter()
                .any(|(text, _)| text == "Upload to Server"),
            "Local card should not show Upload to Server (moved to header Sync button): {local_rects:?}"
        );

        assert!(
            local_r.top() > active_r.bottom(),
            "Local badge ({local_r:?}) should sit below Active ({active_r:?})"
        );
        assert!(
            (local_r.right() - active_r.right()).abs() < 8.0,
            "Local badge ({local_r:?}) should align on right edge under Active ({active_r:?})"
        );
        assert!(
            meta_r.right() < local_r.left(),
            "Local meta ({meta_r:?}) must stay left of Local badge ({local_r:?})"
        );
        assert!(
            meta_r.top() > name_r.bottom(),
            "Local meta ({meta_r:?}) should sit below account name ({name_r:?})"
        );
        assert!(
            (meta_r.center().y - local_r.center().y).abs() < 6.0,
            "Local meta ({meta_r:?}) should share row with Local badge ({local_r:?})"
        );
        assert!(
            edit_r.top() > local_r.bottom(),
            "Action button ({edit_r:?}) should sit below Local badge ({local_r:?})"
        );

        let acct = ActiveAccountSession {
            server_url: DEFAULT_SERVER_URL.to_string(),
            user: UserProfile {
                user_id: "u-layout".to_string(),
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

        let server_rects = collect_text_rects(&ctx, &mut app, 1.0);
        let s_name_r = find_rect(&server_rects, "Alex Silva");
        let s_sub_r = find_rect(&server_rects, "alex@example.com (@alex_notes)");
        let s_active_r = find_rect(&server_rects, "Active");
        let s_server_r = find_rect(&server_rects, "Server");
        let s_synced_r = find_rect(&server_rects, "Synced");
        let s_meta_r = find_rect(&server_rects, "0 notebooks");
        let s_url_r = find_rect(&server_rects, DEFAULT_SERVER_URL);

        assert!(
            s_sub_r.top() > s_name_r.bottom(),
            "Server subtitle ({s_sub_r:?}) should sit below name ({s_name_r:?})"
        );
        assert!(
            s_server_r.top() > s_active_r.bottom(),
            "Server badge ({s_server_r:?}) should sit below Active ({s_active_r:?})"
        );
        assert!(
            (s_server_r.right() - s_active_r.right()).abs() < 8.0,
            "Server badge ({s_server_r:?}) should align on right edge under Active ({s_active_r:?})"
        );
        assert!(
            s_synced_r.right() < s_server_r.left(),
            "Synced pill ({s_synced_r:?}) should sit left of Server badge ({s_server_r:?})"
        );
        assert!(
            s_meta_r.right() < s_synced_r.left(),
            "Server meta ({s_meta_r:?}) must stay left of Synced pill ({s_synced_r:?})"
        );
        assert!(
            s_url_r.top() > s_server_r.bottom(),
            "Server URL ({s_url_r:?}) should sit cleanly below meta/badges ({s_server_r:?})"
        );
    }

    #[test]
    fn edit_and_sync_subview_navigation_supports_back_arrow_and_save_return() {
        fn run_frames(
            ctx: &egui::Context,
            app: &mut PaperCraftApp,
            time: &mut f64,
            frames: usize,
        ) -> Vec<(String, egui::Rect)> {
            fn visit(shape: &Shape, out: &mut Vec<(String, egui::Rect)>) {
                match shape {
                    Shape::Text(text) => {
                        out.push((text.galley.text().to_string(), text.visual_bounding_rect()));
                    }
                    Shape::Vec(items) => {
                        for item in items {
                            visit(item, out);
                        }
                    }
                    _ => {}
                }
            }
            let mut out_rects = Vec::new();
            for _ in 0..frames {
                *time += 0.04;
                let out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280.0, 900.0),
                        )),
                        time: Some(*time),
                        predicted_dt: 0.04,
                        ..Default::default()
                    },
                    |ui| {
                        crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                        crate::inspector::show(app, ui);
                    },
                );
                out_rects.clear();
                for clipped in &out.shapes {
                    visit(&clipped.shape, &mut out_rects);
                }
                out.drop_without_applying_deltas();
            }
            out_rects
        }

        fn click_at(ctx: &egui::Context, app: &mut PaperCraftApp, pos: egui::Pos2, time: &mut f64) {
            for pressed in [true, false] {
                *time += 0.04;
                let out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280.0, 900.0),
                        )),
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
                    },
                    |ui| {
                        crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                        crate::inspector::show(app, ui);
                    },
                );
                out.drop_without_applying_deltas();
            }
        }

        fn rect_of(rects: &[(String, egui::Rect)], needle: &str) -> egui::Rect {
            rects
                .iter()
                .find(|(text, _)| text == needle)
                .map(|(_, rect)| *rect)
                .unwrap_or_else(|| panic!("missing {needle:?} in {rects:?}"))
        }

        let ctx = egui::Context::default();
        let mut session = papercraft_engine::Session::open_web_cache(None);
        session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "alyx" }),
            )
            .unwrap();
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        app.view.active_modal = None;
        crate::open_profile_drawer(&mut app);

        let mut time = 0.0_f64;
        let main_rects = run_frames(&ctx, &mut app, &mut time, 8);
        assert!(
            !main_rects.iter().any(|(t, _)| t == "Details"),
            "segmented switch must be absent: {main_rects:?}"
        );

        // Clicking Edit inside the active card opens Edit Account in the sidebar.
        let edit_pos = rect_of(&main_rects, "Edit").center();
        click_at(&ctx, &mut app, edit_pos, &mut time);
        assert_eq!(app.view.account.profile_section, ProfileSection::Profile);

        let edit_rects = run_frames(&ctx, &mut app, &mut time, 8);
        let edit_title_r = rect_of(&edit_rects, "Edit Account");

        // Clicking the back arrow returns to Accounts without saving.
        let back_pos = egui::pos2(edit_title_r.left() - 40.0, edit_title_r.center().y);
        click_at(&ctx, &mut app, back_pos, &mut time);
        assert_eq!(app.view.account.profile_section, ProfileSection::Accounts);

        // Enter Edit again, rename local profile, and click Save Nickname -> saves and returns.
        let main_rects_2 = run_frames(&ctx, &mut app, &mut time, 8);
        click_at(
            &ctx,
            &mut app,
            rect_of(&main_rects_2, "Edit").center(),
            &mut time,
        );
        assert_eq!(app.view.account.profile_section, ProfileSection::Profile);
        app.view.account.local_profile_rename_draft = "Alyx Updated".to_string();
        let save_rects = run_frames(&ctx, &mut app, &mut time, 6);
        click_at(
            &ctx,
            &mut app,
            rect_of(&save_rects, "Save Nickname").center(),
            &mut time,
        );
        assert_eq!(app.view.account.profile_section, ProfileSection::Accounts);
        assert_eq!(
            app.session
                .active_local_profile()
                .map(|p| p.nickname.as_str()),
            Some("Alyx Updated")
        );

        // Clicking Sync in the Profile | Sync switch opens Sync, and clicking Profile returns.
        let after_save_rects = run_frames(&ctx, &mut app, &mut time, 8);
        assert!(after_save_rects.iter().any(|(t, _)| t == "Alyx Updated"));
        click_at(
            &ctx,
            &mut app,
            rect_of(&after_save_rects, "Sync").center(),
            &mut time,
        );
        assert_eq!(app.view.account.profile_section, ProfileSection::Sync);
        let sync_rects = run_frames(&ctx, &mut app, &mut time, 8);
        assert!(
            !sync_rects.iter().any(|(t, _)| t == "Backup (.pcraftpkg)"),
            "Sync tab must omit redundant Backup card: {sync_rects:?}"
        );
        click_at(
            &ctx,
            &mut app,
            rect_of(&sync_rects, "Profile").center(),
            &mut time,
        );
        assert_eq!(app.view.account.profile_section, ProfileSection::Accounts);

        // Signed-in server save confirmation also returns from Edit Account to Accounts.
        let acct = ActiveAccountSession {
            server_url: DEFAULT_SERVER_URL.to_string(),
            user: UserProfile {
                user_id: "u-edit".to_string(),
                email: Some("alex@example.com".to_string()),
                username: "alex_notes".to_string(),
                display_name: "Alex".to_string(),
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
        app.session
            .enter_online_server_session(acct.clone(), empty_pull);
        app.view.account.profile_section = ProfileSection::Profile;
        let mut updated_acct = acct;
        updated_acct.user.display_name = "Alex New".to_string();
        let session_gen = app.sync_worker.current_session_generation();
        apply_worker_event(
            &mut app,
            &ctx,
            SyncWorkerEvent::ProfileUpdated {
                session_generation: session_gen,
                updated_session: updated_acct,
            },
        );
        assert_eq!(app.view.account.profile_section, ProfileSection::Accounts);
    }
}
