//! Dedicated minimalist Settings drawer: Pen stylus shortcuts, studio appearance, and system info.

use egui::{CornerRadius, Frame, Margin, Rect, RichText, Sense, Stroke, Vec2};
use papercraft_doc::{
    PenKind, PenSettings, StylusButtonAction, StylusButtonConfig, UserStudioConfig,
};
use serde_json::json;

use crate::icons::{
    Icon, SwitchLayout, SwitchOption, compact_icon_button, icon_button, paint_icon,
    section_card_header, segmented_switch,
};
use crate::theme::Tokens;
use crate::{PaperCraftApp, SettingsTab};

pub(crate) fn card_frame(t: &Tokens) -> Frame {
    Frame::new()
        .fill(t.field_bg)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(10))
        .stroke(Stroke::new(1.0, t.border))
}

struct StylusButtonRowSpec<'a> {
    id_salt: &'a str,
    title: &'a str,
    idle_badge: &'a str,
    active_badge: &'a str,
    active: bool,
    current: StylusButtonAction,
    options: &'a [StylusButtonAction],
}

fn draw_stylus_button_row(
    ui: &mut egui::Ui,
    spec: StylusButtonRowSpec<'_>,
    t: &Tokens,
) -> Option<StylusButtonAction> {
    let fill = if spec.active {
        t.accent_soft
    } else {
        t.panel_bg
    };
    let border = if spec.active { t.accent } else { t.border };
    let stroke_w = if spec.active { 1.5 } else { 1.0 };
    let title_color = if spec.active { t.accent } else { t.text };
    let badge_label = if spec.active {
        spec.active_badge
    } else {
        spec.idle_badge
    };
    let badge_bg = if spec.active {
        t.accent.gamma_multiply(0.16)
    } else {
        t.field_bg
    };
    let badge_border = if spec.active {
        t.accent.gamma_multiply(0.45)
    } else {
        t.border
    };
    let badge_fg = if spec.active { t.accent } else { t.text_muted };

    let mut chosen = None;
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(stroke_w, border))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(spec.title)
                            .size(12.2)
                            .strong()
                            .color(title_color),
                    )
                    .selectable(false),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    Frame::new()
                        .fill(badge_bg)
                        .stroke(Stroke::new(1.0, badge_border))
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(Margin::symmetric(6, 2))
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(badge_label).size(9.8).color(badge_fg),
                                )
                                .selectable(false),
                            );
                        });
                });
            });
            ui.add_space(5.0);
            let combo_w = ui.available_width();
            crate::theme::studio_combo_box(ui, spec.id_salt, spec.current.label(), combo_w, |ui| {
                for &opt in spec.options {
                    if ui
                        .selectable_label(opt == spec.current, opt.label())
                        .on_hover_text(opt.description())
                        .clicked()
                    {
                        chosen = Some(opt);
                    }
                }
            });
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(spec.current.description())
                        .size(11.0)
                        .color(t.text_muted),
                )
                .selectable(false)
                .wrap(),
            );
        });

    chosen.filter(|&next| next != spec.current)
}

fn draw_touch_and_palm_card(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    card_frame(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        section_card_header(
            ui,
            Icon::Pencil,
            "Touch & Palm Rejection",
            "Control how finger and palm touches behave on the canvas while writing with a stylus.",
            t,
        );

        let mut stylus_only = app.session.prefs.stylus_only_drawing;
        let check_resp = ui
            .checkbox(&mut stylus_only, "Draw with stylus only")
            .on_hover_text(
                "Ignore finger and resting palm touches when drawing, erasing, or placing shapes. Drag with one finger to pan the page and pinch with two fingers to zoom; mouse input still draws normally.",
            );
        if check_resp.changed() {
            let _ = app.session.execute(
                "stylus.setInputMode",
                &json!({ "stylus_only_drawing": stylus_only }),
            );
        }
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Finger and resting palm touches pan the canvas instead of leaving stray marks. Stylus and mouse input continue drawing normally, and toolbar controls remain tappable by touch.",
                )
                .size(11.0)
                .color(t.text_muted),
            )
            .wrap(),
        );
    });
}

fn draw_stylus_disconnected_card(ui: &mut egui::Ui, t: &Tokens) {
    card_frame(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        section_card_header(
            ui,
            Icon::BallPen,
            "Stylus detection",
            "Connect a supported stylus or pen display to configure hardware button shortcuts.",
            t,
        );

        Frame::new()
            .fill(t.panel_bg)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(7))
            .inner_margin(Margin::symmetric(9, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (dot_rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                    ui.painter()
                        .circle_filled(dot_rect.center(), 4.0, t.text_muted);
                    ui.label(
                        RichText::new("No stylus detected")
                            .size(11.5)
                            .strong()
                            .color(t.text_muted),
                    );
                });
            });

        ui.add_space(8.0);
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Bring your stylus near the screen or connect a pen tablet to detect it automatically and customize Side Button 1, Side Button 2, and Eraser End shortcuts.",
                )
                .size(11.0)
                .color(t.text_muted),
            )
            .wrap(),
        );
    });
}

fn draw_stylus_shortcuts_card(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let cfg = app.session.prefs.stylus_buttons.sanitized();
    let is_customized = cfg != StylusButtonConfig::default();
    let tel = app.stylus.telemetry;
    let device_label = app.stylus.connected_device_label();

    let mut reset_requested = false;
    let mut next_barrel = None;
    let mut next_secondary = None;
    let mut next_eraser = None;

    card_frame(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        section_card_header(
            ui,
            Icon::BallPen,
            "Stylus shortcuts",
            "Assign quick tool and edit actions to your connected stylus buttons.",
            t,
        );

        Frame::new()
            .fill(t.success_bg)
            .stroke(Stroke::new(1.0, t.success_border))
            .corner_radius(CornerRadius::same(7))
            .inner_margin(Margin::symmetric(9, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let reset_w = crate::icons::compact_icon_button_width(ui, "Reset");
                    let left_w =
                        (ui.available_width() - reset_w - ui.spacing().item_spacing.x).max(40.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(left_w, 28.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            let (dot_rect, _) =
                                ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                            ui.painter()
                                .circle_filled(dot_rect.center(), 4.0, t.success_fg);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("Connected · {device_label}"))
                                        .size(11.5)
                                        .strong()
                                        .color(t.success_fg),
                                )
                                .truncate(),
                            );
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if compact_icon_button(
                            ui,
                            Icon::Reset,
                            Some("Reset"),
                            false,
                            is_customized,
                            t,
                        )
                        .on_hover_text("Restore default stylus button shortcuts")
                        .clicked()
                        {
                            reset_requested = true;
                        }
                    });
                });
            });

        ui.add_space(8.0);
        next_barrel = draw_stylus_button_row(
            ui,
            StylusButtonRowSpec {
                id_salt: "stylus_barrel_button_combo",
                title: "Side Button 1",
                idle_badge: "Primary Barrel",
                active_badge: "Pressed",
                active: tel.barrel_held,
                current: cfg.barrel_button,
                options: StylusButtonAction::ALL,
            },
            t,
        );

        ui.add_space(6.0);
        next_secondary = draw_stylus_button_row(
            ui,
            StylusButtonRowSpec {
                id_salt: "stylus_secondary_barrel_combo",
                title: "Side Button 2",
                idle_badge: "Secondary Barrel",
                active_badge: "Pressed",
                active: tel.secondary_barrel_held,
                current: cfg.secondary_barrel_button,
                options: StylusButtonAction::ALL,
            },
            t,
        );

        ui.add_space(6.0);
        next_eraser = draw_stylus_button_row(
            ui,
            StylusButtonRowSpec {
                id_salt: "stylus_eraser_end_combo",
                title: "Eraser End",
                idle_badge: "Inverted Tip",
                active_badge: "Active",
                active: tel.eraser_inverted,
                current: cfg.eraser_end,
                options: StylusButtonAction::ERASER_END_OPTIONS,
            },
            t,
        );
    });

    if reset_requested {
        let _ = app
            .session
            .execute("stylus.setButtons", &json!({ "reset": true }));
    } else if next_barrel.is_some() || next_secondary.is_some() || next_eraser.is_some() {
        let mut payload = serde_json::Map::new();
        if let Some(action) = next_barrel {
            payload.insert("barrel_button".to_string(), json!(action));
        }
        if let Some(action) = next_secondary {
            payload.insert("secondary_barrel_button".to_string(), json!(action));
        }
        if let Some(action) = next_eraser {
            payload.insert("eraser_end".to_string(), json!(action));
        }
        let _ = app
            .session
            .execute("stylus.setButtons", &serde_json::Value::Object(payload));
    }
}

