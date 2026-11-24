//! Dedicated full-window Profile, Account Security, Sync & Workspace screen.

use egui::{CornerRadius, Margin, Sense, Stroke, Vec2};
use papercraft_sync::{SyncConnectionState, SyncStatusSummary};

use crate::PaperCraftApp;
use crate::account_modal::{
    BannerTone, PillIcon, PillStyle, ProfileSection, SignedOutProfileSection, draw_hairline,
    draw_pill_badge, draw_section_card, draw_status_banner, draw_sync_connection_pill,
    handle_auth_submit_on_enter, render_account_email_card, render_auth_banners,
    render_auth_card_contents, render_change_password_card,
    render_detected_local_accounts_migration_card, render_local_profiles_manager,
    render_logout_safeguard_actions, render_logout_safeguard_body, render_profile_identity_card,
    render_server_probe_section, render_session_sign_out_card, render_signed_in_account_card,
    render_signed_in_server_card, render_signed_in_transfer_section, trigger_test_connection,
};
use crate::icons::{
    Icon, SwitchLayout, SwitchOption, compact_icon_button, icon_button, paint_icon,
    segmented_switch,
};
use crate::modal::{primary_action_button, secondary_action_button};
use crate::sync_worker::trigger_manual_sync;
use crate::theme::Tokens;

const HEADER_ROW_HEIGHT: f32 = 28.0;
const MAX_CONTENT_WIDTH: f32 = 940.0;
const RAIL_WIDTH: f32 = 228.0;
const COMPACT_BREAKPOINT: f32 = 760.0;

/// Renders the dedicated Profile & Cloud Sync screen.
pub fn show(app: &mut PaperCraftApp, ui: &mut egui::Ui) {
    let t = Tokens::for_theme(app.session.prefs.theme);
    let summary = app.session.sync_status_summary();
    let cloud_pending = app.session.cloud_pending_changes_count();

    draw_profile_header(app, ui, &summary, cloud_pending, &t);
    crate::inspector::show(app, ui);

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(t.panel_bg))
        .show(ui, |ui| {
            let avail = ui.available_rect_before_wrap();
            let side_pad = 24.0_f32;
            let content_w = (avail.width() - side_pad * 2.0).clamp(280.0, MAX_CONTENT_WIDTH);
            let left = avail.left() + ((avail.width() - content_w) * 0.5).max(side_pad);
            let inner_rect = egui::Rect::from_min_max(
                egui::pos2(left, avail.top() + 20.0),
                egui::pos2(
                    left + content_w,
                    (avail.bottom() - 16.0).max(avail.top() + 60.0),
                ),
            );

            ui.scope_builder(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
                ui.set_width(content_w);
                if content_w >= COMPACT_BREAKPOINT {
                    draw_two_column_layout(app, ui, &summary, cloud_pending, content_w, &t);
                } else {
                    draw_compact_layout(app, ui, &summary, cloud_pending, &t);
                }
            });
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
        .fill(t.toolbar_bg)
        .inner_margin(Margin::symmetric(12, 8))
        .stroke(Stroke::new(1.0, t.border));

    egui::Panel::top("papercraft_profile_header")
        .frame(frame)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(HEADER_ROW_HEIGHT);
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);

                let back_label = if app.view.profile_return_screen == crate::AppScreen::Editor {
                    "Back to Notebook"
                } else {
                    "Back to Library"
                };
                if icon_button(ui, Icon::PrevPage, Some(back_label), false, true, t)
                    .on_hover_text("Return (Esc)")
                    .clicked()
                {
                    crate::close_profile_screen(app);
                }

                ui.separator();

                let (badge_rect, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
                ui.painter()
                    .rect_filled(badge_rect, CornerRadius::same(7), t.accent_soft);
                paint_icon(
                    ui.painter(),
                    badge_rect.shrink(5.5),
                    Icon::Account,
                    t.accent,
                );

                ui.label(
                    egui::RichText::new("Profile & Cloud Sync")
                        .size(14.5)
                        .strong()
                        .color(t.text),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.set_height(HEADER_ROW_HEIGHT);
                    ui.spacing_mut().item_spacing.x = 6.0;

                    if compact_icon_button(
                        ui,
                        Icon::Settings,
                        Some("Settings"),
                        app.view.inspector_open,
                        true,
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

                    if summary.signed_in {
                        let in_flight = app.session.is_sync_in_flight();
                        let can_sync = !in_flight && summary.viewing_cloud;
                        let sync_lbl = if in_flight { "Syncing..." } else { "Sync Now" };
                        if primary_action_button(ui, sync_lbl, can_sync, t)
                            .on_hover_text(if summary.viewing_cloud {
                                format!("Sync cloud workspace ({cloud_pending} pending)")
                            } else {
                                "Switch to Cloud Workspace to sync".to_string()
                            })
                            .clicked()
                        {
                            app.view.account.error_message = None;
                            app.view.account.status_message = None;
                            trigger_manual_sync(app, ui.ctx());
                        }
                    }

                    if summary.viewing_cloud {
                        draw_sync_connection_pill(ui, summary.connection, t);
                    } else if summary.signed_in {
                        draw_pill_badge(
                            ui,
                            PillStyle {
                                fill: t.toolbar_bg,
                                border: t.border,
                                fg: t.text_muted,
                                icon: PillIcon::Dot,
                            },
                            "Viewing Local",
                        );
                    } else {
                        draw_pill_badge(
                            ui,
                            PillStyle {
                                fill: t.toolbar_bg,
                                border: t.border,
                                fg: t.text_muted,
                                icon: PillIcon::Dot,
                            },
                            "Local Only",
                        );
                    }
                });
            });
        });
}

