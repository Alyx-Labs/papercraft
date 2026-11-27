//! Responsive, plain-English control center UI for PaperCraft Server.

mod overview;
mod settings;
mod users;
mod widgets;

use egui::{Color32, CornerRadius, Margin, Sense, Stroke, Vec2};
use papercraft_doc::UiTheme;
use papercraft_updater::UpdateStatus;

use crate::controller::{ControlTab, NoticeBanner, NoticeTone, ServerController, ServerStatus};
use crate::icons::{Icon, SwitchOption, header_icon_button, paint_icon, segmented_switch};
use crate::theme::Tokens;
use crate::ui::widgets::{
    danger_button, draw_banner, draw_status_pill, header_primary_button, primary_button,
    secondary_button,
};

pub const SIDEBAR_BREAKPOINT: f32 = 760.0;
pub const TWO_COL_BREAKPOINT: f32 = 720.0;
pub const USER_TABLE_BREAKPOINT: f32 = 660.0;
const SIDEBAR_WIDTH: f32 = 216.0;
const MAX_CONTENT_WIDTH: f32 = 1280.0;

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
    update_status: &UpdateStatus,
) -> UiActions {
    let mut actions = UiActions::default();
    let ctx = ui.ctx().clone();
    let viewport_w = ui.available_width();
    let show_sidebar = viewport_w >= SIDEBAR_BREAKPOINT;

    let header_resp = egui::Panel::top("bnc_srv_header")
        .frame(
            egui::Frame::new()
                .fill(t.header_bg)
                .inner_margin(Margin::symmetric(16, 10))
                .stroke(Stroke::NONE),
        )
        .show(ui, |ui| {
            render_header(ctrl, viewport_w, ui, t);
        });
    ui.painter().hline(
        header_resp.response.rect.x_range(),
        header_resp.response.rect.bottom() - 0.5,
        Stroke::new(1.0, t.header_border),
    );

    let footer_resp = egui::Panel::bottom("bnc_srv_footer")
        .frame(
            egui::Frame::new()
                .fill(t.panel_bg)
                .inner_margin(Margin::symmetric(16, 9))
                .stroke(Stroke::NONE),
        )
        .show(ui, |ui| {
            render_footer(viewport_w, ui, t, &mut actions);
        });
    ui.painter().hline(
        footer_resp.response.rect.x_range(),
        footer_resp.response.rect.top() + 0.5,
        Stroke::new(1.0, t.border),
    );

    if show_sidebar {
        let sidebar_resp = egui::Panel::left("bnc_srv_sidebar")
            .drag_to_open(false)
            .resizable(false)
            .exact_size(SIDEBAR_WIDTH)
            .frame(
                egui::Frame::new()
                    .fill(t.panel_bg)
                    .inner_margin(Margin::symmetric(12, 14))
                    .stroke(Stroke::NONE),
            )
            .show(ui, |ui| {
                render_sidebar(ctrl, update_status, ui, t);
            });
        ui.painter().vline(
            sidebar_resp.response.rect.right() - 0.5,
            sidebar_resp.response.rect.y_range(),
            Stroke::new(1.0, t.border),
        );
    }

    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(t.canvas_bg)
                .inner_margin(Margin::ZERO),
        )
        .show(ui, |ui| {
            if !show_sidebar {
                egui::Frame::new()
                    .inner_margin(Margin {
                        left: 16,
                        right: 16,
                        top: 10,
                        bottom: 4,
                    })
                    .show(ui, |ui| {
                        render_compact_top_nav(ctrl, viewport_w, ui, t);
                    });
            }

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let avail_w = ui.available_width();
                    let side_pad = if avail_w < 600.0 { 16.0 } else { 24.0 };
                    let content_w = (avail_w - side_pad * 2.0).clamp(160.0, MAX_CONTENT_WIDTH);
                    let left_pad = ((avail_w - content_w) * 0.5).max(side_pad);

                    ui.add_space(if show_sidebar { 16.0 } else { 8.0 });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(left_pad);
                        ui.allocate_ui_with_layout(
                            Vec2::new(content_w, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(content_w);
                                ui.spacing_mut().item_spacing.y = 0.0;
                                render_global_banners(ctrl, ui, t);
                                match ctrl.active_tab {
                                    ControlTab::Overview => {
                                        overview::render_overview_page(ctrl, ui, t)
                                    }
                                    ControlTab::Users => users::render_users_page(ctrl, ui, t),
                                    ControlTab::Trash => users::render_trash_page(ctrl, ui, t),
                                    ControlTab::Settings => settings::render_settings_page(
                                        ctrl,
                                        &ctx,
                                        update_status,
                                        ui,
                                        t,
                                        &mut actions,
                                    ),
                                }
                            },
                        );
                    });
                    ui.add_space(20.0);
                });
        });

    ctrl.auto_save_settings();
    actions
}