fn draw_appearance_controls(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    switch_id_salt: &str,
    t: &Tokens,
) {
    let current = app.session.prefs.theme;
    let active_idx = papercraft_doc::UiTheme::ALL
        .iter()
        .position(|&theme| theme == current);
    let options: Vec<SwitchOption<'_>> = papercraft_doc::UiTheme::ALL
        .iter()
        .map(|&theme| {
            let icon = match theme {
                papercraft_doc::UiTheme::Light => Icon::Sun,
                papercraft_doc::UiTheme::Dark => Icon::Moon,
            };
            SwitchOption::new(Some(icon), Some(theme.label()), theme.description(), true)
        })
        .collect();
    let (clicked, _) = segmented_switch(
        ui,
        switch_id_salt,
        &options,
        active_idx,
        SwitchLayout::EqualWidth,
        t,
    );
    if let Some(idx) = clicked
        && let Some(&theme) = papercraft_doc::UiTheme::ALL.get(idx)
    {
        let _ = app.session.execute("theme.set", &json!({ "theme": theme }));
    }

    ui.add_space(10.0);
    let mut auto_contrast = app.session.prefs.auto_contrast_with_theme;
    let check_resp = ui
        .checkbox(&mut auto_contrast, "Auto-adapt canvas colors to theme")
        .on_hover_text(
            "Invert paper polarity and rebalance ink contrast along its hue when switching themes. Saved files and exports keep their original colors.",
        );
    if check_resp.changed() {
        let _ = app.session.execute(
            "theme.set",
            &json!({ "auto_contrast_with_theme": auto_contrast }),
        );
    }
    ui.add(
        egui::Label::new(
            RichText::new(
                "Shifts paper light/dark to match the theme and adjusts ink brightness along its original hue for high contrast without changing exports.",
            )
            .size(11.0)
            .color(t.text_muted),
        )
        .wrap(),
    );
}

fn draw_appearance_card(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let current = app.session.prefs.theme;
    card_frame(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let header_icon = match current {
            papercraft_doc::UiTheme::Light => Icon::Sun,
            papercraft_doc::UiTheme::Dark => Icon::Moon,
        };
        section_card_header(
            ui,
            header_icon,
            "Appearance",
            "Switch the studio workspace between a bright daylight look and a low-glare dark interface.",
            t,
        );
        draw_appearance_controls(app, ui, "inspector_theme_switch", t);
    });
}

