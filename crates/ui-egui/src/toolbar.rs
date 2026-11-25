//! Compact 3-zone header with a floating context tray for tool and selection settings.

use egui::{Color32, CornerRadius, FontId, Sense, Stroke, StrokeKind, Vec2};
use papercraft_color::Color;
use papercraft_doc::{
    EraserMode, HighlighterMode, ListItemKind, PaletteTarget, PenKind, SavedElement, ShapeKind,
    ShapeLineStyle, SmoothingConfig, StabilizationPreset, TextAlignment, TextFont, TextItem,
    TextStyleAttr, find_text_url_spans, parse_list_line_prefix, sanitize_link_url,
};
use papercraft_engine::ToolKind;
use papercraft_geom::{Affine2, Point};
use serde_json::json;

use crate::PaperCraftApp;
use crate::color_picker::{
    ColorPopoverMode, format_color_label, paint_checkerboard_circle, same_rgb, same_rgba,
};
use crate::icons::{
    Icon, SidebarEdge, SidebarEdgeButtonSpec, SidebarEdgeSize, SquareButtonSpec, SwitchLayout,
    SwitchOption, TOOLBAR_CONTROL_HEIGHT, compact_icon_button, paint_icon, segmented_switch,
    segmented_switch_width, sidebar_edge_button_sized, sidebar_edge_button_sized_width,
    sized_icon_button, toolbar_icon_button, tray_icon_button, tray_icon_button_width,
};
use crate::theme::{Tokens, to_color32};

const TOOLBAR_ROW_HEIGHT: f32 = TOOLBAR_CONTROL_HEIGHT;
const CONTEXT_TRAY_ROW_HEIGHT: f32 = 32.0;
const CONTEXT_TRAY_TILE_SIZE: f32 = 26.0;
const CONTEXT_TRAY_CORNER_RADIUS: u8 = 11;
const ZONE_ITEM_SPACING: f32 = 3.0;
const EDGE_TAB_NEIGHBOR_GAP: f32 = 6.0;
const DIVIDER_WIDTH: f32 = 8.0;
const ZOOM_PILL_WIDTH: f32 = 44.0;
const RIGHT_UTILITIES_WIDTH: f32 =
    TOOLBAR_ROW_HEIGHT * 7.0 + ZOOM_PILL_WIDTH + DIVIDER_WIDTH * 2.0 + ZONE_ITEM_SPACING * 9.0;
const RULER_ANGLE_CONTROL_WIDTH: f32 = 62.0;
const SHAPE_CURVE_CONTROL_WIDTH: f32 = 84.0;
const PATH_SIZE_SLIDER_WIDTH: f32 = 126.0;
const PATH_SIZE_TRACK_HEIGHT: f32 = 14.0;
const PATH_SIZE_THUMB_RADIUS: f32 = 8.8;
const PATH_SIZE_THUMB_HOVER_RADIUS: f32 = 9.8;
const CONTEXT_TRAY_SETTLE_OFFSET_Y: f32 = 8.0;
const COLOR_DOCK_OUTER_MARGIN: f32 = 12.0;
const COLOR_DOCK_INNER_MARGIN_X: f32 = 8.0;
const COLOR_DOCK_INNER_MARGIN_Y: f32 = 8.0;
const COLOR_DOCK_CORNER_RADIUS: u8 = 16;
const COLOR_DOCK_INNER_CORNER_RADIUS: u8 = 8;
const COLOR_DOCK_GRIP_SIZE: Vec2 = Vec2::new(32.0, 20.0);
const COLOR_DOCK_GRIP_ICON_SIZE: f32 = 18.0;
const COLOR_DOCK_DIVIDER_H: f32 = 6.0;
const COLOR_DOCK_ITEM_GAP: f32 = 2.0;
const COLOR_DOCK_WILDCARD_SIZE: Vec2 = Vec2::new(32.0, 26.0);
const COLOR_DOCK_SWATCH_SIZE: Vec2 = Vec2::new(32.0, 24.0);
const COLOR_DOCK_GRID_CELL_SIZE: Vec2 = Vec2::new(22.0, 22.0);
const COLOR_DOCK_PALETTE_BTN_SIZE: f32 = 32.0;
const COLOR_DOCK_PALETTE_ICON_SIZE: f32 = 22.0;
const COLOR_DOCK_SHAPE_SWITCH_W: f32 = 54.0;
const COLOR_DOCK_SHAPE_SWITCH_H: f32 = 64.0;
const COLOR_DOCK_SHAPE_SPECIAL_SIZE: Vec2 = Vec2::new(26.0, 26.0);
const COLOR_DOCK_SHAPE_SWATCH_SIZE: Vec2 = Vec2::new(26.0, 24.0);
const COLOR_DOCK_SETTLE_OFFSET_X: f32 = 8.0;
const COLOR_DOCK_DRAG_SMOOTH_RATE: f32 = 34.0;
const COLOR_DOCK_GLIDE_SMOOTH_RATE: f32 = 18.0;

#[derive(Clone, Debug)]
pub(crate) enum ContextTrayMode {
    Selection(Vec<crate::SelectedObject>),
    SelectEmpty,
    ShapeInsert(PenKind),
    TextInsert,
    Ruler { tool: ToolKind },
    Pen(PenKind),
    Eraser,
    LaserPointer,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ChannelPreview {
    Color(Color),
    None,
    Mixed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ShapeChannelInfo {
    active_target: crate::ShapeColorTarget,
    border: ChannelPreview,
    fill: ChannelPreview,
}

#[derive(Clone, Debug)]
enum ColorDockMode {
    Pen(PenKind),
    ShapeInsert {
        border: ChannelPreview,
        fill: ChannelPreview,
    },
    Selection {
        color: Option<Color>,
        color_is_common: bool,
        target: PaletteTarget,
        picker_kind: PenKind,
        shape_channels: Option<ShapeChannelInfo>,
    },
    TextInsert,
    LaserPointer,
}

/// Smooth reveal transition state shared by floating tool trays.
#[derive(Clone, Debug)]
pub(crate) struct TrayTransition<M> {
    progress: f32,
    last_mode: Option<M>,
}

impl<M> Default for TrayTransition<M> {
    fn default() -> Self {
        Self {
            progress: 0.0,
            last_mode: None,
        }
    }
}

impl<M: Clone> TrayTransition<M> {
    fn advance(&mut self, mode: Option<M>, dt: f32) -> (f32, Option<M>, bool) {
        if let Some(mode) = mode.as_ref() {
            self.last_mode = Some(mode.clone());
        }
        let target = if mode.is_some() { 1.0 } else { 0.0 };
        self.progress = crate::motion::step_towards(
            self.progress,
            target,
            dt,
            crate::motion::STANDARD_DURATION_S,
        );
        let animating = (self.progress - target).abs() > 0.001;
        let display_mode = self.last_mode.clone();
        if !animating && target == 0.0 {
            self.last_mode = None;
        }
        (
            crate::motion::smoothstep(self.progress),
            display_mode,
            animating,
        )
    }
}

pub(crate) type ContextTrayTransition = TrayTransition<ContextTrayMode>;

/// Session state for the floating, draggable color dock.
///
/// Opens vertically docked to the left edge of the canvas until dragged by the user,
/// glides smoothly during drags and resets, and stays within the visible workspace.
#[derive(Clone, Debug)]
pub(crate) struct ColorDockState {
    transition: TrayTransition<ColorDockMode>,
    custom_pos: Option<egui::Pos2>,
    visual_pos: Option<egui::Pos2>,
    drag_grab_offset: Option<egui::Vec2>,
    is_dragging: bool,
    pub(crate) last_rect: egui::Rect,
    pub(crate) selected_wildcard: Option<PaletteTarget>,
}

impl Default for ColorDockState {
    fn default() -> Self {
        Self {
            transition: TrayTransition::default(),
            custom_pos: None,
            visual_pos: None,
            drag_grab_offset: None,
            is_dragging: false,
            last_rect: egui::Rect::NOTHING,
            selected_wildcard: None,
        }
    }
}

impl ColorDockState {
    fn clear_ephemeral(&mut self) {
        self.visual_pos = None;
        self.drag_grab_offset = None;
        self.is_dragging = false;
        self.last_rect = egui::Rect::NOTHING;
    }

    fn clamp_pos(pos: egui::Pos2, min_pos: egui::Pos2, max_pos: egui::Pos2) -> egui::Pos2 {
        let x = if pos.x.is_finite() {
            pos.x.clamp(min_pos.x, max_pos.x)
        } else {
            min_pos.x
        };
        let y = if pos.y.is_finite() {
            pos.y.clamp(min_pos.y, max_pos.y)
        } else {
            min_pos.y
        };
        egui::pos2(x, y)
    }
}

fn normalized_degrees(degrees: f64) -> f64 {
    if degrees.is_finite() {
        (degrees + 180.0).rem_euclid(360.0) - 180.0
    } else {
        0.0
    }
}

fn pen_icon(kind: PenKind) -> Icon {
    match kind {
        PenKind::FountainPen => Icon::FountainPen,
        PenKind::BallPen => Icon::BallPen,
        PenKind::BrushPen => Icon::BrushPen,
        PenKind::Highlighter => Icon::Highlighter,
        PenKind::Pencil => Icon::Pencil,
    }
}

fn measure_left_zone(ui: &egui::Ui) -> f32 {
    let notebook_w = sidebar_edge_button_sized_width(ui, "", None, SidebarEdgeSize::Toolbar);
    notebook_w + EDGE_TAB_NEIGHBOR_GAP + TOOLBAR_ROW_HEIGHT
}

fn measure_center_zone(ui: &egui::Ui) -> f32 {
    let dummy_options = [SwitchOption::new(Some(Icon::Select), None, "", true); 10];
    let tools_w = segmented_switch_width(ui, &dummy_options, SwitchLayout::Toolbar);
    let addons_w = 3.0 * TOOLBAR_ROW_HEIGHT + 3.0 * ZONE_ITEM_SPACING;
    tools_w + addons_w
}

fn measure_right_zone(ui: &egui::Ui) -> f32 {
    RIGHT_UTILITIES_WIDTH
        + EDGE_TAB_NEIGHBOR_GAP
        + sidebar_edge_button_sized_width(ui, "", None, SidebarEdgeSize::Toolbar)
}

fn hairline_divider_sized(ui: &mut egui::Ui, row_height: f32, t: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(DIVIDER_WIDTH, row_height), Sense::hover());
    if ui.is_rect_visible(rect) {
        let x = rect.center().x;
        let pad_y = if row_height > 30.0 { 6.0 } else { 5.0 };
        ui.painter().line_segment(
            [
                egui::pos2(x, rect.top() + pad_y),
                egui::pos2(x, rect.bottom() - pad_y),
            ],
            Stroke::new(1.0, t.border),
        );
    }
}

fn hairline_divider(ui: &mut egui::Ui, t: &Tokens) {
    hairline_divider_sized(ui, TOOLBAR_ROW_HEIGHT, t);
}

fn draw_size_tile_bg(
    ui: &mut egui::Ui,
    t: &Tokens,
    selected: bool,
) -> (egui::Rect, egui::Response, f32) {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(CONTEXT_TRAY_TILE_SIZE, CONTEXT_TRAY_ROW_HEIGHT),
        Sense::click(),
    );
    let tile_rect =
        egui::Rect::from_center_size(rect.center(), Vec2::splat(CONTEXT_TRAY_TILE_SIZE));
    let hover_t =
        crate::motion::hover_factor(ui.ctx(), resp.id.with("hov"), resp.hovered() && !selected);
    let select_t = crate::motion::hover_factor(ui.ctx(), resp.id.with("sel"), selected);
    let rest_fill =
        crate::motion::lerp_color(Color32::TRANSPARENT, t.border.gamma_multiply(0.4), hover_t);
    let fill = crate::motion::lerp_color(rest_fill, t.accent_soft, select_t);
    let p = ui.painter();
    p.rect_filled(tile_rect, CornerRadius::same(7), fill);
    if select_t > 0.01 {
        p.rect_stroke(
            tile_rect,
            CornerRadius::same(7),
            Stroke::new(1.2, t.accent.gamma_multiply(select_t)),
            StrokeKind::Inside,
        );
    }
    (tile_rect, resp, select_t)
}

fn pen_path_size_range(kind: PenKind) -> ([(&'static str, f32); 3], f32, f32) {
    match kind {
        PenKind::Highlighter => (
            [
                ("Narrow (14 pt)", 14.0),
                ("Standard (24 pt)", 24.0),
                ("Wide (38 pt)", 38.0),
            ],
            4.0,
            64.0,
        ),
        PenKind::Pencil => (
            [
                ("Fine (1.6 pt)", 1.6),
                ("Medium (2.8 pt)", 2.8),
                ("Bold (5.5 pt)", 5.5),
            ],
            0.5,
            16.0,
        ),
        PenKind::FountainPen | PenKind::BallPen | PenKind::BrushPen => (
            [
                ("Fine (2.2 pt)", 2.2),
                ("Medium (4.5 pt)", 4.5),
                ("Bold (8.0 pt)", 8.0),
            ],
            0.5,
            24.0,
        ),
    }
}

#[derive(Clone, Copy)]
struct PathSizeSpec {
    presets: [(&'static str, f32); 3],
    min: f32,
    max: f32,
    current: f32,
    display_color: Color,
    preset_tooltip_prefix: &'static str,
    slider_tooltip_prefix: &'static str,
}

#[derive(Clone, Copy, Default)]
struct PathSliderDragState {
    active: bool,
    history_recorded: bool,
    needs_persist: bool,
}

fn color_contrast_dist(a: Color32, b: Color32) -> f32 {
    let dr = (f32::from(a.r()) - f32::from(b.r())) / 255.0;
    let dg = (f32::from(a.g()) - f32::from(b.g())) / 255.0;
    let db = (f32::from(a.b()) - f32::from(b.b())) / 255.0;
    ((dr * dr + dg * dg + db * db) / 3.0).sqrt()
}

fn paint_drag_size_readout(
    ctx: &egui::Context,
    id: egui::Id,
    thumb_center: egui::Pos2,
    label: &str,
    t: &Tokens,
) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, id));
    let galley = painter.layout_no_wrap(label.to_string(), FontId::proportional(11.0), t.text);
    let pill_size = galley.size() + Vec2::new(12.0, 6.0);
    let screen = ctx.content_rect().shrink(4.0);
    let min_x = screen.left();
    let max_x = (screen.right() - pill_size.x).max(min_x);
    let left = (thumb_center.x - pill_size.x * 0.5).clamp(min_x, max_x);
    let max_y = (screen.bottom() - pill_size.y).max(screen.top());
    let top = (thumb_center.y + 15.0).clamp(screen.top(), max_y);
    let pill_rect = egui::Rect::from_min_size(egui::pos2(left, top), pill_size);
    painter.rect(
        pill_rect,
        CornerRadius::same(6),
        t.field_bg,
        Stroke::new(1.0, t.border),
        StrokeKind::Inside,
    );
    let text_pos = egui::pos2(
        pill_rect.center().x - galley.size().x * 0.5,
        pill_rect.center().y - galley.size().y * 0.5,
    );
    painter.galley(text_pos, galley, t.text);
}

fn update_path_slider_drag(
    ui: &egui::Ui,
    resp: &egui::Response,
    drag_id: egui::Id,
    spec: &PathSizeSpec,
    travel_min_x: f32,
    travel_w: f32,
) -> Option<(f32, bool, bool)> {
    let span = (spec.max - spec.min).max(f32::EPSILON);
    let current = if spec.current.is_finite() {
        spec.current
    } else {
        spec.min
    };
    let pos_to_size = |pos: egui::Pos2| -> f32 {
        let frac = ((pos.x - travel_min_x) / travel_w).clamp(0.0, 1.0);
        let raw = spec.min + frac * span;
        ((raw * 10.0).round() / 10.0).clamp(spec.min, spec.max)
    };

    let mut drag_state = ui
        .ctx()
        .data(|d| d.get_temp::<PathSliderDragState>(drag_id))
        .unwrap_or_default();
    let mut emission = None;
    let pointer_pos = resp
        .interact_pointer_pos()
        .or_else(|| ui.input(|i| i.pointer.interact_pos()));

    if resp.dragged_by(egui::PointerButton::Primary) {
        drag_state.active = true;
        if let Some(pos) = pointer_pos {
            let next_val = pos_to_size(pos);
            if (next_val - current).abs() >= 0.05 {
                let rec = !drag_state.history_recorded;
                drag_state.history_recorded = true;
                drag_state.needs_persist = true;
                emission = Some((next_val, rec, false));
            }
        }
    } else if drag_state.active {
        let final_val = pointer_pos.map(pos_to_size).unwrap_or(current);
        if drag_state.needs_persist || (final_val - current).abs() >= 0.05 {
            let rec = !drag_state.history_recorded;
            emission = Some((final_val, rec, true));
        }
        drag_state = PathSliderDragState::default();
    } else if resp.clicked_by(egui::PointerButton::Primary)
        && let Some(pos) = pointer_pos
    {
        let next_val = pos_to_size(pos);
        if (next_val - current).abs() >= 0.05 {
            emission = Some((next_val, true, true));
        }
    }

    ui.ctx().data_mut(|d| d.insert_temp(drag_id, drag_state));
    emission
}

struct PathSliderRenderArgs<'a> {
    track_rect: egui::Rect,
    travel_min_x: f32,
    travel_w: f32,
    drag_id: egui::Id,
    spec: &'a PathSizeSpec,
    display_val: f32,
    is_dragging: bool,
    hovered: bool,
}

fn paint_path_size_slider(
    ui: &mut egui::Ui,
    args: &PathSliderRenderArgs<'_>,
    t: &Tokens,
) -> egui::Pos2 {
    let spec = args.spec;
    let span = (spec.max - spec.min).max(f32::EPSILON);
    let target_frac = ((args.display_val - spec.min) / span).clamp(0.0, 1.0);
    let anim_frac = ui
        .ctx()
        .animate_value_with_time(
            args.drag_id.with("frac"),
            target_frac,
            if args.is_dragging {
                0.0
            } else {
                crate::motion::MICRO_DURATION_S
            },
        )
        .clamp(0.0, 1.0);
    let hover_t = crate::motion::hover_factor(
        ui.ctx(),
        args.drag_id.with("hov"),
        args.hovered && !args.is_dragging,
    );
    let drag_t = crate::motion::hover_factor(ui.ctx(), args.drag_id.with("act"), args.is_dragging);
    let active_t = hover_t.max(drag_t);

    let track_rect = args.track_rect;
    let thumb_cx = args.travel_min_x + anim_frac * args.travel_w;
    let thumb_center = egui::pos2(thumb_cx, track_rect.center().y);
    if !ui.is_rect_visible(track_rect) {
        return thumb_center;
    }

    let cap_pad = PATH_SIZE_TRACK_HEIGHT * 0.5;
    let track_bg = if t.is_dark {
        t.field_bg
    } else {
        crate::motion::lerp_color(t.canvas_bg, t.border, 0.28)
    };
    let fill_color = crate::motion::lerp_color(t.accent, t.accent_hover, active_t);
    let track_border = crate::motion::lerp_color(t.border, t.accent.gamma_multiply(0.65), active_t);

    let p = ui.painter();
    let track_cr = CornerRadius::same(7);
    p.rect_filled(track_rect, track_cr, track_bg);
    let fill_right = (thumb_cx + cap_pad).clamp(
        (track_rect.left() + PATH_SIZE_TRACK_HEIGHT).min(track_rect.right()),
        track_rect.right(),
    );
    let fill_rect =
        egui::Rect::from_min_max(track_rect.min, egui::pos2(fill_right, track_rect.max.y));
    p.rect_filled(fill_rect, track_cr, fill_color);
    p.rect_stroke(
        track_rect,
        track_cr,
        Stroke::new(1.0, track_border),
        StrokeKind::Inside,
    );

    let thumb_r = crate::motion::lerp_f32(
        PATH_SIZE_THUMB_RADIUS,
        PATH_SIZE_THUMB_HOVER_RADIUS,
        active_t,
    );
    if active_t > 0.01 {
        p.circle_filled(
            thumb_center,
            thumb_r + 2.5 * active_t,
            t.accent.gamma_multiply(0.18 * active_t),
        );
    }
    p.circle(
        thumb_center,
        thumb_r,
        t.field_bg,
        Stroke::new(2.5, fill_color),
    );
    let clean_c = spec.display_color.sanitized();
    let core_color = if clean_c.a <= 0.01 {
        t.text_muted
    } else if clean_c.a < 0.25 {
        to_color32(clean_c.with_alpha(0.25))
    } else {
        let ink_c32 = to_color32(clean_c);
        if color_contrast_dist(ink_c32, t.field_bg) > 0.16 {
            ink_c32
        } else {
            fill_color
        }
    };
    p.circle_filled(thumb_center, 3.0, core_color);
    thumb_center
}

fn draw_path_size_slider(
    ui: &mut egui::Ui,
    id_salt: impl egui::AsIdSalt,
    spec: &PathSizeSpec,
    t: &Tokens,
) -> Option<(f32, bool, bool)> {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(PATH_SIZE_SLIDER_WIDTH, CONTEXT_TRAY_ROW_HEIGHT),
        Sense::click_and_drag(),
    );
    let track_rect = egui::Rect::from_min_max(
        egui::pos2(
            rect.left() + 5.0,
            rect.center().y - PATH_SIZE_TRACK_HEIGHT * 0.5,
        ),
        egui::pos2(
            rect.right() - 3.0,
            rect.center().y + PATH_SIZE_TRACK_HEIGHT * 0.5,
        ),
    );
    let cap_pad = PATH_SIZE_TRACK_HEIGHT * 0.5;
    let travel_min_x = track_rect.left() + cap_pad;
    let travel_max_x = (track_rect.right() - cap_pad).max(travel_min_x + 1.0);
    let travel_w = (travel_max_x - travel_min_x).max(1.0);

    let drag_id = ui.make_persistent_id(id_salt);
    let emission = update_path_slider_drag(ui, &resp, drag_id, spec, travel_min_x, travel_w);
    let current = if spec.current.is_finite() {
        spec.current
    } else {
        spec.min
    };
    let display_val = emission.map_or(current, |(v, _, _)| v);
    let is_dragging = resp.dragged_by(egui::PointerButton::Primary);
    let thumb_center = paint_path_size_slider(
        ui,
        &PathSliderRenderArgs {
            track_rect,
            travel_min_x,
            travel_w,
            drag_id,
            spec,
            display_val,
            is_dragging,
            hovered: resp.hovered(),
        },
        t,
    );

    let cursor = if is_dragging {
        egui::CursorIcon::Grabbing
    } else {
        egui::CursorIcon::PointingHand
    };
    let resp = resp.on_hover_cursor(cursor);
    if is_dragging {
        paint_drag_size_readout(
            ui.ctx(),
            drag_id.with("readout"),
            thumb_center,
            &format!("{display_val:.1} pt"),
            t,
        );
    } else {
        resp.on_hover_text(format!(
            "{} ({display_val:.1} pt) — drag or click to adjust",
            spec.slider_tooltip_prefix
        ));
    }
    emission
}

fn push_path_size_controls<'a>(
    flow: &mut DockFlow<'a>,
    id_salt: impl egui::AsIdSalt + Clone + 'a,
    spec: PathSizeSpec,
    on_apply: impl Fn(&mut PaperCraftApp, f32, bool, bool) + Clone + 'a,
) {
    flow.group();
    for (idx, (label, preset_size)) in spec.presets.iter().copied().enumerate() {
        let selected = (spec.current - preset_size).abs() < 0.3;
        let dot_r = [3.1_f32, 5.3, 7.8][idx];
        let disp_color = spec.display_color.sanitized();
        let tip = format!("{}{label}", spec.preset_tooltip_prefix);
        let apply = on_apply.clone();
        let draw_fn = move |app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens| {
            let (rect, resp, _) = draw_size_tile_bg(ui, t, selected);
            let dot_col = if disp_color.a <= 0.01 {
                t.text_muted
            } else if disp_color.a < 0.25 {
                to_color32(disp_color.with_alpha(0.25))
            } else {
                to_color32(disp_color)
            };
            ui.painter().circle_filled(rect.center(), dot_r, dot_col);
            if resp.on_hover_text(tip).clicked() {
                apply(app, preset_size, true, true);
            }
        };
        if idx == 0 {
            flow.push(CONTEXT_TRAY_TILE_SIZE, draw_fn);
        } else {
            flow.push_cohesive(CONTEXT_TRAY_TILE_SIZE, draw_fn);
        }
    }

    let apply = on_apply;
    flow.push_cohesive(PATH_SIZE_SLIDER_WIDTH, move |app, ui, t| {
        if let Some((next_size, record_history, persist_size)) =
            draw_path_size_slider(ui, id_salt, &spec, t)
        {
            apply(app, next_size, record_history, persist_size);
        }
    });
}

fn color_dot_sized(
    ui: &mut egui::Ui,
    size: Vec2,
    c: Color,
    selected: bool,
    tooltip: &str,
    t: &Tokens,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let min_dim = if size.x.is_finite() && size.y.is_finite() {
        size.x.min(size.y)
    } else {
        20.0
    };
    let scale = if min_dim > 20.0 {
        (min_dim / 20.0).clamp(1.0, 1.5)
    } else {
        1.0
    };
    let hover_t =
        crate::motion::hover_factor(ui.ctx(), resp.id.with("hov"), resp.hovered() && !selected);
    let select_t = crate::motion::hover_factor(ui.ctx(), resp.id.with("sel"), selected);
    let dot_r = 6.5 * scale;
    let rest_r = crate::motion::lerp_f32(dot_r, 7.9 * scale, hover_t);
    let ring_r = crate::motion::lerp_f32(rest_r, 8.3 * scale, select_t);
    let rest_c = if t.is_dark {
        crate::motion::lerp_color(
            Color32::from_rgba_unmultiplied(255, 255, 255, 52),
            t.text,
            hover_t,
        )
    } else {
        crate::motion::lerp_color(t.border, t.text_muted, hover_t)
    };
    let ring_c = crate::motion::lerp_color(rest_c, t.accent, select_t);
    let rest_w = crate::motion::lerp_f32(0.9 * scale, 1.4 * scale, hover_t);
    let ring_w = crate::motion::lerp_f32(rest_w, 1.8 * scale, select_t);

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let center = rect.center();
        let clean = c.sanitized();
        if clean.a < 0.99 {
            paint_checkerboard_circle(p, center, dot_r, t);
        }
        p.circle_filled(center, dot_r, to_color32(clean));
        p.circle_stroke(center, ring_r, Stroke::new(ring_w, ring_c));
    }
    resp.on_hover_text(tooltip)
}

fn wildcard_dot_sized(
    ui: &mut egui::Ui,
    size: Vec2,
    c: Color,
    selected: bool,
    tooltip: &str,
    t: &Tokens,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let min_dim = if size.x.is_finite() && size.y.is_finite() {
        size.x.min(size.y)
    } else {
        22.0
    };
    let scale = (min_dim / 22.0).clamp(0.85, 1.5);
    let hover_t =
        crate::motion::hover_factor(ui.ctx(), resp.id.with("hov"), resp.hovered() && !selected);
    let select_t = crate::motion::hover_factor(ui.ctx(), resp.id.with("sel"), selected);

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let center = rect.center();
        let ring_r = crate::motion::lerp_f32(7.6 * scale, 8.0 * scale, hover_t.max(select_t));
        let ring_w = crate::motion::lerp_f32(1.4 * scale, 1.7 * scale, hover_t.max(select_t));

        const SEGMENTS: usize = 18;
        for i in 0..SEGMENTS {
            let a0 = (i as f32 / SEGMENTS as f32) * std::f32::consts::TAU;
            let a1 = ((i + 1) as f32 / SEGMENTS as f32) * std::f32::consts::TAU;
            let hue = (i as f32 + 0.5) / SEGMENTS as f32;
            let seg_col = to_color32(Color::from_hsv(hue, 0.90, 0.95, 1.0));
            let p0 = center + Vec2::new(a0.cos(), a0.sin()) * ring_r;
            let p1 = center + Vec2::new(a1.cos(), a1.sin()) * ring_r;
            p.line_segment([p0, p1], Stroke::new(ring_w, seg_col));
        }

        let fill_r = 5.8 * scale;
        let clean = c.sanitized();
        if clean.a < 0.99 {
            paint_checkerboard_circle(p, center, fill_r, t);
        }
        p.circle_filled(center, fill_r, to_color32(clean));
        let inner_rim = if t.is_dark {
            Color32::from_white_alpha(44)
        } else {
            Color32::from_black_alpha(36)
        };
        p.circle_stroke(center, fill_r, Stroke::new(0.6 * scale, inner_rim));

        if select_t > 0.01 {
            let sel_r = crate::motion::lerp_f32(9.2 * scale, 10.0 * scale, select_t);
            p.circle_stroke(
                center,
                sel_r,
                Stroke::new(1.6 * scale, t.accent.gamma_multiply(select_t)),
            );
        }
    }
    resp.on_hover_text(tooltip)
}

fn no_color_dot_sized(
    ui: &mut egui::Ui,
    size: Vec2,
    selected: bool,
    tooltip: &str,
    t: &Tokens,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let min_dim = if size.x.is_finite() && size.y.is_finite() {
        size.x.min(size.y)
    } else {
        22.0
    };
    let scale = (min_dim / 22.0).clamp(0.85, 1.5);
    let hover_t =
        crate::motion::hover_factor(ui.ctx(), resp.id.with("hov"), resp.hovered() && !selected);
    let select_t = crate::motion::hover_factor(ui.ctx(), resp.id.with("sel"), selected);

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let center = rect.center();
        let disc_r = 7.6 * scale;
        let neutral_fill = if t.is_dark {
            Color32::from_rgb(44, 48, 56)
        } else {
            Color32::from_rgb(245, 246, 248)
        };
        let rim_rest = if t.is_dark {
            Color32::from_rgba_unmultiplied(255, 255, 255, 52)
        } else {
            t.border
        };
        let rim_c = crate::motion::lerp_color(rim_rest, t.text_muted, hover_t);
        p.circle_filled(center, disc_r, neutral_fill);
        p.circle_stroke(center, disc_r, Stroke::new(1.1 * scale, rim_c));

        let slash_c = if t.is_dark {
            Color32::from_rgb(0xEF, 0x44, 0x44)
        } else {
            Color32::from_rgb(0xDC, 0x26, 0x26)
        };
        let d = (disc_r - 1.2 * scale) * std::f32::consts::FRAC_1_SQRT_2;
        p.line_segment(
            [center + Vec2::new(-d, d), center + Vec2::new(d, -d)],
            Stroke::new(2.0 * scale, slash_c),
        );

        if select_t > 0.01 {
            let sel_r = crate::motion::lerp_f32(9.2 * scale, 10.0 * scale, select_t);
            p.circle_stroke(
                center,
                sel_r,
                Stroke::new(1.6 * scale, t.accent.gamma_multiply(select_t)),
            );
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip)
}

fn draw_left_zone(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    ui.spacing_mut().item_spacing.x = ZONE_ITEM_SPACING;

    let nb_active = app.view.pages_sidebar_open;
    let tip = if nb_active {
        format!(
            "Notebook: {}\nHide Pages & page navigation sidebar (Shortcut: P)",
            app.session.doc.title
        )
    } else {
        format!(
            "Notebook: {}\nShow Pages & page navigation sidebar (Shortcut: P)",
            app.session.doc.title
        )
    };
    if sidebar_edge_button_sized(
        ui,
        SidebarEdgeButtonSpec {
            edge: SidebarEdge::Left,
            icon: Icon::Notebook,
            label: "",
            max_width: None,
            open: nb_active,
            enabled: true,
        },
        SidebarEdgeSize::Toolbar,
        t,
    )
    .on_hover_text(tip)
    .clicked()
    {
        app.view.pages_sidebar_open = !app.view.pages_sidebar_open;
        if app.view.pages_sidebar_open {
            app.view.notebook_title_draft = app.session.doc.title.clone();
            if ui.ctx().content_rect().width() < crate::DUAL_SIDEBAR_MIN_VIEWPORT {
                app.view.inspector_open = false;
            }
        }
    }

    ui.add_space((EDGE_TAB_NEIGHBOR_GAP - ZONE_ITEM_SPACING).max(0.0));

    if toolbar_icon_button(ui, Icon::Library, None, false, true, t)
        .on_hover_text("Return to Notebooks Home (Shortcut: H)")
        .clicked()
    {
        crate::return_to_home(app);
    }
}

fn clear_overlay_tools(app: &mut PaperCraftApp, close_color_picker: bool) {
    crate::canvas::commit_pending_text(app);
    app.view.text_insert_mode = false;
    app.view.clear_selection();
    app.view.text_style_open = false;
    app.view.smoothing_popover_open = false;
    app.view.text_link_editor_open = false;
    app.view.text_link_draft = None;
    app.view.text_link_input.clear();
    app.view.shape_insert_mode = false;
    app.view.shape_drag_start = None;
    app.view.shape_drag_end = None;
    app.view.shape_drag_touch_id = None;
    if close_color_picker {
        app.view.color_picker.close();
    }
}

fn pen_tool_name(kind: PenKind) -> &'static str {
    match kind {
        PenKind::FountainPen => "FountainPen",
        PenKind::BallPen => "BallPen",
        PenKind::BrushPen => "BrushPen",
        PenKind::Highlighter => "Highlighter",
        PenKind::Pencil => "Pencil",
    }
}

fn clear_ruler_drag(app: &mut PaperCraftApp) {
    app.view.ruler_active = false;
    app.view.ruler_drag_mode = None;
    app.view.ruler_drag_touch_id = None;
    app.view.ruler_drag_anchor = None;
}

fn standard_ruler_dimensions(app: &PaperCraftApp) -> (f64, f64) {
    let page_w = f64::from(app.session.doc.width).max(64.0);
    let page_h = f64::from(app.session.doc.height).max(64.0);
    let length = (page_w * 0.72).clamp(64.0, page_w);
    let page_cap = (page_w.min(page_h) * 0.25).max(8.0);
    let depth = crate::canvas::RULER_DEPTH.min(page_cap).max(8.0);
    (length, depth)
}

fn toggle_ruler_tool(app: &mut PaperCraftApp) {
    let activating = !app.view.ruler_active;
    if activating {
        let angle = if app.view.ruler_initialized && app.view.ruler_angle.is_finite() {
            app.view.ruler_angle
        } else {
            0.0
        };
        let center = app.camera_center_page_point();
        let (length, depth) = standard_ruler_dimensions(app);
        app.view.ruler_center = [center.x, center.y];
        app.view.ruler_angle = angle;
        app.view.ruler_length = length;
        app.view.ruler_depth = depth;
        app.view.ruler_initialized = true;
    }
    app.view.ruler_active = activating;
    if activating {
        clear_overlay_tools(app, true);
        if !matches!(app.session.active_tool, ToolKind::Pen(_)) {
            let last_pen = app.session.active_pen_kind();
            let _ = app
                .session
                .execute("tool.select", &json!({ "tool": pen_tool_name(last_pen) }));
        }
    }
    app.view.ruler_drag_mode = None;
    app.view.ruler_drag_touch_id = None;
    app.view.ruler_drag_anchor = None;
}

fn toggle_text_insert_tool(app: &mut PaperCraftApp) {
    let text_active = app.view.text_insert_mode || app.view.text_editing;
    if text_active {
        crate::canvas::commit_pending_text(app);
        app.view.text_insert_mode = false;
    } else {
        app.view.text_insert_mode = true;
        app.view.text_color_draft = app.session.active_pen().color;
        app.view.text_size_draft = 24.0;
        app.view.text_font_draft = TextFont::Sans;
        app.view.text_line_spacing_draft = papercraft_doc::DEFAULT_TEXT_LINE_SPACING;
        app.view.text_letter_spacing_draft = papercraft_doc::DEFAULT_TEXT_LETTER_SPACING;
        app.view.text_max_width_draft = None;
    }
    app.view.clear_selection();
    app.view.text_style_open = false;
    app.view.smoothing_popover_open = false;
    app.view.text_link_editor_open = false;
    app.view.text_link_draft = None;
    app.view.text_link_input.clear();
    app.view.shape_insert_mode = false;
    clear_ruler_drag(app);
    app.view.shape_drag_start = None;
    app.view.shape_drag_end = None;
    app.view.shape_drag_touch_id = None;
    app.view.color_picker.close();
}

fn draw_tool_section(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    ui.spacing_mut().item_spacing.x = ZONE_ITEM_SPACING;
    let overlay_tool =
        app.view.text_insert_mode || app.view.text_editing || app.view.shape_insert_mode;
    let text_active = app.view.text_insert_mode || app.view.text_editing;
    let active_idx = if app.view.shape_insert_mode {
        Some(9)
    } else if text_active {
        Some(8)
    } else if !overlay_tool {
        match app.session.active_tool {
            ToolKind::Select => Some(0),
            ToolKind::Pen(PenKind::FountainPen) => Some(1),
            ToolKind::Pen(PenKind::BallPen) => Some(2),
            ToolKind::Pen(PenKind::BrushPen) => Some(3),
            ToolKind::Pen(PenKind::Pencil) => Some(4),
            ToolKind::Pen(PenKind::Highlighter) => Some(5),
            ToolKind::Eraser => Some(6),
            ToolKind::LaserPointer => Some(7),
        }
    } else {
        None
    };

    let pen_tip = |kind: PenKind, key: &str| {
        format!("{} (Shortcut: {key})\n{}", kind.label(), kind.description())
    };
    let fountain_tip = pen_tip(PenKind::FountainPen, "1");
    let ball_tip = pen_tip(PenKind::BallPen, "2");
    let brush_tip = pen_tip(PenKind::BrushPen, "3");
    let pencil_tip = pen_tip(PenKind::Pencil, "6");
    let hl_tip = pen_tip(PenKind::Highlighter, "4");

    let select_icon = if app.session.active_tool == ToolKind::Select && app.view.freeform_lasso_mode
    {
        Icon::Lasso
    } else {
        Icon::Select
    };
    let options = [
        SwitchOption::new(
            Some(select_icon),
            None,
            "Select (Shortcut: V)\nSelect and move objects with rectangle or freeform lasso, resize, rotate, copy, or duplicate.",
            true,
        ),
        SwitchOption::new(
            Some(pen_icon(PenKind::FountainPen)),
            None,
            &fountain_tip,
            true,
        ),
        SwitchOption::new(Some(pen_icon(PenKind::BallPen)), None, &ball_tip, true),
        SwitchOption::new(Some(pen_icon(PenKind::BrushPen)), None, &brush_tip, true),
        SwitchOption::new(Some(pen_icon(PenKind::Pencil)), None, &pencil_tip, true),
        SwitchOption::new(Some(pen_icon(PenKind::Highlighter)), None, &hl_tip, true),
        SwitchOption::new(
            Some(Icon::Eraser),
            None,
            "Eraser (Shortcut: 5)\nRemove whole strokes, slice paths, or filter to highlighter marks only.",
            true,
        ),
        SwitchOption::new(
            Some(Icon::LaserPointer),
            None,
            "Laser Pointer (Shortcut: 7)\nTemporary fading spotlight or trail for presentations (never saved to the page).",
            true,
        ),
        SwitchOption::new(
            Some(Icon::Text),
            None,
            "Text Box\nClick anywhere on the page to write formatted rich text, lists, and links.",
            true,
        ),
        SwitchOption::new(
            Some(Icon::ShapeTool),
            None,
            "Shapes\nDraw lines, rectangles, ellipses, triangles, arrows, or polygons with fill and dashed styles.",
            true,
        ),
    ];

    let (clicked, _) = segmented_switch(
        ui,
        "toolbar_primary_tool_switch",
        &options,
        active_idx,
        SwitchLayout::Toolbar,
        t,
    );
    let select_engine_tool = |app: &mut PaperCraftApp, tool: &str| {
        clear_overlay_tools(app, false);
        let _ = app.session.execute("tool.select", &json!({ "tool": tool }));
    };
    match clicked {
        Some(0) => crate::activate_select_tool(app),
        Some(1) => select_engine_tool(app, pen_tool_name(PenKind::FountainPen)),
        Some(2) => select_engine_tool(app, pen_tool_name(PenKind::BallPen)),
        Some(3) => select_engine_tool(app, pen_tool_name(PenKind::BrushPen)),
        Some(4) => select_engine_tool(app, pen_tool_name(PenKind::Pencil)),
        Some(5) => select_engine_tool(app, pen_tool_name(PenKind::Highlighter)),
        Some(6) => select_engine_tool(app, "Eraser"),
        Some(7) => select_engine_tool(app, "LaserPointer"),
        Some(8) => toggle_text_insert_tool(app),
        Some(9) => {
            crate::canvas::commit_pending_text(app);
            app.view.shape_insert_mode = !app.view.shape_insert_mode;
            app.view.text_insert_mode = false;
            app.view.clear_selection();
            app.view.text_style_open = false;
            app.view.smoothing_popover_open = false;
            clear_ruler_drag(app);
            app.view.shape_drag_start = None;
            app.view.shape_drag_end = None;
            app.view.shape_drag_touch_id = None;
            app.view.color_picker.close();
        }
        _ => {}
    }

    if toolbar_icon_button(ui, Icon::Ruler, None, app.view.ruler_active, true, t)
        .on_hover_text(
            "Ruler — draw near either long edge to make a straight line; drag the middle to move it, the round handle to rotate, or set an exact angle in the context tray",
        )
        .clicked()
    {
        toggle_ruler_tool(app);
    }

    if toolbar_icon_button(ui, Icon::Image, None, false, true, t)
        .on_hover_text(
            "Insert image from file (PNG, JPEG, WebP, GIF, BMP, ICO, TIFF) — or paste with Ctrl+V",
        )
        .clicked()
    {
        crate::canvas::commit_pending_text(app);
        let _ = crate::trigger_open_image_dialog(app);
    }

    let sticker_resp = toolbar_icon_button(
        ui,
        Icon::Sticker,
        None,
        app.view.elements_popover_open,
        true,
        t,
    )
    .on_hover_text("Stickers & Elements — stamp AI stickers or your saved notes and figures");
    if sticker_resp.clicked() {
        crate::canvas::commit_pending_text(app);
        app.view.smoothing_popover_open = false;
        app.view.elements_popover_open = !app.view.elements_popover_open;
    }
}

