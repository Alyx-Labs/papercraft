//! Shared low-noise UI primitives, formatting helpers, and clipboard guards.

use egui::{Color32, CornerRadius, Margin, Sense, Stroke, Vec2};

use crate::controller::{NoticeBanner, NoticeTone, ServerStatus};
use crate::icons::{Icon, icon_button, paint_icon};
use crate::theme::Tokens;

pub const CARD_RADIUS: f32 = 10.0;
pub const CONTROL_RADIUS: f32 = 7.0;
pub const INPUT_HEIGHT: f32 = 30.0;
pub const COMPACT_HEADER_BREAKPOINT: f32 = 600.0;

pub fn draw_section_panel(ui: &mut egui::Ui, t: &Tokens, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(t.panel_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CARD_RADIUS)
        .inner_margin(Margin::symmetric(18, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            body(ui);
        });
}

pub fn draw_soft_strip(
    ui: &mut egui::Ui,
    fill: Color32,
    stroke: Stroke,
    margin: Margin,
    body: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            body(ui);
        });
}

pub struct PageHeaderBadge<'a> {
    pub label: &'a str,
    pub fill: Color32,
    pub border: Color32,
    pub fg: Color32,
}

pub fn draw_page_header(
    ui: &mut egui::Ui,
    title: &str,
    badge: Option<PageHeaderBadge<'_>>,
    subtitle: &str,
    t: &Tokens,
    right_content: impl FnOnce(&mut egui::Ui),
) {
    let avail_w = ui.available_width();
    if avail_w >= COMPACT_HEADER_BREAKPOINT {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(egui::RichText::new(title).size(18.0).strong().color(t.text));
            if let Some(b) = badge {
                draw_compact_pill(ui, None, b.label, b.fill, b.border, b.fg);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                right_content(ui);
            });
        });
        if !subtitle.is_empty() {
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(egui::RichText::new(subtitle).size(12.0).color(t.text_muted))
                    .wrap(),
            );
        }
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(egui::RichText::new(title).size(17.5).strong().color(t.text));
            if let Some(b) = badge {
                draw_compact_pill(ui, None, b.label, b.fill, b.border, b.fg);
            }
        });
        if !subtitle.is_empty() {
            ui.add_space(3.0);
            ui.add(
                egui::Label::new(egui::RichText::new(subtitle).size(11.5).color(t.text_muted))
                    .wrap(),
            );
        }
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.spacing_mut().item_spacing.y = 6.0;
            right_content(ui);
        });
    }
}

pub fn draw_section_header(ui: &mut egui::Ui, icon: Icon, title: &str, subtitle: &str, t: &Tokens) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 7.0;
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        paint_icon(ui.painter(), ir, icon, t.accent);
        ui.label(egui::RichText::new(title).size(14.0).strong().color(t.text));
    });
    if !subtitle.is_empty() {
        ui.add_space(3.0);
        ui.add(
            egui::Label::new(egui::RichText::new(subtitle).size(11.5).color(t.text_muted)).wrap(),
        );
    }
}