fn draw_app_scale_card(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let current_scale = UserStudioConfig::sanitize_ui_scale(app.session.prefs.ui_scale);
    let current_pct = (current_scale * 100.0).round() as i32;
    let is_default = (current_scale - UserStudioConfig::DEFAULT_UI_SCALE).abs() <= 0.005;
    let mut next_scale: Option<f32> = None;

    card_frame(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        section_card_header(
            ui,
            Icon::ZoomIn,
            "App Scale",
            "Resize interface panels, text, and controls. Also works anytime with Ctrl+Plus, Ctrl+Minus, and Ctrl+0.",
            t,
        );

        ui.horizontal(|ui| {
            ui.label(RichText::new("Current scale").size(12.0).color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if compact_icon_button(ui, Icon::Reset, Some("Reset"), false, !is_default, t)
                    .on_hover_text("Reset app scale to 100% (Ctrl+0)")
                    .clicked()
                {
                    next_scale = Some(UserStudioConfig::DEFAULT_UI_SCALE);
                }

                let at_max = current_scale >= UserStudioConfig::MAX_UI_SCALE - 0.005;
                if compact_icon_button(ui, Icon::ZoomIn, None, false, !at_max, t)
                    .on_hover_text("Increase app scale by 10% (Ctrl+Plus)")
                    .clicked()
                {
                    next_scale = Some(((current_scale + 0.1) * 10.0).round() / 10.0);
                }

                Frame::new()
                    .fill(if is_default {
                        t.toolbar_bg
                    } else {
                        t.accent_soft
                    })
                    .stroke(Stroke::new(
                        1.0,
                        if is_default { t.border } else { t.accent },
                    ))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(10, 4))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!("{current_pct}%"))
                                .size(12.0)
                                .strong()
                                .monospace()
                                .color(if is_default { t.text } else { t.accent }),
                        );
                    });

                let at_min = current_scale <= UserStudioConfig::MIN_UI_SCALE + 0.005;
                if compact_icon_button(ui, Icon::ZoomOut, None, false, !at_min, t)
                    .on_hover_text("Decrease app scale by 10% (Ctrl+Minus)")
                    .clicked()
                {
                    next_scale = Some(((current_scale - 0.1) * 10.0).round() / 10.0);
                }
            });
        });

        ui.add_space(8.0);
        let presets: &[(f32, &str)] = &[
            (0.80, "80%"),
            (1.00, "100%"),
            (1.25, "125%"),
            (1.50, "150%"),
            (1.75, "175%"),
        ];
        let active_preset = presets
            .iter()
            .position(|(val, _)| (current_scale - *val).abs() <= 0.005);
        let options: Vec<SwitchOption<'_>> = presets
            .iter()
            .map(|(val, label)| {
                let tip = if (*val - 1.0).abs() <= 0.005 {
                    "Default 100% interface scale (Ctrl+0)"
                } else {
                    "Set interface scale"
                };
                SwitchOption::new(None, Some(*label), tip, true)
            })
            .collect();
        let (clicked, _) = segmented_switch(
            ui,
            "inspector_ui_scale_presets",
            &options,
            active_preset,
            SwitchLayout::EqualWidth,
            t,
        );
        if let Some(idx) = clicked
            && let Some((val, _)) = presets.get(idx)
        {
            next_scale = Some(*val);
        }
    });

    if let Some(scale) = next_scale {
        let clean = UserStudioConfig::sanitize_ui_scale(scale);
        let _ = app
            .session
            .execute("theme.set", &json!({ "ui_scale": clean }));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_app_updates_card(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    use papercraft_updater::{UpdateStatus, format_bytes};

    let current_ver = app.updater.config().map_or_else(
        || env!("CARGO_PKG_VERSION").to_string(),
        |cfg| cfg.current_version.to_string(),
    );
    let is_custom_feed = app.updater.config().is_some_and(|cfg| cfg.is_custom_feed);
    let status = app.updater.status().clone();
    let is_busy = app.updater.is_busy();

    card_frame(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        section_card_header(
            ui,
            Icon::MoveDown,
            "App Updates",
            "Check for new official PaperCraft releases and install updates in place.",
            t,
        );

        ui.horizontal(|ui| {
            ui.label(RichText::new("Installed version").size(12.0).color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                Frame::new()
                    .fill(t.panel_bg)
                    .stroke(Stroke::new(1.0, t.border))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(8, 3))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!("v{current_ver}"))
                                .size(11.5)
                                .strong()
                                .monospace()
                                .color(t.text),
                        );
                    });
                if is_custom_feed {
                    Frame::new()
                        .fill(t.accent_soft)
                        .stroke(Stroke::new(1.0, t.accent))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(7, 3))
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
        });

        ui.add_space(8.0);
        let mut auto_check = app.session.prefs.auto_check_updates;
        let toggle_resp = ui
            .checkbox(&mut auto_check, "Check for updates automatically on startup")
            .on_hover_text(
                "Quietly check GitHub Releases for new versions when PaperCraft opens.",
            );
        if toggle_resp.changed() {
            let _ = app
                .session
                .execute("updates.setAutoCheck", &json!({ "enabled": auto_check }));
        }
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Checks GitHub Releases quietly when PaperCraft starts. Turn off for completely offline use.",
                )
                .size(11.0)
                .color(t.text_muted),
            )
            .wrap(),
        );

        ui.add_space(8.0);
        let mut desktop_shortcut = app.session.prefs.auto_create_desktop_shortcut;
        let shortcut_resp = crate::icons::toggle_switch_row(
            ui,
            "settings_desktop_shortcut_switch",
            "Create Desktop shortcut automatically",
            "Keep a Desktop shortcut pointing to PaperCraft and refresh it automatically after updates.",
            &mut desktop_shortcut,
            t,
        );
        if shortcut_resp.changed() {
            app.set_desktop_shortcut_enabled(desktop_shortcut);
        }
        ui.add(
            egui::Label::new(
                RichText::new(format!(
                    "{} Also refreshed automatically whenever PaperCraft updates. Place the app in the permanent folder where you want to keep it.",
                    papercraft_updater::shortcut_architecture_hint()
                ))
                .size(11.0)
                .color(t.text_muted),
            )
            .wrap(),
        );
        if let Some((is_err, ref msg)) = app.view.desktop_shortcut_feedback {
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(msg)
                        .size(11.0)
                        .color(if is_err { t.danger_fg } else { t.success_fg }),
                )
                .wrap(),
            );
        }

        ui.add_space(8.0);
        let (status_line, status_color) = match &status {
            UpdateStatus::Idle => ("Ready to check for updates.".to_string(), t.text_muted),
            UpdateStatus::Checking { .. } => {
                ("Checking for new releases...".to_string(), t.accent)
            }
            UpdateStatus::UpToDate { latest_version } => (
                papercraft_updater::format_up_to_date_status(
                    None,
                    app.updater.config().map(|cfg| &cfg.current_version),
                    latest_version,
                ),
                t.success_fg,
            ),
            UpdateStatus::Available(u) => (
                format!("Update available: v{}", u.version),
                t.accent,
            ),
            UpdateStatus::Downloading {
                downloaded_bytes,
                total_bytes,
                ..
            } => {
                if *total_bytes > 0 {
                    (
                        format!(
                            "Downloading: {} / {}",
                            format_bytes(*downloaded_bytes),
                            format_bytes(*total_bytes)
                        ),
                        t.accent,
                    )
                } else {
                    (
                        format!("Downloading: {}", format_bytes(*downloaded_bytes)),
                        t.accent,
                    )
                }
            }
            UpdateStatus::ReadyToInstall { update, .. } => (
                format!("v{} verified and ready to install.", update.version),
                t.success_fg,
            ),
            UpdateStatus::NoPlatformAsset { latest_version, .. } => (
                format!("Release v{latest_version} requires manual download."),
                t.warning_fg,
            ),
            UpdateStatus::Error { message, .. } => (message.clone(), t.danger_fg),
        };
        ui.add(egui::Label::new(RichText::new(status_line).size(11.5).color(status_color)).wrap());

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let check_lbl = if matches!(status, UpdateStatus::Checking { .. }) {
                "Checking..."
            } else {
                "Check for Updates"
            };
            if compact_icon_button(ui, Icon::RotateCw, Some(check_lbl), false, !is_busy, t)
                .on_hover_text("Check GitHub Releases for a newer PaperCraft version now")
                .clicked()
            {
                app.trigger_update_check(false, ui.ctx());
            }

            let details_lbl = match &status {
                UpdateStatus::ReadyToInstall { .. } => Some("Install & Restart"),
                UpdateStatus::Available(_) | UpdateStatus::Downloading { .. } => {
                    Some("View Update")
                }
                UpdateStatus::Error { .. } | UpdateStatus::NoPlatformAsset { .. } => {
                    Some("Details")
                }
                _ => None,
            };
            if let Some(lbl) = details_lbl
                && compact_icon_button(ui, Icon::MoveDown, Some(lbl), true, true, t)
                    .on_hover_text("Open the Software Update window")
                    .clicked()
            {
                crate::modal::open(app, crate::modal::ActiveModal::AppUpdate);
            }
        });
    });
}

fn draw_system_info_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    monospace: bool,
    tooltip: Option<&str>,
    t: &Tokens,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(label).size(11.5).color(t.text_muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut text = RichText::new(value).size(11.5).strong().color(t.text);
            if monospace {
                text = text.monospace();
            }
            let resp = ui.add(egui::Label::new(text).truncate());
            if let Some(tip) = tooltip {
                resp.on_hover_text(tip);
            }
        });
    });
}

fn system_platform_label() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        papercraft_updater::current_platform_label()
    }
    #[cfg(target_arch = "wasm32")]
    {
        "Web (WebAssembly)".to_string()
    }
}

fn system_renderer_label(wgpu_enabled: bool) -> &'static str {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if wgpu_enabled {
            "wgpu (Hardware GPU)"
        } else {
            "Default"
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        if wgpu_enabled {
            "wgpu (Browser GPU)"
        } else {
            "Browser Canvas"
        }
    }
}

fn system_storage_info(app: &PaperCraftApp) -> (String, Option<String>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(dir) = app.session.library_dir() {
            let full = dir.display().to_string();
            let short = dir
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|s| !s.is_empty())
                .map_or_else(|| full.clone(), ToString::to_string);
            (short, Some(full))
        } else {
            ("In-memory session".to_string(), None)
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        ("Browser localStorage".to_string(), None)
    }
}

fn system_workspace_info(app: &PaperCraftApp) -> (String, Option<String>) {
    let summary = app.session.sync_status_summary();
    if summary.viewing_cloud {
        let handle = summary
            .username
            .as_deref()
            .map_or_else(|| "Cloud".to_string(), |u| format!("@{u}"));
        let tip = if summary.server_url.trim().is_empty() {
            None
        } else {
            Some(format!("Connected server: {}", summary.server_url))
        };
        (format!("Cloud ({handle})"), tip)
    } else {
        let prof = summary
            .active_local_profile_name
            .as_deref()
            .unwrap_or("Personal");
        (format!("Local ({prof})"), None)
    }
}