const CONTEXT_TRAY_OUTER_MARGIN: f32 = 12.0;
const CONTEXT_TRAY_INNER_MARGIN_X: f32 = 12.0;
const CONTEXT_TRAY_INNER_MARGIN_Y: f32 = 8.0;
const CONTEXT_TRAY_ROW_GAP: f32 = 5.0;
const CONTEXT_TRAY_MIN_CANVAS_WIDTH: f32 = 184.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BreakTier {
    /// Preferred break point at a hairline section divider (`|`, omitted if a row breaks here).
    Divider,
    /// Logical group boundary without a visible divider.
    Group,
    /// Individual control boundary inside a group.
    Inner,
    /// Tightly cohesive cluster (e.g. color dots); split only when narrower than the cluster.
    Cohesive,
}

impl BreakTier {
    const fn penalty(self) -> f32 {
        match self {
            Self::Divider => 0.0,
            Self::Group => 350.0,
            Self::Inner => 2_500.0,
            Self::Cohesive => 62_500.0,
        }
    }
}

type DockDrawFn<'a> = Box<dyn FnOnce(&mut PaperCraftApp, &mut egui::Ui, &Tokens) + 'a>;

struct DockEntry<'a> {
    width: f32,
    break_before: BreakTier,
    has_divider_before: bool,
    draw: DockDrawFn<'a>,
}

struct DockFlow<'a> {
    entries: Vec<DockEntry<'a>>,
    max_content_w: f32,
    pending_divider: bool,
    pending_group: bool,
}

impl<'a> DockFlow<'a> {
    fn new(max_content_w: f32) -> Self {
        Self {
            entries: Vec::new(),
            max_content_w: max_content_w.max(120.0),
            pending_divider: false,
            pending_group: false,
        }
    }

    fn divider(&mut self) {
        if !self.entries.is_empty() {
            self.pending_divider = true;
        }
    }

    fn group(&mut self) {
        if !self.entries.is_empty() && !self.pending_divider {
            self.pending_group = true;
        }
    }

    fn push(
        &mut self,
        width: f32,
        draw: impl FnOnce(&mut PaperCraftApp, &mut egui::Ui, &Tokens) + 'a,
    ) {
        self.push_with_tier(width, BreakTier::Inner, draw);
    }

    fn push_cohesive(
        &mut self,
        width: f32,
        draw: impl FnOnce(&mut PaperCraftApp, &mut egui::Ui, &Tokens) + 'a,
    ) {
        self.push_with_tier(width, BreakTier::Cohesive, draw);
    }

    fn push_with_tier(
        &mut self,
        width: f32,
        default_tier: BreakTier,
        draw: impl FnOnce(&mut PaperCraftApp, &mut egui::Ui, &Tokens) + 'a,
    ) {
        let has_divider_before = self.pending_divider && !self.entries.is_empty();
        let break_before = if has_divider_before {
            BreakTier::Divider
        } else if self.pending_group && !self.entries.is_empty() {
            BreakTier::Group
        } else {
            default_tier
        };
        self.pending_divider = false;
        self.pending_group = false;
        self.entries.push(DockEntry {
            width: if width.is_finite() {
                width.max(0.0)
            } else {
                0.0
            },
            break_before,
            has_divider_before,
            draw: Box::new(draw),
        });
    }
}

#[derive(Clone, Debug, PartialEq)]
struct PlannedDockRow {
    start: usize,
    end: usize,
    width: f32,
}

#[derive(Clone, Copy, Debug)]
struct DockSpec {
    width: f32,
    break_before: BreakTier,
    has_divider_before: bool,
}

fn plan_dock_rows(
    specs: &[DockSpec],
    max_w: f32,
    spacing_x: f32,
    divider_extra: f32,
) -> Vec<PlannedDockRow> {
    let n = specs.len();
    if n == 0 {
        return Vec::new();
    }

    let mut prefix_w = Vec::with_capacity(n + 1);
    let mut prefix_div = Vec::with_capacity(n + 1);
    prefix_w.push(0.0_f32);
    prefix_div.push(0usize);
    for spec in specs {
        let prev_w = prefix_w.last().copied().unwrap_or(0.0);
        let prev_d = prefix_div.last().copied().unwrap_or(0);
        prefix_w.push(prev_w + spec.width.max(0.0));
        prefix_div.push(prev_d + usize::from(spec.has_divider_before));
    }

    let row_width = |i: usize, j: usize| -> f32 {
        let items_w = prefix_w[j] - prefix_w[i];
        let gaps = (j - i).saturating_sub(1) as f32 * spacing_x;
        let divs = (prefix_div[j] - prefix_div[i + 1]) as f32 * divider_extra;
        items_w + gaps + divs
    };

    let total_w = row_width(0, n);
    if total_w <= max_w + 0.01 {
        return vec![PlannedDockRow {
            start: 0,
            end: n,
            width: total_w,
        }];
    }

    let mut greedy_rows = Vec::new();
    let mut start = 0usize;
    while start < n {
        let mut end = start + 1;
        while end < n && row_width(start, end + 1) <= max_w + 0.01 {
            end += 1;
        }
        greedy_rows.push(PlannedDockRow {
            start,
            end,
            width: row_width(start, end),
        });
        start = end;
    }

    let min_rows = greedy_rows.len();
    if min_rows <= 1 {
        return greedy_rows;
    }

    let avg_lost_per_break = spacing_x + 0.5 * divider_extra;
    let est_total = (total_w - (min_rows - 1) as f32 * avg_lost_per_break).max(1.0);
    let target_w = (est_total / min_rows as f32).clamp(1.0, max_w.max(1.0));

    let stride = n + 1;
    let mut dp = vec![f32::INFINITY; (min_rows + 1) * stride];
    let mut prev = vec![None::<usize>; (min_rows + 1) * stride];
    dp[0] = 0.0;

    for r in 1..=min_rows {
        for j in r..=n {
            let mut best_cost = f32::INFINITY;
            let mut best_i = None;
            for i in (r - 1)..j {
                let prev_cost = dp[(r - 1) * stride + i];
                if !prev_cost.is_finite() {
                    continue;
                }
                let w = row_width(i, j);
                if j > i + 1 && w > max_w + 0.01 {
                    continue;
                }
                let dev = w - target_w;
                let penalty = if i > 0 {
                    specs[i].break_before.penalty()
                } else {
                    0.0
                };
                let cost = prev_cost + dev * dev + penalty;
                if cost < best_cost {
                    best_cost = cost;
                    best_i = Some(i);
                }
            }
            dp[r * stride + j] = best_cost;
            prev[r * stride + j] = best_i;
        }
    }

    if !dp[min_rows * stride + n].is_finite() {
        return greedy_rows;
    }

    let mut out = Vec::with_capacity(min_rows);
    let mut cur = n;
    for r in (1..=min_rows).rev() {
        let Some(split) = prev[r * stride + cur] else {
            return greedy_rows;
        };
        out.push(PlannedDockRow {
            start: split,
            end: cur,
            width: row_width(split, cur),
        });
        cur = split;
    }
    if cur != 0 {
        return greedy_rows;
    }
    out.reverse();
    out
}

const SHAPE_KIND_ITEMS: [(ShapeKind, Icon); 6] = [
    (ShapeKind::Line, Icon::LineShape),
    (ShapeKind::Rectangle, Icon::RectangleShape),
    (ShapeKind::Ellipse, Icon::EllipseShape),
    (ShapeKind::Triangle, Icon::TriangleShape),
    (ShapeKind::Arrow, Icon::ArrowShape),
    (ShapeKind::Polygon, Icon::PolygonShape),
];

fn draw_shape_kind_switch(
    ui: &mut egui::Ui,
    id_salt: impl egui::AsIdSalt,
    active: Option<ShapeKind>,
    tooltip_suffix: &str,
    t: &Tokens,
) -> Option<ShapeKind> {
    let tooltips: Vec<String> = SHAPE_KIND_ITEMS
        .iter()
        .map(|(kind, _)| format!("{}{tooltip_suffix}", kind.label()))
        .collect();
    let options: Vec<SwitchOption<'_>> = SHAPE_KIND_ITEMS
        .iter()
        .enumerate()
        .map(|(idx, (kind, icon))| {
            SwitchOption::new(Some(*icon), Some(kind.label()), &tooltips[idx], true)
        })
        .collect();
    let active_idx = active.and_then(|cur| SHAPE_KIND_ITEMS.iter().position(|(k, _)| *k == cur));
    let (clicked, _) = segmented_switch(
        ui,
        id_salt,
        &options,
        active_idx,
        SwitchLayout::TrayCompact,
        t,
    );
    clicked.and_then(|idx| SHAPE_KIND_ITEMS.get(idx).map(|(kind, _)| *kind))
}

fn push_shape_kind_controls<'a>(
    flow: &mut DockFlow<'a>,
    ui: &egui::Ui,
    id_salt: impl egui::AsIdSalt + Clone + 'a,
    active: Option<ShapeKind>,
    tooltip_suffix: &'a str,
    on_select: impl Fn(&mut PaperCraftApp, ShapeKind) + Clone + 'a,
) {
    let tooltips: Vec<String> = SHAPE_KIND_ITEMS
        .iter()
        .map(|(kind, _)| format!("{}{tooltip_suffix}", kind.label()))
        .collect();
    let options: Vec<SwitchOption<'_>> = SHAPE_KIND_ITEMS
        .iter()
        .enumerate()
        .map(|(idx, (kind, icon))| {
            SwitchOption::new(Some(*icon), Some(kind.label()), &tooltips[idx], true)
        })
        .collect();
    let switch_w = segmented_switch_width(ui, &options, SwitchLayout::TrayCompact);
    if switch_w <= flow.max_content_w {
        flow.push(switch_w, move |app, ui, t| {
            if let Some(kind) = draw_shape_kind_switch(ui, id_salt, active, tooltip_suffix, t) {
                on_select(app, kind);
            }
        });
    } else {
        for (idx, (kind, icon)) in SHAPE_KIND_ITEMS.iter().copied().enumerate() {
            let btn_w = tray_icon_button_width(ui, kind.label());
            let tip = tooltips[idx].clone();
            let cb = on_select.clone();
            flow.push(btn_w, move |app, ui, t| {
                if tray_icon_button(ui, icon, Some(kind.label()), active == Some(kind), true, t)
                    .on_hover_text(tip)
                    .clicked()
                {
                    cb(app, kind);
                }
            });
        }
    }
}

fn push_shape_quick_bar<'a>(app: &PaperCraftApp, ui: &egui::Ui, flow: &mut DockFlow<'a>) {
    let active_kind = app.view.selected_shape_kind;
    push_shape_kind_controls(
        flow,
        ui,
        "toolbar_shape_insert_kind_switch",
        Some(active_kind),
        " — drag across the page to draw",
        |app, kind| {
            app.view.selected_shape_kind = kind;
            app.view.shape_drag_start = None;
            app.view.shape_drag_end = None;
            app.view.shape_drag_touch_id = None;
        },
    );

    let initial_angle = normalized_degrees(app.view.shape_angle_degrees);
    flow.push(RULER_ANGLE_CONTROL_WIDTH, move |app, ui, _t| {
        let mut angle_degrees = initial_angle;
        let angle = ui.add_sized(
            Vec2::new(RULER_ANGLE_CONTROL_WIDTH, CONTEXT_TRAY_ROW_HEIGHT),
            egui::DragValue::new(&mut angle_degrees)
                .range(-180.0..=180.0)
                .fixed_decimals(0)
                .suffix("°")
                .speed(1.0),
        );
        if angle.changed() {
            app.view.shape_angle_degrees = normalized_degrees(angle_degrees);
        }
        angle.on_hover_text(
            "Shape angle in degrees — double-click to type a value or drag to adjust",
        );
    });

    let curve_enabled = matches!(
        active_kind,
        ShapeKind::Rectangle | ShapeKind::Triangle | ShapeKind::Polygon
    );
    flow.push(SHAPE_CURVE_CONTROL_WIDTH, move |app, ui, _t| {
        let curve = ui.add_enabled_ui(curve_enabled, |ui| {
            ui.add_sized(
                Vec2::new(SHAPE_CURVE_CONTROL_WIDTH, CONTEXT_TRAY_ROW_HEIGHT),
                egui::DragValue::new(&mut app.view.shape_curve)
                    .range(0.0..=128.0)
                    .fixed_decimals(0)
                    .prefix("Curve ")
                    .suffix(" pt")
                    .speed(1.0),
            )
        });
        curve.response.on_hover_text(
            "Curve in page points — rounds rectangle, triangle, and polygon corners. Set 0 for sharp corners.",
        );
    });

    flow.divider();
    push_shape_line_style_controls(flow, Some(app.view.shape_line_style), |app, style| {
        app.view.shape_line_style = style;
    });
}

fn draw_line_style_button(
    ui: &mut egui::Ui,
    style: ShapeLineStyle,
    selected: bool,
    t: &Tokens,
) -> egui::Response {
    let (rect, resp, select_t) = draw_size_tile_bg(ui, t, selected);
    if ui.is_rect_visible(rect) {
        let stroke_c = crate::motion::lerp_color(t.text_muted, t.accent, select_t);
        let y = rect.center().y;
        let x0 = rect.left() + 5.0;
        let x1 = rect.right() - 5.0;
        let p = ui.painter();
        match style {
            ShapeLineStyle::Solid => {
                p.line_segment(
                    [egui::pos2(x0, y), egui::pos2(x1, y)],
                    Stroke::new(2.0, stroke_c),
                );
            }
            ShapeLineStyle::Dashed => {
                p.line_segment(
                    [egui::pos2(x0, y), egui::pos2(x0 + 6.5, y)],
                    Stroke::new(2.0, stroke_c),
                );
                p.line_segment(
                    [egui::pos2(x1 - 6.5, y), egui::pos2(x1, y)],
                    Stroke::new(2.0, stroke_c),
                );
            }
            ShapeLineStyle::Dotted => {
                for frac in [0.0_f32, 0.5, 1.0] {
                    let x = x0 + 1.8 + frac * (x1 - x0 - 3.6);
                    p.circle_filled(egui::pos2(x, y), 1.55, stroke_c);
                }
            }
        }
    }
    let tip = match style {
        ShapeLineStyle::Solid => "Solid outline",
        ShapeLineStyle::Dashed => "Dashed outline",
        ShapeLineStyle::Dotted => "Dotted outline",
    };
    resp.on_hover_text(tip)
}

fn push_shape_line_style_controls<'a>(
    flow: &mut DockFlow<'a>,
    active_line_style: Option<ShapeLineStyle>,
    on_select: impl Fn(&mut PaperCraftApp, ShapeLineStyle) + Clone + 'a,
) {
    for (idx, &style) in ShapeLineStyle::ALL.iter().enumerate() {
        let selected = active_line_style == Some(style);
        let cb = on_select.clone();
        let draw_fn = move |app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens| {
            if draw_line_style_button(ui, style, selected, t).clicked() {
                cb(app, style);
            }
        };
        if idx == 0 {
            flow.push(CONTEXT_TRAY_TILE_SIZE, draw_fn);
        } else {
            flow.push_cohesive(CONTEXT_TRAY_TILE_SIZE, draw_fn);
        }
    }
}

pub(crate) fn selected_targets_json(objects: &[crate::SelectedObject]) -> Vec<serde_json::Value> {
    objects
        .iter()
        .map(|object| match object {
            crate::SelectedObject::Stroke(id) => json!({ "kind": "stroke", "id": id.0 }),
            crate::SelectedObject::Shape(index) => json!({ "kind": "shape", "index": index }),
            crate::SelectedObject::Text(index) => json!({ "kind": "text", "index": index }),
            crate::SelectedObject::Image(index) => json!({ "kind": "image", "index": index }),
        })
        .collect()
}

fn set_selected_color(app: &mut PaperCraftApp, color: Color) {
    if app.view.selected_objects.len() > 1 {
        let targets = selected_targets_json(&app.view.selected_objects);
        let _ = app.session.execute(
            "selection.update",
            &json!({ "targets": targets, "color": color }),
        );
        return;
    }
    let Some(selected) = app.view.selected_object else {
        return;
    };
    let _ = match selected {
        crate::SelectedObject::Stroke(id) => app
            .session
            .execute("stroke.update", &json!({ "id": id.0, "color": color })),
        crate::SelectedObject::Shape(index) => app
            .session
            .execute("shape.update", &json!({ "index": index, "color": color })),
        crate::SelectedObject::Text(index) => app
            .session
            .execute("text.update", &json!({ "index": index, "color": color })),
        crate::SelectedObject::Image(_) => Ok(serde_json::Value::Null),
    };
}

pub(crate) fn effective_shape_insert_border(app: &PaperCraftApp) -> Option<Color> {
    app.view
        .shape_border_enabled
        .then(|| {
            app.view
                .shape_border_color
                .unwrap_or_else(|| app.session.active_pen().color)
                .sanitized()
        })
        .filter(|c| c.a > 0.0)
}

pub(crate) fn effective_shape_insert_fill(app: &PaperCraftApp) -> Option<Color> {
    app.view
        .shape_fill_enabled
        .then(|| app.view.shape_fill_color.map(Color::sanitized))
        .flatten()
        .filter(|c| c.a > 0.0)
}

pub(crate) fn is_shape_color_context(app: &PaperCraftApp) -> bool {
    if app.view.text_insert_mode || app.view.text_editing {
        return false;
    }
    if app.view.shape_insert_mode {
        return true;
    }
    if app.session.active_tool != ToolKind::Select {
        return false;
    }
    let has_selection = !app.view.selected_objects.is_empty() || app.view.selected_object.is_some();
    if !has_selection {
        return false;
    }
    if app.view.selected_objects.is_empty() {
        matches!(
            app.view.selected_object,
            Some(crate::SelectedObject::Shape(_))
        )
    } else {
        app.view
            .selected_objects
            .iter()
            .all(|object| matches!(object, crate::SelectedObject::Shape(_)))
    }
}

pub(crate) fn apply_shape_color_from_dock(app: &mut PaperCraftApp, color: Option<Color>) {
    let target_channel = app.view.shape_color_target;
    if app.view.shape_insert_mode {
        match (target_channel, color) {
            (crate::ShapeColorTarget::Border, Some(c)) => {
                let clean = c.sanitized();
                app.view.shape_border_enabled = clean.a > 0.0;
                app.view.shape_border_color = Some(clean);
            }
            (crate::ShapeColorTarget::Border, None) => {
                app.view.shape_border_enabled = false;
            }
            (crate::ShapeColorTarget::Fill, Some(c)) => {
                let clean = c.sanitized();
                app.view.shape_fill_enabled = clean.a > 0.0;
                app.view.shape_fill_color = Some(clean);
            }
            (crate::ShapeColorTarget::Fill, None) => {
                app.view.shape_fill_enabled = false;
            }
        }
        return;
    }
    if app.session.active_tool != ToolKind::Select {
        return;
    }
    let selection: Vec<crate::SelectedObject> = if app.view.selected_objects.is_empty() {
        app.view.selected_object.into_iter().collect()
    } else {
        app.view.selected_objects.clone()
    };
    if selection.is_empty()
        || !selection
            .iter()
            .all(|object| matches!(object, crate::SelectedObject::Shape(_)))
    {
        return;
    }
    if selection.len() > 1 {
        let targets = selected_targets_json(&selection);
        let params = match (target_channel, color) {
            (crate::ShapeColorTarget::Border, Some(c)) => {
                json!({ "targets": targets, "color": c.sanitized() })
            }
            (crate::ShapeColorTarget::Border, None) => {
                json!({ "targets": targets, "color": Color::TRANSPARENT })
            }
            (crate::ShapeColorTarget::Fill, Some(c)) => {
                let clean = c.sanitized();
                if clean.a > 0.0 {
                    json!({ "targets": targets, "fill_color": clean })
                } else {
                    json!({ "targets": targets, "clear_fill": true })
                }
            }
            (crate::ShapeColorTarget::Fill, None) => {
                json!({ "targets": targets, "clear_fill": true })
            }
        };
        let _ = app.session.execute("selection.update", &params);
        return;
    }
    let Some(crate::SelectedObject::Shape(index)) = selection.first().copied() else {
        return;
    };
    let params = match (target_channel, color) {
        (crate::ShapeColorTarget::Border, Some(c)) => {
            json!({ "index": index, "color": c.sanitized() })
        }
        (crate::ShapeColorTarget::Border, None) => {
            json!({ "index": index, "color": Color::TRANSPARENT })
        }
        (crate::ShapeColorTarget::Fill, Some(c)) => {
            let clean = c.sanitized();
            if clean.a > 0.0 {
                json!({ "index": index, "fill_color": clean })
            } else {
                json!({ "index": index, "clear_fill": true })
            }
        }
        (crate::ShapeColorTarget::Fill, None) => {
            json!({ "index": index, "clear_fill": true })
        }
    };
    let _ = app.session.execute("shape.update", &params);
}

#[derive(Clone, Copy)]
enum PaletteApplyAction {
    Pen(PenKind),
    Shape,
    Selection,
    TextDraft,
}

struct PaletteDockSpec {
    color: Option<Color>,
    color_is_common: bool,
    target: PaletteTarget,
    picker_kind: PenKind,
    include_contrast_tip: bool,
    picker_tooltip: &'static str,
    action: PaletteApplyAction,
    shape_channels: Option<ShapeChannelInfo>,
}

struct ColorDockLayout {
    cols: usize,
    cell_size: Vec2,
    inner_w: f32,
    frame_w: f32,
    frame_h: f32,
}

fn plan_color_dock_layout(
    swatch_count: usize,
    has_wildcard: bool,
    has_palette_btn: bool,
    has_shape_channels: bool,
    max_frame_h: f32,
) -> ColorDockLayout {
    let shape_block_h = if has_shape_channels {
        COLOR_DOCK_SHAPE_SWITCH_H + COLOR_DOCK_ITEM_GAP + COLOR_DOCK_DIVIDER_H + COLOR_DOCK_ITEM_GAP
    } else {
        0.0
    };
    let special_row_h = if has_shape_channels {
        COLOR_DOCK_SHAPE_SPECIAL_SIZE.y
    } else {
        COLOR_DOCK_WILDCARD_SIZE.y
    };
    let wildcard_block_h = if has_wildcard {
        special_row_h + COLOR_DOCK_ITEM_GAP + COLOR_DOCK_DIVIDER_H + COLOR_DOCK_ITEM_GAP
    } else {
        0.0
    };
    let palette_block_h = if has_palette_btn {
        2.0 + COLOR_DOCK_ITEM_GAP + COLOR_DOCK_PALETTE_BTN_SIZE
    } else {
        0.0
    };
    let overhead_h = COLOR_DOCK_INNER_MARGIN_Y * 2.0
        + COLOR_DOCK_GRIP_SIZE.y
        + COLOR_DOCK_ITEM_GAP
        + COLOR_DOCK_DIVIDER_H
        + COLOR_DOCK_ITEM_GAP
        + shape_block_h
        + wildcard_block_h
        + palette_block_h;
    let swatch_h = if has_shape_channels {
        COLOR_DOCK_SHAPE_SWATCH_SIZE.y
    } else {
        COLOR_DOCK_SWATCH_SIZE.y
    };
    let avail_swatch_h = (max_frame_h - overhead_h).max(swatch_h);
    let max_rows = ((avail_swatch_h + COLOR_DOCK_ITEM_GAP) / (swatch_h + COLOR_DOCK_ITEM_GAP))
        .floor()
        .max(1.0) as usize;
    let min_cols = if has_shape_channels { 2 } else { 1 };
    let cols = if swatch_count <= max_rows && min_cols == 1 {
        1
    } else {
        swatch_count.div_ceil(max_rows).clamp(min_cols, 4)
    };
    let rows = if swatch_count == 0 {
        0
    } else {
        swatch_count.div_ceil(cols)
    };
    let cell_size = if cols == 1 {
        COLOR_DOCK_SWATCH_SIZE
    } else if has_shape_channels && cols == 2 {
        COLOR_DOCK_SHAPE_SWATCH_SIZE
    } else {
        COLOR_DOCK_GRID_CELL_SIZE
    };
    let grid_w = if cols == 1 {
        COLOR_DOCK_SWATCH_SIZE.x
    } else {
        cols as f32 * cell_size.x + (cols - 1) as f32 * COLOR_DOCK_ITEM_GAP
    };
    let grid_h = if rows == 0 {
        0.0
    } else {
        rows as f32 * cell_size.y + (rows - 1) as f32 * COLOR_DOCK_ITEM_GAP
    };
    let inner_w = grid_w
        .max(COLOR_DOCK_GRIP_SIZE.x)
        .max(if has_shape_channels {
            COLOR_DOCK_SHAPE_SWITCH_W
        } else {
            0.0
        })
        .max(if has_wildcard {
            if has_shape_channels {
                COLOR_DOCK_SHAPE_SPECIAL_SIZE.x * 2.0 + COLOR_DOCK_ITEM_GAP
            } else {
                COLOR_DOCK_WILDCARD_SIZE.x
            }
        } else {
            0.0
        })
        .max(if has_palette_btn {
            COLOR_DOCK_PALETTE_BTN_SIZE
        } else {
            0.0
        });
    ColorDockLayout {
        cols,
        cell_size,
        inner_w,
        frame_w: inner_w + COLOR_DOCK_INNER_MARGIN_X * 2.0,
        frame_h: overhead_h + grid_h,
    }
}

fn summarize_channel_preview(mut iter: impl Iterator<Item = Option<Color>>) -> ChannelPreview {
    let Some(first) = iter.next() else {
        return ChannelPreview::None;
    };
    let first_col = first.map(Color::sanitized).filter(|c| c.a > 0.0);
    for next in iter {
        let next_col = next.map(Color::sanitized).filter(|c| c.a > 0.0);
        match (first_col, next_col) {
            (None, None) => {}
            (Some(a), Some(b)) if same_rgba(a, b) => {}
            _ => return ChannelPreview::Mixed,
        }
    }
    match first_col {
        Some(c) => ChannelPreview::Color(c),
        None => ChannelPreview::None,
    }
}

fn active_channel_color(info: ShapeChannelInfo) -> (Option<Color>, bool) {
    let preview = match info.active_target {
        crate::ShapeColorTarget::Border => info.border,
        crate::ShapeColorTarget::Fill => info.fill,
    };
    match preview {
        ChannelPreview::Color(c) => (Some(c), true),
        ChannelPreview::None => (None, true),
        ChannelPreview::Mixed => (None, false),
    }
}

fn color_dock_mode(app: &PaperCraftApp) -> Option<ColorDockMode> {
    if app.view.text_insert_mode || app.view.text_editing {
        return Some(ColorDockMode::TextInsert);
    }
    if app.view.shape_insert_mode {
        let border =
            effective_shape_insert_border(app).map_or(ChannelPreview::None, ChannelPreview::Color);
        let fill =
            effective_shape_insert_fill(app).map_or(ChannelPreview::None, ChannelPreview::Color);
        return Some(ColorDockMode::ShapeInsert { border, fill });
    }
    if app.session.active_tool == ToolKind::Select
        && (!app.view.selected_objects.is_empty() || app.view.selected_object.is_some())
    {
        let selection: Vec<crate::SelectedObject> = if app.view.selected_objects.is_empty() {
            app.view.selected_object.into_iter().collect()
        } else {
            app.view.selected_objects.clone()
        };
        if selection.len() > 1 {
            if selection
                .iter()
                .all(|object| matches!(object, crate::SelectedObject::Shape(_)))
            {
                let shapes: Vec<&papercraft_doc::GeometricShape> = selection
                    .iter()
                    .filter_map(|object| match object {
                        crate::SelectedObject::Shape(idx) => {
                            app.session.doc.shapes.get(*idx).map(|s| s.as_ref())
                        }
                        _ => None,
                    })
                    .collect();
                if shapes.is_empty() {
                    return None;
                }
                let border = summarize_channel_preview(
                    shapes
                        .iter()
                        .map(|s| (s.color.a > 0.0).then_some(s.color.sanitized())),
                );
                let fill = summarize_channel_preview(
                    shapes
                        .iter()
                        .map(|s| s.fill_color.map(Color::sanitized).filter(|fc| fc.a > 0.0)),
                );
                let info = ShapeChannelInfo {
                    active_target: app.view.shape_color_target,
                    border,
                    fill,
                };
                let (color, color_is_common) = active_channel_color(info);
                return Some(ColorDockMode::Selection {
                    color,
                    color_is_common,
                    target: PaletteTarget::Ink,
                    picker_kind: PenKind::FountainPen,
                    shape_channels: Some(info),
                });
            }
            let all_highlighters = selection.iter().all(|object| match object {
                crate::SelectedObject::Stroke(id) => app
                    .session
                    .doc
                    .strokes
                    .iter()
                    .find(|stroke| stroke.id == *id)
                    .is_some_and(|stroke| stroke.pen.kind == PenKind::Highlighter),
                _ => false,
            });
            let picker_kind = if all_highlighters {
                PenKind::Highlighter
            } else {
                PenKind::FountainPen
            };
            let first_color = selection
                .iter()
                .find_map(|object| selected_object_color(app, *object))?;
            let color_is_common = selection.iter().all(|object| {
                selected_object_color(app, *object)
                    .is_some_and(|color| same_rgba(color, first_color))
            });
            return Some(ColorDockMode::Selection {
                color: Some(first_color),
                color_is_common,
                target: PaletteTarget::for_pen(picker_kind),
                picker_kind,
                shape_channels: None,
            });
        }
        return match selection.first().copied()? {
            crate::SelectedObject::Stroke(id) => {
                let stroke = app.session.doc.strokes.iter().find(|s| s.id == id)?;
                Some(ColorDockMode::Selection {
                    color: Some(stroke.pen.color),
                    color_is_common: true,
                    target: PaletteTarget::for_pen(stroke.pen.kind),
                    picker_kind: stroke.pen.kind,
                    shape_channels: None,
                })
            }
            crate::SelectedObject::Shape(index) => {
                let shape = app.session.doc.shapes.get(index)?;
                let border = if shape.color.a > 0.0 {
                    ChannelPreview::Color(shape.color.sanitized())
                } else {
                    ChannelPreview::None
                };
                let fill = match shape
                    .fill_color
                    .map(Color::sanitized)
                    .filter(|fc| fc.a > 0.0)
                {
                    Some(fc) => ChannelPreview::Color(fc),
                    None => ChannelPreview::None,
                };
                let info = ShapeChannelInfo {
                    active_target: app.view.shape_color_target,
                    border,
                    fill,
                };
                let (color, color_is_common) = active_channel_color(info);
                Some(ColorDockMode::Selection {
                    color,
                    color_is_common,
                    target: PaletteTarget::Ink,
                    picker_kind: PenKind::FountainPen,
                    shape_channels: Some(info),
                })
            }
            crate::SelectedObject::Text(index) => {
                let item = app.session.doc.texts.get(index)?.as_ref().sanitized();
                let effective_color = item.effective_color_across(0, item.text.len());
                Some(ColorDockMode::Selection {
                    color: Some(effective_color.unwrap_or(item.color)),
                    color_is_common: effective_color.is_some(),
                    target: PaletteTarget::Ink,
                    picker_kind: PenKind::FountainPen,
                    shape_channels: None,
                })
            }
            crate::SelectedObject::Image(_) => None,
        };
    }
    match app.session.active_tool {
        ToolKind::Pen(kind) => Some(ColorDockMode::Pen(kind)),
        ToolKind::LaserPointer => Some(ColorDockMode::LaserPointer),
        ToolKind::Eraser | ToolKind::Select => None,
    }
}

fn palette_dock_spec(app: &PaperCraftApp, mode: &ColorDockMode) -> Option<PaletteDockSpec> {
    match mode {
        ColorDockMode::Pen(kind) => {
            let pen = app.session.pen(*kind);
            Some(PaletteDockSpec {
                color: Some(pen.color),
                color_is_common: true,
                target: PaletteTarget::for_pen(*kind),
                picker_kind: *kind,
                include_contrast_tip: true,
                picker_tooltip: "Palettes — Switch, duplicate, or create custom color palettes",
                action: PaletteApplyAction::Pen(*kind),
                shape_channels: None,
            })
        }
        ColorDockMode::ShapeInsert { border, fill } => {
            let info = ShapeChannelInfo {
                active_target: app.view.shape_color_target,
                border: *border,
                fill: *fill,
            };
            let (color, color_is_common) = active_channel_color(info);
            Some(PaletteDockSpec {
                color,
                color_is_common,
                target: PaletteTarget::Ink,
                picker_kind: PenKind::FountainPen,
                include_contrast_tip: false,
                picker_tooltip: "Palettes — Switch, duplicate, or create custom color palettes",
                action: PaletteApplyAction::Shape,
                shape_channels: Some(info),
            })
        }
        ColorDockMode::Selection {
            color,
            color_is_common,
            target,
            picker_kind,
            shape_channels,
        } => Some(PaletteDockSpec {
            color: *color,
            color_is_common: *color_is_common,
            target: *target,
            picker_kind: *picker_kind,
            include_contrast_tip: false,
            picker_tooltip: "Palettes — Switch, duplicate, or create custom color palettes",
            action: if shape_channels.is_some() {
                PaletteApplyAction::Shape
            } else {
                PaletteApplyAction::Selection
            },
            shape_channels: *shape_channels,
        }),
        ColorDockMode::TextInsert => {
            let (_, range_start, range_end) = draft_text_range(app);
            let snapshot = draft_text_snapshot(app);
            let effective_color = snapshot.effective_color_across(range_start, range_end);
            Some(PaletteDockSpec {
                color: Some(effective_color.unwrap_or(app.view.text_color_draft)),
                color_is_common: effective_color.is_some(),
                target: PaletteTarget::Ink,
                picker_kind: PenKind::FountainPen,
                include_contrast_tip: false,
                picker_tooltip: "Palettes — Switch, duplicate, or create custom color palettes",
                action: PaletteApplyAction::TextDraft,
                shape_channels: None,
            })
        }
        ColorDockMode::LaserPointer => None,
    }
}

fn apply_palette_dock_color(
    app: &mut PaperCraftApp,
    action: PaletteApplyAction,
    sw_color: Option<Color>,
) {
    match action {
        PaletteApplyAction::Shape => {
            apply_shape_color_from_dock(app, sw_color);
        }
        PaletteApplyAction::Pen(kind) => {
            let Some(color) = sw_color else {
                return;
            };
            let _ = app
                .session
                .execute("pen.setSettings", &json!({ "kind": kind, "color": color }));
        }
        PaletteApplyAction::Selection => {
            let Some(color) = sw_color else {
                return;
            };
            set_selected_color(app, color);
        }
        PaletteApplyAction::TextDraft => {
            let Some(color) = sw_color else {
                return;
            };
            apply_draft_text_color(app, color);
            if app.view.text_editing {
                app.view.text_editor_needs_focus = true;
            }
        }
    }
}

fn draw_color_dock_grip(
    ui: &mut egui::Ui,
    grip_id: egui::Id,
    inner_w: f32,
    is_dragging: bool,
    t: &Tokens,
) -> egui::Response {
    let (grip_slot, _) =
        ui.allocate_exact_size(Vec2::new(inner_w, COLOR_DOCK_GRIP_SIZE.y), Sense::hover());
    let grip_rect = egui::Rect::from_center_size(grip_slot.center(), COLOR_DOCK_GRIP_SIZE);
    let grip_resp = ui.interact(grip_rect, grip_id, Sense::click_and_drag());
    let active_drag = is_dragging || grip_resp.dragged();
    let hov_t = crate::motion::hover_factor(
        ui.ctx(),
        grip_id.with("hov"),
        grip_resp.hovered() && !active_drag,
    );
    let act_t = crate::motion::hover_factor(ui.ctx(), grip_id.with("act"), active_drag);
    let pill_fill = crate::motion::lerp_color(
        crate::motion::lerp_color(Color32::TRANSPARENT, t.border.gamma_multiply(0.45), hov_t),
        t.accent_soft,
        act_t,
    );
    let icon_color = crate::motion::lerp_color(
        crate::motion::lerp_color(t.text_muted, t.text, hov_t),
        t.accent,
        act_t,
    );
    if ui.is_rect_visible(grip_rect) {
        ui.painter().rect_filled(
            grip_rect,
            CornerRadius::same(COLOR_DOCK_INNER_CORNER_RADIUS),
            pill_fill,
        );
        let icon_rect = egui::Rect::from_center_size(
            grip_rect.center(),
            Vec2::splat(COLOR_DOCK_GRIP_ICON_SIZE),
        );
        paint_icon(ui.painter(), icon_rect, Icon::GripHorizontal, icon_color);
    }
    let grip_resp = if active_drag {
        grip_resp.on_hover_cursor(egui::CursorIcon::Grabbing)
    } else {
        grip_resp.on_hover_cursor(egui::CursorIcon::Grab)
    };
    if !active_drag {
        grip_resp.clone().on_hover_text(
            "Drag to move colors anywhere\nDouble-click to dock back to the left side",
        )
    } else {
        grip_resp
    }
}

