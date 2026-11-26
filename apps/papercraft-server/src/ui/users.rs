//! Registered Users and 30-day Server Trash administration views.

use egui::{Color32, CornerRadius, Margin, Sense, Stroke, Vec2};

use crate::controller::{
    NoticeBanner, NoticeTone, ServerController, account_delete_confirmation_matches,
    account_delete_confirmation_phrase,
};
use crate::icons::{Icon, icon_button, icon_button_width, paint_icon};
use crate::theme::Tokens;
use crate::ui::USER_TABLE_BREAKPOINT;
use crate::ui::widgets::{
    CARD_RADIUS, CONTROL_RADIUS, INPUT_HEIGHT, ManualConfirmInputSpec, PageHeaderBadge,
    danger_button, danger_labeled_button, danger_labeled_button_width, draw_banner,
    draw_compact_pill, draw_hairline, draw_interactive_pill, draw_manual_confirm_input,
    draw_page_header, draw_soft_strip, format_trash_remaining, format_utc_date, primary_button,
    secondary_button, short_user_id, user_monogram,
};
use papercraft_server_cli::db::{
    ACCOUNT_TRASH_RETENTION_DAYS, PasswordResetOutcome, TrashedUserSummary, UserSummary,
};

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

struct ActiveTableColumns {
    account_w: f32,
    joined_w: f32,
    id_w: f32,
    notebooks_w: f32,
    actions_w: f32,
    show_full_id: bool,
    labeled_copy_btn: bool,
}

impl ActiveTableColumns {
    fn from_width(table_w: f32, ui: &egui::Ui) -> Self {
        let inner_w = (table_w - 32.0).max(200.0);
        let col_gap = 12.0_f32;
        let labeled_copy_btn = table_w >= 740.0;
        let copy_w = if labeled_copy_btn {
            icon_button_width(ui, Some("Copy ID"))
        } else {
            28.0
        };
        let reset_w = icon_button_width(ui, Some("Reset"));
        let trash_w = danger_labeled_button_width(ui, "Trash");
        let actions_w = copy_w + reset_w + trash_w + 12.0;
        let notebooks_w = 100.0_f32;
        let joined_w = 88.0_f32;
        let show_full_id = table_w >= 930.0;
        let id_w = if show_full_id {
            264.0
        } else if table_w >= 840.0 {
            148.0
        } else {
            92.0
        };
        let account_w =
            (inner_w - actions_w - notebooks_w - joined_w - id_w - col_gap * 4.0).max(160.0);
        Self {
            account_w,
            joined_w,
            id_w,
            notebooks_w,
            actions_w,
            show_full_id,
            labeled_copy_btn,
        }
    }
}

struct TrashTableColumns {
    account_w: f32,
    deleted_w: f32,
    expires_w: f32,
    id_w: Option<f32>,
    notebooks_w: f32,
    action_w: f32,
}

impl TrashTableColumns {
    fn from_width(table_w: f32) -> Self {
        let inner_w = (table_w - 32.0).max(200.0);
        let col_gap = 12.0_f32;
        let deleted_w = 90.0_f32;
        let expires_w = if table_w >= 820.0 { 176.0 } else { 150.0 };
        let id_w = if table_w >= 1040.0 {
            Some(264.0)
        } else if table_w >= 860.0 {
            Some(144.0)
        } else {
            None
        };
        let notebooks_w = 100.0_f32;
        let action_w = 106.0_f32;
        let gap_count = if id_w.is_some() { 5.0 } else { 4.0 };
        let fixed_w = deleted_w
            + expires_w
            + id_w.unwrap_or(0.0)
            + notebooks_w
            + action_w
            + col_gap * gap_count;
        let account_w = (inner_w - fixed_w).max(150.0);
        Self {
            account_w,
            deleted_w,
            expires_w,
            id_w,
            notebooks_w,
            action_w,
        }
    }
}