pub fn draw_status_pill(ui: &mut egui::Ui, status: &ServerStatus, on_header: bool, t: &Tokens) {
    let (fill, border, fg, icon, label) = match status {
        ServerStatus::Running => (
            t.success_bg,
            t.success_border,
            t.success_fg,
            Icon::CheckCircle,
            "Running",
        ),
        ServerStatus::Stopped => {
            if on_header {
                (
                    t.header_control_hover,
                    t.header_border,
                    t.header_fg_muted,
                    Icon::Server,
                    "Stopped",
                )
            } else {
                (
                    t.toolbar_bg,
                    t.border,
                    t.text_muted,
                    Icon::Server,
                    "Stopped",
                )
            }
        }
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

pub fn draw_banner(
    ui: &mut egui::Ui,
    banner: &NoticeBanner,
    dismissible: bool,
    t: &Tokens,
) -> bool {
    let mut dismissed = false;
    let (bg, border, fg, icon) = match banner.tone {
        NoticeTone::Info => (t.accent_soft, t.accent, t.text, Icon::Info),
        NoticeTone::Success => (
            t.success_bg,
            t.success_border,
            t.success_fg,
            Icon::CheckCircle,
        ),
        NoticeTone::Warning => (
            t.warning_bg,
            t.warning_border,
            t.warning_fg,
            Icon::AlertCircle,
        ),
        NoticeTone::Error => (t.danger_bg, t.danger_border, t.danger_fg, Icon::AlertCircle),
    };
    egui::Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let close_reserve = if dismissible { 36.0 } else { 0.0 };
            let text_w = (ui.available_width() - 22.0 - close_reserve).max(80.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
                paint_icon(ui.painter(), ir, icon, fg);
                ui.allocate_ui_with_layout(
                    Vec2::new(text_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&banner.text).size(11.5).color(fg),
                            )
                            .wrap(),
                        );
                    },
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

pub fn draw_singleline_input(
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

pub fn draw_metric_cell(
    ui: &mut egui::Ui,
    icon: Option<Icon>,
    label: &str,
    value: &str,
    value_size: f32,
    t: &Tokens,
) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            if let Some(ic) = icon {
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(13.0), Sense::hover());
                paint_icon(ui.painter(), ir, ic, t.text_muted);
            }
            ui.label(egui::RichText::new(label).size(11.0).color(t.text_muted));
        });
        ui.label(
            egui::RichText::new(value)
                .size(value_size)
                .strong()
                .color(t.text),
        );
    });
}

pub fn draw_hairline(ui: &mut egui::Ui, t: &Tokens) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, t.border.gamma_multiply(0.65)),
    );
}

pub fn draw_compact_pill(
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

pub fn draw_interactive_pill(
    ui: &mut egui::Ui,
    icon: Option<Icon>,
    label: &str,
    fill: Color32,
    border: Color32,
    fg: Color32,
    tooltip: &str,
) -> egui::Response {
    let font = egui::FontId::proportional(11.2);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, fg);
    let icon_dim = 12.0_f32;
    let pad_x = 9.0_f32;
    let icon_gap = 5.0_f32;
    let icon_w = if icon.is_some() {
        icon_dim + icon_gap
    } else {
        0.0
    };
    let width = pad_x * 2.0 + icon_w + galley.size().x;
    let height = 24.0_f32;
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let active_border = if resp.hovered() { fg } else { border };
        p.rect_filled(rect, CornerRadius::same(12), fill);
        p.rect_stroke(
            rect,
            CornerRadius::same(12),
            Stroke::new(1.0, active_border),
            egui::StrokeKind::Inside,
        );
        let text_x = match icon {
            Some(ic) => {
                let ir = egui::Rect::from_center_size(
                    egui::pos2(rect.left() + pad_x + icon_dim * 0.5, rect.center().y),
                    Vec2::splat(icon_dim),
                );
                paint_icon(p, ir, ic, fg);
                ir.right() + icon_gap
            }
            None => rect.left() + pad_x,
        };
        let text_pos = egui::pos2(text_x, rect.center().y - galley.size().y * 0.5);
        p.galley(text_pos, galley, fg);
    }
    resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if !tooltip.is_empty() {
        resp = resp.on_hover_text(tooltip);
    }
    resp
}

pub fn measure_compact_pill_width(ui: &egui::Ui, has_icon: bool, label: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(11.0),
        Color32::WHITE,
    );
    let icon_w = if has_icon { 12.0 + 4.0 } else { 0.0 };
    16.0 + icon_w + galley.size().x + 2.0
}

pub struct ActionButtonStyle {
    pub fill: Color32,
    pub hover_fill: Color32,
    pub border: Color32,
    pub fg: Color32,
}

pub fn primary_button(
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

pub fn header_primary_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    enabled: bool,
    t: &Tokens,
) -> egui::Response {
    let style = if t.is_dark {
        ActionButtonStyle {
            fill: t.accent,
            hover_fill: t.accent_hover,
            border: t.accent,
            fg: t.on_accent,
        }
    } else {
        ActionButtonStyle {
            fill: t.header_active_bg,
            hover_fill: t.accent_soft,
            border: t.header_active_bg,
            fg: t.header_active_fg,
        }
    };
    styled_action_button(ui, icon, label, style, enabled)
}