fn draw_color_dock_divider(ui: &mut egui::Ui, inner_w: f32, t: &Tokens) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(inner_w, COLOR_DOCK_DIVIDER_H), Sense::hover());
    if ui.is_rect_visible(rect) {
        let y = rect.center().y;
        ui.painter().line_segment(
            [
                egui::pos2(rect.left() + 4.0, y),
                egui::pos2(rect.right() - 4.0, y),
            ],
            Stroke::new(1.0, t.border.gamma_multiply(0.8)),
        );
    }
}

fn draw_color_dock_swatch_grid(
    ui: &mut egui::Ui,
    layout: &ColorDockLayout,
    count: usize,
    mut draw_cell: impl FnMut(&mut egui::Ui, usize, Vec2),
) {
    if count == 0 {
        return;
    }
    if layout.cols <= 1 {
        for idx in 0..count {
            draw_cell(ui, idx, layout.cell_size);
        }
        return;
    }
    for row_start in (0..count).step_by(layout.cols) {
        let row_end = (row_start + layout.cols).min(count);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(COLOR_DOCK_ITEM_GAP);
            for idx in row_start..row_end {
                draw_cell(ui, idx, layout.cell_size);
            }
        });
    }
}

fn draw_channel_indicator_dot(
    painter: &egui::Painter,
    center: egui::Pos2,
    preview: ChannelPreview,
    paper_color: papercraft_doc::PaperColor,
    theme: papercraft_doc::UiTheme,
    auto_contrast: bool,
    t: &Tokens,
) {
    let r = 4.5_f32;
    match preview {
        ChannelPreview::Color(c) => {
            let resolved = paper_color.resolve_ink_color(theme, auto_contrast, c, false);
            if resolved.a < 0.99 {
                paint_checkerboard_circle(painter, center, r, t);
            }
            painter.circle_filled(center, r, to_color32(resolved));
            let rim = if t.is_dark {
                Color32::from_white_alpha(44)
            } else {
                Color32::from_black_alpha(36)
            };
            painter.circle_stroke(center, r, Stroke::new(0.7, rim));
        }
        ChannelPreview::None => {
            let neutral_fill = if t.is_dark {
                Color32::from_rgb(44, 48, 56)
            } else {
                Color32::from_rgb(245, 246, 248)
            };
            let rim = if t.is_dark {
                Color32::from_rgba_unmultiplied(255, 255, 255, 52)
            } else {
                t.border
            };
            let slash_c = if t.is_dark {
                Color32::from_rgb(0xEF, 0x44, 0x44)
            } else {
                Color32::from_rgb(0xDC, 0x26, 0x26)
            };
            painter.circle_filled(center, r, neutral_fill);
            painter.circle_stroke(center, r, Stroke::new(0.8, rim));
            let d = (r - 0.8) * std::f32::consts::FRAC_1_SQRT_2;
            painter.line_segment(
                [center + Vec2::new(-d, d), center + Vec2::new(d, -d)],
                Stroke::new(1.2, slash_c),
            );
        }
        ChannelPreview::Mixed => {
            painter.circle_stroke(center, r - 0.4, Stroke::new(1.2, t.text_muted));
        }
    }
}

fn draw_shape_channel_switch(
    ui: &mut egui::Ui,
    inner_w: f32,
    info: ShapeChannelInfo,
    paper_color: papercraft_doc::PaperColor,
    theme: papercraft_doc::UiTheme,
    auto_contrast: bool,
    t: &Tokens,
) -> Option<crate::ShapeColorTarget> {
    let track_w = inner_w.max(COLOR_DOCK_SHAPE_SWITCH_W);
    let track_h = COLOR_DOCK_SHAPE_SWITCH_H;
    let track_pad = 2.0_f32;
    let seg_gap = 2.0_f32;
    let seg_w = (track_w - track_pad * 2.0).max(20.0);
    let seg_h = ((track_h - track_pad * 2.0 - seg_gap) * 0.5).max(16.0);

    let track_id = ui.make_persistent_id("color_dock_shape_channel_switch");
    let (track_rect, _) = ui.allocate_exact_size(Vec2::new(track_w, track_h), Sense::hover());

    let seg_x = track_rect.left() + track_pad;
    let seg_rects = [
        egui::Rect::from_min_size(
            egui::pos2(seg_x, track_rect.top() + track_pad),
            Vec2::new(seg_w, seg_h),
        ),
        egui::Rect::from_min_size(
            egui::pos2(seg_x, track_rect.top() + track_pad + seg_h + seg_gap),
            Vec2::new(seg_w, seg_h),
        ),
    ];

    let segments = [
        (
            crate::ShapeColorTarget::Border,
            Icon::RectangleShape,
            "Border",
            info.border,
            "Border — edit shape outline color",
        ),
        (
            crate::ShapeColorTarget::Fill,
            Icon::FillBucket,
            "Fill",
            info.fill,
            "Fill — edit shape interior fill color",
        ),
    ];

    let active_idx = usize::from(info.active_target == crate::ShapeColorTarget::Fill);
    let mut clicked = None;
    let mut hovered = [false; 2];
    for (idx, (target, _, _, _, tip)) in segments.iter().enumerate() {
        let seg_id = track_id.with(("seg", idx));
        let resp = ui
            .interact(seg_rects[idx], seg_id, Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(*tip);
        if resp.clicked() {
            clicked = Some(*target);
        }
        hovered[idx] = resp.hovered();
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

        for idx in 0..2 {
            let is_active = active_idx == idx;
            let hover_t = crate::motion::hover_factor(
                ui.ctx(),
                track_id.with(("hov", idx)),
                hovered[idx] && !is_active,
            );
            if hover_t > 0.02 {
                p.rect_filled(
                    seg_rects[idx],
                    CornerRadius::same(6),
                    t.border.gamma_multiply(0.35 * hover_t),
                );
            }
        }

        let target_r = seg_rects[active_idx];
        let rel_y0 = target_r.top() - track_rect.top();
        let rel_y1 = target_r.bottom() - track_rect.top();
        let anim_y0 = track_rect.top()
            + ui.ctx().animate_value_with_time(
                track_id.with("pill_y0"),
                rel_y0,
                crate::motion::MICRO_DURATION_S,
            );
        let anim_y1 = track_rect.top()
            + ui.ctx().animate_value_with_time(
                track_id.with("pill_y1"),
                rel_y1,
                crate::motion::MICRO_DURATION_S,
            );
        let pill_rect = egui::Rect::from_min_max(
            egui::pos2(target_r.left(), anim_y0),
            egui::pos2(target_r.right(), anim_y1),
        );
        p.rect(
            pill_rect,
            CornerRadius::same(6),
            t.accent_soft,
            Stroke::new(1.2, t.accent),
            StrokeKind::Inside,
        );

        for (idx, (_, icon, label, preview, _)) in segments.iter().enumerate() {
            let is_active = active_idx == idx;
            let select_t =
                crate::motion::hover_factor(ui.ctx(), track_id.with(("sel", idx)), is_active);
            let hover_t = crate::motion::hover_factor(
                ui.ctx(),
                track_id.with(("hov", idx)),
                hovered[idx] && !is_active,
            );
            let rest_fg = crate::motion::lerp_color(t.text_muted, t.text, 0.35);
            let hovered_fg = crate::motion::lerp_color(rest_fg, t.text, hover_t);
            let fg = crate::motion::lerp_color(hovered_fg, t.accent, select_t);

            let seg_r = seg_rects[idx];
            let label_galley =
                p.layout_no_wrap((*label).to_string(), FontId::proportional(9.5), fg);
            let icon_dim = 12.0_f32;
            let dot_d = 9.0_f32;
            let h_gap = 4.0_f32;
            let v_gap = 1.5_f32;
            let total_h = icon_dim + v_gap + label_galley.size().y;
            let start_y = seg_r.center().y - total_h * 0.5;
            let top_cy = start_y + icon_dim * 0.5;
            let top_w = icon_dim + h_gap + dot_d;
            let top_x = seg_r.center().x - top_w * 0.5;

            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(top_x + icon_dim * 0.5, top_cy),
                Vec2::splat(icon_dim),
            );
            paint_icon(p, icon_rect, *icon, fg);

            let dot_center = egui::pos2(top_x + icon_dim + h_gap + dot_d * 0.5, top_cy);
            draw_channel_indicator_dot(
                p,
                dot_center,
                *preview,
                paper_color,
                theme,
                auto_contrast,
                t,
            );

            let text_pos = egui::pos2(
                seg_r.center().x - label_galley.size().x * 0.5,
                start_y + icon_dim + v_gap,
            );
            p.galley_with_override_text_color(text_pos, label_galley, fg);
        }
    }

    clicked
}

fn draw_palette_dock_controls(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    layout: &ColorDockLayout,
    spec: &PaletteDockSpec,
    t: &Tokens,
) {
    let theme = app.session.prefs.theme;
    let (auto_contrast, paper_color) = crate::effective_doc_ink_context(app);
    let is_highlighter = spec.picker_kind == PenKind::Highlighter;
    let active_pal = app.session.prefs.active_palette();
    let active_is_builtin = active_pal.is_some_and(|p| p.built_in);
    let active_id = active_pal.map_or_else(
        || papercraft_doc::BUILTIN_PALETTE_CLASSIC_ID.to_string(),
        |p| p.id.clone(),
    );
    let swatches = app.session.prefs.palette(spec.target).to_vec();
    let wildcard_color = app.session.prefs.wildcard_color(spec.target);
    let display_wildcard =
        paper_color.resolve_ink_color(theme, auto_contrast, wildcard_color, is_highlighter);

    if let Some(channels) = spec.shape_channels {
        if let Some(next_target) = draw_shape_channel_switch(
            ui,
            layout.inner_w,
            channels,
            paper_color,
            theme,
            auto_contrast,
            t,
        ) {
            app.view.shape_color_target = next_target;
            let next_preview = match next_target {
                crate::ShapeColorTarget::Border => channels.border,
                crate::ShapeColorTarget::Fill => channels.fill,
            };
            let matches_wc =
                matches!(next_preview, ChannelPreview::Color(c) if same_rgba(c, wildcard_color));
            let matches_pal = matches!(next_preview, ChannelPreview::Color(c) if swatches.iter().any(|&s| same_rgba(c, s)));
            if matches_wc && !matches_pal {
                app.view.color_dock.selected_wildcard = Some(spec.target);
            } else {
                app.view.color_dock.selected_wildcard = None;
            }
            if app.view.color_picker.is_wildcard_mode()
                && app.view.color_picker.target == spec.target
            {
                let sync_col = match next_preview {
                    ChannelPreview::Color(c) => c,
                    ChannelPreview::None | ChannelPreview::Mixed => wildcard_color,
                };
                app.view.color_picker.sync_from_color(sync_col);
            }
        }
        draw_color_dock_divider(ui, layout.inner_w, t);
    }

    let matches_wildcard =
        spec.color_is_common && spec.color.is_some_and(|c| same_rgba(c, wildcard_color));
    let matches_any_palette = spec.color_is_common
        && spec
            .color
            .is_some_and(|c| swatches.iter().any(|&s| same_rgba(c, s)));
    let zero_alpha_wildcard = spec.shape_channels.is_some()
        && spec.color_is_common
        && spec.color.is_none()
        && wildcard_color.a <= 0.0
        && app.view.color_dock.selected_wildcard == Some(spec.target);
    let wildcard_selected = zero_alpha_wildcard
        || (matches_wildcard
            && (app.view.color_dock.selected_wildcard == Some(spec.target)
                || !matches_any_palette));

    let wildcard_label = format_color_label(wildcard_color);
    let wildcard_tip = if wildcard_selected {
        format!(
            "Custom color ({wildcard_label}) — Independent of palettes\nClick or right-click to change color"
        )
    } else {
        format!(
            "Custom color ({wildcard_label}) — Independent of palettes\nClick to select · Click again or right-click to pick any color"
        )
    };
    let wildcard_resp = if let Some(channels) = spec.shape_channels {
        let no_color_selected = spec.color_is_common && spec.color.is_none() && !wildcard_selected;
        let no_color_tip = match channels.active_target {
            crate::ShapeColorTarget::Border => "No border — remove shape outline color",
            crate::ShapeColorTarget::Fill => "No fill — remove shape interior fill",
        };
        let special_row_w = COLOR_DOCK_SHAPE_SPECIAL_SIZE.x * 2.0 + COLOR_DOCK_ITEM_GAP;
        let side_pad = ((layout.inner_w - special_row_w) * 0.5).max(0.0);
        let (wc_resp, no_resp) = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(COLOR_DOCK_ITEM_GAP);
                if side_pad > 0.0 {
                    ui.add_space(side_pad);
                }
                let wc = wildcard_dot_sized(
                    ui,
                    COLOR_DOCK_SHAPE_SPECIAL_SIZE,
                    display_wildcard,
                    wildcard_selected,
                    &wildcard_tip,
                    t,
                );
                let nc = no_color_dot_sized(
                    ui,
                    COLOR_DOCK_SHAPE_SPECIAL_SIZE,
                    no_color_selected,
                    no_color_tip,
                    t,
                );
                (wc, nc)
            })
            .inner;
        if no_resp.clicked() {
            apply_palette_dock_color(app, spec.action, None);
            app.view.color_dock.selected_wildcard = None;
            if app.view.color_picker.open {
                crate::color_picker::commit_palette_name_draft(app);
                app.view.color_picker.close();
            }
        }
        wc_resp
    } else {
        wildcard_dot_sized(
            ui,
            Vec2::new(
                layout.inner_w.max(COLOR_DOCK_WILDCARD_SIZE.x),
                COLOR_DOCK_WILDCARD_SIZE.y,
            ),
            display_wildcard,
            wildcard_selected,
            &wildcard_tip,
            t,
        )
    };
    if wildcard_resp.clicked() {
        if !wildcard_selected {
            apply_palette_dock_color(app, spec.action, Some(wildcard_color));
            app.view.color_dock.selected_wildcard = Some(spec.target);
            if app.view.color_picker.open {
                match app.view.color_picker.mode {
                    ColorPopoverMode::Wildcard if app.view.color_picker.target == spec.target => {
                        app.view.color_picker.sync_from_color(wildcard_color);
                    }
                    ColorPopoverMode::EditSwatch { .. }
                    | ColorPopoverMode::PaletteEditor { .. } => {
                        crate::color_picker::commit_palette_name_draft(app);
                        app.view.color_picker.close();
                    }
                    _ => {}
                }
            }
        } else if app.view.color_picker.is_wildcard_mode()
            && app.view.color_picker.target == spec.target
        {
            app.view.color_picker.close();
        } else {
            app.view.color_picker.open_wildcard(
                spec.picker_kind,
                wildcard_color,
                wildcard_resp.rect,
            );
        }
    }
    if wildcard_resp.secondary_clicked() {
        apply_palette_dock_color(app, spec.action, Some(wildcard_color));
        app.view.color_dock.selected_wildcard = Some(spec.target);
        app.view
            .color_picker
            .open_wildcard(spec.picker_kind, wildcard_color, wildcard_resp.rect);
    }

    draw_color_dock_divider(ui, layout.inner_w, t);

    draw_color_dock_swatch_grid(ui, layout, swatches.len(), |ui, idx, cell_size| {
        let Some(&sw_color) = swatches.get(idx) else {
            return;
        };
        let selected = !wildcard_selected
            && spec.color_is_common
            && spec.color.is_some_and(|c| same_rgba(c, sw_color));
        let display_sw =
            paper_color.resolve_ink_color(theme, auto_contrast, sw_color, is_highlighter);
        let right_action = if active_is_builtin {
            "Right-click to manage or duplicate palette"
        } else {
            "Right-click to edit or remove"
        };
        let sw_label = format_color_label(sw_color);
        let disp_label = format_color_label(display_sw);
        let tip = if spec.include_contrast_tip && auto_contrast && !same_rgb(display_sw, sw_color) {
            format!(
                "{sw_label} (shown as {disp_label} in {} theme) — Click to select · {right_action}",
                theme.label()
            )
        } else if spec.include_contrast_tip {
            format!("{sw_label} — Click to select · {right_action}")
        } else {
            format!("{sw_label} — Click to apply · {right_action}")
        };
        let resp = color_dot_sized(ui, cell_size, display_sw, selected, &tip, t);
        if resp.clicked() {
            apply_palette_dock_color(app, spec.action, Some(sw_color));
            app.view.color_dock.selected_wildcard = None;
            if app.view.color_picker.open && app.view.color_picker.target == spec.target {
                match &mut app.view.color_picker.mode {
                    ColorPopoverMode::Wildcard => {
                        app.view.color_picker.close();
                    }
                    ColorPopoverMode::EditSwatch {
                        preset_id,
                        slot_index,
                    } if *preset_id == active_id && !active_is_builtin => {
                        *slot_index = idx;
                        app.view.color_picker.sync_from_color(sw_color);
                    }
                    ColorPopoverMode::PaletteEditor {
                        preset_id,
                        selected_slot,
                        confirm_delete,
                    } if *preset_id == active_id && !active_is_builtin => {
                        *selected_slot = idx;
                        *confirm_delete = false;
                        app.view.color_picker.sync_from_color(sw_color);
                    }
                    _ => {}
                }
            }
        }
        if resp.secondary_clicked() {
            if !active_is_builtin {
                apply_palette_dock_color(app, spec.action, Some(sw_color));
                app.view.color_dock.selected_wildcard = None;
                app.view.color_picker.open_edit_swatch(
                    spec.target,
                    active_id.clone(),
                    idx,
                    sw_color,
                    resp.rect,
                );
            } else {
                app.view.color_picker.open_palettes(
                    spec.picker_kind,
                    spec.color.unwrap_or(wildcard_color),
                    resp.rect,
                );
            }
        }
    });

    ui.add_space(2.0);
    let palettes_open =
        app.view.color_picker.is_palette_mode() && app.view.color_picker.target == spec.target;
    let picker_resp = sized_icon_button(
        ui,
        Icon::Palette,
        SquareButtonSpec {
            size: COLOR_DOCK_PALETTE_BTN_SIZE,
            icon_dim: COLOR_DOCK_PALETTE_ICON_SIZE,
            corner_radius: COLOR_DOCK_INNER_CORNER_RADIUS,
        },
        palettes_open,
        true,
        t,
    )
    .on_hover_text(spec.picker_tooltip);
    if picker_resp.clicked() {
        if palettes_open {
            crate::color_picker::commit_palette_name_draft(app);
            app.view.color_picker.close();
        } else {
            app.view.color_picker.open_palettes(
                spec.picker_kind,
                spec.color.unwrap_or(wildcard_color),
                picker_resp.rect,
            );
        }
    }
}

const LASER_COLOR_COUNT: usize = 4;

fn laser_colors() -> [(&'static str, Color); LASER_COLOR_COUNT] {
    [
        ("Red laser", Color::from_rgba8(0xEF, 0x44, 0x44, 255)),
        ("Green laser", Color::from_rgba8(0x22, 0xC5, 0x5E, 255)),
        ("Cyan laser", Color::from_rgba8(0x38, 0xBD, 0xF8, 255)),
        ("Amber laser", Color::from_rgba8(0xFA, 0xCC, 0x15, 255)),
    ]
}

fn draw_laser_dock_controls(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    layout: &ColorDockLayout,
    t: &Tokens,
) {
    let colors = laser_colors();
    let current_color = app.view.laser_color;
    draw_color_dock_swatch_grid(ui, layout, colors.len(), |ui, idx, cell_size| {
        let (tip, color) = colors[idx];
        let selected = same_rgb(current_color, color);
        if color_dot_sized(ui, cell_size, color, selected, tip, t).clicked() {
            app.view.laser_color = color;
        }
    });
}

fn push_selected_stroke_controls(
    app: &PaperCraftApp,
    ui: &egui::Ui,
    flow: &mut DockFlow<'_>,
    t: &Tokens,
    id: papercraft_doc::StrokeId,
) -> bool {
    let Some(stroke) = app
        .session
        .doc
        .strokes
        .iter()
        .find(|stroke| stroke.id == id)
        .cloned()
    else {
        return false;
    };

    let short_label = stroke.pen.kind.short_label();
    let label_w = crate::theme::studio_row_label_width(
        ui,
        egui::RichText::new(short_label)
            .size(11.5)
            .color(t.text_muted),
    );
    flow.push(label_w, move |_app, ui, t| {
        crate::theme::studio_row_label(
            ui,
            egui::RichText::new(short_label)
                .size(11.5)
                .color(t.text_muted),
            CONTEXT_TRAY_ROW_HEIGHT,
        );
    });
    let (presets, min_size, max_size) = pen_path_size_range(stroke.pen.kind);
    let (eff_contrast, eff_paper) = crate::effective_doc_ink_context(app);
    let display_color = eff_paper.resolve_ink_color(
        app.session.prefs.theme,
        eff_contrast,
        stroke.pen.color,
        stroke.pen.kind == PenKind::Highlighter,
    );
    let cur_size = stroke.pen.size;
    push_path_size_controls(
        flow,
        ("selected_stroke_size_slider", id.0),
        PathSizeSpec {
            presets,
            min: min_size,
            max: max_size,
            current: cur_size,
            display_color,
            preset_tooltip_prefix: "Set selected stroke thickness to ",
            slider_tooltip_prefix: "Adjust selected stroke thickness",
        },
        move |app, next_size, record_history, _persist_size| {
            if !record_history && (next_size - cur_size).abs() < 0.01 {
                return;
            }
            let _ = app.session.execute(
                "stroke.update",
                &json!({ "id": id.0, "size": next_size, "record_history": record_history }),
            );
        },
    );
    true
}

fn push_selected_shape_controls(
    app: &PaperCraftApp,
    ui: &egui::Ui,
    flow: &mut DockFlow<'_>,
    t: &Tokens,
    index: usize,
) -> bool {
    let Some(shape) = app
        .session
        .doc
        .shapes
        .get(index)
        .map(|shape| shape.as_ref().sanitized())
    else {
        return false;
    };
    push_shape_kind_controls(
        flow,
        ui,
        ("toolbar_selected_shape_kind_switch", index),
        Some(shape.kind),
        " (change selected shape)",
        move |app, kind| {
            let _ = app
                .session
                .execute("shape.update", &json!({ "index": index, "kind": kind }));
        },
    );

    let initial_angle = normalized_degrees(shape.angle_degrees);
    flow.push(RULER_ANGLE_CONTROL_WIDTH, move |app, ui, _t| {
        let mut angle_degrees = initial_angle;
        let angle = ui.add_sized(
            Vec2::new(RULER_ANGLE_CONTROL_WIDTH, CONTEXT_TRAY_ROW_HEIGHT),
            egui::DragValue::new(&mut angle_degrees)
                .range(-180.0..=180.0)
                .fixed_decimals(0)
                .suffix("°")
                .speed(1.0),
        );
        if angle.changed() {
            let _ = app.session.execute(
                "shape.update",
                &json!({ "index": index, "angle_degrees": angle_degrees }),
            );
        }
        angle.on_hover_text("Shape angle — double-click to type a value or drag to adjust");
    });

    let curve_enabled = matches!(
        shape.kind,
        ShapeKind::Rectangle | ShapeKind::Triangle | ShapeKind::Polygon
    );
    let initial_curve = shape.curve;
    flow.push(SHAPE_CURVE_CONTROL_WIDTH, move |app, ui, _t| {
        let mut curve_value = initial_curve;
        let curve = ui.add_enabled_ui(curve_enabled, |ui| {
            ui.add_sized(
                Vec2::new(SHAPE_CURVE_CONTROL_WIDTH, CONTEXT_TRAY_ROW_HEIGHT),
                egui::DragValue::new(&mut curve_value)
                    .range(0.0..=128.0)
                    .fixed_decimals(0)
                    .prefix("Curve ")
                    .suffix(" pt")
                    .speed(1.0),
            )
        });
        if curve.inner.changed() {
            let _ = app.session.execute(
                "shape.update",
                &json!({ "index": index, "curve": curve_value }),
            );
        }
        curve.inner.on_hover_text(
            "Curve — rounds rectangle, triangle, and polygon corners. Set 0 for sharp corners.",
        );
    });

    let (presets, min_width, max_width) = pen_path_size_range(PenKind::FountainPen);
    let shape_color = shape.color;
    let shape_width = shape.width;
    let (eff_contrast, eff_paper) = crate::effective_doc_ink_context(app);
    let display_color = if shape_color.a > 0.0 {
        eff_paper.resolve_ink_color(app.session.prefs.theme, eff_contrast, shape_color, false)
    } else {
        Color::from_rgba8(t.text_muted.r(), t.text_muted.g(), t.text_muted.b(), 255)
    };
    push_path_size_controls(
        flow,
        ("selected_shape_size_slider", index),
        PathSizeSpec {
            presets,
            min: min_width,
            max: max_width,
            current: shape_width,
            display_color,
            preset_tooltip_prefix: "Set selected shape outline to ",
            slider_tooltip_prefix: "Adjust selected shape outline",
        },
        move |app, next_width, record_history, _persist_size| {
            if !record_history && (next_width - shape_width).abs() < 0.01 {
                return;
            }
            let _ = app.session.execute(
                "shape.update",
                &json!({ "index": index, "width": next_width, "record_history": record_history }),
            );
        },
    );
    flow.divider();
    push_shape_line_style_controls(flow, Some(shape.line_style), move |app, style| {
        let _ = app.session.execute(
            "shape.update",
            &json!({ "index": index, "line_style": style }),
        );
    });
    true
}

fn push_selected_text_controls(
    app: &PaperCraftApp,
    ui: &egui::Ui,
    flow: &mut DockFlow<'_>,
    t: &Tokens,
    index: usize,
) -> bool {
    let Some(item) = app
        .session
        .doc
        .texts
        .get(index)
        .map(|item| item.as_ref().sanitized())
    else {
        return false;
    };

    let text_label_w = crate::theme::studio_row_label_width(
        ui,
        egui::RichText::new("Text").size(11.5).color(t.text_muted),
    );
    let full_len = item.text.len();
    let effective_font = item.effective_font_across(0, full_len);
    let font_label = effective_font.map_or("Mixed", TextFont::label);
    let combo_w = crate::theme::studio_combo_box_width(ui, font_label, 68.0);
    flow.push(
        text_label_w + ZONE_ITEM_SPACING + combo_w,
        move |app, ui, t| {
            crate::theme::studio_row_label(
                ui,
                egui::RichText::new("Text").size(11.5).color(t.text_muted),
                CONTEXT_TRAY_ROW_HEIGHT,
            );
            let mut chosen_font: Option<TextFont> = None;
            let font_response = crate::theme::studio_combo_box(
                ui,
                ("selected_text_toolbar_font", index),
                font_label,
                68.0,
                |ui| {
                    for option in TextFont::ALL {
                        if ui
                            .selectable_label(effective_font == Some(*option), option.label())
                            .clicked()
                        {
                            chosen_font = Some(*option);
                        }
                    }
                },
            );
            font_response
                .response
                .on_hover_text("Font — choose how the letters look");
            if let Some(next_font) = chosen_font {
                let _ = app
                    .session
                    .execute("text.update", &json!({ "index": index, "font": next_font }));
            }
        },
    );

    let initial_size = item.effective_size_across(0, full_len).unwrap_or(item.size);
    flow.push(68.0, move |app, ui, _t| {
        let mut size = initial_size;
        let size_response = ui.add_sized(
            Vec2::new(68.0, CONTEXT_TRAY_ROW_HEIGHT),
            egui::DragValue::new(&mut size)
                .range(6.0..=120.0)
                .speed(1.0)
                .suffix(" px"),
        );
        if size_response.changed() {
            let _ = app
                .session
                .execute("text.update", &json!({ "index": index, "size": size }));
        }
        size_response.on_hover_text("Text size — larger text stands out more on the page");
    });

    let initial_line_spacing = item.line_spacing;
    flow.push(82.0, move |app, ui, _t| {
        let mut line_spacing = initial_line_spacing;
        let line_resp = ui.add_sized(
            Vec2::new(82.0, CONTEXT_TRAY_ROW_HEIGHT),
            egui::DragValue::new(&mut line_spacing)
                .range(0.8..=3.0)
                .speed(0.05)
                .fixed_decimals(2)
                .prefix("Line ")
                .suffix("×"),
        );
        if line_resp.changed() {
            let _ = app.session.execute(
                "text.update",
                &json!({ "index": index, "line_spacing": line_spacing }),
            );
        }
        line_resp.on_hover_text("Line spacing — adjust vertical distance between lines");
    });

    let initial_letter_spacing = item.letter_spacing;
    flow.push(92.0, move |app, ui, _t| {
        let mut letter_spacing = initial_letter_spacing;
        let letter_resp = ui.add_sized(
            Vec2::new(92.0, CONTEXT_TRAY_ROW_HEIGHT),
            egui::DragValue::new(&mut letter_spacing)
                .range(-4.0..=32.0)
                .speed(0.2)
                .fixed_decimals(1)
                .prefix("Letter ")
                .suffix(" px"),
        );
        if letter_resp.changed() {
            let _ = app.session.execute(
                "text.update",
                &json!({ "index": index, "letter_spacing": letter_spacing }),
            );
        }
        letter_resp.on_hover_text("Letter spacing — adjust horizontal space between characters");
    });

    let full_len = item.text.len();
    let (bold_on, italic_on, underline_on, hl_active) = item.style_active_across(0, full_len);
    let has_bullets = item
        .text
        .lines()
        .any(|l| parse_list_line_prefix(l).0 == Some(ListItemKind::Bullet));
    let has_checks = item.text.lines().any(|l| {
        matches!(
            parse_list_line_prefix(l).0,
            Some(ListItemKind::Checkbox { .. })
        )
    });
    let hl_yellow = Color::from_rgba8(0xFE, 0xF0, 0x8A, 0xCC);
    let item_rc = std::rc::Rc::new(item);
    let item_for_rich = std::rc::Rc::clone(&item_rc);
    let hl_is_active = hl_active.is_some();
    push_rich_text_style_bar(
        flow,
        ui,
        ("selected_text_align", index),
        item_rc.alignment,
        bold_on,
        italic_on,
        underline_on,
        hl_is_active,
        has_bullets,
        has_checks,
        move |app, action| {
            let mut updated = (*item_for_rich).clone();
            let mut update_payload = serde_json::Map::new();
            match action {
                RichTextAction::ToggleBold => {
                    updated.toggle_span_style(0, full_len, TextStyleAttr::Bold);
                    update_payload.insert("spans".into(), json!(updated.spans));
                }
                RichTextAction::ToggleItalic => {
                    updated.toggle_span_style(0, full_len, TextStyleAttr::Italic);
                    update_payload.insert("spans".into(), json!(updated.spans));
                }
                RichTextAction::ToggleUnderline => {
                    updated.toggle_span_style(0, full_len, TextStyleAttr::Underline);
                    update_payload.insert("spans".into(), json!(updated.spans));
                }
                RichTextAction::ToggleHighlight => {
                    updated.set_span_highlight(0, full_len, (!hl_is_active).then_some(hl_yellow));
                    update_payload.insert("spans".into(), json!(updated.spans));
                }
                RichTextAction::SetAlignment(align) => {
                    update_payload.insert("alignment".into(), json!(align));
                }
                RichTextAction::ToggleList(want_checkbox) => {
                    let _ = updated.toggle_list_in_range(0, full_len, want_checkbox);
                    update_payload.insert("text".into(), json!(updated.text));
                    update_payload.insert("spans".into(), json!(updated.spans));
                }
            }
            update_payload.insert("index".into(), json!(index));
            let _ = app
                .session
                .execute("text.update", &serde_json::Value::Object(update_payload));
        },
    );

    flow.divider();
    let item_for_edit = std::rc::Rc::clone(&item_rc);
    flow.push(
        tray_icon_button_width(ui, "Edit text"),
        move |app, ui, t| {
            if tray_icon_button(
                ui,
                crate::icons::Icon::Edit,
                Some("Edit text"),
                false,
                true,
                t,
            )
            .on_hover_text("Edit this text directly on the page")
            .clicked()
            {
                crate::canvas::start_editing_text_item(app, index, &item_for_edit);
            }
        },
    );

    let existing_link = item_rc.primary_link_url();
    let link_active = app.view.text_link_editor_open || existing_link.is_some();
    let existing_for_link = existing_link.clone();
    flow.push(tray_icon_button_width(ui, "Link"), move |app, ui, t| {
        if tray_icon_button(ui, Icon::Link, Some("Link"), link_active, true, t)
            .on_hover_text("Attach or edit a clickable link URL for this text")
            .clicked()
        {
            app.view.text_link_editor_open = !app.view.text_link_editor_open;
            if app.view.text_link_editor_open {
                app.view.text_link_input = existing_for_link.unwrap_or_default();
            }
        }
    });

    if let Some(url) = existing_link {
        flow.push(tray_icon_button_width(ui, "Open"), move |app, ui, t| {
            if tray_icon_button(ui, Icon::ArrowShape, Some("Open"), false, true, t)
                .on_hover_text(format!("Open link: {url}"))
                .clicked()
            {
                let _ = crate::open_external_url(app, ui.ctx(), &url);
            }
        });
    }

    if app.view.text_link_editor_open {
        push_inline_link_editor_bar(app, ui, flow, move |app, action| match action {
            LinkEditorAction::Apply(url) => {
                let _ = app
                    .session
                    .execute("text.update", &json!({ "index": index, "link_url": url }));
                app.view.text_link_editor_open = false;
            }
            LinkEditorAction::Clear => {
                let _ = app.session.execute(
                    "text.update",
                    &json!({ "index": index, "clear_link_url": true }),
                );
                app.view.text_link_input.clear();
                app.view.text_link_editor_open = false;
            }
        });
    }
    true
}

#[derive(Clone, Copy)]
enum RichTextAction {
    ToggleBold,
    ToggleItalic,
    ToggleUnderline,
    ToggleHighlight,
    SetAlignment(TextAlignment),
    ToggleList(bool),
}

#[allow(clippy::too_many_arguments)]
fn push_rich_text_style_bar<'a>(
    flow: &mut DockFlow<'a>,
    ui: &egui::Ui,
    align_id_salt: impl egui::AsIdSalt + 'a,
    alignment: TextAlignment,
    bold_active: bool,
    italic_active: bool,
    underline_active: bool,
    highlight_active: bool,
    bullet_active: bool,
    checkbox_active: bool,
    on_action: impl Fn(&mut PaperCraftApp, RichTextAction) + Clone + 'a,
) {
    flow.divider();

    let style_buttons = [
        (
            Icon::Bold,
            bold_active,
            "Bold — highlight text or apply to the text box",
            RichTextAction::ToggleBold,
        ),
        (
            Icon::Italic,
            italic_active,
            "Italic — highlight text or apply to the text box",
            RichTextAction::ToggleItalic,
        ),
        (
            Icon::Underline,
            underline_active,
            "Underline — highlight text or apply to the text box",
            RichTextAction::ToggleUnderline,
        ),
        (
            Icon::Highlighter,
            highlight_active,
            "Highlight — tint selected text with a soft marker background",
            RichTextAction::ToggleHighlight,
        ),
    ];
    for (idx, (icon, active, tip, action)) in style_buttons.into_iter().enumerate() {
        let cb = on_action.clone();
        let draw_fn = move |app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens| {
            if tray_icon_button(ui, icon, None, active, true, t)
                .on_hover_text(tip)
                .clicked()
            {
                cb(app, action);
            }
        };
        if idx == 0 {
            flow.push(CONTEXT_TRAY_ROW_HEIGHT, draw_fn);
        } else {
            flow.push_cohesive(CONTEXT_TRAY_ROW_HEIGHT, draw_fn);
        }
    }

    flow.divider();
    let align_opts = [
        SwitchOption::new(Some(Icon::AlignLeft), None, "Align left", true),
        SwitchOption::new(Some(Icon::AlignCenter), None, "Align center", true),
        SwitchOption::new(Some(Icon::AlignRight), None, "Align right", true),
    ];
    let align_w = segmented_switch_width(ui, &align_opts, SwitchLayout::TrayCompact);
    let align_idx = match alignment {
        TextAlignment::Left => 0,
        TextAlignment::Center => 1,
        TextAlignment::Right => 2,
    };
    let cb_align = on_action.clone();
    flow.push(align_w, move |app, ui, t| {
        let opts = [
            SwitchOption::new(Some(Icon::AlignLeft), None, "Align left", true),
            SwitchOption::new(Some(Icon::AlignCenter), None, "Align center", true),
            SwitchOption::new(Some(Icon::AlignRight), None, "Align right", true),
        ];
        let (clicked_align, _) = segmented_switch(
            ui,
            align_id_salt,
            &opts,
            Some(align_idx),
            SwitchLayout::TrayCompact,
            t,
        );
        if let Some(idx) = clicked_align {
            let next = match idx {
                0 => TextAlignment::Left,
                1 => TextAlignment::Center,
                _ => TextAlignment::Right,
            };
            cb_align(app, RichTextAction::SetAlignment(next));
        }
    });

    let cb_bullet = on_action.clone();
    flow.push(CONTEXT_TRAY_ROW_HEIGHT, move |app, ui, t| {
        if tray_icon_button(ui, Icon::ListView, None, bullet_active, true, t)
            .on_hover_text("Bullet list — toggle bullet prefixes on selected or all lines")
            .clicked()
        {
            cb_bullet(app, RichTextAction::ToggleList(false));
        }
    });
    flow.push_cohesive(CONTEXT_TRAY_ROW_HEIGHT, move |app, ui, t| {
        if tray_icon_button(ui, Icon::CheckList, None, checkbox_active, true, t)
            .on_hover_text("Checklist — toggle clickable [ ] / [x] tasks on lines")
            .clicked()
        {
            on_action(app, RichTextAction::ToggleList(true));
        }
    });
}

enum LinkEditorAction {
    Apply(String),
    Clear,
}

fn inline_link_editor_widths(
    ui: &egui::Ui,
    text_link_input: &str,
    max_content_w: f32,
) -> (Option<String>, f32, f32) {
    let normalized = sanitize_link_url(text_link_input);
    let apply_w = tray_icon_button_width(ui, "Apply");
    let extra_btns = if normalized.is_some() { 2 } else { 1 };
    let chrome_w =
        apply_w + (extra_btns as f32) * CONTEXT_TRAY_ROW_HEIGHT + (extra_btns as f32 + 1.0) * 4.0;
    let full_bar_w = 190.0 + chrome_w;
    let alloc_w = full_bar_w.min(max_content_w.max(120.0));
    let input_w = (alloc_w - chrome_w).clamp(64.0, 190.0);
    let bar_w = input_w + chrome_w;
    (normalized, input_w, bar_w)
}

fn push_inline_link_editor_bar<'a>(
    app: &PaperCraftApp,
    ui: &egui::Ui,
    flow: &mut DockFlow<'a>,
    on_action: impl FnOnce(&mut PaperCraftApp, LinkEditorAction) + 'a,
) {
    flow.divider();
    let (_, _, bar_w) =
        inline_link_editor_widths(ui, &app.view.text_link_input, flow.max_content_w);
    let max_content_w = flow.max_content_w;
    flow.push(bar_w, move |app, ui, t| {
        if let Some(action) = draw_inline_link_editor_bar(app, ui, t, max_content_w) {
            on_action(app, action);
        }
    });
}