pub fn render_users_page(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    let total = ctrl.users_snapshot.len();
    let count_text = if total == 1 {
        "1 active".to_string()
    } else {
        format!("{total} active")
    };
    let folder_name = storage_folder_name(ctrl);
    let subtitle = format!("Accounts stored in `{folder_name}` (papercraft.redb).");

    draw_page_header(
        ui,
        "Registered Users",
        Some(PageHeaderBadge {
            label: &count_text,
            fill: t.toolbar_bg,
            border: t.border,
            fg: t.text_muted,
        }),
        &subtitle,
        t,
        |ui| {
            let (reg_icon, reg_lbl, reg_fill, reg_border, reg_fg, reg_tip) =
                if ctrl.draft_settings.allow_registration {
                    (
                        Icon::LockOpen,
                        "New sign-ups open",
                        t.success_bg,
                        t.success_border,
                        t.success_fg,
                        "New account sign-ups are open. Click to close registrations.",
                    )
                } else {
                    (
                        Icon::Lock,
                        "New sign-ups closed",
                        t.warning_bg,
                        t.warning_border,
                        t.warning_fg,
                        "New account sign-ups are closed. Click to allow new sign-ups.",
                    )
                };
            if draw_interactive_pill(
                ui,
                Some(reg_icon),
                reg_lbl,
                reg_fill,
                reg_border,
                reg_fg,
                reg_tip,
            )
            .clicked()
            {
                ctrl.draft_settings.allow_registration = !ctrl.draft_settings.allow_registration;
            }

            if icon_button(ui, Icon::RotateCw, Some("Refresh"), false, true, t)
                .on_hover_text("Reload active and trashed accounts from the database")
                .clicked()
            {
                ctrl.refresh_users();
                if ctrl.is_running() {
                    ctrl.refresh_stats();
                }
            }
        },
    );

    ui.add_space(12.0);

    if let Some(outcome) = ctrl.last_password_reset.clone() {
        render_password_reset_card(ctrl, &outcome, ui, t);
        ui.add_space(12.0);
    }

    if let Some(ref err) = ctrl.users_error {
        draw_table_container(ui, t, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(16, 14))
                .show(ui, |ui| {
                    draw_banner(
                        ui,
                        &NoticeBanner {
                            tone: NoticeTone::Error,
                            text: err.clone(),
                        },
                        false,
                        t,
                    );
                });
        });
        return;
    }

    if ctrl.users_snapshot.is_empty() {
        draw_table_container(ui, t, |ui| {
            render_users_empty_state(ctrl.draft_settings.allow_registration, ui, t);
        });
        return;
    }

    let matches = filter_active_users(ctrl);
    let total_users = ctrl.users_snapshot.len();
    let total_notebooks: usize = ctrl.users_snapshot.iter().map(|u| u.notebook_count).sum();
    let summary_text = format_active_summary(matches.len(), total_users, total_notebooks);

    render_search_and_summary_bar(
        ctrl,
        "Search active accounts by name, email, @username, or ID...",
        &summary_text,
        ui,
        t,
    );
    ui.add_space(10.0);

    draw_table_container(ui, t, |ui| {
        if matches.is_empty() {
            render_search_no_matches_box(&ctrl.user_search, "accounts", ui, t);
            return;
        }
        render_active_users_rows(ctrl, &matches, ui, t);
    });

    if ui.available_width() < 640.0 {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(summary_text)
                .size(11.0)
                .color(t.text_muted),
        );
    }
}

pub fn render_trash_page(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    let trashed_total = ctrl.trashed_users_snapshot.len();
    let count_text = format!("{trashed_total} in Trash");
    let (badge_fill, badge_border, badge_fg) = if trashed_total > 0 {
        (t.danger_bg, t.danger_border, t.danger_fg)
    } else {
        (t.toolbar_bg, t.border, t.text_muted)
    };

    draw_page_header(
        ui,
        "Server Trash",
        Some(PageHeaderBadge {
            label: &count_text,
            fill: badge_fill,
            border: badge_border,
            fg: badge_fg,
        }),
        "Deleted accounts awaiting automatic expiration or administrator recovery.",
        t,
        |ui| {
            if icon_button(ui, Icon::RotateCw, Some("Refresh"), false, true, t)
                .on_hover_text("Reload active and trashed accounts from the database")
                .clicked()
            {
                ctrl.refresh_users();
                if ctrl.is_running() {
                    ctrl.refresh_stats();
                }
            }
        },
    );

    ui.add_space(10.0);
    render_retention_rule_strip(trashed_total > 0, ui, t);
    ui.add_space(12.0);

    if let Some(ref err) = ctrl.users_error {
        draw_table_container(ui, t, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(16, 14))
                .show(ui, |ui| {
                    draw_banner(
                        ui,
                        &NoticeBanner {
                            tone: NoticeTone::Error,
                            text: err.clone(),
                        },
                        false,
                        t,
                    );
                });
        });
        return;
    }

    if ctrl.trashed_users_snapshot.is_empty() {
        draw_table_container(ui, t, |ui| {
            render_trash_empty_state(ui, t);
        });
        return;
    }

    let matches = filter_trashed_users(ctrl);
    let total_notebooks: usize = ctrl
        .trashed_users_snapshot
        .iter()
        .map(|u| u.notebook_count)
        .sum();
    let summary_text = format_trash_summary(matches.len(), trashed_total, total_notebooks);

    render_search_and_summary_bar(
        ctrl,
        "Search trashed accounts by name, email, @username, or ID...",
        &summary_text,
        ui,
        t,
    );
    ui.add_space(10.0);

    draw_table_container(ui, t, |ui| {
        if matches.is_empty() {
            render_search_no_matches_box(&ctrl.user_search, "trashed accounts", ui, t);
            return;
        }
        render_trashed_users_rows(ctrl, &matches, ui, t);
    });

    if ui.available_width() < 640.0 {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(summary_text)
                .size(11.0)
                .color(t.text_muted),
        );
    }
}

