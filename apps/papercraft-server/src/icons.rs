//! Semantic Lucide SVG icon registry for PaperCraft Server.

use egui::{
    Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Vec2, pos2,
};

use crate::theme::{MICRO_DURATION_S, Tokens, hover_factor, lerp_color, lerp_f32};

/// Semantic icon variants backed by vendored Lucide 1.72.0 outline SVGs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Server,
    CheckCircle,
    Square,
    RotateCw,
    Reset,
    FolderOpen,
    Copy,
    Check,
    Link,
    Account,
    Notebook,
    Settings,
    Sun,
    Moon,
    ChevronDown,
    ChevronRight,
    AlertCircle,
    Info,
    Close,
    Search,
    Trash,
    MoveDown,
    ExternalLink,
}

fn icon_svg(icon: Icon) -> &'static str {
    match icon {
        Icon::Server => include_str!("../../../crates/ui-egui/assets/icons/lucide/server.svg"),
        Icon::CheckCircle => {
            include_str!("../../../crates/ui-egui/assets/icons/lucide/circle-check.svg")
        }
        Icon::Square => include_str!("../../../crates/ui-egui/assets/icons/lucide/square.svg"),
        Icon::RotateCw => {
            include_str!("../../../crates/ui-egui/assets/icons/lucide/rotate-cw.svg")
        }
        Icon::Reset => include_str!("../../../crates/ui-egui/assets/icons/lucide/rotate-ccw.svg"),
        Icon::FolderOpen => {
            include_str!("../../../crates/ui-egui/assets/icons/lucide/folder-open.svg")
        }
        Icon::Copy => include_str!("../../../crates/ui-egui/assets/icons/lucide/copy.svg"),
        Icon::Check => include_str!("../../../crates/ui-egui/assets/icons/lucide/check.svg"),
        Icon::Link => include_str!("../../../crates/ui-egui/assets/icons/lucide/link.svg"),
        Icon::Account => {
            include_str!("../../../crates/ui-egui/assets/icons/lucide/user-round.svg")
        }
        Icon::Notebook => {
            include_str!("../../../crates/ui-egui/assets/icons/lucide/book-open.svg")
        }
        Icon::Settings => {
            include_str!("../../../crates/ui-egui/assets/icons/lucide/settings-2.svg")
        }
        Icon::Sun => include_str!("../../../crates/ui-egui/assets/icons/lucide/sun.svg"),
        Icon::Moon => include_str!("../../../crates/ui-egui/assets/icons/lucide/moon.svg"),
        Icon::ChevronDown => {
            include_str!("../../../crates/ui-egui/assets/icons/lucide/chevron-down.svg")
        }
        Icon::ChevronRight => {
            include_str!("../../../crates/ui-egui/assets/icons/lucide/chevron-right.svg")
        }
        Icon::AlertCircle => {
            include_str!("../../../crates/ui-egui/assets/icons/lucide/circle-exclamation-point.svg")
        }
        Icon::Info => include_str!("../../../crates/ui-egui/assets/icons/lucide/info.svg"),
        Icon::Close => include_str!("../../../crates/ui-egui/assets/icons/lucide/x.svg"),
        Icon::Search => include_str!("../../../crates/ui-egui/assets/icons/lucide/search.svg"),
        Icon::Trash => include_str!("../../../crates/ui-egui/assets/icons/lucide/trash.svg"),
        Icon::MoveDown => {
            include_str!("../../../crates/ui-egui/assets/icons/lucide/arrow-down.svg")
        }
        Icon::ExternalLink => {
            include_str!("../../../crates/ui-egui/assets/icons/lucide/arrow-up-right.svg")
        }
    }
}

#[derive(Clone)]
enum CachedIconTexture {
    Ready(egui::TextureHandle),
    Failed,
}