fn draw_two_column_layout(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    cloud_pending: usize,
    content_w: f32,
    t: &Tokens,
) {
    let gap = 18.0_f32;
    let body_w = (content_w - RAIL_WIDTH - gap).max(280.0);
    let max_h = ui.available_height().max(180.0);

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.allocate_ui_with_layout(
            Vec2::new(RAIL_WIDTH, max_h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(RAIL_WIDTH);
                draw_sidebar_rail(app, ui, summary, t);
            },
        );

        ui.allocate_ui_with_layout(
            Vec2::new(body_w, max_h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(body_w);
                egui::ScrollArea::vertical()
                    .id_salt("papercraft_profile_body_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width().min(body_w));
                        draw_active_section_content(app, ui, summary, cloud_pending, t);
                        ui.add_space(16.0);
                    });
            },
        );
    });
}

fn draw_compact_layout(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    cloud_pending: usize,
    t: &Tokens,
) {
    if summary.signed_in {
        let options = [
            SwitchOption::new(
                Some(Icon::Account),
                Some("Profile"),
                "Profile & email",
                true,
            ),
            SwitchOption::new(
                Some(Icon::Reset),
                Some("Security"),
                "Password & sign-out",
                true,
            ),
            SwitchOption::new(Some(Icon::Sync), Some("Sync"), "Server & sync status", true),
            SwitchOption::new(
                Some(Icon::Inbox),
                Some("Workspaces"),
                "Local profiles",
                true,
            ),
        ];
        let active = match app.view.account.profile_section {
            ProfileSection::Profile => 0,
            ProfileSection::Security => 1,
            ProfileSection::SyncServer => 2,
            ProfileSection::Workspaces => 3,
        };
        let (clicked, _) = segmented_switch(
            ui,
            "profile_compact_tabs_signed_in",
            &options,
            Some(active),
            SwitchLayout::EqualWidth,
            t,
        );
        if let Some(idx) = clicked {
            app.view.account.profile_section = match idx {
                0 => ProfileSection::Profile,
                1 => ProfileSection::Security,
                2 => ProfileSection::SyncServer,
                _ => ProfileSection::Workspaces,
            };
        }
    } else {
        let options = [
            SwitchOption::new(
                Some(Icon::Server),
                Some("Cloud Account"),
                "Sign in, create account, or complete password reset",
                true,
            ),
            SwitchOption::new(
                Some(Icon::Inbox),
                Some("Local Workspaces"),
                "Manage offline local profiles",
                true,
            ),
        ];
        let active = match app.view.account.signed_out_section {
            SignedOutProfileSection::CloudAccount => 0,
            SignedOutProfileSection::LocalWorkspaces => 1,
        };
        let (clicked, _) = segmented_switch(
            ui,
            "profile_compact_tabs_signed_out",
            &options,
            Some(active),
            SwitchLayout::EqualWidth,
            t,
        );
        if let Some(idx) = clicked {
            app.view.account.signed_out_section = if idx == 0 {
                SignedOutProfileSection::CloudAccount
            } else {
                SignedOutProfileSection::LocalWorkspaces
            };
        }
    }

    ui.add_space(12.0);
    egui::ScrollArea::vertical()
        .id_salt("papercraft_profile_compact_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            draw_active_section_content(app, ui, summary, cloud_pending, t);
            ui.add_space(16.0);
        });
}