fn storage_folder_name(ctrl: &ServerController) -> String {
    let data_path = ctrl.draft_settings.resolved_data_path();
    data_path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .map_or_else(|| data_path.display().to_string(), ToString::to_string)
}

fn filter_active_users(ctrl: &ServerController) -> Vec<usize> {
    let query = ctrl.user_search.trim().to_ascii_lowercase();
    ctrl.users_snapshot
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
        .collect()
}

fn filter_trashed_users(ctrl: &ServerController) -> Vec<usize> {
    let query = ctrl.user_search.trim().to_ascii_lowercase();
    ctrl.trashed_users_snapshot
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
        .collect()
}

fn format_active_summary(shown: usize, total_users: usize, total_notebooks: usize) -> String {
    let nb_label = if total_notebooks == 1 {
        "notebook"
    } else {
        "notebooks"
    };
    if shown == total_users {
        format!("{total_users} active · {total_notebooks} {nb_label} total")
    } else {
        format!("Showing {shown} of {total_users} active · {total_notebooks} {nb_label} total")
    }
}

fn format_trash_summary(shown: usize, total_trashed: usize, total_notebooks: usize) -> String {
    let nb_label = if total_notebooks == 1 {
        "notebook"
    } else {
        "notebooks"
    };
    if shown == total_trashed {
        format!(
            "{total_trashed} in Trash · {total_notebooks} {nb_label} kept ({ACCOUNT_TRASH_RETENTION_DAYS}-day retention)"
        )
    } else {
        format!("Showing {shown} of {total_trashed} in Trash · {total_notebooks} {nb_label} kept")
    }
}

fn draw_table_container(ui: &mut egui::Ui, t: &Tokens, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(t.panel_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CARD_RADIUS)
        .inner_margin(Margin::ZERO)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            body(ui);
        });
}

fn render_retention_rule_strip(has_items: bool, ui: &mut egui::Ui, t: &Tokens) {
    let (fill, border, icon_fg) = if has_items {
        (t.warning_bg, t.warning_border, t.warning_fg)
    } else {
        (t.toolbar_bg, t.border, t.text_muted)
    };
    draw_soft_strip(
        ui,
        fill,
        Stroke::new(1.0, border),
        Margin::symmetric(12, 9),
        |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
                paint_icon(ui.painter(), ir, Icon::Info, icon_fg);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!(
                            "{ACCOUNT_TRASH_RETENTION_DAYS}-day retention: Deleted accounts and their synced notebooks remain in server Trash for {ACCOUNT_TRASH_RETENTION_DAYS} days before automatic permanent deletion. Restore an account anytime before its expiration date."
                        ))
                        .size(11.5)
                        .color(t.text),
                    )
                    .wrap(),
                );
            });
        },
    );
}