fn icon_texture(ctx: &egui::Context, icon: Icon) -> Option<egui::TextureHandle> {
    let cache_id = egui::Id::unique(("pc-server-lucide", icon));
    if let Some(cached) = ctx.data_mut(|data| data.get_temp::<CachedIconTexture>(cache_id)) {
        return match cached {
            CachedIconTexture::Ready(texture) => Some(texture),
            CachedIconTexture::Failed => None,
        };
    }

    let svg = icon_svg(icon).replace("currentColor", "#ffffff");
    let image = match egui_extras::image::load_svg_bytes_with_size(
        svg.as_bytes(),
        egui::SizeHint::Width(72),
        &Default::default(),
    ) {
        Ok(image) => image,
        Err(error) => {
            log::warn!("Could not render embedded Lucide SVG {icon:?}: {error}");
            ctx.data_mut(|data| data.insert_temp(cache_id, CachedIconTexture::Failed));
            return None;
        }
    };

    let texture = ctx.load_texture(
        format!("pc-server-lucide-{icon:?}"),
        image,
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| {
        data.insert_temp(cache_id, CachedIconTexture::Ready(texture.clone()));
    });
    Some(texture)
}

/// Paints a theme-tinted Lucide outline icon centered inside `rect`.
pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    if let Some(texture) = icon_texture(painter.ctx(), icon) {
        painter.image(
            texture.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
            color,
        );
    }
}

/// Measures the rendered width of an [`icon_button`] with `label`.
pub fn icon_button_width(ui: &egui::Ui, label: Option<&str>) -> f32 {
    let icon_dim = 15.0_f32;
    let pad_left = 9.0_f32;
    let gap = 6.0_f32;
    let pad_right = 10.0_f32;
    match label.filter(|txt| !txt.is_empty()) {
        Some(txt) => {
            let galley = ui.painter().layout_no_wrap(
                txt.to_string(),
                FontId::proportional(12.2),
                Color32::WHITE,
            );
            (pad_left + icon_dim + gap + galley.size().x + pad_right).max(28.0)
        }
        None => 28.0,
    }
}

/// Tactile button with an aligned Lucide icon and optional label.
pub fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: Option<&str>,
    selected: bool,
    enabled: bool,
    t: &Tokens,
) -> Response {
    draw_icon_button(ui, icon, label, selected, enabled, false, t)
}

/// Top-header variant of [`icon_button`] using header contrast tokens.
pub fn header_icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: Option<&str>,
    selected: bool,
    enabled: bool,
    t: &Tokens,
) -> Response {
    draw_icon_button(ui, icon, label, selected, enabled, true, t)
}