fn draw_system_info_card(app: &PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let platform = system_platform_label();
    let renderer = system_renderer_label(app.wgpu_enabled);
    let (workspace, workspace_tip) = system_workspace_info(app);
    let notebooks = format!(
        "{} active · {} in Trash",
        app.session.notebooks.len(),
        app.session.trashed_notebooks.len()
    );
    let (storage, storage_tip) = system_storage_info(app);

    card_frame(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        section_card_header(
            ui,
            Icon::Info,
            "System Info",
            "Installed build, runtime environment, and workspace storage details.",
            t,
        );

        Frame::new()
            .fill(t.panel_bg)
            .corner_radius(CornerRadius::same(7))
            .inner_margin(Margin::symmetric(10, 8))
            .stroke(Stroke::new(1.0, t.border))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 5.0;
                draw_system_info_row(ui, "Version", &version, true, None, t);
                draw_system_info_row(ui, "Platform", &platform, false, None, t);
                draw_system_info_row(ui, "Renderer", renderer, false, None, t);
                draw_system_info_row(
                    ui,
                    "Workspace",
                    &workspace,
                    false,
                    workspace_tip.as_deref(),
                    t,
                );
                draw_system_info_row(ui, "Notebooks", &notebooks, false, None, t);
                draw_system_info_row(ui, "Storage", &storage, true, storage_tip.as_deref(), t);
            });
    });
}

fn draw_shortcuts_card(ui: &mut egui::Ui, t: &Tokens) {
    card_frame(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        section_card_header(
            ui,
            Icon::Info,
            "Keyboard & Canvas Shortcuts",
            "Quick keyboard and pointer shortcuts for switching tools and navigating pages.",
            t,
        );

        let shortcuts = [
            (
                "1 – 7",
                "Switch Fountain, Ball, Brush, Highlighter, Eraser, Pencil, Laser",
            ),
            ("V", "Switch to Select tool (Box marquee or Freeform Lasso)"),
            ("[ / ]", "Decrease / Increase active nib size"),
            (
                "Draw & Hold",
                "Pause at the end of a stroke to snap shapes or lines",
            ),
            (
                "Ctrl+C / X / V / D",
                "Copy, cut, paste, or duplicate selected items",
            ),
            ("Shift + Drag", "Snap current stroke to a straight line"),
            ("Space / Middle", "Pan canvas view"),
            ("Ctrl + Wheel", "Zoom canvas smoothly around cursor"),
            (
                "Ctrl + + / - / 0",
                "Scale app interface larger, smaller, or reset to 100%",
            ),
            ("Ctrl+Z / Ctrl+Y", "Undo / Redo stroke or page edits"),
            ("P", "Open or close Pages sidebar"),
            ("S", "Open or close this Settings drawer"),
            ("H", "Return to Notebooks Library home screen"),
            ("Esc", "Close active customization or export modal"),
        ];
        for (keys, desc) in shortcuts {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(102.0, 0.0),
                    egui::Layout::left_to_right(egui::Align::Min),
                    |ui| {
                        ui.label(
                            RichText::new(keys)
                                .size(10.5)
                                .monospace()
                                .strong()
                                .color(t.accent),
                        );
                    },
                );
                ui.add(egui::Label::new(RichText::new(desc).size(11.0).color(t.text_muted)).wrap());
            });
        }
    });
}

fn draw_settings_header(app: &mut PaperCraftApp, ui: &mut egui::Ui, kind: PenKind, t: &Tokens) {
    ui.horizontal(|ui| {
        ui.set_height(28.0);
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
        paint_icon(ui.painter(), ir, Icon::Settings, t.accent);
        ui.label(RichText::new("Settings").size(17.0).strong().color(t.text));
        if app.view.settings_tab == SettingsTab::Pen {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, Icon::Reset, Some("Reset Tool"), false, true, t)
                    .on_hover_text(
                        "Restore factory default settings for the currently selected pen.",
                    )
                    .clicked()
                {
                    let mut defaults = PenSettings::for_kind(kind);
                    if let Some(size) = UserStudioConfig::DEFAULT_PEN_SIZES.get(kind.index()) {
                        defaults.size = *size;
                    }
                    defaults.color = app.session.pen(kind).color;
                    let _ = app.session.execute(
                        "pen.setSettings",
                        &json!({ "kind": kind, "settings": defaults }),
                    );
                }
            });
        }
    });
    ui.label(
        RichText::new("Hardware stylus shortcuts, studio appearance, and system info.")
            .size(11.5)
            .color(t.text_muted),
    );
    ui.add_space(6.0);

    let system_tip = if cfg!(target_arch = "wasm32") {
        "System info and keyboard shortcuts"
    } else {
        "System info, app updates, and keyboard shortcuts"
    };
    let tabs = [
        (
            SettingsTab::Pen,
            Icon::FountainPen,
            "Pen",
            "Touch palm rejection, stylus detection, hardware button shortcuts, and active pen reset",
        ),
        (
            SettingsTab::Appearance,
            Icon::Palette,
            "Appearance",
            "Studio theme, app interface scale, and automatic canvas color contrast",
        ),
        (SettingsTab::System, Icon::Info, "System", system_tip),
    ];
    let active_idx = tabs
        .iter()
        .position(|(tab, _, _, _)| *tab == app.view.settings_tab);
    let options: Vec<SwitchOption<'_>> = tabs
        .iter()
        .map(|(_, icon, label, tip)| SwitchOption::new(Some(*icon), Some(label), tip, true))
        .collect();
    let (clicked, _) = segmented_switch(
        ui,
        "inspector_settings_tab_switch",
        &options,
        active_idx,
        SwitchLayout::EqualWidth,
        t,
    );
    if let Some(idx) = clicked
        && let Some((tab, _, _, _)) = tabs.get(idx)
    {
        app.view.settings_tab = *tab;
    }
}

pub(crate) fn export_target_with_ext(stem: &str, ext: &str) -> String {
    let trimmed = stem.trim();
    let base = trimmed
        .strip_suffix(".pdf")
        .or_else(|| trimmed.strip_suffix(".svg"))
        .or_else(|| trimmed.strip_suffix(".png"))
        .or_else(|| trimmed.strip_suffix(".pcraft"))
        .or_else(|| trimmed.strip_suffix(".ncraft"))
        .unwrap_or(trimmed);
    let clean = if base.trim().is_empty() {
        "exports/Quick_Notes"
    } else {
        base.trim()
    };
    format!("{clean}.{ext}")
}

pub(crate) fn title_to_export_stem(title: &str) -> String {
    let mut slug = String::with_capacity(title.len());
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
        } else if (ch == ' ' || ch == '-' || ch == '_') && !slug.ends_with('_') {
            slug.push('_');
        }
    }
    let clean = slug.trim_matches('_');
    if clean.is_empty() {
        "exports/Quick_Notes".to_string()
    } else {
        format!("exports/{clean}")
    }
}

pub(crate) fn title_to_default_filename(title: &str, ext: &str) -> String {
    let stem = title_to_export_stem(title);
    let base = stem.strip_prefix("exports/").unwrap_or(&stem);
    let clean_ext = ext.trim_start_matches('.');
    if clean_ext.is_empty() {
        base.to_string()
    } else {
        format!("{base}.{clean_ext}")
    }
}