fn draw_inline_link_editor_bar(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
    max_content_w: f32,
) -> Option<LinkEditorAction> {
    let mut action = None;
    let (normalized, input_w, bar_w) =
        inline_link_editor_widths(ui, &app.view.text_link_input, max_content_w);
    let alloc_w = bar_w.min(ui.max_rect().width().max(120.0));
    ui.allocate_ui_with_layout(
        Vec2::new(alloc_w, CONTEXT_TRAY_ROW_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().interact_size.y = CONTEXT_TRAY_ROW_HEIGHT;
            ui.spacing_mut().item_spacing.x = 4.0;
            let edit_resp = ui.add_sized(
                Vec2::new(input_w, CONTEXT_TRAY_ROW_HEIGHT),
                egui::TextEdit::singleline(&mut app.view.text_link_input)
                    .id_salt("text_dock_link_url_input")
                    .margin(egui::Margin::symmetric(8, 2))
                    .align(egui::Align2::LEFT_CENTER)
                    .hint_text("https://example.com"),
            );
            let enter_apply =
                edit_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let apply_clicked = tray_icon_button(
                ui,
                Icon::Check,
                Some("Apply"),
                false,
                normalized.is_some(),
                t,
            )
            .on_hover_text("Apply link URL (Enter)")
            .clicked();
            if (apply_clicked || enter_apply)
                && let Some(url) = normalized.clone()
            {
                action = Some(LinkEditorAction::Apply(url));
            }
            if let Some(url) = normalized
                && tray_icon_button(ui, Icon::ArrowShape, None, false, true, t)
                    .on_hover_text(format!("Test link: {url}"))
                    .clicked()
            {
                let _ = crate::open_external_url(app, ui.ctx(), &url);
            }
            if tray_icon_button(ui, Icon::Close, None, false, true, t)
                .on_hover_text("Remove link / close link editor")
                .clicked()
            {
                action = Some(LinkEditorAction::Clear);
            }
        },
    );
    action
}

fn push_selected_image_controls(
    app: &PaperCraftApp,
    ui: &egui::Ui,
    flow: &mut DockFlow<'_>,
    t: &Tokens,
    index: usize,
) -> bool {
    let Some(image) = app.session.doc.images.get(index) else {
        return false;
    };
    let label_w = crate::theme::studio_row_label_width(
        ui,
        egui::RichText::new("Image").size(11.5).color(t.text_muted),
    );
    let initial_angle = image.angle_degrees;
    flow.push(label_w + ZONE_ITEM_SPACING + 92.0, move |app, ui, t| {
        crate::theme::studio_row_label(
            ui,
            egui::RichText::new("Image").size(11.5).color(t.text_muted),
            CONTEXT_TRAY_ROW_HEIGHT,
        );
        let mut angle = initial_angle;
        let angle_resp = ui.add_sized(
            Vec2::new(92.0, CONTEXT_TRAY_ROW_HEIGHT),
            egui::DragValue::new(&mut angle)
                .range(-180.0..=180.0)
                .speed(1.0)
                .fixed_decimals(0)
                .prefix("Angle ")
                .suffix("°"),
        );
        if angle_resp.changed() {
            let _ = app.session.execute(
                "image.update",
                &json!({ "index": index, "angle_degrees": angle }),
            );
        }
        angle_resp.on_hover_text("Rotate selected image around its center");
    });
    true
}

fn push_selection_controls<'a>(
    app: &PaperCraftApp,
    ui: &egui::Ui,
    flow: &mut DockFlow<'a>,
    t: &Tokens,
    selected: &'a [crate::SelectedObject],
) -> bool {
    let shown = if selected.len() > 1 {
        push_multi_selection_controls(app, ui, flow, t, selected);
        true
    } else {
        match selected.first().copied() {
            Some(crate::SelectedObject::Stroke(id)) => {
                push_selected_stroke_controls(app, ui, flow, t, id)
            }
            Some(crate::SelectedObject::Shape(index)) => {
                push_selected_shape_controls(app, ui, flow, t, index)
            }
            Some(crate::SelectedObject::Text(index)) => {
                push_selected_text_controls(app, ui, flow, t, index)
            }
            Some(crate::SelectedObject::Image(index)) => {
                push_selected_image_controls(app, ui, flow, t, index)
            }
            None => false,
        }
    };
    if shown {
        flow.divider();
        push_select_mode_switch(app, ui, flow, false, "toolbar_selection_mode_switch");
    }
    shown
}

fn selected_object_color(app: &PaperCraftApp, object: crate::SelectedObject) -> Option<Color> {
    match object {
        crate::SelectedObject::Stroke(id) => app
            .session
            .doc
            .strokes
            .iter()
            .find(|stroke| stroke.id == id)
            .map(|stroke| stroke.pen.color),
        crate::SelectedObject::Shape(index) => {
            app.session.doc.shapes.get(index).and_then(|shape| {
                if shape.color.a > 0.0 {
                    Some(shape.color)
                } else {
                    shape.fill_color.filter(|c| c.a > 0.0)
                }
            })
        }
        crate::SelectedObject::Text(index) => {
            app.session.doc.texts.get(index).map(|item| item.color)
        }
        crate::SelectedObject::Image(_) => None,
    }
}

fn push_multi_selection_controls<'a>(
    app: &PaperCraftApp,
    ui: &egui::Ui,
    flow: &mut DockFlow<'a>,
    t: &Tokens,
    selected: &'a [crate::SelectedObject],
) {
    if selected.is_empty() {
        return;
    }

    let all_shapes = selected
        .iter()
        .all(|object| matches!(object, crate::SelectedObject::Shape(_)));
    if all_shapes {
        let mut common_kind = None;
        let mut kinds_match = true;
        for object in selected {
            let crate::SelectedObject::Shape(index) = object else {
                kinds_match = false;
                continue;
            };
            let Some(shape) = app.session.doc.shapes.get(*index) else {
                kinds_match = false;
                continue;
            };
            match common_kind {
                None => common_kind = Some(shape.kind),
                Some(kind) if kind == shape.kind => {}
                Some(_) => kinds_match = false,
            }
        }
        let active_kind = if kinds_match { common_kind } else { None };
        let label_w = crate::theme::studio_row_label_width(
            ui,
            egui::RichText::new("Format").size(11.5).color(t.text_muted),
        );
        flow.push(label_w, |_app, ui, t| {
            crate::theme::studio_row_label(
                ui,
                egui::RichText::new("Format").size(11.5).color(t.text_muted),
                CONTEXT_TRAY_ROW_HEIGHT,
            );
        });
        push_shape_kind_controls(
            flow,
            ui,
            "toolbar_multi_shape_kind_switch",
            active_kind,
            " (convert all selected shapes)",
            move |app, kind| {
                let targets = selected_targets_json(selected);
                let _ = app.session.execute(
                    "selection.update",
                    &json!({ "targets": targets, "shape_kind": kind }),
                );
            },
        );
        flow.divider();
        let common_line_style = {
            let mut first = None;
            let mut all_match = true;
            for object in selected {
                let crate::SelectedObject::Shape(idx) = object else {
                    all_match = false;
                    continue;
                };
                let Some(s) = app.session.doc.shapes.get(*idx) else {
                    all_match = false;
                    continue;
                };
                match first {
                    None => first = Some(s.line_style),
                    Some(ls) if ls == s.line_style => {}
                    Some(_) => all_match = false,
                }
            }
            if all_match { first } else { None }
        };
        push_shape_line_style_controls(flow, common_line_style, move |app, style| {
            let targets = selected_targets_json(selected);
            let _ = app.session.execute(
                "selection.update",
                &json!({ "targets": targets, "line_style": style }),
            );
        });
    } else {
        let label = format!("{} selected", selected.len());
        let label_w = crate::theme::studio_row_label_width(
            ui,
            egui::RichText::new(&label).size(11.5).color(t.text_muted),
        );
        flow.push(label_w, move |_app, ui, t| {
            crate::theme::studio_row_label(
                ui,
                egui::RichText::new(label).size(11.5).color(t.text_muted),
                CONTEXT_TRAY_ROW_HEIGHT,
            );
        });
    }
}

fn push_eraser_quick_bar(app: &PaperCraftApp, ui: &egui::Ui, flow: &mut DockFlow<'_>) {
    let is_stroke = app.session.eraser_mode == EraserMode::StrokeEraser;
    let stroke_desc = EraserMode::StrokeEraser.description();
    let precision_desc = EraserMode::PrecisionEraser.description();
    let options = [
        SwitchOption::new(Some(Icon::StrokeEraser), Some("Stroke"), stroke_desc, true),
        SwitchOption::new(
            Some(Icon::PrecisionEraser),
            Some("Precision"),
            precision_desc,
            true,
        ),
    ];
    let switch_w = segmented_switch_width(ui, &options, SwitchLayout::TrayCompact);
    flow.push(switch_w, move |app, ui, t| {
        let opts = [
            SwitchOption::new(Some(Icon::StrokeEraser), Some("Stroke"), stroke_desc, true),
            SwitchOption::new(
                Some(Icon::PrecisionEraser),
                Some("Precision"),
                precision_desc,
                true,
            ),
        ];
        let (clicked, _) = segmented_switch(
            ui,
            "toolbar_eraser_mode_switch",
            &opts,
            Some(usize::from(!is_stroke)),
            SwitchLayout::TrayCompact,
            t,
        );
        if let Some(idx) = clicked {
            let mode = if idx == 0 {
                "StrokeEraser"
            } else {
                "PrecisionEraser"
            };
            let _ = app
                .session
                .execute("eraser.setSettings", &json!({ "mode": mode }));
        }
    });

    let eraser_sizes: [(&str, f32, f32); 3] = [
        ("Small (12 px)", 12.0, 3.5),
        ("Medium (28 px)", 28.0, 6.1),
        ("Large (56 px)", 56.0, 9.0),
    ];
    for (idx, (label, preset_size, icon_r)) in eraser_sizes.into_iter().enumerate() {
        let selected = (app.session.eraser_size - preset_size).abs() < 4.0;
        let draw_fn = move |app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens| {
            let (rect, resp, select_t) = draw_size_tile_bg(ui, t, selected);
            let ring_c = crate::motion::lerp_color(t.text_muted, t.accent, select_t);
            ui.painter()
                .circle(rect.center(), icon_r, t.field_bg, Stroke::new(1.7, ring_c));
            if resp
                .on_hover_text(format!("Set eraser diameter to {label}"))
                .clicked()
            {
                let _ = app
                    .session
                    .execute("eraser.setSettings", &json!({ "size": preset_size }));
            }
        };
        if idx == 0 {
            flow.push(CONTEXT_TRAY_TILE_SIZE, draw_fn);
        } else {
            flow.push_cohesive(CONTEXT_TRAY_TILE_SIZE, draw_fn);
        }
    }
    flow.divider();
    let hl_only = app.session.eraser_highlighter_only;
    flow.push(
        tray_icon_button_width(ui, "Highlighter only"),
        move |app, ui, t| {
            if tray_icon_button(
                ui,
                Icon::Highlighter,
                Some("Highlighter only"),
                hl_only,
                true,
                t,
            )
            .on_hover_text(
                "Highlighter only — erases highlighter strokes while leaving pen and pencil ink untouched",
            )
            .clicked()
            {
                let _ = app.session.execute(
                    "eraser.setSettings",
                    &json!({ "highlighter_only": !app.session.eraser_highlighter_only }),
                );
            }
        },
    );
}

fn push_pen_quick_bar(
    app: &PaperCraftApp,
    ui: &egui::Ui,
    flow: &mut DockFlow<'_>,
    kind: PenKind,
    show_highlighter_modes: bool,
) {
    let pen = app.session.pen(kind).clone();

    if kind == PenKind::Highlighter && show_highlighter_modes {
        let is_freehand = app.session.highlighter_mode == HighlighterMode::Freehand;
        let freehand_tip = format!(
            "{}\n{}",
            HighlighterMode::Freehand.label(),
            HighlighterMode::Freehand.description()
        );
        let straight_tip = format!(
            "{}\n{}",
            HighlighterMode::Straight.label(),
            HighlighterMode::Straight.description()
        );
        let options = [
            SwitchOption::new(Some(Icon::HighlighterFreehand), None, &freehand_tip, true),
            SwitchOption::new(Some(Icon::HighlighterStraight), None, &straight_tip, true),
        ];
        let switch_w = segmented_switch_width(ui, &options, SwitchLayout::TrayCompact);
        flow.push(switch_w, move |app, ui, t| {
            let opts = [
                SwitchOption::new(Some(Icon::HighlighterFreehand), None, &freehand_tip, true),
                SwitchOption::new(Some(Icon::HighlighterStraight), None, &straight_tip, true),
            ];
            let (clicked, _) = segmented_switch(
                ui,
                "toolbar_highlighter_mode_switch",
                &opts,
                Some(usize::from(!is_freehand)),
                SwitchLayout::TrayCompact,
                t,
            );
            if let Some(idx) = clicked {
                let mode = if idx == 0 { "Freehand" } else { "Straight" };
                let _ = app
                    .session
                    .execute("highlighter.setSettings", &json!({ "mode": mode }));
            }
        });
        flow.divider();
    }

    let (presets, min_size, max_size) = pen_path_size_range(kind);
    let (eff_contrast, eff_paper) = crate::effective_doc_ink_context(app);
    let disp_color = eff_paper.resolve_ink_color(
        app.session.prefs.theme,
        eff_contrast,
        pen.color,
        matches!(pen.kind, papercraft_doc::PenKind::Highlighter),
    );
    let cur_size = pen.size;
    push_path_size_controls(
        flow,
        ("pen_size_slider", kind),
        PathSizeSpec {
            presets,
            min: min_size,
            max: max_size,
            current: cur_size,
            display_color: disp_color,
            preset_tooltip_prefix: "Set stroke thickness to ",
            slider_tooltip_prefix: "Adjust stroke thickness",
        },
        move |app, next_size, _record_history, persist_size| {
            if !persist_size && (next_size - cur_size).abs() < 0.01 {
                return;
            }
            let _ = app.session.execute(
                "pen.setSettings",
                &json!({ "kind": kind, "size": next_size, "persist_size": persist_size }),
            );
        },
    );

    if show_highlighter_modes {
        let eff_strength = pen.smoothing.effective_strength();
        let sm_label = smoothing_value_label(eff_strength);
        let sm_tip = format!(
            "Stroke smoothing ({}) — {}\nClick to adjust or disable for faster sharp turns.",
            sm_label,
            papercraft_doc::SmoothingConfig::strength_guidance(eff_strength)
        );
        let btn_w = tray_icon_button_width(ui, &sm_label);
        let is_open = app.view.smoothing_popover_open;
        flow.divider();
        flow.push(btn_w, move |app, ui, t| {
            let resp = tray_icon_button(ui, Icon::Stabilizer, Some(&sm_label), is_open, true, t)
                .on_hover_text(sm_tip);
            if app.view.smoothing_popover_open {
                app.view.smoothing_popover_anchor = Some(resp.rect);
            }
            if resp.clicked() {
                app.view.smoothing_popover_open = !app.view.smoothing_popover_open;
                app.view.smoothing_popover_anchor = Some(resp.rect);
                app.view.elements_popover_open = false;
                app.view.color_picker.close();
            }
        });
    }
}

fn draw_center_zone(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    ui.spacing_mut().item_spacing.x = ZONE_ITEM_SPACING;
    draw_tool_section(app, ui, t);
}

fn draw_zoom_percent_pill(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ZOOM_PILL_WIDTH, TOOLBAR_ROW_HEIGHT),
        Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let hover_t = crate::motion::hover_factor(ui.ctx(), resp.id.with("hov"), resp.hovered());
        let fill =
            crate::motion::lerp_color(Color32::TRANSPARENT, t.border.gamma_multiply(0.45), hover_t);
        let stroke_c =
            crate::motion::lerp_color(t.border, t.text_muted.gamma_multiply(0.55), hover_t);
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(9), fill);
        p.rect_stroke(
            rect,
            CornerRadius::same(9),
            Stroke::new(1.0, stroke_c),
            StrokeKind::Inside,
        );
        let label = format!("{:.0}%", app.view.zoom * 100.0);
        let galley = p.layout_no_wrap(label, FontId::monospace(12.0), t.text_muted);
        let pos = egui::pos2(
            rect.center().x - galley.size().x * 0.5,
            rect.center().y - galley.size().y * 0.5,
        );
        p.galley(pos, galley, t.text_muted);
    }
    if resp
        .on_hover_text("Current zoom — click to reset and fit entire page in view")
        .clicked()
    {
        app.view.motion.fit_glide_requested = true;
        ui.ctx().request_repaint();
    }
}

fn draw_right_utility_controls(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    ui.spacing_mut().item_spacing.x = ZONE_ITEM_SPACING;
    let can_undo = app.session.history.can_undo();
    let can_redo = app.session.history.can_redo();
    if toolbar_icon_button(ui, Icon::Undo, None, false, can_undo, t)
        .on_hover_text("Undo last edit (Ctrl+Z)")
        .clicked()
    {
        let _ = app.session.execute("edit.undo", &json!({}));
    }
    if toolbar_icon_button(ui, Icon::Redo, None, false, can_redo, t)
        .on_hover_text("Redo undone edit (Ctrl+Y or Ctrl+Shift+Z)")
        .clicked()
    {
        let _ = app.session.execute("edit.redo", &json!({}));
    }
    if toolbar_icon_button(
        ui,
        Icon::ClearPage,
        None,
        false,
        !app.session.doc.strokes.is_empty()
            || !app.session.doc.texts.is_empty()
            || !app.session.doc.shapes.is_empty()
            || !app.session.doc.images.is_empty(),
        t,
    )
    .on_hover_text(
        "Clear all ink, inserted text, images, and shapes from active page (can be undone)",
    )
    .clicked()
    {
        let _ = app.session.execute("page.clear", &json!({}));
    }

    hairline_divider(ui, t);

    if toolbar_icon_button(ui, Icon::ZoomOut, None, false, true, t)
        .on_hover_text("Zoom out canvas")
        .clicked()
    {
        step_zoom(app, ui.ctx(), 0.85);
    }
    draw_zoom_percent_pill(app, ui, t);
    if toolbar_icon_button(ui, Icon::ZoomIn, None, false, true, t)
        .on_hover_text("Zoom in canvas (or hold Ctrl + Mouse Wheel)")
        .clicked()
    {
        step_zoom(app, ui.ctx(), 1.0 / 0.85);
    }

    hairline_divider(ui, t);

    let export_active = app.view.active_modal == Some(crate::modal::ActiveModal::Export);
    if toolbar_icon_button(ui, Icon::Export, None, export_active, true, t)
        .on_hover_text("Export notebook as PDF, PNG, SVG, or .pcraft")
        .clicked()
    {
        if export_active {
            crate::modal::close(app);
        } else {
            crate::modal::open(app, crate::modal::ActiveModal::Export);
        }
    }

    draw_sync_profile_button(app, ui, t);
}

/// Renders the `36x36` Profile & Cloud Sync icon button with a live connection status dot.
pub(crate) fn draw_sync_profile_button(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let acct_active = app.view.screen == crate::AppScreen::Profile
        || app.view.active_modal == Some(crate::modal::ActiveModal::AccountSync);
    let summary = app.session.sync_status_summary();
    let cloud_pending = app.session.cloud_pending_changes_count();
    let (state_label, cloud_dot) = crate::account_modal::status_badge_style(summary.connection, t);
    let dot_color = if summary.viewing_cloud {
        cloud_dot
    } else {
        t.text_muted
    };
    let sync_tip = if summary.viewing_cloud {
        let handle = summary
            .username
            .as_deref()
            .map_or_else(String::new, |u| format!(" · @{u}"));
        format!(
            "Profile: {}{handle}\nStatus: {} ({cloud_pending} waiting to sync)\nServer: {}\nClick to open Profile & Cloud Sync.",
            summary.workspace_badge_text(),
            state_label,
            summary.server_url
        )
    } else if summary.signed_in {
        let prof = summary
            .active_local_profile_name
            .as_deref()
            .unwrap_or("Personal");
        format!(
            "Workspace: Local Only ({prof})\nCloud account ({}) is connected separately.\nClick to open Profile & Cloud Sync.",
            summary
                .email
                .as_deref()
                .or(summary.username.as_deref())
                .unwrap_or("account")
        )
    } else {
        let prof = summary
            .active_local_profile_name
            .as_deref()
            .unwrap_or("Personal");
        format!(
            "Workspace: Local Only ({prof})\nStored only on this device. Click to open Profile, manage local profiles, or connect Cloud Sync."
        )
    };
    let sync_resp = toolbar_icon_button(ui, Icon::Sync, None, acct_active, true, t);
    if ui.is_rect_visible(sync_resp.rect) {
        let dot_c = egui::pos2(sync_resp.rect.right() - 6.5, sync_resp.rect.bottom() - 6.5);
        ui.painter().circle_filled(dot_c, 4.2, t.toolbar_bg);
        ui.painter().circle_filled(dot_c, 3.0, dot_color);
    }
    if sync_resp.on_hover_text(sync_tip).clicked() {
        crate::open_profile_screen(app);
    }
}

fn draw_settings_edge_button(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    #[cfg(not(target_arch = "wasm32"))]
    let has_update = app.updater.has_actionable_update();
    #[cfg(target_arch = "wasm32")]
    let has_update = false;

    let settings_tip = if has_update {
        "Settings (Shortcut: S)\nPen shortcuts, studio appearance, system info, and an available app update."
    } else {
        "Settings (Shortcut: S)\nPen shortcuts, studio appearance, and system info."
    };
    let settings_resp = sidebar_edge_button_sized(
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
    .on_hover_text(settings_tip);
    if has_update && ui.is_rect_visible(settings_resp.rect) {
        let dot_c = egui::pos2(
            settings_resp.rect.right() - 7.0,
            settings_resp.rect.top() + 6.5,
        );
        ui.painter().circle_filled(dot_c, 4.2, t.toolbar_bg);
        ui.painter().circle_filled(dot_c, 3.0, t.accent);
    }
    if settings_resp.clicked() {
        app.view.inspector_open = !app.view.inspector_open;
        if app.view.inspector_open
            && ui.ctx().content_rect().width() < crate::DUAL_SIDEBAR_MIN_VIEWPORT
        {
            app.view.pages_sidebar_open = false;
        }
    }
}

fn push_ruler_angle_control(app: &PaperCraftApp, flow: &mut DockFlow<'_>) {
    let initial_angle = if app.view.ruler_angle.is_finite() {
        normalized_degrees(app.view.ruler_angle.to_degrees())
    } else {
        0.0
    };
    flow.push(RULER_ANGLE_CONTROL_WIDTH, move |app, ui, _t| {
        let mut angle_degrees = initial_angle;
        let response = ui.add_sized(
            Vec2::new(RULER_ANGLE_CONTROL_WIDTH, CONTEXT_TRAY_ROW_HEIGHT),
            egui::DragValue::new(&mut angle_degrees)
                .range(-180.0..=180.0)
                .fixed_decimals(0)
                .suffix("°")
                .speed(1.0),
        );
        if response.changed() && angle_degrees.is_finite() {
            app.view.ruler_angle = angle_degrees.to_radians();
            let center = crate::canvas::clamp_ruler_center(
                app,
                Point::new(app.view.ruler_center[0], app.view.ruler_center[1]),
            );
            app.view.ruler_center = [center.x, center.y];
        }
        response.on_hover_text(
            "Ruler angle in degrees — double-click to type a value or drag to adjust",
        );
    });
}

fn draft_text_range(app: &PaperCraftApp) -> (Option<(usize, usize)>, usize, usize) {
    let len = app.view.text_draft.len();
    let sel = app
        .view
        .text_selection_bytes
        .and_then(|[a, b]| (app.view.text_editing && a < b && b <= len).then_some((a, b)));
    let (start, end) = sel.unwrap_or((0, len));
    (sel, start, end)
}

fn draft_text_snapshot(app: &PaperCraftApp) -> TextItem {
    TextItem {
        text: app.view.text_draft.clone(),
        size: app.view.text_size_draft,
        color: app.view.text_color_draft,
        font: app.view.text_font_draft,
        alignment: app.view.text_alignment_draft,
        spans: app.view.text_spans_draft.clone(),
        ..TextItem::default()
    }
}

fn apply_draft_text_span_or_box(
    app: &mut PaperCraftApp,
    set_box: impl FnOnce(&mut crate::ViewState),
    mutate_spans: impl FnOnce(&mut TextItem, usize, usize, bool),
) {
    let len = app.view.text_draft.len();
    let partial = draft_text_range(app)
        .0
        .filter(|&(a, b)| !(a == 0 && b == len));
    let mut item = draft_text_snapshot(app);
    if let Some((a, b)) = partial {
        mutate_spans(&mut item, a, b, true);
    } else {
        set_box(&mut app.view);
        if len > 0 {
            mutate_spans(&mut item, 0, len, false);
        }
    }
    app.view.text_spans_draft = item.spans;
}

pub(crate) fn apply_draft_text_font(app: &mut PaperCraftApp, font: TextFont) {
    apply_draft_text_span_or_box(
        app,
        |v| v.text_font_draft = font,
        |item, a, b, is_partial| {
            item.set_span_font(a, b, is_partial.then_some(font));
        },
    );
}

pub(crate) fn apply_draft_text_size(app: &mut PaperCraftApp, size: f32) {
    let clamped = if size.is_finite() {
        size.clamp(6.0, 120.0)
    } else {
        24.0
    };
    apply_draft_text_span_or_box(
        app,
        |v| v.text_size_draft = clamped,
        |item, a, b, is_partial| {
            item.set_span_size(a, b, is_partial.then_some(clamped));
        },
    );
}

pub(crate) fn apply_draft_text_color(app: &mut PaperCraftApp, color: Color) {
    let clean = color.sanitized();
    apply_draft_text_span_or_box(
        app,
        |v| v.text_color_draft = clean,
        |item, a, b, is_partial| {
            item.set_span_color(a, b, is_partial.then_some(clean));
        },
    );
}

fn push_text_insert_controls(
    app: &PaperCraftApp,
    ui: &egui::Ui,
    flow: &mut DockFlow<'_>,
    t: &Tokens,
) {
    let (_, range_start, range_end) = draft_text_range(app);
    let snapshot = draft_text_snapshot(app);
    let effective_font = snapshot.effective_font_across(range_start, range_end);
    let font_label = effective_font.map_or("Mixed", TextFont::label);
    let effective_size = snapshot.effective_size_across(range_start, range_end);

    let font_label_w = crate::theme::studio_row_label_width(
        ui,
        egui::RichText::new("Font").size(11.5).color(t.text_muted),
    );
    let combo_w = crate::theme::studio_combo_box_width(ui, font_label, 82.0);
    flow.push(
        font_label_w + ZONE_ITEM_SPACING + combo_w,
        move |app, ui, t| {
            crate::theme::studio_row_label(
                ui,
                egui::RichText::new("Font").size(11.5).color(t.text_muted),
                CONTEXT_TRAY_ROW_HEIGHT,
            );
            let mut chosen_font: Option<TextFont> = None;
            crate::theme::studio_combo_box(ui, "text_insert_font", font_label, 82.0, |ui| {
                for font in TextFont::ALL {
                    if ui
                        .selectable_label(effective_font == Some(*font), font.label())
                        .clicked()
                    {
                        chosen_font = Some(*font);
                    }
                }
            });
            if let Some(font) = chosen_font {
                apply_draft_text_font(app, font);
                if app.view.text_editing {
                    app.view.text_editor_needs_focus = true;
                }
            }
        },
    );

    let size_label_w = crate::theme::studio_row_label_width(
        ui,
        egui::RichText::new("Size").size(11.5).color(t.text_muted),
    );
    let initial_size = effective_size.unwrap_or(app.view.text_size_draft);
    flow.push(
        size_label_w + ZONE_ITEM_SPACING + 72.0,
        move |app, ui, t| {
            crate::theme::studio_row_label(
                ui,
                egui::RichText::new("Size").size(11.5).color(t.text_muted),
                CONTEXT_TRAY_ROW_HEIGHT,
            );
            let mut size_val = initial_size;
            let size_resp = ui
                .add_sized(
                    Vec2::new(72.0, CONTEXT_TRAY_ROW_HEIGHT),
                    egui::DragValue::new(&mut size_val)
                        .range(6.0..=120.0)
                        .speed(1.0)
                        .suffix(" px"),
                )
                .on_hover_text("Text size — larger text stands out more on the page");
            if size_resp.changed() {
                apply_draft_text_size(app, size_val);
            }
            if (size_resp.drag_stopped() || size_resp.lost_focus()) && app.view.text_editing {
                app.view.text_editor_needs_focus = true;
            }
        },
    );

    flow.push(82.0, |app, ui, _t| {
        let line_resp = ui
            .add_sized(
                Vec2::new(82.0, CONTEXT_TRAY_ROW_HEIGHT),
                egui::DragValue::new(&mut app.view.text_line_spacing_draft)
                    .range(0.8..=3.0)
                    .speed(0.05)
                    .fixed_decimals(2)
                    .prefix("Line ")
                    .suffix("×"),
            )
            .on_hover_text("Line spacing — adjust vertical distance between lines");
        if (line_resp.drag_stopped() || line_resp.lost_focus()) && app.view.text_editing {
            app.view.text_editor_needs_focus = true;
        }
    });

    flow.push(92.0, |app, ui, _t| {
        let letter_resp = ui
            .add_sized(
                Vec2::new(92.0, CONTEXT_TRAY_ROW_HEIGHT),
                egui::DragValue::new(&mut app.view.text_letter_spacing_draft)
                    .range(-4.0..=32.0)
                    .speed(0.2)
                    .fixed_decimals(1)
                    .prefix("Letter ")
                    .suffix(" px"),
            )
            .on_hover_text("Letter spacing — adjust horizontal space between characters");
        if (letter_resp.drag_stopped() || letter_resp.lost_focus()) && app.view.text_editing {
            app.view.text_editor_needs_focus = true;
        }
    });

    let temp_item = snapshot;
    let (bold_on, italic_on, underline_on, hl_active) =
        temp_item.style_active_across(range_start, range_end);
    let has_bullets = temp_item
        .text
        .lines()
        .any(|l| parse_list_line_prefix(l).0 == Some(ListItemKind::Bullet));
    let has_checks = temp_item.text.lines().any(|l| {
        matches!(
            parse_list_line_prefix(l).0,
            Some(ListItemKind::Checkbox { .. })
        )
    });
    let hl_yellow = Color::from_rgba8(0xFE, 0xF0, 0x8A, 0xCC);
    let hl_is_active = hl_active.is_some();
    let temp_rc = std::rc::Rc::new(temp_item);
    push_rich_text_style_bar(
        flow,
        ui,
        "text_insert_align",
        app.view.text_alignment_draft,
        bold_on,
        italic_on,
        underline_on,
        hl_is_active,
        has_bullets,
        has_checks,
        move |app, action| {
            let mut item = (*temp_rc).clone();
            match action {
                RichTextAction::ToggleBold => {
                    item.toggle_span_style(range_start, range_end, TextStyleAttr::Bold);
                    app.view.text_spans_draft = item.spans;
                }
                RichTextAction::ToggleItalic => {
                    item.toggle_span_style(range_start, range_end, TextStyleAttr::Italic);
                    app.view.text_spans_draft = item.spans;
                }
                RichTextAction::ToggleUnderline => {
                    item.toggle_span_style(range_start, range_end, TextStyleAttr::Underline);
                    app.view.text_spans_draft = item.spans;
                }
                RichTextAction::ToggleHighlight => {
                    item.set_span_highlight(
                        range_start,
                        range_end,
                        (!hl_is_active).then_some(hl_yellow),
                    );
                    app.view.text_spans_draft = item.spans;
                }
                RichTextAction::SetAlignment(align) => {
                    app.view.text_alignment_draft = align;
                }
                RichTextAction::ToggleList(want_checkbox) => {
                    let _ = item.toggle_list_in_range(range_start, range_end, want_checkbox);
                    app.view.text_draft = item.text;
                    app.view.text_prev_draft = app.view.text_draft.clone();
                    app.view.text_spans_draft = item.spans;
                }
            }
            if app.view.text_editing {
                app.view.text_editor_needs_focus = true;
            }
        },
    );

    flow.divider();
    let detected_draft_url = find_text_url_spans(&app.view.text_draft)
        .into_iter()
        .next()
        .map(|s| s.url);
    let has_link = app.view.text_link_draft.is_some() || detected_draft_url.is_some();
    let link_active = app.view.text_link_editor_open || has_link;
    flow.push(tray_icon_button_width(ui, "Link"), move |app, ui, t| {
        if tray_icon_button(ui, Icon::Link, Some("Link"), link_active, true, t)
            .on_hover_text("Insert or attach a clickable link URL to this text")
            .clicked()
        {
            app.view.text_link_editor_open = !app.view.text_link_editor_open;
            if app.view.text_link_editor_open {
                app.view.text_link_input = app
                    .view
                    .text_link_draft
                    .clone()
                    .or(detected_draft_url)
                    .unwrap_or_default();
            } else if app.view.text_editing {
                app.view.text_editor_needs_focus = true;
            }
        }
    });
    if app.view.text_link_editor_open {
        push_inline_link_editor_bar(app, ui, flow, |app, action| match action {
            LinkEditorAction::Apply(url) => {
                app.view.text_link_draft = Some(url.clone());
                if app.view.text_editing {
                    if app.view.text_draft.trim().is_empty() {
                        app.view.text_draft = url;
                    }
                    app.view.text_editor_needs_focus = true;
                } else if app.view.text_insert_mode {
                    let _ = crate::insert_text_on_page(app, &url, Some(url.clone()));
                }
                app.view.text_link_editor_open = false;
            }
            LinkEditorAction::Clear => {
                app.view.text_link_draft = None;
                app.view.text_link_input.clear();
                app.view.text_link_editor_open = false;
                if app.view.text_editing {
                    app.view.text_editor_needs_focus = true;
                }
            }
        });
    }
}

fn context_tray_mode(app: &PaperCraftApp) -> Option<ContextTrayMode> {
    if app.view.text_insert_mode || app.view.text_editing {
        return Some(ContextTrayMode::TextInsert);
    }
    if app.view.shape_insert_mode {
        return Some(ContextTrayMode::ShapeInsert(app.session.active_pen_kind()));
    }
    if app.session.active_tool == ToolKind::Select
        && (!app.view.selected_objects.is_empty() || app.view.selected_object.is_some())
    {
        let selection = if app.view.selected_objects.is_empty() {
            app.view.selected_object.into_iter().collect()
        } else {
            app.view.selected_objects.clone()
        };
        return Some(ContextTrayMode::Selection(selection));
    }
    if app.view.ruler_active {
        return Some(ContextTrayMode::Ruler {
            tool: app.session.active_tool,
        });
    }
    match app.session.active_tool {
        ToolKind::Pen(kind) => Some(ContextTrayMode::Pen(kind)),
        ToolKind::Eraser => Some(ContextTrayMode::Eraser),
        ToolKind::Select => Some(ContextTrayMode::SelectEmpty),
        ToolKind::LaserPointer => Some(ContextTrayMode::LaserPointer),
    }
}

fn select_mode_switch_options(show_labels: bool) -> [SwitchOption<'static>; 2] {
    [
        SwitchOption::new(
            Some(Icon::Select),
            show_labels.then_some("Box"),
            "Rectangular marquee — drag a box across items to select",
            true,
        ),
        SwitchOption::new(
            Some(Icon::Lasso),
            show_labels.then_some("Lasso"),
            "Freeform lasso — draw around items to select them",
            true,
        ),
    ]
}

fn push_select_mode_switch(
    app: &PaperCraftApp,
    ui: &egui::Ui,
    flow: &mut DockFlow<'_>,
    show_labels: bool,
    id_salt: &'static str,
) {
    let options = select_mode_switch_options(show_labels);
    let switch_w = segmented_switch_width(ui, &options, SwitchLayout::TrayCompact);
    let lasso_mode = app.view.freeform_lasso_mode;
    flow.push(switch_w, move |app, ui, t| {
        let opts = select_mode_switch_options(show_labels);
        let (clicked, _) = segmented_switch(
            ui,
            id_salt,
            &opts,
            Some(usize::from(lasso_mode)),
            SwitchLayout::TrayCompact,
            t,
        );
        if let Some(idx) = clicked {
            app.view.freeform_lasso_mode = idx == 1;
        }
    });
}

fn push_select_empty_bar(app: &PaperCraftApp, ui: &egui::Ui, flow: &mut DockFlow<'_>) {
    push_select_mode_switch(app, ui, flow, true, "toolbar_select_mode_switch");
    if app.session.clipboard.is_some() {
        flow.divider();
        flow.push(tray_icon_button_width(ui, "Paste"), |app, ui, t| {
            if tray_icon_button(ui, Icon::Paste, Some("Paste"), false, true, t)
                .on_hover_text("Paste clipboard selection onto current page (Ctrl+V)")
                .clicked()
                && let Ok(val) = app.session.execute("selection.paste", &json!({}))
            {
                crate::select_stamped_result(app, &val);
            }
        });
    }
}

fn push_laser_quick_bar(app: &PaperCraftApp, ui: &egui::Ui, flow: &mut DockFlow<'_>) {
    let options = [
        SwitchOption::new(
            Some(Icon::LaserPointer),
            Some("Trail"),
            "Fading trail — draws a glowing laser line that fades after ~2 seconds",
            true,
        ),
        SwitchOption::new(
            Some(Icon::EllipseShape),
            Some("Dot"),
            "Spotlight dot — projects a temporary laser dot while holding down",
            true,
        ),
    ];
    let switch_w = segmented_switch_width(ui, &options, SwitchLayout::TrayCompact);
    let trail_mode = app.view.laser_trail_mode;
    flow.push(switch_w, move |app, ui, t| {
        let opts = [
            SwitchOption::new(
                Some(Icon::LaserPointer),
                Some("Trail"),
                "Fading trail — draws a glowing laser line that fades after ~2 seconds",
                true,
            ),
            SwitchOption::new(
                Some(Icon::EllipseShape),
                Some("Dot"),
                "Spotlight dot — projects a temporary laser dot while holding down",
                true,
            ),
        ];
        let (clicked, _) = segmented_switch(
            ui,
            "toolbar_laser_mode_switch",
            &opts,
            Some(usize::from(!trail_mode)),
            SwitchLayout::TrayCompact,
            t,
        );
        if let Some(idx) = clicked {
            app.view.laser_trail_mode = idx == 0;
        }
    });
}

fn push_context_controls<'a>(
    app: &PaperCraftApp,
    ui: &egui::Ui,
    flow: &mut DockFlow<'a>,
    t: &Tokens,
    mode: &'a ContextTrayMode,
) {
    match mode {
        ContextTrayMode::Selection(selected) => {
            let _ = push_selection_controls(app, ui, flow, t, selected);
        }
        ContextTrayMode::SelectEmpty => push_select_empty_bar(app, ui, flow),
        ContextTrayMode::ShapeInsert(pen_kind) => {
            push_shape_quick_bar(app, ui, flow);
            flow.divider();
            push_pen_quick_bar(app, ui, flow, *pen_kind, false);
        }
        ContextTrayMode::TextInsert => push_text_insert_controls(app, ui, flow, t),
        ContextTrayMode::Ruler { tool } => {
            push_ruler_angle_control(app, flow);
            flow.divider();
            match tool {
                ToolKind::Pen(kind) => push_pen_quick_bar(app, ui, flow, *kind, true),
                ToolKind::Eraser => push_eraser_quick_bar(app, ui, flow),
                ToolKind::Select => push_select_empty_bar(app, ui, flow),
                ToolKind::LaserPointer => push_laser_quick_bar(app, ui, flow),
            }
        }
        ContextTrayMode::Pen(kind) => push_pen_quick_bar(app, ui, flow, *kind, true),
        ContextTrayMode::Eraser => push_eraser_quick_bar(app, ui, flow),
        ContextTrayMode::LaserPointer => push_laser_quick_bar(app, ui, flow),
    }
}