fn draw_icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: Option<&str>,
    selected: bool,
    enabled: bool,
    on_header: bool,
    t: &Tokens,
) -> Response {
    let icon_dim = 15.0_f32;
    let height = 28.0_f32;
    let pad_left = 9.0_f32;
    let gap = 6.0_f32;
    let pad_right = 10.0_f32;
    let (rest_fg, muted_fg) = if on_header {
        (t.header_fg, t.header_fg_muted)
    } else {
        (t.text, t.text_muted)
    };
    let base_color = if enabled {
        rest_fg
    } else {
        muted_fg.gamma_multiply(0.55)
    };

    let label = label.filter(|txt| !txt.is_empty());
    let galley = label.map(|txt| {
        ui.painter()
            .layout_no_wrap(txt.to_string(), FontId::proportional(12.2), base_color)
    });
    let width = match &galley {
        Some(g) => (pad_left + icon_dim + gap + g.size().x + pad_right).max(28.0),
        None => 28.0,
    };

    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), sense);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = enabled && response.hovered();
        let hover_t = hover_factor(ui.ctx(), response.id.with("hov"), hovered && !selected);
        let select_t = if enabled {
            hover_factor(ui.ctx(), response.id.with("sel"), selected)
        } else {
            0.0
        };
        let (rest_fill_base, hov_fill, act_fill, rest_s, hov_s, act_s, act_fg) = if on_header {
            (
                if t.is_dark {
                    t.field_bg
                } else {
                    Color32::TRANSPARENT
                },
                t.header_control_hover,
                t.header_active_bg,
                t.header_border,
                t.header_fg_muted.gamma_multiply(0.65),
                if t.is_dark {
                    t.accent
                } else {
                    t.header_active_bg
                },
                t.header_active_fg,
            )
        } else {
            (
                t.field_bg,
                t.border.gamma_multiply(0.45),
                t.accent_soft,
                t.border,
                t.text_muted.gamma_multiply(0.5),
                t.accent,
                t.accent,
            )
        };
        let rest_fill = lerp_color(rest_fill_base, hov_fill, hover_t);
        let fill = lerp_color(rest_fill, act_fill, select_t);
        let rest_stroke = lerp_color(rest_s, hov_s, hover_t);
        let stroke_color = lerp_color(rest_stroke, act_s, select_t);
        let stroke = Stroke::new(lerp_f32(1.0, 1.5, select_t), stroke_color);
        let foreground = if enabled {
            lerp_color(rest_fg, act_fg, select_t)
        } else {
            base_color
        };

        painter.rect_filled(rect, CornerRadius::same(7), fill);
        painter.rect_stroke(rect, CornerRadius::same(7), stroke, StrokeKind::Inside);

        let icon_rect = match &galley {
            Some(g) => {
                let ir = Rect::from_center_size(
                    pos2(rect.left() + pad_left + icon_dim * 0.5, rect.center().y),
                    Vec2::splat(icon_dim),
                );
                let text_pos = pos2(ir.right() + gap, rect.center().y - g.size().y * 0.5);
                painter.galley_with_override_text_color(text_pos, g.clone(), foreground);
                ir
            }
            None => Rect::from_center_size(rect.center(), Vec2::splat(icon_dim)),
        };
        paint_icon(painter, icon_rect, icon, foreground);
    }

    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// One segment in [`segmented_switch`].
#[derive(Clone, Copy, Debug)]
pub struct SwitchOption<'a> {
    pub icon: Option<Icon>,
    pub label: &'a str,
    pub tooltip: &'a str,
}

impl<'a> SwitchOption<'a> {
    pub const fn new(icon: Option<Icon>, label: &'a str, tooltip: &'a str) -> Self {
        Self {
            icon,
            label,
            tooltip,
        }
    }
}