fn render_search_and_summary_bar(
    ctrl: &mut ServerController,
    hint: &str,
    summary_text: &str,
    ui: &mut egui::Ui,
    t: &Tokens,
) {
    let avail_w = ui.available_width();
    let show_summary_inline = avail_w >= 640.0;
    let has_query = !ctrl.user_search.is_empty();
    let clear_w = if has_query { 34.0 } else { 0.0 };
    let search_cap = if show_summary_inline {
        (avail_w - 220.0).clamp(180.0, 420.0)
    } else {
        avail_w
    };
    let box_w = (search_cap - clear_w).max(140.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        egui::Frame::new()
            .fill(t.field_bg)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CONTROL_RADIUS)
            .inner_margin(Margin::symmetric(9, 0))
            .show(ui, |ui| {
                ui.set_width((box_w - 18.0).max(60.0));
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

        if show_summary_inline {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(summary_text)
                        .size(11.5)
                        .color(t.text_muted),
                );
            });
        }
    });
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
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;

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
                    .size(11.5)
                    .color(t.text_muted),
                )
                .wrap(),
            );

            let copied = ctrl.was_recently_copied(&outcome.temporary_password);
            draw_soft_strip(ui, t.field_bg, Stroke::NONE, Margin::symmetric(12, 8), |ui| {
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

fn render_active_users_rows(
    ctrl: &mut ServerController,
    matches: &[usize],
    ui: &mut egui::Ui,
    t: &Tokens,
) {
    enum UserRowAction {
        CopyId(String),
        RequestReset(String),
        ConfirmReset(String),
        RequestDelete(String),
        ConfirmDelete(String),
        Cancel,
    }

    let table_w = ui.available_width();
    let wide_table = table_w >= USER_TABLE_BREAKPOINT;
    let cols = ActiveTableColumns::from_width(table_w, ui);

    if wide_table {
        render_active_table_header(&cols, ui, t);
    }

    let mut action: Option<UserRowAction> = None;
    for (row_idx, &idx) in matches.iter().enumerate() {
        if row_idx > 0 {
            draw_hairline(ui, t);
        }
        let user = ctrl.users_snapshot[idx].clone();
        let copied = ctrl.was_recently_copied(&user.user_id);
        let confirm_reset = ctrl.pending_reset_user.as_deref() == Some(user.user_id.as_str());
        let confirm_delete = ctrl.pending_delete_user.as_deref() == Some(user.user_id.as_str());
        let mut confirm_draft = DeleteConfirmDraft {
            input: &mut ctrl.delete_confirm_input,
            paste_blocked: &mut ctrl.delete_confirm_paste_blocked,
            focus_pending: &mut ctrl.delete_confirm_focus_pending,
        };

        let click = if wide_table {
            render_active_table_row(&user, copied, confirm_reset, confirm_delete, &cols, ui, t)
        } else {
            render_active_compact_row(&user, copied, confirm_reset, confirm_delete, ui, t)
        };

        let drawer_click = if confirm_reset {
            render_reset_confirmation_drawer(ui, t)
        } else if confirm_delete {
            render_delete_confirmation_drawer(&user, &mut confirm_draft, ui, t)
        } else {
            RowClick::None
        };

        let effective = match click {
            RowClick::None => drawer_click,
            other => other,
        };
        match effective {
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
}

fn render_active_table_header(cols: &ActiveTableColumns, ui: &mut egui::Ui, t: &Tokens) {
    egui::Frame::new()
        .fill(t.toolbar_bg)
        .corner_radius(CornerRadius {
            nw: 9,
            ne: 9,
            sw: 0,
            se: 0,
        })
        .inner_margin(Margin::symmetric(16, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                draw_col_header(ui, cols.account_w, "ACCOUNT", false, t);
                draw_col_header(ui, cols.joined_w, "JOINED", false, t);
                draw_col_header(ui, cols.id_w, "USER ID", false, t);
                draw_col_header(ui, cols.notebooks_w, "NOTEBOOKS", false, t);
                draw_col_header(ui, cols.actions_w, "ACTIONS", true, t);
            });
        });
    draw_hairline(ui, t);
}

fn render_active_table_row(
    user: &UserSummary,
    copied: bool,
    confirm_reset: bool,
    confirm_delete: bool,
    cols: &ActiveTableColumns,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> RowClick {
    let mut row_click = RowClick::None;
    let row_fill = if confirm_delete {
        t.danger_bg.gamma_multiply(0.45)
    } else if confirm_reset {
        t.accent_soft.gamma_multiply(0.45)
    } else {
        Color32::TRANSPARENT
    };

    egui::Frame::new()
        .fill(row_fill)
        .inner_margin(Margin::symmetric(16, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;

                ui.allocate_ui_with_layout(
                    Vec2::new(cols.account_w, 0.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_width(cols.account_w);
                        ui.spacing_mut().item_spacing.x = 10.0;
                        draw_user_avatar(ui, &user.display_name, &user.username, false, t);
                        let info_w = (cols.account_w - 42.0).max(80.0);
                        draw_active_identity_block(ui, user, info_w, t);
                    },
                );

                let joined = format_utc_date(user.created_at_ms);
                ui.allocate_ui_with_layout(
                    Vec2::new(cols.joined_w, 28.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_width(cols.joined_w);
                        ui.label(egui::RichText::new(joined).size(12.0).color(t.text_muted));
                    },
                );

                ui.allocate_ui_with_layout(
                    Vec2::new(cols.id_w, 28.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_width(cols.id_w);
                        ui.set_max_width(cols.id_w);
                        let displayed = if cols.show_full_id || cols.id_w >= 130.0 {
                            user.user_id.clone()
                        } else {
                            short_user_id(&user.user_id)
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(displayed)
                                    .size(11.0)
                                    .monospace()
                                    .color(t.text_muted),
                            )
                            .truncate(),
                        )
                        .on_hover_text(format!("User ID: {}", user.user_id));
                    },
                );

                let nb_text = format_notebook_count(user.notebook_count);
                ui.allocate_ui_with_layout(
                    Vec2::new(cols.notebooks_w, 28.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_width(cols.notebooks_w);
                        draw_compact_pill(
                            ui,
                            Some(Icon::Notebook),
                            &nb_text,
                            t.toolbar_bg,
                            t.border,
                            t.text,
                        );
                    },
                );

                ui.allocate_ui_with_layout(
                    Vec2::new(cols.actions_w, 28.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.set_width(cols.actions_w);
                        ui.spacing_mut().item_spacing.x = 6.0;
                        row_click = draw_active_action_buttons(
                            ui,
                            copied,
                            confirm_reset,
                            confirm_delete,
                            cols.labeled_copy_btn,
                            t,
                        );
                    },
                );
            });
        });
    row_click
}