fn tray_workspace_bounds(
    viewport: egui::Rect,
    header_rect: egui::Rect,
    workspace_rect: egui::Rect,
) -> egui::Rect {
    if workspace_rect.is_positive() && workspace_rect.width() >= CONTEXT_TRAY_MIN_CANVAS_WIDTH {
        workspace_rect
    } else {
        egui::Rect::from_min_max(
            egui::pos2(viewport.left(), header_rect.bottom().max(viewport.top())),
            viewport.max,
        )
    }
}

/// Renders the floating context tray centered within `workspace_rect` and the
/// floating draggable color dock (initially docked on the left side of `workspace_rect`).
pub(crate) fn show_context_tray_in_rect(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    header_rect: egui::Rect,
    workspace_rect: egui::Rect,
) {
    let top_tray_rect = show_top_context_tray_in_rect(app, ui, header_rect, workspace_rect);
    show_color_dock_in_rect(app, ui, header_rect, workspace_rect, top_tray_rect);
}

fn show_top_context_tray_in_rect(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    header_rect: egui::Rect,
    workspace_rect: egui::Rect,
) -> Option<egui::Rect> {
    let mode = context_tray_mode(app);
    let tray_open = mode.is_some();
    let area_id = ui.make_persistent_id("papercraft_context_tray");
    let dt = ui.ctx().input(|i| i.stable_dt);
    let (reveal, display_mode, animating) = app.view.context_tray.advance(mode, dt);
    if animating {
        ui.ctx().request_repaint();
    }
    let display_mode = display_mode?;
    if reveal <= 0.001 {
        return None;
    }

    let t = Tokens::for_theme(app.session.prefs.theme);
    let viewport = ui.ctx().content_rect();
    let bounds = tray_workspace_bounds(viewport, header_rect, workspace_rect);
    let bounds_left = bounds.left() + CONTEXT_TRAY_OUTER_MARGIN;
    let bounds_right = (bounds.right() - CONTEXT_TRAY_OUTER_MARGIN).max(bounds_left + 120.0);
    let max_frame_w = (bounds_right - bounds_left).max(160.0);
    let max_content_w = (max_frame_w - CONTEXT_TRAY_INNER_MARGIN_X * 2.0).max(120.0);

    let mut flow = DockFlow::new(max_content_w);
    push_context_controls(app, ui, &mut flow, &t, &display_mode);
    if flow.entries.is_empty() {
        return None;
    }

    let specs: Vec<DockSpec> = flow
        .entries
        .iter()
        .map(|e| DockSpec {
            width: e.width,
            break_before: e.break_before,
            has_divider_before: e.has_divider_before,
        })
        .collect();
    let divider_extra = DIVIDER_WIDTH + ZONE_ITEM_SPACING;
    let rows = plan_dock_rows(&specs, max_content_w, ZONE_ITEM_SPACING, divider_extra);
    if rows.is_empty() {
        return None;
    }

    let dock_content_w = rows
        .iter()
        .map(|row| row.width)
        .fold(0.0_f32, f32::max)
        .max(1.0);
    let frame_w = dock_content_w + CONTEXT_TRAY_INNER_MARGIN_X * 2.0;
    let target_center_x = if workspace_rect.is_positive() {
        workspace_rect.center().x
    } else {
        bounds.center().x
    };
    let max_x = (bounds_right - frame_w).max(bounds_left);
    let x_offset = (target_center_x - frame_w * 0.5).clamp(bounds_left, max_x);
    let y_offset = header_rect.bottom().max(bounds.top().max(viewport.top()))
        + 5.0
        + (1.0 - reveal) * CONTEXT_TRAY_SETTLE_OFFSET_Y;

    let mut entries: Vec<Option<DockEntry<'_>>> = flow.entries.into_iter().map(Some).collect();

    let area_resp = egui::Area::new(area_id)
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(x_offset, y_offset))
        .constrain_to(viewport.shrink(4.0))
        .interactable(tray_open)
        .fade_in(false)
        .show(ui.ctx(), |ui| {
            ui.set_max_width(max_frame_w);
            ui.multiply_opacity(reveal);
            egui::Frame::new()
                .fill(t.panel_bg)
                .stroke(Stroke::new(1.0, t.border))
                .corner_radius(CornerRadius::same(CONTEXT_TRAY_CORNER_RADIUS))
                .inner_margin(egui::Margin::symmetric(
                    CONTEXT_TRAY_INNER_MARGIN_X as i8,
                    CONTEXT_TRAY_INNER_MARGIN_Y as i8,
                ))
                .show(ui, |ui| {
                    ui.add_enabled_ui(tray_open, |ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing =
                                Vec2::new(ZONE_ITEM_SPACING, CONTEXT_TRAY_ROW_GAP);
                            for row in &rows {
                                let (row_rect, _) = ui.allocate_exact_size(
                                    Vec2::new(dock_content_w, CONTEXT_TRAY_ROW_HEIGHT),
                                    Sense::hover(),
                                );
                                let row_left =
                                    (row_rect.center().x - row.width * 0.5).max(row_rect.left());
                                let child_rect = egui::Rect::from_min_max(
                                    egui::pos2(row_left, row_rect.top()),
                                    egui::pos2(row_rect.right(), row_rect.bottom()),
                                );
                                let mut row_ui = ui.new_child(
                                    egui::UiBuilder::new()
                                        .max_rect(child_rect)
                                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                                );
                                row_ui.spacing_mut().item_spacing =
                                    Vec2::new(ZONE_ITEM_SPACING, 0.0);
                                for idx in row.start..row.end {
                                    let Some(entry) = entries.get_mut(idx).and_then(Option::take)
                                    else {
                                        continue;
                                    };
                                    if idx > row.start && entry.has_divider_before {
                                        hairline_divider_sized(
                                            &mut row_ui,
                                            CONTEXT_TRAY_ROW_HEIGHT,
                                            &t,
                                        );
                                    }
                                    (entry.draw)(app, &mut row_ui, &t);
                                }
                            }
                        });
                    });
                });
        });
    Some(area_resp.response.rect)
}

fn show_color_dock_in_rect(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    header_rect: egui::Rect,
    workspace_rect: egui::Rect,
    top_tray_rect: Option<egui::Rect>,
) {
    let mode = color_dock_mode(app);
    let dock_open = mode.is_some();
    let area_id = ui.make_persistent_id("papercraft_color_dock");
    let grip_id = area_id.with("grip");
    let dt = crate::motion::sanitize_dt(ui.ctx().input(|i| i.stable_dt));
    let (reveal, display_mode, animating) = app.view.color_dock.transition.advance(mode, dt);
    if animating {
        ui.ctx().request_repaint();
    }
    let Some(display_mode) = display_mode else {
        app.view.color_dock.clear_ephemeral();
        return;
    };
    if reveal <= 0.001 {
        app.view.color_dock.clear_ephemeral();
        return;
    }

    let palette_spec = palette_dock_spec(app, &display_mode);
    let (swatch_count, has_wildcard, has_palette_btn, has_shape_channels) = match &palette_spec {
        Some(spec) => (
            app.session.prefs.palette(spec.target).len(),
            true,
            true,
            spec.shape_channels.is_some(),
        ),
        None => (LASER_COLOR_COUNT, false, false, false),
    };

    let t = Tokens::for_theme(app.session.prefs.theme);
    let viewport = ui.ctx().content_rect();
    let bounds = tray_workspace_bounds(viewport, header_rect, workspace_rect);
    let max_frame_h = (bounds.height() - COLOR_DOCK_OUTER_MARGIN * 2.0).max(96.0);
    let layout = plan_color_dock_layout(
        swatch_count,
        has_wildcard,
        has_palette_btn,
        has_shape_channels,
        max_frame_h,
    );

    let bounds_min = egui::pos2(
        bounds.left() + COLOR_DOCK_OUTER_MARGIN,
        bounds.top().max(header_rect.bottom()).max(viewport.top()) + 5.0,
    );
    let bounds_max = egui::pos2(
        (bounds.right() - COLOR_DOCK_OUTER_MARGIN - layout.frame_w).max(bounds_min.x),
        (bounds.bottom() - COLOR_DOCK_OUTER_MARGIN - layout.frame_h).max(bounds_min.y),
    );

    let safe_top_y = top_tray_rect.map_or(bounds_min.y, |r| (r.bottom() + 8.0).max(bounds_min.y));
    let centered_y = bounds.center().y - layout.frame_h * 0.5;
    let default_y = if bounds_max.y >= safe_top_y {
        centered_y.clamp(safe_top_y, bounds_max.y)
    } else {
        centered_y.clamp(bounds_min.y, bounds_max.y)
    };
    let default_pos = egui::pos2(bounds_min.x, default_y);

    let prev_visual = app.view.color_dock.visual_pos;
    if app.view.color_dock.is_dragging && dock_open {
        let (down, ptr_pos) = ui
            .ctx()
            .input(|i| (i.pointer.primary_down(), i.pointer.interact_pos()));
        if down
            && let Some(ptr) = ptr_pos
            && ptr.x.is_finite()
            && ptr.y.is_finite()
            && let Some(offset) = app.view.color_dock.drag_grab_offset
        {
            app.view.color_dock.custom_pos = Some(ColorDockState::clamp_pos(
                ptr + offset,
                bounds_min,
                bounds_max,
            ));
        }
    }

    let target_pos = ColorDockState::clamp_pos(
        app.view.color_dock.custom_pos.unwrap_or(default_pos),
        bounds_min,
        bounds_max,
    );
    let next_visual = match prev_visual {
        None => target_pos,
        Some(cur) => {
            let cur_clamped = ColorDockState::clamp_pos(cur, bounds_min, bounds_max);
            let delta = target_pos - cur_clamped;
            if delta.length_sq() <= 0.10 || dt <= 0.0 {
                target_pos
            } else {
                let rate = if app.view.color_dock.is_dragging {
                    COLOR_DOCK_DRAG_SMOOTH_RATE
                } else {
                    COLOR_DOCK_GLIDE_SMOOTH_RATE
                };
                let alpha = (1.0 - (-rate * dt).exp()).clamp(0.0, 1.0);
                ColorDockState::clamp_pos(cur_clamped + delta * alpha, bounds_min, bounds_max)
            }
        }
    };
    app.view.color_dock.visual_pos = Some(next_visual);
    if (target_pos - next_visual).length_sq() > 0.10 {
        ui.ctx().request_repaint();
    }
    if let Some(prev) = prev_visual {
        let moved = next_visual - prev;
        if moved.length_sq() > 0.001
            && app.view.color_picker.open
            && app.view.color_dock.last_rect.is_positive()
            && app
                .view
                .color_dock
                .last_rect
                .intersects(app.view.color_picker.anchor_rect.expand(4.0))
        {
            app.view.color_picker.anchor_rect = app.view.color_picker.anchor_rect.translate(moved);
        }
    }

    let settle_x = if app.view.color_dock.custom_pos.is_none() {
        -(1.0 - reveal) * COLOR_DOCK_SETTLE_OFFSET_X
    } else {
        0.0
    };
    let render_pos = egui::pos2(next_visual.x + settle_x, next_visual.y);

    let drag_lift = crate::motion::hover_factor(
        ui.ctx(),
        area_id.with("drag_lift"),
        app.view.color_dock.is_dragging,
    );
    let border_c = crate::motion::lerp_color(t.border, t.accent.gamma_multiply(0.75), drag_lift);
    let shadow_alpha_base = if t.is_dark { 96.0_f32 } else { 28.0_f32 };
    let shadow_alpha_drag = if t.is_dark { 142.0_f32 } else { 48.0_f32 };
    let shadow = egui::Shadow {
        offset: [
            0,
            crate::motion::lerp_f32(4.0, 9.0, drag_lift).round() as i8,
        ],
        blur: crate::motion::lerp_f32(14.0, 22.0, drag_lift).round() as u8,
        spread: 0,
        color: Color32::from_black_alpha(
            crate::motion::lerp_f32(shadow_alpha_base, shadow_alpha_drag, drag_lift).round() as u8,
        ),
    };

    let area_resp = egui::Area::new(area_id)
        .order(egui::Order::Middle)
        .fixed_pos(render_pos)
        .constrain_to(viewport.shrink(4.0))
        .interactable(dock_open)
        .fade_in(false)
        .show(ui.ctx(), |ui| {
            ui.multiply_opacity(reveal);
            let frame_resp = egui::Frame::new()
                .fill(t.panel_bg)
                .stroke(Stroke::new(1.0, border_c))
                .corner_radius(CornerRadius::same(COLOR_DOCK_CORNER_RADIUS))
                .shadow(shadow)
                .inner_margin(egui::Margin::symmetric(
                    COLOR_DOCK_INNER_MARGIN_X as i8,
                    COLOR_DOCK_INNER_MARGIN_Y as i8,
                ))
                .show(ui, |ui| {
                    ui.add_enabled_ui(dock_open, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.set_width(layout.inner_w);
                            ui.spacing_mut().item_spacing = Vec2::splat(COLOR_DOCK_ITEM_GAP);
                            let grip_resp = draw_color_dock_grip(
                                ui,
                                grip_id,
                                layout.inner_w,
                                app.view.color_dock.is_dragging,
                                &t,
                            );
                            draw_color_dock_divider(ui, layout.inner_w, &t);
                            if let Some(spec) = palette_spec.as_ref() {
                                draw_palette_dock_controls(app, ui, &layout, spec, &t);
                            } else {
                                draw_laser_dock_controls(app, ui, &layout, &t);
                            }
                            grip_resp
                        })
                        .inner
                    })
                    .inner
                });
            frame_resp.inner
        });

    app.view.color_dock.last_rect = area_resp.response.rect;
    let grip_resp = area_resp.inner;

    if grip_resp.double_clicked() && dock_open {
        app.view.color_dock.custom_pos = None;
        app.view.color_dock.is_dragging = false;
        app.view.color_dock.drag_grab_offset = None;
        ui.ctx().request_repaint();
    } else if grip_resp.dragged() && dock_open {
        app.view.color_dock.is_dragging = true;
        if app.view.color_dock.drag_grab_offset.is_none() {
            let origin = ui
                .ctx()
                .input(|i| i.pointer.press_origin())
                .or_else(|| grip_resp.interact_pointer_pos())
                .or_else(|| ui.ctx().pointer_interact_pos());
            if let Some(p) = origin
                && p.x.is_finite()
                && p.y.is_finite()
            {
                app.view.color_dock.drag_grab_offset = Some(next_visual - p);
            }
        }
        if let Some(ptr) = grip_resp
            .interact_pointer_pos()
            .or_else(|| ui.ctx().pointer_interact_pos())
            .filter(|p| p.x.is_finite() && p.y.is_finite())
        {
            let offset = *app
                .view
                .color_dock
                .drag_grab_offset
                .get_or_insert(next_visual - ptr);
            let clamped = ColorDockState::clamp_pos(ptr + offset, bounds_min, bounds_max);
            app.view.color_dock.custom_pos = Some(clamped);
            if dt <= 0.0 {
                app.view.color_dock.visual_pos = Some(clamped);
            }
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        ui.ctx().request_repaint();
    } else if grip_resp.is_pointer_button_down_on() && dock_open {
        app.view.color_dock.is_dragging = false;
        if app.view.color_dock.drag_grab_offset.is_none()
            && let Some(ptr) = grip_resp
                .interact_pointer_pos()
                .or_else(|| ui.ctx().pointer_interact_pos())
                .filter(|p| p.x.is_finite() && p.y.is_finite())
        {
            app.view.color_dock.drag_grab_offset = Some(next_visual - ptr);
        }
    } else {
        app.view.color_dock.is_dragging = false;
        app.view.color_dock.drag_grab_offset = None;
    }
}

fn show_context_tray(app: &mut PaperCraftApp, ui: &mut egui::Ui, header_rect: egui::Rect) {
    let viewport = ui.ctx().content_rect();
    let workspace_rect = egui::Rect::from_min_max(
        egui::pos2(viewport.left(), header_rect.bottom()),
        viewport.max,
    );
    show_context_tray_in_rect(app, ui, header_rect, workspace_rect);
}

fn step_zoom(app: &mut PaperCraftApp, ctx: &egui::Context, factor: f64) {
    let (base_zoom, base_pan) = match app.view.motion.camera_glide {
        Some(glide) => (glide.target_zoom, glide.target_pan),
        None => (app.view.zoom, app.view.pan),
    };
    let mut xf = Affine2::new(f64::from(base_zoom), base_pan[0], base_pan[1]);
    let page_center = xf.to_screen(Point::new(
        f64::from(app.session.doc.width) * 0.5,
        f64::from(app.session.doc.height) * 0.5,
    ));
    xf.zoom_around(page_center, factor, 0.15, 6.0);
    app.view.motion.fit_glide_requested = false;
    app.view.motion.camera_glide = Some(crate::motion::CameraGlide::new(
        app.view.zoom,
        app.view.pan,
        xf.scale as f32,
        [xf.offset_x, xf.offset_y],
        [page_center.x, page_center.y],
    ));
    ctx.request_repaint();
}

pub fn show(app: &mut PaperCraftApp, ui: &mut egui::Ui) {
    let header_rect = show_inner(app, ui);
    show_context_tray(app, ui, header_rect);
}

pub(crate) fn show_inner(app: &mut PaperCraftApp, ui: &mut egui::Ui) -> egui::Rect {
    let t = Tokens::for_theme(app.session.prefs.theme);
    let frame = egui::Frame::new()
        .fill(t.toolbar_bg)
        .inner_margin(egui::Margin {
            left: 0,
            right: 0,
            top: 8,
            bottom: 8,
        })
        .stroke(Stroke::new(1.0, t.border));

    egui::Panel::top("papercraft_toolbar")
        .frame(frame)
        .show(ui, |ui| {
            let avail_w = ui.available_width();
            let left_w = measure_left_zone(ui);
            let center_w = measure_center_zone(ui);
            let right_w = measure_right_zone(ui);
            let min_gap = 8.0_f32;

            if avail_w >= left_w + center_w + right_w + min_gap * 2.0 {
                ui.horizontal(|ui| {
                    ui.set_height(TOOLBAR_ROW_HEIGHT);
                    ui.spacing_mut().item_spacing = Vec2::new(ZONE_ITEM_SPACING, 0.0);
                    let row_left = ui.cursor().left();

                    draw_left_zone(app, ui, &t);

                    let ideal_center_x = row_left + (avail_w - center_w) * 0.5;
                    let min_center_x =
                        (ui.cursor().left() - ZONE_ITEM_SPACING).max(row_left) + min_gap;
                    let max_center_x = row_left + avail_w - right_w - center_w - min_gap;
                    let target_center_x =
                        ideal_center_x.clamp(min_center_x, max_center_x.max(min_center_x));
                    let left_gap = (target_center_x - ui.cursor().left()).max(0.0);
                    ui.add_space(left_gap);

                    draw_center_zone(app, ui, &t);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.set_height(TOOLBAR_ROW_HEIGHT);
                        ui.spacing_mut().item_spacing = Vec2::new(ZONE_ITEM_SPACING, 0.0);
                        draw_settings_edge_button(app, ui, &t);
                        ui.add_space((EDGE_TAB_NEIGHBOR_GAP - ZONE_ITEM_SPACING).max(0.0));
                        ui.allocate_ui_with_layout(
                            Vec2::new(RIGHT_UTILITIES_WIDTH, TOOLBAR_ROW_HEIGHT),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                draw_right_utility_controls(app, ui, &t);
                            },
                        );
                    });
                });
            } else {
                ui.horizontal(|ui| {
                    ui.set_height(TOOLBAR_ROW_HEIGHT);
                    ui.spacing_mut().item_spacing = Vec2::new(ZONE_ITEM_SPACING, 0.0);
                    draw_left_zone(app, ui, &t);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.set_height(TOOLBAR_ROW_HEIGHT);
                        ui.spacing_mut().item_spacing = Vec2::new(ZONE_ITEM_SPACING, 0.0);
                        draw_settings_edge_button(app, ui, &t);
                    });
                });
                ui.add_space(6.0);
                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(12, 0))
                    .show(ui, |ui| {
                        crate::theme::studio_wrapped_row(
                            ui,
                            Vec2::new(ZONE_ITEM_SPACING, 6.0),
                            TOOLBAR_ROW_HEIGHT,
                            |ui| {
                                ui.set_min_height(TOOLBAR_ROW_HEIGHT);
                                draw_center_zone(app, ui, &t);
                                hairline_divider(ui, &t);
                                draw_right_utility_controls(app, ui, &t);
                            },
                        );
                    });
            }
        })
        .response
        .rect
}

pub(crate) fn show_elements_popover(app: &mut PaperCraftApp, ui: &mut egui::Ui) {
    if !app.view.elements_popover_open {
        return;
    }
    let ctx = ui.ctx().clone();
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.view.elements_popover_open = false;
        return;
    }

    let t = Tokens::for_theme(app.session.prefs.theme);
    let theme = app.session.prefs.theme;
    let (auto_contrast, paper_color) = crate::effective_doc_ink_context(app);
    let built_ins = SavedElement::built_ins();
    let user_saved = app.session.prefs.saved_elements.clone();

    let mut close = false;
    let mut place_id: Option<String> = None;
    let mut delete_id: Option<String> = None;
    let mut save_selected_name: Option<String> = None;
    let has_selection = !app.view.selected_objects.is_empty();

    let viewport = ctx.content_rect();
    let pop_x = (viewport.center().x - 202.0).clamp(12.0, (viewport.right() - 416.0).max(12.0));
    let area_resp = egui::Area::new(ui.make_persistent_id("papercraft_elements_popover"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(pop_x, 56.0))
        .default_size(Vec2::new(404.0, 420.0))
        .constrain_to(viewport.shrink(6.0))
        .fade_in(false)
        .show(&ctx, |ui| {
            egui::Frame::new()
                .fill(t.panel_bg)
                .stroke(Stroke::new(1.0, t.border))
                .corner_radius(CornerRadius::same(12))
                .shadow(egui::Shadow {
                    offset: [0, 6],
                    blur: 18,
                    spread: 0,
                    color: Color32::from_black_alpha(if t.is_dark { 110 } else { 32 }),
                })
                .inner_margin(egui::Margin::same(14))
                .show(ui, |ui| {
                    ui.set_width(376.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Stickers & Elements")
                                .size(14.0)
                                .strong()
                                .color(t.text),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if compact_icon_button(ui, Icon::Close, None, false, true, &t)
                                .on_hover_text("Close Stickers & Elements library (Esc)")
                                .clicked()
                            {
                                close = true;
                            }
                        });
                    });
                    ui.label(
                        egui::RichText::new(
                            "Click any sticker or saved element to place it onto the current page.",
                        )
                        .size(11.5)
                        .color(t.text_muted),
                    );
                    ui.add_space(8.0);

                    let starter_label = format!("App Presets ({})", built_ins.len());
                    let saved_label = format!("My Stickers ({})", user_saved.len());
                    let switch_options = [
                        SwitchOption::new(
                            Some(Icon::Preset),
                            Some(starter_label.as_str()),
                            "Built-in sticker presets included with the app",
                            true,
                        ),
                        SwitchOption::new(
                            Some(Icon::Sticker),
                            Some(saved_label.as_str()),
                            "Custom stickers and elements you saved from your pages",
                            true,
                        ),
                    ];
                    let active_tab_idx = match app.view.stickers_tab {
                        crate::StickersTab::Starter => 0,
                        crate::StickersTab::Saved => 1,
                    };
                    let (clicked_tab, _) = segmented_switch(
                        ui,
                        "papercraft_stickers_tab_switch",
                        &switch_options,
                        Some(active_tab_idx),
                        SwitchLayout::EqualWidth,
                        &t,
                    );
                    if let Some(idx) = clicked_tab {
                        app.view.stickers_tab = match idx {
                            0 => crate::StickersTab::Starter,
                            _ => crate::StickersTab::Saved,
                        };
                    }
                    ui.add_space(8.0);

                    egui::ScrollArea::vertical()
                        .min_scrolled_height(276.0)
                        .max_height(276.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| match app.view.stickers_tab {
                            crate::StickersTab::Starter => {
                                draw_element_section(
                                    ui,
                                    &t,
                                    theme,
                                    auto_contrast,
                                    paper_color,
                                    &built_ins,
                                    false,
                                    &mut place_id,
                                    &mut delete_id,
                                );
                            }
                            crate::StickersTab::Saved => {
                                if has_selection {
                                    egui::Frame::new()
                                        .fill(t.field_bg)
                                        .stroke(Stroke::new(1.0, t.border))
                                        .corner_radius(CornerRadius::same(8))
                                        .inner_margin(egui::Margin::same(10))
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.label(
                                                egui::RichText::new("Save current selection")
                                                    .size(11.5)
                                                    .strong()
                                                    .color(t.text),
                                            );
                                            ui.add_space(4.0);
                                            ui.horizontal(|ui| {
                                                let save_w =
                                                    tray_icon_button_width(ui, "Save Selection");
                                                let input_w =
                                                    (ui.available_width() - save_w - 6.0).max(80.0);
                                                let edit_resp = ui.add_sized(
                                                    Vec2::new(input_w, CONTEXT_TRAY_ROW_HEIGHT),
                                                    egui::TextEdit::singleline(
                                                        &mut app.view.element_name_draft,
                                                    )
                                                    .hint_text("Sticker name...")
                                                    .font(FontId::proportional(11.5))
                                                    .align(egui::Align2::LEFT_CENTER),
                                                );
                                                let enter_save = edit_resp.lost_focus()
                                                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                                let clicked_save = tray_icon_button(
                                                    ui,
                                                    Icon::Check,
                                                    Some("Save Selection"),
                                                    true,
                                                    true,
                                                    &t,
                                                )
                                                .on_hover_text(
                                                    "Save selected page items as a reusable sticker",
                                                )
                                                .clicked();
                                                if enter_save || clicked_save {
                                                    let trimmed =
                                                        app.view.element_name_draft.trim();
                                                    save_selected_name = Some(if trimmed.is_empty() {
                                                        "Sticker".to_string()
                                                    } else {
                                                        trimmed.to_string()
                                                    });
                                                }
                                            });
                                        });
                                    ui.add_space(8.0);
                                }
                                if user_saved.is_empty() {
                                    egui::Frame::new()
                                        .fill(t.field_bg)
                                        .stroke(Stroke::new(1.0, t.border))
                                        .corner_radius(CornerRadius::same(8))
                                        .inner_margin(egui::Margin::same(12))
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.label(
                                                egui::RichText::new("No custom stickers yet.")
                                                    .size(12.0)
                                                    .strong()
                                                    .color(t.text),
                                            );
                                            ui.add_space(3.0);
                                            ui.label(
                                                egui::RichText::new(
                                                    "Select strokes, shapes, text, or images on a page, then open My Stickers to save them as reusable stickers here.",
                                                )
                                                .size(11.5)
                                                .color(t.text_muted),
                                            );
                                        });
                                } else {
                                    draw_element_section(
                                        ui,
                                        &t,
                                        theme,
                                        auto_contrast,
                                        paper_color,
                                        &user_saved,
                                        true,
                                        &mut place_id,
                                        &mut delete_id,
                                    );
                                }
                            }
                        });
                });
        });

    if let Some(id) = place_id {
        let center = app.camera_center_page_point();
        if let Ok(val) = app.session.execute(
            "element.place",
            &json!({
                "id": id,
                "x": center.x,
                "y": center.y,
            }),
        ) {
            crate::select_stamped_result(app, &val);
        }
        app.view.elements_popover_open = false;
        return;
    }
    if let Some(name) = save_selected_name {
        let targets = selected_targets_json(&app.view.selected_objects);
        let _ = app.session.execute(
            "element.save",
            &json!({
                "name": name,
                "targets": targets,
            }),
        );
        app.view.element_name_draft.clear();
    }
    if let Some(id) = delete_id {
        let _ = app.session.execute("element.delete", &json!({ "id": id }));
    }
    if close
        || (ctx.input(|i| i.pointer.any_pressed())
            && !area_resp.response.rect.contains(
                ctx.input(|i| i.pointer.interact_pos())
                    .unwrap_or(egui::Pos2::ZERO),
            ))
    {
        app.view.elements_popover_open = false;
    }
}

const SMOOTHING_QUICK_LEVELS: [(&str, f32, &str); 4] = [
    (
        "Off",
        0.0,
        "Disable smoothing (1:1 direct tracking, zero corner lag)",
    ),
    (
        "Low",
        0.25,
        "Light smoothing for fast handwriting with crisp turns",
    ),
    ("Med", 0.50, "Balanced smoothing for everyday notes"),
    (
        "High",
        0.80,
        "Strong smoothing for deliberate curves and precision lines",
    ),
];

const SMOOTHING_TRACK_H: f32 = 12.0;
const SMOOTHING_THUMB_R: f32 = 7.2;
const SMOOTHING_THUMB_HOVER_R: f32 = 8.2;

#[derive(Clone, Copy, Default)]
struct SmoothingSliderDragState {
    active: bool,
    needs_persist: bool,
}

fn smoothing_value_label(strength: f32) -> String {
    let clean = if strength.is_finite() {
        strength.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if clean <= SmoothingConfig::SMOOTHING_OFF_EPSILON {
        "Off".to_string()
    } else {
        let pct = (clean * 100.0).round().clamp(1.0, 100.0) as u8;
        format!("{pct}%")
    }
}

fn apply_smoothing_strength(app: &mut PaperCraftApp, kind: PenKind, strength: f32, persist: bool) {
    let clean = if strength.is_finite() {
        strength.clamp(0.0, 1.0)
    } else {
        SmoothingConfig::DEFAULT_SMOOTHING_STRENGTH
    };
    let _ = app.session.execute(
        "pen.setSmoothing",
        &json!({
            "kind": kind,
            "preset": StabilizationPreset::ArithmeticBuffer10,
            "stabilizer_strength": clean,
            "persist": persist,
        }),
    );
}

fn draw_smoothing_quick_switch(
    ui: &mut egui::Ui,
    id_salt: &str,
    strength: f32,
    t: &Tokens,
) -> Option<f32> {
    let s = if strength.is_finite() {
        strength.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let active_idx = if s <= SmoothingConfig::SMOOTHING_OFF_EPSILON {
        Some(0)
    } else {
        SMOOTHING_QUICK_LEVELS
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, (_, val, _))| (s - *val).abs() <= 0.04)
            .map(|(idx, _)| idx)
    };
    let options: Vec<SwitchOption<'_>> = SMOOTHING_QUICK_LEVELS
        .iter()
        .map(|(label, _, tip)| SwitchOption::new(None, Some(*label), tip, true))
        .collect();
    let (clicked, _) = segmented_switch(
        ui,
        id_salt,
        &options,
        active_idx,
        SwitchLayout::EqualWidth,
        t,
    );
    clicked.and_then(|idx| SMOOTHING_QUICK_LEVELS.get(idx).map(|(_, val, _)| *val))
}

fn update_smoothing_slider_drag(
    ui: &egui::Ui,
    resp: &egui::Response,
    drag_id: egui::Id,
    clean_cur: f32,
    travel_min_x: f32,
    travel_w: f32,
) -> Option<(f32, bool)> {
    let pos_to_val = |pos: egui::Pos2| -> f32 {
        let frac = ((pos.x - travel_min_x) / travel_w).clamp(0.0, 1.0);
        ((frac * 100.0).round() / 100.0).clamp(0.0, 1.0)
    };
    let mut state = ui
        .ctx()
        .data(|d| d.get_temp::<SmoothingSliderDragState>(drag_id))
        .unwrap_or_default();
    let pointer_pos = resp
        .interact_pointer_pos()
        .or_else(|| ui.input(|i| i.pointer.interact_pos()));
    let mut emission = None;

    if resp.dragged_by(egui::PointerButton::Primary) {
        state.active = true;
        if let Some(pos) = pointer_pos {
            let next = pos_to_val(pos);
            if (next - clean_cur).abs() >= 0.005 {
                state.needs_persist = true;
                emission = Some((next, false));
            }
        }
    } else if state.active {
        let final_val = pointer_pos.map(pos_to_val).unwrap_or(clean_cur);
        if state.needs_persist || (final_val - clean_cur).abs() >= 0.005 {
            emission = Some((final_val, true));
        }
        state = SmoothingSliderDragState::default();
    } else if resp.clicked_by(egui::PointerButton::Primary)
        && let Some(pos) = pointer_pos
    {
        let next = pos_to_val(pos);
        if (next - clean_cur).abs() >= 0.005 {
            emission = Some((next, true));
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(drag_id, state));
    emission
}

fn paint_smoothing_slider_readout(
    ctx: &egui::Context,
    id: egui::Id,
    thumb_center: egui::Pos2,
    label: &str,
    t: &Tokens,
) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, id));
    let galley = painter.layout_no_wrap(label.to_string(), FontId::proportional(11.0), t.text);
    let pill_size = galley.size() + Vec2::new(12.0, 6.0);
    let screen = ctx.content_rect().shrink(4.0);
    let left = (thumb_center.x - pill_size.x * 0.5).clamp(
        screen.left(),
        (screen.right() - pill_size.x).max(screen.left()),
    );
    let top = (thumb_center.y + 14.0).clamp(
        screen.top(),
        (screen.bottom() - pill_size.y).max(screen.top()),
    );
    let pill_rect = egui::Rect::from_min_size(egui::pos2(left, top), pill_size);
    painter.rect(
        pill_rect,
        CornerRadius::same(6),
        t.field_bg,
        Stroke::new(1.0, t.border),
        StrokeKind::Inside,
    );
    painter.galley(
        egui::pos2(
            pill_rect.center().x - galley.size().x * 0.5,
            pill_rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        t.text,
    );
}

fn draw_smoothing_strength_slider(
    ui: &mut egui::Ui,
    id_salt: impl egui::AsIdSalt,
    current_strength: f32,
    width: f32,
    t: &Tokens,
) -> Option<(f32, bool)> {
    let clean_cur = if current_strength.is_finite() {
        current_strength.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(width.max(80.0), 26.0), Sense::click_and_drag());
    let track_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 4.0, rect.center().y - SMOOTHING_TRACK_H * 0.5),
        egui::pos2(
            rect.right() - 4.0,
            rect.center().y + SMOOTHING_TRACK_H * 0.5,
        ),
    );
    let cap_pad = SMOOTHING_TRACK_H * 0.5;
    let travel_min_x = track_rect.left() + cap_pad;
    let travel_max_x = (track_rect.right() - cap_pad).max(travel_min_x + 1.0);
    let travel_w = (travel_max_x - travel_min_x).max(1.0);

    let drag_id = ui.make_persistent_id(id_salt);
    let emission =
        update_smoothing_slider_drag(ui, &resp, drag_id, clean_cur, travel_min_x, travel_w);
    let display_val = emission.map_or(clean_cur, |(v, _)| v);
    let is_dragging = resp.dragged_by(egui::PointerButton::Primary);
    let anim_frac = ui
        .ctx()
        .animate_value_with_time(
            drag_id.with("frac"),
            display_val,
            if is_dragging {
                0.0
            } else {
                crate::motion::MICRO_DURATION_S
            },
        )
        .clamp(0.0, 1.0);
    let hover_t = crate::motion::hover_factor(
        ui.ctx(),
        drag_id.with("hov"),
        resp.hovered() && !is_dragging,
    );
    let drag_t = crate::motion::hover_factor(ui.ctx(), drag_id.with("act"), is_dragging);
    let active_t = hover_t.max(drag_t);

    let thumb_cx = travel_min_x + anim_frac * travel_w;
    let thumb_center = egui::pos2(thumb_cx, track_rect.center().y);
    if ui.is_rect_visible(track_rect) {
        let track_bg = if t.is_dark {
            t.panel_bg
        } else {
            crate::motion::lerp_color(t.canvas_bg, t.border, 0.28)
        };
        let fill_color = if display_val <= SmoothingConfig::SMOOTHING_OFF_EPSILON {
            t.text_muted
        } else {
            crate::motion::lerp_color(t.accent, t.accent_hover, active_t)
        };
        let track_border =
            crate::motion::lerp_color(t.border, t.accent.gamma_multiply(0.65), active_t);
        let p = ui.painter();
        let cr = CornerRadius::same(6);
        p.rect_filled(track_rect, cr, track_bg);
        if display_val > SmoothingConfig::SMOOTHING_OFF_EPSILON {
            let fill_right = (thumb_cx + cap_pad).clamp(
                (track_rect.left() + SMOOTHING_TRACK_H).min(track_rect.right()),
                track_rect.right(),
            );
            p.rect_filled(
                egui::Rect::from_min_max(track_rect.min, egui::pos2(fill_right, track_rect.max.y)),
                cr,
                fill_color,
            );
        }
        p.rect_stroke(
            track_rect,
            cr,
            Stroke::new(1.0, track_border),
            StrokeKind::Inside,
        );

        let thumb_r = crate::motion::lerp_f32(SMOOTHING_THUMB_R, SMOOTHING_THUMB_HOVER_R, active_t);
        if active_t > 0.01 {
            p.circle_filled(
                thumb_center,
                thumb_r + 2.5 * active_t,
                t.accent.gamma_multiply(0.18 * active_t),
            );
        }
        p.circle(
            thumb_center,
            thumb_r,
            t.field_bg,
            Stroke::new(2.2, fill_color),
        );
    }

    let label = smoothing_value_label(display_val);
    let resp = resp.on_hover_cursor(if is_dragging {
        egui::CursorIcon::Grabbing
    } else {
        egui::CursorIcon::PointingHand
    });
    if is_dragging {
        paint_smoothing_slider_readout(ui.ctx(), drag_id.with("readout"), thumb_center, &label, t);
    } else {
        resp.on_hover_text(format!(
            "Stroke smoothing ({label}) — drag or click to adjust"
        ));
    }
    emission
}