fn render_header(ctrl: &mut ServerController, viewport_w: f32, ui: &mut egui::Ui, t: &Tokens) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;

        let (badge_rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
        let badge_border = if t.is_dark {
            t.accent.gamma_multiply(0.45)
        } else {
            t.header_active_bg
        };
        ui.painter().rect(
            badge_rect,
            CornerRadius::same(8),
            t.header_active_bg,
            Stroke::new(1.0, badge_border),
            egui::StrokeKind::Inside,
        );
        paint_icon(
            ui.painter(),
            badge_rect.shrink(7.0),
            Icon::Server,
            t.header_active_fg,
        );

        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(
                egui::RichText::new("PaperCraft Server")
                    .size(15.0)
                    .strong()
                    .color(t.header_fg),
            );
            if viewport_w >= 580.0 {
                ui.label(
                    egui::RichText::new("Personal sync server")
                        .size(11.0)
                        .color(t.header_fg_muted),
                );
            }
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            let (theme_icon, next_theme, theme_tip) = match ctrl.draft_settings.theme {
                UiTheme::Light => (Icon::Moon, UiTheme::Dark, "Switch to Dark theme"),
                UiTheme::Dark => (Icon::Sun, UiTheme::Light, "Switch to Light theme"),
            };
            if header_icon_button(ui, theme_icon, None, false, true, t)
                .on_hover_text(theme_tip)
                .clicked()
            {
                ctrl.draft_settings.theme = next_theme;
            }

            if ctrl.is_running() {
                if danger_button(ui, Icon::Square, "Stop", true, t)
                    .on_hover_text("Gracefully save the database and stop accepting syncs")
                    .clicked()
                {
                    ctrl.stop_server();
                }
            } else {
                let start_lbl = if viewport_w < 560.0 {
                    "Start"
                } else {
                    "Start Server"
                };
                if header_primary_button(ui, Icon::Server, start_lbl, true, t)
                    .on_hover_text("Start the PaperCraft sync server now")
                    .clicked()
                {
                    ctrl.start_server();
                }
            }

            draw_status_pill(ui, &ctrl.status, true, t);
        });
    });
}

enum SidebarBadgeTone {
    Neutral,
    Danger,
    Accent,
}

struct SidebarBadge<'a> {
    text: &'a str,
    tone: SidebarBadgeTone,
}

fn render_sidebar(
    ctrl: &mut ServerController,
    update_status: &UpdateStatus,
    ui: &mut egui::Ui,
    t: &Tokens,
) {
    let users_count = ctrl.users_snapshot.len().to_string();
    let trash_count = ctrl.trashed_users_snapshot.len();
    let trash_count_str = trash_count.to_string();
    let has_update = matches!(
        update_status,
        UpdateStatus::Available(_)
            | UpdateStatus::Downloading { .. }
            | UpdateStatus::ReadyToInstall { .. }
    );

    ui.spacing_mut().item_spacing.y = 3.0;

    draw_sidebar_group_label(ui, "SERVER", t);
    if draw_sidebar_nav_item(
        ui,
        Icon::Server,
        "Overview",
        ctrl.active_tab == ControlTab::Overview,
        None,
        t,
    ) {
        ctrl.set_active_tab(ControlTab::Overview);
    }

    ui.add_space(10.0);
    draw_sidebar_group_label(ui, "ACCOUNTS", t);
    if draw_sidebar_nav_item(
        ui,
        Icon::Account,
        "Users",
        ctrl.active_tab == ControlTab::Users,
        Some(SidebarBadge {
            text: &users_count,
            tone: SidebarBadgeTone::Neutral,
        }),
        t,
    ) {
        ctrl.set_active_tab(ControlTab::Users);
    }
    if draw_sidebar_nav_item(
        ui,
        Icon::Trash,
        "Trash",
        ctrl.active_tab == ControlTab::Trash,
        Some(SidebarBadge {
            text: &trash_count_str,
            tone: if trash_count > 0 {
                SidebarBadgeTone::Danger
            } else {
                SidebarBadgeTone::Neutral
            },
        }),
        t,
    ) {
        ctrl.set_active_tab(ControlTab::Trash);
    }

    ui.add_space(10.0);
    draw_sidebar_group_label(ui, "CONFIGURATION", t);
    let update_badge = has_update.then_some(SidebarBadge {
        text: "Update",
        tone: SidebarBadgeTone::Accent,
    });
    if draw_sidebar_nav_item(
        ui,
        Icon::Settings,
        "Settings",
        ctrl.active_tab == ControlTab::Settings,
        update_badge,
        t,
    ) {
        ctrl.set_active_tab(ControlTab::Settings);
    }

    if ui.available_height() >= 104.0 {
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            render_sidebar_footer(ctrl, ui, t);
        });
    }
}

fn draw_sidebar_group_label(ui: &mut egui::Ui, label: &str, t: &Tokens) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(label)
                .size(10.5)
                .strong()
                .color(t.text_muted),
        );
    });
    ui.add_space(3.0);
}