fn draw_sidebar_rail(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    t: &Tokens,
) {
    draw_identity_summary_card(ui, summary, t);
    ui.add_space(10.0);

    egui::Frame::new()
        .fill(t.toolbar_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            if summary.signed_in {
                let items = [
                    (
                        ProfileSection::Profile,
                        Icon::Account,
                        "Profile",
                        "Display name, @username & email",
                    ),
                    (
                        ProfileSection::Security,
                        Icon::Reset,
                        "Security",
                        "Password & active session",
                    ),
                    (
                        ProfileSection::SyncServer,
                        Icon::Sync,
                        "Sync & Server",
                        "Sync state & server URL",
                    ),
                    (
                        ProfileSection::Workspaces,
                        Icon::Inbox,
                        "Workspaces",
                        "Cloud & local profiles",
                    ),
                ];
                for (sec, icon, title, subtitle) in items {
                    let active = app.view.account.profile_section == sec;
                    if draw_rail_nav_button(ui, icon, title, subtitle, active, t) {
                        app.view.account.profile_section = sec;
                    }
                }
            } else {
                let items = [
                    (
                        SignedOutProfileSection::CloudAccount,
                        Icon::Server,
                        "Cloud Account",
                        "Sign in, sign up, or reset password",
                    ),
                    (
                        SignedOutProfileSection::LocalWorkspaces,
                        Icon::Inbox,
                        "Local Workspaces",
                        "Offline profiles on this device",
                    ),
                ];
                for (sec, icon, title, subtitle) in items {
                    let active = app.view.account.signed_out_section == sec;
                    if draw_rail_nav_button(ui, icon, title, subtitle, active, t) {
                        app.view.account.signed_out_section = sec;
                    }
                }
            }
        });
}

fn draw_identity_summary_card(ui: &mut egui::Ui, summary: &SyncStatusSummary, t: &Tokens) {
    egui::Frame::new()
        .fill(t.toolbar_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;

            let (primary, secondary, monogram) = if summary.signed_in {
                let disp = summary
                    .display_name
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .or(summary.username.as_deref())
                    .unwrap_or("Account")
                    .to_string();
                let handle = summary
                    .username
                    .as_deref()
                    .map_or_else(String::new, |u| format!("@{u}"));
                let mono = disp
                    .chars()
                    .find(|c| c.is_alphanumeric())
                    .map_or_else(|| "U".to_string(), |c| c.to_uppercase().collect());
                (disp, handle, mono)
            } else {
                let nick = summary
                    .active_local_profile_name
                    .clone()
                    .unwrap_or_else(|| "Personal".to_string());
                let mono = nick
                    .chars()
                    .find(|c| c.is_alphanumeric())
                    .map_or_else(|| "L".to_string(), |c| c.to_uppercase().collect());
                (nick, "Local Only Workspace".to_string(), mono)
            };

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
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
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(primary)
                                .size(13.5)
                                .strong()
                                .color(t.text),
                        )
                        .wrap(),
                    );
                    if !secondary.is_empty() {
                        ui.label(
                            egui::RichText::new(secondary)
                                .size(11.5)
                                .monospace()
                                .color(t.text_muted),
                        );
                    }
                });
            });

            if let Some(email) = summary.email.as_deref().filter(|s| !s.trim().is_empty()) {
                draw_hairline(ui, t);
                ui.add(
                    egui::Label::new(egui::RichText::new(email).size(11.0).color(t.text_muted))
                        .wrap(),
                );
            }
        });
}

fn draw_rail_nav_button(
    ui: &mut egui::Ui,
    icon: Icon,
    title: &str,
    subtitle: &str,
    selected: bool,
    t: &Tokens,
) -> bool {
    let fill = if selected {
        t.accent_soft
    } else {
        egui::Color32::TRANSPARENT
    };
    let border = if selected {
        Stroke::new(1.0, t.accent)
    } else {
        Stroke::NONE
    };
    let title_color = if selected { t.accent } else { t.text };

    let resp = egui::Frame::new()
        .fill(fill)
        .stroke(border)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                paint_icon(
                    ui.painter(),
                    ir,
                    icon,
                    if selected { t.accent } else { t.text_muted },
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(title)
                                .size(12.5)
                                .strong()
                                .color(title_color),
                        )
                        .selectable(false),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(subtitle).size(10.5).color(t.text_muted),
                        )
                        .selectable(false)
                        .wrap(),
                    );
                });
            });
        })
        .response
        .interact(Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.clicked()
}