pub(crate) fn show_smoothing_popover(app: &mut PaperCraftApp, ui: &mut egui::Ui) {
    if !app.view.smoothing_popover_open {
        return;
    }
    let ToolKind::Pen(kind) = app.session.active_tool else {
        app.view.smoothing_popover_open = false;
        return;
    };
    if app.view.shape_insert_mode || app.view.text_insert_mode || app.view.text_editing {
        app.view.smoothing_popover_open = false;
        return;
    }
    let ctx = ui.ctx().clone();
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.view.smoothing_popover_open = false;
        return;
    }

    let t = Tokens::for_theme(app.session.prefs.theme);
    let sm = app.session.pen(kind).smoothing.clone();
    let eff_strength = sm.effective_strength();
    let is_off = sm.is_smoothing_disabled();
    let value_label = smoothing_value_label(eff_strength);

    let viewport = ctx.content_rect();
    let pop_w = 264.0_f32;
    let anchor = app.view.smoothing_popover_anchor;
    let pop_x = anchor
        .map_or(viewport.center().x - pop_w * 0.5, |r| {
            r.center().x - pop_w * 0.5
        })
        .clamp(8.0, (viewport.right() - pop_w - 8.0).max(8.0));
    let pop_y = anchor
        .map_or(92.0, |r| r.bottom() + 6.0)
        .clamp(8.0, (viewport.bottom() - 144.0).max(8.0));

    let mut next_strength: Option<(f32, bool)> = None;

    let area_resp = egui::Area::new(ui.make_persistent_id("papercraft_smoothing_popover"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(pop_x, pop_y))
        .default_size(Vec2::new(pop_w, 134.0))
        .constrain_to(viewport.shrink(6.0))
        .fade_in(false)
        .show(&ctx, |ui| {
            egui::Frame::new()
                .fill(t.panel_bg)
                .stroke(Stroke::new(1.0, t.border))
                .corner_radius(CornerRadius::same(10))
                .shadow(egui::Shadow {
                    offset: [0, 6],
                    blur: 18,
                    spread: 0,
                    color: Color32::from_black_alpha(if t.is_dark { 110 } else { 32 }),
                })
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(pop_w - 24.0);
                    ui.horizontal(|ui| {
                        let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                        paint_icon(ui.painter(), ir, Icon::Stabilizer, t.accent);
                        ui.label(
                            egui::RichText::new("Stroke Smoothing")
                                .size(13.0)
                                .strong()
                                .color(t.text),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            egui::Frame::new()
                                .fill(if is_off { t.field_bg } else { t.accent_soft })
                                .stroke(Stroke::new(1.0, if is_off { t.border } else { t.accent }))
                                .corner_radius(CornerRadius::same(5))
                                .inner_margin(egui::Margin::symmetric(7, 2))
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new(&value_label)
                                            .size(11.0)
                                            .strong()
                                            .monospace()
                                            .color(if is_off { t.text_muted } else { t.accent }),
                                    );
                                });
                        });
                    });

                    ui.add_space(6.0);
                    if let Some(val) = draw_smoothing_quick_switch(
                        ui,
                        "toolbar_smoothing_quick_levels",
                        eff_strength,
                        &t,
                    ) {
                        next_strength = Some((val, true));
                    }

                    ui.add_space(4.0);
                    let slider_w = ui.available_width();
                    if let Some((val, persist)) = draw_smoothing_strength_slider(
                        ui,
                        ("toolbar_smoothing_strength_slider", kind),
                        eff_strength,
                        slider_w,
                        &t,
                    ) {
                        next_strength = Some((val, persist));
                    }

                    ui.add_space(3.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(
                                papercraft_doc::SmoothingConfig::strength_guidance(eff_strength),
                            )
                            .size(11.0)
                            .color(t.text_muted),
                        )
                        .wrap(),
                    );
                });
        });

    if let Some((val, persist)) = next_strength {
        apply_smoothing_strength(app, kind, val, persist);
    }

    let pressed_pos = ctx.input(|i| {
        if i.pointer.any_pressed() {
            i.pointer.interact_pos()
        } else {
            None
        }
    });
    if let Some(pos) = pressed_pos {
        let in_popover = area_resp.response.rect.contains(pos);
        let in_anchor = anchor.is_some_and(|r| r.contains(pos));
        if !in_popover && !in_anchor {
            app.view.smoothing_popover_open = false;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_element_section(
    ui: &mut egui::Ui,
    t: &Tokens,
    theme: papercraft_doc::UiTheme,
    auto_contrast: bool,
    paper_color: papercraft_doc::PaperColor,
    elements: &[SavedElement],
    allow_delete: bool,
    place_id: &mut Option<String>,
    delete_id: &mut Option<String>,
) {
    let cols = 3_usize;
    let card_w = 108.0_f32;
    let gap = 8.0_f32;
    for chunk in elements.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for el in chunk {
                let preview_h = 58.0_f32;
                let total_h = 86.0_f32;
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(card_w, total_h), Sense::click());
                let hov =
                    crate::motion::hover_factor(ui.ctx(), resp.id.with("hov"), resp.hovered());
                let fill = crate::motion::lerp_color(t.field_bg, t.accent_soft, hov * 0.45);
                let border_c = crate::motion::lerp_color(t.border, t.accent, hov);
                if ui.is_rect_visible(rect) {
                    let p = ui.painter();
                    p.rect_filled(rect, CornerRadius::same(8), fill);
                    p.rect_stroke(
                        rect,
                        CornerRadius::same(8),
                        Stroke::new(1.0, border_c),
                        StrokeKind::Inside,
                    );
                    let preview_rect = egui::Rect::from_min_size(
                        rect.min + Vec2::new(6.0, 6.0),
                        Vec2::new(card_w - 12.0, preview_h),
                    );
                    let tray_fill = crate::motion::lerp_color(t.field_bg, t.canvas_bg, 0.35);
                    p.rect_filled(preview_rect, CornerRadius::same(6), tray_fill);
                    paint_element_preview(
                        ui,
                        preview_rect,
                        el,
                        t,
                        theme,
                        auto_contrast,
                        paper_color,
                    );
                    let label_rect = egui::Rect::from_min_max(
                        egui::pos2(rect.left() + 6.0, rect.top() + preview_h + 6.0),
                        egui::pos2(
                            rect.right() - if allow_delete { 24.0 } else { 6.0 },
                            rect.bottom() - 4.0,
                        ),
                    );
                    let galley = p.layout(
                        el.name.clone(),
                        FontId::proportional(11.0),
                        t.text,
                        label_rect.width().max(12.0),
                    );
                    p.galley(
                        egui::pos2(
                            label_rect.left(),
                            label_rect.center().y - galley.size().y * 0.5,
                        ),
                        galley,
                        t.text,
                    );
                }
                if allow_delete {
                    let del_rect = egui::Rect::from_center_size(
                        egui::pos2(rect.right() - 13.0, rect.bottom() - 13.0),
                        Vec2::splat(18.0),
                    );
                    let del_resp = ui.interact(del_rect, resp.id.with("del"), Sense::click());
                    if ui.is_rect_visible(del_rect) {
                        paint_icon(
                            ui.painter(),
                            egui::Rect::from_center_size(del_rect.center(), Vec2::splat(12.0)),
                            Icon::Trash,
                            if del_resp.hovered() {
                                t.danger_fg
                            } else {
                                t.text_muted
                            },
                        );
                    }
                    if del_resp
                        .on_hover_text(format!("Delete saved sticker \"{}\"", el.name))
                        .clicked()
                    {
                        *delete_id = Some(el.id.clone());
                    } else if resp
                        .on_hover_text(format!("Place \"{}\" onto current page", el.name))
                        .clicked()
                    {
                        *place_id = Some(el.id.clone());
                    }
                } else if resp
                    .on_hover_text(format!("Place \"{}\" onto current page", el.name))
                    .clicked()
                {
                    *place_id = Some(el.id.clone());
                }
            }
        });
        ui.add_space(6.0);
    }
}