fn render_active_compact_row(
    user: &UserSummary,
    copied: bool,
    confirm_reset: bool,
    confirm_delete: bool,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> RowClick {
    let mut row_click = RowClick::None;
    let row_fill = if confirm_delete {
        t.danger_bg.gamma_multiply(0.45)
    } else if confirm_reset {
        t.accent_soft.gamma_multiply(0.45)
    } else {
        Color32::TRANSPARENT
    };

    egui::Frame::new()
        .fill(row_fill)
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;

            let nb_text = format_notebook_count(user.notebook_count);
            let nb_pill_w = crate::ui::widgets::measure_compact_pill_width(ui, true, &nb_text);
            let left_w = (ui.available_width() - 42.0 - nb_pill_w - 14.0).max(90.0);

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                draw_user_avatar(ui, &user.display_name, &user.username, false, t);
                draw_active_identity_block(ui, user, left_w, t);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    draw_compact_pill(
                        ui,
                        Some(Icon::Notebook),
                        &nb_text,
                        t.toolbar_bg,
                        t.border,
                        t.text,
                    );
                });
            });

            let joined = format_utc_date(user.created_at_ms);
            let short_id = short_user_id(&user.user_id);
            let meta_line = format!("Joined {joined} · ID {short_id}");
            ui.add(
                egui::Label::new(
                    egui::RichText::new(meta_line)
                        .size(11.0)
                        .color(t.text_muted),
                )
                .wrap(),
            )
            .on_hover_text(format!("User ID: {}", user.user_id));

            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    row_click = draw_active_action_buttons(
                        ui,
                        copied,
                        confirm_reset,
                        confirm_delete,
                        true,
                        t,
                    );
                });
            });
        });

    row_click
}

fn draw_active_action_buttons(
    ui: &mut egui::Ui,
    copied: bool,
    confirm_reset: bool,
    confirm_delete: bool,
    labeled_copy: bool,
    t: &Tokens,
) -> RowClick {
    let mut row_click = RowClick::None;

    if danger_labeled_button(ui, Icon::Trash, "Trash", confirm_delete, t)
        .on_hover_text("Move this user account to 30-day server Trash")
        .clicked()
    {
        row_click = if confirm_delete {
            RowClick::Cancel
        } else {
            RowClick::RequestDelete
        };
    }

    if icon_button(ui, Icon::Reset, Some("Reset"), confirm_reset, true, t)
        .on_hover_text("Reset password and generate a random recovery code")
        .clicked()
    {
        row_click = if confirm_reset {
            RowClick::Cancel
        } else {
            RowClick::RequestReset
        };
    }

    let (copy_icon, copy_lbl, copy_tip) = if copied {
        (
            Icon::Check,
            labeled_copy.then_some("Copied"),
            "User ID copied",
        )
    } else {
        (
            Icon::Copy,
            labeled_copy.then_some("Copy ID"),
            "Copy full user ID",
        )
    };
    if icon_button(ui, copy_icon, copy_lbl, copied, true, t)
        .on_hover_text(copy_tip)
        .clicked()
    {
        row_click = RowClick::CopyId;
    }

    row_click
}

fn render_reset_confirmation_drawer(ui: &mut egui::Ui, t: &Tokens) -> RowClick {
    let mut click = RowClick::None;
    egui::Frame::new()
        .fill(t.accent_soft)
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "Generate a random one-time recovery code? Their current password will stop working and active devices will be signed out until they log in with the code and set a new password.",
                    )
                    .size(11.5)
                    .color(t.text),
                )
                .wrap(),
            );
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if primary_button(ui, Icon::Reset, "Generate Reset Code", true, t).clicked() {
                        click = RowClick::ConfirmReset;
                    }
                    if secondary_button(ui, "Cancel", true, t).clicked() {
                        click = RowClick::Cancel;
                    }
                });
            });
        });
    click
}

fn render_delete_confirmation_drawer(
    user: &UserSummary,
    confirm_draft: &mut DeleteConfirmDraft<'_>,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> RowClick {
    let mut click = RowClick::None;
    let account_lbl = user
        .email
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map_or_else(|| format!("@{}", user.username), ToString::to_string);
    let expected = account_delete_confirmation_phrase(&user.username);
    let matched = account_delete_confirmation_matches(confirm_draft.input, &expected);

    let outer = egui::Frame::new()
        .fill(t.danger_bg)
        .inner_margin(Margin::symmetric(18, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
                paint_icon(ui.painter(), ir, Icon::AlertCircle, t.danger_fg);
                ui.label(
                    egui::RichText::new("Confirm Account Deletion")
                        .size(12.5)
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
                    .size(11.5)
                    .color(t.text_muted),
                )
                .wrap(),
            );

            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(
                    egui::RichText::new("Type this confirmation manually:")
                        .size(11.0)
                        .color(t.text_muted),
                );
                egui::Frame::new()
                    .fill(t.field_bg)
                    .stroke(Stroke::new(1.0, t.border))
                    .corner_radius(CornerRadius::same(5))
                    .inner_margin(Margin::symmetric(8, 3))
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&expected)
                                    .size(11.5)
                                    .strong()
                                    .monospace()
                                    .color(t.text),
                            )
                            .selectable(false),
                        );
                    });
            });

            let (hint_text, hint_color) = if *confirm_draft.paste_blocked && !matched {
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
                    ui.label(egui::RichText::new(hint_text).size(11.0).color(hint_color));
                });
            });

            let input_border = if matched {
                t.success_border
            } else {
                t.danger_border
            };
            let input_res = draw_manual_confirm_input(
                ui,
                &ManualConfirmInputSpec {
                    id_salt: "bnc_srv_delete_user_confirm",
                    width: ui.available_width().min(460.0),
                    hint: &expected,
                    border_color: input_border,
                    matched,
                },
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
                click = RowClick::Cancel;
            }

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if secondary_button(ui, "Cancel", true, t).clicked() {
                    click = RowClick::Cancel;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if danger_button(ui, Icon::Trash, "Move to Trash", matched, t).clicked()
                        || (input_res.enter_pressed && matched)
                    {
                        click = RowClick::ConfirmDelete;
                    }
                });
            });
        });

    let bar_rect = egui::Rect::from_min_max(
        outer.response.rect.left_top(),
        egui::pos2(
            outer.response.rect.left() + 3.0,
            outer.response.rect.bottom(),
        ),
    );
    ui.painter()
        .rect_filled(bar_rect, CornerRadius::ZERO, t.danger_fg);

    click
}