fn draw_sidebar_nav_item(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    active: bool,
    badge: Option<SidebarBadge<'_>>,
    t: &Tokens,
) -> bool {
    let width = ui.available_width();
    let height = 34.0_f32;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let hovered = resp.hovered() && !active;
        let fill = if active {
            t.accent_soft
        } else if hovered {
            t.toolbar_bg
        } else {
            Color32::TRANSPARENT
        };
        p.rect_filled(rect, CornerRadius::same(8), fill);

        if active {
            let bar = egui::Rect::from_min_max(
                egui::pos2(rect.left() + 3.0, rect.top() + 7.0),
                egui::pos2(rect.left() + 6.0, rect.bottom() - 7.0),
            );
            p.rect_filled(bar, CornerRadius::same(2), t.accent);
        }

        let fg = if active { t.accent } else { t.text };
        let icon_fg = if active { t.accent } else { t.text_muted };
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 18.0, rect.center().y),
            Vec2::splat(16.0),
        );
        paint_icon(p, icon_rect, icon, icon_fg);

        let font = egui::FontId::proportional(13.0);
        let text_galley = p.layout_no_wrap(label.to_string(), font, fg);
        p.galley(
            egui::pos2(
                icon_rect.right() + 9.0,
                rect.center().y - text_galley.size().y * 0.5,
            ),
            text_galley,
            fg,
        );

        if let Some(b) = badge {
            let (b_fill, b_fg) = match b.tone {
                SidebarBadgeTone::Neutral => (t.toolbar_bg, t.text_muted),
                SidebarBadgeTone::Danger => (t.danger_bg, t.danger_fg),
                SidebarBadgeTone::Accent => (t.accent, t.on_accent),
            };
            let b_galley =
                p.layout_no_wrap(b.text.to_string(), egui::FontId::proportional(11.0), b_fg);
            let b_w = (b_galley.size().x + 12.0).max(22.0);
            let b_h = 18.0_f32;
            let b_rect = egui::Rect::from_center_size(
                egui::pos2(rect.right() - 8.0 - b_w * 0.5, rect.center().y),
                Vec2::new(b_w, b_h),
            );
            p.rect_filled(b_rect, CornerRadius::same(9), b_fill);
            p.galley(
                egui::pos2(
                    b_rect.center().x - b_galley.size().x * 0.5,
                    b_rect.center().y - b_galley.size().y * 0.5,
                ),
                b_galley,
                b_fg,
            );
        }
    }

    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

fn render_sidebar_footer(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    let current_ver = ctrl.updater.config().map_or_else(
        || env!("CARGO_PKG_VERSION").to_string(),
        |cfg| cfg.current_version.to_string(),
    );
    ui.spacing_mut().item_spacing.y = 8.0;
    ui.label(
        egui::RichText::new(papercraft_doc::format_alpha_version(&current_ver))
            .size(11.0)
            .color(t.text_muted),
    );

    let scope_text = format!(
        "{}:{}",
        ctrl.draft_settings.network_scope.label(),
        ctrl.draft_settings.port
    );
    let (reg_icon, reg_text, reg_color) = if ctrl.draft_settings.allow_registration {
        (Icon::LockOpen, "Sign-ups open", t.success_fg)
    } else {
        (Icon::Lock, "Sign-ups closed", t.warning_fg)
    };

    let card_resp = egui::Frame::new()
        .fill(t.toolbar_bg)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 5.0;

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(13.0), Sense::hover());
                paint_icon(ui.painter(), ir, Icon::Globe, t.text_muted);
                ui.add(
                    egui::Label::new(egui::RichText::new(&scope_text).size(11.0).color(t.text))
                        .truncate(),
                );
            });

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(13.0), Sense::hover());
                paint_icon(ui.painter(), ir, reg_icon, reg_color);
                ui.label(
                    egui::RichText::new(reg_text)
                        .size(11.0)
                        .strong()
                        .color(reg_color),
                );
            });
        })
        .response
        .interact(Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Open network access and sign-up settings on Overview");

    if card_resp.clicked() {
        ctrl.set_active_tab(ControlTab::Overview);
    }
}

fn render_compact_top_nav(
    ctrl: &mut ServerController,
    viewport_w: f32,
    ui: &mut egui::Ui,
    t: &Tokens,
) {
    let users_label = format!("Users ({})", ctrl.users_snapshot.len());
    let trash_label = format!("Trash ({})", ctrl.trashed_users_snapshot.len());
    let show_icons = viewport_w >= 540.0;
    let options = [
        SwitchOption::new(
            show_icons.then_some(Icon::Server),
            "Overview",
            "Server status, connection addresses, and access controls",
        ),
        SwitchOption::new(
            show_icons.then_some(Icon::Account),
            &users_label,
            "Active registered accounts stored on this server",
        ),
        SwitchOption::new(
            show_icons.then_some(Icon::Trash),
            &trash_label,
            "Deleted accounts held in server Trash for 30 days",
        ),
        SwitchOption::new(
            show_icons.then_some(Icon::Settings),
            "Settings",
            "Storage folder, sync limits, startup, and updates",
        ),
    ];
    let active_idx = match ctrl.active_tab {
        ControlTab::Overview => 0,
        ControlTab::Users => 1,
        ControlTab::Trash => 2,
        ControlTab::Settings => 3,
    };
    if let Some(clicked) = segmented_switch(ui, "bnc_srv_primary_tabs", &options, active_idx, t) {
        let next = match clicked {
            0 => ControlTab::Overview,
            1 => ControlTab::Users,
            2 => ControlTab::Trash,
            _ => ControlTab::Settings,
        };
        ctrl.set_active_tab(next);
    }
}