pub fn secondary_button(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    t: &Tokens,
) -> egui::Response {
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

pub fn fixed_secondary_button(
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

pub fn danger_button(
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

pub fn danger_labeled_button_width(ui: &egui::Ui, label: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(12.2),
        Color32::WHITE,
    );
    (9.0 + 15.0 + 6.0 + galley.size().x + 10.0).max(28.0)
}

pub fn danger_labeled_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    selected: bool,
    t: &Tokens,
) -> egui::Response {
    let icon_dim = 15.0_f32;
    let height = 28.0_f32;
    let pad_left = 9.0_f32;
    let gap = 6.0_f32;
    let pad_right = 10.0_f32;
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(12.2),
        t.danger_fg,
    );
    let width = (pad_left + icon_dim + gap + galley.size().x + pad_right).max(28.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let hovered = resp.hovered();
        let fill = if selected {
            t.danger_border.gamma_multiply(0.45)
        } else if hovered {
            t.danger_bg
        } else {
            t.danger_bg.gamma_multiply(0.7)
        };
        let border = if selected || hovered {
            t.danger_fg
        } else {
            t.danger_border
        };
        p.rect_filled(rect, CornerRadius::same(7), fill);
        p.rect_stroke(
            rect,
            CornerRadius::same(7),
            Stroke::new(if selected { 1.5 } else { 1.0 }, border),
            egui::StrokeKind::Inside,
        );
        let ir = egui::Rect::from_center_size(
            egui::pos2(rect.left() + pad_left + icon_dim * 0.5, rect.center().y),
            Vec2::splat(icon_dim),
        );
        paint_icon(p, ir, icon, t.danger_fg);
        let text_pos = egui::pos2(ir.right() + gap, rect.center().y - galley.size().y * 0.5);
        p.galley(text_pos, galley, t.danger_fg);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
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

pub struct ManualConfirmInputSpec<'a> {
    pub id_salt: &'a str,
    pub width: f32,
    pub hint: &'a str,
    pub border_color: Color32,
    pub matched: bool,
}

pub struct ManualConfirmInputResult {
    pub edit_id: egui::Id,
    pub changed: bool,
    pub enter_pressed: bool,
    pub paste_blocked: bool,
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

pub fn draw_manual_confirm_input(
    ui: &mut egui::Ui,
    spec: &ManualConfirmInputSpec<'_>,
    value: &mut String,
    t: &Tokens,
) -> ManualConfirmInputResult {
    let input_width = spec.width.max(0.0).min(ui.available_width().max(0.0));
    let (box_rect, box_resp) =
        ui.allocate_exact_size(Vec2::new(input_width, INPUT_HEIGHT), Sense::hover());
    let edit_id = ui.make_persistent_id(spec.id_salt);
    let has_focus = ui.ctx().memory(|m| m.has_focus(edit_id));
    let stroke = if has_focus {
        Stroke::new(1.5, t.accent)
    } else if box_resp.hovered() {
        Stroke::new(1.0, t.text_muted.gamma_multiply(0.55))
    } else {
        Stroke::new(1.2, spec.border_color)
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
    let trailing_w = if spec.matched { 20.0 } else { 0.0 };
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
            .hint_text(
                egui::RichText::new(spec.hint)
                    .color(t.text_muted)
                    .size(12.0),
            ),
    );
    let enter_pressed = resp.lost_focus() && child.input(|i| i.key_pressed(egui::Key::Enter));
    if spec.matched {
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

pub fn user_monogram(display_name: &str, username: &str) -> String {
    display_name
        .chars()
        .chain(username.chars())
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| "U".to_string(), |c| c.to_uppercase().collect::<String>())
}

pub fn short_user_id(user_id: &str) -> String {
    let prefix: String = user_id.chars().take(8).collect();
    if user_id.chars().count() > 8 {
        format!("{prefix}...")
    } else {
        prefix
    }
}

pub fn format_utc_date(created_at_ms: i64) -> String {
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

pub fn format_trash_remaining(expires_at_ms: i64, now_ms: i64) -> (String, bool) {
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