fn render_trashed_users_rows(
    ctrl: &mut ServerController,
    matches: &[usize],
    ui: &mut egui::Ui,
    t: &Tokens,
) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    let table_w = ui.available_width();
    let wide_table = table_w >= USER_TABLE_BREAKPOINT;
    let cols = TrashTableColumns::from_width(table_w);

    if wide_table {
        render_trash_table_header(&cols, ui, t);
    }

    let mut restore_target: Option<String> = None;
    for (row_idx, &idx) in matches.iter().enumerate() {
        if row_idx > 0 {
            draw_hairline(ui, t);
        }
        let user = &ctrl.trashed_users_snapshot[idx];
        let clicked = if wide_table {
            render_trashed_table_row(user, now_ms, &cols, ui, t)
        } else {
            render_trashed_compact_row(user, now_ms, ui, t)
        };
        if clicked {
            restore_target = Some(user.user_id.clone());
        }
    }

    if let Some(user_id) = restore_target {
        ctrl.restore_trashed_user(&user_id);
    }
}

fn render_trash_table_header(cols: &TrashTableColumns, ui: &mut egui::Ui, t: &Tokens) {
    egui::Frame::new()
        .fill(t.toolbar_bg)
        .corner_radius(CornerRadius {
            nw: 9,
            ne: 9,
            sw: 0,
            se: 0,
        })
        .inner_margin(Margin::symmetric(16, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                draw_col_header(ui, cols.account_w, "ACCOUNT", false, t);
                draw_col_header(ui, cols.deleted_w, "DELETED", false, t);
                draw_col_header(ui, cols.expires_w, "EXPIRES", false, t);
                if let Some(id_w) = cols.id_w {
                    draw_col_header(ui, id_w, "USER ID", false, t);
                }
                draw_col_header(ui, cols.notebooks_w, "NOTEBOOKS", false, t);
                draw_col_header(ui, cols.action_w, "ACTION", true, t);
            });
        });
    draw_hairline(ui, t);
}

fn render_trashed_table_row(
    user: &TrashedUserSummary,
    now_ms: i64,
    cols: &TrashTableColumns,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let mut restore_clicked = false;
    let (expiry_label, urgent) = format_trash_remaining(user.expires_at_ms, now_ms);
    let deleted_date = format_utc_date(user.deleted_at_ms);
    let expires_date = format_utc_date(user.expires_at_ms);
    let nb_text = format_notebook_count(user.notebook_count);

    egui::Frame::new()
        .inner_margin(Margin::symmetric(16, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;

                ui.allocate_ui_with_layout(
                    Vec2::new(cols.account_w, 0.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_width(cols.account_w);
                        ui.spacing_mut().item_spacing.x = 10.0;
                        draw_user_avatar(ui, &user.display_name, &user.username, true, t);
                        let info_w = (cols.account_w - 42.0).max(80.0);
                        draw_trashed_identity_block(ui, user, info_w, cols.id_w.is_none(), t);
                    },
                );

                ui.allocate_ui_with_layout(
                    Vec2::new(cols.deleted_w, 34.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_width(cols.deleted_w);
                        ui.label(
                            egui::RichText::new(deleted_date)
                                .size(12.0)
                                .color(t.text_muted),
                        );
                    },
                );

                ui.allocate_ui_with_layout(
                    Vec2::new(cols.expires_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(cols.expires_w);
                        ui.spacing_mut().item_spacing.y = 3.0;
                        ui.label(
                            egui::RichText::new(expires_date)
                                .size(12.0)
                                .strong()
                                .color(t.text),
                        );
                        let (pill_fill, pill_border, pill_fg) = if urgent {
                            (t.danger_bg, t.danger_border, t.danger_fg)
                        } else {
                            (t.warning_bg, t.warning_border, t.warning_fg)
                        };
                        draw_compact_pill(ui, None, &expiry_label, pill_fill, pill_border, pill_fg);
                    },
                );

                if let Some(id_w) = cols.id_w {
                    ui.allocate_ui_with_layout(
                        Vec2::new(id_w, 34.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_width(id_w);
                            ui.set_max_width(id_w);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&user.user_id)
                                        .size(11.0)
                                        .monospace()
                                        .color(t.text_muted),
                                )
                                .truncate(),
                            )
                            .on_hover_text(format!("User ID: {}", user.user_id));
                        },
                    );
                }

                ui.allocate_ui_with_layout(
                    Vec2::new(cols.notebooks_w, 34.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_width(cols.notebooks_w);
                        draw_compact_pill(
                            ui,
                            Some(Icon::Notebook),
                            &nb_text,
                            t.toolbar_bg,
                            t.border,
                            t.text,
                        );
                    },
                );

                ui.allocate_ui_with_layout(
                    Vec2::new(cols.action_w, 34.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.set_width(cols.action_w);
                        if primary_button(ui, Icon::Reset, "Restore", true, t)
                            .on_hover_text(
                                "Restore this account and all synced notebooks from Trash",
                            )
                            .clicked()
                        {
                            restore_clicked = true;
                        }
                    },
                );
            });
        });

    restore_clicked
}