fn render_global_banners(ctrl: &mut ServerController, ui: &mut egui::Ui, t: &Tokens) {
    let mut had_banner = false;

    if ctrl.has_pending_restart() {
        had_banner = true;
        egui::Frame::new()
            .fill(t.accent_soft)
            .stroke(Stroke::new(1.0, t.accent))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if ui.available_width() >= 580.0 {
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
                            if primary_button(ui, Icon::RotateCw, "Apply & Restart", true, t)
                                .clicked()
                            {
                                ctrl.restart_server();
                            }
                            if secondary_button(ui, "Revert", true, t).clicked() {
                                ctrl.revert_draft();
                            }
                        });
                    });
                } else {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.horizontal(|ui| {
                        let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                        paint_icon(ui.painter(), ir, Icon::Info, t.accent);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(
                                    "Settings changed — restart the server to apply.",
                                )
                                .size(12.0)
                                .strong()
                                .color(t.text),
                            )
                            .wrap(),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if primary_button(ui, Icon::RotateCw, "Apply & Restart", true, t)
                                .clicked()
                            {
                                ctrl.restart_server();
                            }
                            if secondary_button(ui, "Revert", true, t).clicked() {
                                ctrl.revert_draft();
                            }
                        });
                    });
                }
            });
    }

    if let ServerStatus::Error(ref err) = ctrl.status {
        if had_banner {
            ui.add_space(8.0);
        }
        had_banner = true;
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

    if let Some(banner) = ctrl.notice_banner.clone() {
        if had_banner {
            ui.add_space(8.0);
        }
        had_banner = true;
        if draw_banner(ui, &banner, true, t) {
            ctrl.notice_banner = None;
        }
    }

    if had_banner {
        ui.add_space(12.0);
    }
}