/// Renders a full-width animated segmented selector matching PaperCraft's switches.
pub fn segmented_switch(
    ui: &mut egui::Ui,
    id_salt: impl egui::AsIdSalt,
    options: &[SwitchOption<'_>],
    active: usize,
    t: &Tokens,
) -> Option<usize> {
    if options.is_empty() {
        return None;
    }

    let track_h = 30.0_f32;
    let track_pad = 2.0_f32;
    let seg_gap = 2.0_f32;
    let seg_h = track_h - track_pad * 2.0;
    let n = options.len();
    let gaps_total = seg_gap * n.saturating_sub(1) as f32;
    let track_w = ui.available_width().max(n as f32 * 64.0);
    let seg_w = (track_w - track_pad * 2.0 - gaps_total).max(24.0) / n as f32;

    let track_id = ui.make_persistent_id(id_salt);
    let (track_rect, _) = ui.allocate_exact_size(Vec2::new(track_w, track_h), Sense::hover());

    let mut seg_rects = Vec::with_capacity(n);
    let mut x = track_rect.left() + track_pad;
    let seg_top = track_rect.top() + track_pad;
    for _ in 0..n {
        seg_rects.push(Rect::from_min_size(
            pos2(x, seg_top),
            Vec2::new(seg_w, seg_h),
        ));
        x += seg_w + seg_gap;
    }

    let mut clicked = None;
    let mut hovered = Vec::with_capacity(n);
    for (idx, opt) in options.iter().enumerate() {
        let mut resp = ui
            .interact(seg_rects[idx], track_id.with(("seg", idx)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if !opt.tooltip.is_empty() {
            resp = resp.on_hover_text(opt.tooltip);
        }
        if resp.clicked() {
            clicked = Some(idx);
        }
        hovered.push(resp.hovered());
    }

    if ui.is_rect_visible(track_rect) {
        let p = ui.painter();
        p.rect(
            track_rect,
            CornerRadius::same(8),
            t.toolbar_bg,
            Stroke::new(1.0, t.border),
            StrokeKind::Inside,
        );

        paint_switch_pill_and_labels(
            ui,
            SwitchPaintArgs {
                track_id,
                track_rect,
                seg_rects: &seg_rects,
                options,
                active,
                hovered: &hovered,
                t,
            },
        );
    }

    clicked
}

struct SwitchPaintArgs<'a> {
    track_id: egui::Id,
    track_rect: Rect,
    seg_rects: &'a [Rect],
    options: &'a [SwitchOption<'a>],
    active: usize,
    hovered: &'a [bool],
    t: &'a Tokens,
}

fn paint_switch_pill_and_labels(ui: &egui::Ui, args: SwitchPaintArgs<'_>) {
    let p = ui.painter();
    let t = args.t;
    for (idx, _) in args.options.iter().enumerate() {
        let hover_t = hover_factor(
            ui.ctx(),
            args.track_id.with(("hov", idx)),
            args.hovered[idx] && idx != args.active,
        );
        if hover_t > 0.02 {
            p.rect_filled(
                args.seg_rects[idx],
                CornerRadius::same(6),
                t.border.gamma_multiply(0.35 * hover_t),
            );
        }
    }

    if let Some(target_r) = args.seg_rects.get(args.active) {
        let rel_x0 = target_r.left() - args.track_rect.left();
        let rel_x1 = target_r.right() - args.track_rect.left();
        let anim_x0 = args.track_rect.left()
            + ui.ctx().animate_value_with_time(
                args.track_id.with("pill_x0"),
                rel_x0,
                MICRO_DURATION_S,
            );
        let anim_x1 = args.track_rect.left()
            + ui.ctx().animate_value_with_time(
                args.track_id.with("pill_x1"),
                rel_x1,
                MICRO_DURATION_S,
            );
        let pill_rect = Rect::from_min_max(
            pos2(anim_x0, target_r.top()),
            pos2(anim_x1, target_r.bottom()),
        );
        p.rect(
            pill_rect,
            CornerRadius::same(6),
            t.accent_soft,
            Stroke::new(1.2, t.accent),
            StrokeKind::Inside,
        );
    }

    for (idx, opt) in args.options.iter().enumerate() {
        let is_active = idx == args.active;
        let select_t = hover_factor(ui.ctx(), args.track_id.with(("sel", idx)), is_active);
        let hover_t = hover_factor(
            ui.ctx(),
            args.track_id.with(("hov", idx)),
            args.hovered[idx] && !is_active,
        );
        let rest_fg = lerp_color(t.text_muted, t.text, 0.35);
        let hovered_fg = lerp_color(rest_fg, t.text, hover_t);
        let fg = lerp_color(hovered_fg, t.accent, select_t);
        let galley = p.layout_no_wrap(opt.label.to_string(), FontId::proportional(11.8), fg);
        let seg_rect = args.seg_rects[idx];
        match opt.icon {
            Some(icon) => {
                let icon_dim = 14.0_f32;
                let gap = 5.0_f32;
                let total_w = icon_dim + gap + galley.size().x;
                let start_x = seg_rect.center().x - total_w * 0.5;
                let icon_rect = Rect::from_center_size(
                    pos2(start_x + icon_dim * 0.5, seg_rect.center().y),
                    Vec2::splat(icon_dim),
                );
                paint_icon(p, icon_rect, icon, fg);
                let text_pos = pos2(
                    start_x + icon_dim + gap,
                    seg_rect.center().y - galley.size().y * 0.5,
                );
                p.galley(text_pos, galley, fg);
            }
            None => {
                let text_pos = pos2(
                    seg_rect.center().x - galley.size().x * 0.5,
                    seg_rect.center().y - galley.size().y * 0.5,
                );
                p.galley(text_pos, galley, fg);
            }
        }
    }
}

/// Renders an animated boolean pill toggle switch (`36x20`) styled with active theme tokens.
pub fn toggle_switch(
    ui: &mut egui::Ui,
    id_salt: impl egui::AsIdSalt,
    on: &mut bool,
    enabled: bool,
    t: &Tokens,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(36.0, 20.0), sense);
    if enabled && resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, *on, ""));

    if ui.is_rect_visible(rect) {
        let anim_t = ui.ctx().animate_value_with_time(
            ui.make_persistent_id(id_salt),
            if *on { 1.0 } else { 0.0 },
            MICRO_DURATION_S,
        );
        let alpha = if enabled { 1.0 } else { 0.55 };
        let track_fill = lerp_color(t.field_bg, t.accent, anim_t).gamma_multiply(alpha);
        let track_stroke = lerp_color(t.border, t.accent, anim_t).gamma_multiply(alpha);
        let knob_fill = lerp_color(t.text_muted, t.on_accent, anim_t).gamma_multiply(alpha);

        let p = ui.painter();
        p.rect(
            rect,
            CornerRadius::same(10),
            track_fill,
            Stroke::new(1.0, track_stroke),
            StrokeKind::Inside,
        );
        let knob_x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), anim_t);
        p.circle_filled(pos2(knob_x, rect.center().y), 7.0, knob_fill);
    }

    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Renders a labeled settings row with a clickable label on the left and a right-aligned pill switch.