fn paint_element_preview(
    ui: &egui::Ui,
    rect: egui::Rect,
    el: &SavedElement,
    t: &Tokens,
    theme: papercraft_doc::UiTheme,
    auto_contrast: bool,
    paper_color: papercraft_doc::PaperColor,
) {
    let painter = ui.painter();
    let w = el.width.max(8.0) as f32;
    let h = el.height.max(8.0) as f32;
    let pad = 4.0_f32;
    let avail_w = (rect.width() - pad * 2.0).max(8.0);
    let avail_h = (rect.height() - pad * 2.0).max(8.0);
    let scale = (avail_w / w).min(avail_h / h);
    let ox = rect.center().x - w * scale * 0.5;
    let oy = rect.center().y - h * scale * 0.5;
    let to_pos = |pt: Point| egui::pos2(ox + (pt.x as f32) * scale, oy + (pt.y as f32) * scale);

    if !el.images.is_empty() {
        let clipped = painter.with_clip_rect(rect);
        for image in &el.images {
            let clean = image.sanitized();
            let Some(handle) =
                crate::canvas::cached_page_image_texture(ui.ctx(), "papercraft_elem_prev", image)
            else {
                continue;
            };
            let corners_page = clean.corners();
            let corners = [
                to_pos(corners_page[0]),
                to_pos(corners_page[1]),
                to_pos(corners_page[2]),
                to_pos(corners_page[3]),
            ];
            crate::canvas::paint_textured_image_quad(&clipped, handle.id(), corners);
        }
    }

    for shape in &el.shapes {
        let shape = shape.sanitized();
        let stroke_c =
            to_color32(paper_color.resolve_ink_color(theme, auto_contrast, shape.color, false));
        let fill_c = shape
            .fill_color
            .map(|fc| to_color32(paper_color.resolve_ink_color(theme, auto_contrast, fc, false)));
        let sw = (shape.width * scale).clamp(1.0, 2.5);
        for (points, closed) in shape.path_segments() {
            if points.len() < 2 {
                continue;
            }
            let pts: Vec<egui::Pos2> = points.into_iter().map(to_pos).collect();
            if closed && let Some(fc) = fill_c {
                crate::canvas::paint_polygon_fill(painter, &pts, fc);
            }
            if shape.color.a > 0.0 && stroke_c.a() > 0 {
                crate::canvas::paint_polyline_with_style(
                    painter,
                    &pts,
                    closed,
                    sw,
                    stroke_c,
                    shape.line_style,
                );
            }
        }
    }
    for stroke in &el.strokes {
        let is_hl = stroke.pen.kind == PenKind::Highlighter;
        let c = to_color32(paper_color.resolve_ink_color(
            theme,
            auto_contrast,
            stroke.pen.color,
            is_hl,
        ));
        let sw = (stroke.pen.size * scale).clamp(1.0, 3.0);
        if stroke.smoothed_points.len() >= 2 {
            let pts: Vec<egui::Pos2> = stroke
                .smoothed_points
                .iter()
                .map(|p| to_pos(p.point()))
                .collect();
            painter.add(egui::Shape::line(pts, Stroke::new(sw, c)));
        }
    }
    for txt in &el.texts {
        let c = to_color32(paper_color.resolve_ink_color(theme, auto_contrast, txt.color, false));
        let first_line = txt.text.lines().next().unwrap_or("");
        if !first_line.is_empty() {
            let preview_txt: String = first_line.chars().take(12).collect();
            let font_sz = (txt.size * scale).clamp(7.5, 11.5);
            painter.text(
                to_pos(Point::new(txt.x, txt.y)),
                egui::Align2::LEFT_TOP,
                preview_txt,
                FontId::proportional(font_sz),
                c,
            );
        } else if el.shapes.is_empty() && el.strokes.is_empty() && el.images.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Text",
                FontId::proportional(10.0),
                t.text_muted,
            );
        }
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

    #[test]
    fn highlighter_toolbar_stays_single_row_at_1280() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        let _ = app
            .session
            .execute("tool.select", &json!({ "tool": "Highlighter" }));

        let ctx = egui::Context::default();
        let mut rect = egui::Rect::NOTHING;
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1280.0, 800.0),
                )),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                rect = show_inner(&mut app, ui);
            });
            out.drop_without_applying_deltas();
        }
        assert!(
            rect.height() <= 56.0,
            "expected single-row highlighter toolbar at width 1280, got height {:.1}",
            rect.height()
        );
    }

    #[test]
    fn toolbar_uses_single_row_at_1280_and_hides_preset_dropdown() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);

        for width in [1280.0_f32, 1440.0, 1920.0] {
            let ctx = egui::Context::default();
            let mut rect = egui::Rect::NOTHING;
            let mut last_texts = String::new();
            for _ in 0..2 {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(width, 800.0),
                    )),
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    rect = show_inner(&mut app, ui);
                });
                last_texts = collect_text_shapes(&out.shapes);
                out.drop_without_applying_deltas();
            }
            assert!(
                rect.height() >= 48.0 && rect.height() <= 56.0,
                "expected enlarged single-row toolbar at width {width}, got height {:.1}",
                rect.height()
            );
            assert!(
                !last_texts.contains("Everyday Fountain"),
                "top toolbar must not show preset selector dropdown, rendered text:\n{last_texts}"
            );
            assert!(
                !last_texts.contains("1/1"),
                "top toolbar must not show inline page counter, rendered text:\n{last_texts}"
            );
            assert!(!last_texts.contains("Library"));
            assert!(!last_texts.contains("Settings"));
            assert!(!last_texts.contains(&app.session.doc.title));
        }

        // Narrow window gracefully wraps instead of clipping off-screen.
        let ctx = egui::Context::default();
        let mut narrow_rect = egui::Rect::NOTHING;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(840.0, 560.0),
            )),
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            crate::theme::apply_light_visuals(ui.ctx());
            narrow_rect = show_inner(&mut app, ui);
        });
        assert!(narrow_rect.height() > 56.0);
        assert!(narrow_rect.width() <= 840.5);
        out.drop_without_applying_deltas();
    }

    #[test]
    fn notebook_edge_button_is_icon_only_regardless_of_title_length() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);

        let measure_toolbar = |app: &mut PaperCraftApp| -> (egui::Rect, Vec<String>) {
            let ctx = egui::Context::default();
            let mut rect = egui::Rect::NOTHING;
            let mut rendered_rows = Vec::new();
            for _ in 0..2 {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1280.0, 800.0),
                    )),
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    rect = show_inner(app, ui);
                });
                rendered_rows.clear();
                fn collect_rows(shape: &egui::Shape, out: &mut Vec<String>) {
                    match shape {
                        egui::Shape::Text(text) => {
                            let s: String = text
                                .galley
                                .rows
                                .iter()
                                .flat_map(|r| r.row.glyphs.iter().map(|g| g.chr))
                                .collect();
                            out.push(s);
                        }
                        egui::Shape::Vec(items) => {
                            for s in items {
                                collect_rows(s, out);
                            }
                        }
                        _ => {}
                    }
                }
                for clipped in &out.shapes {
                    collect_rows(&clipped.shape, &mut rendered_rows);
                }
                out.drop_without_applying_deltas();
            }
            (rect, rendered_rows)
        };

        app.session
            .execute("notebook.rename", &json!({ "title": "Math" }))
            .unwrap();
        let (short_rect, short_rows) = measure_toolbar(&mut app);
        assert!(short_rect.height() <= 56.0);
        assert!(!short_rows.iter().any(|row| row == "Math"));

        app.session
            .execute(
                "notebook.rename",
                &json!({
                    "title": "Mathsssssssssssssssssssssssssssssssssssssssssssssssssssssssss"
                }),
            )
            .unwrap();
        let (long_rect, long_rows) = measure_toolbar(&mut app);
        assert!(long_rect.height() <= 56.0);
        assert!(!long_rows.iter().any(|row| row.starts_with("Math")));
    }

    #[test]
    fn sidebar_edge_buttons_are_flush_and_notebook_precedes_library() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.session
            .execute("notebook.rename", &json!({ "title": "Calculus Notes" }))
            .unwrap();

        let render_toolbar = |app: &mut PaperCraftApp, width: f32, height: f32| {
            let ctx = egui::Context::default();
            let mut header_rect = egui::Rect::NOTHING;
            let mut texts: Vec<(String, egui::Rect)> = Vec::new();
            let mut edge_tabs: Vec<egui::Rect> = Vec::new();
            let mut square_btns: Vec<egui::Rect> = Vec::new();
            for step in 0..2 {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(width, height),
                    )),
                    time: Some(f64::from(step) * 0.05),
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    header_rect = show_inner(app, ui);
                });
                texts.clear();
                edge_tabs.clear();
                square_btns.clear();
                fn scan(
                    shape: &egui::Shape,
                    header: egui::Rect,
                    texts: &mut Vec<(String, egui::Rect)>,
                    tabs: &mut Vec<egui::Rect>,
                    squares: &mut Vec<egui::Rect>,
                ) {
                    match shape {
                        egui::Shape::Text(text) => {
                            let r = text.visual_bounding_rect();
                            if header.contains(r.center()) {
                                texts.push((text.galley.text().to_string(), r));
                            }
                        }
                        egui::Shape::Rect(rs)
                            if (rs.rect.height() - TOOLBAR_ROW_HEIGHT).abs() <= 0.5
                                && (rs.corner_radius.ne >= 14 || rs.corner_radius.nw >= 14) =>
                        {
                            tabs.push(rs.rect);
                        }
                        egui::Shape::Rect(rs)
                            if (rs.rect.height() - TOOLBAR_ROW_HEIGHT).abs() <= 0.5
                                && (rs.rect.width() - TOOLBAR_ROW_HEIGHT).abs() <= 0.5 =>
                        {
                            squares.push(rs.rect);
                        }
                        egui::Shape::Vec(items) => {
                            for item in items {
                                scan(item, header, texts, tabs, squares);
                            }
                        }
                        _ => {}
                    }
                }
                for clipped in &out.shapes {
                    scan(
                        &clipped.shape,
                        header_rect,
                        &mut texts,
                        &mut edge_tabs,
                        &mut square_btns,
                    );
                }
                out.drop_without_applying_deltas();
            }
            (ctx, header_rect, texts, edge_tabs, square_btns)
        };

        let (ctx, header_rect, texts, edge_tabs, square_btns) =
            render_toolbar(&mut app, 1280.0, 800.0);
        assert!(
            !texts.iter().any(|(label, _)| matches!(
                label.as_str(),
                "Calculus Notes" | "Library" | "Settings"
            )),
            "expected icon-only toolbar buttons without text labels, got {texts:?}"
        );

        let left_tab = edge_tabs
            .iter()
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .copied()
            .expect("left sidebar edge tab rect");
        let right_tab = edge_tabs
            .iter()
            .max_by(|a, b| a.right().total_cmp(&b.right()))
            .copied()
            .expect("right sidebar edge tab rect");
        let lib_rect = square_btns
            .iter()
            .filter(|r| r.left() > left_tab.right())
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .copied()
            .expect("Library icon button right of left edge tab");
        assert!(
            left_tab.center().x < lib_rect.center().x,
            "expected Notebook edge button ({:.1}) to the left of Library ({:.1})",
            left_tab.center().x,
            lib_rect.center().x
        );
        assert!(
            (left_tab.left() - header_rect.left()).abs() <= 2.5,
            "expected left edge tab flush to header left ({:.1}), got {:.1}",
            header_rect.left(),
            left_tab.left()
        );
        assert!(
            (right_tab.right() - header_rect.right()).abs() <= 2.5,
            "expected right edge tab flush to header right ({:.1}), got {:.1}",
            header_rect.right(),
            right_tab.right()
        );

        let click_at = |pt: egui::Pos2, app: &mut PaperCraftApp| {
            for (pressed, time) in [(true, 0.20), (false, 0.25)] {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1280.0, 800.0),
                    )),
                    time: Some(time),
                    events: vec![
                        egui::Event::PointerMoved(pt),
                        egui::Event::PointerButton {
                            pos: pt,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    let _ = show_inner(app, ui);
                });
                out.drop_without_applying_deltas();
            }
        };

        let was_pages_open = app.view.pages_sidebar_open;
        click_at(left_tab.center(), &mut app);
        assert_ne!(app.view.pages_sidebar_open, was_pages_open);

        let was_inspector_open = app.view.inspector_open;
        click_at(right_tab.center(), &mut app);
        assert_ne!(app.view.inspector_open, was_inspector_open);

        // Narrow viewport wraps below, while top row keeps edge buttons flush left/right.
        let (_, narrow_header, _, narrow_tabs, narrow_squares) =
            render_toolbar(&mut app, 840.0, 560.0);
        assert!(narrow_header.height() > 56.0);
        let narrow_left = narrow_tabs
            .iter()
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .copied()
            .expect("narrow left edge tab");
        let narrow_right = narrow_tabs
            .iter()
            .max_by(|a, b| a.right().total_cmp(&b.right()))
            .copied()
            .expect("narrow right edge tab");
        let narrow_lib = narrow_squares
            .iter()
            .filter(|r| r.top() < narrow_left.bottom() && r.left() > narrow_left.right())
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .copied()
            .expect("narrow Library icon button");
        assert!(narrow_left.center().x < narrow_lib.center().x);
        assert!((narrow_left.left() - narrow_header.left()).abs() <= 2.5);
        assert!((narrow_right.right() - narrow_header.right()).abs() <= 2.5);
    }

    #[test]
    fn context_tray_shows_text_controls_including_line_and_letter_spacing_during_editing() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        crate::activate_select_tool(&mut app);
        app.view.text_insert_mode = true;
        app.view.text_editing = true;
        app.view.text_draft = "Sample".to_string();

        let ctx = egui::Context::default();
        let mut rendered = String::new();
        for frame in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1280.0, 800.0),
                )),
                time: Some(f64::from(frame) * 0.05 + 0.1),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                show(&mut app, ui);
            });
            rendered = collect_text_shapes(&out.shapes);
            out.drop_without_applying_deltas();
        }
        assert!(rendered.contains("Font"), "rendered:\n{rendered}");
        assert!(rendered.contains("Size"), "rendered:\n{rendered}");
        assert!(rendered.contains("Line "), "rendered:\n{rendered}");
        assert!(rendered.contains("1.20"), "rendered:\n{rendered}");
        assert!(rendered.contains("Letter "), "rendered:\n{rendered}");
        assert!(rendered.contains("0.0"), "rendered:\n{rendered}");
    }

    fn find_text_center_y(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<f32> {
        fn visit(shape: &egui::epaint::Shape, needle: &str) -> Option<f32> {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.text().contains(needle) => {
                    Some(text.pos.y + text.galley.size().y * 0.5)
                }
                egui::epaint::Shape::Vec(children) => {
                    children.iter().find_map(|child| visit(child, needle))
                }
                _ => None,
            }
        }
        shapes.iter().find_map(|c| visit(&c.shape, needle))
    }

    #[test]
    fn context_tray_text_and_link_controls_vertically_align_single_and_wrapped() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        crate::activate_select_tool(&mut app);
        app.view.text_insert_mode = true;
        app.view.text_editing = true;
        app.view.text_draft = "Sample".to_string();
        app.view.text_link_editor_open = true;

        let ctx = egui::Context::default();
        let mut wide_shapes = Vec::new();
        for frame in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1280.0, 800.0),
                )),
                time: Some(f64::from(frame) * 0.05 + 0.1),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                show(&mut app, ui);
            });
            wide_shapes = out.shapes.clone();
            out.drop_without_applying_deltas();
        }

        let wide_labels = [
            "Font",
            "Sans",
            "Size",
            "24",
            "Line ",
            "Letter ",
            "Link",
            "https://example.com",
            "Apply",
        ];
        let wide_ys: Vec<(&str, f32)> = wide_labels
            .iter()
            .map(|label| {
                let y = find_text_center_y(&wide_shapes, label).unwrap_or_else(|| {
                    panic!(
                        "expected wide tray text {label:?}; rendered:\n{}",
                        collect_text_shapes(&wide_shapes)
                    )
                });
                (*label, y)
            })
            .collect();
        let min_y = wide_ys
            .iter()
            .map(|(_, y)| *y)
            .fold(f32::INFINITY, f32::min);
        let max_y = wide_ys
            .iter()
            .map(|(_, y)| *y)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            max_y - min_y <= 1.5,
            "single-row tray text centers should align within 1.5px, got {wide_ys:?}"
        );

        // Also check wrapped tray layout at a narrower viewport so first-row and wrapped-row
        // items each share a crisp horizontal centerline.
        let mut narrow_shapes = Vec::new();
        for frame in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(560.0, 800.0),
                )),
                time: Some(f64::from(frame) * 0.05 + 0.4),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                show(&mut app, ui);
            });
            narrow_shapes = out.shapes.clone();
            out.drop_without_applying_deltas();
        }
        let font_y = find_text_center_y(&narrow_shapes, "Font").unwrap_or_else(|| {
            panic!(
                "Font label missing at 560px; rendered:\n{}",
                collect_text_shapes(&narrow_shapes)
            )
        });
        let sans_y = find_text_center_y(&narrow_shapes, "Sans").expect("Sans combo");
        let size_y = find_text_center_y(&narrow_shapes, "Size").expect("Size label");
        let px_y = find_text_center_y(&narrow_shapes, "24").expect("Size DragValue");
        assert!(
            (font_y - sans_y).abs() <= 1.5
                && (font_y - size_y).abs() <= 1.5
                && (size_y - px_y).abs() <= 1.5,
            "row 1 items should align: font={font_y}, sans={sans_y}, size={size_y}, px={px_y}"
        );
        let hint_y =
            find_text_center_y(&narrow_shapes, "https://example.com").expect("Link TextEdit hint");
        let apply_y = find_text_center_y(&narrow_shapes, "Apply").expect("Apply button");
        assert!(
            (hint_y - apply_y).abs() <= 1.5,
            "link TextEdit hint and Apply button should align: hint={hint_y}, apply={apply_y}"
        );
    }

    #[test]
    fn toolbar_hides_style_favorites_star_and_stays_single_row_at_1280() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);

        let count_toolbar_shapes = |app: &mut PaperCraftApp| -> (egui::Rect, usize) {
            let ctx = egui::Context::default();
            let mut rect = egui::Rect::NOTHING;
            let mut shape_count = 0;
            for frame in 0..2 {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1280.0, 800.0),
                    )),
                    time: Some(f64::from(frame) * 0.05),
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    rect = show_inner(app, ui);
                });
                shape_count = out.shapes.len();
                out.drop_without_applying_deltas();
            }
            (rect, shape_count)
        };

        let (base_rect, base_shapes) = count_toolbar_shapes(&mut app);
        assert!(base_rect.height() <= 56.0);

        let tools = [
            ("FountainPen", 2.2, "#141923"),
            ("BallPen", 4.5, "#1D4ED8"),
            ("BrushPen", 8.0, "#DC2626"),
            ("Highlighter", 24.0, "#FACC15"),
            ("Pencil", 2.8, "#334155"),
            ("FountainPen", 3.5, "#059669"),
            ("BallPen", 1.8, "#7C3AED"),
            ("Pencil", 5.0, "#0F172A"),
        ];
        for (tool, size, hex) in tools {
            app.session
                .execute("tool.select", &json!({ "tool": tool }))
                .unwrap();
            app.session
                .execute(
                    "pen.setSettings",
                    &json!({ "kind": tool, "size": size, "color_hex": hex }),
                )
                .unwrap();
            app.session
                .execute("styleFavorite.save", &json!({}))
                .unwrap();
        }
        assert_eq!(
            app.session.prefs.style_favorites.len(),
            papercraft_doc::MAX_STYLE_FAVORITES
        );

        let (fav_rect, fav_shapes) = count_toolbar_shapes(&mut app);
        assert!(
            fav_rect.height() <= 56.0,
            "expected single-row toolbar at 1280px, got height {:.1}",
            fav_rect.height()
        );
        assert_eq!(
            fav_shapes, base_shapes,
            "toolbar must not render style favorite chips or star pin button"
        );
    }

    fn find_tray_frame_rect(
        shapes: &[egui::epaint::ClippedShape],
        panel_bg: Color32,
        header_bottom: f32,
    ) -> Option<egui::Rect> {
        fn visit(
            shape: &egui::Shape,
            panel_bg: Color32,
            header_bottom: f32,
            best: &mut Option<egui::Rect>,
        ) {
            match shape {
                egui::Shape::Rect(rect_shape)
                    if rect_shape.fill == panel_bg
                        && rect_shape.corner_radius
                            == CornerRadius::same(CONTEXT_TRAY_CORNER_RADIUS)
                        && rect_shape.rect.top() >= header_bottom
                        && rect_shape.rect.top() <= header_bottom + 24.0
                        && rect_shape.rect.width() >= 80.0
                        && rect_shape.rect.height() >= 30.0
                        && rect_shape.rect.height() < 320.0 =>
                {
                    let replace = best.is_none_or(|cur| rect_shape.rect.area() > cur.area());
                    if replace {
                        *best = Some(rect_shape.rect);
                    }
                }
                egui::Shape::Vec(items) => {
                    for item in items {
                        visit(item, panel_bg, header_bottom, best);
                    }
                }
                _ => {}
            }
        }
        let mut best = None;
        for clipped in shapes {
            visit(&clipped.shape, panel_bg, header_bottom, &mut best);
        }
        best
    }

    fn collect_text_boxes_inside(
        shapes: &[egui::epaint::ClippedShape],
        bounds: egui::Rect,
    ) -> Vec<(String, egui::Rect)> {
        let mut out = Vec::new();
        fn visit(shape: &egui::Shape, bounds: egui::Rect, out: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(text) => {
                    let r = egui::Rect::from_min_size(text.pos, text.galley.size());
                    if bounds.intersects(r) && !text.galley.text().trim().is_empty() {
                        out.push((text.galley.text().to_string(), r));
                    }
                }
                egui::Shape::Vec(items) => {
                    for item in items {
                        visit(item, bounds, out);
                    }
                }
                _ => {}
            }
        }
        for clipped in shapes {
            visit(&clipped.shape, bounds, &mut out);
        }
        out
    }

    fn collect_tray_widget_bounds_by_row(
        shapes: &[egui::epaint::ClippedShape],
        tray_rect: egui::Rect,
    ) -> Vec<(f32, f32, f32)> {
        // Collect all interactive/visible sub-rects strictly inside the tray frame (excluding the
        // frame rect itself and hairline dividers) and cluster them into horizontal rows by center Y.
        let mut rects = Vec::new();
        fn visit(shape: &egui::Shape, tray_rect: egui::Rect, out: &mut Vec<egui::Rect>) {
            match shape {
                egui::Shape::Rect(rs)
                    if tray_rect.contains_rect(rs.rect.shrink(0.5))
                        && (rs.rect.width() - tray_rect.width()).abs() > 2.0
                        && rs.rect.width() >= 10.0
                        && rs.rect.height() >= 14.0 =>
                {
                    out.push(rs.rect);
                }
                egui::Shape::Circle(cs) if tray_rect.contains(cs.center) && cs.radius >= 5.0 => {
                    out.push(egui::Rect::from_center_size(
                        cs.center,
                        Vec2::splat(cs.radius * 2.0),
                    ));
                }
                egui::Shape::Text(text) => {
                    let r = egui::Rect::from_min_size(text.pos, text.galley.size());
                    if tray_rect.contains(r.center()) && !text.galley.text().trim().is_empty() {
                        out.push(r);
                    }
                }
                egui::Shape::Vec(items) => {
                    for item in items {
                        visit(item, tray_rect, out);
                    }
                }
                _ => {}
            }
        }
        for clipped in shapes {
            visit(&clipped.shape, tray_rect, &mut rects);
        }
        rects.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
        let mut rows: Vec<(f32, f32, f32)> = Vec::new(); // (center_y, min_x, max_x)
        for r in rects {
            let cy = r.center().y;
            if let Some(last) = rows.last_mut()
                && (cy - last.0).abs() <= 14.0
            {
                last.1 = last.1.min(r.left());
                last.2 = last.2.max(r.right());
            } else {
                rows.push((cy, r.left(), r.right()));
            }
        }
        rows
    }

    #[test]
    fn plan_dock_rows_balances_widths_and_suppresses_boundary_dividers() {
        let specs = vec![
            DockSpec {
                width: 120.0,
                break_before: BreakTier::Divider,
                has_divider_before: false,
            },
            DockSpec {
                width: 120.0,
                break_before: BreakTier::Inner,
                has_divider_before: false,
            },
            DockSpec {
                width: 120.0,
                break_before: BreakTier::Divider,
                has_divider_before: true,
            },
            DockSpec {
                width: 120.0,
                break_before: BreakTier::Inner,
                has_divider_before: false,
            },
            DockSpec {
                width: 120.0,
                break_before: BreakTier::Divider,
                has_divider_before: true,
            },
        ];

        // Fits on single row when max_w accommodates total width.
        let single = plan_dock_rows(&specs, 700.0, 2.0, 10.0);
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].start, 0);
        assert_eq!(single[0].end, 5);

        // Slight overflow (560 < 628) must split proportionally near the middle, NOT leave 1 orphan.
        let balanced = plan_dock_rows(&specs, 560.0, 2.0, 10.0);
        assert_eq!(balanced.len(), 2);
        assert_eq!(balanced[0].start, 0);
        assert_eq!(balanced[0].end, 2);
        assert_eq!(balanced[1].start, 2);
        assert_eq!(balanced[1].end, 5);
        // Divider before index 2 is suppressed when row 2 starts at index 2!
        assert!((balanced[0].width - 242.0).abs() < 0.01);
        assert!((balanced[1].width - 374.0).abs() < 0.01);
    }

    #[test]
    fn selected_text_dock_wraps_proportionally_and_centers_inside_canvas_with_open_sidebars() {
        for viewport_w in [1120.0_f32, 1240.0_f32] {
            let mut app = PaperCraftApp::default();
            crate::open_notebook_in_editor(&mut app, 0);
            app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
            app.view.pages_sidebar_open = true;
            app.view.inspector_open = true;
            app.session
                .execute(
                    "text.insert",
                    &json!({ "text": "Notebook Heading", "x": 120.0, "y": 500.0 }),
                )
                .unwrap();
            crate::activate_select_tool(&mut app);
            app.view.selected_object = Some(crate::SelectedObject::Text(0));
            app.view.selected_objects = vec![crate::SelectedObject::Text(0)];

            let ctx = egui::Context::default();
            let mut shapes = Vec::new();
            let mut tray_rect_opt = None;
            let t = Tokens::for_theme(app.session.prefs.theme);
            for frame in 0..4 {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(viewport_w, 860.0),
                    )),
                    time: Some(f64::from(frame) * 0.08 + 0.1),
                    predicted_dt: 0.08,
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
                shapes = out.shapes.clone();
                tray_rect_opt = find_tray_frame_rect(&shapes, t.panel_bg, 90.0);
                out.drop_without_applying_deltas();
            }

            let tray_rect = tray_rect_opt.expect("context tray frame rect");
            let canvas_left = crate::pages_sidebar::DEFAULT_SIDEBAR_WIDTH;
            let canvas_right = viewport_w - crate::inspector::DEFAULT_INSPECTOR_WIDTH;
            let canvas_center_x = (canvas_left + canvas_right) * 0.5;

            assert!(
                tray_rect.left() >= canvas_left + 10.0,
                "viewport {viewport_w}: tray {tray_rect:?} overlapped left sidebar ({canvas_left})"
            );
            assert!(
                tray_rect.right() <= canvas_right - 10.0,
                "viewport {viewport_w}: tray {tray_rect:?} overlapped right inspector ({canvas_right})"
            );
            assert!(
                (tray_rect.center().x - canvas_center_x).abs() <= 2.0,
                "viewport {viewport_w}: tray center {} should match canvas center {}",
                tray_rect.center().x,
                canvas_center_x
            );
            assert!(
                tray_rect.height() > 54.0,
                "viewport {viewport_w}: expected multi-row wrapped text dock, got height {:.1}",
                tray_rect.height()
            );

            let text_boxes = collect_text_boxes_inside(&shapes, tray_rect);
            assert!(
                text_boxes.iter().any(|(txt, _)| txt.contains("Edit text")),
                "expected Edit text button inside tray"
            );
            for hidden in ["Copy", "Cut", "Duplicate", "Save", "Paste", "Delete"] {
                assert!(
                    !text_boxes.iter().any(|(txt, _)| txt == hidden),
                    "did not expect {hidden:?} button inside selection tray"
                );
            }
            for (label, box_rect) in &text_boxes {
                assert!(
                    box_rect.left() >= tray_rect.left() + 4.0
                        && box_rect.right() <= tray_rect.right() - 4.0,
                    "viewport {viewport_w}: text {label:?} ({box_rect:?}) overflowed tray {tray_rect:?}"
                );
            }

            let rows = collect_tray_widget_bounds_by_row(&shapes, tray_rect);
            assert!(
                rows.len() >= 2,
                "viewport {viewport_w}: expected >= 2 visual rows in wrapped dock, got {rows:?}"
            );
            let mut min_row_w = f32::INFINITY;
            let mut max_row_w = 0.0_f32;
            let mut min_row_cx = f32::INFINITY;
            let mut max_row_cx = f32::NEG_INFINITY;
            for (cy, min_x, max_x) in &rows {
                let row_w = max_x - min_x;
                let row_center = (min_x + max_x) * 0.5;
                min_row_w = min_row_w.min(row_w);
                max_row_w = max_row_w.max(row_w);
                min_row_cx = min_row_cx.min(row_center);
                max_row_cx = max_row_cx.max(row_center);
                assert!(
                    (row_center - tray_rect.center().x).abs() <= 4.0,
                    "viewport {viewport_w}: row at y={cy:.1} [{min_x:.1}..{max_x:.1}] center {row_center:.1} not centered in tray center {:.1}",
                    tray_rect.center().x
                );
            }
            assert!(
                max_row_cx - min_row_cx <= 1.5,
                "viewport {viewport_w}: wrapped rows should share center axis within 1.5px, got span {:.2} in {rows:?}",
                max_row_cx - min_row_cx
            );
            assert!(
                min_row_w / max_row_w >= 0.60,
                "viewport {viewport_w}: expected proportional row widths, got min={min_row_w:.1}, max={max_row_w:.1}, rows={rows:?}"
            );
        }
    }

    #[test]
    fn compact_pen_dock_centers_over_canvas_when_only_left_sidebar_is_open() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
        app.view.pages_sidebar_open = true;
        app.view.inspector_open = false;

        let ctx = egui::Context::default();
        let t = Tokens::for_theme(app.session.prefs.theme);
        let mut tray_rect_opt = None;
        for frame in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1280.0, 800.0),
                )),
                time: Some(f64::from(frame) * 0.08 + 0.1),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
            tray_rect_opt = find_tray_frame_rect(&out.shapes, t.panel_bg, 90.0);
            out.drop_without_applying_deltas();
        }
        let tray_rect = tray_rect_opt.expect("pen tray rect");
        let canvas_center_x = (crate::pages_sidebar::DEFAULT_SIDEBAR_WIDTH + 1280.0) * 0.5;
        assert!(
            (tray_rect.center().x - canvas_center_x).abs() <= 2.0,
            "pen dock center {:.1} should match canvas center {:.1} (not viewport center 640.0)",
            tray_rect.center().x,
            canvas_center_x
        );
        assert!(
            (48.0..=54.0).contains(&tray_rect.height()),
            "expected proportional ~50px tray height, got {tray_rect:?}"
        );
    }

    #[test]
    fn shape_insert_dock_wraps_cleanly_even_on_narrow_canvas_with_open_sidebars() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
        app.view.pages_sidebar_open = true;
        app.view.inspector_open = true;
        app.view.shape_insert_mode = true;

        let ctx = egui::Context::default();
        let t = Tokens::for_theme(app.session.prefs.theme);
        let viewport_w = 1040.0_f32; // canvas = 1040 - 280 - 372 = 388px (wraps compactly with open sidebars)
        let mut tray_rect_opt = None;
        for frame in 0..4 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(viewport_w, 800.0),
                )),
                time: Some(f64::from(frame) * 0.08 + 0.1),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
            tray_rect_opt = find_tray_frame_rect(&out.shapes, t.panel_bg, 90.0);
            out.drop_without_applying_deltas();
        }
        let tray_rect = tray_rect_opt.expect("shape insert tray rect");
        let canvas_left = crate::pages_sidebar::DEFAULT_SIDEBAR_WIDTH;
        let canvas_right = viewport_w - crate::inspector::DEFAULT_INSPECTOR_WIDTH;
        assert!(
            tray_rect.left() >= canvas_left + 10.0 && tray_rect.right() <= canvas_right - 10.0,
            "narrow shape tray {tray_rect:?} must stay inside canvas [{canvas_left}..{canvas_right}]"
        );
        assert!(tray_rect.height() > 60.0);
    }

    #[test]
    fn activating_ruler_switches_from_non_pen_tools_to_last_used_pen() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);

        // 1. Default session starts on FountainPen; switching to Select then clicking Ruler
        //    returns directly to FountainPen.
        crate::activate_select_tool(&mut app);
        assert_eq!(app.session.active_tool, ToolKind::Select);

        let ctx = egui::Context::default();
        let mut ruler_btn_center = None;
        for frame in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1280.0, 800.0),
                )),
                time: Some(f64::from(frame) * 0.05),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                let _ = show_inner(&mut app, ui);
            });
            // Locate the 36x36 Ruler button immediately to the right of the 10-tool switch track.
            let mut switch_right = None;
            let mut btn_candidates = Vec::new();
            fn scan(
                shape: &egui::Shape,
                switch_right: &mut Option<f32>,
                btns: &mut Vec<egui::Rect>,
            ) {
                match shape {
                    egui::Shape::Rect(rs) if rs.rect.top() < 52.0 => {
                        if rs.rect.width() >= 250.0
                            && (rs.rect.height() - TOOLBAR_ROW_HEIGHT).abs() <= 1.0
                        {
                            *switch_right = Some(rs.rect.right());
                        } else if (rs.rect.width() - TOOLBAR_ROW_HEIGHT).abs() <= 1.0
                            && (rs.rect.height() - TOOLBAR_ROW_HEIGHT).abs() <= 1.0
                        {
                            btns.push(rs.rect);
                        }
                    }
                    egui::Shape::Vec(items) => {
                        for item in items {
                            scan(item, switch_right, btns);
                        }
                    }
                    _ => {}
                }
            }
            for clipped in &out.shapes {
                scan(&clipped.shape, &mut switch_right, &mut btn_candidates);
            }
            if let Some(sr) = switch_right {
                ruler_btn_center = btn_candidates
                    .into_iter()
                    .filter(|r| r.left() >= sr - 0.5)
                    .min_by(|a, b| a.left().total_cmp(&b.left()))
                    .map(|r| r.center());
            }
            out.drop_without_applying_deltas();
        }
        let ruler_pos = ruler_btn_center.expect("ruler button center in toolbar");

        for (pressed, time) in [(true, 0.2), (false, 0.25)] {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1280.0, 800.0),
                )),
                time: Some(time),
                events: vec![
                    egui::Event::PointerMoved(ruler_pos),
                    egui::Event::PointerButton {
                        pos: ruler_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                let _ = show_inner(&mut app, ui);
            });
            out.drop_without_applying_deltas();
        }
        assert!(app.view.ruler_active);
        assert_eq!(app.session.active_tool, ToolKind::Pen(PenKind::FountainPen));

        // Turn ruler off via toggle; active tool remains unchanged.
        toggle_ruler_tool(&mut app);
        assert!(!app.view.ruler_active);
        assert_eq!(app.session.active_tool, ToolKind::Pen(PenKind::FountainPen));

        // 2. Use BallPen, switch to Eraser / LaserPointer / Select / TextInsert, and confirm
        //    activating ruler switches back to BallPen every time.
        app.session
            .execute("tool.select", &json!({ "tool": "BallPen" }))
            .unwrap();
        for non_pen in ["Eraser", "LaserPointer", "Select"] {
            app.view.ruler_active = false;
            app.session
                .execute("tool.select", &json!({ "tool": non_pen }))
                .unwrap();
            toggle_ruler_tool(&mut app);
            assert!(app.view.ruler_active);
            assert_eq!(
                app.session.active_tool,
                ToolKind::Pen(PenKind::BallPen),
                "activating ruler from {non_pen} must switch to last used pen (BallPen)"
            );
        }

        // 3. Use Pencil via styleFavorite.apply, enter Select + TextInsert, then activate Ruler.
        app.view.ruler_active = false;
        app.session
            .execute("tool.select", &json!({ "tool": "Pencil" }))
            .unwrap();
        crate::activate_select_tool(&mut app);
        app.view.text_insert_mode = true;
        toggle_ruler_tool(&mut app);
        assert!(app.view.ruler_active);
        assert!(!app.view.text_insert_mode);
        assert_eq!(app.session.active_tool, ToolKind::Pen(PenKind::Pencil));

        // 4. When already on a pen (Highlighter), activating ruler keeps Highlighter active.
        app.view.ruler_active = false;
        app.session
            .execute("tool.select", &json!({ "tool": "Highlighter" }))
            .unwrap();
        toggle_ruler_tool(&mut app);
        assert!(app.view.ruler_active);
        assert_eq!(app.session.active_tool, ToolKind::Pen(PenKind::Highlighter));
    }

    #[test]
    fn select_mode_switch_is_interactive_when_empty_and_when_object_selected() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
        crate::activate_select_tool(&mut app);
        assert!(!app.view.freeform_lasso_mode);

        let ctx = egui::Context::default();
        let t = Tokens::for_theme(app.session.prefs.theme);
        let mut tray_rect = egui::Rect::NOTHING;
        for frame in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1280.0, 800.0),
                )),
                time: Some(f64::from(frame) * 0.08 + 0.1),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
            if let Some(r) = find_tray_frame_rect(&out.shapes, t.panel_bg, 90.0) {
                tray_rect = r;
            }
            out.drop_without_applying_deltas();
        }
        assert!(tray_rect.is_positive());

        // Click "Lasso" in empty select tray.
        let boxes = {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1280.0, 800.0),
                    )),
                    time: Some(0.35),
                    ..Default::default()
                },
                |ui| app.show_ui(ui),
            );
            let b = collect_text_boxes_inside(&out.shapes, tray_rect);
            out.drop_without_applying_deltas();
            b
        };
        let lasso_center = boxes
            .iter()
            .find(|(txt, _)| txt == "Lasso")
            .map(|(_, r)| r.center())
            .expect("expected Lasso button in select tray");
        let click_lasso = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1280.0, 800.0),
                )),
                time: Some(0.40),
                events: vec![
                    egui::Event::PointerMoved(lasso_center),
                    egui::Event::PointerButton {
                        pos: lasso_center,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::PointerButton {
                        pos: lasso_center,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ui| app.show_ui(ui),
        );
        click_lasso.drop_without_applying_deltas();
        assert!(
            app.view.freeform_lasso_mode,
            "clicking Lasso in select tray must enable freeform_lasso_mode"
        );

        // Now select a shape and verify the compact [Box | Lasso] toggle remains present & working.
        app.session
            .execute(
                "shape.insert",
                &json!({ "kind": "Rectangle", "x1": 100.0, "y1": 100.0, "x2": 220.0, "y2": 180.0 }),
            )
            .unwrap();
        app.view.selected_object = Some(crate::SelectedObject::Shape(0));
        app.view.selected_objects = vec![crate::SelectedObject::Shape(0)];
        let mut sel_tray = egui::Rect::NOTHING;
        let mut sel_shapes = Vec::new();
        let mut sel_boxes = Vec::new();
        for frame in 0..2 {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1280.0, 800.0),
                    )),
                    time: Some(0.48 + f64::from(frame) * 0.08),
                    predicted_dt: 0.08,
                    ..Default::default()
                },
                |ui| app.show_ui(ui),
            );
            if let Some(r) = find_tray_frame_rect(&out.shapes, t.panel_bg, 90.0) {
                sel_tray = r;
                sel_shapes = out.shapes.clone();
                sel_boxes = collect_text_boxes_inside(&out.shapes, sel_tray);
            }
            out.drop_without_applying_deltas();
        }
        assert!(sel_tray.is_positive());
        for hidden in ["Copy", "Cut", "Duplicate", "Save", "Paste", "Delete"] {
            assert!(
                !sel_boxes.iter().any(|(txt, _)| txt == hidden),
                "selection tray should omit {hidden:?}"
            );
        }
        let rows = collect_tray_widget_bounds_by_row(&sel_shapes, sel_tray);
        let (row_cy, _, row_max_x) = rows
            .last()
            .copied()
            .expect("expected widget rows in selection tray");
        // Compact [Box | Lasso] switch sits at the right end of the last row (~62px wide).
        let box_click_pos = egui::pos2(row_max_x - 45.0, row_cy);
        let click_box = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1280.0, 800.0),
                )),
                time: Some(0.70),
                events: vec![
                    egui::Event::PointerMoved(box_click_pos),
                    egui::Event::PointerButton {
                        pos: box_click_pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::PointerButton {
                        pos: box_click_pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ui| app.show_ui(ui),
        );
        click_box.drop_without_applying_deltas();
        assert!(
            !app.view.freeform_lasso_mode,
            "clicking Box in selection tray must switch back to Box marquee mode"
        );
    }

    #[test]
    fn selection_tray_omits_clipboard_and_delete_buttons_while_shortcuts_and_sticker_save_work() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
        app.session
            .execute(
                "shape.insert",
                &json!({ "kind": "Rectangle", "x1": 120.0, "y1": 140.0, "x2": 260.0, "y2": 240.0 }),
            )
            .unwrap();
        crate::activate_select_tool(&mut app);
        app.view.selected_object = Some(crate::SelectedObject::Shape(0));
        app.view.selected_objects = vec![crate::SelectedObject::Shape(0)];
        let ctx = egui::Context::default();
        assert!(crate::copy_selection(&mut app, Some(&ctx), false));
        assert!(app.session.clipboard.is_some());

        let t = Tokens::for_theme(app.session.prefs.theme);
        let mut tray_rect = egui::Rect::NOTHING;
        let mut tray_boxes = Vec::new();
        for frame in 0..3 {
            let (shapes, _) = step_editor_frame(
                &ctx,
                &mut app,
                Vec2::new(1280.0, 800.0),
                f64::from(frame) * 0.08 + 0.1,
                0.08,
                Vec::new(),
            );
            if let Some(r) = find_tray_frame_rect(&shapes, t.panel_bg, 90.0) {
                tray_rect = r;
                tray_boxes = collect_text_boxes_inside(&shapes, r);
            }
        }
        assert!(tray_rect.is_positive());
        assert!(
            tray_boxes.iter().any(|(txt, _)| txt == "Rectangle"),
            "expected shape controls in selection tray, got {tray_boxes:?}"
        );
        assert!(
            tray_boxes.iter().any(|(txt, _)| txt.contains("Curve")),
            "expected curve control in selection tray, got {tray_boxes:?}"
        );
        for hidden in ["Copy", "Cut", "Duplicate", "Save", "Paste", "Delete"] {
            assert!(
                !tray_boxes.iter().any(|(txt, _)| txt == hidden),
                "unexpected {hidden:?} button in selection tray: {tray_boxes:?}"
            );
        }

        // Open Stickers & Elements -> My Stickers while the shape is selected and save via popover.
        app.view.elements_popover_open = true;
        app.view.stickers_tab = crate::StickersTab::Saved;
        let mut pop_shapes = Vec::new();
        for f in 0..3 {
            pop_shapes = step_editor_frame(
                &ctx,
                &mut app,
                Vec2::new(1280.0, 800.0),
                0.36 + f64::from(f) * 0.02,
                0.02,
                Vec::new(),
            )
            .0;
        }
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 800.0));
        let save_btn_pos = collect_text_boxes_inside(&pop_shapes, viewport)
            .into_iter()
            .find(|(txt, _)| txt == "Save Selection")
            .map(|(_, r)| r.center())
            .expect("expected Save Selection button in My Stickers popover");
        for (i, pressed) in [true, false].into_iter().enumerate() {
            step_editor_frame(
                &ctx,
                &mut app,
                Vec2::new(1280.0, 800.0),
                0.44 + i as f64 * 0.02,
                0.02,
                vec![
                    egui::Event::PointerMoved(save_btn_pos),
                    egui::Event::PointerButton {
                        pos: save_btn_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(app.session.prefs.saved_elements.len(), 1);

        // Close popover and verify keyboard Delete still deletes the selected shape.
        app.view.elements_popover_open = false;
        step_editor_frame(
            &ctx,
            &mut app,
            Vec2::new(1280.0, 800.0),
            0.50,
            0.016,
            vec![egui::Event::Key {
                key: egui::Key::Delete,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.session.doc.shapes.is_empty());
    }

    fn step_editor_frame(
        ctx: &egui::Context,
        app: &mut PaperCraftApp,
        viewport: Vec2,
        time: f64,
        dt: f32,
        events: Vec<egui::Event>,
    ) -> (Vec<egui::epaint::ClippedShape>, egui::CursorIcon) {
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                time: Some(time),
                predicted_dt: dt,
                events,
                ..Default::default()
            },
            |ui| app.show_ui(ui),
        );
        let shapes = out.shapes.clone();
        let cursor = out.platform_output.cursor_icon;
        out.drop_without_applying_deltas();
        (shapes, cursor)
    }

    #[test]
    fn floating_color_dock_opens_on_left_edge_for_color_tools_and_hides_for_non_color_tools() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
        app.view.pages_sidebar_open = false;
        app.view.inspector_open = false;

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;
        for _ in 0..4 {
            time += 0.08;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.08, Vec::new());
        }

        // 1. Pen mode: top tray keeps thickness dots; color dock opens vertically on left edge.
        let t = Tokens::for_theme(app.session.prefs.theme);
        let (shapes, _) =
            step_editor_frame(&ctx, &mut app, viewport, time + 0.08, 0.08, Vec::new());
        time += 0.08;
        let top_tray = find_tray_frame_rect(&shapes, t.panel_bg, 90.0).expect("pen top tray");
        assert!(
            top_tray.width() <= 345.0 && (48.0..=54.0).contains(&top_tray.height()),
            "pen top tray should only hold thickness dots + horizontal slider + smoothing pill (no swatches), got {top_tray:?}"
        );
        let dock_rect = app.view.color_dock.last_rect;
        assert!(dock_rect.is_positive(), "pen mode must show color dock");
        assert!(
            dock_rect.height() > dock_rect.width() && (48.0..=54.0).contains(&dock_rect.width()),
            "color dock should be a slim proportional vertical capsule, got {dock_rect:?}"
        );
        assert!(
            (dock_rect.left() - COLOR_DOCK_OUTER_MARGIN).abs() <= 2.0,
            "color dock must open on the left edge of canvas, got {dock_rect:?}"
        );
        assert!(
            dock_rect.top() > top_tray.bottom(),
            "color dock must sit cleanly below top tray"
        );

        // 2. Opening Pages sidebar on the left glides the docked color bar inward so it stays visible.
        app.view.pages_sidebar_open = true;
        for _ in 0..12 {
            time += 0.05;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.05, Vec::new());
        }
        let shifted_dock = app.view.color_dock.last_rect;
        let canvas_left = crate::pages_sidebar::DEFAULT_SIDEBAR_WIDTH;
        assert!(
            shifted_dock.left() >= canvas_left + COLOR_DOCK_OUTER_MARGIN - 2.0,
            "color dock {shifted_dock:?} must stay clear of left pages sidebar ({canvas_left})"
        );
        app.view.pages_sidebar_open = false;
        for _ in 0..12 {
            time += 0.05;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.05, Vec::new());
        }

        // 3. Eraser has no colors -> color dock fades out and disappears.
        app.session
            .execute("tool.select", &json!({ "tool": "Eraser" }))
            .unwrap();
        for _ in 0..6 {
            time += 0.08;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.08, Vec::new());
        }
        assert!(
            !app.view.color_dock.last_rect.is_positive(),
            "eraser must hide color dock"
        );

        // 4. Select empty and Image selection have no colors -> hidden.
        crate::activate_select_tool(&mut app);
        for _ in 0..4 {
            time += 0.08;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.08, Vec::new());
        }
        assert!(!app.view.color_dock.last_rect.is_positive());

        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([0x28, 0x66, 0xE8, 0xFF]));
        let mut png_bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut png_bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        app.session
            .execute(
                "image.insert",
                &json!({
                    "x": 100.0,
                    "y": 100.0,
                    "w": 80.0,
                    "h": 80.0,
                    "base64_bytes": base64_encode_for_test(&png_bytes),
                }),
            )
            .unwrap();
        app.view.selected_object = Some(crate::SelectedObject::Image(0));
        app.view.selected_objects = vec![crate::SelectedObject::Image(0)];
        for _ in 0..4 {
            time += 0.08;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.08, Vec::new());
        }
        assert!(
            !app.view.color_dock.last_rect.is_positive(),
            "image-only selection must hide color dock"
        );

        // 5. LaserPointer has 4 laser colors -> dock shows on the left side cleanly.
        app.view.clear_selection();
        app.session
            .execute("tool.select", &json!({ "tool": "LaserPointer" }))
            .unwrap();
        for _ in 0..4 {
            time += 0.08;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.08, Vec::new());
        }
        assert!(
            app.view.color_dock.last_rect.is_positive(),
            "laser pointer must show laser colors in floating color dock"
        );
    }

    fn base64_encode_for_test(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b0 = u32::from(chunk[0]);
            let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
            let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            out.push(if chunk.len() > 1 {
                TABLE[((n >> 6) & 63) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                TABLE[(n & 63) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    #[test]
    fn floating_color_dock_drags_smoothly_via_tip_grip_preserves_pos_and_double_click_redocks() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
        app.view.pages_sidebar_open = true;
        app.view.inspector_open = false;

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;
        for _ in 0..4 {
            time += 0.08;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.08, Vec::new());
        }

        let initial_rect = app.view.color_dock.last_rect;
        assert!(initial_rect.is_positive());
        let grip_pos = egui::pos2(
            initial_rect.center().x,
            initial_rect.top() + COLOR_DOCK_INNER_MARGIN_Y + COLOR_DOCK_GRIP_SIZE.y * 0.5,
        );

        // Hover grip -> Grab cursor.
        time += 0.03;
        let (_, hover_cursor) = step_editor_frame(
            &ctx,
            &mut app,
            viewport,
            time,
            0.03,
            vec![egui::Event::PointerMoved(grip_pos)],
        );
        assert_eq!(hover_cursor, egui::CursorIcon::Grab);

        // Press and drag grip to right-center of canvas (~450px right, ~60px up).
        time += 0.03;
        let _ = step_editor_frame(
            &ctx,
            &mut app,
            viewport,
            time,
            0.03,
            vec![
                egui::Event::PointerMoved(grip_pos),
                egui::Event::PointerButton {
                    pos: grip_pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );

        let drag_target = egui::pos2(grip_pos.x + 450.0, grip_pos.y - 60.0);
        time += 0.02;
        let (_, drag_cursor) = step_editor_frame(
            &ctx,
            &mut app,
            viewport,
            time,
            0.02,
            vec![egui::Event::PointerMoved(drag_target)],
        );
        assert_eq!(drag_cursor, egui::CursorIcon::Grabbing);

        // Smooth glide across intermediate frames (not teleporting in one frame, then settling cleanly).
        time += 0.016;
        let _ = step_editor_frame(
            &ctx,
            &mut app,
            viewport,
            time,
            0.016,
            vec![egui::Event::PointerMoved(drag_target)],
        );
        let mid_rect = app.view.color_dock.last_rect;
        assert!(
            mid_rect.left() > initial_rect.left() + 20.0
                && mid_rect.left() < initial_rect.left() + 445.0,
            "expected smooth intermediate drag glide between {initial_rect:?} and target, got {mid_rect:?}"
        );

        // Release and settle smoothly.
        time += 0.02;
        let _ = step_editor_frame(
            &ctx,
            &mut app,
            viewport,
            time,
            0.02,
            vec![
                egui::Event::PointerMoved(drag_target),
                egui::Event::PointerButton {
                    pos: drag_target,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        for _ in 0..15 {
            time += 0.025;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.025, Vec::new());
        }
        let settled_rect = app.view.color_dock.last_rect;
        assert!(
            (settled_rect.left() - (initial_rect.left() + 450.0)).abs() <= 4.0,
            "expected settled dock near dragged target, got {settled_rect:?} from {initial_rect:?}"
        );
        assert!(
            app.session.doc.strokes.is_empty(),
            "dragging the color dock must never draw strokes on canvas"
        );

        // Click a swatch inside the moved dock -> changes pen color without drawing on canvas.
        let initial_pen_color = app.session.pen(PenKind::FountainPen).color;
        let fourth_swatch_pos = egui::pos2(
            settled_rect.center().x,
            settled_rect.top()
                + COLOR_DOCK_INNER_MARGIN_Y
                + COLOR_DOCK_GRIP_SIZE.y
                + COLOR_DOCK_DIVIDER_H * 2.0
                + COLOR_DOCK_WILDCARD_SIZE.y
                + COLOR_DOCK_ITEM_GAP * 7.0
                + COLOR_DOCK_SWATCH_SIZE.y * 3.5,
        );
        for pressed in [true, false] {
            time += 0.025;
            let _ = step_editor_frame(
                &ctx,
                &mut app,
                viewport,
                time,
                0.025,
                vec![
                    egui::Event::PointerMoved(fourth_swatch_pos),
                    egui::Event::PointerButton {
                        pos: fourth_swatch_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_ne!(
            app.session.pen(PenKind::FountainPen).color,
            initial_pen_color,
            "clicking swatch inside moved dock must change pen color"
        );
        assert!(app.session.doc.strokes.is_empty());

        // Switch to Eraser (hides dock) and back to FountainPen -> keeps user's dragged position!
        app.session
            .execute("tool.select", &json!({ "tool": "Eraser" }))
            .unwrap();
        for _ in 0..6 {
            time += 0.08;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.08, Vec::new());
        }
        app.session
            .execute("tool.select", &json!({ "tool": "FountainPen" }))
            .unwrap();
        for _ in 0..6 {
            time += 0.08;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.08, Vec::new());
        }
        assert!(
            (app.view.color_dock.last_rect.left() - settled_rect.left()).abs() <= 2.0,
            "switching tools within the same session must preserve dragged color dock position"
        );

        // Double-click grip handle -> smoothly glides back to the left edge!
        let moved_grip_pos = egui::pos2(
            settled_rect.center().x,
            settled_rect.top() + COLOR_DOCK_INNER_MARGIN_Y + COLOR_DOCK_GRIP_SIZE.y * 0.5,
        );
        for pressed in [true, false, true, false] {
            time += 0.03;
            let _ = step_editor_frame(
                &ctx,
                &mut app,
                viewport,
                time,
                0.03,
                vec![
                    egui::Event::PointerMoved(moved_grip_pos),
                    egui::Event::PointerButton {
                        pos: moved_grip_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(
            app.view.color_dock.custom_pos.is_none(),
            "double-clicking grip handle must reset custom_pos back to side docking"
        );
        for _ in 0..20 {
            time += 0.03;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.03, Vec::new());
        }
        let redocked_rect = app.view.color_dock.last_rect;
        assert!(
            (redocked_rect.left() - initial_rect.left()).abs() <= 2.0,
            "expected dock to glide back to initial left edge {initial_rect:?}, got {redocked_rect:?}"
        );
    }

    #[test]
    fn color_picker_opened_from_left_color_dock_positions_to_its_right_without_overlapping() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;
        for _ in 0..4 {
            time += 0.08;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.08, Vec::new());
        }

        let dock_rect = app.view.color_dock.last_rect;
        let palette_btn_pos = egui::pos2(
            dock_rect.center().x,
            dock_rect.bottom() - COLOR_DOCK_INNER_MARGIN_Y - COLOR_DOCK_PALETTE_BTN_SIZE * 0.5,
        );
        for pressed in [true, false] {
            time += 0.04;
            let _ = step_editor_frame(
                &ctx,
                &mut app,
                viewport,
                time,
                0.04,
                vec![
                    egui::Event::PointerMoved(palette_btn_pos),
                    egui::Event::PointerButton {
                        pos: palette_btn_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(app.view.color_picker.open);

        // Let the popover Area initialize and finish its fade-in animation.
        let mut shapes = Vec::new();
        for _ in 0..6 {
            time += 0.05;
            shapes = step_editor_frame(&ctx, &mut app, viewport, time, 0.05, Vec::new()).0;
        }
        let t = Tokens::for_theme(app.session.prefs.theme);
        let mut picker_rect = None;
        fn find_picker(shape: &egui::Shape, fill: Color32, out: &mut Option<egui::Rect>) {
            match shape {
                egui::Shape::Rect(rs)
                    if rs.fill == fill
                        && rs.corner_radius == CornerRadius::same(10)
                        && rs.rect.width() >= 280.0
                        && rs.rect.height() >= 200.0 =>
                {
                    *out = Some(rs.rect);
                }
                egui::Shape::Vec(items) => {
                    for s in items {
                        find_picker(s, fill, out);
                    }
                }
                _ => {}
            }
        }
        for c in &shapes {
            find_picker(&c.shape, t.field_bg, &mut picker_rect);
        }
        let picker = picker_rect.expect("color picker popover rect");
        assert!(
            picker.left() >= dock_rect.right(),
            "color picker {picker:?} should open cleanly to the right of left-docked color bar {dock_rect:?}"
        );
    }

    #[test]
    fn placing_sticker_centers_at_active_camera_viewport_not_page_center() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;
        for _ in 0..3 {
            time += 0.08;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.08, Vec::new());
        }

        // Zoom in and pan so the camera center rests over the upper-left quadrant of the page,
        // well away from the geometric page center (page_w * 0.5, page_h * 0.5).
        app.view.fit_requested = false;
        app.view.motion.camera_glide = None;
        app.view.zoom = 1.8;
        let canvas_rect = app.view.last_canvas_rect.expect("canvas rect recorded");
        let target_page = papercraft_geom::Point::new(220.0, 260.0);
        app.view.pan = [
            f64::from(canvas_rect.center().x) - target_page.x * 1.8,
            f64::from(canvas_rect.center().y) - target_page.y * 1.8,
        ];
        app.view.elements_popover_open = true;

        let mut shapes = Vec::new();
        for _ in 0..3 {
            time += 0.06;
            shapes = step_editor_frame(&ctx, &mut app, viewport, time, 0.06, Vec::new()).0;
        }

        let expected_center = app.camera_center_page_point();
        assert!((expected_center.x - target_page.x).abs() < 1.0);
        assert!((expected_center.y - target_page.y).abs() < 1.0);

        // Find and click the "Reminder" starter sticker card in the open popover.
        fn find_text_center(
            shapes: &[egui::epaint::ClippedShape],
            label: &str,
        ) -> Option<egui::Pos2> {
            fn visit(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
                match shape {
                    egui::Shape::Text(t) if t.galley.text() == label => {
                        Some(t.pos + t.galley.size() * 0.5)
                    }
                    egui::Shape::Vec(v) => v.iter().find_map(|s| visit(s, label)),
                    _ => None,
                }
            }
            shapes.iter().find_map(|c| visit(&c.shape, label))
        }

        let sticker_pos =
            find_text_center(&shapes, "Reminder").expect("Reminder sticker label should render");
        for pressed in [true, false] {
            time += 0.04;
            let _ = step_editor_frame(
                &ctx,
                &mut app,
                viewport,
                time,
                0.04,
                vec![
                    egui::Event::PointerMoved(sticker_pos),
                    egui::Event::PointerButton {
                        pos: sticker_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }

        assert!(!app.view.elements_popover_open);
        assert_eq!(app.session.doc.images.len(), 1);
        let placed = &app.session.doc.images[0];
        let placed_cx = placed.x + placed.width * 0.5;
        let placed_cy = placed.y + placed.height * 0.5;
        assert!(
            (placed_cx - expected_center.x).abs() < 1.0
                && (placed_cy - expected_center.y).abs() < 1.0,
            "expected sticker centered near camera {expected_center:?}, got ({placed_cx}, {placed_cy})"
        );
        let page_cx = f64::from(app.session.doc.width) * 0.5;
        let page_cy = f64::from(app.session.doc.height) * 0.5;
        assert!(
            (placed_cx - page_cx).abs() > 100.0 && (placed_cy - page_cy).abs() > 100.0,
            "sticker should not be placed at geometric page center ({page_cx}, {page_cy})"
        );
    }

    #[test]
    fn activating_ruler_centers_at_active_camera_and_keeps_standard_size_regardless_of_zoom() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;
        for _ in 0..3 {
            time += 0.08;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.08, Vec::new());
        }

        let expected_length = (f64::from(app.session.doc.width) * 0.72)
            .clamp(64.0, f64::from(app.session.doc.width).max(64.0));
        let expected_depth = crate::canvas::RULER_DEPTH;

        // 1. Zoom in and pan to the upper-left quadrant, well away from page center.
        app.view.fit_requested = false;
        app.view.motion.camera_glide = None;
        app.view.zoom = 1.8;
        let canvas_rect = app.view.last_canvas_rect.expect("canvas rect recorded");
        let target_page = papercraft_geom::Point::new(220.0, 260.0);
        app.view.pan = [
            f64::from(canvas_rect.center().x) - target_page.x * 1.8,
            f64::from(canvas_rect.center().y) - target_page.y * 1.8,
        ];

        toggle_ruler_tool(&mut app);
        assert!(app.view.ruler_active);
        let expected_center = app.camera_center_page_point();
        assert!((app.view.ruler_center[0] - expected_center.x).abs() < 1.0);
        assert!((app.view.ruler_center[1] - expected_center.y).abs() < 1.0);
        let page_cx = f64::from(app.session.doc.width) * 0.5;
        let page_cy = f64::from(app.session.doc.height) * 0.5;
        assert!(
            (app.view.ruler_center[0] - page_cx).abs() > 100.0
                && (app.view.ruler_center[1] - page_cy).abs() > 100.0,
            "ruler should spawn at active camera center, not page center"
        );
        assert!(
            (app.view.ruler_length - expected_length).abs() < 1e-6,
            "ruler length must stay standard ({expected_length}) at zoom 1.8, got {}",
            app.view.ruler_length
        );
        assert!(
            (app.view.ruler_depth - expected_depth).abs() < 1e-6,
            "ruler depth must stay standard ({expected_depth}) at zoom 1.8, got {}",
            app.view.ruler_depth
        );

        // 2. Deactivate, rotate ruler preference, pan/zoom to bottom-right quadrant, and re-activate.
        toggle_ruler_tool(&mut app);
        assert!(!app.view.ruler_active);
        app.view.ruler_angle = std::f64::consts::FRAC_PI_2;
        app.view.zoom = 3.5;
        let target_page_2 = papercraft_geom::Point::new(980.0, 1340.0);
        app.view.pan = [
            f64::from(canvas_rect.center().x) - target_page_2.x * 3.5,
            f64::from(canvas_rect.center().y) - target_page_2.y * 3.5,
        ];

        toggle_ruler_tool(&mut app);
        assert!(app.view.ruler_active);
        let expected_center_2 = app.camera_center_page_point();
        assert!((app.view.ruler_center[0] - expected_center_2.x).abs() < 1.0);
        assert!((app.view.ruler_center[1] - expected_center_2.y).abs() < 1.0);
        assert!(
            (app.view.ruler_angle - std::f64::consts::FRAC_PI_2).abs() < 1e-6,
            "subsequent activation should keep the user's ruler angle"
        );
        assert!(
            (app.view.ruler_length - expected_length).abs() < 1e-6,
            "vertical ruler length must stay standard ({expected_length}) at zoom 3.5, got {}",
            app.view.ruler_length
        );
        assert!(
            (app.view.ruler_depth - expected_depth).abs() < 1e-6,
            "vertical ruler depth must stay standard ({expected_depth}) at zoom 3.5, got {}",
            app.view.ruler_depth
        );

        // 3. Active camera_glide uses the glide's target camera center & keeps standard size.
        toggle_ruler_tool(&mut app);
        let glide_target = papercraft_geom::Point::new(140.0, 180.0);
        let glide_zoom = 2.5_f32;
        app.view.motion.camera_glide = Some(crate::motion::CameraGlide::new(
            app.view.zoom,
            app.view.pan,
            glide_zoom,
            [
                f64::from(canvas_rect.center().x) - glide_target.x * f64::from(glide_zoom),
                f64::from(canvas_rect.center().y) - glide_target.y * f64::from(glide_zoom),
            ],
            [
                f64::from(canvas_rect.center().x),
                f64::from(canvas_rect.center().y),
            ],
        ));
        toggle_ruler_tool(&mut app);
        assert!((app.view.ruler_center[0] - glide_target.x).abs() < 1.0);
        assert!((app.view.ruler_center[1] - glide_target.y).abs() < 1.0);
        assert!((app.view.ruler_length - expected_length).abs() < 1e-6);
        assert!((app.view.ruler_depth - expected_depth).abs() < 1e-6);
    }

    #[test]
    fn floating_color_dock_wildcard_is_pinned_separate_for_ink_and_highlighter_and_opens_only_picker()
     {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;
        for _ in 0..4 {
            time += 0.08;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.08, Vec::new());
        }
        let initial_dock_rect = app.view.color_dock.last_rect;
        let initial_ink_wildcard = app.session.prefs.wildcard_ink;
        let initial_hl_wildcard = app.session.prefs.wildcard_highlighter;

        // Click the pinned wildcard dot at the top of the dock (right below grip + divider).
        let wildcard_pos = egui::pos2(
            initial_dock_rect.center().x,
            initial_dock_rect.top()
                + COLOR_DOCK_INNER_MARGIN_Y
                + COLOR_DOCK_GRIP_SIZE.y
                + COLOR_DOCK_ITEM_GAP
                + COLOR_DOCK_DIVIDER_H
                + COLOR_DOCK_ITEM_GAP
                + COLOR_DOCK_WILDCARD_SIZE.y * 0.5,
        );
        for pressed in [true, false] {
            time += 0.04;
            let _ = step_editor_frame(
                &ctx,
                &mut app,
                viewport,
                time,
                0.04,
                vec![
                    egui::Event::PointerMoved(wildcard_pos),
                    egui::Event::PointerButton {
                        pos: wildcard_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(app.session.active_pen().color, initial_ink_wildcard);
        assert_eq!(
            app.view.color_dock.selected_wildcard,
            Some(PaletteTarget::Ink)
        );
        assert!(
            !app.view.color_picker.open,
            "first click selects the wildcard color without opening a popover"
        );

        // Click the first Classic palette swatch right below the wildcard divider to switch back.
        let first_swatch_pos = egui::pos2(
            initial_dock_rect.center().x,
            initial_dock_rect.top()
                + COLOR_DOCK_INNER_MARGIN_Y
                + COLOR_DOCK_GRIP_SIZE.y
                + COLOR_DOCK_DIVIDER_H * 2.0
                + COLOR_DOCK_WILDCARD_SIZE.y
                + COLOR_DOCK_ITEM_GAP * 4.0
                + COLOR_DOCK_SWATCH_SIZE.y * 0.5,
        );
        for pressed in [true, false] {
            time += 0.04;
            let _ = step_editor_frame(
                &ctx,
                &mut app,
                viewport,
                time,
                0.04,
                vec![
                    egui::Event::PointerMoved(first_swatch_pos),
                    egui::Event::PointerButton {
                        pos: first_swatch_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(
            app.session.active_pen().color.to_hex(),
            papercraft_color::INK_PALETTE[0].color.to_hex()
        );
        assert_eq!(app.view.color_dock.selected_wildcard, None);
        assert!(
            (app.view.color_dock.last_rect.height() - initial_dock_rect.height()).abs() <= 1.0,
            "wildcard dot stays pinned in the dock without changing dock height"
        );

        // Click wildcard dot to alternate back, then click again while active to open pure Color Picker.
        for _click in 0..2 {
            for pressed in [true, false] {
                time += 0.04;
                let _ = step_editor_frame(
                    &ctx,
                    &mut app,
                    viewport,
                    time,
                    0.04,
                    vec![
                        egui::Event::PointerMoved(wildcard_pos),
                        egui::Event::PointerButton {
                            pos: wildcard_pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        }
        assert_eq!(app.session.active_pen().color, initial_ink_wildcard);
        assert!(app.view.color_picker.is_wildcard_mode());
        assert!(!app.view.color_picker.is_palette_mode());

        // Switching to Highlighter keeps a completely separate wildcard color.
        app.view.color_picker.close();
        app.session
            .execute("tool.select", &json!({ "tool": "Highlighter" }))
            .unwrap();
        for _ in 0..6 {
            time += 0.05;
            let _ = step_editor_frame(&ctx, &mut app, viewport, time, 0.05, Vec::new());
        }
        let hl_dock = app.view.color_dock.last_rect;
        let hl_wildcard_pos = egui::pos2(
            hl_dock.center().x,
            hl_dock.top()
                + COLOR_DOCK_INNER_MARGIN_Y
                + COLOR_DOCK_GRIP_SIZE.y
                + COLOR_DOCK_ITEM_GAP
                + COLOR_DOCK_DIVIDER_H
                + COLOR_DOCK_ITEM_GAP
                + COLOR_DOCK_WILDCARD_SIZE.y * 0.5,
        );
        for pressed in [true, false] {
            time += 0.04;
            let _ = step_editor_frame(
                &ctx,
                &mut app,
                viewport,
                time,
                0.04,
                vec![
                    egui::Event::PointerMoved(hl_wildcard_pos),
                    egui::Event::PointerButton {
                        pos: hl_wildcard_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(app.session.active_pen().color, initial_hl_wildcard);
        assert_eq!(app.session.prefs.wildcard_ink, initial_ink_wildcard);
        assert_ne!(initial_ink_wildcard, initial_hl_wildcard);
    }

    fn find_slider_track_inside(
        shapes: &[egui::epaint::ClippedShape],
        tray_rect: egui::Rect,
    ) -> Option<egui::Rect> {
        let mut best: Option<egui::Rect> = None;
        fn visit(shape: &egui::Shape, tray_rect: egui::Rect, best: &mut Option<egui::Rect>) {
            match shape {
                egui::Shape::Rect(rs)
                    if tray_rect.contains_rect(rs.rect.shrink(0.5))
                        && (rs.rect.height() - PATH_SIZE_TRACK_HEIGHT).abs() <= 0.5
                        && rs.rect.width() >= 70.0 =>
                {
                    let replace = best.is_none_or(|cur| rs.rect.width() > cur.width());
                    if replace {
                        *best = Some(rs.rect);
                    }
                }
                egui::Shape::Vec(items) => {
                    for item in items {
                        visit(item, tray_rect, best);
                    }
                }
                _ => {}
            }
        }
        for clipped in shapes {
            visit(&clipped.shape, tray_rect, &mut best);
        }
        best
    }

    #[test]
    fn path_size_slider_adjusts_all_pens_shape_insert_and_selected_paths_with_single_undo() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let t = Tokens::for_theme(app.session.prefs.theme);
        let mut time = 0.0_f64;

        let settle_tray = |ctx: &egui::Context,
                           app: &mut PaperCraftApp,
                           time: &mut f64|
         -> (egui::Rect, egui::Rect) {
            let mut shapes = Vec::new();
            for _ in 0..4 {
                *time += 0.08;
                shapes = step_editor_frame(ctx, app, viewport, *time, 0.08, Vec::new()).0;
            }
            let tray = find_tray_frame_rect(&shapes, t.panel_bg, 90.0).expect("tray rect");
            let track = find_slider_track_inside(&shapes, tray).expect("slider track inside tray");
            (tray, track)
        };

        let click_pos =
            |ctx: &egui::Context, app: &mut PaperCraftApp, time: &mut f64, pos: egui::Pos2| {
                for pressed in [true, false] {
                    *time += 0.03;
                    let _ = step_editor_frame(
                        ctx,
                        app,
                        viewport,
                        *time,
                        0.03,
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

        let drag_across = |ctx: &egui::Context,
                           app: &mut PaperCraftApp,
                           time: &mut f64,
                           start: egui::Pos2,
                           mid: egui::Pos2,
                           end: egui::Pos2| {
            *time += 0.03;
            let _ = step_editor_frame(
                ctx,
                app,
                viewport,
                *time,
                0.03,
                vec![
                    egui::Event::PointerMoved(start),
                    egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            for pos in [mid, end] {
                *time += 0.03;
                let _ = step_editor_frame(
                    ctx,
                    app,
                    viewport,
                    *time,
                    0.03,
                    vec![egui::Event::PointerMoved(pos)],
                );
            }
            *time += 0.03;
            let _ = step_editor_frame(
                ctx,
                app,
                viewport,
                *time,
                0.03,
                vec![
                    egui::Event::PointerMoved(end),
                    egui::Event::PointerButton {
                        pos: end,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        };

        // 1. All 5 pen kinds expose the 3 presets + horizontal slider and respond to clicks.
        for (tool_name, kind, expected_min_after_click) in [
            ("FountainPen", PenKind::FountainPen, 16.0_f32),
            ("BallPen", PenKind::BallPen, 16.0),
            ("BrushPen", PenKind::BrushPen, 16.0),
            ("Highlighter", PenKind::Highlighter, 42.0),
            ("Pencil", PenKind::Pencil, 10.0),
        ] {
            app.session
                .execute("tool.select", &json!({ "tool": tool_name }))
                .unwrap();
            let (_, track) = settle_tray(&ctx, &mut app, &mut time);
            let target = egui::pos2(track.left() + track.width() * 0.85, track.center().y);
            click_pos(&ctx, &mut app, &mut time, target);
            let actual = app.session.pen(kind).size;
            assert!(
                actual >= expected_min_after_click,
                "{tool_name}: expected slider click at 85% to raise size >= {expected_min_after_click}, got {actual}"
            );
        }

        // 2. Shape insert mode also exposes the slider and inserts geometric shapes at that width.
        app.session
            .execute("tool.select", &json!({ "tool": "FountainPen" }))
            .unwrap();
        app.view.shape_insert_mode = true;
        let (_, shape_insert_track) = settle_tray(&ctx, &mut app, &mut time);
        let insert_target = egui::pos2(
            shape_insert_track.left() + shape_insert_track.width() * 0.75,
            shape_insert_track.center().y,
        );
        click_pos(&ctx, &mut app, &mut time, insert_target);
        let expected_shape_w = app.session.pen(PenKind::FountainPen).size;
        assert!(expected_shape_w > 12.0);
        app.session
            .execute(
                "shape.insert",
                &json!({ "kind": "Rectangle", "x1": 100.0, "y1": 100.0, "x2": 220.0, "y2": 180.0 }),
            )
            .unwrap();
        assert!((app.session.doc.shapes[0].width - expected_shape_w).abs() < 0.05);
        app.view.shape_insert_mode = false;

        // 3. Selected stroke: dragging slider updates stroke width and coalesces into 1 Undo step.
        app.session
            .execute(
                "stroke.draw",
                &json!({
                    "pen": papercraft_doc::PenSettings::fountain_pen(),
                    "points": [
                        { "x": 80.0, "y": 80.0, "pressure": 0.5 },
                        { "x": 160.0, "y": 110.0, "pressure": 0.6 },
                    ]
                }),
            )
            .unwrap();
        let stroke_id = app.session.doc.strokes[0].id;
        let orig_stroke_size = app.session.doc.strokes[0].pen.size;
        crate::activate_select_tool(&mut app);
        app.view.selected_object = Some(crate::SelectedObject::Stroke(stroke_id));
        app.view.selected_objects = vec![crate::SelectedObject::Stroke(stroke_id)];
        let (_, stroke_track) = settle_tray(&ctx, &mut app, &mut time);
        drag_across(
            &ctx,
            &mut app,
            &mut time,
            egui::pos2(
                stroke_track.left() + stroke_track.width() * 0.25,
                stroke_track.center().y,
            ),
            egui::pos2(
                stroke_track.left() + stroke_track.width() * 0.55,
                stroke_track.center().y,
            ),
            egui::pos2(
                stroke_track.left() + stroke_track.width() * 0.90,
                stroke_track.center().y,
            ),
        );
        let dragged_stroke_size = app.session.doc.strokes[0].pen.size;
        assert!(
            dragged_stroke_size > 18.0,
            "expected selected stroke drag to reach > 18 pt, got {dragged_stroke_size}"
        );
        app.session.execute("edit.undo", &json!({})).unwrap();
        assert!(
            (app.session.doc.strokes[0].pen.size - orig_stroke_size).abs() < 0.05,
            "single edit.undo must restore original stroke size {orig_stroke_size}, got {}",
            app.session.doc.strokes[0].pen.size
        );

        // 4. Selected geometric shape: dragging slider updates shape width and coalesces into 1 Undo step.
        let orig_shape_width = app.session.doc.shapes[0].width;
        app.view.selected_object = Some(crate::SelectedObject::Shape(0));
        app.view.selected_objects = vec![crate::SelectedObject::Shape(0)];
        let (_, sel_shape_track) = settle_tray(&ctx, &mut app, &mut time);
        drag_across(
            &ctx,
            &mut app,
            &mut time,
            egui::pos2(
                sel_shape_track.left() + sel_shape_track.width() * 0.60,
                sel_shape_track.center().y,
            ),
            egui::pos2(
                sel_shape_track.left() + sel_shape_track.width() * 0.35,
                sel_shape_track.center().y,
            ),
            egui::pos2(
                sel_shape_track.left() + sel_shape_track.width() * 0.10,
                sel_shape_track.center().y,
            ),
        );
        let dragged_shape_width = app.session.doc.shapes[0].width;
        assert!(
            dragged_shape_width < 4.5,
            "expected selected shape drag to reach < 4.5 pt, got {dragged_shape_width}"
        );
        app.session.execute("edit.undo", &json!({})).unwrap();
        assert!(
            (app.session.doc.shapes[0].width - orig_shape_width).abs() < 0.05,
            "single edit.undo must restore original shape width {orig_shape_width}, got {}",
            app.session.doc.shapes[0].width
        );
    }

    #[test]
    fn pen_context_tray_smoothing_popover_toggles_and_adjusts_strength() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;

        let settle = |ctx: &egui::Context, app: &mut PaperCraftApp, time: &mut f64| {
            let mut shapes = Vec::new();
            for _ in 0..4 {
                *time += 0.08;
                shapes = step_editor_frame(ctx, app, viewport, *time, 0.08, Vec::new()).0;
            }
            shapes
        };

        let find_text =
            |shapes: &[egui::epaint::ClippedShape], needle: &str| -> Option<egui::Pos2> {
                fn visit(shape: &egui::Shape, needle: &str) -> Option<egui::Pos2> {
                    match shape {
                        egui::Shape::Text(t) if t.galley.text() == needle => {
                            Some(t.pos + t.galley.rect.center().to_vec2())
                        }
                        egui::Shape::Vec(items) => items.iter().find_map(|s| visit(s, needle)),
                        _ => None,
                    }
                }
                shapes.iter().find_map(|c| visit(&c.shape, needle))
            };

        let click_at =
            |ctx: &egui::Context, app: &mut PaperCraftApp, time: &mut f64, pos: egui::Pos2| {
                for pressed in [true, false] {
                    *time += 0.03;
                    let _ = step_editor_frame(
                        ctx,
                        app,
                        viewport,
                        *time,
                        0.03,
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

        let collect_all_texts = |shapes: &[egui::epaint::ClippedShape]| -> String {
            let mut out = String::new();
            fn visit(shape: &egui::Shape, buf: &mut String) {
                match shape {
                    egui::Shape::Text(t) => {
                        buf.push_str(t.galley.text());
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
            for c in shapes {
                visit(&c.shape, &mut out);
            }
            out
        };

        let shapes = settle(&ctx, &mut app, &mut time);
        let pill_pos =
            find_text(&shapes, "50%").expect("default 50% smoothing pill in context tray");
        assert_eq!(
            app.session.pen(PenKind::FountainPen).smoothing.preset(),
            papercraft_doc::StabilizationPreset::ArithmeticBuffer10
        );
        assert_eq!(
            app.session.pen(PenKind::FountainPen).smoothing.buffer_size,
            10
        );
        click_at(&ctx, &mut app, &mut time, pill_pos);
        assert!(app.view.smoothing_popover_open);

        let pop_shapes = settle(&ctx, &mut app, &mut time);
        let pop_texts = collect_all_texts(&pop_shapes);
        assert!(!pop_texts.contains("Style:"));
        assert!(!pop_texts.contains("All styles"));
        assert!(!pop_texts.contains("buffer"));
        assert!(!pop_texts.contains("20"));
        let off_pos = find_text(&pop_shapes, "Off").expect("Off level inside smoothing popover");
        click_at(&ctx, &mut app, &mut time, off_pos);
        assert!(
            app.session
                .pen(PenKind::FountainPen)
                .smoothing
                .is_smoothing_disabled()
        );
        assert!(
            app.session.prefs.pen_smoothings[PenKind::FountainPen.index()].is_smoothing_disabled()
        );

        let pop_shapes2 = settle(&ctx, &mut app, &mut time);
        let high_pos =
            find_text(&pop_shapes2, "High").expect("High level inside smoothing popover");
        click_at(&ctx, &mut app, &mut time, high_pos);
        assert_eq!(
            app.session.pen(PenKind::FountainPen).smoothing.preset(),
            papercraft_doc::StabilizationPreset::ArithmeticBuffer10
        );
        assert_eq!(
            app.session
                .pen(PenKind::FountainPen)
                .smoothing
                .strength_percent(),
            80
        );
        assert_eq!(
            app.session.pen(PenKind::FountainPen).smoothing.buffer_size,
            16
        );
        assert_eq!(
            app.session.prefs.pen_smoothings[PenKind::FountainPen.index()].strength_percent(),
            80
        );

        // Switching to BallPen keeps its independent 50% (buffer size 10) smoothing setting.
        time += 0.03;
        let _ = step_editor_frame(
            &ctx,
            &mut app,
            viewport,
            time,
            0.03,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(!app.view.smoothing_popover_open);

        app.session
            .execute("tool.select", &json!({ "tool": "BallPen" }))
            .unwrap();
        let ball_shapes = settle(&ctx, &mut app, &mut time);
        let ball_pill = find_text(&ball_shapes, "50%").expect("BallPen starts with default 50%");
        assert_eq!(app.session.pen(PenKind::BallPen).smoothing.buffer_size, 10);
        click_at(&ctx, &mut app, &mut time, ball_pill);
        assert!(app.view.smoothing_popover_open);

        let ball_pop = settle(&ctx, &mut app, &mut time);
        let low_pos = find_text(&ball_pop, "Low").expect("Low level inside BallPen popover");
        click_at(&ctx, &mut app, &mut time, low_pos);
        assert_eq!(
            app.session
                .pen(PenKind::BallPen)
                .smoothing
                .strength_percent(),
            25
        );
        assert_eq!(app.session.pen(PenKind::BallPen).smoothing.buffer_size, 5);
        assert_eq!(
            app.session
                .pen(PenKind::FountainPen)
                .smoothing
                .strength_percent(),
            80
        );
        assert_eq!(
            app.session.pen(PenKind::FountainPen).smoothing.buffer_size,
            16
        );

        // Pressing Escape closes the smoothing popover.
        time += 0.03;
        let _ = step_editor_frame(
            &ctx,
            &mut app,
            viewport,
            time,
            0.03,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(!app.view.smoothing_popover_open);
    }

    #[test]
    fn shape_context_tray_omits_fill_and_opacity_pills_while_keeping_line_styles() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let t = Tokens::for_theme(app.session.prefs.theme);
        let mut time = 0.0_f64;

        let assert_no_fill_controls =
            |ctx: &egui::Context, app: &mut PaperCraftApp, time: &mut f64, label: &str| {
                let mut shapes = Vec::new();
                for _ in 0..4 {
                    *time += 0.08;
                    shapes = step_editor_frame(ctx, app, viewport, *time, 0.08, Vec::new()).0;
                }
                let tray_rect =
                    find_tray_frame_rect(&shapes, t.panel_bg, 90.0).expect("shape top tray");
                let boxes = collect_text_boxes_inside(&shapes, tray_rect);
                for forbidden in ["Fill", "20%", "45%", "85%"] {
                    assert!(
                        !boxes.iter().any(|(txt, _)| txt == forbidden),
                        "{label}: fixed top tray must not contain {forbidden:?}, got {boxes:?}"
                    );
                }
                assert!(
                    boxes.iter().any(|(txt, _)| txt == "Rectangle"),
                    "{label}: expected shape kind controls in top tray, got {boxes:?}"
                );
                fn count_swatch_dots(shape: &egui::Shape, tray_rect: egui::Rect) -> usize {
                    match shape {
                        egui::Shape::Circle(c)
                            if tray_rect.contains(c.center) && (c.radius - 6.5).abs() < 0.2 =>
                        {
                            1
                        }
                        egui::Shape::Vec(items) => {
                            items.iter().map(|s| count_swatch_dots(s, tray_rect)).sum()
                        }
                        _ => 0,
                    }
                }
                let swatch_dots: usize = shapes
                    .iter()
                    .map(|c| count_swatch_dots(&c.shape, tray_rect))
                    .sum();
                assert_eq!(
                    swatch_dots, 0,
                    "{label}: fixed top context tray must not contain fill color dots"
                );
            };

        // 1. ShapeInsert mode.
        app.view.shape_insert_mode = true;
        app.view.shape_fill_enabled = true;
        app.view.shape_fill_color =
            Some(papercraft_color::Color::from_rgba8(0x28, 0x66, 0xE8, 115));
        assert_no_fill_controls(&ctx, &mut app, &mut time, "ShapeInsert");
        app.view.shape_insert_mode = false;

        // 2. Single selected shape.
        app.session
            .execute(
                "shape.insert",
                &json!({
                    "kind": "Rectangle",
                    "x1": 100.0,
                    "y1": 100.0,
                    "x2": 220.0,
                    "y2": 180.0,
                    "fill_color": papercraft_color::Color::from_rgba8(0x28, 0x66, 0xE8, 115),
                }),
            )
            .unwrap();
        crate::activate_select_tool(&mut app);
        app.view.selected_object = Some(crate::SelectedObject::Shape(0));
        app.view.selected_objects = vec![crate::SelectedObject::Shape(0)];
        assert_no_fill_controls(&ctx, &mut app, &mut time, "SingleShape");

        // 3. Multi-selected shapes.
        app.session
            .execute(
                "shape.insert",
                &json!({
                    "kind": "Ellipse",
                    "x1": 260.0,
                    "y1": 100.0,
                    "x2": 360.0,
                    "y2": 180.0,
                }),
            )
            .unwrap();
        app.view.selected_objects = vec![
            crate::SelectedObject::Shape(0),
            crate::SelectedObject::Shape(1),
        ];
        assert_no_fill_controls(&ctx, &mut app, &mut time, "MultiShape");
    }

    #[test]
    fn floating_color_dock_shape_selection_switches_border_and_fill_uses_wheel_and_no_color() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);

        let initial_border = papercraft_color::Color::from_rgba8(0x11, 0x18, 0x27, 255);
        app.session
            .execute(
                "shape.insert",
                &json!({
                    "kind": "Rectangle",
                    "x1": 120.0,
                    "y1": 120.0,
                    "x2": 280.0,
                    "y2": 220.0,
                    "color": initial_border,
                }),
            )
            .unwrap();
        crate::activate_select_tool(&mut app);
        app.view.selected_object = Some(crate::SelectedObject::Shape(0));
        app.view.selected_objects = vec![crate::SelectedObject::Shape(0)];

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;
        let settle = |ctx: &egui::Context, app: &mut PaperCraftApp, time: &mut f64| {
            let mut shapes = Vec::new();
            for _ in 0..5 {
                *time += 0.08;
                shapes = step_editor_frame(ctx, app, viewport, *time, 0.08, Vec::new()).0;
            }
            shapes
        };
        let click_at =
            |ctx: &egui::Context, app: &mut PaperCraftApp, time: &mut f64, pos: egui::Pos2| {
                for pressed in [true, false] {
                    *time += 0.04;
                    let _ = step_editor_frame(
                        ctx,
                        app,
                        viewport,
                        *time,
                        0.04,
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

        let shapes = settle(&ctx, &mut app, &mut time);
        let dock_rect = app.view.color_dock.last_rect;
        assert!(dock_rect.is_positive());
        let dock_texts = collect_text_boxes_inside(&shapes, dock_rect);
        let border_pos = dock_texts
            .iter()
            .find(|(txt, _)| txt == "Border")
            .map(|(_, r)| r.center())
            .expect("Border toggle inside floating color dock");
        let fill_pos = dock_texts
            .iter()
            .find(|(txt, _)| txt == "Fill")
            .map(|(_, r)| r.center())
            .expect("Fill toggle inside floating color dock");

        // 1. Switch to Fill and click swatch #3 (Crimson Red) in the 2-column grid.
        click_at(&ctx, &mut app, &mut time, fill_pos);
        assert_eq!(app.view.shape_color_target, crate::ShapeColorTarget::Fill);

        let special_y = dock_rect.top()
            + COLOR_DOCK_INNER_MARGIN_Y
            + COLOR_DOCK_GRIP_SIZE.y
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_DIVIDER_H
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_SHAPE_SWITCH_H
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_DIVIDER_H
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_SHAPE_SPECIAL_SIZE.y * 0.5;
        let wc_pos = egui::pos2(
            dock_rect.center().x - (COLOR_DOCK_SHAPE_SPECIAL_SIZE.x + COLOR_DOCK_ITEM_GAP) * 0.5,
            special_y,
        );
        let no_color_pos = egui::pos2(
            dock_rect.center().x + (COLOR_DOCK_SHAPE_SPECIAL_SIZE.x + COLOR_DOCK_ITEM_GAP) * 0.5,
            special_y,
        );
        let swatch_row0_y = special_y
            + COLOR_DOCK_SHAPE_SPECIAL_SIZE.y * 0.5
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_DIVIDER_H
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_SHAPE_SWATCH_SIZE.y * 0.5;
        let swatch_crimson_pos = egui::pos2(
            dock_rect.center().x + (COLOR_DOCK_SHAPE_SWATCH_SIZE.x + COLOR_DOCK_ITEM_GAP) * 0.5,
            swatch_row0_y + COLOR_DOCK_SHAPE_SWATCH_SIZE.y + COLOR_DOCK_ITEM_GAP,
        );

        click_at(&ctx, &mut app, &mut time, swatch_crimson_pos);
        let fill_after_swatch = app.session.doc.shapes[0]
            .fill_color
            .expect("fill_color set from floating dock swatch");
        assert!(same_rgb(
            fill_after_swatch,
            papercraft_color::INK_PALETTE[3].color
        ));
        assert!(same_rgb(app.session.doc.shapes[0].color, initial_border));

        // 2. Open Custom Color wheel popover on Fill (click 1 selects wildcard, click 2 opens wheel).
        click_at(&ctx, &mut app, &mut time, wc_pos);
        click_at(&ctx, &mut app, &mut time, wc_pos);
        assert!(app.view.color_picker.is_wildcard_mode());
        let pop_shapes = settle(&ctx, &mut app, &mut time);
        assert!(collect_text_shapes(&pop_shapes).contains("Shape Fill Color"));

        let sv_rect = pop_shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Mesh(m) => {
                    let mut r = egui::Rect::NOTHING;
                    for v in &m.vertices {
                        r.extend_with(v.pos);
                    }
                    if r.height() >= 100.0 { Some(r) } else { None }
                }
                _ => None,
            })
            .next()
            .expect("SV square mesh in Shape Fill Color popover");
        let wheel_pick_pos = egui::pos2(sv_rect.right() - 10.0, sv_rect.top() + 10.0);
        click_at(&ctx, &mut app, &mut time, wheel_pick_pos);
        let fill_after_wheel = app.session.doc.shapes[0]
            .fill_color
            .expect("fill_color set from color wheel");
        assert!(!same_rgb(
            fill_after_wheel,
            papercraft_color::INK_PALETTE[3].color
        ));
        assert!(same_rgb(app.session.doc.shapes[0].color, initial_border));

        // 3. Click No Color on Fill -> clears fill_color while preserving border.
        click_at(&ctx, &mut app, &mut time, no_color_pos);
        assert!(app.session.doc.shapes[0].fill_color.is_none());
        assert!(!app.view.color_picker.open);
        assert!(same_rgb(app.session.doc.shapes[0].color, initial_border));

        // 4. Switch to Border, click No Color -> border alpha becomes 0.0; click swatch -> restored opaque.
        click_at(&ctx, &mut app, &mut time, border_pos);
        assert_eq!(app.view.shape_color_target, crate::ShapeColorTarget::Border);
        click_at(&ctx, &mut app, &mut time, no_color_pos);
        assert_eq!(app.session.doc.shapes[0].color.a, 0.0);

        click_at(&ctx, &mut app, &mut time, swatch_crimson_pos);
        assert!(app.session.doc.shapes[0].color.a > 0.99);
        assert!(same_rgb(
            app.session.doc.shapes[0].color,
            papercraft_color::INK_PALETTE[3].color
        ));
        assert!(app.session.doc.shapes[0].fill_color.is_none());
    }

    #[test]
    fn floating_color_dock_shape_insert_mode_keeps_pen_color_and_commits_borderless_filled_shape() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
        let orig_pen_color = app.session.active_pen().color;
        assert!(orig_pen_color.a > 0.99);

        app.view.shape_insert_mode = true;
        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;
        let mut shapes = Vec::new();
        for _ in 0..5 {
            time += 0.08;
            shapes = step_editor_frame(&ctx, &mut app, viewport, time, 0.08, Vec::new()).0;
        }
        let dock_rect = app.view.color_dock.last_rect;
        let dock_texts = collect_text_boxes_inside(&shapes, dock_rect);
        let border_pos = dock_texts
            .iter()
            .find(|(txt, _)| txt == "Border")
            .map(|(_, r)| r.center())
            .expect("Border toggle in ShapeInsert dock");
        let fill_pos = dock_texts
            .iter()
            .find(|(txt, _)| txt == "Fill")
            .map(|(_, r)| r.center())
            .expect("Fill toggle in ShapeInsert dock");

        let click_at =
            |ctx: &egui::Context, app: &mut PaperCraftApp, time: &mut f64, pos: egui::Pos2| {
                for pressed in [true, false] {
                    *time += 0.04;
                    let _ = step_editor_frame(
                        ctx,
                        app,
                        viewport,
                        *time,
                        0.04,
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

        let special_y = dock_rect.top()
            + COLOR_DOCK_INNER_MARGIN_Y
            + COLOR_DOCK_GRIP_SIZE.y
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_DIVIDER_H
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_SHAPE_SWITCH_H
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_DIVIDER_H
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_SHAPE_SPECIAL_SIZE.y * 0.5;
        let no_color_pos = egui::pos2(
            dock_rect.center().x + (COLOR_DOCK_SHAPE_SPECIAL_SIZE.x + COLOR_DOCK_ITEM_GAP) * 0.5,
            special_y,
        );
        let swatch_row0_y = special_y
            + COLOR_DOCK_SHAPE_SPECIAL_SIZE.y * 0.5
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_DIVIDER_H
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_SHAPE_SWATCH_SIZE.y * 0.5;
        let swatch_crimson_pos = egui::pos2(
            dock_rect.center().x + (COLOR_DOCK_SHAPE_SWATCH_SIZE.x + COLOR_DOCK_ITEM_GAP) * 0.5,
            swatch_row0_y + COLOR_DOCK_SHAPE_SWATCH_SIZE.y + COLOR_DOCK_ITEM_GAP,
        );

        click_at(&ctx, &mut app, &mut time, fill_pos);
        click_at(&ctx, &mut app, &mut time, swatch_crimson_pos);
        click_at(&ctx, &mut app, &mut time, border_pos);
        click_at(&ctx, &mut app, &mut time, no_color_pos);

        assert_eq!(effective_shape_insert_border(&app), None);
        let eff_fill = effective_shape_insert_fill(&app).expect("fill enabled");
        assert!(same_rgb(eff_fill, papercraft_color::INK_PALETTE[3].color));
        assert_eq!(
            app.session.active_pen().color,
            orig_pen_color,
            "setting shape border to No Color must never mutate pen.color"
        );

        // Drag on the canvas to insert a borderless filled shape.
        let canvas_rect = app.view.last_canvas_rect.expect("canvas rect");
        let p0 = canvas_rect.center() - Vec2::new(40.0, 30.0);
        let p1 = canvas_rect.center() + Vec2::new(40.0, 30.0);
        for (pos, pressed) in [(p0, true), (p1, false)] {
            time += 0.04;
            let _ = step_editor_frame(
                &ctx,
                &mut app,
                viewport,
                time,
                0.04,
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
        assert_eq!(app.session.doc.shapes.len(), 1);
        assert_eq!(app.session.doc.shapes[0].color.a, 0.0);
        let committed_fill = app.session.doc.shapes[0]
            .fill_color
            .expect("inserted shape carries fill color");
        assert!(same_rgb(
            committed_fill,
            papercraft_color::INK_PALETTE[3].color
        ));
        assert!(!app.view.shape_insert_mode);
        assert_eq!(app.session.active_tool, ToolKind::Select);
        assert_eq!(
            app.view.selected_objects,
            vec![crate::SelectedObject::Shape(0)]
        );

        // Context tray and color dock immediately open editing for the inserted shape.
        for _ in 0..4 {
            time += 0.08;
            shapes = step_editor_frame(&ctx, &mut app, viewport, time, 0.08, Vec::new()).0;
        }
        let sel_dock_rect = app.view.color_dock.last_rect;
        let sel_dock_texts = collect_text_boxes_inside(&shapes, sel_dock_rect);
        let sel_border_pos = sel_dock_texts
            .iter()
            .find(|(txt, _)| txt == "Border")
            .map(|(_, r)| r.center())
            .expect("Border toggle in Selection color dock after shape insert");
        let sel_swatch_ocean_pos = egui::pos2(
            dock_rect.center().x - (COLOR_DOCK_SHAPE_SWATCH_SIZE.x + COLOR_DOCK_ITEM_GAP) * 0.5,
            swatch_row0_y + COLOR_DOCK_SHAPE_SWATCH_SIZE.y + COLOR_DOCK_ITEM_GAP,
        );
        click_at(&ctx, &mut app, &mut time, sel_border_pos);
        click_at(&ctx, &mut app, &mut time, sel_swatch_ocean_pos);
        assert!(
            app.session.doc.shapes[0].color.a > 0.99,
            "clicking Border swatch right after shape creation must style the new shape directly"
        );
    }

    #[test]
    fn floating_color_dock_multi_shape_selection_updates_and_undoes_atomically() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);

        app.session
            .execute(
                "shape.insert",
                &json!({ "kind": "Rectangle", "x1": 80.0, "y1": 80.0, "x2": 180.0, "y2": 160.0 }),
            )
            .unwrap();
        app.session
            .execute(
                "shape.insert",
                &json!({ "kind": "Ellipse", "x1": 220.0, "y1": 80.0, "x2": 320.0, "y2": 160.0 }),
            )
            .unwrap();
        crate::activate_select_tool(&mut app);
        app.view.selected_objects = vec![
            crate::SelectedObject::Shape(0),
            crate::SelectedObject::Shape(1),
        ];

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;
        let mut shapes = Vec::new();
        for _ in 0..5 {
            time += 0.08;
            shapes = step_editor_frame(&ctx, &mut app, viewport, time, 0.08, Vec::new()).0;
        }
        let dock_rect = app.view.color_dock.last_rect;
        let dock_texts = collect_text_boxes_inside(&shapes, dock_rect);
        let border_pos = dock_texts
            .iter()
            .find(|(txt, _)| txt == "Border")
            .map(|(_, r)| r.center())
            .expect("Border toggle");
        let fill_pos = dock_texts
            .iter()
            .find(|(txt, _)| txt == "Fill")
            .map(|(_, r)| r.center())
            .expect("Fill toggle");

        let click_at =
            |ctx: &egui::Context, app: &mut PaperCraftApp, time: &mut f64, pos: egui::Pos2| {
                for pressed in [true, false] {
                    *time += 0.04;
                    let _ = step_editor_frame(
                        ctx,
                        app,
                        viewport,
                        *time,
                        0.04,
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

        let special_y = dock_rect.top()
            + COLOR_DOCK_INNER_MARGIN_Y
            + COLOR_DOCK_GRIP_SIZE.y
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_DIVIDER_H
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_SHAPE_SWITCH_H
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_DIVIDER_H
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_SHAPE_SPECIAL_SIZE.y * 0.5;
        let no_color_pos = egui::pos2(
            dock_rect.center().x + (COLOR_DOCK_SHAPE_SPECIAL_SIZE.x + COLOR_DOCK_ITEM_GAP) * 0.5,
            special_y,
        );
        let swatch_row0_y = special_y
            + COLOR_DOCK_SHAPE_SPECIAL_SIZE.y * 0.5
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_DIVIDER_H
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_SHAPE_SWATCH_SIZE.y * 0.5;
        let swatch_cobalt_pos = egui::pos2(
            dock_rect.center().x + (COLOR_DOCK_SHAPE_SWATCH_SIZE.x + COLOR_DOCK_ITEM_GAP) * 0.5,
            swatch_row0_y,
        );

        click_at(&ctx, &mut app, &mut time, fill_pos);
        click_at(&ctx, &mut app, &mut time, swatch_cobalt_pos);
        assert!(app.session.doc.shapes.iter().all(|s| {
            s.fill_color
                .is_some_and(|fc| same_rgb(fc, papercraft_color::INK_PALETTE[1].color))
        }));

        click_at(&ctx, &mut app, &mut time, no_color_pos);
        assert!(
            app.session
                .doc
                .shapes
                .iter()
                .all(|s| s.fill_color.is_none())
        );
        app.session.execute("edit.undo", &json!({})).unwrap();
        assert!(app.session.doc.shapes.iter().all(|s| {
            s.fill_color
                .is_some_and(|fc| same_rgb(fc, papercraft_color::INK_PALETTE[1].color))
        }));

        click_at(&ctx, &mut app, &mut time, border_pos);
        click_at(&ctx, &mut app, &mut time, no_color_pos);
        assert!(app.session.doc.shapes.iter().all(|s| s.color.a == 0.0));
        app.session.execute("edit.undo", &json!({})).unwrap();
        assert!(app.session.doc.shapes.iter().all(|s| s.color.a > 0.99));
    }

    #[test]
    fn color_picker_alpha_applies_to_shapes_pens_text_and_handles_zero_percent_gracefully() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);

        app.session
            .execute(
                "shape.insert",
                &json!({
                    "kind": "Rectangle",
                    "x1": 100.0,
                    "y1": 100.0,
                    "x2": 260.0,
                    "y2": 200.0,
                }),
            )
            .unwrap();
        crate::activate_select_tool(&mut app);
        app.view.selected_object = Some(crate::SelectedObject::Shape(0));
        app.view.selected_objects = vec![crate::SelectedObject::Shape(0)];

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut time = 0.0_f64;
        let settle = |ctx: &egui::Context, app: &mut PaperCraftApp, time: &mut f64| {
            let mut shapes = Vec::new();
            for _ in 0..5 {
                *time += 0.08;
                shapes = step_editor_frame(ctx, app, viewport, *time, 0.08, Vec::new()).0;
            }
            shapes
        };
        let click_at =
            |ctx: &egui::Context, app: &mut PaperCraftApp, time: &mut f64, pos: egui::Pos2| {
                for pressed in [true, false] {
                    *time += 0.04;
                    let _ = step_editor_frame(
                        ctx,
                        app,
                        viewport,
                        *time,
                        0.04,
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
        let find_alpha_bar = |shapes: &[egui::epaint::ClippedShape]| {
            shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Mesh(m) => {
                        let mut r = egui::Rect::NOTHING;
                        for v in &m.vertices {
                            r.extend_with(v.pos);
                        }
                        if (r.width() - 232.0).abs() < 2.0 && (r.height() - 14.0).abs() < 2.0 {
                            Some(r)
                        } else {
                            None
                        }
                    }
                    _ => None,
                })
                .next()
        };

        let shapes = settle(&ctx, &mut app, &mut time);
        let dock_rect = app.view.color_dock.last_rect;
        let dock_texts = collect_text_boxes_inside(&shapes, dock_rect);
        let border_pos = dock_texts
            .iter()
            .find(|(txt, _)| txt == "Border")
            .map(|(_, r)| r.center())
            .expect("Border toggle");
        let fill_pos = dock_texts
            .iter()
            .find(|(txt, _)| txt == "Fill")
            .map(|(_, r)| r.center())
            .expect("Fill toggle");

        let special_y = dock_rect.top()
            + COLOR_DOCK_INNER_MARGIN_Y
            + COLOR_DOCK_GRIP_SIZE.y
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_DIVIDER_H
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_SHAPE_SWITCH_H
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_DIVIDER_H
            + COLOR_DOCK_ITEM_GAP
            + COLOR_DOCK_SHAPE_SPECIAL_SIZE.y * 0.5;
        let wc_pos = egui::pos2(
            dock_rect.center().x - (COLOR_DOCK_SHAPE_SPECIAL_SIZE.x + COLOR_DOCK_ITEM_GAP) * 0.5,
            special_y,
        );

        // 1. Switch to Fill, open Wildcard ColorPicker, adjust alpha to 30% -> shape fill gets 0.30 alpha.
        click_at(&ctx, &mut app, &mut time, fill_pos);
        click_at(&ctx, &mut app, &mut time, wc_pos);
        click_at(&ctx, &mut app, &mut time, wc_pos);
        assert!(app.view.color_picker.is_wildcard_mode());

        let pop_shapes = settle(&ctx, &mut app, &mut time);
        let alpha_bar = find_alpha_bar(&pop_shapes).expect("alpha bar in Shape Fill Color");
        click_at(
            &ctx,
            &mut app,
            &mut time,
            egui::pos2(
                alpha_bar.left() + alpha_bar.width() * 0.30,
                alpha_bar.center().y,
            ),
        );
        let fill = app.session.doc.shapes[0].fill_color.expect("fill applied");
        assert!(
            (fill.a - 0.30).abs() < 0.02,
            "expected 0.30 alpha, got {}",
            fill.a
        );

        // 2. Dragging alpha to 0% clears fill_color without closing the picker or trapping wildcard selection.
        click_at(
            &ctx,
            &mut app,
            &mut time,
            egui::pos2(alpha_bar.left() + 0.5, alpha_bar.center().y),
        );
        assert!(app.session.doc.shapes[0].fill_color.is_none());
        assert!(app.view.color_picker.open);
        assert_eq!(
            app.view.color_dock.selected_wildcard,
            Some(PaletteTarget::Ink)
        );

        // Raising alpha to 65% restores fill_color cleanly.
        click_at(
            &ctx,
            &mut app,
            &mut time,
            egui::pos2(
                alpha_bar.left() + alpha_bar.width() * 0.65,
                alpha_bar.center().y,
            ),
        );
        let restored_fill = app.session.doc.shapes[0].fill_color.expect("fill restored");
        assert!(
            (restored_fill.a - 0.65).abs() < 0.02,
            "expected 0.65 alpha, got {}",
            restored_fill.a
        );

        // 3. Switch to Border and set 50% opacity via alpha bar.
        click_at(&ctx, &mut app, &mut time, border_pos);
        click_at(&ctx, &mut app, &mut time, wc_pos);
        if !app.view.color_picker.is_wildcard_mode() {
            click_at(&ctx, &mut app, &mut time, wc_pos);
        }
        let border_pop = settle(&ctx, &mut app, &mut time);
        let border_alpha = find_alpha_bar(&border_pop).expect("alpha bar in Shape Border Color");
        click_at(
            &ctx,
            &mut app,
            &mut time,
            egui::pos2(
                border_alpha.left() + border_alpha.width() * 0.50,
                border_alpha.center().y,
            ),
        );
        assert!(
            (app.session.doc.shapes[0].color.a - 0.50).abs() < 0.02,
            "expected 0.50 border alpha, got {}",
            app.session.doc.shapes[0].color.a
        );

        // 4. Pen wildcard picker preserves alpha on the active pen.
        app.view.color_picker.close();
        app.view.clear_selection();
        app.session
            .execute("tool.select", &json!({ "tool": "FountainPen" }))
            .unwrap();
        let pen_shapes = settle(&ctx, &mut app, &mut time);
        let pen_dock = app.view.color_dock.last_rect;
        let pen_wc_pos = egui::pos2(
            pen_dock.center().x,
            pen_dock.top()
                + COLOR_DOCK_INNER_MARGIN_Y
                + COLOR_DOCK_GRIP_SIZE.y
                + COLOR_DOCK_ITEM_GAP
                + COLOR_DOCK_DIVIDER_H
                + COLOR_DOCK_ITEM_GAP
                + COLOR_DOCK_WILDCARD_SIZE.y * 0.5,
        );
        click_at(&ctx, &mut app, &mut time, pen_wc_pos);
        if !app.view.color_picker.is_wildcard_mode() {
            click_at(&ctx, &mut app, &mut time, pen_wc_pos);
        }
        let _ = pen_shapes;
        let pen_pop = settle(&ctx, &mut app, &mut time);
        let pen_alpha = find_alpha_bar(&pen_pop).expect("alpha bar for Pen color");
        click_at(
            &ctx,
            &mut app,
            &mut time,
            egui::pos2(
                pen_alpha.left() + pen_alpha.width() * 0.45,
                pen_alpha.center().y,
            ),
        );
        assert!(
            (app.session.active_pen().color.a - 0.45).abs() < 0.02,
            "expected 0.45 pen alpha, got {}",
            app.session.active_pen().color.a
        );

        // 5. Text insert mode preserves alpha on text_color_draft.
        app.view.color_picker.close();
        app.view.text_insert_mode = true;
        let _ = settle(&ctx, &mut app, &mut time);
        let text_dock = app.view.color_dock.last_rect;
        let text_wc_pos = egui::pos2(
            text_dock.center().x,
            text_dock.top()
                + COLOR_DOCK_INNER_MARGIN_Y
                + COLOR_DOCK_GRIP_SIZE.y
                + COLOR_DOCK_ITEM_GAP
                + COLOR_DOCK_DIVIDER_H
                + COLOR_DOCK_ITEM_GAP
                + COLOR_DOCK_WILDCARD_SIZE.y * 0.5,
        );
        click_at(&ctx, &mut app, &mut time, text_wc_pos);
        if !app.view.color_picker.is_wildcard_mode() {
            click_at(&ctx, &mut app, &mut time, text_wc_pos);
        }
        let text_pop = settle(&ctx, &mut app, &mut time);
        let text_alpha = find_alpha_bar(&text_pop).expect("alpha bar for Text color");
        click_at(
            &ctx,
            &mut app,
            &mut time,
            egui::pos2(
                text_alpha.left() + text_alpha.width() * 0.70,
                text_alpha.center().y,
            ),
        );
        assert!(
            (app.view.text_color_draft.a - 0.70).abs() < 0.02,
            "expected 0.70 text draft alpha, got {}",
            app.view.text_color_draft.a
        );
    }
}