fn render_trashed_compact_row(
    user: &TrashedUserSummary,
    now_ms: i64,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let mut restore_clicked = false;
    let (expiry_label, urgent) = format_trash_remaining(user.expires_at_ms, now_ms);
    let deleted_date = format_utc_date(user.deleted_at_ms);
    let expires_date = format_utc_date(user.expires_at_ms);
    let nb_text = format_notebook_count(user.notebook_count);

    egui::Frame::new()
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                draw_user_avatar(ui, &user.display_name, &user.username, true, t);
                let pill_w =
                    crate::ui::widgets::measure_compact_pill_width(ui, false, &expiry_label);
                let left_w = (ui.available_width() - 42.0 - pill_w - 14.0).max(90.0);
                draw_trashed_identity_block(ui, user, left_w, false, t);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (pill_fill, pill_border, pill_fg) = if urgent {
                        (t.danger_bg, t.danger_border, t.danger_fg)
                    } else {
                        (t.warning_bg, t.warning_border, t.warning_fg)
                    };
                    draw_compact_pill(ui, None, &expiry_label, pill_fill, pill_border, pill_fg);
                });
            });

            let short_id = short_user_id(&user.user_id);
            let meta = format!("Deleted {deleted_date} · Expires {expires_date} · ID {short_id}");
            ui.add(
                egui::Label::new(egui::RichText::new(meta).size(11.0).color(t.text_muted)).wrap(),
            )
            .on_hover_text(format!("User ID: {}", user.user_id));

            ui.horizontal(|ui| {
                draw_compact_pill(
                    ui,
                    Some(Icon::Notebook),
                    &nb_text,
                    t.toolbar_bg,
                    t.border,
                    t.text,
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, Icon::Reset, "Restore", true, t)
                        .on_hover_text("Restore this account and all synced notebooks from Trash")
                        .clicked()
                    {
                        restore_clicked = true;
                    }
                });
            });
        });

    restore_clicked
}

fn draw_col_header(ui: &mut egui::Ui, width: f32, label: &str, right_align: bool, t: &Tokens) {
    let layout = if right_align {
        egui::Layout::right_to_left(egui::Align::Center)
    } else {
        egui::Layout::left_to_right(egui::Align::Center)
    };
    ui.allocate_ui_with_layout(Vec2::new(width, 16.0), layout, |ui| {
        ui.set_width(width);
        ui.label(
            egui::RichText::new(label)
                .size(10.5)
                .strong()
                .color(t.text_muted),
        );
    });
}

fn draw_user_avatar(
    ui: &mut egui::Ui,
    display_name: &str,
    username: &str,
    trashed: bool,
    t: &Tokens,
) {
    let (avatar_rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
    let (fill, stroke, fg) = if trashed {
        (t.danger_bg, t.danger_border, t.danger_fg)
    } else {
        (t.accent_soft, t.accent.gamma_multiply(0.35), t.accent)
    };
    ui.painter().circle_filled(avatar_rect.center(), 16.0, fill);
    ui.painter()
        .circle_stroke(avatar_rect.center(), 16.0, Stroke::new(1.0, stroke));
    let initial = user_monogram(display_name, username);
    ui.painter().text(
        avatar_rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        egui::FontId::proportional(13.0),
        fg,
    );
}

fn draw_active_identity_block(ui: &mut egui::Ui, user: &UserSummary, width: f32, t: &Tokens) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing.y = 2.0;

            draw_name_and_handle_row(
                ui,
                &user.display_name,
                &user.username,
                user.must_change_password,
                width,
                t,
            );
            if let Some(email) = user.email.as_deref().filter(|s| !s.trim().is_empty()) {
                ui.add(
                    egui::Label::new(egui::RichText::new(email).size(11.5).color(t.text_muted))
                        .truncate(),
                )
                .on_hover_text(email);
            }
        },
    );
}