pub fn toggle_switch_row(
    ui: &mut egui::Ui,
    id_salt: impl egui::AsIdSalt,
    label: &str,
    tooltip: &str,
    on: &mut bool,
    t: &Tokens,
) -> egui::Response {
    let was_on = *on;
    let switch_w = 36.0_f32;
    let gap = 8.0_f32;
    ui.horizontal(|ui| {
        let label_w = (ui.available_width() - switch_w - gap).max(80.0);
        let mut label_resp = ui
            .allocate_ui_with_layout(
                Vec2::new(label_w, 20.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(label).size(12.0).color(t.text))
                            .wrap()
                            .sense(Sense::click()),
                    )
                },
            )
            .inner
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if !tooltip.is_empty() {
            label_resp = label_resp.on_hover_text(tooltip);
        }
        if label_resp.clicked() {
            *on = !*on;
        }

        ui.add_space(gap);
        let mut switch_resp = toggle_switch(ui, id_salt, on, true, t);
        if !tooltip.is_empty() {
            switch_resp = switch_resp.on_hover_text(tooltip);
        }

        let mut combined = label_resp.union(switch_resp);
        if *on != was_on {
            combined.mark_changed();
        }
        combined
    })
    .inner
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use egui::{RawInput, epaint::Primitive};

    #[test]
    fn all_server_lucide_icons_render_cleanly() {
        let all = [
            Icon::Server,
            Icon::CheckCircle,
            Icon::Square,
            Icon::RotateCw,
            Icon::Reset,
            Icon::FolderOpen,
            Icon::Copy,
            Icon::Check,
            Icon::Link,
            Icon::Account,
            Icon::Notebook,
            Icon::Settings,
            Icon::Sun,
            Icon::Moon,
            Icon::ChevronDown,
            Icon::ChevronRight,
            Icon::AlertCircle,
            Icon::Info,
            Icon::Close,
            Icon::Search,
            Icon::Trash,
            Icon::MoveDown,
            Icon::ExternalLink,
        ];
        let rect = Rect::from_min_size(pos2(16.0, 16.0), Vec2::splat(16.0));
        for icon in all {
            let ctx = egui::Context::default();
            let out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(64.0))),
                    ..Default::default()
                },
                |ui| {
                    paint_icon(ui.painter(), rect, icon, Color32::WHITE);
                },
            );
            let primitives = ctx.tessellate(out.shapes.clone(), 1.0);
            out.drop_without_applying_deltas();
            let vertices: usize = primitives
                .iter()
                .filter_map(|c| match &c.primitive {
                    Primitive::Mesh(m) => Some(m.vertices.len()),
                    _ => None,
                })
                .sum();
            assert!(vertices > 0, "icon {icon:?} produced zero vertices");
        }
    }
}