pub(crate) const DEFAULT_INSPECTOR_WIDTH: f32 = 372.0;

pub fn show(app: &mut PaperCraftApp, ui: &mut egui::Ui) {
    let _ = show_inner(app, ui);
}

pub(crate) fn show_inner(app: &mut PaperCraftApp, ui: &mut egui::Ui) -> Rect {
    let t = Tokens::for_theme(app.session.prefs.theme);
    let prev_open = app.view.inspector_open;
    let mut panel_open = prev_open;
    let reveal_id = ui.make_persistent_id("papercraft_inspector_reveal");
    let reveal = crate::motion::panel_reveal_factor(ui.ctx(), reveal_id, panel_open);
    let content_opacity = crate::motion::panel_content_opacity(reveal);

    let frame = Frame::new()
        .fill(t.panel_bg)
        .inner_margin(Margin::same(12))
        .stroke(Stroke::new(1.0, t.border));

    let rect = crate::fixed_side_panel(
        ui,
        egui::Panel::right("papercraft_inspector").drag_to_open(false),
        DEFAULT_INSPECTOR_WIDTH,
        frame,
    )
    .show_collapsible(ui, &mut panel_open, |ui| {
        ui.multiply_opacity(content_opacity);
        let kind = app.session.active_pen_kind();
        draw_settings_header(app, ui, kind, &t);
        ui.separator();

        let dt = ui.ctx().input(|i| i.stable_dt);
        if app
            .view
            .motion
            .settings_tab
            .advance(app.view.settings_tab, dt)
        {
            ui.ctx().request_repaint();
        }
        let tab_opacity = app.view.motion.settings_tab.opacity();

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.scope(|ui| {
                ui.multiply_opacity(tab_opacity);
                match app.view.settings_tab {
                    SettingsTab::Pen => {
                        draw_touch_and_palm_card(app, ui, &t);
                        ui.add_space(8.0);
                        if app.is_stylus_connected() {
                            draw_stylus_shortcuts_card(app, ui, &t);
                        } else {
                            draw_stylus_disconnected_card(ui, &t);
                        }
                    }
                    SettingsTab::Appearance => {
                        draw_appearance_card(app, ui, &t);
                        ui.add_space(8.0);
                        draw_app_scale_card(app, ui, &t);
                    }
                    SettingsTab::System => {
                        draw_system_info_card(app, ui, &t);
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            ui.add_space(8.0);
                            draw_app_updates_card(app, ui, &t);
                        }
                        ui.add_space(8.0);
                        draw_shortcuts_card(ui, &t);
                    }
                }
            });
        });
    })
    .map_or(Rect::NOTHING, |resp| resp.response.rect);
    if panel_open != prev_open {
        app.view.inspector_open = panel_open;
    }
    rect
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod auto_contrast_ui_tests {
    use super::*;
    use crate::{AppScreen, SettingsTab, motion};
    use egui::{Pos2, RawInput, Rect, Shape, Vec2};
    use papercraft_color::Color;
    use papercraft_doc::{PaperColor, UiTheme};
    use serde_json::json;

    pub(super) fn find_text_rect_in_shapes(
        shapes: &[egui::epaint::ClippedShape],
        needle: &str,
    ) -> Option<Rect> {
        fn visit(shape: &Shape, needle: &str) -> Option<Rect> {
            match shape {
                Shape::Text(t) if t.galley.text().contains(needle) => {
                    Some(t.galley.rect.translate(t.pos.to_vec2()))
                }
                Shape::Vec(list) => list.iter().find_map(|s| visit(s, needle)),
                _ => None,
            }
        }
        shapes.iter().find_map(|cs| visit(&cs.shape, needle))
    }

    pub(super) fn run_editor_frame(
        ctx: &egui::Context,
        app: &mut PaperCraftApp,
        mut input: RawInput,
        time_s: &mut f64,
    ) -> Vec<egui::epaint::ClippedShape> {
        *time_s += 0.5;
        input.time = Some(*time_s);
        let mut out = ctx.run_ui(input, |ui| app.show_ui(ui));
        let shapes = std::mem::take(&mut out.shapes);
        out.drop_without_applying_deltas();
        shapes
    }

    pub(super) fn base_input() -> RawInput {
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 820.0))),
            ..Default::default()
        }
    }

    pub(super) fn click_at(
        ctx: &egui::Context,
        app: &mut PaperCraftApp,
        pos: Pos2,
        time_s: &mut f64,
    ) -> Vec<egui::epaint::ClippedShape> {
        let mut down = base_input();
        down.events.push(egui::Event::PointerMoved(pos));
        down.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = run_editor_frame(ctx, app, down, time_s);

        let mut up = base_input();
        up.events.push(egui::Event::PointerMoved(pos));
        up.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = run_editor_frame(ctx, app, up, time_s);
        run_editor_frame(ctx, app, base_input(), time_s)
    }

    fn sample_opaque_stroke_color(
        app: &PaperCraftApp,
        stroke_id: papercraft_doc::StrokeId,
    ) -> Color {
        let (_, _, ribbon) = app
            .mesh_cache
            .get(&stroke_id)
            .expect("rendered stroke mesh should be cached");
        let v = ribbon
            .vertices
            .iter()
            .max_by(|a, b| a.rgba[3].total_cmp(&b.rgba[3]))
            .expect("mesh should have vertices");
        Color::new(v.rgba[0], v.rgba[1], v.rgba[2], v.rgba[3])
    }

    #[test]
    fn appearance_checkbox_toggles_auto_contrast_and_recolors_green_ink_in_dark_theme() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        app.view.screen = AppScreen::Editor;
        app.view.motion.screen = motion::ScreenTransition::new(AppScreen::Editor);
        app.view.inspector_open = true;
        app.view.settings_tab = SettingsTab::Appearance;

        // Draw a Forest Green stroke (#1D7A46) on WarmCream paper, then switch to Dark theme.
        let green_hex = "#1D7A46";
        let green = Color::from_hex(green_hex).unwrap();
        app.session
            .execute("tool.select", &json!({ "tool": "FountainPen" }))
            .unwrap();
        app.session
            .execute(
                "pen.setSettings",
                &json!({ "kind": "FountainPen", "color_hex": green_hex }),
            )
            .unwrap();
        let draw_res = app
            .session
            .execute(
                "stroke.draw",
                &json!({
                    "kind": "FountainPen",
                    "points": [
                        { "x": 120.0, "y": 140.0, "pressure": 1.0, "time_ms": 0.0 },
                        { "x": 200.0, "y": 150.0, "pressure": 1.0, "time_ms": 16.0 },
                        { "x": 280.0, "y": 160.0, "pressure": 1.0, "time_ms": 32.0 }
                    ]
                }),
            )
            .unwrap();
        let stroke_id = papercraft_doc::StrokeId(draw_res["strokeId"].as_u64().unwrap());

        app.session
            .execute("theme.set", &json!({ "theme": "Dark" }))
            .unwrap();
        assert!(!app.session.prefs.auto_contrast_with_theme);

        // With checkbox default OFF, Dark theme leaves green ink canonical (#1D7A46).
        let mut time_s = 1.0_f64;
        let _ = run_editor_frame(&ctx, &mut app, base_input(), &mut time_s);
        let shapes = run_editor_frame(&ctx, &mut app, base_input(), &mut time_s);
        let check_rect = find_text_rect_in_shapes(&shapes, "Auto-adapt canvas colors to theme")
            .expect("Appearance checkbox label should be rendered on Appearance settings tab");
        let off_color = sample_opaque_stroke_color(&app, stroke_id);
        assert!(
            (off_color.r - green.r).abs() < 0.02
                && (off_color.g - green.g).abs() < 0.02
                && (off_color.b - green.b).abs() < 0.02,
            "when checkbox is OFF, ink must remain canonical green, got {:?}",
            off_color
        );

        // Click checkbox ON -> adapts paper to dark and lifts green ink for high contrast.
        let shapes_on = click_at(&ctx, &mut app, check_rect.center(), &mut time_s);
        assert!(app.session.prefs.auto_contrast_with_theme);
        assert_eq!(app.session.prefs.theme, UiTheme::Dark);

        let dark_bg = PaperColor::WarmCream
            .resolved_colors(UiTheme::Dark, true)
            .background;
        let on_color = sample_opaque_stroke_color(&app, stroke_id);
        assert!(
            on_color.g > on_color.r && on_color.g > on_color.b,
            "adapted ink must stay dominantly green, got {:?}",
            on_color
        );
        assert!(
            papercraft_color::contrast_ratio(on_color, dark_bg) >= 6.49,
            "adapted green contrast ratio on dark paper too low: {:.2}",
            papercraft_color::contrast_ratio(on_color, dark_bg)
        );

        // Click checkbox OFF again -> immediately invalidates cache and restores canonical ink.
        let check_rect_on =
            find_text_rect_in_shapes(&shapes_on, "Auto-adapt canvas colors to theme")
                .expect("Appearance checkbox label should remain visible after toggle");
        let _ = click_at(&ctx, &mut app, check_rect_on.center(), &mut time_s);
        assert!(!app.session.prefs.auto_contrast_with_theme);
        let restored = sample_opaque_stroke_color(&app, stroke_id);
        assert!(
            (restored.r - green.r).abs() < 0.02
                && (restored.g - green.g).abs() < 0.02
                && (restored.b - green.b).abs() < 0.02
        );
    }

    #[test]
    fn settings_three_way_tab_switch_cycles_between_pen_appearance_and_system() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        app.view.screen = AppScreen::Editor;
        app.view.motion.screen = motion::ScreenTransition::new(AppScreen::Editor);
        app.view.inspector_open = true;
        app.view.settings_tab = SettingsTab::Pen;

        let mut time_s = 1.0_f64;
        let _ = run_editor_frame(&ctx, &mut app, base_input(), &mut time_s);
        let shapes_pen = run_editor_frame(&ctx, &mut app, base_input(), &mut time_s);
        assert!(find_text_rect_in_shapes(&shapes_pen, "Stylus detection").is_some());
        assert!(find_text_rect_in_shapes(&shapes_pen, "Input stabilization").is_none());

        let appearance_tab = find_text_rect_in_shapes(&shapes_pen, "Appearance")
            .expect("Appearance tab should be rendered in 3-way switch");
        let shapes_app = click_at(&ctx, &mut app, appearance_tab.center(), &mut time_s);
        assert_eq!(app.view.settings_tab, SettingsTab::Appearance);
        assert!(
            find_text_rect_in_shapes(&shapes_app, "Auto-adapt canvas colors to theme").is_some()
        );
        assert!(find_text_rect_in_shapes(&shapes_app, "App Updates").is_none());

        let system_tab = find_text_rect_in_shapes(&shapes_app, "System")
            .expect("System tab should be rendered in 3-way switch");
        let shapes_sys = click_at(&ctx, &mut app, system_tab.center(), &mut time_s);
        assert_eq!(app.view.settings_tab, SettingsTab::System);
        assert!(find_text_rect_in_shapes(&shapes_sys, "System Info").is_some());
        #[cfg(not(target_arch = "wasm32"))]
        assert!(find_text_rect_in_shapes(&shapes_sys, "App Updates").is_some());

        let pen_tab = find_text_rect_in_shapes(&shapes_sys, "Pen")
            .expect("Pen tab should be rendered in 3-way switch");
        let shapes_back = click_at(&ctx, &mut app, pen_tab.center(), &mut time_s);
        assert_eq!(app.view.settings_tab, SettingsTab::Pen);
        assert!(find_text_rect_in_shapes(&shapes_back, "Stylus detection").is_some());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn collect_text_shapes(shapes: &[egui::epaint::ClippedShape]) -> String {
        let mut out = String::new();
        fn visit(shape: &egui::Shape, buf: &mut String) {
            match shape {
                egui::Shape::Text(text) => {
                    buf.push_str(text.galley.text());
                    buf.push('\n');
                }
                egui::Shape::Vec(items) => {
                    for s in items {
                        visit(s, buf);
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

    fn render_inspector_tab(app: &mut PaperCraftApp, tab: SettingsTab) -> (Rect, String) {
        app.view.inspector_open = true;
        app.view.settings_tab = tab;
        let ctx = egui::Context::default();
        let mut last_texts = String::new();
        let mut panel_rect = Rect::NOTHING;
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                panel_rect = show_inner(app, ui);
            });
            let has_shapes = !out.shapes.is_empty();
            last_texts = collect_text_shapes(&out.shapes);
            out.drop_without_applying_deltas();
            assert!(has_shapes);
            assert!(
                (panel_rect.width() - DEFAULT_INSPECTOR_WIDTH).abs() <= 0.5
                    && panel_rect.right() <= 1280.5,
                "tab {tab:?} inspector rect {panel_rect:?} overflowed or differed from default {DEFAULT_INSPECTOR_WIDTH}",
            );
        }
        assert!(last_texts.lines().any(|line| line == "Pen"));
        assert!(last_texts.lines().any(|line| line == "Appearance"));
        assert!(last_texts.lines().any(|line| line == "System"));
        assert!(!last_texts.lines().any(|line| line == "Telemetry"));
        assert!(!last_texts.lines().any(|line| line == "Page"));
        assert!(!last_texts.contains("Input stabilization"));
        assert!(!last_texts.contains("Smoothing strength"));
        assert!(!last_texts.contains("Smoothing style"));
        (panel_rect, last_texts)
    }

    #[test]
    fn system_tab_renders_system_info_app_updates_and_shortcuts_without_telemetry() {
        let mut app = PaperCraftApp::default();
        let (_, sys_texts) = render_inspector_tab(&mut app, SettingsTab::System);
        assert!(sys_texts.contains("System Info"));
        assert!(sys_texts.contains("Version"));
        assert!(sys_texts.contains(&format!("v{}", env!("CARGO_PKG_VERSION"))));
        assert!(sys_texts.contains("Platform"));
        assert!(sys_texts.contains("Renderer"));
        assert!(sys_texts.contains("Workspace"));
        assert!(sys_texts.contains("Notebooks"));
        assert!(sys_texts.contains("Storage"));
        #[cfg(not(target_arch = "wasm32"))]
        {
            assert!(sys_texts.contains("App Updates"));
            assert!(sys_texts.contains("Check for updates automatically on startup"));
        }
        assert!(sys_texts.contains("Keyboard & Canvas Shortcuts"));
        assert!(!sys_texts.contains("Live Stylus Telemetry"));
        assert!(!sys_texts.contains("Hardware Stylus Active"));
        assert!(!sys_texts.contains("Auto-adapt canvas colors to theme"));
        assert!(!sys_texts.contains("Paper Ruling"));
    }

    #[test]
    fn appearance_tab_renders_theme_switch_and_auto_contrast_without_app_updates() {
        let mut app = PaperCraftApp::default();
        let (_, app_texts) = render_inspector_tab(&mut app, SettingsTab::Appearance);
        assert!(app_texts.contains("Appearance"));
        assert!(app_texts.contains("Light"));
        assert!(app_texts.contains("Dark"));
        assert!(app_texts.contains("Auto-adapt canvas colors to theme"));
        assert!(app_texts.contains("App Scale"));
        assert!(app_texts.contains("100%"));
        assert!(app_texts.contains("80%"));
        assert!(app_texts.contains("125%"));
        assert!(app_texts.contains("150%"));
        assert!(app_texts.contains("175%"));
        assert!(!app_texts.contains("App Updates"));
        assert!(!app_texts.contains("System Info"));
        assert!(!app_texts.contains("Live Stylus Telemetry"));
        assert!(!app_texts.contains("Keyboard & Canvas Shortcuts"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn app_updates_checkbox_persists_auto_check_preference() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        app.view.screen = crate::AppScreen::Editor;
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
        app.view.inspector_open = true;
        app.view.settings_tab = SettingsTab::System;
        assert!(app.session.prefs.auto_check_updates);

        let mut time_s = 1.0_f64;
        let _ = auto_contrast_ui_tests::run_editor_frame(
            &ctx,
            &mut app,
            auto_contrast_ui_tests::base_input(),
            &mut time_s,
        );
        let shapes = auto_contrast_ui_tests::run_editor_frame(
            &ctx,
            &mut app,
            auto_contrast_ui_tests::base_input(),
            &mut time_s,
        );
        let check_rect = auto_contrast_ui_tests::find_text_rect_in_shapes(
            &shapes,
            "Check for updates automatically on startup",
        )
        .expect("Auto-check updates checkbox should render");
        let _ = auto_contrast_ui_tests::click_at(&ctx, &mut app, check_rect.center(), &mut time_s);
        assert!(!app.session.prefs.auto_check_updates);
    }

    fn find_text_center(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<egui::Pos2> {
        fn visit(shape: &egui::Shape, needle: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == needle => {
                    Some(text.pos + text.galley.rect.center().to_vec2())
                }
                egui::Shape::Vec(items) => items.iter().find_map(|s| visit(s, needle)),
                _ => None,
            }
        }
        shapes.iter().find_map(|c| visit(&c.shape, needle))
    }

    #[test]
    fn settings_header_has_pen_appearance_and_system_tabs_on_single_row() {
        let mut app = PaperCraftApp::default();
        app.view.inspector_open = true;
        let ctx = egui::Context::default();
        let mut shapes = Vec::new();
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                let _ = show_inner(&mut app, ui);
            });
            shapes = out.shapes.clone();
            out.drop_without_applying_deltas();
        }
        let pen_pos = find_text_center(&shapes, "Pen").unwrap();
        let app_pos = find_text_center(&shapes, "Appearance").unwrap();
        let sys_pos = find_text_center(&shapes, "System").unwrap();
        assert!((pen_pos.y - app_pos.y).abs() <= 1.0);
        assert!((pen_pos.y - sys_pos.y).abs() <= 1.0);
        assert!(pen_pos.x < app_pos.x && app_pos.x < sys_pos.x);
        assert!(find_text_center(&shapes, "Telemetry").is_none());
        assert!(find_text_center(&shapes, "Reset Tool").is_some());
        assert!(find_text_center(&shapes, "Page").is_none());
    }

    #[test]
    fn app_scale_presets_and_reset_update_session_scale() {
        let mut app = PaperCraftApp::default();
        app.view.inspector_open = true;
        app.view.settings_tab = SettingsTab::Appearance;
        let ctx = egui::Context::default();

        let render_step =
            |ctx: &egui::Context, app: &mut PaperCraftApp, t: f64, events: Vec<egui::Event>| {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(t),
                    events,
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    let _ = show_inner(app, ui);
                });
                let shapes = out.shapes.clone();
                out.drop_without_applying_deltas();
                shapes
            };

        let mut shapes = Vec::new();
        for f in 0..4 {
            shapes = render_step(&ctx, &mut app, f as f64 * 0.03, Vec::new());
        }
        let preset_125 = find_text_center(&shapes, "125%").expect("125% preset button");

        for (pressed, t) in [(true, 0.20), (false, 0.24)] {
            let _ = render_step(
                &ctx,
                &mut app,
                t,
                vec![
                    egui::Event::PointerMoved(preset_125),
                    egui::Event::PointerButton {
                        pos: preset_125,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!((app.session.prefs.ui_scale - 1.25).abs() < 1e-4);
        shapes = render_step(&ctx, &mut app, 0.28, Vec::new());
        let reset_pos = find_text_center(&shapes, "Reset").expect("Reset button");

        for (pressed, t) in [(true, 0.35), (false, 0.39)] {
            let _ = render_step(
                &ctx,
                &mut app,
                t,
                vec![
                    egui::Event::PointerMoved(reset_pos),
                    egui::Event::PointerButton {
                        pos: reset_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!((app.session.prefs.ui_scale - 1.0).abs() < 1e-4);
    }

    #[test]
    fn stylus_shortcuts_card_visibility_hot_unplug_dropdown_and_reset() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let ctx = egui::Context::default();
        let connected = Arc::new(AtomicBool::new(false));
        let connected_hook = Arc::clone(&connected);

        let hooks = crate::PlatformHooks {
            connected_pen_devices: Some(Box::new(move || {
                if connected_hook.load(Ordering::Relaxed) {
                    vec![crate::stylus::ConnectedStylusInfo {
                        name: "Active Pen Digitizer".to_string(),
                        is_external: true,
                    }]
                } else {
                    Vec::new()
                }
            })),
            ..crate::PlatformHooks::default()
        };
        let mut app = PaperCraftApp::new(papercraft_engine::Session::default(), hooks);
        app.view.screen = crate::AppScreen::Editor;
        app.view.inspector_open = true;
        app.view.settings_tab = SettingsTab::Pen;

        let render_step =
            |ctx: &egui::Context, app: &mut PaperCraftApp, t: f64, events: Vec<egui::Event>| {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 1300.0),
                    )),
                    time: Some(t),
                    events,
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    if app.last_ctx.is_none() {
                        app.last_ctx = Some(ui.ctx().clone());
                    }
                    app.poll_connected_stylus_devices(t);
                    let _ = show_inner(app, ui);
                });
                let texts = collect_text_shapes(&out.shapes);
                let shapes = out.shapes.clone();
                out.drop_without_applying_deltas();
                (texts, shapes)
            };

        // 1. Disconnected: Pen tab shows Stylus detection status and hides Stylus shortcuts.
        let (texts_off, _) = render_step(&ctx, &mut app, 0.02, Vec::new());
        assert!(texts_off.contains("Stylus detection"));
        assert!(texts_off.contains("No stylus detected"));
        assert!(!texts_off.contains("Stylus shortcuts"));
        assert!(!texts_off.contains("Input stabilization"));

        // 2. Connect stylus and re-poll after interval: card appears with device name and 3 rows.
        connected.store(true, Ordering::Relaxed);
        app.stylus.telemetry.barrel_held = true;
        let (texts_on, _) = render_step(&ctx, &mut app, 2.0, Vec::new());
        assert!(texts_on.contains("Stylus shortcuts"));
        assert!(texts_on.contains("Connected · Active Pen Digitizer"));
        assert!(texts_on.contains("Side Button 1"));
        assert!(texts_on.contains("Side Button 2"));
        assert!(texts_on.contains("Eraser End"));
        assert!(texts_on.contains("Pressed"));

        // 3. Open Side Button 1 dropdown ("Eraser (Hold)") and pick "Undo".
        app.stylus.telemetry.barrel_held = false;
        let (_, shapes_idle) = render_step(&ctx, &mut app, 2.05, Vec::new());
        let combo_pos =
            find_text_center(&shapes_idle, "Eraser (Hold)").expect("Side Button 1 combo box");

        for (pressed, t) in [(true, 2.10), (false, 2.14)] {
            let _ = render_step(
                &ctx,
                &mut app,
                t,
                vec![
                    egui::Event::PointerMoved(combo_pos),
                    egui::Event::PointerButton {
                        pos: combo_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        let (_, shapes_popup) = render_step(&ctx, &mut app, 2.18, Vec::new());
        let undo_pos = find_text_center(&shapes_popup, "Undo").expect("Undo dropdown item");
        for (pressed, t) in [(true, 2.22), (false, 2.26)] {
            let _ = render_step(
                &ctx,
                &mut app,
                t,
                vec![
                    egui::Event::PointerMoved(undo_pos),
                    egui::Event::PointerButton {
                        pos: undo_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(
            app.session.prefs.stylus_buttons.barrel_button,
            StylusButtonAction::Undo
        );

        // 4. Click Reset button inside Stylus shortcuts card -> restores default config.
        let (_, shapes_custom) = render_step(&ctx, &mut app, 2.30, Vec::new());
        let reset_pos = find_text_center(&shapes_custom, "Reset").expect("Reset shortcuts button");
        for (pressed, t) in [(true, 2.34), (false, 2.38)] {
            let _ = render_step(
                &ctx,
                &mut app,
                t,
                vec![
                    egui::Event::PointerMoved(reset_pos),
                    egui::Event::PointerButton {
                        pos: reset_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(
            app.session.prefs.stylus_buttons,
            StylusButtonConfig::default()
        );

        // 5. Hot-unplugging hides Stylus shortcuts and restores detection prompt.
        connected.store(false, Ordering::Relaxed);
        let (texts_unplugged, _) = render_step(&ctx, &mut app, 4.5, Vec::new());
        assert!(!texts_unplugged.contains("Stylus shortcuts"));
        assert!(texts_unplugged.contains("No stylus detected"));
    }

    #[test]
    fn touch_and_palm_card_renders_when_disconnected_and_toggles_stylus_only_drawing() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        app.view.screen = crate::AppScreen::Editor;
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
        app.view.inspector_open = true;
        app.view.settings_tab = SettingsTab::Pen;
        assert!(!app.is_stylus_connected());
        assert!(!app.session.prefs.stylus_only_drawing);

        let mut time_s = 1.0_f64;
        let _ = auto_contrast_ui_tests::run_editor_frame(
            &ctx,
            &mut app,
            auto_contrast_ui_tests::base_input(),
            &mut time_s,
        );
        let shapes = auto_contrast_ui_tests::run_editor_frame(
            &ctx,
            &mut app,
            auto_contrast_ui_tests::base_input(),
            &mut time_s,
        );
        assert!(
            auto_contrast_ui_tests::find_text_rect_in_shapes(&shapes, "Touch & Palm Rejection")
                .is_some()
        );
        let check_rect =
            auto_contrast_ui_tests::find_text_rect_in_shapes(&shapes, "Draw with stylus only")
                .expect("Draw with stylus only checkbox should render on Pen tab");

        let shapes_on =
            auto_contrast_ui_tests::click_at(&ctx, &mut app, check_rect.center(), &mut time_s);
        assert!(app.session.prefs.stylus_only_drawing);

        let check_rect_on =
            auto_contrast_ui_tests::find_text_rect_in_shapes(&shapes_on, "Draw with stylus only")
                .expect("Draw with stylus only checkbox should remain visible when enabled");
        let _ =
            auto_contrast_ui_tests::click_at(&ctx, &mut app, check_rect_on.center(), &mut time_s);
        assert!(!app.session.prefs.stylus_only_drawing);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn desktop_shortcut_switch_renders_on_system_tab_and_invokes_hook() {
        use std::sync::{Arc, Mutex};

        let calls = Arc::new(Mutex::new(Vec::<bool>::new()));
        let calls_hook = Arc::clone(&calls);

        let ctx = egui::Context::default();
        let hooks = crate::PlatformHooks {
            sync_desktop_shortcut: Some(Box::new(move |enabled| {
                calls_hook
                    .lock()
                    .map_err(|err| err.to_string())?
                    .push(enabled);
                Ok(if enabled {
                    "Desktop shortcut active (mock).".to_string()
                } else {
                    "Desktop shortcut removed (mock).".to_string()
                })
            })),
            ..crate::PlatformHooks::default()
        };
        let mut app = PaperCraftApp::new(papercraft_engine::Session::default(), hooks);
        app.view.screen = crate::AppScreen::Editor;
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
        app.view.inspector_open = true;
        app.view.settings_tab = SettingsTab::System;
        assert!(app.session.prefs.auto_create_desktop_shortcut);

        let mut time_s = 1.0_f64;
        let _ = auto_contrast_ui_tests::run_editor_frame(
            &ctx,
            &mut app,
            auto_contrast_ui_tests::base_input(),
            &mut time_s,
        );
        let shapes = auto_contrast_ui_tests::run_editor_frame(
            &ctx,
            &mut app,
            auto_contrast_ui_tests::base_input(),
            &mut time_s,
        );
        let switch_label = auto_contrast_ui_tests::find_text_rect_in_shapes(
            &shapes,
            "Create Desktop shortcut automatically",
        )
        .expect("Desktop shortcut switch should render in System settings");

        let shapes_off =
            auto_contrast_ui_tests::click_at(&ctx, &mut app, switch_label.center(), &mut time_s);
        assert!(!app.session.prefs.auto_create_desktop_shortcut);
        assert!(
            auto_contrast_ui_tests::find_text_rect_in_shapes(
                &shapes_off,
                "Desktop shortcut removed (mock)."
            )
            .is_some()
        );

        let switch_label_off = auto_contrast_ui_tests::find_text_rect_in_shapes(
            &shapes_off,
            "Create Desktop shortcut automatically",
        )
        .expect("Desktop shortcut switch should stay visible when turned off");
        let shapes_on = auto_contrast_ui_tests::click_at(
            &ctx,
            &mut app,
            switch_label_off.center(),
            &mut time_s,
        );
        assert!(app.session.prefs.auto_create_desktop_shortcut);
        assert!(
            auto_contrast_ui_tests::find_text_rect_in_shapes(
                &shapes_on,
                "Desktop shortcut active (mock)."
            )
            .is_some()
        );
        assert_eq!(*calls.lock().unwrap(), vec![false, true]);
    }
}