fn draw_trashed_identity_block(
    ui: &mut egui::Ui,
    user: &TrashedUserSummary,
    width: f32,
    show_short_id: bool,
    t: &Tokens,
) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing.y = 2.0;

            draw_name_and_handle_row(ui, &user.display_name, &user.username, false, width, t);
            let short_id = short_user_id(&user.user_id);
            let subline = match user.email.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(email) if show_short_id => format!("{email} · ID {short_id}"),
                Some(email) => email.to_string(),
                None if show_short_id => format!("ID {short_id}"),
                None => String::new(),
            };
            if !subline.is_empty() {
                ui.add(
                    egui::Label::new(egui::RichText::new(&subline).size(11.5).color(t.text_muted))
                        .truncate(),
                )
                .on_hover_text(subline);
            }
        },
    );
}

fn draw_name_and_handle_row(
    ui: &mut egui::Ui,
    display_name: &str,
    username: &str,
    must_change_password: bool,
    width: f32,
    t: &Tokens,
) {
    let primary_name = if display_name.trim().is_empty() {
        format!("@{username}")
    } else {
        display_name.to_string()
    };
    let at_handle = format!("@{username}");
    let show_handle = !primary_name.eq_ignore_ascii_case(&at_handle)
        && !primary_name.eq_ignore_ascii_case(username);
    let reset_pill_w = if must_change_password {
        crate::ui::widgets::measure_compact_pill_width(ui, false, "Reset pending") + 6.0
    } else {
        0.0
    };

    ui.horizontal(|ui| {
        ui.set_max_width(width);
        ui.spacing_mut().item_spacing.x = 6.0;
        let name_budget = if show_handle {
            ((width - reset_pill_w) * 0.56).max(40.0)
        } else {
            (width - reset_pill_w).max(48.0)
        };
        ui.scope(|ui| {
            ui.set_max_width(name_budget);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&primary_name)
                        .size(13.0)
                        .strong()
                        .color(t.text),
                )
                .truncate(),
            )
            .on_hover_text(&primary_name);
        });
        if show_handle {
            let handle_budget = (width - reset_pill_w - name_budget - 6.0).max(32.0);
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
        if must_change_password {
            draw_compact_pill(
                ui,
                None,
                "Reset pending",
                t.warning_bg,
                t.warning_border,
                t.warning_fg,
            );
        }
    });
}

fn format_notebook_count(count: usize) -> String {
    if count == 1 {
        "1 notebook".to_string()
    } else {
        format!("{count} notebooks")
    }
}

fn render_search_no_matches_box(query: &str, noun: &str, ui: &mut egui::Ui, t: &Tokens) {
    egui::Frame::new()
        .inner_margin(Margin::symmetric(16, 26))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(format!("No {noun} match \"{}\".", query.trim()))
                        .size(12.5)
                        .color(t.text_muted),
                );
            });
        });
}

fn render_users_empty_state(registration_open: bool, ui: &mut egui::Ui, t: &Tokens) {
    egui::Frame::new()
        .inner_margin(Margin::symmetric(20, 32))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                let (badge_rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                ui.painter().rect(
                    badge_rect,
                    CornerRadius::same(10),
                    t.accent_soft,
                    Stroke::new(1.0, t.accent.gamma_multiply(0.35)),
                    egui::StrokeKind::Inside,
                );
                paint_icon(ui.painter(), badge_rect.shrink(10.0), Icon::Account, t.accent);

                ui.label(
                    egui::RichText::new("No registered users yet")
                        .size(14.0)
                        .strong()
                        .color(t.text),
                );
                let hint = if registration_open {
                    "When someone creates an account in PaperCraft Desktop or Web, it will appear here."
                } else {
                    "New account sign-ups are closed, and no accounts exist in this database yet."
                };
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(hint)
                            .size(12.0)
                            .color(t.text_muted),
                    )
                    .wrap(),
                );
            });
        });
}

fn render_trash_empty_state(ui: &mut egui::Ui, t: &Tokens) {
    egui::Frame::new()
        .inner_margin(Margin::symmetric(20, 32))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                let (badge_rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                ui.painter().rect(
                    badge_rect,
                    CornerRadius::same(10),
                    t.toolbar_bg,
                    Stroke::new(1.0, t.border),
                    egui::StrokeKind::Inside,
                );
                paint_icon(
                    ui.painter(),
                    badge_rect.shrink(10.0),
                    Icon::Trash,
                    t.text_muted,
                );

                ui.label(
                    egui::RichText::new("Server Trash is empty")
                        .size(14.0)
                        .strong()
                        .color(t.text),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!(
                            "Deleted accounts stay in server Trash for {ACCOUNT_TRASH_RETENTION_DAYS} days so you can restore them before automatic permanent deletion."
                        ))
                        .size(12.0)
                        .color(t.text_muted),
                    )
                    .wrap(),
                );
            });
        });
}