fn draw_active_section_content(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    cloud_pending: usize,
    t: &Tokens,
) {
    ui.spacing_mut().item_spacing.y = 0.0;

    if summary.signed_in && app.view.account.logout_confirm_open {
        render_logout_safeguard_body(app, ui, t);
        ui.add_space(10.0);
        draw_section_card(ui, t, |ui| {
            render_logout_safeguard_actions(app, ui, t);
        });
        return;
    }

    if summary.signed_in {
        match app.view.account.profile_section {
            ProfileSection::Profile => {
                draw_section_card(ui, t, |ui| {
                    render_profile_identity_card(app, ui, t);
                });
                ui.add_space(10.0);
                draw_section_card(ui, t, |ui| {
                    render_account_email_card(app, ui, summary, t);
                });
            }
            ProfileSection::Security => {
                draw_section_card(ui, t, |ui| {
                    render_change_password_card(app, ui, t);
                });
                ui.add_space(10.0);
                draw_section_card(ui, t, |ui| {
                    render_session_sign_out_card(app, ui, summary, t);
                });
            }
            ProfileSection::SyncServer => {
                draw_section_card(ui, t, |ui| {
                    render_signed_in_account_card(app, ui, summary, cloud_pending, t);
                });
                if !app.session.accessible_local_profiles().is_empty() {
                    ui.add_space(10.0);
                    draw_section_card(ui, t, |ui| {
                        render_detected_local_accounts_migration_card(app, ui, summary, t);
                    });
                }
                ui.add_space(10.0);
                let mut test_on_enter = false;
                draw_section_card(ui, t, |ui| {
                    test_on_enter = render_signed_in_server_card(app, ui, summary, t);
                });
                if test_on_enter && !app.view.account.is_testing() {
                    trigger_test_connection(app, ui.ctx());
                }
            }
            ProfileSection::Workspaces => {
                draw_signed_in_workspaces_section(app, ui, summary, t);
            }
        }
    } else {
        match app.view.account.signed_out_section {
            SignedOutProfileSection::CloudAccount => {
                let mut test_on_enter = false;
                draw_section_card(ui, t, |ui| {
                    test_on_enter = render_server_probe_section(app, ui, t, true);
                });
                ui.add_space(10.0);
                let mut submit_on_enter = false;
                draw_section_card(ui, t, |ui| {
                    submit_on_enter = render_auth_card_contents(app, ui, t, true);
                });
                ui.add_space(10.0);
                draw_self_hosted_download_disclaimer(app, ui, t);
                if test_on_enter && !app.view.account.is_testing() {
                    trigger_test_connection(app, ui.ctx());
                }
                if submit_on_enter {
                    handle_auth_submit_on_enter(app, ui.ctx());
                }
            }
            SignedOutProfileSection::LocalWorkspaces => {
                draw_section_card(ui, t, |ui| {
                    let active_name = summary
                        .active_local_profile_name
                        .as_deref()
                        .unwrap_or("Personal");
                    let nb_count = app.session.notebooks.len();
                    ui.horizontal(|ui| {
                        let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                        paint_icon(ui.painter(), ir, Icon::Inbox, t.accent);
                        ui.label(
                            egui::RichText::new(format!("On This Device: {active_name}"))
                                .size(13.5)
                                .strong()
                                .color(t.text),
                        );
                    });
                    ui.add_space(3.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!(
                                "{nb_count} notebook(s) in this local workspace. Local profiles stay completely offline until you sign in or transfer notebooks."
                            ))
                            .size(11.0)
                            .color(t.text_muted),
                        )
                        .wrap(),
                    );
                    ui.add_space(10.0);
                    draw_hairline(ui, t);
                    ui.add_space(10.0);
                    render_local_profiles_manager(app, ui, t);
                });
            }
        }
    }

    if let Some(ref conflict) = summary.last_conflict_notice {
        ui.add_space(8.0);
        draw_status_banner(ui, BannerTone::Info, conflict, t);
    }
    if let Some(ref err) = summary.last_error
        && app.view.account.error_message.is_none()
        && summary.connection == SyncConnectionState::Error
    {
        ui.add_space(8.0);
        draw_status_banner(ui, BannerTone::Error, err, t);
    }
    render_auth_banners(app, ui, t);
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
                                    egui::RichText::new("github.com/alyxhard/papercraft")
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