fn render_footer(viewport_w: f32, ui: &mut egui::Ui, t: &Tokens, actions: &mut UiActions) {
    let hint = if viewport_w >= 560.0 {
        "Closing this window hides it to the system tray; the server keeps running."
    } else {
        "Closing hides to tray; server keeps running."
    };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(hint).size(11.0).color(t.text_muted));
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use egui::{Pos2, RawInput, Rect, Shape, Vec2};
    use papercraft_doc::UiTheme;
    use papercraft_server_cli::config::ensure_data_dir_and_meta;
    use papercraft_server_cli::db::{PasswordResetOutcome, ServerDb, TokenTtls, UserSummary};
    use papercraft_updater::{
        AppId, AssetFormat, AvailableUpdate, InstallCapability, PlatformAsset, SemVer,
        StagedUpdate, UpdateStatus,
    };

    use super::settings::draw_update_progress_bar;
    use super::widgets::format_utc_date;
    use super::{ControlTab, ServerController, render_window, render_window_with_update_status};
    use crate::settings::{CONFIG_FILE_NAME, ServerControlSettings};
    use crate::theme::{Tokens, apply_visuals};

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

    fn collect_exact_text_entries(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        let mut out = Vec::new();
        fn visit(shape: &Shape, buf: &mut Vec<String>) {
            match shape {
                Shape::Text(t) => buf.push(t.galley.text().to_string()),
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

    fn find_text_rect(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<Rect> {
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
    }

    fn find_exact_text_center(shapes: &[egui::epaint::ClippedShape], target: &str) -> Option<Pos2> {
        fn visit(shape: &Shape, target: &str, out: &mut Option<Pos2>) {
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

    #[test]
    fn ui_renders_light_dark_stopped_running_and_settings_states_cleanly() {
        let dir = temp_dir("render");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = ServerControlSettings {
            data_dir: dir.0.join("vault").to_string_lossy().into_owned(),
            port: 18944,
            auto_start_server: false,
            ..Default::default()
        };
        settings.save_atomic(&cfg_path).unwrap();

        let mut ctrl = ServerController::new(cfg_path, false);
        let ctx = egui::Context::default();

        // 1. Stopped + Light theme + Narrow 540px Overview
        ctrl.draft_settings.theme = UiTheme::Light;
        apply_visuals(&ctx, ctrl.theme());
        let light_tokens = Tokens::for_theme(ctrl.theme());
        let out1 = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(540.0, 720.0))),
                ..Default::default()
            },
            |ui| {
                render_window(&mut ctrl, ui, &light_tokens);
            },
        );
        let text1 = collect_text(&out1.shapes);
        out1.drop_without_applying_deltas();
        assert!(text1.contains("PaperCraft Server"));
        assert!(text1.contains("Stopped"));
        assert!(text1.contains("Start"));
        assert!(text1.contains("Connection Addresses"));
        assert!(text1.contains("Who can connect"));
        assert!(text1.contains("New sign-ups open"));
        assert!(!text1.contains("Stay signed in (days)"));

        // 2. Running + Dark theme + Wide 1040px Overview
        ctrl.draft_settings.theme = UiTheme::Dark;
        ctrl.start_server();
        if !ctrl.is_running() {
            ctrl.draft_settings.port = 28944;
            ctrl.start_server();
        }
        assert!(ctrl.is_running());

        apply_visuals(&ctx, ctrl.theme());
        let dark_tokens = Tokens::for_theme(ctrl.theme());
        let out2 = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1040.0, 760.0))),
                ..Default::default()
            },
            |ui| {
                render_window(&mut ctrl, ui, &dark_tokens);
            },
        );
        let text2 = collect_text(&out2.shapes);
        out2.drop_without_applying_deltas();
        assert!(text2.contains("Running"));
        assert!(text2.contains("Accounts"));
        assert!(text2.contains("Unsynced local accounts"));
        assert!(text2.contains("Local notebooks"));
        assert!(text2.contains("Counts only"));
        assert!(text2.contains("Allow new account sign-ups"));
        ctrl.stop_server();

        // 3. Settings tab shows Storage, Limits, Startup & App Updates
        ctrl.set_active_tab(ControlTab::Settings);
        let out3 = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1040.0, 960.0))),
                ..Default::default()
            },
            |ui| {
                render_window(&mut ctrl, ui, &dark_tokens);
            },
        );
        let text3 = collect_text(&out3.shapes);
        out3.drop_without_applying_deltas();
        assert!(text3.contains("Where to save your notebooks"));
        assert!(text3.contains("Stay signed in (days)"));
        assert!(text3.contains("App Updates"));
        assert!(text3.contains("Check for updates automatically on startup"));
        assert!(text3.contains("Check for Updates"));
    }

    #[test]
    fn ui_renders_users_tab_and_reset_delete_states_cleanly() {
        let dir = temp_dir("render-users");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = ServerControlSettings {
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
        let raw_wide = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1120.0, 900.0))),
            ..Default::default()
        };
        apply_visuals(&ctx, ctrl.theme());
        let dark_tokens = Tokens::for_theme(ctrl.theme());

        // 1. Empty Users tab state
        let out_empty = ctx.run_ui(raw_wide.clone(), |ui| {
            render_window(&mut ctrl, ui, &dark_tokens);
        });
        let text_empty = collect_text(&out_empty.shapes);
        out_empty.drop_without_applying_deltas();
        assert!(text_empty.contains("Registered Users"));
        assert!(text_empty.contains("No registered users yet"));
        assert!(text_empty.contains("New sign-ups open"));
        assert!(!text_empty.contains("Where to save your notebooks"));

        // 2. Populated user table row
        ctrl.users_snapshot = vec![UserSummary {
            user_id: "12345678-abcd-efgh-ijkl-999999999999".to_string(),
            email: None,
            username: "maria_notes".to_string(),
            display_name: "Maria Clara".to_string(),
            notebook_count: 3,
            created_at_ms: 1_763_424_000_000,
            must_change_password: false,
        }];
        let out_populated = ctx.run_ui(raw_wide.clone(), |ui| {
            render_window(&mut ctrl, ui, &dark_tokens);
        });
        let text_populated = collect_text(&out_populated.shapes);
        out_populated.drop_without_applying_deltas();
        assert!(text_populated.contains("Maria Clara"));
        assert!(text_populated.contains("@maria_notes"));
        assert!(text_populated.contains("3 notebooks"));
        assert!(text_populated.contains("2025-11-18"));
        assert!(text_populated.contains("12345678-abcd-efgh-ijkl-999999999999"));
        assert!(text_populated.contains("Copy ID"));
        assert!(text_populated.contains("Reset"));
        assert!(text_populated.contains("Trash"));

        // 3. Reset confirmation, generated recovery code card, and delete confirmation
        ctrl.request_reset_password("12345678-abcd-efgh-ijkl-999999999999");
        let out_reset = ctx.run_ui(raw_wide.clone(), |ui| {
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
        let out_delete = ctx.run_ui(raw_wide, |ui| {
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
        let settings = ServerControlSettings {
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
                            &UpdateStatus::Idle,
                        );
                    },
                );
                let text = collect_text(&out.shapes);
                let shapes = out.shapes.clone();
                out.drop_without_applying_deltas();
                (text, shapes)
            };

        let click_at = |ctrl: &mut ServerController, time: &mut f64, width: f32, pos: Pos2| {
            for pressed in [true, false] {
                let _ = run_ui(
                    ctrl,
                    time,
                    width,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        };

        // 1. Narrow 460px width renders row and top nav cleanly without clipping.
        let (narrow_text, _) = run_ui(&mut ctrl, &mut t, 460.0, Vec::new());
        assert!(narrow_text.contains("Users (1)"));
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
        let move_btn = find_exact_text_center(&paste_shapes, "Move to Trash").unwrap();
        click_at(&mut ctrl, &mut t, 560.0, move_btn);
        assert_eq!(ctrl.users_snapshot.len(), 1);
        assert!(ctrl.trashed_users_snapshot.is_empty());

        // 3. Click the manual confirmation input and type `delete alice_notes` to enable Move to Trash.
        let manual_lbl = find_exact_text_center(&paste_shapes, "Manual Confirmation").unwrap();
        let input_pos = egui::pos2(manual_lbl.x + 12.0, manual_lbl.y + 24.0);
        click_at(&mut ctrl, &mut t, 560.0, input_pos);
        let _ = run_ui(
            &mut ctrl,
            &mut t,
            560.0,
            vec![egui::Event::Text("delete alice_notes".to_string())],
        );
        let (matched_text, matched_shapes) = run_ui(&mut ctrl, &mut t, 560.0, Vec::new());
        assert!(matched_text.contains("Confirmation matched"));
        let enabled_move_btn = find_exact_text_center(&matched_shapes, "Move to Trash").unwrap();
        click_at(&mut ctrl, &mut t, 560.0, enabled_move_btn);
        assert!(ctrl.users_snapshot.is_empty());
        assert_eq!(ctrl.trashed_users_snapshot.len(), 1);

        // 4. Top nav renders Trash (1); clicking Trash (1) shows 30-day retention & Restore.
        let (empty_active_text, empty_active_shapes) = run_ui(&mut ctrl, &mut t, 560.0, Vec::new());
        assert!(empty_active_text.contains("Users (0)"));
        assert!(empty_active_text.contains("Trash (1)"));
        let trash_tab = find_exact_text_center(&empty_active_shapes, "Trash (1)").unwrap();
        click_at(&mut ctrl, &mut t, 560.0, trash_tab);
        assert_eq!(ctrl.active_tab, ControlTab::Trash);

        let (trash_text, trash_shapes) = run_ui(&mut ctrl, &mut t, 560.0, Vec::new());
        assert!(trash_text.contains("30-day retention"));
        assert!(trash_text.contains("30 days left to restore"));
        assert!(trash_text.contains("Restore"));

        // 5. Clicking Restore brings Alice back to Users.
        let restore_btn = find_exact_text_center(&trash_shapes, "Restore").unwrap();
        click_at(&mut ctrl, &mut t, 560.0, restore_btn);
        assert_eq!(ctrl.users_snapshot.len(), 1);
        assert!(ctrl.trashed_users_snapshot.is_empty());
        assert_eq!(ctrl.active_tab, ControlTab::Users);
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

    #[test]
    fn server_update_states_render_inside_app_updates_card_in_consistent_order() {
        let dir = temp_dir("update-order");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = ServerControlSettings {
            data_dir: dir.0.join("vault").to_string_lossy().into_owned(),
            port: 18947,
            auto_start_server: false,
            theme: UiTheme::Dark,
            ..Default::default()
        };
        settings.save_atomic(&cfg_path).unwrap();
        let mut ctrl = ServerController::new(cfg_path, false);
        ctrl.set_active_tab(ControlTab::Settings);

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
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1040.0, 960.0))),
            ..Default::default()
        };
        apply_visuals(&ctx, ctrl.theme());
        let dark_tokens = Tokens::for_theme(ctrl.theme());

        // 1. Available state renders inside App Updates.
        let available_status = UpdateStatus::Available(Box::new(update.clone()));
        let out_avail = ctx.run_ui(raw_tall.clone(), |ui| {
            let _ =
                render_window_with_update_status(&mut ctrl, ui, &dark_tokens, &available_status);
        });
        let text_avail = collect_text(&out_avail.shapes);
        let y_updates_hdr = find_text_y(&out_avail.shapes, "App Updates").unwrap();
        let y_avail = find_text_y(&out_avail.shapes, "Update available: v0.2.0").unwrap();
        let y_auto_chk = find_text_y(
            &out_avail.shapes,
            "Check for updates automatically on startup",
        )
        .unwrap();
        out_avail.drop_without_applying_deltas();

        assert!(text_avail.contains("Download Update"));
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
            update: Box::new(update),
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
        let dir = temp_dir("desktop-shortcut-switch");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = ServerControlSettings {
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
        ctrl.set_active_tab(ControlTab::Settings);
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

        let ctx = egui::Context::default();
        apply_visuals(&ctx, ctrl.theme());
        let tokens = Tokens::for_theme(ctrl.theme());
        let idle = UpdateStatus::Idle;
        let run_frame = |ctrl: &mut ServerController, t: f64, events: Vec<egui::Event>| {
            let out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1040.0, 1200.0))),
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
        let shortcut_rect =
            find_text_rect(&shapes, "Create Desktop shortcut automatically").unwrap();
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
        assert!(find_text_rect(&shapes_off, "Server shortcut removed (mock).").is_none());

        let pos_on = find_text_rect(&shapes_off, "Create Desktop shortcut automatically")
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
        assert!(find_text_rect(&shapes_on, "Server shortcut ready (mock).").is_none());
        assert_eq!(*calls.lock().unwrap(), vec![false, true]);

        let (reloaded, warning) = ServerControlSettings::load_or_init(&cfg_path);
        assert!(warning.is_none());
        assert!(reloaded.auto_create_desktop_shortcut);
    }

    #[test]
    fn autostart_switch_renders_below_desktop_shortcut_and_invokes_hook() {
        let dir = temp_dir("autostart-switch");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = ServerControlSettings {
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
        ctrl.set_active_tab(ControlTab::Settings);
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

        let ctx = egui::Context::default();
        apply_visuals(&ctx, ctrl.theme());
        let tokens = Tokens::for_theme(ctrl.theme());
        let idle = UpdateStatus::Idle;
        let run_frame = |ctrl: &mut ServerController, t: f64, events: Vec<egui::Event>| {
            let out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1040.0, 1200.0))),
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
        let autostart_rect =
            find_text_rect(&shapes, "Launch automatically at system startup").unwrap();
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
        assert!(find_text_rect(&shapes_off, "Startup launch removed (mock).").is_none());

        let pos_on = find_text_rect(&shapes_off, "Launch automatically at system startup")
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
        assert!(find_text_rect(&shapes_on, "Startup launch ready (mock).").is_none());
        assert_eq!(*calls.lock().unwrap(), vec![(false, true), (true, true)]);

        ctrl.set_autostart_sync(Box::new(|_, _| Err("Startup registry locked".to_string())));
        let autostart_err_pos =
            find_text_rect(&shapes_on, "Launch automatically at system startup")
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
        assert!(find_text_rect(&shapes_err, "Startup registry locked").is_some());

        ctrl.set_launch_at_login_enabled(true);
        let (reloaded, warning) = ServerControlSettings::load_or_init(&cfg_path);
        assert!(warning.is_none());
        assert!(reloaded.launch_at_login);
    }

    #[test]
    fn autostart_window_switch_renders_below_autostart_and_persists_when_enabled() {
        let dir = temp_dir("autostart-window-switch");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = ServerControlSettings {
            data_dir: dir.0.join("vault").to_string_lossy().into_owned(),
            port: 18950,
            auto_start_server: false,
            theme: UiTheme::Dark,
            ..Default::default()
        };
        settings.save_atomic(&cfg_path).unwrap();

        let mut ctrl = ServerController::new(cfg_path.clone(), false);
        ctrl.set_active_tab(ControlTab::Settings);
        assert!(ctrl.draft_settings.launch_at_login);
        assert!(!ctrl.draft_settings.open_window_at_login);

        let ctx = egui::Context::default();
        apply_visuals(&ctx, ctrl.theme());
        let tokens = Tokens::for_theme(ctrl.theme());
        let idle = UpdateStatus::Idle;
        let run_frame = |ctrl: &mut ServerController, t: f64, events: Vec<egui::Event>| {
            let out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1040.0, 1250.0))),
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
        let chk_rect = find_text_rect(&shapes, "Open control window at system startup").unwrap();
        assert!(y_autostart < chk_rect.min.y);

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
        let _shapes_checked = run_frame(
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

        let (reloaded, warning) = ServerControlSettings::load_or_init(&cfg_path);
        assert!(warning.is_none());
        assert!(reloaded.open_window_at_login);

        ctrl.set_launch_at_login_enabled(false);
        let shapes_disabled = run_frame(&mut ctrl, 1.20, Vec::new());
        let pos_disabled =
            find_text_rect(&shapes_disabled, "Open control window at system startup")
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
    }

    #[test]
    fn local_discovery_metrics_and_note_do_not_overlap_at_narrow_or_wide_widths() {
        let dir = temp_dir("local-discovery-no-overlap");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = ServerControlSettings {
            data_dir: dir.0.join("vault").to_string_lossy().into_owned(),
            port: 18952,
            auto_start_server: false,
            theme: UiTheme::Light,
            ..Default::default()
        };
        settings.save_atomic(&cfg_path).unwrap();

        let mut ctrl = ServerController::new(cfg_path, false);
        ctrl.start_server();
        if !ctrl.is_running() {
            ctrl.draft_settings.port = 28952;
            ctrl.start_server();
        }
        assert!(ctrl.is_running());

        let ctx = egui::Context::default();
        apply_visuals(&ctx, ctrl.theme());
        let tokens = Tokens::for_theme(ctrl.theme());

        for (w, h) in [(460.0, 1300.0), (1040.0, 820.0), (1440.0, 900.0)] {
            let out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(w, h))),
                    ..Default::default()
                },
                |ui| {
                    let _ = render_window_with_update_status(
                        &mut ctrl,
                        ui,
                        &tokens,
                        &UpdateStatus::Idle,
                    );
                },
            );
            let shapes = out.shapes.clone();
            out.drop_without_applying_deltas();
            let r_accounts = find_text_rect(&shapes, "Unsynced local accounts").unwrap();
            let r_notebooks = find_text_rect(&shapes, "Local notebooks").unwrap();
            let r_note = find_text_rect(&shapes, "Counts only").unwrap();

            assert!(
                !r_accounts.intersects(r_notebooks),
                "metrics overlapped at {w}x{h}: {r_accounts:?} vs {r_notebooks:?}"
            );
            assert!(
                !r_accounts.intersects(r_note),
                "accounts overlapped note at {w}x{h}: {r_accounts:?} vs {r_note:?}"
            );
            assert!(
                !r_notebooks.intersects(r_note),
                "notebooks overlapped note at {w}x{h}: {r_notebooks:?} vs {r_note:?}"
            );
        }
        ctrl.stop_server();
    }

    #[test]
    fn overview_shows_network_access_and_registration_above_the_fold_without_duplicate_hide() {
        let dir = temp_dir("overview-above-fold");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = ServerControlSettings {
            data_dir: dir.0.join("vault").to_string_lossy().into_owned(),
            port: 18953,
            auto_start_server: false,
            theme: UiTheme::Dark,
            ..Default::default()
        };
        settings.save_atomic(&cfg_path).unwrap();

        let mut ctrl = ServerController::new(cfg_path, false);
        let ctx = egui::Context::default();
        apply_visuals(&ctx, ctrl.theme());
        let tokens = Tokens::for_theme(ctrl.theme());

        let out = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1040.0, 700.0))),
                ..Default::default()
            },
            |ui| {
                let _ =
                    render_window_with_update_status(&mut ctrl, ui, &tokens, &UpdateStatus::Idle);
            },
        );
        let exact_texts = collect_exact_text_entries(&out.shapes);
        let shapes = out.shapes.clone();
        out.drop_without_applying_deltas();
        assert!(exact_texts.iter().any(|t| t == "Hide to Tray"));
        assert!(!exact_texts.iter().any(|t| t == "Hide"));

        for label in [
            "Who can connect",
            "This PC only",
            "Local network / VPN",
            "Custom IP",
            "Allow new account sign-ups",
            "New sign-ups open",
        ] {
            let rect = find_text_rect(&shapes, label)
                .unwrap_or_else(|| panic!("expected `{label}` to be rendered"));
            assert!(
                rect.max.y <= 700.0,
                "expected `{label}` above the fold (max.y <= 700), got {:?}",
                rect
            );
        }
    }

    #[test]
    fn wide_users_and_trash_tables_display_full_ids_dates_and_clear_actions() {
        let dir = temp_dir("wide-tables");
        let vault_dir = dir.0.join("vault");
        let _ = ensure_data_dir_and_meta(&vault_dir).unwrap();
        let (alice_id, bob_id) = {
            let db = ServerDb::open(&vault_dir.join("papercraft.redb")).unwrap();
            let ttls = TokenTtls {
                access_ttl_ms: 3_600_000,
                refresh_ttl_ms: 86_400_000,
            };
            let a = db
                .register_user_with_email(
                    Some("alice@example.com"),
                    "alice_notes",
                    "strong-password-1",
                    Some("Alice Notes"),
                    Some("Desk"),
                    true,
                    ttls,
                )
                .unwrap()
                .user
                .user_id;
            let b = db
                .register_user_with_email(
                    Some("bob@example.com"),
                    "bob_notes",
                    "strong-password-2",
                    Some("Bob Notes"),
                    Some("Laptop"),
                    true,
                    ttls,
                )
                .unwrap()
                .user
                .user_id;
            db.trash_user(&b).unwrap();
            (a, b)
        };

        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = ServerControlSettings {
            data_dir: vault_dir.to_string_lossy().into_owned(),
            port: 18954,
            auto_start_server: false,
            theme: UiTheme::Light,
            ..Default::default()
        };
        settings.save_atomic(&cfg_path).unwrap();

        let mut ctrl = ServerController::new(cfg_path, false);
        let ctx = egui::Context::default();
        apply_visuals(&ctx, ctrl.theme());
        let tokens = Tokens::for_theme(ctrl.theme());

        // Active Users wide table
        ctrl.set_active_tab(ControlTab::Users);
        let out_users = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 760.0))),
                ..Default::default()
            },
            |ui| {
                let _ =
                    render_window_with_update_status(&mut ctrl, ui, &tokens, &UpdateStatus::Idle);
            },
        );
        let users_text = collect_text(&out_users.shapes);
        out_users.drop_without_applying_deltas();
        assert!(users_text.contains(&alice_id));
        assert!(users_text.contains("alice@example.com"));
        assert!(users_text.contains("@alice_notes"));
        assert!(users_text.contains("Copy ID"));
        assert!(users_text.contains("Reset"));
        assert!(users_text.contains("Trash"));

        // Trash wide table
        ctrl.set_active_tab(ControlTab::Trash);
        let expected_expires = format_utc_date(ctrl.trashed_users_snapshot[0].expires_at_ms);
        let out_trash = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 760.0))),
                ..Default::default()
            },
            |ui| {
                let _ =
                    render_window_with_update_status(&mut ctrl, ui, &tokens, &UpdateStatus::Idle);
            },
        );
        let trash_text = collect_text(&out_trash.shapes);
        out_trash.drop_without_applying_deltas();
        assert!(trash_text.contains(&bob_id));
        assert!(trash_text.contains("30-day retention"));
        assert!(trash_text.contains(&expected_expires));
        assert!(trash_text.contains("30 days left to restore"));
        assert!(trash_text.contains("Restore"));
    }
}