fn draw_signed_in_workspaces_section(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    t: &Tokens,
) {
    draw_section_card(ui, t, |ui| {
        let acct_label = summary
            .email
            .clone()
            .or_else(|| summary.username.as_ref().map(|u| format!("@{u}")))
            .unwrap_or_else(|| "Cloud Account".to_string());
        ui.horizontal(|ui| {
            let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            paint_icon(ui.painter(), ir, Icon::Sync, t.accent);
            ui.label(
                egui::RichText::new(format!("Cloud Workspace · {acct_label}"))
                    .size(13.5)
                    .strong()
                    .color(t.text),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if summary.viewing_cloud {
                    draw_pill_badge(
                        ui,
                        PillStyle {
                            fill: t.success_bg,
                            border: t.success_border,
                            fg: t.success_fg,
                            icon: PillIcon::GreenCheck(t),
                        },
                        "Active",
                    );
                } else if secondary_action_button(ui, "Switch to Cloud", true, t).clicked() {
                    crate::canvas::commit_pending_text(app);
                    if let Err(err) = app
                        .session
                        .execute("profile.switch", &serde_json::json!({ "target": "cloud" }))
                    {
                        app.view.account.error_message = Some(err.to_string());
                    } else {
                        app.view.account.error_message = None;
                        app.view.profile_return_screen = crate::AppScreen::Home;
                        crate::sync_view_to_active_notebook(app);
                    }
                }
            });
        });
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "Switch between Cloud Sync and isolated Local Only profiles on this computer.",
                )
                .size(11.0)
                .color(t.text_muted),
            )
            .wrap(),
        );
        ui.add_space(10.0);
        draw_hairline(ui, t);
        ui.add_space(10.0);
        render_local_profiles_manager(app, ui, t);
    });

    if !app.session.accessible_local_profiles().is_empty() {
        ui.add_space(10.0);
        draw_section_card(ui, t, |ui| {
            render_detected_local_accounts_migration_card(app, ui, summary, t);
        });
    }

    ui.add_space(10.0);
    draw_section_card(ui, t, |ui| {
        app.view.account.import_local_open = true;
        render_signed_in_transfer_section(app, ui, t);
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::AppScreen;
    use crate::account_modal::AuthTab;
    use crate::sync_worker::{SyncWorkerEvent, apply_worker_event};
    use egui::{Shape, epaint::ClippedShape};
    use papercraft_engine::AuthWorkspaceMode;
    use papercraft_sync::ActiveAccountSession;
    use papercraft_sync_protocol::{DEFAULT_SERVER_URL, UserProfile};

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

        app.view.account.tab = AuthTab::CreateAccount;
        let signup_text = render_profile_text(&ctx, &mut app);
        assert!(signup_text.contains("Profile & Cloud Sync"));
        assert!(signup_text.contains("Username"));
        assert!(signup_text.contains("Display Name"));

        app.view.account.tab = AuthTab::SignIn;
        apply_worker_event(
            &mut app,
            &ctx,
            SyncWorkerEvent::PasswordResetRequired {
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
        app.session.switch_to_authenticated_library(
            ActiveAccountSession {
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
            },
            AuthWorkspaceMode::SwitchOnly,
        );
        crate::open_profile_screen(&mut app);

        app.view.account.profile_section = ProfileSection::Profile;
        let profile_text = render_profile_text(&ctx, &mut app);
        assert!(profile_text.contains("Profile Details"));
        assert!(profile_text.contains("Save Profile"));
        assert!(profile_text.contains("Email Address"));
        assert!(profile_text.contains("@alex_notes"));

        app.view.account.profile_section = ProfileSection::Security;
        let _ = app
            .session
            .execute("theme.set", &serde_json::json!({ "theme": "Dark" }));
        let sec_text = render_profile_text(&ctx, &mut app);
        assert!(sec_text.contains("Change Password"));
        assert!(sec_text.contains("Update Password"));
        assert!(sec_text.contains("Session & Sign Out"));

        app.view.account.profile_section = ProfileSection::SyncServer;
        let sync_text = render_profile_text(&ctx, &mut app);
        assert!(sync_text.contains("Server Address"));
        assert!(sync_text.contains("PENDING UPLOADS"));

        app.view.account.profile_section = ProfileSection::Workspaces;
        let ws_text = render_profile_text(&ctx, &mut app);
        assert!(ws_text.contains("Cloud Workspace"));

        crate::close_profile_screen(&mut app);
        assert_eq!(app.view.screen, AppScreen::Home);
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
        assert!(light_text.contains("github.com/alyxhard/papercraft"));

        let link_pos = find_text_center(&shapes, "github.com/alyxhard/papercraft")
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
            vec!["https://github.com/alyxhard/papercraft".to_string()]
        );

        let _ = app
            .session
            .execute("theme.set", &serde_json::json!({ "theme": "Dark" }));
        let (dark_text, _) = run_settled(&ctx, &mut app, &mut time);
        assert!(dark_text.contains("PaperCraft Server"));
        assert!(dark_text.contains("standalone local desktop app"));
        assert!(dark_text.contains("github.com/alyxhard/papercraft"));
    }
}
